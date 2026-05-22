//! Cargo manifest and workspace path resolution for ostool invocations.

use std::{
    env::current_dir,
    path::{Path, PathBuf},
};

use anyhow::{Context, anyhow, bail};

use crate::utils::PathResultExt;

/// Immutable Cargo manifest and workspace paths for one ostool invocation.
#[derive(Clone, Debug)]
pub struct ProjectLayout {
    manifest_path: PathBuf,
    manifest_dir: PathBuf,
    workspace_dir: PathBuf,
}

impl ProjectLayout {
    /// Creates a project layout from already-resolved manifest and workspace paths.
    pub(crate) fn from_manifest_parts(
        manifest_path: PathBuf,
        manifest_dir: PathBuf,
        workspace_dir: PathBuf,
    ) -> Self {
        Self {
            manifest_path,
            manifest_dir,
            workspace_dir,
        }
    }

    /// Returns the canonical Cargo manifest path used by this invocation.
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }

    /// Returns the package directory containing the selected manifest.
    pub fn manifest_dir(&self) -> &Path {
        &self.manifest_dir
    }

    /// Returns the Cargo workspace root from metadata.
    pub fn workspace_dir(&self) -> &Path {
        &self.workspace_dir
    }
}

/// Resolves manifest and workspace paths from an optional manifest or directory.
pub fn resolve_project_layout(input: Option<PathBuf>) -> anyhow::Result<ProjectLayout> {
    let manifest_path = resolve_manifest_path(input)?;
    let manifest_dir = manifest_path
        .parent()
        .ok_or_else(|| anyhow!("manifest has no parent: {}", manifest_path.display()))?
        .to_path_buf();

    let metadata = cargo_metadata::MetadataCommand::new()
        .manifest_path(&manifest_path)
        .no_deps()
        .exec()
        .with_context(|| {
            format!(
                "failed to load cargo metadata from {}",
                manifest_path.display()
            )
        })?;

    Ok(ProjectLayout {
        manifest_path,
        manifest_dir,
        workspace_dir: PathBuf::from(metadata.workspace_root.as_std_path()),
    })
}

/// Resolves a manifest path from a file, directory, or current working directory.
fn resolve_manifest_path(input: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let path = match input {
        Some(path) => path,
        None => current_dir().context("failed to get current working directory")?,
    };

    let manifest_path = if path.is_dir() {
        path.join("Cargo.toml")
    } else {
        path
    };

    if manifest_path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        bail!(
            "manifest must be a Cargo.toml file or a directory containing Cargo.toml: {}",
            manifest_path.display()
        );
    }

    if !manifest_path.exists() {
        bail!("Cargo.toml not found: {}", manifest_path.display());
    }

    manifest_path
        .canonicalize()
        .with_path("failed to canonicalize manifest path", &manifest_path)
}
