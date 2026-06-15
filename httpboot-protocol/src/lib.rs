#![cfg_attr(not(feature = "std"), no_std)]

pub const SERIAL_PROTOCOL_VERSION: u16 = 1;
pub const SERIAL_READY_PREFIX: &str = "AXLOADER READY ";
pub const SERIAL_BOOT_PREFIX: &str = "AXLOADER BOOT ";

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum BootArch {
    X86_64,
    Aarch64,
    Loongarch64,
    Riscv64,
    Other,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ImageFormat {
    Elf64,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerialReadyMessage<'a> {
    pub protocol_version: u16,
    pub board: &'a str,
    pub arch: BootArch,
    pub loader_version: Option<&'a str>,
}

#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SerialBootOfferMessage<'a> {
    pub protocol_version: u16,
    pub boot_id: &'a str,
    pub kernel_url: &'a str,
    pub kernel_size: u64,
    pub image_format: ImageFormat,
    pub arch: BootArch,
    pub entry_symbol: Option<&'a str>,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelPublishResponse {
    pub boot_id: String,
    pub kernel_url: String,
    pub kernel_size: u64,
    pub kernel_sha256: Option<String>,
}

#[cfg(all(feature = "std", feature = "serde"))]
#[derive(Debug)]
pub enum SerialMessageError {
    InvalidPrefix(&'static str),
    Json(serde_json::Error),
}

#[cfg(all(feature = "std", feature = "serde"))]
impl core::fmt::Display for SerialMessageError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidPrefix(prefix) => {
                write!(
                    f,
                    "serial line does not start with expected prefix `{prefix}`"
                )
            }
            Self::Json(err) => write!(f, "failed to parse serial message JSON: {err}"),
        }
    }
}

#[cfg(all(feature = "std", feature = "serde"))]
impl std::error::Error for SerialMessageError {}

#[cfg(all(feature = "std", feature = "serde"))]
impl From<serde_json::Error> for SerialMessageError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}

#[cfg(all(feature = "std", feature = "serde"))]
pub fn render_serial_ready(message: &SerialReadyMessage<'_>) -> Result<String, serde_json::Error> {
    Ok(format!(
        "{SERIAL_READY_PREFIX}{}",
        serde_json::to_string(message)?
    ))
}

#[cfg(all(feature = "std", feature = "serde"))]
pub fn parse_serial_ready(line: &str) -> Result<SerialReadyMessage<'_>, SerialMessageError> {
    let body = line
        .trim()
        .strip_prefix(SERIAL_READY_PREFIX)
        .ok_or(SerialMessageError::InvalidPrefix(SERIAL_READY_PREFIX))?;
    Ok(serde_json::from_str(body)?)
}

#[cfg(all(feature = "std", feature = "serde"))]
pub fn render_serial_boot_offer(
    message: &SerialBootOfferMessage<'_>,
) -> Result<String, serde_json::Error> {
    Ok(format!(
        "{SERIAL_BOOT_PREFIX}{}",
        serde_json::to_string(message)?
    ))
}

#[cfg(all(feature = "std", feature = "serde"))]
pub fn parse_serial_boot_offer(
    line: &str,
) -> Result<SerialBootOfferMessage<'_>, SerialMessageError> {
    let body = line
        .trim()
        .strip_prefix(SERIAL_BOOT_PREFIX)
        .ok_or(SerialMessageError::InvalidPrefix(SERIAL_BOOT_PREFIX))?;
    Ok(serde_json::from_str(body)?)
}

#[cfg(test)]
mod tests {
    use super::{
        BootArch, ImageFormat, SERIAL_BOOT_PREFIX, SERIAL_PROTOCOL_VERSION, SERIAL_READY_PREFIX,
        SerialBootOfferMessage, SerialReadyMessage, parse_serial_boot_offer, parse_serial_ready,
        render_serial_boot_offer, render_serial_ready,
    };

    #[test]
    fn serializes_loader_control_messages() {
        let offer = SerialBootOfferMessage {
            protocol_version: SERIAL_PROTOCOL_VERSION,
            boot_id: "boot-1",
            kernel_url: "http://127.0.0.1/kernel.elf",
            kernel_size: 4096,
            image_format: ImageFormat::Elf64,
            arch: BootArch::X86_64,
            entry_symbol: Some("httpboot_entry"),
        };
        let value = serde_json::to_value(&offer).unwrap();
        assert_eq!(value["image_format"], "elf64");
    }

    #[test]
    fn renders_and_parses_serial_ready_message() {
        let ready = SerialReadyMessage {
            protocol_version: SERIAL_PROTOCOL_VERSION,
            board: "asus-nuc15crh",
            arch: BootArch::X86_64,
            loader_version: Some("axloader"),
        };

        let line = render_serial_ready(&ready).unwrap();

        assert!(line.starts_with(SERIAL_READY_PREFIX));
        assert_eq!(parse_serial_ready(&line).unwrap(), ready);
    }

    #[test]
    fn renders_and_parses_serial_boot_offer_message() {
        let offer = SerialBootOfferMessage {
            protocol_version: SERIAL_PROTOCOL_VERSION,
            boot_id: "boot-1",
            kernel_url: "http://10.3.10.192:2999/boot/kernel.elf",
            kernel_size: 4096,
            image_format: ImageFormat::Elf64,
            arch: BootArch::X86_64,
            entry_symbol: Some("httpboot_entry"),
        };

        let line = render_serial_boot_offer(&offer).unwrap();

        assert!(line.starts_with(SERIAL_BOOT_PREFIX));
        assert_eq!(parse_serial_boot_offer(&line).unwrap(), offer);
    }
}
