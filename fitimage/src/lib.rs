//! # fitimage - FIT Image Library
//!
//! A Rust library for creating U-Boot compatible FIT (Flattened Image Tree) images.
//!
//! ## Features
//!
//! - Complete FIT image creation functionality
//! - Support for kernel, FDT (device tree), and ramdisk components
//! - Gzip compression support
//! - Multiple hash algorithms (MD5, SHA1, CRC32)
//! - U-Boot compatible device tree structure
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use fitimage::{FitImageBuilder, FitImageConfig, ComponentConfig};
//!
//! // Create FIT image configuration
//! let config = FitImageConfig::new("My FIT Image")
//!     .with_kernel(
//!         ComponentConfig::new("kernel", vec![/* kernel data */])
//!             .with_load_address(0x80080000)
//!             .with_entry_point(0x80080000)
//!     )
//!     .with_fdt(
//!         ComponentConfig::new("fdt", vec![/* fdt data */])
//!             .with_load_address(0x82000000)
//!     );
//!
//! // Build FIT image
//! let mut builder = FitImageBuilder::new();
//! let fit_data = builder.build(config).unwrap();
//! ```
//!
//! ## Modules
//!
//! - [`fit`] - Core FIT image building functionality
//! - [`compression`] - Compression algorithms (gzip)
//! - [`hash`] - Hash calculation utilities (MD5, SHA1, CRC32)
//! - [`crc`] - CRC32 checksum calculation
//! - [`error`] - Error types and result definitions

/// Compression algorithms support (gzip, etc.)
pub mod compression;

/// CRC32 checksum calculation utilities.
pub mod crc;

/// Error types and result definitions for FIT image operations.
pub mod error;

/// Core FIT image building functionality.
pub mod fit;

/// Hash calculation utilities (MD5, SHA1, CRC32).
pub mod hash;

// Re-export main types for convenience
pub use compression::traits::CompressionInterface;
pub use crc::calculate_crc32;
pub use error::{MkImageError, Result};
pub use fit::{ComponentConfig, FitImageBuilder, FitImageConfig};
pub use hash::{calculate_hashes, default_hash_algorithms, HashAlgorithm, HashResult};

/// Current version of the fitimage implementation
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// FIT image magic number
pub const FIT_MAGIC: &[u8] = b"FIT";
