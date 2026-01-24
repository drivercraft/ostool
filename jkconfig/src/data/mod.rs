//! Configuration data structures and schema parsing.
//!
//! This module provides the core data structures for managing JSON Schema-based
//! configuration, including:
//!
//! - Schema parsing and conversion to internal representation
//! - Configuration value management
//! - Serialization to TOML/JSON formats
//!
//! ## Architecture
//!
//! The data module is organized into several submodules:
//!
//! - [`app_data`] - Main application data container
//! - [`item`] - Individual configuration items
//! - [`menu`] - Menu structure for navigation
//! - [`oneof`] - OneOf/AnyOf schema variant handling
//! - [`schema`] - JSON Schema parsing utilities
//! - [`types`] - Element type definitions

/// Main application data container and configuration management.
pub mod app_data;

/// Individual configuration item representation.
pub mod item;

/// Menu structure for hierarchical navigation.
pub mod menu;

/// OneOf/AnyOf schema variant handling.
pub mod oneof;

/// JSON Schema parsing utilities.
pub mod schema;

/// Element type definitions for different data types.
pub mod types;

pub use app_data::AppData;
