//! Prepare boot artifacts from the canonical runtime ELF.

use std::path::{Path, PathBuf};

use anyhow::Context as _;
use chrono::Utc;
use object::Architecture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{
    artifact::elf_metadata::read_elf_boot_metadata,
    boot::fit::{self, FitInput},
    build::config::BuildConfig,
    invocation::Invocation,
    utils::PathResultExt,
};

pub use crate::artifact::elf_metadata::{ElfBootMetadata, ElfLoadSegment};

const MANIFEST_VERSION: u8 = 1;
const MANIFEST_FILE_NAME: &str = "boot-artifacts.json";
const DEFAULT_FIT_FILE_NAME: &str = "image.fit";
const BOOT_SCRIPT_FILE_NAME: &str = "boot.cmd";
const ROOTFS_DIR_NAME: &str = "rootfs";
const BOOT_PARTITION_DIR_NAME: &str = "boot-partition";

/// Options for `ostool boot prepare`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootPrepareOptions {
    /// Output directory for the prepared boot package.
    pub output_dir: Option<PathBuf>,
    /// Optional DTB path to copy into the package and attach to FIT.
    pub dtb_path: Option<PathBuf>,
    /// FIT image generation options.
    pub fit: FitPrepareOptions,
    /// Create the rootfs staging directory.
    pub rootfs: bool,
    /// Create the boot partition staging directory.
    pub boot_partition: bool,
}

impl Default for BootPrepareOptions {
    fn default() -> Self {
        Self {
            output_dir: None,
            dtb_path: None,
            fit: FitPrepareOptions::default(),
            rootfs: true,
            boot_partition: true,
        }
    }
}

/// FIT image generation options for boot preparation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FitPrepareOptions {
    /// Whether to generate a FIT image.
    pub enabled: bool,
    /// Optional override for the FIT kernel `load` property.
    pub kernel_load_addr: Option<u64>,
    /// Optional override for the FIT kernel `entry` property.
    pub kernel_entry_addr: Option<u64>,
    /// Optional override for the FIT FDT `load` property.
    pub fdt_load_addr: Option<u64>,
    /// FIT kernel `os` property. Defaults to `linux` for compatibility.
    pub kernel_os: String,
}

impl Default for FitPrepareOptions {
    fn default() -> Self {
        Self {
            enabled: true,
            kernel_load_addr: None,
            kernel_entry_addr: None,
            fdt_load_addr: None,
            kernel_os: "linux".into(),
        }
    }
}

/// Stable v1 manifest for prepared boot artifacts.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct PreparedBootArtifacts {
    pub version: u8,
    pub generated_at: String,
    pub kernel_elf: PathBuf,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kernel_bin: Option<PathBuf>,
    pub elf_metadata: ElfBootMetadata,
    pub artifacts: Vec<PreparedBootArtifact>,
    #[serde(skip)]
    pub manifest_path: PathBuf,
}

impl PreparedBootArtifacts {
    pub fn manifest_path(&self) -> &Path {
        &self.manifest_path
    }
}

/// One file or directory in a prepared boot package.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct PreparedBootArtifact {
    pub kind: PreparedBootArtifactKind,
    pub path: PathBuf,
}

/// Prepared boot package artifact kind.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PreparedBootArtifactKind {
    Dtb,
    FitImage,
    BootScript,
    RootfsDir,
    BootPartitionDir,
}

/// Builds runtime artifacts, then prepares the boot artifact package.
pub async fn build_and_prepare_boot_artifacts(
    invocation: &mut Invocation,
    config: &BuildConfig,
    config_path: Option<&Path>,
    options: BootPrepareOptions,
) -> anyhow::Result<PreparedBootArtifacts> {
    crate::build::prepare_runtime_artifacts(invocation, config, config_path, false).await?;
    prepare_boot_artifacts(invocation, options).await
}

/// Prepares a boot artifact package from an already prepared runtime ELF.
pub async fn prepare_boot_artifacts(
    invocation: &mut Invocation,
    options: BootPrepareOptions,
) -> anyhow::Result<PreparedBootArtifacts> {
    let output_dir = options
        .output_dir
        .clone()
        .unwrap_or_else(|| invocation.build_dir().join("boot"));
    fs::create_dir_all(&output_dir)
        .await
        .with_path("failed to create directory", &output_dir)?;

    let kernel_elf = invocation
        .runtime_artifacts()
        .elf()
        .ok_or_else(|| anyhow!("boot prepare requires a prepared ELF artifact"))?
        .to_path_buf();
    let elf_metadata = read_elf_boot_metadata(&kernel_elf)?;
    let dtb_path = prepare_dtb(&output_dir, options.dtb_path.as_deref()).await?;
    let mut artifacts = Vec::new();

    if let Some(path) = &dtb_path {
        artifacts.push(PreparedBootArtifact {
            kind: PreparedBootArtifactKind::Dtb,
            path: path.clone(),
        });
    }

    let mut kernel_bin = invocation.runtime_artifacts().bin().map(PathBuf::from);
    let fit_path = if options.fit.enabled {
        kernel_bin = Some(invocation.ensure_runtime_bin()?);
        let generated = prepare_fit_image(
            invocation,
            &options.fit,
            &elf_metadata,
            dtb_path.clone(),
            &output_dir,
        )
        .await?;
        let path = generated.path().to_path_buf();
        artifacts.push(PreparedBootArtifact {
            kind: PreparedBootArtifactKind::FitImage,
            path: path.clone(),
        });
        Some(path)
    } else {
        None
    };

    let boot_script = prepare_boot_script(&output_dir).await?;
    artifacts.push(PreparedBootArtifact {
        kind: PreparedBootArtifactKind::BootScript,
        path: boot_script.clone(),
    });

    if options.rootfs {
        let rootfs = output_dir.join(ROOTFS_DIR_NAME);
        fs::create_dir_all(&rootfs)
            .await
            .with_path("failed to create directory", &rootfs)?;
        artifacts.push(PreparedBootArtifact {
            kind: PreparedBootArtifactKind::RootfsDir,
            path: rootfs,
        });
    }

    if options.boot_partition {
        let boot_partition = prepare_boot_partition(
            &output_dir,
            fit_path.as_deref(),
            dtb_path.as_deref(),
            &boot_script,
        )
        .await?;
        artifacts.push(PreparedBootArtifact {
            kind: PreparedBootArtifactKind::BootPartitionDir,
            path: boot_partition,
        });
    }

    let manifest_path = output_dir.join(MANIFEST_FILE_NAME);
    let manifest = PreparedBootArtifacts {
        version: MANIFEST_VERSION,
        generated_at: Utc::now().to_rfc3339(),
        kernel_elf,
        kernel_bin,
        elf_metadata,
        artifacts,
        manifest_path,
    };
    write_manifest(&manifest).await?;
    Ok(manifest)
}

async fn prepare_fit_image(
    invocation: &mut Invocation,
    options: &FitPrepareOptions,
    metadata: &ElfBootMetadata,
    dtb_path: Option<PathBuf>,
    output_dir: &Path,
) -> anyhow::Result<fit::GeneratedFitImage> {
    let arch = invocation
        .runtime_arch()
        .ok_or_else(|| anyhow!("Cannot determine architecture for FIT image generation"))?;
    reject_non_boot_fit_arch(arch)?;
    let kernel_path = invocation.ensure_runtime_bin()?;
    fit::generate_fit_image(FitInput {
        kernel_path,
        dtb_path,
        arch,
        kernel_load_addr: options.kernel_load_addr.unwrap_or(metadata.load),
        kernel_entry_addr: options.kernel_entry_addr.unwrap_or(metadata.entry),
        fdt_load_addr: options.fdt_load_addr,
        kernel_os: Some(options.kernel_os.clone()),
        output_path: Some(output_dir.join(DEFAULT_FIT_FILE_NAME)),
    })
    .await
}

fn reject_non_boot_fit_arch(arch: Architecture) -> anyhow::Result<()> {
    fit::fit_arch_name(arch).map(|_| ())
}

async fn prepare_dtb(
    output_dir: &Path,
    dtb_path: Option<&Path>,
) -> anyhow::Result<Option<PathBuf>> {
    let Some(dtb_path) = dtb_path else {
        return Ok(None);
    };
    let file_name = dtb_path
        .file_name()
        .ok_or_else(|| anyhow!("invalid DTB file path: {}", dtb_path.display()))?;
    let output_path = output_dir.join(file_name);
    fs::copy(dtb_path, &output_path).await.with_context(|| {
        format!(
            "failed to copy DTB from {} to {}",
            dtb_path.display(),
            output_path.display()
        )
    })?;
    Ok(Some(output_path))
}

async fn prepare_boot_script(output_dir: &Path) -> anyhow::Result<PathBuf> {
    let output_path = output_dir.join(BOOT_SCRIPT_FILE_NAME);
    fs::write(
        &output_path,
        "echo Loading ostool FIT image\nload mmc 0:1 ${loadaddr} /image.fit\nbootm ${loadaddr}\n",
    )
    .await
    .with_path("failed to write file", &output_path)?;
    Ok(output_path)
}

async fn prepare_boot_partition(
    output_dir: &Path,
    fit_path: Option<&Path>,
    dtb_path: Option<&Path>,
    boot_script: &Path,
) -> anyhow::Result<PathBuf> {
    let boot_partition = output_dir.join(BOOT_PARTITION_DIR_NAME);
    fs::create_dir_all(&boot_partition)
        .await
        .with_path("failed to create directory", &boot_partition)?;

    if let Some(path) = fit_path {
        copy_to_dir(path, &boot_partition, Path::new(DEFAULT_FIT_FILE_NAME)).await?;
    }
    if let Some(path) = dtb_path {
        let file_name = path
            .file_name()
            .ok_or_else(|| anyhow!("invalid DTB file path: {}", path.display()))?;
        copy_to_dir(path, &boot_partition, Path::new(file_name)).await?;
    }
    copy_to_dir(
        boot_script,
        &boot_partition,
        Path::new(BOOT_SCRIPT_FILE_NAME),
    )
    .await?;

    Ok(boot_partition)
}

async fn copy_to_dir(
    source: &Path,
    dest_dir: &Path,
    file_name: impl AsRef<Path>,
) -> anyhow::Result<PathBuf> {
    let dest = dest_dir.join(file_name);
    fs::copy(source, &dest).await.with_context(|| {
        format!(
            "failed to copy boot artifact from {} to {}",
            source.display(),
            dest.display()
        )
    })?;
    Ok(dest)
}

async fn write_manifest(manifest: &PreparedBootArtifacts) -> anyhow::Result<()> {
    let content = serde_json::to_vec_pretty(manifest)?;
    fs::write(&manifest.manifest_path, content)
        .await
        .with_path("failed to write file", &manifest.manifest_path)
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{
        artifact::{
            runtime::{RuntimeArtifactOptions, prepare_runtime_artifacts},
            state::OutputArtifacts,
        },
        boot::prepare::{
            BootPrepareOptions, FitPrepareOptions, PreparedBootArtifactKind, prepare_boot_artifacts,
        },
        invocation::{Invocation, InvocationOptions},
    };

    fn write_single_crate_manifest(dir: &std::path::Path) {
        fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(dir.join("src")).unwrap();
        fs::write(dir.join("src/lib.rs"), "").unwrap();
    }

    #[tokio::test]
    async fn prepare_boot_artifacts_writes_v1_manifest_without_fit() {
        let temp = tempfile::tempdir().unwrap();
        write_single_crate_manifest(temp.path());
        let source = std::env::current_exe().unwrap();
        let input = temp.path().join("sample-elf");
        fs::copy(source, &input).unwrap();
        let mut invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap();
        let prepared = prepare_runtime_artifacts(
            &invocation.process_context().unwrap(),
            RuntimeArtifactOptions {
                elf_path: input,
                to_bin: false,
                bin_dir: None,
                debug: false,
                cargo_artifact_dir: None,
                strip_elf: false,
                objcopy_program: "false".into(),
            },
        )
        .unwrap();
        invocation.apply_prepared_runtime_artifacts(prepared);
        let output_dir = temp.path().join("target").join("boot");
        let dtb = temp.path().join("board.dtb");
        fs::write(&dtb, [1_u8, 2, 3]).unwrap();

        let manifest = prepare_boot_artifacts(
            &mut invocation,
            BootPrepareOptions {
                output_dir: Some(output_dir.clone()),
                dtb_path: Some(dtb),
                fit: FitPrepareOptions {
                    enabled: false,
                    ..FitPrepareOptions::default()
                },
                rootfs: true,
                boot_partition: true,
            },
        )
        .await
        .unwrap();

        assert_eq!(manifest.version, 1);
        assert_eq!(
            manifest.manifest_path(),
            output_dir.join("boot-artifacts.json")
        );
        assert!(manifest.kernel_bin.is_none());
        assert!(
            manifest
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == PreparedBootArtifactKind::Dtb)
        );
        assert!(
            manifest
                .artifacts
                .iter()
                .any(|artifact| artifact.kind == PreparedBootArtifactKind::BootScript)
        );
        assert!(output_dir.join("rootfs").is_dir());
        assert!(output_dir.join("boot-partition").join("boot.cmd").is_file());

        let manifest_json = fs::read_to_string(output_dir.join("boot-artifacts.json")).unwrap();
        assert!(manifest_json.contains("\"version\": 1"));
        assert!(manifest_json.contains("\"elf_metadata\""));
    }

    #[test]
    fn output_artifacts_default_remains_empty() {
        let artifacts = OutputArtifacts::default();

        assert!(artifacts.elf().is_none());
        assert!(artifacts.bin().is_none());
    }
}
