//! Gzip compression implementation.
//!
//! Provides gzip compression and decompression functionality using the flate2 library.

use std::io::{Read, Write};

use crate::compression::traits::CompressionInterface;
use crate::error::Result;
use flate2::{read::GzDecoder, write::GzEncoder, Compression as GzipLevel};

/// Gzip compressor with configurable compression level.
pub struct GzipCompressor {
    /// Compression level (0-9, where 0 means no compression).
    level: u8,
    /// Whether compression is enabled (false means data is copied directly).
    enabled: bool,
}

impl Default for GzipCompressor {
    fn default() -> Self {
        Self::new(6)
    }
}

impl GzipCompressor {
    /// Creates a new gzip compressor with the specified compression level.
    ///
    /// # Arguments
    ///
    /// * `level` - Compression level from 0 to 9. Level 0 disables compression.
    pub fn new(level: u8) -> Self {
        Self {
            level: level.clamp(0, 9),
            enabled: level > 0,
        }
    }

    /// Creates a disabled compressor instance that passes data through unchanged.
    pub fn new_disabled() -> Self {
        Self {
            level: 0,
            enabled: false,
        }
    }

    /// Gets the flate2 compression level.
    fn get_compression_level(&self) -> GzipLevel {
        if !self.enabled {
            GzipLevel::none()
        } else {
            match self.level {
                0 => GzipLevel::none(),
                1 => GzipLevel::fast(),
                9 => GzipLevel::best(),
                _ => GzipLevel::default(),
            }
        }
    }
}

impl CompressionInterface for GzipCompressor {
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>> {
        if !self.enabled {
            // If compression is disabled, return a copy of the data.
            return Ok(data.to_vec());
        }

        let mut encoder = GzEncoder::new(Vec::new(), self.get_compression_level());

        encoder.write_all(data).map_err(|e| {
            crate::error::MkImageError::compression_error(format!("Gzip compression failed: {}", e))
        })?;

        encoder.finish().map_err(|e| {
            crate::error::MkImageError::compression_error(format!("Gzip finish failed: {}", e))
        })
    }

    fn decompress(&self, compressed_data: &[u8]) -> Result<Vec<u8>> {
        if !self.enabled {
            // If compression was not applied, return a copy of the data.
            return Ok(compressed_data.to_vec());
        }

        let mut decoder = GzDecoder::new(compressed_data);
        let mut buffer = Vec::new();

        decoder.read_to_end(&mut buffer).map_err(|e| {
            crate::error::MkImageError::compression_error(format!(
                "Gzip decompression failed: {}",
                e
            ))
        })?;

        Ok(buffer)
    }

    fn get_name(&self) -> &'static str {
        if self.enabled {
            "gzip"
        } else {
            "none"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gzip_compression() {
        let compressor = GzipCompressor::new(6);
        // 使用更大的数据以确保压缩效果
        let original_data = "Hello, World! This is a test string for gzip compression. ".repeat(10);
        let original_bytes = original_data.as_bytes();

        // 测试压缩
        let compressed = compressor
            .compress(original_bytes)
            .expect("Compression should succeed");
        assert!(
            compressed.len() < original_bytes.len(),
            "Compressed data should be smaller"
        );

        // 测试解压缩
        let decompressed = compressor
            .decompress(&compressed)
            .expect("Decompression should succeed");
        assert_eq!(
            decompressed, original_bytes,
            "Decompressed data should match original"
        );
    }

    #[test]
    fn test_disabled_compression() {
        let compressor = GzipCompressor::new_disabled();
        let original_data = b"Hello, World!";

        // 禁用压缩时应该返回原数据
        let compressed = compressor
            .compress(original_data)
            .expect("Compression should succeed");
        assert_eq!(
            compressed, original_data,
            "Disabled compression should return original data"
        );

        let decompressed = compressor
            .decompress(&compressed)
            .expect("Decompression should succeed");
        assert_eq!(
            decompressed, original_data,
            "Decompressed data should match original"
        );
    }

    #[test]
    fn test_compressor_name() {
        let enabled_compressor = GzipCompressor::new(6);
        assert_eq!(enabled_compressor.get_name(), "gzip");

        let disabled_compressor = GzipCompressor::new_disabled();
        assert_eq!(disabled_compressor.get_name(), "none");
    }
}
