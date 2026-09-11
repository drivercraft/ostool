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
use serialport::{ClearBuffer, SerialPort, TTYPort};
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

pub(super) struct PhysicalSerial {
    pub(super) writer: tokio_serial::SerialStream,
    receive: Arc<ReceiveBuffer>,
    reader: Option<JoinHandle<()>>,
}

impl PhysicalSerial {
    pub(super) fn new(mut port: TTYPort) -> io::Result<Self> {
        port.set_timeout(READ_TIMEOUT)?;
        port.clear(ClearBuffer::Input)?;
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
        })
    }

    pub(super) fn stop_reader(&mut self) {
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
}

impl Drop for PhysicalSerial {
    fn drop(&mut self) {
        self.stop_reader();
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
