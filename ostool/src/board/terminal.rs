use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    sync::{Arc, Condvar, Mutex},
    time::Duration,
};

use anyhow::Context as _;
use futures::{SinkExt, StreamExt};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::sterm::SerialTerm;

pub async fn run_serial_terminal(ws_url: reqwest::Url) -> anyhow::Result<()> {
    let (stream, _) = tokio_tungstenite::connect_async(ws_url.as_str())
        .await
        .with_context(|| format!("failed to connect serial websocket {}", ws_url))?;
    let (mut sink, mut stream) = stream.split();

    let bridge = Arc::new(ByteBridge::default());
    let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();

    let read_bridge = bridge.clone();
    let read_task = tokio::spawn(async move {
        while let Some(message) = stream.next().await {
            match message.context("serial websocket read failed")? {
                Message::Text(text) => read_bridge.push(text.as_str().as_bytes()),
                Message::Binary(bytes) => read_bridge.push(bytes.as_ref()),
                Message::Close(_) => {
                    read_bridge.close();
                    break;
                }
                Message::Ping(_) | Message::Pong(_) => {}
                _ => {}
            }
        }
        read_bridge.close();
        Ok::<(), anyhow::Error>(())
    });

    let write_task = tokio::spawn(async move {
        while let Some(bytes) = outbound_rx.recv().await {
            sink.send(Message::Binary(bytes.into()))
                .await
                .context("serial websocket write failed")?;
        }
        let _ = sink.send(Message::Close(None)).await;
        Ok::<(), anyhow::Error>(())
    });

    let reader = BridgeReader {
        bridge: bridge.clone(),
    };
    let writer = BridgeWriter {
        tx: outbound_tx.clone(),
    };
    let mut terminal =
        SerialTerm::new_with_byte_callback(Box::new(writer), Box::new(reader), |_handle, _byte| {});

    let run_result = terminal.run().await;
    drop(outbound_tx);

    read_task.abort();
    write_task.abort();

    if let Err(err) = read_task.await {
        if !err.is_cancelled() {
            log::debug!("serial websocket reader join error: {err}");
        }
    }
    if let Err(err) = write_task.await {
        if !err.is_cancelled() {
            log::debug!("serial websocket writer join error: {err}");
        }
    }

    run_result
}

#[derive(Debug, Default)]
struct ByteBridge {
    state: Mutex<BridgeState>,
    ready: Condvar,
}

#[derive(Debug, Default)]
struct BridgeState {
    buffer: VecDeque<u8>,
    closed: bool,
}

impl ByteBridge {
    fn push(&self, bytes: &[u8]) {
        let mut state = self.state.lock().unwrap();
        state.buffer.extend(bytes);
        self.ready.notify_all();
    }

    fn close(&self) {
        let mut state = self.state.lock().unwrap();
        state.closed = true;
        self.ready.notify_all();
    }

    fn read(&self, buf: &mut [u8]) -> io::Result<usize> {
        let mut state = self.state.lock().unwrap();
        while state.buffer.is_empty() && !state.closed {
            let (next_state, timeout) = self
                .ready
                .wait_timeout(state, Duration::from_millis(100))
                .unwrap();
            state = next_state;
            if timeout.timed_out() && state.buffer.is_empty() && !state.closed {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "waiting for websocket data timed out",
                ));
            }
        }

        if state.buffer.is_empty() && state.closed {
            return Ok(0);
        }

        let len = buf.len().min(state.buffer.len());
        for slot in buf.iter_mut().take(len) {
            *slot = state.buffer.pop_front().unwrap();
        }
        Ok(len)
    }
}

struct BridgeReader {
    bridge: Arc<ByteBridge>,
}

impl Read for BridgeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.bridge.read(buf)
    }
}

struct BridgeWriter {
    tx: mpsc::UnboundedSender<Vec<u8>>,
}

impl Write for BridgeWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tx
            .send(buf.to_vec())
            .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "websocket writer closed"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::{io::Read, time::Instant};

    use super::ByteBridge;

    #[test]
    fn bridge_reads_text_and_binary_bytes_in_order() {
        let bridge = ByteBridge::default();
        bridge.push(b"abc");
        bridge.push(&[0x00, 0xff]);
        bridge.close();

        let mut reader = super::BridgeReader {
            bridge: std::sync::Arc::new(bridge),
        };
        let mut buffer = [0u8; 5];
        let read = reader.read(&mut buffer).unwrap();
        assert_eq!(read, 5);
        assert_eq!(&buffer, b"abc\x00\xff");
    }

    #[test]
    fn bridge_read_times_out_when_no_data_arrives() {
        let bridge = ByteBridge::default();
        let mut reader = super::BridgeReader {
            bridge: std::sync::Arc::new(bridge),
        };
        let mut buffer = [0u8; 1];

        let start = Instant::now();
        let err = reader.read(&mut buffer).unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::TimedOut);
        assert!(start.elapsed() >= std::time::Duration::from_millis(90));
    }
}
