use crate::{BackendFuture, PortLocator, SerialBackend, SerialIo};
use httpboot_protocol::{SerialFlowControl, SerialParameters, SerialParity, SerialStopBits};
use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};
use std::{io, path::PathBuf};

#[derive(Default)]
pub struct NativeBackend;

impl SerialBackend for NativeBackend {
    fn ports(&self) -> BackendFuture<'_, Vec<PortLocator>> {
        Box::pin(async {
            tokio::task::spawn_blocking(enumerate)
                .await
                .map_err(io::Error::other)?
        })
    }
    fn open(
        &self,
        port: PortLocator,
        parameters: SerialParameters,
    ) -> BackendFuture<'_, Box<dyn SerialIo>> {
        Box::pin(async move {
            tokio::task::spawn_blocking(move || open(&port.name, parameters))
                .await
                .map_err(io::Error::other)?
        })
    }
}

fn enumerate() -> io::Result<Vec<PortLocator>> {
    let mut ports = serialport::available_ports()
        .map_err(io::Error::other)?
        .into_iter()
        .map(|p| {
            let serial_number = match p.port_type {
                serialport::SerialPortType::UsbPort(usb) => usb.serial_number,
                _ => None,
            };
            let canonical =
                std::fs::canonicalize(&p.port_name).unwrap_or_else(|_| PathBuf::from(&p.port_name));
            PortLocator {
                name: canonical.to_string_lossy().into_owned(),
                aliases: vec![p.port_name],
                serial_number,
            }
        })
        .collect::<Vec<_>>();
    if let Ok(entries) = std::fs::read_dir("/dev/serial/by-path") {
        for entry in entries.flatten() {
            if let Ok(path) = std::fs::canonicalize(entry.path())
                && let Some(port) = ports.iter_mut().find(|p| p.name == path.to_string_lossy())
            {
                port.aliases
                    .push(entry.path().to_string_lossy().into_owned());
            }
        }
    }
    ports.sort_by(|a, b| a.name.cmp(&b.name));
    ports.dedup_by(|a, b| a.name == b.name);
    Ok(ports)
}

/// Reject line settings the portable host driver cannot represent before opening any port.
pub fn validate_host_parameters(p: SerialParameters) -> io::Result<()> {
    p.validate().map_err(io::Error::other)?;
    u32::try_from(p.baud_rate).map_err(io::Error::other)?;
    if matches!(p.parity, SerialParity::Mark | SerialParity::Space)
        || p.stop_bits == SerialStopBits::OnePointFive
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "host serial backend does not support reported parity or stop bits",
        ));
    }
    Ok(())
}

pub(crate) fn configure(port: &mut dyn SerialPort, p: SerialParameters) -> io::Result<()> {
    validate_host_parameters(p)?;
    let baud = u32::try_from(p.baud_rate).map_err(io::Error::other)?;
    let parity = match p.parity {
        SerialParity::None => Parity::None,
        SerialParity::Odd => Parity::Odd,
        SerialParity::Even => Parity::Even,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "host does not support mark/space parity",
            ));
        }
    };
    let stops = match p.stop_bits {
        SerialStopBits::One => StopBits::One,
        SerialStopBits::Two => StopBits::Two,
        _ => {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "host does not support 1.5 stop bits",
            ));
        }
    };
    port.set_baud_rate(baud).map_err(io::Error::other)?;
    let bits = if p.data_bits == 7 {
        DataBits::Seven
    } else {
        DataBits::Eight
    };
    port.set_data_bits(bits).map_err(io::Error::other)?;
    port.set_parity(parity).map_err(io::Error::other)?;
    port.set_stop_bits(stops).map_err(io::Error::other)?;
    let flow = match p.flow_control {
        SerialFlowControl::None => FlowControl::None,
        SerialFlowControl::RtsCts => FlowControl::Hardware,
    };
    port.set_flow_control(flow).map_err(io::Error::other)?;
    if port.baud_rate().map_err(io::Error::other)? != baud
        || port.data_bits().map_err(io::Error::other)? != bits
        || port.parity().map_err(io::Error::other)? != parity
        || port.stop_bits().map_err(io::Error::other)? != stops
        || port.flow_control().map_err(io::Error::other)? != flow
    {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "serial driver did not apply reported line parameters",
        ));
    }
    Ok(())
}

#[cfg(unix)]
pub(crate) struct RestoreSettings {
    file: std::fs::File,
    #[cfg(not(target_os = "linux"))]
    settings: nix::sys::termios::Termios,
    #[cfg(target_os = "linux")]
    settings: libc::termios2,
}
#[cfg(unix)]
impl Drop for RestoreSettings {
    fn drop(&mut self) {
        #[cfg(not(target_os = "linux"))]
        let result = nix::sys::termios::tcsetattr(
            &self.file,
            nix::sys::termios::SetArg::TCSANOW,
            &self.settings,
        )
        .map_err(io::Error::other);
        #[cfg(target_os = "linux")]
        let result = {
            use std::os::fd::AsRawFd;
            // SAFETY: the duplicate descriptor remains live until Drop finishes;
            // TCSETS2 reads a complete captured termios2, retaining no pointer.
            if unsafe { libc::ioctl(self.file.as_raw_fd(), libc::TCSETS2, &self.settings) } < 0 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        };
        if let Err(error) = result {
            log::warn!("failed to restore serial settings: {error}");
        }
    }
}

#[cfg(unix)]
fn open(name: &str, parameters: SerialParameters) -> io::Result<Box<dyn SerialIo>> {
    use std::os::{
        fd::{AsRawFd, FromRawFd, IntoRawFd},
        unix::fs::OpenOptionsExt,
    };
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
        .open(name)?;
    let original = nix::sys::termios::tcgetattr(&file).map_err(io::Error::other)?;
    #[cfg(target_os = "linux")]
    let settings = {
        let mut settings = std::mem::MaybeUninit::<libc::termios2>::uninit();
        // SAFETY: TCGETS2 initializes the entire correctly aligned object on success;
        // file is live and the kernel retains no pointer after ioctl returns.
        if unsafe { libc::ioctl(file.as_raw_fd(), libc::TCGETS2, settings.as_mut_ptr()) } < 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful TCGETS2 initialized every field.
        unsafe { settings.assume_init() }
    };
    // SAFETY: file owns a live tty descriptor. Nonblocking flock has no retained pointer.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let restore = RestoreSettings {
        file: file.try_clone()?,
        #[cfg(not(target_os = "linux"))]
        settings: original.clone(),
        #[cfg(target_os = "linux")]
        settings,
    };
    // SAFETY: ownership of the validated live tty descriptor moves exactly once into TTYPort.
    let mut port = unsafe { serialport::TTYPort::from_raw_fd(file.into_raw_fd()) };
    port.set_exclusive(true).map_err(io::Error::other)?;
    let mut raw = original;
    nix::sys::termios::cfmakeraw(&mut raw);
    raw.control_flags
        .insert(nix::sys::termios::ControlFlags::CLOCAL | nix::sys::termios::ControlFlags::CREAD);
    nix::sys::termios::tcsetattr(&restore.file, nix::sys::termios::SetArg::TCSANOW, &raw)
        .map_err(io::Error::other)?;
    configure(&mut port, parameters)?;
    Ok(Box::new(crate::physical::PhysicalSerial::restoring(
        port, restore,
    )?))
}

#[cfg(not(unix))]
fn open(name: &str, parameters: SerialParameters) -> io::Result<Box<dyn SerialIo>> {
    use tokio_serial::SerialPortBuilderExt;
    let baud = u32::try_from(parameters.baud_rate).map_err(io::Error::other)?;
    let mut port = tokio_serial::new(name, baud)
        .open_native_async()
        .map_err(io::Error::other)?;
    configure(&mut port, parameters)?;
    Ok(Box::new(port))
}
#[cfg(not(unix))]
impl SerialIo for tokio_serial::SerialStream {
    fn configure(&mut self, parameters: SerialParameters) -> io::Result<()> {
        configure(self, parameters)
    }
}
