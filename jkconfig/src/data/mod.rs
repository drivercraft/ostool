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
//! - [`app_data`] - Runtime state and persisted document types
//! - [`item`] - Individual configuration items
//! - [`menu`] - Menu structure for navigation
//! - [`oneof`] - OneOf/AnyOf schema variant handling
//! - [`path`] - Canonical element paths
//! - [`resolver`] - Shared tree lookup logic
//! - [`schema`] - JSON Schema parsing utilities
//! - [`types`] - Element type definitions
//! - [`visit`] - Tree traversal helpers

/// Runtime state and configuration document types.
pub mod app_data;

/// Hook definitions and controlled mutation APIs.
pub mod hook;

/// Individual configuration item representation.
pub mod item;

/// Menu structure for hierarchical navigation.
pub mod menu;

/// OneOf/AnyOf schema variant handling.
pub mod oneof;

/// Canonical element path support.
pub mod path;

/// Tree lookup and menu resolution.
pub mod resolver;

/// JSON Schema parsing utilities.
pub mod schema;

/// Element type definitions for different data types.
pub mod types;

/// Read-only tree traversal helpers.
pub mod visit;

pub use app_data::{AppState, ConfigDocument};
pub use hook::{
    ElementHook, HookContext, HookFlow, HookMutation, HookOption, InputBinding, InputKind,
    InputPageSpec, MessageLevel, MultiSelectBinding, MultiSelectSpec, SingleSelectBinding,
    SingleSelectSpec,
};
pub use path::ElementPath;
