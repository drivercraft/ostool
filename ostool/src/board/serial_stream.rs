use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use std::io::ErrorKind;

use anyhow::Context as _;
use futures::{SinkExt, StreamExt};
use tokio::{
    io::{AsyncReadExt, split},
    task::JoinHandle,
    time::timeout,
};
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

    let (runner_stream, bridge_stream) = tokio::io::duplex(64 * 1024);
    let (runner_rx, runner_tx) = split(runner_stream);
    let (mut bridge_rx, mut bridge_tx) = split(bridge_stream);

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
            mut read_task,
            mut write_task,
        } = self;
        let shutdown = async {
            let write_result = (&mut write_task).await;
            let read_result = (&mut read_task).await;

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
                write_task.abort();
                read_task.abort();
                let _ = write_task.await;
                let _ = read_task.await;
                Err(anyhow::anyhow!(
                    "serial websocket shutdown timed out after {}s",
                    duration.as_secs_f64()
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use tokio::{sync::Notify, task::JoinHandle};

    use super::{SerialStreamTasks, connect_serial_stream, websocket_request, write_bridge_bytes};

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
