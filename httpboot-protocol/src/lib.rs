#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(test)]
extern crate std;

use core::str;
#[cfg(feature = "std")]
use std::vec::Vec;

pub const DISCOVERY_PROTOCOL_VERSION: u16 = 1;
pub const DISCOVERY_ADVERTISE_TYPE: &str = "ostool_httpboot_advertise";
pub const DISCOVERY_SOLICIT_TYPE: &str = "ostool_httpboot_solicit";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootManifest<'a> {
    pub kernel_url: &'a str,
    pub kernel_size: u64,
    pub kernel_load_addr: u64,
    pub entry_point: u64,
    pub arch: &'a str,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpBootManifest {
    pub kernel_url: String,
    pub kernel_size: u64,
    pub kernel_load_addr: String,
    pub entry_point: String,
    pub arch: String,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpBootArtifactRequest {
    pub remote_name: Option<String>,
    pub kernel_load_addr: String,
    pub entry_point: String,
    pub arch: String,
}

#[cfg(feature = "std")]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpBootArtifactResponse {
    pub kernel_url: String,
    pub manifest_url: String,
    pub kernel_size: u64,
}

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestError {
    MissingField(&'static str),
    InvalidJson(&'static str),
    InvalidNumber(&'static str),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DownloadError {
    EmptyBody,
    BodyTooLarge,
    NonUtf8Body,
    InvalidManifest(ManifestError),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UrlError {
    EmptyUrl,
    MissingPathSeparator,
    OutputTooSmall,
    MalformedDevicePath,
    NonUtf8Uri,
    UriNotFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ParseNumberError {
    InvalidDigit,
    Empty,
    Overflow,
}

pub fn parse_manifest(input: &str) -> Result<BootManifest<'_>, ManifestError> {
    Ok(BootManifest {
        kernel_url: json_string_field(input, "kernel_url")?,
        kernel_size: json_u64_field(input, "kernel_size")?,
        kernel_load_addr: parse_addr(json_string_field(input, "kernel_load_addr")?)
            .map_err(|_| ManifestError::InvalidNumber("kernel_load_addr"))?,
        entry_point: parse_addr(json_string_field(input, "entry_point")?)
            .map_err(|_| ManifestError::InvalidNumber("entry_point"))?,
        arch: json_string_field(input, "arch")?,
    })
}

pub fn parse_downloaded_manifest(
    body: &[u8],
    max_len: usize,
) -> Result<BootManifest<'_>, DownloadError> {
    if body.is_empty() {
        return Err(DownloadError::EmptyBody);
    }
    if body.len() > max_len {
        return Err(DownloadError::BodyTooLarge);
    }

    let manifest = str::from_utf8(body).map_err(|_| DownloadError::NonUtf8Body)?;
    parse_manifest(manifest).map_err(DownloadError::InvalidManifest)
}

pub fn write_sibling_manifest_url<'a>(
    loader_url: &str,
    output: &'a mut [u8],
) -> Result<&'a str, UrlError> {
    let loader_url = loader_url.trim();
    if loader_url.is_empty() {
        return Err(UrlError::EmptyUrl);
    }

    let slash = loader_url
        .rfind('/')
        .ok_or(UrlError::MissingPathSeparator)?;
    let prefix = &loader_url[..slash + 1];
    let needed = prefix.len() + b"manifest.json".len();
    if needed > output.len() {
        return Err(UrlError::OutputTooSmall);
    }

    output[..prefix.len()].copy_from_slice(prefix.as_bytes());
    output[prefix.len()..needed].copy_from_slice(b"manifest.json");

    str::from_utf8(&output[..needed]).map_err(|_| UrlError::NonUtf8Uri)
}

pub fn uri_from_device_path(device_path: &[u8]) -> Result<&str, UrlError> {
    const DEVICE_PATH_TYPE_MESSAGING: u8 = 0x03;
    const DEVICE_PATH_TYPE_END: u8 = 0x7f;
    const DEVICE_PATH_SUBTYPE_URI: u8 = 0x18;
    const DEVICE_PATH_SUBTYPE_END_ENTIRE: u8 = 0xff;

    let mut offset = 0;
    let mut uri = None;

    while offset + 4 <= device_path.len() {
        let node_type = device_path[offset];
        let node_subtype = device_path[offset + 1];
        let node_len =
            u16::from_le_bytes([device_path[offset + 2], device_path[offset + 3]]) as usize;
        if node_len < 4 || offset + node_len > device_path.len() {
            return Err(UrlError::MalformedDevicePath);
        }

        if node_type == DEVICE_PATH_TYPE_MESSAGING && node_subtype == DEVICE_PATH_SUBTYPE_URI {
            let payload = trim_trailing_nul(&device_path[offset + 4..offset + node_len]);
            uri = Some(str::from_utf8(payload).map_err(|_| UrlError::NonUtf8Uri)?);
        }

        offset += node_len;
        if node_type == DEVICE_PATH_TYPE_END && node_subtype == DEVICE_PATH_SUBTYPE_END_ENTIRE {
            return uri.ok_or(UrlError::UriNotFound);
        }
    }

    Err(UrlError::MalformedDevicePath)
}

pub fn parse_addr(input: &str) -> Result<u64, ParseNumberError> {
    let value = input.trim();
    let (radix, digits) = if let Some(hex) = value
        .strip_prefix("0x")
        .or_else(|| value.strip_prefix("0X"))
    {
        (16, hex)
    } else {
        (10, value)
    };

    parse_u64_digits(digits, radix)
}

fn trim_trailing_nul(bytes: &[u8]) -> &[u8] {
    match bytes.iter().rposition(|byte| *byte != 0) {
        Some(last) => &bytes[..=last],
        None => &[],
    }
}

fn json_string_field<'a>(input: &'a str, key: &'static str) -> Result<&'a str, ManifestError> {
    let value = field_value(input, key)?;
    parse_json_string(value).ok_or(ManifestError::InvalidJson(key))
}

fn json_u64_field(input: &str, key: &'static str) -> Result<u64, ManifestError> {
    let value = field_value(input, key)?;
    let end = value
        .bytes()
        .position(|byte| !byte.is_ascii_digit() && byte != b'_')
        .unwrap_or(value.len());
    if end == 0 {
        return Err(ManifestError::InvalidNumber(key));
    }
    parse_u64_digits(&value[..end], 10).map_err(|_| ManifestError::InvalidNumber(key))
}

fn field_value<'a>(input: &'a str, key: &'static str) -> Result<&'a str, ManifestError> {
    let key_start = find_json_key(input, key).ok_or(ManifestError::MissingField(key))?;
    let after_key = &input[key_start + key.len() + 2..];
    let colon = after_key
        .bytes()
        .position(|byte| byte == b':')
        .ok_or(ManifestError::InvalidJson(key))?;
    Ok(after_key[colon + 1..].trim_start())
}

fn find_json_key(input: &str, key: &str) -> Option<usize> {
    let quoted_len = key.len() + 2;
    let bytes = input.as_bytes();
    let mut index = 0;

    while index + quoted_len <= bytes.len() {
        if bytes[index] == b'"'
            && input[index + 1..].starts_with(key)
            && bytes.get(index + quoted_len - 1) == Some(&b'"')
        {
            return Some(index);
        }
        index += 1;
    }

    None
}

fn parse_json_string(input: &str) -> Option<&str> {
    let bytes = input.as_bytes();
    if bytes.first() != Some(&b'"') {
        return None;
    }

    let mut index = 1;
    while index < bytes.len() {
        match bytes[index] {
            b'\\' => return None,
            b'"' => return Some(&input[1..index]),
            _ => index += 1,
        }
    }

    None
}

fn parse_u64_digits(input: &str, radix: u32) -> Result<u64, ParseNumberError> {
    let mut value = 0u64;
    let mut saw_digit = false;

    for byte in input.bytes() {
        if byte == b'_' {
            continue;
        }
        let digit = match byte {
            b'0'..=b'9' => (byte - b'0') as u32,
            b'a'..=b'f' => (byte - b'a' + 10) as u32,
            b'A'..=b'F' => (byte - b'A' + 10) as u32,
            _ => return Err(ParseNumberError::InvalidDigit),
        };
        if digit >= radix {
            return Err(ParseNumberError::InvalidDigit);
        }
        value = value
            .checked_mul(radix as u64)
            .and_then(|value| value.checked_add(digit as u64))
            .ok_or(ParseNumberError::Overflow)?;
        saw_digit = true;
    }

    saw_digit.then_some(value).ok_or(ParseNumberError::Empty)
}

#[cfg(test)]
mod tests {
    use std::vec::Vec;

    use super::{
        BootArch, BootManifest, BootOfferResponse, BootOfferState, DiscoverySolicit, DownloadError,
        ImageFormat, LoaderCapabilities, LoaderHelloRequest, ManifestError,
        parse_downloaded_manifest, parse_manifest, uri_from_device_path,
        write_sibling_manifest_url,
    };

    #[test]
    fn parses_server_manifest() {
        let manifest = parse_manifest(
            r#"{
                "kernel_url": "http://127.0.0.1:2999/boot/boards/demo/current/kernel.bin",
                "kernel_size": 4096,
                "kernel_load_addr": "0x200000",
                "entry_point": "0x200006",
                "arch": "x86_64"
            }"#,
        )
        .unwrap();

        assert_eq!(
            manifest,
            BootManifest {
                kernel_url: "http://127.0.0.1:2999/boot/boards/demo/current/kernel.bin",
                kernel_size: 4096,
                kernel_load_addr: 0x20_0000,
                entry_point: 0x20_0006,
                arch: "x86_64",
            }
        );
    }

    #[test]
    fn rejects_missing_manifest_field() {
        let err = parse_manifest(r#"{"kernel_size": 1}"#).unwrap_err();
        assert_eq!(err, ManifestError::MissingField("kernel_url"));
    }

    #[test]
    fn rejects_escaped_manifest_strings_for_now() {
        let err = parse_manifest(
            r#"{
                "kernel_url": "http:\/\/127.0.0.1\/kernel.bin",
                "kernel_size": 1,
                "kernel_load_addr": "0x200000",
                "entry_point": "0x200000",
                "arch": "x86_64"
            }"#,
        )
        .unwrap_err();
        assert_eq!(err, ManifestError::InvalidJson("kernel_url"));
    }

    #[test]
    fn parses_downloaded_manifest_bytes() {
        let manifest = parse_downloaded_manifest(
            br#"{
                "kernel_url": "http://127.0.0.1:2999/kernel.bin",
                "kernel_size": 4096,
                "kernel_load_addr": "0x200000",
                "entry_point": "0x200000",
                "arch": "x86_64"
            }"#,
            1024,
        )
        .unwrap();

        assert_eq!(manifest.kernel_url, "http://127.0.0.1:2999/kernel.bin");
        assert_eq!(manifest.kernel_size, 4096);
        assert_eq!(manifest.kernel_load_addr, 0x20_0000);
        assert_eq!(manifest.entry_point, 0x20_0000);
        assert_eq!(manifest.arch, "x86_64");
    }

    #[test]
    fn rejects_empty_or_oversized_downloaded_manifest() {
        assert_eq!(
            parse_downloaded_manifest(b"", 1024),
            Err(DownloadError::EmptyBody)
        );
        assert_eq!(
            parse_downloaded_manifest(br#"{"kernel_size":1}"#, 4),
            Err(DownloadError::BodyTooLarge)
        );
    }

    #[test]
    fn rejects_non_utf8_downloaded_manifest() {
        assert_eq!(
            parse_downloaded_manifest(&[0xff, 0xfe], 1024),
            Err(DownloadError::NonUtf8Body)
        );
    }

    #[test]
    fn wraps_downloaded_manifest_parse_errors() {
        assert_eq!(
            parse_downloaded_manifest(br#"{"kernel_size":1}"#, 1024),
            Err(DownloadError::InvalidManifest(ManifestError::MissingField(
                "kernel_url"
            )))
        );
    }

    #[test]
    fn builds_manifest_url_next_to_loader_url() {
        let mut output = [0u8; 128];
        let manifest_url = write_sibling_manifest_url(
            "http://127.0.0.1:2999/boot/boards/demo/current/BOOTX64.EFI",
            &mut output,
        )
        .unwrap();

        assert_eq!(
            manifest_url,
            "http://127.0.0.1:2999/boot/boards/demo/current/manifest.json"
        );
    }

    #[test]
    fn rejects_manifest_url_output_that_is_too_small() {
        let mut output = [0u8; 8];
        assert!(write_sibling_manifest_url("http://host/BOOTX64.EFI", &mut output).is_err());
    }

    #[test]
    fn extracts_uri_from_device_path() {
        let uri = b"http://host/EFI/BOOT/BOOTX64.EFI";
        let node_len = 4 + uri.len() + 1;
        let mut bytes = Vec::new();
        bytes.extend_from_slice(&[0x03, 0x18]);
        bytes.extend_from_slice(&(node_len as u16).to_le_bytes());
        bytes.extend_from_slice(uri);
        bytes.push(0);
        bytes.extend_from_slice(&[0x7f, 0xff, 4, 0]);

        assert_eq!(
            uri_from_device_path(&bytes).unwrap(),
            "http://host/EFI/BOOT/BOOTX64.EFI"
        );
    }

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
}
