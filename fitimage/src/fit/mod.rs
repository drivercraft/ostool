//! FIT (Flattened Image Tree) module.
//!
//! Provides functionality for creating and processing U-Boot FIT image format.

pub mod builder;
pub mod config;
pub mod fdt_header;
pub mod fdt_tokens;
pub mod standard_dt_builder;
pub mod string_table;

// 重新导出主要类型
pub use builder::FitImageBuilder;
pub use config::{ComponentConfig, FitImageConfig};
pub use fdt_header::{FdtHeader, MemReserveEntry, FDT_LAST_COMP_VERSION, FDT_MAGIC, FDT_VERSION};
pub use fdt_tokens::{FdtToken, FdtTokenUtils, FDT_STRUCT_ALIGN};
pub use standard_dt_builder::StandardFdtBuilder;
pub use string_table::StringTable;
