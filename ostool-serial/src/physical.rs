//! Physical RX keeps draining even when the HTTP/WebSocket executor is busy.

use std::{
    collections::VecDeque,
    io::{self, Read},
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    task::{Context, Poll},
    thread::JoinHandle,
    time::Duration,
};

use futures_util::task::AtomicWaker;
use serialport::{SerialPort, TTYPort};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

const CHUNK_SIZE: usize = 4096;
const QUEUE_BYTES: usize = CHUNK_SIZE * 64;
const READ_TIMEOUT: Duration = Duration::from_millis(20);

#[derive(Default)]
struct ReceiveState {
    bytes: VecDeque<u8>,
    closed: bool,
    error: Option<io::Error>,
}

#[derive(Default)]
struct ReceiveBuffer {
    state: Mutex<ReceiveState>,
    waker: AtomicWaker,
    stopped: AtomicBool,
}

/// One blocking native reader feeds an async console without waiting for network IO.
/// Drop joins that reader before restoring the original line settings and closing IO.
pub struct PhysicalSerial {
    pub(crate) writer: tokio_serial::SerialStream,
    receive: Arc<ReceiveBuffer>,
    reader: Option<JoinHandle<()>>,
    restore: Option<super::native::RestoreSettings>,
}

impl PhysicalSerial {
    /// Start the sole receive worker without clearing any already buffered input.
    pub fn new(mut port: TTYPort) -> io::Result<Self> {
        port.set_timeout(READ_TIMEOUT)?;

        let writer = port.try_clone_native()?.try_into()?;
        let receive = Arc::new(ReceiveBuffer::default());
        let shared = receive.clone();
        let reader = std::thread::Builder::new()
            .name("serial-rx".into())
            .spawn(move || {
                let mut buffer = [0; CHUNK_SIZE];
                let error = loop {
                    if shared.stopped.load(Ordering::Acquire) {
                        break None;
                    }
                    match port.read(&mut buffer) {
                        Ok(0) => break None,
                        Ok(size) => {
                            let mut state = shared.state.lock().expect("serial RX mutex poisoned");
                            if size > QUEUE_BYTES - state.bytes.len() {
                                break Some(io::Error::other("physical serial RX buffer full"));
                            }
                            state.bytes.extend(&buffer[..size]);
                            drop(state);
                            shared.waker.wake();
                        }
                        Err(error)
                            if matches!(
                                error.kind(),
                                io::ErrorKind::TimedOut
                                    | io::ErrorKind::WouldBlock
                                    | io::ErrorKind::Interrupted
                            ) => {}
                        Err(error) => break Some(error),
                    }
                };
                let mut state = shared.state.lock().expect("serial RX mutex poisoned");
                state.error = error;
                state.closed = true;
                drop(state);
                shared.waker.wake();
            })?;
        Ok(Self {
            writer,
            receive,
            reader: Some(reader),
            restore: None,
        })
    }

    /// Stop and join the reader; this can block for its finite native read timeout.
    pub fn stop_reader(&mut self) {
        self.receive.stopped.store(true, Ordering::Release);
        if let Some(reader) = self.reader.take() {
            // Never wait for queue capacity or network I/O. The worker's only
            // blocking operation is the finite serial read timeout. Join before
            // releasing the lease, including cancellation, to prevent an old
            // reader consuming bytes from the next owner's session.
            if reader.join().is_err() {
                log::error!("physical serial receive worker panicked");
            }
        }
    }
    /// Inspect output queued in the native driver without performing tcdrain.
    pub fn bytes_to_write(&self) -> io::Result<u32> {
        self.writer.bytes_to_write().map_err(io::Error::other)
    }
    /// Explicit manual-console cleanup; discovery never calls this operation.
    pub fn clear(&self, buffer: serialport::ClearBuffer) -> io::Result<()> {
        self.writer.clear(buffer).map_err(io::Error::other)
    }
    #[cfg(test)]
    fn test_receive_snapshotter(
        &self,
    ) -> impl Fn() -> (Vec<u8>, bool, bool) + Send + Sync + 'static {
        let receive = self.receive.clone();
        move || {
            let state = receive.state.lock().expect("serial RX mutex poisoned");
            (
                state.bytes.iter().copied().collect(),
                state.closed,
                state.error.is_some(),
            )
        }
    }
}

impl Drop for PhysicalSerial {
    fn drop(&mut self) {
        self.stop_reader();
        drop(self.restore.take());
    }
}

impl AsyncRead for PhysicalSerial {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        output: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if output.remaining() == 0 {
            return Poll::Ready(Ok(()));
        }
        // Register before inspecting state; producer publication and wake cannot
        // be lost between an empty check and returning Pending.
        self.receive.waker.register(cx.waker());
        let mut state = self.receive.state.lock().expect("serial RX mutex poisoned");
        if !state.bytes.is_empty() {
            let size = output.remaining().min(state.bytes.len());
            let (first, second) = state.bytes.as_slices();
            let first_size = size.min(first.len());
            output.put_slice(&first[..first_size]);
            output.put_slice(&second[..size - first_size]);
            state.bytes.drain(..size);
            return Poll::Ready(Ok(()));
        }
        if state.closed {
            return Poll::Ready(state.error.take().map_or(Ok(()), Err));
        }
        Poll::Pending
    }
}

impl AsyncWrite for PhysicalSerial {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.writer).poll_write(cx, bytes)
    }
    fn poll_flush(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        // Unbuffered writes: never invoke tcdrain on the network executor.
        Poll::Ready(Ok(()))
    }
    fn poll_shutdown(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<io::Result<()>> {
        Poll::Ready(Ok(()))
    }
}

impl crate::SerialIo for PhysicalSerial {
    fn configure(
        &mut self,
        parameters: httpboot_protocol::SerialParameters,
    ) -> std::io::Result<()> {
        super::native::configure(&mut self.writer, parameters)
    }
}

impl PhysicalSerial {
    pub(crate) fn restoring(
        port: TTYPort,
        restore: super::native::RestoreSettings,
    ) -> io::Result<Self> {
        let mut stream = Self::new(port)?;
        stream.restore = Some(restore);
        Ok(stream)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tokio::io::AsyncReadExt;

    #[tokio::test]
    async fn receive_overflow_preserves_all_accepted_bytes_before_error() {
        let (mut board, port) = TTYPort::pair().unwrap();
        board.set_timeout(Duration::from_millis(200)).unwrap();
        let mut serial = PhysicalSerial::new(port).unwrap();
        let snapshot = serial.test_receive_snapshotter();
        let writer = std::thread::spawn(move || {
            let _ = (0..4096).try_for_each(|_| board.write_all(&[0x5a; 128]));
            board
        });
        let _board = writer.join().unwrap();
        let (expected, closed, error) = snapshot();
        assert!(closed && error && !expected.is_empty());
        assert!(expected.len() <= QUEUE_BYTES);
        let mut actual = Vec::new();
        let error = tokio::time::timeout(Duration::from_secs(2), serial.read_to_end(&mut actual))
            .await
            .unwrap()
            .unwrap_err();
        assert!(error.to_string().contains("physical serial RX buffer full"));
        assert_eq!(actual, expected);
    }
}
