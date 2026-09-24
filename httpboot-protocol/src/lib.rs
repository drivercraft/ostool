#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

use core::{fmt, str::FromStr};

#[cfg(feature = "alloc")]
use alloc::{string::String, vec::Vec};

pub const PROTOCOL_VERSION: u16 = 3;
pub const LEGACY_PROTOCOL_VERSION: u16 = 2;
pub const MAX_HOST_CMDLINE_BYTES: usize = 4095;
pub const MAX_HTTP_BOOT_INITRAMFS_BYTES: usize = 256 * 1024 * 1024;
pub const DISCOVERY_PORT: u16 = 2998;
pub const MAX_DISCOVERY_DATAGRAM_BYTES: usize = 1400;

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

#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", serde(rename_all = "snake_case"))]
pub enum BootArch {
    X86_64,
    Aarch64,
    Loongarch64,
    Riscv64,
    Other,
}

#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", serde(rename_all = "snake_case"))]
pub enum ImageFormat {
    Elf64,
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

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderDiscoveryProbe {
    pub protocol_version: u16,
    pub mac_address: MacAddress,
    pub current_mac_address: MacAddress,
    pub arch: BootArch,
    pub loader_version: String,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderDiscoveryOffer {
    pub protocol_version: u16,
    pub server_id: String,
    pub control_base_url: String,
    pub registration_id: String,
    pub expires_in_ms: u64,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderPollRequest {
    pub protocol_version: u16,
    pub registration_id: String,
    pub mac_address: MacAddress,
    pub current_mac_address: MacAddress,
    pub ip_address: String,
    pub arch: BootArch,
    pub loader_version: String,
    pub hardware: LoaderHardwareInfo,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "json", serde(tag = "state", rename_all = "snake_case"))]
pub enum LoaderPollResponse {
    Unbound,
    BoundIdle {
        board_id: String,
    },
    Boot {
        board_id: String,
        session_id: String,
        boot_id: String,
        kernel_path: String,
        kernel_size: u64,
        kernel_sha256: String,
        arch: BootArch,
        image_format: ImageFormat,
        entry_symbol: Option<String>,
        initramfs: Option<BootFile>,
        cmdline: Option<String>,
    },
    Reject {
        code: String,
        message: String,
        retry_after_ms: Option<u64>,
    },
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderStatusReport {
    pub protocol_version: u16,
    pub registration_id: String,
    pub mac_address: MacAddress,
    pub session_id: String,
    pub boot_id: String,
    pub status: LoaderStatusPhase,
}

#[cfg(feature = "alloc")]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoaderStatusResponse {
    pub session_id: String,
    pub boot_id: String,
    pub registration_id: Option<String>,
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

#[cfg(all(feature = "alloc", feature = "json"))]
pub fn encode_discovery_probe(
    probe: &LoaderDiscoveryProbe,
) -> Result<Vec<u8>, DiscoveryMessageError> {
    let bytes = serde_json::to_vec(probe).map_err(DiscoveryMessageError::Json)?;
    if bytes.len() > MAX_DISCOVERY_DATAGRAM_BYTES {
        return Err(DiscoveryMessageError::TooLarge(bytes.len()));
    }
    Ok(bytes)
}

#[cfg(all(feature = "alloc", feature = "json"))]
#[derive(Debug)]
pub enum DiscoveryMessageError {
    TooLarge(usize),
    Json(serde_json::Error),
}

#[cfg(all(feature = "alloc", feature = "json"))]
impl fmt::Display for DiscoveryMessageError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooLarge(size) => write!(
                f,
                "discovery datagram is {size} bytes, limit is {MAX_DISCOVERY_DATAGRAM_BYTES}"
            ),
            Self::Json(err) => write!(f, "failed to encode discovery datagram: {err}"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for DiscoveryMessageError {}

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
    fn discovery_packet_stays_below_the_mtu_budget() {
        let probe = LoaderDiscoveryProbe {
            protocol_version: PROTOCOL_VERSION,
            mac_address: "02:00:00:00:00:01".parse().unwrap(),
            current_mac_address: "02:00:00:00:00:01".parse().unwrap(),
            arch: BootArch::X86_64,
            loader_version: "axloader-0.2".into(),
        };
        assert!(encode_discovery_probe(&probe).unwrap().len() <= MAX_DISCOVERY_DATAGRAM_BYTES);
    }

    #[test]
    fn rejects_discovery_packet_above_the_mtu_budget() {
        let probe = LoaderDiscoveryProbe {
            protocol_version: PROTOCOL_VERSION,
            mac_address: "02:00:00:00:00:01".parse().unwrap(),
            current_mac_address: "02:00:00:00:00:01".parse().unwrap(),
            arch: BootArch::X86_64,
            loader_version: "x".repeat(MAX_DISCOVERY_DATAGRAM_BYTES),
        };

        assert!(matches!(
            encode_discovery_probe(&probe),
            Err(DiscoveryMessageError::TooLarge(_))
        ));
    }

    #[test]
    fn rejects_malformed_discovery_json() {
        assert!(serde_json::from_slice::<LoaderDiscoveryProbe>(b"{not-json}").is_err());
        assert!(
            serde_json::from_slice::<LoaderDiscoveryProbe>(br#"{"protocol_version":2}"#).is_err()
        );
    }

    #[test]
    fn loader_poll_state_round_trips() {
        let response = LoaderPollResponse::Boot {
            board_id: "qemu-x86-1".into(),
            session_id: "session-1".into(),
            boot_id: "boot-1".into(),
            kernel_path: "/boot/sessions/session-1/kernel.elf".into(),
            kernel_size: 4096,
            kernel_sha256: "00".repeat(32),
            arch: BootArch::X86_64,
            image_format: ImageFormat::Elf64,
            entry_symbol: Some("httpboot_entry".into()),
            initramfs: Some(BootFile {
                path: "/boot/sessions/session-1/initramfs.cpio".into(),
                size: 1024,
                sha256: "11".repeat(32),
            }),
            cmdline: Some("root=/dev/vda".into()),
        };
        let bytes = serde_json::to_vec(&response).unwrap();
        assert_eq!(
            serde_json::from_slice::<LoaderPollResponse>(&bytes).unwrap(),
            response
        );
    }
}
