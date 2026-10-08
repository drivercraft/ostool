//! A single on-demand discovery actor with transferable serial leases.
use httpboot_protocol::SerialFrameDecoder;
pub use httpboot_protocol::{
    MacAddress, SerialFlowControl, SerialParameters, SerialParity, SerialStopBits,
};
use std::{
    collections::VecDeque,
    future::Future,
    io,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::{mpsc, oneshot, watch},
    time::Instant,
};

mod manager;
mod native;
#[cfg(unix)]
mod physical;
pub use native::{NativeBackend, validate_host_parameters};
#[cfg(unix)]
pub use physical::PhysicalSerial;

pub type BackendFuture<'a, T> = Pin<Box<dyn Future<Output = io::Result<T>> + Send + 'a>>;
/// IO owned by exactly one candidate or session. Configuration never adds a reader.
pub trait SerialIo: AsyncRead + AsyncWrite + Unpin + Send {
    fn configure(&mut self, parameters: SerialParameters) -> io::Result<()>;
}
pub trait SerialBackend: Send + Sync + 'static {
    fn ports(&self) -> BackendFuture<'_, Vec<PortLocator>>;
    fn open(
        &self,
        port: PortLocator,
        parameters: SerialParameters,
    ) -> BackendFuture<'_, Box<dyn SerialIo>>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortLocator {
    pub name: String,
    pub aliases: Vec<String>,
    pub serial_number: Option<String>,
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortSelector {
    Path(String),
    SerialNumber(String),
}
impl PortLocator {
    pub fn matches(&self, selector: &PortSelector) -> bool {
        match selector {
            PortSelector::SerialNumber(sn) => self.serial_number.as_ref() == Some(sn),
            PortSelector::Path(path) => {
                self.name == *path
                    || self.aliases.contains(path)
                    || std::fs::canonicalize(path).is_ok_and(|p| p.to_string_lossy() == self.name)
            }
        }
    }
}

#[derive(Debug, Clone)]
pub struct BindRequest {
    pub owner: String,
    pub generation: u64,
    pub mac_address: MacAddress,
    pub boot_epoch: String,
    pub serial_id: String,
    pub parameters: SerialParameters,
    pub deadline: Instant,
}

#[derive(Debug, thiserror::Error)]
pub enum BindError {
    #[error("serial manager stopped")]
    Stopped,
    #[error("serial binding timed out")]
    Timeout,
    #[error("serial identity discovery failed: {0}")]
    DiscoveryFailed(String),
    #[error("serial resource is busy")]
    Busy,
    #[error("invalid serial identity or parameters")]
    InvalidRequest,
    #[error("serial operation failed: {0}")]
    Io(#[from] io::Error),
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct ManagerSnapshot {
    pub pending: usize,
    pub candidates: usize,
    pub leased: usize,
}

/// Cloneable command channel; one actor owns all discovery and memory locations.
#[derive(Clone)]
pub struct SerialManager {
    commands: mpsc::UnboundedSender<manager::Command>,
    snapshot: watch::Receiver<ManagerSnapshot>,
}
impl SerialManager {
    pub fn new(backend: Arc<dyn SerialBackend>) -> Self {
        manager::start(backend)
    }
    pub fn subscribe(&self) -> watch::Receiver<ManagerSnapshot> {
        self.snapshot.clone()
    }
    pub async fn bind(&self, request: BindRequest) -> Result<SerialLease, BindError> {
        if request.boot_epoch.is_empty()
            || !httpboot_protocol::valid_serial_id(&request.serial_id)
            || request.parameters.validate().is_err()
        {
            return Err(BindError::InvalidRequest);
        }
        let (reply, response) = oneshot::channel();
        self.commands
            .send(manager::Command::Bind(request, reply))
            .map_err(|_| BindError::Stopped)?;
        response.await.map_err(|_| BindError::Timeout)?
    }
    /// Update configured relay exclusions. Completion means conflicting candidate IO stopped.
    pub async fn exclude(&self, selectors: Vec<PortSelector>) -> Result<(), BindError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(manager::Command::Exclude(selectors, reply))
            .map_err(|_| BindError::Stopped)?;
        response.await.map_err(|_| BindError::Stopped)?
    }
    /// Reserve manual/relay IO before an external opener touches the device.
    pub async fn reserve(&self, port: PortLocator) -> Result<PortReservation, BindError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(manager::Command::Reserve(port, reply))
            .map_err(|_| BindError::Stopped)?;
        response.await.map_err(|_| BindError::Stopped)?
    }
    pub async fn wait_owner_released(&self, owner: &str) -> Result<(), BindError> {
        let (reply, response) = oneshot::channel();
        self.commands
            .send(manager::Command::WaitOwner(owner.into(), reply))
            .map_err(|_| BindError::Stopped)?;
        response.await.map_err(|_| BindError::Stopped)
    }
    pub fn invalidate(&self, mac: MacAddress) {
        let _ = self.commands.send(manager::Command::Invalidate(mac));
    }
}

/// Prevents discovery from opening external IO. Close that IO before dropping this guard.
pub struct PortReservation {
    port: String,
    token: u64,
    commands: mpsc::UnboundedSender<manager::Command>,
}
impl Drop for PortReservation {
    fn drop(&mut self) {
        let _ = self
            .commands
            .send(manager::Command::Release(self.port.clone(), self.token));
    }
}

/// Non-cloneable IO lease. Drop starts closing the reader before advertising availability.
/// Call `SerialManager::wait_owner_released` to await the actual asynchronous close.
pub struct SerialLease {
    stream: Option<Box<dyn SerialIo>>,
    prefix: VecDeque<u8>,
    locator: PortLocator,
    token: u64,
    mac: MacAddress,
    commands: mpsc::UnboundedSender<manager::Command>,
    decoder: SerialFrameDecoder,
    observed: Option<[u8; 32]>,
}
impl SerialLease {
    pub fn locator(&self) -> &PortLocator {
        &self.locator
    }
    /// Publish a location only after the device acknowledges this binding.
    pub fn confirm(&self) {
        let _ = self.commands.send(manager::Command::Confirm(
            self.mac,
            self.locator.clone(),
            self.token,
        ));
    }
    pub fn configure(&mut self, parameters: SerialParameters) -> io::Result<()> {
        self.decoder = SerialFrameDecoder::default();
        self.observed = None;
        self.stream
            .as_mut()
            .expect("live lease")
            .configure(parameters)
    }
    /// Only fresh bytes from the current reader can prove a subsequent boot.
    pub fn take_observed_id(&mut self) -> Option<[u8; 32]> {
        self.observed.take()
    }
}
impl Drop for SerialLease {
    fn drop(&mut self) {
        let stream = self.stream.take();
        let commands = self.commands.clone();
        let port = self.locator.name.clone();
        let token = self.token;
        let close = move || {
            drop(stream);
            let _ = commands.send(manager::Command::Release(port, token));
        };
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(close);
        } else {
            close();
        }
    }
}
impl AsyncRead for SerialLease {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        if !self.prefix.is_empty() {
            while out.remaining() > 0 {
                let Some(b) = self.prefix.pop_front() else {
                    break;
                };
                out.put_slice(&[b]);
            }
            return Poll::Ready(Ok(()));
        }
        let start = out.filled().len();
        let result = Pin::new(self.stream.as_mut().expect("live lease")).poll_read(cx, out);
        if let Poll::Ready(Ok(())) = &result {
            for b in &out.filled()[start..] {
                if let Some(id) = self.decoder.push(*b) {
                    self.observed = Some(id);
                }
            }
        }
        if matches!(&result, Poll::Ready(Err(_)))
            || matches!(&result, Poll::Ready(Ok(())) if out.filled().len() == start && out.remaining() > 0)
        {
            let _ = self.commands.send(manager::Command::Invalidate(self.mac));
        }
        result
    }
}
impl AsyncWrite for SerialLease {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        let result = Pin::new(self.stream.as_mut().expect("live lease")).poll_write(cx, bytes);
        if matches!(result, Poll::Ready(Err(_))) {
            let _ = self.commands.send(manager::Command::Invalidate(self.mac));
        }
        result
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let result = Pin::new(self.stream.as_mut().expect("live lease")).poll_flush(cx);
        if matches!(result, Poll::Ready(Err(_))) {
            let _ = self.commands.send(manager::Command::Invalidate(self.mac));
        }
        result
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(self.stream.as_mut().expect("live lease")).poll_shutdown(cx)
    }
}
