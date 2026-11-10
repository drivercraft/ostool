#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::{fmt, str::FromStr};

mod serial;
pub use serial::*;

#[cfg(feature = "alloc")]
use alloc::string::{String, ToString as _};

/// The only device control protocol understood by this release.
pub const DEVICE_PROTOCOL_VERSION: u16 = 7;
pub const DEVICE_PROTOCOL_VERSION_HEADER: &str = "X-Axloader-Protocol-Version";
pub const UNSUPPORTED_PROTOCOL_CODE: &str = "unsupported_protocol";
pub const UNSUPPORTED_PROTOCOL_MESSAGE: &str = "axloader protocol 7 is required";
pub const MAX_HOST_CMDLINE_BYTES: usize = 4095;
pub const MAX_HTTP_BOOT_INITRAMFS_BYTES: usize = 256 * 1024 * 1024;

pub fn valid_host_cmdline(value: &str) -> bool {
    value.len() <= MAX_HOST_CMDLINE_BYTES
        && value
            .bytes()
            .all(|byte| byte == b' ' || byte.is_ascii_graphic())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct MacAddress([u8; 6]);

impl MacAddress {
    pub const ZERO: Self = Self([0; 6]);

    pub const fn new(octets: [u8; 6]) -> Self {
        Self(octets)
    }

    pub const fn octets(self) -> [u8; 6] {
        self.0
    }

    pub fn is_zero(self) -> bool {
        self.0 == [0; 6]
    }
}

impl fmt::Display for MacAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            self.0[0], self.0[1], self.0[2], self.0[3], self.0[4], self.0[5]
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseMacAddressError {
    InvalidLength,
    InvalidOctet,
}

impl fmt::Display for ParseMacAddressError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidLength => f.write_str("MAC address must contain six hexadecimal octets"),
            Self::InvalidOctet => f.write_str("MAC address contains an invalid hexadecimal octet"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for ParseMacAddressError {}

impl FromStr for MacAddress {
    type Err = ParseMacAddressError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let mut octets = [0_u8; 6];
        let mut parts = value.split(':');
        for octet in &mut octets {
            let part = parts.next().ok_or(ParseMacAddressError::InvalidLength)?;
            if part.len() != 2 {
                return Err(ParseMacAddressError::InvalidOctet);
            }
            *octet =
                u8::from_str_radix(part, 16).map_err(|_| ParseMacAddressError::InvalidOctet)?;
        }
        if parts.next().is_some() {
            return Err(ParseMacAddressError::InvalidLength);
        }
        Ok(Self(octets))
    }
}

#[cfg(feature = "json")]
impl serde::Serialize for MacAddress {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.collect_str(self)
    }
}

#[cfg(feature = "json")]
impl<'de> serde::Deserialize<'de> for MacAddress {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        struct Visitor;

        impl serde::de::Visitor<'_> for Visitor {
            type Value = MacAddress;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a six-octet colon-separated MAC address")
            }

            fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
            where
                E: serde::de::Error,
            {
                value.parse().map_err(E::custom)
            }
        }

        deserializer.deserialize_str(Visitor)
    }
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct LoaderHardwareInfo {
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub version: Option<String>,
    pub serial: Option<String>,
}

/// A v7 loader advertises its own HTTP endpoint without registering with a server.
#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderAnnouncement {
    pub protocol_version: u16,
    pub mac_address: MacAddress,
    pub current_mac_address: MacAddress,
    #[cfg_attr(feature = "json", serde(deserialize_with = "deserialize_arch_name"))]
    pub arch: String,
    pub loader_version: String,
    pub boot_epoch: String,
    pub http_port: u16,
    #[cfg_attr(feature = "json", serde(default))]
    pub serial_id: Option<String>,
    #[cfg_attr(feature = "json", serde(default))]
    pub serial_ready: bool,
}

/// The caller owns the boot ID; the loader owns the current boot epoch.
#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(deny_unknown_fields))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceBootJob {
    pub boot_id: String,
    pub kernel: DeviceBootImage,
    pub initramfs: Option<DeviceBootImage>,
    pub cmdline: Option<String>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(deny_unknown_fields))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceBootImage {
    pub size: u64,
    pub sha256: String,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceBootStatus {
    pub boot_id: String,
    pub phase: String,
    pub kernel_received: bool,
    pub initramfs_received: bool,
    pub last_error: Option<DeviceOperationError>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceOperationError {
    pub code: String,
    pub message: String,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderDeviceStatus {
    pub protocol_version: u16,
    pub boot_epoch: String,
    pub mac_address: MacAddress,
    pub current_mac_address: MacAddress,
    #[cfg_attr(feature = "json", serde(deserialize_with = "deserialize_arch_name"))]
    pub arch: String,
    pub loader_version: String,
    pub hardware: LoaderHardwareInfo,
    pub boot: Option<DeviceBootStatus>,
    pub ota: Option<LoaderOtaState>,
    #[cfg_attr(feature = "json", serde(default))]
    pub serial: Option<LoaderSerialStatus>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderOtaState {
    pub active_sha256: String,
    pub running_sha256: String,
    pub pending_update_id: Option<String>,
    /// A pending image is runnable only after its attempt record was flushed.
    pub trial: bool,
    pub source: Option<OtaSource>,
    pub last_update_id: Option<String>,
    pub last_outcome: Option<OtaOutcome>,
}

#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", serde(rename_all = "snake_case"))]
pub enum OtaSource {
    Direct,
    Server,
}

#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", serde(rename_all = "snake_case"))]
pub enum OtaOutcome {
    Confirmed,
    RolledBack,
    Failed,
}

/// A session-scoped boot file authenticated before kernel handoff.
#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootFile {
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "json", serde(tag = "phase", rename_all = "snake_case"))]
pub enum LoaderStatusPhase {
    Accepted,
    Downloading { received: u64, total: u64 },
    Verified,
    ReadyToHandoff,
    Failed { code: String, message: String },
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(deny_unknown_fields))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderStatusResponse {
    pub session_id: String,
    pub boot_id: String,
    pub boot_epoch: Option<String>,
    pub status: Option<LoaderStatusPhase>,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelPublishResponse {
    pub boot_id: String,
    pub kernel_url: String,
    pub kernel_size: u64,
    pub kernel_sha256: Option<String>,
}

#[cfg(feature = "alloc")]
impl DeviceOperationError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

#[cfg(feature = "alloc")]
impl fmt::Display for DeviceOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.code, self.message)
    }
}

#[cfg(feature = "alloc")]
impl core::error::Error for DeviceOperationError {}

#[cfg(feature = "alloc")]
pub fn unsupported_protocol_error() -> DeviceOperationError {
    DeviceOperationError::new(UNSUPPORTED_PROTOCOL_CODE, UNSUPPORTED_PROTOCOL_MESSAGE)
}

#[cfg(feature = "alloc")]
pub fn validate_device_protocol_version(version: u16) -> Result<(), DeviceOperationError> {
    if version == DEVICE_PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(unsupported_protocol_error())
    }
}

#[cfg(feature = "alloc")]
pub fn device_protocol_version_header_value() -> String {
    DEVICE_PROTOCOL_VERSION.to_string()
}

/// Validates the wire header without depending on an HTTP framework.
#[cfg(feature = "alloc")]
pub fn validate_device_protocol_version_header(
    value: Option<&str>,
) -> Result<(), DeviceOperationError> {
    let version = value
        .and_then(|value| value.parse::<u16>().ok())
        .ok_or_else(unsupported_protocol_error)?;
    validate_device_protocol_version(version)
}

/// Decodes the shared device error shape for either control frontend or host.
#[cfg(all(feature = "alloc", feature = "json"))]
pub fn decode_device_operation_error(
    body: &[u8],
) -> Result<DeviceOperationError, serde_json::Error> {
    serde_json::from_slice(body)
}

#[cfg(feature = "json")]
fn deserialize_arch_name<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let arch = <String as serde::Deserialize>::deserialize(deserializer)?;
    if arch.trim().is_empty() {
        return Err(serde::de::Error::custom(
            "loader architecture name must not be empty",
        ));
    }
    Ok(arch)
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn normalizes_mac_addresses() {
        let mac: MacAddress = "AA:0b:0C:0d:EE:fF".parse().unwrap();
        assert_eq!(mac.to_string(), "aa:0b:0c:0d:ee:ff");
        assert_eq!(mac.octets(), [0xaa, 0x0b, 0x0c, 0x0d, 0xee, 0xff]);
    }

    #[test]
    fn rejects_malformed_mac_addresses() {
        for value in [
            "",
            "00:11:22:33:44",
            "00:11:22:33:44:555",
            "gg:11:22:33:44:55",
        ] {
            assert!(value.parse::<MacAddress>().is_err(), "accepted {value}");
        }
    }

    #[test]
    fn serializes_mac_as_canonical_string() {
        let mac = MacAddress::new([0, 1, 2, 3, 4, 255]);
        assert_eq!(
            serde_json::to_string(&mac).unwrap(),
            "\"00:01:02:03:04:ff\""
        );
        assert_eq!(
            serde_json::from_str::<MacAddress>("\"00:01:02:03:04:FF\"").unwrap(),
            mac
        );
    }

    #[test]
    fn validates_only_the_v7_protocol_header() {
        assert_eq!(validate_device_protocol_version_header(Some("7")), Ok(()));
        for value in [None, Some(""), Some("6"), Some("8"), Some("seven")] {
            assert_eq!(
                validate_device_protocol_version_header(value),
                Err(unsupported_protocol_error())
            );
        }
    }

    #[test]
    fn boot_job_rejects_legacy_fields_and_accepts_payload_combinations() {
        let kernel = DeviceBootImage {
            size: 4096,
            sha256: "00".repeat(32),
        };
        let base = DeviceBootJob {
            boot_id: "boot-1".into(),
            kernel: kernel.clone(),
            initramfs: None,
            cmdline: None,
        };
        for value in [
            serde_json::json!({
                "boot_id": "boot-1",
                "kernel": {"size": 4096, "sha256": "00".repeat(32)},
                "initramfs": null,
                "cmdline": null
            }),
            serde_json::json!({
                "boot_id": "boot-1",
                "kernel": {"size": 4096, "sha256": "00".repeat(32)},
                "cmdline": "console=ttyS0"
            }),
            serde_json::json!({
                "boot_id": "boot-1",
                "kernel": {"size": 4096, "sha256": "00".repeat(32)},
                "initramfs": {"size": 1024, "sha256": "11".repeat(32)}
            }),
            serde_json::json!({
                "boot_id": "boot-1",
                "kernel": {"size": 4096, "sha256": "00".repeat(32)},
                "initramfs": {"size": 1024, "sha256": "11".repeat(32)},
                "cmdline": "console=ttyS0"
            }),
        ] {
            let decoded: DeviceBootJob = serde_json::from_value(value).unwrap();
            assert_eq!(decoded.kernel, base.kernel);
        }

        for field in ["arch", "image_format", "entry_symbol"] {
            let mut value = serde_json::to_value(&base).unwrap();
            value[field] = serde_json::json!("legacy");
            assert!(serde_json::from_value::<DeviceBootJob>(value).is_err());
        }
        let mut value = serde_json::to_value(&base).unwrap();
        value["kernel"]["entry_symbol"] = serde_json::json!("legacy");
        assert!(serde_json::from_value::<DeviceBootJob>(value).is_err());
    }

    #[test]
    fn loader_arch_is_an_open_nonempty_string() {
        let mac: MacAddress = "02:00:00:00:00:01".parse().unwrap();
        let status: LoaderDeviceStatus = serde_json::from_value(serde_json::json!({
            "protocol_version": DEVICE_PROTOCOL_VERSION,
            "boot_epoch": "epoch",
            "mac_address": mac,
            "current_mac_address": mac,
            "arch": "future-isa",
            "loader_version": "axloader",
            "hardware": {},
            "boot": null,
            "ota": null
        }))
        .unwrap();
        assert_eq!(status.arch, "future-isa");

        let mut value = serde_json::to_value(&status).unwrap();
        value["arch"] = serde_json::json!("");
        assert!(serde_json::from_value::<LoaderDeviceStatus>(value).is_err());
    }

    #[test]
    fn device_errors_keep_open_codes_and_decode_from_body() {
        let error = DeviceOperationError::new("future_device_error", "useful detail");
        let bytes = serde_json::to_vec(&error).unwrap();
        assert_eq!(decode_device_operation_error(&bytes).unwrap(), error);
    }

    #[test]
    fn loader_status_rejects_the_old_registration_field() {
        let value = serde_json::json!({
            "session_id": "session-1",
            "boot_id": "boot-1",
            "registration_id": "epoch-1",
            "status": null
        });
        assert!(serde_json::from_value::<LoaderStatusResponse>(value).is_err());
    }
}
