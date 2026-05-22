//! Invocation options and project layout shared by CLI and library entrypoints.

use std::path::{Path, PathBuf};

use crate::project::{ProjectLayout, resolve_project_layout};

/// Static inputs for one CLI or library invocation.
#[derive(Clone, Debug, Default)]
pub struct InvocationOptions {
    manifest: Option<PathBuf>,
    build_dir: Option<PathBuf>,
    bin_dir: Option<PathBuf>,
    debug: bool,
}

impl InvocationOptions {
    /// Creates immutable invocation options from CLI or library inputs.
    pub fn new(
        manifest: Option<PathBuf>,
        build_dir: Option<PathBuf>,
        bin_dir: Option<PathBuf>,
        debug: bool,
    ) -> Self {
        Self {
            manifest,
            build_dir,
            bin_dir,
            debug,
        }
    }

    /// Returns the optional Cargo manifest path supplied by the caller.
    pub fn manifest(&self) -> Option<&Path> {
        self.manifest.as_deref()
    }

    /// Returns the optional build output directory supplied by the caller.
    pub fn build_dir(&self) -> Option<&Path> {
        self.build_dir.as_deref()
    }

    /// Returns the optional BIN output directory supplied by the caller.
    pub fn bin_dir(&self) -> Option<&Path> {
        self.bin_dir.as_deref()
    }

    /// Returns whether debug-mode runtime artifacts should be preserved.
    pub fn debug(&self) -> bool {
        self.debug
    }
}

/// Top-level immutable inputs plus resolved project layout.
#[derive(Clone, Debug)]
pub struct Invocation {
    options: InvocationOptions,
    project_layout: ProjectLayout,
}

impl Invocation {
    /// Resolves the project layout for this invocation.
    pub fn new(options: InvocationOptions) -> anyhow::Result<Self> {
        let project_layout = resolve_project_layout(options.manifest().map(PathBuf::from))?;
        Ok(Self {
            options,
            project_layout,
        })
    }

    /// Returns immutable options for this invocation.
    pub fn options(&self) -> &InvocationOptions {
        &self.options
    }

    /// Returns the canonical Cargo manifest path used by this invocation.
    pub fn manifest_path(&self) -> &Path {
        self.project_layout.manifest_path()
    }

    /// Returns the package directory containing the selected manifest.
    pub fn manifest_dir(&self) -> &Path {
        self.project_layout.manifest_dir()
    }

    /// Returns the Cargo workspace root from metadata.
    pub fn workspace_dir(&self) -> &Path {
        self.project_layout.workspace_dir()
    }

    pub(crate) fn into_parts(self) -> (InvocationOptions, ProjectLayout) {
        (self.options, self.project_layout)
    }
}
