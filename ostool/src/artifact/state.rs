//! Runtime artifact state shared by build orchestration and runners.

use std::path::{Path, PathBuf};

use anyhow::anyhow;

use crate::artifact::runtime::PreparedRuntimeArtifacts;

/// Runtime artifacts prepared from a build output.
#[derive(Default, Clone, Debug)]
pub struct OutputArtifacts {
    /// Path to the built ELF file.
    elf: Option<PathBuf>,
    /// Path to the converted binary file.
    bin: Option<PathBuf>,
    /// Cargo-reported directory containing the original ELF artifact.
    cargo_artifact_dir: Option<PathBuf>,
    /// Directory containing the runtime artifact consumed by runners.
    runtime_artifact_dir: Option<PathBuf>,
}

impl OutputArtifacts {
    /// Returns the stripped runtime ELF path, when one has been prepared.
    pub fn elf(&self) -> Option<&Path> {
        self.elf.as_deref()
    }

    /// Returns the raw binary image path, when one has been prepared.
    pub fn bin(&self) -> Option<&Path> {
        self.bin.as_deref()
    }

    /// Returns the Cargo artifact directory that produced the runtime ELF.
    pub fn cargo_artifact_dir(&self) -> Option<&Path> {
        self.cargo_artifact_dir.as_deref()
    }

    /// Returns the directory containing the runtime artifacts consumed by runners.
    pub fn runtime_artifact_dir(&self) -> Option<&Path> {
        self.runtime_artifact_dir.as_deref()
    }

    /// Returns whether no runtime artifact has been recorded.
    pub(crate) fn is_empty(&self) -> bool {
        self.elf.is_none()
            && self.bin.is_none()
            && self.cargo_artifact_dir.is_none()
            && self.runtime_artifact_dir.is_none()
    }

    /// Returns the preferred image path for runners that can load BIN or ELF.
    pub(crate) fn runtime_image(&self) -> Option<&Path> {
        self.bin().or_else(|| self.elf())
    }

    /// Returns the prepared BIN path or the caller-provided error message.
    pub(crate) fn require_bin(&self, message: &'static str) -> anyhow::Result<&Path> {
        self.bin().ok_or_else(|| anyhow!(message))
    }

    #[cfg(test)]
    pub(crate) fn set_runtime_artifact_dir(&mut self, path: PathBuf) {
        self.runtime_artifact_dir = Some(path);
    }

    /// Replaces artifact state from a prepared runtime artifact set.
    pub(crate) fn apply_prepared_runtime_artifacts(&mut self, prepared: &PreparedRuntimeArtifacts) {
        self.elf = Some(prepared.elf().to_path_buf());
        self.bin = prepared.bin().map(PathBuf::from);
        self.cargo_artifact_dir = prepared.cargo_artifact_dir().map(PathBuf::from);
        self.runtime_artifact_dir = prepared.runtime_artifact_dir().map(PathBuf::from);
    }
}
