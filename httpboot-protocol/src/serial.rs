//! Wire parameters and streaming identification shared by firmware and hosts.

#[cfg(feature = "alloc")]
use alloc::string::String;

pub const SERIAL_BEACON_PREFIX: &[u8] = b"AXLOADER-SERIAL/1 ";
pub const SERIAL_ID_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct SerialParameters {
    pub baud_rate: u64,
    pub data_bits: u8,
    pub parity: SerialParity,
    pub stop_bits: SerialStopBits,
    pub flow_control: SerialFlowControl,
}

impl SerialParameters {
    /// Reject unknown parameters and formats that cannot carry the ASCII beacon.
    pub fn validate(self) -> Result<(), SerialProtocolError> {
        if self.baud_rate == 0 || !matches!(self.data_bits, 7 | 8) {
            return Err(SerialProtocolError::InvalidParameters);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(rename_all = "snake_case"))]
pub enum SerialParity {
    None,
    Odd,
    Even,
    Mark,
    Space,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(rename_all = "snake_case"))]
pub enum SerialStopBits {
    One,
    OnePointFive,
    Two,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(rename_all = "snake_case"))]
pub enum SerialFlowControl {
    None,
    RtsCts,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "json", serde(rename_all = "snake_case"))]
pub enum SerialBindingMode {
    Bound,
    Direct,
}

#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct SerialBinding {
    pub serial_id: String,
    pub binding_id: String,
    pub mode: SerialBindingMode,
}

#[cfg(feature = "alloc")]
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "json", derive(serde::Serialize, serde::Deserialize))]
pub struct LoaderSerialStatus {
    pub serial_id: String,
    pub ready: bool,
    pub parameters: Option<SerialParameters>,
    pub binding: Option<SerialBinding>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SerialProtocolError {
    InvalidId,
    InvalidParameters,
    Conflict,
    NotReady,
}

impl core::fmt::Display for SerialProtocolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(match self {
            Self::InvalidId => "serial ID must contain 32 lowercase hexadecimal characters",
            Self::InvalidParameters => "unknown or unsupported serial parameters",
            Self::Conflict => "serial binding belongs to another boot or owner",
            Self::NotReady => "serial binding is not ready",
        })
    }
}
impl core::error::Error for SerialProtocolError {}

pub fn valid_serial_id(id: &str) -> bool {
    id.len() == SERIAL_ID_BYTES
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[cfg(feature = "alloc")]
impl LoaderSerialStatus {
    /// Grant a binding only for this boot. An identical retry is idempotent.
    pub fn grant(&mut self, binding: SerialBinding) -> Result<(), SerialProtocolError> {
        if binding.serial_id != self.serial_id || !valid_serial_id(&binding.binding_id) {
            return Err(SerialProtocolError::Conflict);
        }
        if binding.mode == SerialBindingMode::Bound && !self.ready {
            return Err(SerialProtocolError::NotReady);
        }
        match &self.binding {
            Some(current) if current != &binding => Err(SerialProtocolError::Conflict),
            _ => {
                self.binding = Some(binding);
                Ok(())
            }
        }
    }

    /// A stale revocation must never revoke a newer owner's grant.
    pub fn revoke(&mut self, binding_id: &str) -> Result<(), SerialProtocolError> {
        if self
            .binding
            .as_ref()
            .is_none_or(|b| b.binding_id != binding_id)
        {
            return Err(SerialProtocolError::Conflict);
        }
        self.binding = None;
        Ok(())
    }

    pub fn permits_start(&self, binding_id: &str) -> bool {
        self.binding
            .as_ref()
            .is_some_and(|b| b.binding_id == binding_id && b.serial_id == self.serial_id)
    }
}

/// Bounded line decoder. Oversized/noisy lines cannot grow host memory.
pub struct SerialFrameDecoder {
    line: [u8; 64],
    len: usize,
    overflow: bool,
}

impl Default for SerialFrameDecoder {
    fn default() -> Self {
        Self {
            line: [0; 64],
            len: 0,
            overflow: false,
        }
    }
}

impl SerialFrameDecoder {
    /// Returns a complete ID; arbitrary chunk boundaries and CRLF are supported.
    pub fn push(&mut self, byte: u8) -> Option<[u8; SERIAL_ID_BYTES]> {
        if byte != b'\n' {
            if self.len == self.line.len() {
                self.overflow = true;
            } else {
                self.line[self.len] = byte;
                self.len += 1;
            }
            return None;
        }
        let end = self.len - usize::from(self.len > 0 && self.line[self.len - 1] == b'\r');
        let result = if !self.overflow {
            self.line[..end]
                .strip_prefix(SERIAL_BEACON_PREFIX)
                .and_then(|id| {
                    let text = core::str::from_utf8(id).ok()?;
                    valid_serial_id(text)
                        .then(|| id.try_into().expect("validated serial ID length"))
                })
        } else {
            None
        };
        self.len = 0;
        self.overflow = false;
        result
    }
}

#[cfg(all(test, feature = "alloc"))]
mod tests {
    use super::*;
    #[test]
    fn identification_and_grants_reject_stale_owners() {
        let id = "0123456789abcdef0123456789abcdef";
        let mut decoder = SerialFrameDecoder::default();
        let bytes = alloc::format!(
            "{}\n\r\nAXLOADER-SERIAL/1 {id}\r\nkernel\n",
            "x".repeat(100)
        );
        let ids: alloc::vec::Vec<_> = bytes.bytes().filter_map(|b| decoder.push(b)).collect();
        assert_eq!(ids, [*id.as_bytes().first_chunk::<32>().unwrap()]);
        let mut status = LoaderSerialStatus {
            serial_id: id.into(),
            ready: false,
            parameters: None,
            binding: None,
            error: Some("no UART".into()),
        };
        let mut binding = SerialBinding {
            serial_id: id.into(),
            binding_id: "a".repeat(32),
            mode: SerialBindingMode::Bound,
        };
        assert!(!status.permits_start(&binding.binding_id));
        assert_eq!(
            status.grant(binding.clone()),
            Err(SerialProtocolError::NotReady)
        );
        binding.mode = SerialBindingMode::Direct;
        status.grant(binding.clone()).unwrap();
        status.grant(binding.clone()).unwrap();
        assert!(status.permits_start(&binding.binding_id));
        assert!(status.revoke(&"b".repeat(32)).is_err());
        status.revoke(&binding.binding_id).unwrap();
        assert!(!status.permits_start(&binding.binding_id));
        binding.serial_id = "c".repeat(32);
        assert!(status.grant(binding).is_err());
    }
}
