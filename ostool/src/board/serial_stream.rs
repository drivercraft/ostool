use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use std::io::ErrorKind;

use anyhow::Context as _;
use futures::{SinkExt, StreamExt};
use tokio::{io::AsyncReadExt, task::JoinHandle, time::timeout};
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::{
    client::IntoClientRequest as _,
    http::{HeaderValue, header::AUTHORIZATION},
};
use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

use crate::board::terminal::{
    ServerControlAction, ServerControlMessage, classify_server_control_message,
};

pub type BoxedAsyncRead = Box<dyn futures::AsyncRead + Send + Unpin>;
pub type BoxedAsyncWrite = Box<dyn futures::AsyncWrite + Send + Unpin>;

pub struct SerialStreamTasks {
    read_task: JoinHandle<anyhow::Result<()>>,
    write_task: JoinHandle<anyhow::Result<()>>,
}

pub async fn connect_serial_stream(
    ws_url: reqwest::Url,
    authorization: Option<String>,
) -> anyhow::Result<(BoxedAsyncWrite, BoxedAsyncRead, SerialStreamTasks)> {
    let request = websocket_request(&ws_url, authorization.as_deref())?;
    let (stream, _) = tokio_tungstenite::connect_async(request)
        .await
        .with_context(|| format!("failed to connect serial websocket {ws_url}"))?;
    let (mut ws_sink, mut ws_stream) = stream.split();
    let locally_closed = Arc::new(AtomicBool::new(false));

    // Each direction owns its endpoint independently. Splitting one duplex
    // keeps both directions alive until both halves are dropped, hiding remote
    // failures from the reader and local EOF from the WebSocket writer.
    let (runner_rx, mut bridge_tx) = tokio::io::duplex(64 * 1024);
    let (mut bridge_rx, runner_tx) = tokio::io::duplex(64 * 1024);

    let read_task = tokio::spawn({
        let locally_closed = locally_closed.clone();
        async move {
            while let Some(message) = ws_stream.next().await {
                match message.context("serial websocket read failed")? {
                    Message::Binary(bytes) => {
                        if write_bridge_bytes(&mut bridge_tx, &bytes)
                            .await
                            .context("failed to write serial websocket bytes")?
                        {
                            break;
                        }
                    }
                    Message::Text(text) => {
                        if let Ok(control) = serde_json::from_str::<ServerControlMessage>(&text) {
                            match classify_server_control_message(
                                &control,
                                locally_closed.load(Ordering::SeqCst),
                            ) {
                                ServerControlAction::Ignore => continue,
                                ServerControlAction::Close => break,
                                ServerControlAction::Error(err) => return Err(err),
                                ServerControlAction::Forward => {}
                            }
                        }

                        if write_bridge_bytes(&mut bridge_tx, text.as_bytes())
                            .await
                            .context("failed to write text serial websocket payload")?
                        {
                            break;
                        }
                    }
                    Message::Close(_) => {
                        if locally_closed.load(Ordering::SeqCst) {
                            break;
                        }
                        anyhow::bail!(
                            "ostool-server closed the serial websocket; the board session may have been released"
                        );
                    }
                    Message::Ping(_) => {}
                    Message::Pong(_) | Message::Frame(_) => {}
                }
            }

            Ok(())
        }
    });

    let write_task = tokio::spawn({
        let locally_closed = locally_closed.clone();
        async move {
            let mut buffer = [0u8; 4096];
            loop {
                let read = bridge_rx
                    .read(&mut buffer)
                    .await
                    .context("failed to read runner serial bytes")?;
                if read == 0 {
                    break;
                }
                ws_sink
                    .send(Message::Binary(buffer[..read].to_vec().into()))
                    .await
                    .context("serial websocket write failed")?;
            }

            locally_closed.store(true, Ordering::SeqCst);
            let _ = ws_sink
                .send(Message::Text(r#"{"type":"close"}"#.to_string().into()))
                .await;
            let _ = ws_sink.send(Message::Close(None)).await;
            Ok(())
        }
    });

    Ok((
        Box::new(runner_tx.compat_write()),
        Box::new(runner_rx.compat()),
        SerialStreamTasks {
            read_task,
            write_task,
        },
    ))
}

pub(crate) fn websocket_request(
    ws_url: &reqwest::Url,
    authorization: Option<&str>,
) -> anyhow::Result<tokio_tungstenite::tungstenite::http::Request<()>> {
    let mut request = ws_url
        .as_str()
        .into_client_request()
        .context("failed to build serial websocket request")?;
    if let Some(token) = authorization {
        // The URL was origin-checked by BoardServerClient before this point.
        let value = HeaderValue::from_str(&format!("Bearer {token}"))
            .context("invalid websocket authorization header")?;
        request.headers_mut().insert(AUTHORIZATION, value);
    }
    Ok(request)
}

async fn write_bridge_bytes<W>(writer: &mut W, bytes: &[u8]) -> anyhow::Result<bool>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    match tokio::io::AsyncWriteExt::write_all(writer, bytes).await {
        Ok(()) => {}
        Err(err) if err.kind() == ErrorKind::BrokenPipe => return Ok(true),
        Err(err) => return Err(err.into()),
    }
    match tokio::io::AsyncWriteExt::flush(writer).await {
        Ok(()) => Ok(false),
        Err(err) if err.kind() == ErrorKind::BrokenPipe => Ok(true),
        Err(err) => Err(err.into()),
    }
}

impl SerialStreamTasks {
    pub async fn shutdown(self) -> anyhow::Result<()> {
        let write_result = self.write_task.await;
        let read_result = self.read_task.await;

        if let Ok(Err(err)) = write_result {
            return Err(err);
        }
        if let Err(err) = write_result
            && !err.is_cancelled()
        {
            return Err(anyhow::anyhow!("serial websocket writer join error: {err}"));
        }
        if let Ok(Err(err)) = read_result {
            return Err(err);
        }
        if let Err(err) = read_result
            && !err.is_cancelled()
        {
            return Err(anyhow::anyhow!("serial websocket reader join error: {err}"));
        }

        Ok(())
    }

    pub async fn shutdown_with_timeout(self, duration: std::time::Duration) -> anyhow::Result<()> {
        let SerialStreamTasks {
            read_task,
            write_task,
        } = self;
        // Each handle sits inside a slot that releases ownership only once the
        // handle resolves. If the timeout cancels this attempt, the slot still
        // owns whichever tasks have not finished, so they can be aborted and
        // joined instead of being detached.
        let mut write_slot = TaskSlot(Some(write_task));
        let mut read_slot = TaskSlot(Some(read_task));

        let shutdown = async {
            // Both handles are fully awaited before either result is inspected,
            // matching the original behavior: a writer failure must not abandon
            // a reader that is still shutting down.
            let write_result = write_slot.await_handle().await;
            let read_result = read_slot.await_handle().await;

            if let Ok(Err(err)) = write_result {
                return Err(err);
            }
            if let Err(err) = write_result
                && !err.is_cancelled()
            {
                return Err(anyhow::anyhow!("serial websocket writer join error: {err}"));
            }
            if let Ok(Err(err)) = read_result {
                return Err(err);
            }
            if let Err(err) = read_result
                && !err.is_cancelled()
            {
                return Err(anyhow::anyhow!("serial websocket reader join error: {err}"));
            }

            Ok(())
        };

        match timeout(duration, shutdown).await {
            Ok(result) => result,
            Err(_) => {
                // Only the tasks that have not finished yet are still owned by
                // their slot; anything already joined is a no-op here. This also
                // covers the timeout firing while the writer itself is pending.
                write_slot.abort_and_join().await;
                read_slot.abort_and_join().await;
                Err(anyhow::anyhow!(
                    "serial websocket shutdown timed out after {}s",
                    duration.as_secs_f64()
                ))
            }
        }
    }
}

/// Owns one shutdown `JoinHandle` and guarantees it is polled at most once.
struct TaskSlot(Option<JoinHandle<anyhow::Result<()>>>);

impl TaskSlot {
    /// Awaits the handle in place. The slot is cleared only after the `await`
    /// resolves, and that assignment has no intervening `await`, so a future
    /// cancelled mid-poll leaves the still-running task owned by this slot for
    /// timeout cleanup. The `JoinError` result is returned untouched so the
    /// caller keeps the original error handling.
    async fn await_handle(&mut self) -> Result<anyhow::Result<()>, tokio::task::JoinError> {
        let handle = self.0.as_mut().expect("task slot must be occupied");
        let result = handle.await;
        self.0 = None;
        result
    }

    /// Cancels and joins the handle if it is still owned. No-op once the handle
    /// has resolved and the slot cleared itself.
    async fn abort_and_join(&mut self) {
        let Some(handle) = self.0.take() else {
            return;
        };
        handle.abort();
        let _ = handle.await;
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };
    use tokio::{
        sync::{Notify, oneshot},
        task::JoinHandle,
    };

    use super::{SerialStreamTasks, connect_serial_stream, websocket_request, write_bridge_bytes};

    #[tokio::test]
    async fn remote_error_ends_runner_input_while_writer_is_alive() {
        use futures::{AsyncReadExt, SinkExt};
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            ws.send(Message::Binary(b"boot output\n".to_vec().into()))
                .await
                .unwrap();
            ws.send(Message::Text(
                r#"{"type":"error","message":"automatic power-on failed"}"#.into(),
            ))
            .await
            .unwrap();
        });
        let url = reqwest::Url::parse(&format!("ws://{address}/serial")).unwrap();
        let (writer, mut reader, tasks) = connect_serial_stream(url, None).await.unwrap();
        let error = tasks.read_task.await.unwrap().unwrap_err();
        assert!(error.to_string().contains("automatic power-on failed"));
        let mut bytes = Vec::new();
        let result = tokio::time::timeout(
            std::time::Duration::from_secs(1),
            reader.read_to_end(&mut bytes),
        )
        .await;
        tasks.write_task.abort();
        let _ = tasks.write_task.await;
        drop(writer);
        server.await.unwrap();
        result
            .expect("completed WebSocket reader must signal EOF")
            .unwrap();
        assert_eq!(bytes, b"boot output\n");
    }

    #[tokio::test]
    async fn dropping_runner_writer_closes_websocket_while_reader_is_alive() {
        use futures::StreamExt;
        use tokio_tungstenite::tungstenite::Message;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            let mut ws = tokio_tungstenite::accept_async(socket).await.unwrap();
            assert_eq!(
                ws.next().await.unwrap().unwrap(),
                Message::Text(r#"{"type":"close"}"#.into())
            );
        });
        let url = reqwest::Url::parse(&format!("ws://{address}/serial")).unwrap();
        let (writer, reader, tasks) = connect_serial_stream(url, None).await.unwrap();
        drop(writer);
        let mut write_task = tasks.write_task;
        let result = tokio::time::timeout(std::time::Duration::from_secs(1), &mut write_task).await;
        tasks.read_task.abort();
        let _ = tasks.read_task.await;
        write_task.abort();
        drop(reader);
        if result.is_err() {
            server.abort();
        } else {
            server.await.unwrap();
        }
        result
            .expect("writer must observe local EOF independently")
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn secure_websocket_support_is_enabled() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (socket, _) = listener.accept().await.unwrap();
            drop(socket);
        });

        let url = reqwest::Url::parse(&format!("wss://{address}/serial")).unwrap();
        let result = connect_serial_stream(url, None).await;
        server.await.unwrap();

        let error = result
            .err()
            .expect("dummy TLS server should reject the client");
        assert!(!format!("{error:#}").contains("TLS support not compiled in"));
    }

    #[tokio::test]
    async fn shutdown_waits_for_writer_before_reader() {
        let reader_released = Arc::new(Notify::new());
        let writer_finished = Arc::new(AtomicBool::new(false));

        let read_task: JoinHandle<anyhow::Result<()>> = {
            let reader_released = reader_released.clone();
            let writer_finished = writer_finished.clone();
            tokio::spawn(async move {
                while !writer_finished.load(Ordering::SeqCst) {
                    reader_released.notified().await;
                }
                Ok(())
            })
        };

        let write_task: JoinHandle<anyhow::Result<()>> = {
            let reader_released = reader_released.clone();
            let writer_finished = writer_finished.clone();
            tokio::spawn(async move {
                writer_finished.store(true, Ordering::SeqCst);
                reader_released.notify_waiters();
                Ok(())
            })
        };

        SerialStreamTasks {
            read_task,
            write_task,
        }
        .shutdown()
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn shutdown_allows_reader_to_finish_after_local_consumer_closed() {
        let (mut writer, reader) = tokio::io::duplex(1);
        drop(reader);

        let read_task: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            if write_bridge_bytes(&mut writer, b"late console output").await? {
                return Ok(());
            }
            anyhow::bail!("bridge writer unexpectedly stayed open")
        });
        let write_task: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move { Ok(()) });

        SerialStreamTasks {
            read_task,
            write_task,
        }
        .shutdown()
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn shutdown_with_timeout_handles_completed_writer_with_pending_reader() {
        // A `pending` reader can never complete, so it deterministically stays
        // blocked regardless of how the runtime schedules the two tasks. When
        // its future is dropped (aborted), the guard reports `read_stopped`.
        let (read_started_tx, read_started_rx) = oneshot::channel::<()>();
        let (read_stopped_tx, mut read_stopped_rx) = oneshot::channel::<()>();
        let read_task: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            let _guard = HasPendingGuard::new(read_stopped_tx);
            let _ = read_started_tx.send(());
            std::future::pending::<anyhow::Result<()>>().await
        });

        // The writer resolves immediately, so `shutdown_with_timeout` joins it
        // before it ever reaches the pending reader.
        let (write_started_tx, write_started_rx) = oneshot::channel::<()>();
        let (write_stopped_tx, mut write_stopped_rx) = oneshot::channel::<()>();
        let write_task: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            let _guard = HasPendingGuard::new(write_stopped_tx);
            let _ = write_started_tx.send(());
            Ok(())
        });

        // Both tasks are confirmed started before shutdown, so the stop
        // signals below can only come from shutdown's own cleanup.
        assert!(read_started_rx.await.is_ok());
        assert!(write_started_rx.await.is_ok());

        // The timeout is only a termination guard. On the old implementation
        // this panicked with `JoinHandle polled after completion`: the timeout
        // branch re-awaited the writer handle the shutdown attempt had already
        // resolved.
        let error = SerialStreamTasks {
            read_task,
            write_task,
        }
        .shutdown_with_timeout(std::time::Duration::from_secs(1))
        .await
        .expect_err("pending reader must surface a timeout error");

        // Both tasks must be fully stopped by the time shutdown returns: the
        // writer finished on its own, and the blocked reader must have been
        // aborted and joined rather than left detached.
        assert!(
            write_stopped_rx.try_recv().is_ok(),
            "writer task was not observed as stopped"
        );
        assert!(
            read_stopped_rx.try_recv().is_ok(),
            "pending reader task was not aborted/joined"
        );
        assert!(
            error.to_string().contains("shutdown timed out"),
            "unexpected shutdown error: {error}"
        );
    }

    #[tokio::test]
    async fn shutdown_with_timeout_cancels_pending_writer() {
        // The timeout fires while the writer is still pending. Both handles are
        // therefore still owned by their slots and must be cancelled and joined
        // exactly once; a leaked handle would never drop its guard.
        let (writer_started_tx, writer_started_rx) = oneshot::channel::<()>();
        let (writer_stopped_tx, mut writer_stopped_rx) = oneshot::channel::<()>();
        let write_task: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            let _guard = HasPendingGuard::new(writer_stopped_tx);
            let _ = writer_started_tx.send(());
            std::future::pending::<anyhow::Result<()>>().await
        });

        let (reader_started_tx, reader_started_rx) = oneshot::channel::<()>();
        let (reader_stopped_tx, mut reader_stopped_rx) = oneshot::channel::<()>();
        let read_task: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            let _guard = HasPendingGuard::new(reader_stopped_tx);
            let _ = reader_started_tx.send(());
            std::future::pending::<anyhow::Result<()>>().await
        });

        assert!(writer_started_rx.await.is_ok());
        assert!(reader_started_rx.await.is_ok());

        let error = SerialStreamTasks {
            read_task,
            write_task,
        }
        .shutdown_with_timeout(std::time::Duration::from_secs(1))
        .await
        .expect_err("pending writer must surface a timeout error");

        assert!(
            writer_stopped_rx.try_recv().is_ok(),
            "pending writer task was not aborted/joined"
        );
        assert!(
            reader_stopped_rx.try_recv().is_ok(),
            "pending reader task was not aborted/joined"
        );
        assert!(
            error.to_string().contains("shutdown timed out"),
            "unexpected shutdown error: {error}"
        );
    }

    /// Lives across an await point inside a background task and reports exactly
    /// when that task's future is dropped (natural completion or abort). Used
    /// to prove shutdown actually stopped a task instead of detaching it.
    struct HasPendingGuard(Option<oneshot::Sender<()>>);

    impl HasPendingGuard {
        fn new(stopped: oneshot::Sender<()>) -> Self {
            Self(Some(stopped))
        }
    }

    impl Drop for HasPendingGuard {
        fn drop(&mut self) {
            if let Some(stopped) = self.0.take() {
                let _ = stopped.send(());
            }
        }
    }

    #[tokio::test]
    async fn bridge_writer_treats_closed_local_consumer_as_done() {
        let (mut writer, reader) = tokio::io::duplex(1);
        drop(reader);

        assert!(
            write_bridge_bytes(&mut writer, b"late console output")
                .await
                .unwrap()
        );
    }

    #[test]
    fn websocket_request_carries_bearer_token() {
        let url = reqwest::Url::parse("wss://example.invalid/serial").unwrap();
        let request = websocket_request(&url, Some("token-value")).unwrap();
        assert_eq!(
            request.headers().get("authorization").unwrap(),
            "Bearer token-value"
        );
    }
}
