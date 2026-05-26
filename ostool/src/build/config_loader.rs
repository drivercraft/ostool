//! Build configuration loading shared by CLI and menuconfig flows.
//!
//! Loading records the resolved `.build.toml` path and applies the current
//! someboot metadata injection policy after `jkconfig` materializes the config.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use jkconfig::data::ElementHook;

use crate::build::{
    config::{BuildConfig, BuildSystem},
    someboot,
};

/// A loaded build configuration plus the path it was loaded from.
#[derive(Debug, Clone)]
pub struct LoadedBuildConfig {
    path: PathBuf,
    config: BuildConfig,
}

impl LoadedBuildConfig {
    /// Returns the filesystem path used to load the build configuration.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Consumes the wrapper and returns the parsed build configuration.
    pub fn into_config(self) -> BuildConfig {
        self.config
    }
}

/// Resolves an explicit build config path or the workspace default `.build.toml`.
pub fn resolve_build_config_path(workspace_dir: &Path, explicit_path: Option<PathBuf>) -> PathBuf {
    explicit_path.unwrap_or_else(|| workspace_dir.join(".build.toml"))
}

/// Loads a build configuration and applies build-time metadata injections.
pub async fn load_build_config(
    workspace_dir: &Path,
    explicit_path: Option<PathBuf>,
    menu: bool,
    hooks: &[ElementHook],
    enable_someboot_build_config: bool,
) -> anyhow::Result<LoadedBuildConfig> {
    let path = resolve_build_config_path(workspace_dir, explicit_path);

    let Some(mut config): Option<BuildConfig> = jkconfig::run(path.clone(), menu, hooks)
        .await
        .with_context(|| format!("failed to load build config: {}", path.display()))?
    else {
        bail!("No build configuration obtained");
    };

    apply_someboot_build_config(workspace_dir, &mut config, enable_someboot_build_config)?;

    Ok(LoadedBuildConfig { path, config })
}

fn apply_someboot_build_config(
    workspace_dir: &Path,
    config: &mut BuildConfig,
    enable_someboot_build_config: bool,
) -> anyhow::Result<()> {
    if let BuildSystem::Cargo(cargo) = &mut config.system
        && enable_someboot_build_config
        && !cargo.disable_someboot_build_config
    {
        let manifest_path = workspace_dir.join("Cargo.toml");
        let iter = someboot::detect_build_config_for_package(
            &manifest_path,
            &cargo.package,
            &cargo.features,
            &cargo.target,
        )?
        .into_iter();
        cargo.args.extend(iter);
    }

    Ok(())
}
