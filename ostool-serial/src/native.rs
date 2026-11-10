use crate::{BackendFuture, PortLocator, SerialBackend, SerialIo};
use httpboot_protocol::{SerialFlowControl, SerialParameters, SerialParity, SerialStopBits};
use serialport::{DataBits, FlowControl, Parity, SerialPort, StopBits};
use std::{io, path::PathBuf};

/// USB-UART drivers may report the nearest rate their clock dividers can produce.
const BAUD_TOLERANCE_PER_MILLE: u64 = 5;

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

/// Accept only integer readbacks within 0.5% of the requested rate.
fn baud_rate_matches(requested: u32, applied: u32) -> bool {
    requested != 0
        && applied != 0
        && u64::from(requested.abs_diff(applied)) * 1_000
            <= u64::from(requested) * BAUD_TOLERANCE_PER_MILLE
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
    let applied_baud = port.baud_rate().map_err(io::Error::other)?;
    if !baud_rate_matches(baud, applied_baud) {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!("serial driver baud rate mismatch: requested {baud}, applied {applied_baud}"),
        ));
    }
    if applied_baud != baud {
        log::warn!("serial driver quantized baud rate: requested={baud}, applied={applied_baud}");
    }
    let applied_bits = port.data_bits().map_err(io::Error::other)?;
    if applied_bits != bits {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "serial driver data bits mismatch: requested {bits:?}, applied {applied_bits:?}"
            ),
        ));
    }
    let applied_parity = port.parity().map_err(io::Error::other)?;
    if applied_parity != parity {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "serial driver parity mismatch: requested {parity:?}, applied {applied_parity:?}"
            ),
        ));
    }
    let applied_stops = port.stop_bits().map_err(io::Error::other)?;
    if applied_stops != stops {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "serial driver stop bits mismatch: requested {stops:?}, applied {applied_stops:?}"
            ),
        ));
    }
    let applied_flow = port.flow_control().map_err(io::Error::other)?;
    if applied_flow != flow {
        return Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "serial driver flow control mismatch: requested {flow:?}, applied {applied_flow:?}"
            ),
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

#[cfg(test)]
mod tests {
    use super::*;
    use serialport::ClearBuffer;
    use std::{
        io::{Read, Write},
        time::Duration,
    };

    #[derive(Clone, Copy, PartialEq, Eq)]
    enum IgnoredSetting {
        DataBits,
        Parity,
        StopBits,
        FlowControl,
    }

    #[derive(Clone)]
    struct FakeSerialPort {
        baud_rate: u32,
        data_bits: DataBits,
        parity: Parity,
        stop_bits: StopBits,
        flow_control: FlowControl,
        baud_requests: Vec<u32>,
        ignored_setting: Option<IgnoredSetting>,
    }

    impl FakeSerialPort {
        fn new(baud_rate: u32) -> Self {
            Self {
                baud_rate,
                data_bits: DataBits::Eight,
                parity: Parity::None,
                stop_bits: StopBits::One,
                flow_control: FlowControl::None,
                baud_requests: Vec::new(),
                ignored_setting: None,
            }
        }
    }

    impl Read for FakeSerialPort {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Ok(0)
        }
    }

    impl Write for FakeSerialPort {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            Ok(bytes.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SerialPort for FakeSerialPort {
        fn name(&self) -> Option<String> {
            Some("fake".into())
        }

        fn baud_rate(&self) -> serialport::Result<u32> {
            Ok(self.baud_rate)
        }

        fn data_bits(&self) -> serialport::Result<DataBits> {
            Ok(self.data_bits)
        }

        fn flow_control(&self) -> serialport::Result<FlowControl> {
            Ok(self.flow_control)
        }

        fn parity(&self) -> serialport::Result<Parity> {
            Ok(self.parity)
        }

        fn stop_bits(&self) -> serialport::Result<StopBits> {
            Ok(self.stop_bits)
        }

        fn timeout(&self) -> Duration {
            Duration::ZERO
        }

        fn set_baud_rate(&mut self, baud_rate: u32) -> serialport::Result<()> {
            self.baud_requests.push(baud_rate);
            Ok(())
        }

        fn set_data_bits(&mut self, data_bits: DataBits) -> serialport::Result<()> {
            if self.ignored_setting != Some(IgnoredSetting::DataBits) {
                self.data_bits = data_bits;
            }
            Ok(())
        }

        fn set_flow_control(&mut self, flow_control: FlowControl) -> serialport::Result<()> {
            if self.ignored_setting != Some(IgnoredSetting::FlowControl) {
                self.flow_control = flow_control;
            }
            Ok(())
        }

        fn set_parity(&mut self, parity: Parity) -> serialport::Result<()> {
            if self.ignored_setting != Some(IgnoredSetting::Parity) {
                self.parity = parity;
            }
            Ok(())
        }

        fn set_stop_bits(&mut self, stop_bits: StopBits) -> serialport::Result<()> {
            if self.ignored_setting != Some(IgnoredSetting::StopBits) {
                self.stop_bits = stop_bits;
            }
            Ok(())
        }

        fn set_timeout(&mut self, _: Duration) -> serialport::Result<()> {
            Ok(())
        }

        fn write_request_to_send(&mut self, _: bool) -> serialport::Result<()> {
            Ok(())
        }

        fn write_data_terminal_ready(&mut self, _: bool) -> serialport::Result<()> {
            Ok(())
        }

        fn read_clear_to_send(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }

        fn read_data_set_ready(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }

        fn read_ring_indicator(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }

        fn read_carrier_detect(&mut self) -> serialport::Result<bool> {
            Ok(false)
        }

        fn bytes_to_read(&self) -> serialport::Result<u32> {
            Ok(0)
        }

        fn bytes_to_write(&self) -> serialport::Result<u32> {
            Ok(0)
        }

        fn clear(&self, _: ClearBuffer) -> serialport::Result<()> {
            Ok(())
        }

        fn try_clone(&self) -> serialport::Result<Box<dyn SerialPort>> {
            Ok(Box::new(self.clone()))
        }

        fn set_break(&self) -> serialport::Result<()> {
            Ok(())
        }

        fn clear_break(&self) -> serialport::Result<()> {
            Ok(())
        }
    }

    fn parameters(baud_rate: u64) -> SerialParameters {
        SerialParameters {
            baud_rate,
            ..SerialParameters::default()
        }
    }

    #[test]
    fn baud_tolerance_accepts_cp2102_quantization() {
        assert!(baud_rate_matches(115_740, 115_200));
        assert!(baud_rate_matches(115_740, 115_740));
    }

    #[test]
    fn baud_tolerance_accepts_exact_half_percent_boundary() {
        assert!(baud_rate_matches(1_000, 995));
        assert!(baud_rate_matches(1_000, 1_005));
        assert!(baud_rate_matches(2_000, 1_990));
        assert!(baud_rate_matches(2_000, 2_010));
    }

    #[test]
    fn baud_tolerance_rejects_over_half_percent() {
        assert!(!baud_rate_matches(1_000, 994));
        assert!(!baud_rate_matches(1_000, 1_006));
        assert!(!baud_rate_matches(115_740, 117_000));
        assert!(!baud_rate_matches(2_133, 2_400));
    }

    #[test]
    fn baud_tolerance_rejects_zero_and_handles_u32_extremes() {
        assert!(!baud_rate_matches(0, 0));
        assert!(!baud_rate_matches(0, 1));
        assert!(!baud_rate_matches(1, 0));
        assert!(baud_rate_matches(u32::MAX, u32::MAX));
        assert!(!baud_rate_matches(u32::MAX, 0));
    }

    #[test]
    fn configure_accepts_quantized_readback_without_changing_requested_baud() {
        let mut port = FakeSerialPort::new(115_200);
        let requested = parameters(115_740);

        configure(&mut port, requested).unwrap();

        assert_eq!(port.baud_requests, [115_740]);
        assert_eq!(port.baud_rate, 115_200);
        assert_eq!(requested.baud_rate, 115_740);
    }

    #[test]
    fn configure_rejects_unrelated_baud_readback_with_diagnostics() {
        let mut port = FakeSerialPort::new(117_000);

        let error = configure(&mut port, parameters(115_740)).unwrap_err();

        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
        assert!(error.to_string().contains("requested 115740"));
        assert!(error.to_string().contains("applied 117000"));
    }

    #[test]
    fn configure_rejects_each_non_baud_mismatch() {
        let cases = [
            (
                "data bits",
                FakeSerialPort {
                    data_bits: DataBits::Seven,
                    ignored_setting: Some(IgnoredSetting::DataBits),
                    ..FakeSerialPort::new(115_200)
                },
            ),
            (
                "parity",
                FakeSerialPort {
                    parity: Parity::Even,
                    ignored_setting: Some(IgnoredSetting::Parity),
                    ..FakeSerialPort::new(115_200)
                },
            ),
            (
                "stop bits",
                FakeSerialPort {
                    stop_bits: StopBits::Two,
                    ignored_setting: Some(IgnoredSetting::StopBits),
                    ..FakeSerialPort::new(115_200)
                },
            ),
            (
                "flow control",
                FakeSerialPort {
                    flow_control: FlowControl::Hardware,
                    ignored_setting: Some(IgnoredSetting::FlowControl),
                    ..FakeSerialPort::new(115_200)
                },
            ),
        ];

        for (parameter, mut port) in cases {
            let error = configure(&mut port, parameters(115_740)).unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::Unsupported);
            assert!(
                error.to_string().contains(parameter),
                "error did not identify {parameter}: {error}"
            );
        }
    }

    #[test]
    fn configure_rejects_zero_and_u32_overflow_baud_rates() {
        let mut port = FakeSerialPort::new(115_200);
        assert!(configure(&mut port, parameters(0)).is_err());
        assert!(configure(&mut port, parameters(u64::from(u32::MAX) + 1)).is_err());
        assert!(port.baud_requests.is_empty());
    }
}
