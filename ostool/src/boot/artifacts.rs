//! Boot artifact staging value types.

use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use tokio::fs;

use crate::utils::PathResultExt;

const QEMU_DTB_DUMP_PATH: &str = "target/qemu.dtb";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BootArtifactKind {
    QemuDtbDump,
    FitImage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct BootArtifact {
    kind: BootArtifactKind,
    path: PathBuf,
}

impl BootArtifact {
    pub(crate) fn qemu_dtb_dump(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: BootArtifactKind::QemuDtbDump,
            path: path.into(),
        }
    }

    pub(crate) fn fit_image(path: impl Into<PathBuf>) -> Self {
        Self {
            kind: BootArtifactKind::FitImage,
            path: path.into(),
        }
    }

    pub(crate) fn kind(&self) -> BootArtifactKind {
        self.kind
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct QemuDtbDumpArtifact {
    artifact: BootArtifact,
}

impl QemuDtbDumpArtifact {
    fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            artifact: BootArtifact::qemu_dtb_dump(path),
        }
    }

    pub(crate) fn path(&self) -> &Path {
        self.artifact.path()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct StagedBootArtifact {
    bootfile: Option<String>,
    network_transfer_ready: bool,
}

impl StagedBootArtifact {
    pub(crate) fn network(bootfile: impl Into<String>) -> Self {
        Self {
            bootfile: Some(bootfile.into()),
            network_transfer_ready: true,
        }
    }

    pub(crate) fn no_network() -> Self {
        Self {
            bootfile: None,
            network_transfer_ready: false,
        }
    }

    pub(crate) fn bootfile(&self) -> Option<&str> {
        self.bootfile.as_deref()
    }

    pub(crate) fn network_transfer_ready(&self) -> bool {
        self.network_transfer_ready
    }
}

pub(crate) fn default_qemu_dtb_dump_path() -> PathBuf {
    PathBuf::from(QEMU_DTB_DUMP_PATH)
}

pub(crate) async fn prepare_qemu_dtb_dump(
    output_path: impl Into<PathBuf>,
) -> anyhow::Result<QemuDtbDumpArtifact> {
    let output_path = output_path.into();
    if let Err(err) = fs::remove_file(&output_path).await
        && err.kind() != ErrorKind::NotFound
    {
        return Err(err).with_path("failed to remove file", &output_path);
    }

    Ok(QemuDtbDumpArtifact::new(output_path))
}

#[cfg(test)]
mod tests {
    use super::{
        BootArtifact, BootArtifactKind, StagedBootArtifact, default_qemu_dtb_dump_path,
        prepare_qemu_dtb_dump,
    };

    #[test]
    fn boot_artifact_keeps_kind_and_path() {
        let fit = BootArtifact::fit_image("/tmp/image.fit");
        let dtb = BootArtifact::qemu_dtb_dump("target/qemu.dtb");

        assert_eq!(fit.kind(), BootArtifactKind::FitImage);
        assert_eq!(fit.path(), std::path::Path::new("/tmp/image.fit"));
        assert_eq!(dtb.kind(), BootArtifactKind::QemuDtbDump);
        assert_eq!(dtb.path(), std::path::Path::new("target/qemu.dtb"));
    }

    #[test]
    fn staged_boot_artifact_describes_network_transfer() {
        let network = StagedBootArtifact::network("image.fit");
        let no_network = StagedBootArtifact::no_network();

        assert_eq!(network.bootfile(), Some("image.fit"));
        assert!(network.network_transfer_ready());
        assert_eq!(no_network.bootfile(), None);
        assert!(!no_network.network_transfer_ready());
    }

    #[test]
    fn default_qemu_dtb_dump_path_matches_existing_contract() {
        assert_eq!(
            default_qemu_dtb_dump_path(),
            std::path::Path::new("target/qemu.dtb")
        );
    }

    #[tokio::test]
    async fn prepare_qemu_dtb_dump_removes_existing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("qemu.dtb");
        tokio::fs::write(&path, [1_u8, 2, 3]).await.unwrap();

        let artifact = prepare_qemu_dtb_dump(path.clone()).await.unwrap();

        assert_eq!(artifact.path(), path.as_path());
        assert_eq!(artifact.artifact.kind(), BootArtifactKind::QemuDtbDump);
        assert!(!path.exists());
    }

    #[tokio::test]
    async fn prepare_qemu_dtb_dump_ignores_missing_file() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("missing.dtb");

        let artifact = prepare_qemu_dtb_dump(path.clone()).await.unwrap();

        assert_eq!(artifact.path(), path.as_path());
        assert!(!path.exists());
    }
}
