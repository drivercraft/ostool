//! Cargo metadata helpers used by invocation and package-scoped expansion.

use std::path::PathBuf;

use anyhow::{Context, anyhow, bail};
use cargo_metadata::Metadata;

use super::ProjectLayout;

/// Loads workspace metadata for the resolved project layout.
pub fn cargo_metadata(layout: &ProjectLayout) -> anyhow::Result<Metadata> {
    cargo_metadata::MetadataCommand::new()
        .manifest_path(layout.manifest_path())
        .no_deps()
        .exec()
        .with_context(|| {
            format!(
                "failed to load cargo metadata from {}",
                layout.manifest_path().display()
            )
        })
}

/// Finds the manifest directory for a named Cargo package.
pub fn package_manifest_dir(layout: &ProjectLayout, package: &str) -> anyhow::Result<PathBuf> {
    let metadata = cargo_metadata(layout)?;
    let Some(pkg) = metadata.packages.iter().find(|pkg| pkg.name == package) else {
        bail!(
            "package '{}' not found in cargo metadata under {}",
            package,
            layout.manifest_dir().display()
        );
    };

    pkg.manifest_path
        .parent()
        .map(|path| path.as_std_path().to_path_buf())
        .ok_or_else(|| {
            anyhow!(
                "package '{}' manifest has no parent: {}",
                package,
                pkg.manifest_path
            )
        })
}
