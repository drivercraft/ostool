use crate::{
    config::{PowerManagementConfig, SerialPortKey, SerialPortKeyKind},
    virtual_qemu::VirtualBoardManager,
};
use httpboot_protocol::SerialParameters;
use ostool_serial::{BackendFuture, NativeBackend, PortLocator, SerialBackend, SerialIo};
use std::{
    io,
    pin::Pin,
    task::{Context, Poll},
};
use tokio::io::{AsyncRead, AsyncWrite, DuplexStream, ReadBuf};

pub struct ServerSerialBackend {
    pub virtual_boards: VirtualBoardManager,
}
impl SerialBackend for ServerSerialBackend {
    fn ports(&self) -> BackendFuture<'_, Vec<PortLocator>> {
        Box::pin(async {
            let mut ports = NativeBackend.ports().await?;
            let metadata = tokio::task::spawn_blocking(super::discovery::list_serial_ports)
                .await
                .map_err(io::Error::other)?
                .map_err(io::Error::other)?;
            for record in metadata {
                let canonical = std::fs::canonicalize(&record.current_device_path)
                    .map(|p| p.to_string_lossy().into_owned())
                    .unwrap_or(record.current_device_path);
                if let Some(port) = ports.iter_mut().find(|p| p.name == canonical) {
                    if record.serial_number.is_some() {
                        port.serial_number = record.serial_number;
                    }
                    if let Some(alias) = record.usb_path
                        && !port.aliases.contains(&alias)
                    {
                        port.aliases.push(alias);
                    }
                }
            }
            ports.extend(
                self.virtual_boards
                    .snapshots()
                    .await
                    .into_iter()
                    .map(|d| virtual_locator(&d.id)),
            );
            Ok(ports)
        })
    }
    fn open(
        &self,
        port: PortLocator,
        parameters: SerialParameters,
    ) -> BackendFuture<'_, Box<dyn SerialIo>> {
        Box::pin(async move {
            if let Some(id) = port.name.strip_prefix("qemu:") {
                parameters.validate().map_err(io::Error::other)?;
                let io = self
                    .virtual_boards
                    .attach_serial(id)
                    .await
                    .map_err(io::Error::other)?;
                Ok(Box::new(VirtualSerial(io)) as Box<dyn SerialIo>)
            } else {
                NativeBackend.open(port, parameters).await
            }
        })
    }
}
pub fn virtual_locator(id: &str) -> PortLocator {
    PortLocator {
        name: format!("qemu:{id}"),
        aliases: Vec::new(),
        serial_number: None,
    }
}
pub fn locator_for_key(key: &SerialPortKey) -> anyhow::Result<PortLocator> {
    if key.kind == SerialPortKeyKind::Qemu {
        return Ok(virtual_locator(&key.value));
    }
    let resolved = super::discovery::resolve_serial_key(key)?;
    let canonical = std::fs::canonicalize(&resolved.current_device_path)?;
    let mut aliases = vec![key.value.clone(), resolved.current_device_path];
    aliases.extend(resolved.usb_path);
    Ok(PortLocator {
        name: canonical.to_string_lossy().into_owned(),
        aliases,
        serial_number: resolved.serial_number,
    })
}
pub fn relay_selector(power: &PowerManagementConfig) -> Option<ostool_serial::PortSelector> {
    let PowerManagementConfig::ZhongshengRelay(relay) = power else {
        return None;
    };
    Some(match relay.key.kind {
        SerialPortKeyKind::SerialNumber => {
            ostool_serial::PortSelector::SerialNumber(relay.key.value.clone())
        }
        _ => ostool_serial::PortSelector::Path(relay.key.value.clone()),
    })
}
struct VirtualSerial(DuplexStream);
impl SerialIo for VirtualSerial {
    fn configure(&mut self, parameters: SerialParameters) -> io::Result<()> {
        parameters.validate().map_err(io::Error::other)
    }
}
impl AsyncRead for VirtualSerial {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        out: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_read(cx, out)
    }
}
impl AsyncWrite for VirtualSerial {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.0).poll_write(cx, bytes)
    }
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_flush(cx)
    }
    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.0).poll_shutdown(cx)
    }
}
