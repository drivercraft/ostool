//! # jkconfig
//!
//! A Cursive-based TUI component library for JSON Schema configuration.
//!
//! JKConfig automatically generates interactive terminal forms from JSON Schema
//! definitions, making configuration management intuitive and error-free.
//!
//! ## Features
//!
//! - Beautiful TUI interface built with [Cursive](https://github.com/gyscos/cursive)
//! - JSON Schema driven UI generation (Draft 2020-12)
//! - Support for multiple data types: String, Integer, Number, Boolean, Enum, Array, Object, OneOf
//! - Multi-format support: TOML and JSON configuration files
//! - Keyboard shortcuts with Vim-like keybindings
//! - Real-time type validation based on schema constraints
//! - Automatic backup before saving changes
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use jkconfig::data::AppData;
//!
//! // Load configuration with schema
//! let app_data = AppData::new(
//!     Some("config.toml"),
//!     Some("config-schema.json")
//! ).unwrap();
//!
//! // Access the configuration tree
//! let json_value = app_data.root.as_json();
//! ```
//!
//! ## Modules
//!
//! - [`data`] - Configuration data structures and schema parsing
//! - [`run`] - TUI application runner
//! - [`ui`] - UI components and editors
//! - [`web`] - Web server module (requires `web` feature)

// #[macro_use]
// extern crate log;

#[macro_use]
mod log;

/// Configuration data structures and schema parsing.
///
/// This module provides the core data structures for managing configuration
/// data, including schema parsing, value management, and serialization.
pub mod data;

// UI模块暂时注释掉，使用主程序中的 MenuView
/// TUI application runner and main entry points.
pub mod run;

/// UI components and editors for different data types.
pub mod ui;

// Web服务器模块（需要web feature）
/// Web server module for remote configuration editing.
///
/// This module is only available when the `web` feature is enabled.
#[cfg(feature = "web")]
pub mod web;

pub use run::*;
pub use serde_json::Value;
