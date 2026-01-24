//! Error types for fitimage operations

use thiserror::Error;

/// Result type alias for fitimage operations
pub type Result<T> = std::result::Result<T, MkImageError>;

/// Errors that can occur during fitimage operations
#[derive(Error, Debug)]
pub enum MkImageError {
    #[error("Invalid image data: {0}")]
    InvalidImageData(String),

    #[error("Unsupported image type: {0}")]
    UnsupportedImageType(String),

    #[error("Unsupported architecture: {0}")]
    UnsupportedArch(String),

    #[error("Unsupported compression type: {0}")]
    UnsupportedCompression(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("CRC32 checksum mismatch: expected {expected:08x}, calculated {calculated:08x}")]
    CrcMismatch { expected: u32, calculated: u32 },

    #[error("Invalid magic number: expected {expected:08x}, found {found:08x}")]
    InvalidMagic { expected: u32, found: u32 },

    #[error("Image header too large: {size} bytes (max {max} bytes)")]
    HeaderTooLarge { size: usize, max: usize },

    #[error("Image name too long: {len} bytes (max {max} bytes)")]
    NameTooLong { len: usize, max: usize },

    #[error("Data size too large: {size} bytes (max {max} bytes)")]
    DataTooLarge { size: u64, max: u64 },

    #[error("Invalid load address: 0x{address:x}")]
    InvalidLoadAddress { address: u64 },

    #[error("Invalid entry point: 0x{address:x}")]
    InvalidEntryPoint { address: u64 },

    #[error("Failed to parse configuration: {0}")]
    ConfigParse(String),

    #[error("Failed to serialize image: {0}")]
    Serialization(String),

    #[error("Failed to compress data: {0}")]
    Compression(String),

    #[error("FIT serialization error: {0}")]
    FitSerialization(String),

    #[error("Unknown error: {0}")]
    Other(String),
}

impl MkImageError {
    /// Create an invalid image data error
    pub fn invalid_image_data(msg: impl Into<String>) -> Self {
        Self::InvalidImageData(msg.into())
    }

    /// Create an unsupported image type error
    pub fn unsupported_image_type(ty: impl Into<String>) -> Self {
        Self::UnsupportedImageType(ty.into())
    }

    /// Create an unsupported architecture error
    pub fn unsupported_arch(arch: impl Into<String>) -> Self {
        Self::UnsupportedArch(arch.into())
    }

    /// Create an unsupported compression error
    pub fn unsupported_compression(comp: impl Into<String>) -> Self {
        Self::UnsupportedCompression(comp.into())
    }

    /// Create a CRC mismatch error
    pub fn crc_mismatch(expected: u32, calculated: u32) -> Self {
        Self::CrcMismatch {
            expected,
            calculated,
        }
    }

    /// Create an invalid magic number error
    pub fn invalid_magic(expected: u32, found: u32) -> Self {
        Self::InvalidMagic { expected, found }
    }

    /// Create a config parse error
    pub fn config_parse(msg: impl Into<String>) -> Self {
        Self::ConfigParse(msg.into())
    }

    /// Create a serialization error
    pub fn serialization(msg: impl Into<String>) -> Self {
        Self::Serialization(msg.into())
    }

    /// Create an other error
    pub fn other(msg: impl Into<String>) -> Self {
        Self::Other(msg.into())
    }

    /// Create a compression error
    pub fn compression_error(msg: impl Into<String>) -> Self {
        Self::Compression(msg.into())
    }

    /// Create a FIT serialization error
    pub fn fit_serialization_error(msg: impl Into<String>) -> Self {
        Self::FitSerialization(msg.into())
    }
}

impl From<flate2::CompressError> for MkImageError {
    fn from(err: flate2::CompressError) -> Self {
        Self::compression_error(format!("Gzip compression error: {}", err))
    }
}

impl From<flate2::DecompressError> for MkImageError {
    fn from(err: flate2::DecompressError) -> Self {
        Self::compression_error(format!("Gzip decompression error: {}", err))
    }
}

// 移除有问题的serde::de::Error实现
// impl From<serde::de::Error> for MkImageError {
//     fn from(err: serde::de::Error) -> Self {
//         Self::FitSerialization(format!("Deserialization error: {}", err))
//     }
// }
