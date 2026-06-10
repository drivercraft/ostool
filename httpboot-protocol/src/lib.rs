#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "std")]
use std::vec::Vec;

pub const DISCOVERY_PROTOCOL_VERSION: u16 = 1;
pub const DISCOVERY_ADVERTISE_TYPE: &str = "ostool_httpboot_advertise";
pub const DISCOVERY_SOLICIT_TYPE: &str = "ostool_httpboot_solicit";
pub const SERIAL_PROTOCOL_VERSION: u16 = 1;
pub const SERIAL_READY_PREFIX: &str = "AXLOADER READY ";
pub const SERIAL_BOOT_PREFIX: &str = "AXLOADER BOOT ";

#[cfg(feature = "std")]
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

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ImageFormat {
    Elf64,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveryAdvertise {
    pub r#type: String,
    pub version: u16,
    pub server_id: String,
    pub base_url: String,
    pub discovery_port: u16,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoverySolicit {
    pub r#type: String,
    pub version: u16,
    pub arch: BootArch,
    pub board: Option<String>,
    pub mac: String,
    pub nonce: String,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderCapabilities {
    pub image_formats: Vec<ImageFormat>,
    pub range_get: bool,
    pub sha256: bool,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderHelloRequest {
    pub protocol_version: u16,
    pub nonce: String,
    pub arch: BootArch,
    pub board: Option<String>,
    pub mac: String,
    pub firmware_vendor: Option<String>,
    pub loader_version: Option<String>,
    pub capabilities: LoaderCapabilities,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderHelloResponse {
    pub loader_id: String,
    pub board_id: Option<String>,
    pub board_type: Option<String>,
    pub poll_url: String,
    pub wait_after_ms: u64,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum BootOfferState {
    Waiting,
    Ready,
    Error,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootOfferResponse {
    pub state: BootOfferState,
    pub wait_after_ms: Option<u64>,
    pub boot_id: Option<String>,
    pub kernel_url: Option<String>,
    pub kernel_size: Option<u64>,
    pub kernel_sha256: Option<String>,
    pub image_format: Option<ImageFormat>,
    pub arch: Option<BootArch>,
    pub entry_symbol: Option<String>,
    pub message: Option<String>,
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

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialReadyMessage {
    pub protocol_version: u16,
    pub board: String,
    pub arch: BootArch,
    pub loader_version: Option<String>,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SerialBootOfferMessage {
    pub protocol_version: u16,
    pub boot_id: String,
    pub kernel_url: String,
    pub kernel_size: u64,
    pub image_format: ImageFormat,
    pub arch: BootArch,
    pub entry_symbol: Option<String>,
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
pub fn render_serial_ready(message: &SerialReadyMessage) -> Result<String, serde_json::Error> {
    Ok(format!(
        "{SERIAL_READY_PREFIX}{}",
        serde_json::to_string(message)?
    ))
}

#[cfg(all(feature = "std", feature = "serde"))]
pub fn parse_serial_ready(line: &str) -> Result<SerialReadyMessage, SerialMessageError> {
    let body = line
        .trim()
        .strip_prefix(SERIAL_READY_PREFIX)
        .ok_or(SerialMessageError::InvalidPrefix(SERIAL_READY_PREFIX))?;
    Ok(serde_json::from_str(body)?)
}

#[cfg(all(feature = "std", feature = "serde"))]
pub fn render_serial_boot_offer(
    message: &SerialBootOfferMessage,
) -> Result<String, serde_json::Error> {
    Ok(format!(
        "{SERIAL_BOOT_PREFIX}{}",
        serde_json::to_string(message)?
    ))
}

#[cfg(all(feature = "std", feature = "serde"))]
pub fn parse_serial_boot_offer(line: &str) -> Result<SerialBootOfferMessage, SerialMessageError> {
    let body = line
        .trim()
        .strip_prefix(SERIAL_BOOT_PREFIX)
        .ok_or(SerialMessageError::InvalidPrefix(SERIAL_BOOT_PREFIX))?;
    Ok(serde_json::from_str(body)?)
}

#[cfg(test)]
mod tests {
    use super::{
        BootArch, BootOfferResponse, BootOfferState, DiscoverySolicit, ImageFormat,
        LoaderCapabilities, LoaderHelloRequest, SERIAL_BOOT_PREFIX, SERIAL_PROTOCOL_VERSION,
        SERIAL_READY_PREFIX, SerialBootOfferMessage, SerialReadyMessage, parse_serial_boot_offer,
        parse_serial_ready, render_serial_boot_offer, render_serial_ready,
    };

    #[test]
    fn serializes_loader_control_messages() {
        let hello = LoaderHelloRequest {
            protocol_version: 1,
            nonce: "nonce-1".into(),
            arch: BootArch::X86_64,
            board: Some("asus-nuc15crh".into()),
            mac: "1c:69:7a:dc:f3:47".into(),
            firmware_vendor: Some("UEFI".into()),
            loader_version: Some("0.5.11".into()),
            capabilities: LoaderCapabilities {
                image_formats: vec![ImageFormat::Elf64],
                range_get: true,
                sha256: true,
            },
        };
        let value = serde_json::to_value(&hello).unwrap();
        assert_eq!(value["arch"], "x86_64");
        assert_eq!(value["capabilities"]["image_formats"][0], "elf64");

        let offer = BootOfferResponse {
            state: BootOfferState::Ready,
            wait_after_ms: None,
            boot_id: Some("boot-1".into()),
            kernel_url: Some("http://127.0.0.1/kernel.elf".into()),
            kernel_size: Some(4096),
            kernel_sha256: None,
            image_format: Some(ImageFormat::Elf64),
            arch: Some(BootArch::X86_64),
            entry_symbol: Some("httpboot_entry".into()),
            message: None,
        };
        let value = serde_json::to_value(&offer).unwrap();
        assert_eq!(value["state"], "ready");
        assert_eq!(value["image_format"], "elf64");
    }

    #[test]
    fn parses_loader_discovery_solicit() {
        let solicit: DiscoverySolicit = serde_json::from_str(
            r#"{
                "type": "ostool_httpboot_solicit",
                "version": 1,
                "arch": "x86_64",
                "board": "asus-nuc15crh",
                "mac": "1c:69:7a:dc:f3:47",
                "nonce": "nonce-1"
            }"#,
        )
        .unwrap();

        assert_eq!(solicit.arch, BootArch::X86_64);
        assert_eq!(solicit.mac, "1c:69:7a:dc:f3:47");
    }

    #[test]
    fn renders_and_parses_serial_ready_message() {
        let ready = SerialReadyMessage {
            protocol_version: SERIAL_PROTOCOL_VERSION,
            board: "asus-nuc15crh".into(),
            arch: BootArch::X86_64,
            loader_version: Some("axloader".into()),
        };

        let line = render_serial_ready(&ready).unwrap();

        assert!(line.starts_with(SERIAL_READY_PREFIX));
        assert_eq!(parse_serial_ready(&line).unwrap(), ready);
    }

    #[test]
    fn renders_and_parses_serial_boot_offer_message() {
        let offer = SerialBootOfferMessage {
            protocol_version: SERIAL_PROTOCOL_VERSION,
            boot_id: "boot-1".into(),
            kernel_url: "http://10.3.10.192:2999/boot/kernel.elf".into(),
            kernel_size: 4096,
            image_format: ImageFormat::Elf64,
            arch: BootArch::X86_64,
            entry_symbol: Some("httpboot_entry".into()),
        };

        let line = render_serial_boot_offer(&offer).unwrap();

        assert!(line.starts_with(SERIAL_BOOT_PREFIX));
        assert_eq!(parse_serial_boot_offer(&line).unwrap(), offer);
    }
}
