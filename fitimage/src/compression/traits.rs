//! Compression interface definitions.
//!
//! Defines the standard interface that all compression algorithms must implement.

use crate::error::Result;

/// Compression interface trait.
///
/// All compression algorithms must implement this interface.
pub trait CompressionInterface {
    /// Compresses data.
    ///
    /// # Arguments
    ///
    /// * `data` - The raw data to compress
    ///
    /// # Returns
    ///
    /// The compressed data.
    fn compress(&self, data: &[u8]) -> Result<Vec<u8>>;

    /// Decompresses data (mainly used for verification).
    ///
    /// # Arguments
    ///
    /// * `compressed_data` - The compressed data
    ///
    /// # Returns
    ///
    /// The decompressed original data.
    fn decompress(&self, compressed_data: &[u8]) -> Result<Vec<u8>>;

    /// Returns the name of the compression algorithm.
    fn get_name(&self) -> &'static str;
}
