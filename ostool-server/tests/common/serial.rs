use httpboot_protocol::SerialParameters;
use ostool_serial::{BackendFuture, NativeBackend, PortLocator, SerialBackend, SerialIo};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, ReadBuf},
    sync::mpsc,
};

pub struct PtyBackend {
    pub ports: Vec<PortLocator>,
    pub configured: mpsc::UnboundedSender<(String, SerialParameters)>,
}
impl SerialBackend for PtyBackend {
    fn ports(&self) -> BackendFuture<'_, Vec<PortLocator>> {
        Box::pin(async { Ok(self.ports.clone()) })
    }
    fn open(
        &self,
        port: PortLocator,
        parameters: SerialParameters,
    ) -> BackendFuture<'_, Box<dyn SerialIo>> {
        Box::pin(async move {
            let io = NativeBackend.open(port.clone(), parameters).await?;
            self.configured
                .send((port.name.clone(), parameters))
                .unwrap();
            Ok(Box::new(ObservedIo {
                io,
                port: port.name,
                configured: self.configured.clone(),
            }) as Box<dyn SerialIo>)
        })
    }
}
struct ObservedIo {
    io: Box<dyn SerialIo>,
    port: String,
    configured: mpsc::UnboundedSender<(String, SerialParameters)>,
}
impl SerialIo for ObservedIo {
    fn configure(&mut self, p: SerialParameters) -> io::Result<()> {
        self.io.configure(p)?;
        let _ = self.configured.send((self.port.clone(), p));
        Ok(())
    }
}
impl AsyncRead for ObservedIo {
    fn poll_read(
        mut self: Pin<&mut Self>,
        c: &mut Context<'_>,
        b: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_read(c, b)
    }
}
impl AsyncWrite for ObservedIo {
    fn poll_write(
        mut self: Pin<&mut Self>,
        c: &mut Context<'_>,
        b: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.io).poll_write(c, b)
    }
    fn poll_flush(mut self: Pin<&mut Self>, c: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_flush(c)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, c: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.io).poll_shutdown(c)
    }
}
