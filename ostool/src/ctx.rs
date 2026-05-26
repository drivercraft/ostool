//! Application context and runtime state.
//!
//! This module provides the [`AppContext`] type which stores runtime state
//! and build artifacts produced while ostool is operating.

use std::path::PathBuf;

use object::Architecture;

pub use crate::artifact::state::OutputArtifacts;
use crate::build::config::BuildConfig;

/// The runtime context holding transient and final execution state.
#[derive(Default, Clone, Debug)]
pub struct AppContext {
    /// Detected CPU architecture from the ELF file.
    pub arch: Option<Architecture>,
    /// Current build configuration.
    pub build_config: Option<BuildConfig>,
    /// Path to the build configuration file.
    pub build_config_path: Option<PathBuf>,
    /// Generated build artifacts.
    pub artifacts: OutputArtifacts,
}
