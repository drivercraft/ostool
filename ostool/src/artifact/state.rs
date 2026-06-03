//! Artifact state shared by build orchestration and runners.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::anyhow;

use crate::artifact::runtime::PreparedRuntimeArtifacts;

/// Runtime artifacts prepared from a build output.
#[derive(Default, Clone, Debug)]
pub struct OutputArtifacts {
    cargo: Option<CargoArtifactState>,
    runtime: RuntimeArtifactState,
    #[allow(dead_code)]
    debug: DebugArtifactRegistry,
}

#[derive(Clone, Debug)]
struct CargoArtifactState {
    #[allow(dead_code)]
    elf: PathBuf,
    artifact_dir: PathBuf,
}

#[derive(Default, Clone, Debug)]
struct RuntimeArtifactState {
    elf: Option<PathBuf>,
    bin: Option<PathBuf>,
    artifact_dir: Option<PathBuf>,
    source_artifact_dir: Option<PathBuf>,
}

#[allow(dead_code)]
#[derive(Default, Clone, Debug)]
pub(crate) struct DebugArtifactRegistry {
    artifacts: BTreeMap<DebugArtifactKind, DebugArtifact>,
}

#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DebugArtifact {
    kind: DebugArtifactKind,
    path: PathBuf,
}

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum DebugArtifactKind {
    Disassembly,
    ElfInfo,
    Symbols,
}

#[allow(dead_code)]
impl DebugArtifactRegistry {
    pub(crate) fn is_empty(&self) -> bool {
        self.artifacts.is_empty()
    }

    pub(crate) fn register(&mut self, kind: DebugArtifactKind, path: PathBuf) {
        self.artifacts.insert(kind, DebugArtifact { kind, path });
    }

    pub(crate) fn get(&self, kind: DebugArtifactKind) -> Option<&Path> {
        self.artifacts
            .get(&kind)
            .map(|artifact| artifact.path.as_path())
    }
}

impl OutputArtifacts {
    /// Returns the stripped runtime ELF path, when one has been prepared.
    pub fn elf(&self) -> Option<&Path> {
        self.runtime.elf.as_deref()
    }

    /// Returns the raw binary image path, when one has been prepared.
    pub fn bin(&self) -> Option<&Path> {
        self.runtime.bin.as_deref()
    }

    /// Returns the artifact directory that produced the runtime ELF.
    #[allow(dead_code)]
    pub fn cargo_artifact_dir(&self) -> Option<&Path> {
        self.cargo
            .as_ref()
            .map(|cargo| cargo.artifact_dir.as_path())
            .or(self.runtime.source_artifact_dir.as_deref())
    }

    /// Returns the Cargo source artifact directory, when the source came from Cargo.
    pub(crate) fn cargo_source_artifact_dir(&self) -> Option<&Path> {
        self.cargo
            .as_ref()
            .map(|cargo| cargo.artifact_dir.as_path())
    }

    /// Returns the directory containing the runtime artifacts consumed by runners.
    pub fn runtime_artifact_dir(&self) -> Option<&Path> {
        self.runtime.artifact_dir.as_deref()
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
        self.runtime.artifact_dir = Some(path);
    }

    /// Replaces artifact state from a prepared runtime artifact set.
    pub(crate) fn apply_prepared_runtime_artifacts(&mut self, prepared: &PreparedRuntimeArtifacts) {
        self.cargo = prepared
            .cargo_source_artifact_dir()
            .map(|artifact_dir| CargoArtifactState {
                elf: prepared.elf().to_path_buf(),
                artifact_dir: artifact_dir.to_path_buf(),
            });
        self.runtime.elf = Some(prepared.elf().to_path_buf());
        self.runtime.bin = prepared.bin().map(PathBuf::from);
        self.runtime.artifact_dir = prepared.runtime_artifact_dir().map(PathBuf::from);
        self.runtime.source_artifact_dir = prepared.cargo_artifact_dir().map(PathBuf::from);
    }

    #[allow(dead_code)]
    pub(crate) fn debug_artifacts(&self) -> &DebugArtifactRegistry {
        &self.debug
    }

    #[cfg(test)]
    pub(crate) fn register_debug_artifact(&mut self, kind: DebugArtifactKind, path: PathBuf) {
        self.debug.register(kind, path);
    }

    #[cfg(test)]
    pub(crate) fn cargo_source_elf(&self) -> Option<&Path> {
        self.cargo.as_ref().map(|cargo| cargo.elf.as_path())
    }
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use super::{DebugArtifactKind, OutputArtifacts};

    #[test]
    fn debug_artifacts_do_not_change_runtime_image() {
        let mut artifacts = OutputArtifacts::default();
        artifacts.register_debug_artifact(
            DebugArtifactKind::Disassembly,
            PathBuf::from("/tmp/kernel.disassembly"),
        );

        assert!(artifacts.elf().is_none());
        assert!(artifacts.bin().is_none());
        assert!(artifacts.runtime_image().is_none());
        assert_eq!(
            artifacts
                .debug_artifacts()
                .get(DebugArtifactKind::Disassembly),
            Some(Path::new("/tmp/kernel.disassembly"))
        );
    }
}
