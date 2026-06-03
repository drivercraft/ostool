//! Build configuration loading shared by CLI and menuconfig flows.
//!
//! Loading records the resolved `.build.toml` path and materializes the
//! configuration. Callers apply CLI overrides and activate the final build
//! configuration separately.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use jkconfig::data::ElementHook;

use crate::build::config::BuildConfig;

/// Resolves an explicit build config path or the workspace default `.build.toml`.
pub fn resolve_build_config_path(workspace_dir: &Path, explicit_path: Option<PathBuf>) -> PathBuf {
    explicit_path.unwrap_or_else(|| workspace_dir.join(".build.toml"))
}

/// Loads a build configuration without activating invocation state.
pub async fn load_build_config(
    workspace_dir: &Path,
    explicit_path: Option<PathBuf>,
    menu: bool,
    hooks: &[ElementHook],
) -> anyhow::Result<BuildConfig> {
    let path = resolve_build_config_path(workspace_dir, explicit_path);

    let Some(config): Option<BuildConfig> = jkconfig::run(path.clone(), menu, hooks)
        .await
        .with_context(|| format!("failed to load build config: {}", path.display()))?
    else {
        bail!("No build configuration obtained");
    };

    Ok(config)
}
