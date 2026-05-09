#![no_std]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BootManifest<'a> {
    pub kernel_url: &'a str,
    pub kernel_size: u64,
    pub kernel_load_addr: u64,
    pub entry_point: u64,
    pub arch: &'a str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ManifestError {
    MissingField(&'static str),
    InvalidJson(&'static str),
    InvalidNumber(&'static str),
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

pub fn parse_addr(input: &str) -> Result<u64, ()> {
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

fn parse_u64_digits(input: &str, radix: u32) -> Result<u64, ()> {
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
            _ => return Err(()),
        };
        if digit >= radix {
            return Err(());
        }
        value = value
            .checked_mul(radix as u64)
            .and_then(|value| value.checked_add(digit as u64))
            .ok_or(())?;
        saw_digit = true;
    }

    saw_digit.then_some(value).ok_or(())
}

#[cfg(test)]
extern crate std;

#[cfg(test)]
mod tests {
    use super::{BootManifest, ManifestError, parse_addr, parse_manifest};

    #[test]
    fn parses_server_manifest() {
        let manifest = parse_manifest(
            r#"{
                "kernel_url": "http://127.0.0.1:2999/boot/boards/demo/current/kernel.bin",
                "kernel_size": 123456,
                "kernel_load_addr": "0x20_3008_0000",
                "entry_point": "0x20_3008_0000",
                "arch": "loongarch64"
            }"#,
        )
        .unwrap();

        assert_eq!(
            manifest,
            BootManifest {
                kernel_url: "http://127.0.0.1:2999/boot/boards/demo/current/kernel.bin",
                kernel_size: 123456,
                kernel_load_addr: 0x20_3008_0000,
                entry_point: 0x20_3008_0000,
                arch: "loongarch64",
            }
        );
    }

    #[test]
    fn parses_decimal_and_hex_addresses() {
        assert_eq!(parse_addr("2097152"), Ok(0x20_0000));
        assert_eq!(parse_addr("0x20_0000"), Ok(0x20_0000));
    }

    #[test]
    fn rejects_missing_fields() {
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
}
