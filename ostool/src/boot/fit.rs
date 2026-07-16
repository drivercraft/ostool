//! U-Boot FIT image generation.

use std::path::{Path, PathBuf};

use anyhow::Context;
use byte_unit::Byte;
use fitimage::{ComponentConfig, FitImageBuilder, FitImageConfig};
use log::{info, warn};
use object::Architecture;
use tokio::fs;

use crate::utils::PathResultExt;

const KERNEL_COMPONENT_NAME: &str = "kernel";
const FDT_COMPONENT_NAME: &str = "fdt";
const DEFAULT_CONFIG_NAME: &str = "config-ostool";
const FIT_DESCRIPTION: &str = "Various kernels, ramdisks and FDT blobs";
const FIT_IMAGE_NAME: &str = "image.fit";
const LOONGARCH_IMAGE_MAGIC: &[u8] = b"MZ";
const LOONGARCH_IMAGE_ENTRY_OFFSET: usize = 8;
const LOONGARCH_IMAGE_LOAD_OFFSET: usize = 24;

mod errors {
    pub const KERNEL_READ_ERROR: &str = "读取 kernel 文件失败";
    pub const DTB_READ_ERROR: &str = "读取 DTB 文件失败";
    pub const FIT_BUILD_ERROR: &str = "构建 FIT image 失败";
    pub const FIT_SAVE_ERROR: &str = "保存 FIT image 失败";
    pub const DIR_ERROR: &str = "无法获取 kernel 文件目录";
}

#[derive(Clone, Debug)]
pub(crate) struct FitInput {
    pub(crate) kernel_path: PathBuf,
    pub(crate) dtb_path: Option<PathBuf>,
    pub(crate) arch: Architecture,
    pub(crate) kernel_load_addr: u64,
    pub(crate) kernel_entry_addr: u64,
    pub(crate) fdt_load_addr: Option<u64>,
    pub(crate) output_path: Option<PathBuf>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct GeneratedFitImage {
    path: PathBuf,
}

impl GeneratedFitImage {
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

pub(crate) async fn generate_fit_image(input: FitInput) -> anyhow::Result<GeneratedFitImage> {
    info!("Making FIT image...");

    let kernel_data = fs::read(&input.kernel_path)
        .await
        .with_path(errors::KERNEL_READ_ERROR, &input.kernel_path)?;

    info!(
        "kernel: {} (size: {:.2})",
        input.kernel_path.display(),
        Byte::from(kernel_data.len())
    );

    let kernel_entry_addr = resolve_kernel_entry_addr(
        input.arch,
        &kernel_data,
        input.kernel_load_addr,
        input.kernel_entry_addr,
    )?;
    if kernel_entry_addr != input.kernel_entry_addr {
        info!("resolved LoongArch kernel entry: {kernel_entry_addr:#x}");
    }

    let arch_name = fit_arch_name(input.arch)?;
    let dtb_data = if let Some(dtb_path) = &input.dtb_path {
        let data = fs::read(dtb_path)
            .await
            .with_path(errors::DTB_READ_ERROR, dtb_path)?;
        info!(
            "已读取 DTB 文件: {} (大小: {:.2})",
            dtb_path.display(),
            Byte::from(data.len())
        );
        Some(data)
    } else {
        warn!("未指定 DTB 文件，将生成仅包含 kernel 的 FIT image");
        None
    };

    let config = build_default_fit_config(
        arch_name,
        kernel_data,
        dtb_data,
        input.kernel_load_addr,
        kernel_entry_addr,
        input.fdt_load_addr,
    );

    let mut builder = FitImageBuilder::new();
    let fit_data = builder
        .build(config)
        .with_context(|| errors::FIT_BUILD_ERROR.to_string())?;
    let output_path = match input.output_path {
        Some(path) => path,
        None => default_output_path(&input.kernel_path)?,
    };
    fs::write(&output_path, fit_data)
        .await
        .with_path(errors::FIT_SAVE_ERROR, &output_path)?;

    info!("FIT image ok: {}", output_path.display());
    Ok(GeneratedFitImage { path: output_path })
}

pub(crate) fn fit_arch_name(arch: Architecture) -> anyhow::Result<&'static str> {
    match arch {
        Architecture::Aarch64 => Ok("arm64"),
        Architecture::Arm => Ok("arm"),
        Architecture::LoongArch64 => Ok("loongarch"),
        Architecture::Riscv64 => Ok("riscv"),
        other => anyhow::bail!("unsupported architecture for FIT image generation: {other:?}"),
    }
}

fn resolve_kernel_entry_addr(
    arch: Architecture,
    kernel_data: &[u8],
    kernel_load_addr: u64,
    fallback_entry_addr: u64,
) -> anyhow::Result<u64> {
    if arch != Architecture::LoongArch64 || !kernel_data.starts_with(LOONGARCH_IMAGE_MAGIC) {
        return Ok(fallback_entry_addr);
    }

    let image_entry = read_image_header_u64(kernel_data, LOONGARCH_IMAGE_ENTRY_OFFSET)?;
    let image_load_addr = read_image_header_u64(kernel_data, LOONGARCH_IMAGE_LOAD_OFFSET)?;
    let entry_offset = image_entry.checked_sub(image_load_addr).ok_or_else(|| {
        anyhow!(
            "invalid LoongArch image header: entry {image_entry:#x} precedes load address {image_load_addr:#x}"
        )
    })?;
    if entry_offset >= kernel_data.len() as u64 {
        anyhow::bail!(
            "invalid LoongArch image header: entry offset {entry_offset:#x} exceeds image size {:#x}",
            kernel_data.len()
        );
    }

    kernel_load_addr
        .checked_add(entry_offset)
        .ok_or_else(|| anyhow!("LoongArch kernel entry address overflow"))
}

fn read_image_header_u64(kernel_data: &[u8], offset: usize) -> anyhow::Result<u64> {
    let bytes = kernel_data
        .get(offset..offset + size_of::<u64>())
        .ok_or_else(|| anyhow!("truncated LoongArch image header at offset {offset:#x}"))?;
    let bytes = <[u8; size_of::<u64>()]>::try_from(bytes)
        .map_err(|_| anyhow!("invalid LoongArch image header field at offset {offset:#x}"))?;
    Ok(u64::from_le_bytes(bytes))
}

fn default_output_path(kernel_path: &Path) -> anyhow::Result<PathBuf> {
    let output_dir = kernel_path
        .parent()
        .and_then(|p| p.to_str())
        .ok_or_else(|| anyhow!("{}: {}", errors::DIR_ERROR, kernel_path.display()))?;
    Ok(Path::new(output_dir).join(FIT_IMAGE_NAME))
}

fn build_default_fit_config(
    arch_name: &'static str,
    kernel_data: Vec<u8>,
    dtb_data: Option<Vec<u8>>,
    kernel_load_addr: u64,
    kernel_entry_addr: u64,
    fdt_load_addr: Option<u64>,
) -> FitImageConfig {
    let mut config = FitImageConfig::new(FIT_DESCRIPTION).with_kernel(
        ComponentConfig::new(KERNEL_COMPONENT_NAME, kernel_data)
            .with_description("This kernel")
            .with_type("kernel")
            .with_arch(arch_name)
            .with_os("linux")
            .with_compression(false)
            .with_load_address(kernel_load_addr)
            .with_entry_point(kernel_entry_addr),
    );

    let fdt_name = if let Some(data) = dtb_data {
        let mut fdt_config = ComponentConfig::new(FDT_COMPONENT_NAME, data)
            .with_description("This fdt")
            .with_type("flat_dt")
            .with_arch(arch_name);

        if let Some(addr) = fdt_load_addr {
            fdt_config = fdt_config.with_load_address(addr);
        }

        config = config.with_fdt(fdt_config);
        Some(FDT_COMPONENT_NAME)
    } else {
        None
    };

    config
        .with_default_config(DEFAULT_CONFIG_NAME)
        .with_configuration(
            DEFAULT_CONFIG_NAME,
            "ostool configuration",
            Some(KERNEL_COMPONENT_NAME),
            fdt_name,
            None::<String>,
        )
}

#[cfg(test)]
mod tests {
    use super::{
        FitInput, build_default_fit_config, default_output_path, fit_arch_name,
        resolve_kernel_entry_addr,
    };
    use object::Architecture;

    #[test]
    fn maps_supported_architectures_to_fit_names() {
        assert_eq!(fit_arch_name(Architecture::Aarch64).unwrap(), "arm64");
        assert_eq!(fit_arch_name(Architecture::Arm).unwrap(), "arm");
        assert_eq!(
            fit_arch_name(Architecture::LoongArch64).unwrap(),
            "loongarch"
        );
        assert_eq!(fit_arch_name(Architecture::Riscv64).unwrap(), "riscv");
    }

    #[test]
    fn rejects_unsupported_architecture_for_fit_generation() {
        let err = fit_arch_name(Architecture::X86_64).unwrap_err();

        assert!(
            err.to_string()
                .contains("unsupported architecture for FIT image generation: X86_64")
        );
    }

    #[test]
    fn resolves_loongarch_linux_image_entry_from_header() {
        let mut kernel_data = vec![0_u8; 0x40_0000];
        kernel_data[..2].copy_from_slice(b"MZ");
        kernel_data[8..16].copy_from_slice(&0x5c_8b40_u64.to_le_bytes());
        kernel_data[24..32].copy_from_slice(&0x20_0000_u64.to_le_bytes());

        let entry = resolve_kernel_entry_addr(
            Architecture::LoongArch64,
            &kernel_data,
            0x9000_0000_9800_0000,
            0x9000_0000_9800_0000,
        )
        .unwrap();

        assert_eq!(entry, 0x9000_0000_983c_8b40);
    }

    #[test]
    fn default_output_path_uses_kernel_directory() {
        let output_path = default_output_path(std::path::Path::new("/tmp/kernel.bin")).unwrap();

        assert_eq!(output_path, std::path::Path::new("/tmp/image.fit"));
    }

    #[test]
    fn default_fit_config_keeps_linux_kernel_defaults_without_dtb() {
        let config = build_default_fit_config("arm64", vec![1, 2, 3], None, 0x80000, 0x80000, None);
        let kernel = config.kernel.as_ref().unwrap();

        assert_eq!(
            config.description,
            "Various kernels, ramdisks and FDT blobs"
        );
        assert_eq!(kernel.name, "kernel");
        assert_eq!(kernel.component_type.as_deref(), Some("kernel"));
        assert_eq!(kernel.arch.as_deref(), Some("arm64"));
        assert_eq!(kernel.os.as_deref(), Some("linux"));
        assert!(!kernel.compression);
        assert_eq!(kernel.load_address, Some(0x80000));
        assert_eq!(kernel.entry_point, Some(0x80000));
        assert!(config.fdt.is_none());
        assert_eq!(config.default_config.as_deref(), Some("config-ostool"));
        let default_config = config.configurations.get("config-ostool").unwrap();
        assert_eq!(default_config.kernel.as_deref(), Some("kernel"));
        assert!(default_config.fdt.is_none());
        assert!(default_config.ramdisk.is_none());
    }

    #[test]
    fn default_fit_config_adds_optional_fdt_component() {
        let config = build_default_fit_config(
            "riscv",
            vec![1, 2, 3],
            Some(vec![4, 5, 6]),
            0x8020_0000,
            0x8020_0000,
            Some(0x8800_0000),
        );
        let fdt = config.fdt.as_ref().unwrap();
        let default_config = config.configurations.get("config-ostool").unwrap();

        assert_eq!(fdt.name, "fdt");
        assert_eq!(fdt.component_type.as_deref(), Some("flat_dt"));
        assert_eq!(fdt.arch.as_deref(), Some("riscv"));
        assert_eq!(fdt.load_address, Some(0x8800_0000));
        assert_eq!(default_config.fdt.as_deref(), Some("fdt"));
    }

    #[tokio::test]
    async fn generate_fit_image_writes_default_output_path() {
        let temp = tempfile::tempdir().unwrap();
        let kernel_path = temp.path().join("kernel.bin");
        tokio::fs::write(&kernel_path, [1_u8, 2, 3, 4])
            .await
            .unwrap();

        let generated = super::generate_fit_image(FitInput {
            kernel_path: kernel_path.clone(),
            dtb_path: None,
            arch: Architecture::Aarch64,
            kernel_load_addr: 0x80000,
            kernel_entry_addr: 0x80000,
            fdt_load_addr: None,
            output_path: None,
        })
        .await
        .unwrap();

        assert_eq!(generated.path(), temp.path().join("image.fit").as_path());
        let fit_data = tokio::fs::read(generated.path()).await.unwrap();
        assert!(fit_data.len() > 4);
        assert_eq!(&fit_data[..4], &[0xd0, 0x0d, 0xfe, 0xed]);
    }

    #[tokio::test]
    async fn generate_fit_image_reads_optional_dtb() {
        let temp = tempfile::tempdir().unwrap();
        let kernel_path = temp.path().join("kernel.bin");
        let dtb_path = temp.path().join("board.dtb");
        let output_path = temp.path().join("custom.fit");
        tokio::fs::write(&kernel_path, [1_u8, 2, 3, 4])
            .await
            .unwrap();
        tokio::fs::write(&dtb_path, [5_u8, 6, 7, 8]).await.unwrap();

        let generated = super::generate_fit_image(FitInput {
            kernel_path,
            dtb_path: Some(dtb_path),
            arch: Architecture::Riscv64,
            kernel_load_addr: 0x8020_0000,
            kernel_entry_addr: 0x8020_0000,
            fdt_load_addr: Some(0x8800_0000),
            output_path: Some(output_path.clone()),
        })
        .await
        .unwrap();

        assert_eq!(generated.path(), output_path.as_path());
        assert!(
            tokio::fs::metadata(generated.path())
                .await
                .unwrap()
                .is_file()
        );
    }
}
