//! Scoped serial/WebSocket workers. No worker may wait for another transport's I/O.

use std::future::pending;

use anyhow::Context;
use axum::extract::ws::Message;
use futures_util::{Sink, SinkExt, Stream, StreamExt};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite},
    sync::{mpsc, watch},
};

use super::ws::{ClientControlMessage, decode_serial_payload};

// Bound memory in bytes as well as messages. At 1.5 Mbaud the output queue holds
// up to 1.7 seconds of traffic; a persistently stalled consumer ends the session.
const CHUNK_SIZE: usize = 4096;
const QUEUE_CHUNKS: usize = 64;
const CONTROL_MESSAGES: usize = 8;
pub(super) const MAX_COMMAND_SIZE: usize = CHUNK_SIZE * QUEUE_CHUNKS;

pub(super) async fn run<R, W, S, I, E, F, H>(
    serial_rx: &mut R,
    serial_tx: &mut W,
    ws_sender: &mut S,
    ws_receiver: &mut I,
    mut ready: watch::Receiver<bool>,
    heartbeat: F,
) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin,
    W: AsyncWrite + Unpin,
    S: Sink<Message> + Unpin,
    S::Error: std::error::Error + Send + Sync + 'static,
    I: Stream<Item = Result<Message, E>> + Unpin,
    E: std::error::Error + Send + Sync + 'static,
    F: Fn() -> H,
    H: Future<Output = ()>,
{
    let (output_tx, mut output_rx) = mpsc::channel(QUEUE_CHUNKS);
    let (input_tx, mut input_rx) = mpsc::channel::<Vec<u8>>(QUEUE_CHUNKS);
    let (control_tx, mut control_rx) = mpsc::channel(CONTROL_MESSAGES);

    let read_serial = async {
        let mut buffer = [0; CHUNK_SIZE];
        loop {
            let size = serial_rx
                .read(&mut buffer)
                .await
                .context("serial read failed")?;
            if size == 0 {
                // Let the WebSocket writer drain queued final bytes before ending.
                drop(output_tx);
                return pending::<anyhow::Result<()>>().await;
            }
            output_tx.try_send(buffer[..size].to_vec()).map_err(|_| {
                anyhow::anyhow!("serial output buffer full or closed; websocket cannot keep up")
            })?;
            tokio::task::yield_now().await;
        }
    };
    let write_websocket = async {
        ws_sender
            .send(Message::Text(r#"{"type":"opened"}"#.into()))
            .await?;
        loop {
            let message = tokio::select! {
                output = output_rx.recv() => {
                    let Some(output) = output else { return Ok::<(), anyhow::Error>(()); };
                    Message::Binary(output.into())
                }
                Some(control) = control_rx.recv() => control,
            };
            ws_sender
                .send(message)
                .await
                .context("failed to send serial output over websocket")?;
            heartbeat().await;
        }
    };
    let read_websocket = async {
        ready
            .wait_for(|ready| *ready)
            .await
            .context("power-on cancelled")?;
        while let Some(message) = ws_receiver.next().await {
            let payload = match message? {
                Message::Binary(bytes) => Some(bytes.to_vec()),
                Message::Text(text) => {
                    let control: ClientControlMessage = serde_json::from_str(&text)?;
                    match control.kind.as_str() {
                        "close" => return Ok::<(), anyhow::Error>(()),
                        "tx" => Some(decode_serial_payload(control)?),
                        other => anyhow::bail!("unsupported websocket control type `{other}`"),
                    }
                }
                Message::Close(_) => return Ok(()),
                Message::Ping(payload) => {
                    control_tx
                        .try_send(Message::Pong(payload))
                        .context("websocket control buffer full or closed")?;
                    None
                }
                Message::Pong(_) => None,
            };
            if let Some(payload) = payload {
                anyhow::ensure!(
                    payload.len() <= MAX_COMMAND_SIZE,
                    "serial command too large"
                );
                let chunks = payload.len().div_ceil(CHUNK_SIZE);
                // Single producer: reserve the entire command before publishing its first byte.
                anyhow::ensure!(chunks <= input_tx.capacity(), "serial command buffer full");
                for chunk in payload.chunks(CHUNK_SIZE) {
                    input_tx
                        .try_send(chunk.to_vec())
                        .context("serial command buffer closed")?;
                }
            }
            heartbeat().await;
            tokio::task::yield_now().await;
        }
        Ok(())
    };
    let write_serial = async {
        while let Some(payload) = input_rx.recv().await {
            // SerialStream writes directly to the kernel. flush() calls blocking
            // tcdrain(), holding tokio::io::split's mutex and preventing reads.
            super::ws::write_serial_payload(serial_tx, &payload)
                .await
                .context("serial write failed")?;
        }
        Ok::<(), anyhow::Error>(())
    };

    // These futures borrow the transports, so cancellation drops every worker
    // before the caller can reunite/close the serial port. Nothing is detached.
    tokio::select! {
        result = read_serial => result,
        result = write_websocket => result,
        result = read_websocket => result,
        result = write_serial => result,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{Sink, stream};
    use std::{
        io,
        pin::Pin,
        sync::Arc,
        task::{Context, Poll},
        time::Duration,
    };
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        sync::{Notify, mpsc, watch},
    };

    struct OutputSink {
        output: mpsc::UnboundedSender<Message>,
        gate: Option<tokio::sync::oneshot::Receiver<()>>,
        blocked: Arc<Notify>,
    }

    impl Sink<Message> for OutputSink {
        type Error = io::Error;
        fn poll_ready(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
            if let Some(gate) = self.gate.as_mut() {
                if Pin::new(gate).poll(cx).is_pending() {
                    self.blocked.notify_one();
                    return Poll::Pending;
                }
                self.gate = None;
            }
            Poll::Ready(Ok(()))
        }
        fn start_send(self: Pin<&mut Self>, item: Message) -> io::Result<()> {
            self.output.send(item).map_err(io::Error::other)
        }
        fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
        fn poll_close(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    async fn deadline<T>(future: impl Future<Output = T>) -> T {
        tokio::time::timeout(Duration::from_secs(2), future)
            .await
            .expect("transport stopped making progress")
    }

    #[tokio::test]
    async fn blocked_websocket_does_not_stop_serial_receive_and_eof_drains() {
        let (mut board, server) = tokio::io::duplex(64);
        let (mut rx, mut tx) = tokio::io::split(server);
        let (output, mut received) = mpsc::unbounded_channel();
        let (release, gate) = tokio::sync::oneshot::channel();
        let blocked = Arc::new(Notify::new());
        let mut sink = OutputSink {
            output,
            gate: Some(gate),
            blocked: blocked.clone(),
        };
        let (_ready, ready) = watch::channel(true);
        let task = tokio::spawn(async move {
            run(
                &mut rx,
                &mut tx,
                &mut sink,
                &mut stream::pending::<Result<Message, io::Error>>(),
                ready,
                || async {},
            )
            .await
        });
        deadline(blocked.notified()).await;
        let payload: Vec<u8> = (0..1024).map(|i| i as u8).collect();
        // More than the duplex hardware buffer: completion requires serial reads,
        // while the WebSocket sink is provably still blocked by the gate.
        deadline(board.write_all(&payload)).await.unwrap();
        board.shutdown().await.unwrap();
        release.send(()).unwrap();
        deadline(task).await.unwrap().unwrap();
        let mut bytes = Vec::new();
        while let Some(message) = received.recv().await {
            if let Message::Binary(chunk) = message {
                bytes.extend_from_slice(&chunk);
            }
        }
        assert_eq!(bytes, payload);
    }

    #[tokio::test]
    async fn blocked_serial_write_does_not_stop_output_or_peer_close() {
        let (mut board, server) = tokio::io::duplex(16);
        let (mut rx, mut tx) = tokio::io::split(server);
        let (output, mut received) = mpsc::unbounded_channel();
        let mut sink = OutputSink {
            output,
            gate: None,
            blocked: Arc::new(Notify::new()),
        };
        let (commands, mut command_rx) = mpsc::unbounded_channel();
        let mut input = Box::pin(stream::poll_fn(move |cx| command_rx.poll_recv(cx)));
        let (_ready, ready) = watch::channel(true);
        let task = tokio::spawn(async move {
            run(&mut rx, &mut tx, &mut sink, &mut input, ready, || async {}).await
        });
        commands
            .send(Ok::<_, io::Error>(Message::Binary(vec![0x41; 128].into())))
            .unwrap();
        let mut first = [0; 16];
        deadline(board.read_exact(&mut first)).await.unwrap();
        // Leave the rest unread, so the serial writer cannot finish.
        board.write_all(b"still alive").await.unwrap();
        loop {
            if let Message::Binary(bytes) = deadline(received.recv()).await.unwrap() {
                assert_eq!(bytes.as_ref(), b"still alive");
                break;
            }
        }
        commands.send(Ok(Message::Close(None))).unwrap();
        deadline(task).await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn output_overflow_terminates_instead_of_silently_losing_bytes() {
        let (mut board, server) = tokio::io::duplex(CHUNK_SIZE);
        let (mut rx, mut tx) = tokio::io::split(server);
        let (output, _received) = mpsc::unbounded_channel();
        let (_release, gate) = tokio::sync::oneshot::channel();
        let mut sink = OutputSink {
            output,
            gate: Some(gate),
            blocked: Arc::new(Notify::new()),
        };
        let (_ready, ready) = watch::channel(true);
        let task = tokio::spawn(async move {
            run(
                &mut rx,
                &mut tx,
                &mut sink,
                &mut stream::pending::<Result<Message, io::Error>>(),
                ready,
                || async {},
            )
            .await
        });
        let writer = tokio::spawn(async move {
            board
                .write_all(&vec![0; CHUNK_SIZE * (QUEUE_CHUNKS + 2)])
                .await
        });
        let error = deadline(task).await.unwrap().unwrap_err();
        assert!(
            error.to_string().contains("serial output buffer full"),
            "{error:#}"
        );
        let _ = deadline(writer).await.unwrap();
    }

    #[tokio::test]
    async fn sustained_output_larger_than_queue_keeps_byte_order() {
        let (mut board, server) = tokio::io::duplex(CHUNK_SIZE);
        let (mut rx, mut tx) = tokio::io::split(server);
        let (output, mut received) = mpsc::unbounded_channel();
        let mut sink = OutputSink {
            output,
            gate: None,
            blocked: Arc::new(Notify::new()),
        };
        let (_ready, ready) = watch::channel(true);
        let task = tokio::spawn(async move {
            run(
                &mut rx,
                &mut tx,
                &mut sink,
                &mut stream::pending::<Result<Message, io::Error>>(),
                ready,
                || async {},
            )
            .await
        });
        let payload: Vec<u8> = (0..CHUNK_SIZE * QUEUE_CHUNKS * 4)
            .map(|i| (i % 251) as u8)
            .collect();
        deadline(board.write_all(&payload)).await.unwrap();
        board.shutdown().await.unwrap();
        deadline(task).await.unwrap().unwrap();
        let mut bytes = Vec::new();
        while let Some(message) = received.recv().await {
            if let Message::Binary(chunk) = message {
                bytes.extend_from_slice(&chunk);
            }
        }
        assert_eq!(bytes, payload);
    }

    #[tokio::test]
    async fn oversized_command_is_rejected_before_writing_any_byte() {
        let (mut board, server) = tokio::io::duplex(64);
        let (mut rx, mut tx) = tokio::io::split(server);
        let (output, _received) = mpsc::unbounded_channel();
        let mut sink = OutputSink {
            output,
            gate: None,
            blocked: Arc::new(Notify::new()),
        };
        let (_ready, ready) = watch::channel(true);
        let mut input = stream::iter([Ok::<_, io::Error>(Message::Binary(
            vec![0; MAX_COMMAND_SIZE + 1].into(),
        ))]);
        let err = run(&mut rx, &mut tx, &mut sink, &mut input, ready, || async {})
            .await
            .unwrap_err();
        assert!(err.to_string().contains("serial command too large"));
        drop(rx.unsplit(tx));
        let mut bytes = Vec::new();
        board.read_to_end(&mut bytes).await.unwrap();
        assert!(bytes.is_empty());
    }

    #[tokio::test]
    async fn cancelling_workers_returns_serial_ownership() {
        let (mut board, server) = tokio::io::duplex(64);
        let (mut rx, mut tx) = tokio::io::split(server);
        let (output, _received) = mpsc::unbounded_channel();
        let (_release, gate) = tokio::sync::oneshot::channel();
        let blocked = Arc::new(Notify::new());
        let mut sink = OutputSink {
            output,
            gate: Some(gate),
            blocked: blocked.clone(),
        };
        let (_ready, ready) = watch::channel(true);
        let mut incoming = stream::pending::<Result<Message, io::Error>>();
        tokio::select! {
            _ = run(&mut rx, &mut tx, &mut sink, &mut incoming, ready, || async {}) => panic!("transport ended early"),
            _ = blocked.notified() => {}
        }
        let mut server = rx.unsplit(tx);
        board.write_all(b"after cancellation").await.unwrap();
        let mut bytes = [0; 18];
        deadline(server.read_exact(&mut bytes)).await.unwrap();
        assert_eq!(&bytes, b"after cancellation");
    }
}
