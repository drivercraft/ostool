//! U-Boot FIT image generation.

use std::path::{Path, PathBuf};

use anyhow::Context;
use byte_unit::Byte;
use fdt_edit::Fdt;
use fitimage::{ComponentConfig, FitImageBuilder, FitImageConfig};
use log::{info, warn};
use object::Architecture;
use tokio::fs;

use crate::{
    artifact::{
        elf_metadata::ElfMetadata,
        runtime::{PreparedRuntimeArtifacts, RuntimeArtifactOptions, prepare_runtime_artifacts},
    },
    boot::fit_config::{FitConfig, FitFormat, FitOs, loongarch_image_entry},
    process::ProcessContext,
    utils::PathResultExt,
};

const KERNEL_COMPONENT_NAME: &str = "kernel";
const FDT_COMPONENT_NAME: &str = "fdt";
const RAMDISK_COMPONENT_NAME: &str = "ramdisk";
const DEFAULT_CONFIG_NAME: &str = "config-ostool";
const FIT_DESCRIPTION: &str = "Various kernels, ramdisks and FDT blobs";
const FIT_IMAGE_NAME: &str = "image.fit";

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
    pub(crate) initramfs_path: Option<PathBuf>,
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

/// Builds a configured payload from one prepared ELF selection. Paths in the
/// configuration are already resolved by the caller; this does not run a guest.
/// Symbols come from the original ELF, even when the runtime copy is stripped.
#[allow(dead_code)] // Consumed by the boot-prepare entry point in the next change.
pub(crate) async fn generate_configured_fit(
    context: &ProcessContext,
    prepared: &PreparedRuntimeArtifacts,
    config: &FitConfig,
) -> anyhow::Result<GeneratedFitImage> {
    let source_data = fs::read(prepared.source_elf())
        .await
        .with_path("failed to read source ELF", prepared.source_elf())?;
    let metadata = ElfMetadata::parse(&source_data)
        .with_context(|| format!("invalid source ELF: {}", prepared.source_elf().display()))?;
    validate_elf_load_data(&metadata, source_data.len())?;
    fit_arch_name(metadata.arch)?;

    let runtime_data = if prepared.source_elf() == prepared.elf() {
        source_data
    } else {
        let runtime_data = fs::read(prepared.elf())
            .await
            .with_path("failed to read runtime ELF", prepared.elf())?;
        let runtime_metadata = ElfMetadata::parse(&runtime_data)
            .with_context(|| format!("invalid runtime ELF: {}", prepared.elf().display()))?;
        validate_elf_load_data(&runtime_metadata, runtime_data.len())?;
        if metadata.arch != runtime_metadata.arch
            || metadata.entry != runtime_metadata.entry
            || metadata.load_segments != runtime_metadata.load_segments
            || metadata.load_sections != runtime_metadata.load_sections
        {
            anyhow::bail!(
                "source and runtime ELF load layouts differ; prepare the selected ELF again"
            );
        }
        runtime_data
    };
    let load = config.resolve_load(&metadata)?;
    let output = match &config.output {
        Some(path) => path.clone(),
        None => default_output_path(prepared.elf())?,
    };
    // An output alias must not destroy an input needed by subsequent preparation.
    if let Ok(destination) = output.canonicalize() {
        for input in [
            Some(prepared.source_elf()),
            Some(prepared.elf()),
            config.fdt.as_ref().map(|fdt| fdt.path.as_path()),
        ]
        .into_iter()
        .flatten()
        {
            if input.canonicalize().ok().as_ref() == Some(&destination) {
                anyhow::bail!("FIT output would overwrite an input: {}", output.display());
            }
        }
    }
    let kernel_data = match config.format {
        FitFormat::Elf => runtime_data,
        FitFormat::Bin => {
            // Derive from this runtime ELF, never reuse an unrelated/stale BIN.
            let temporary = tempfile::tempdir().context("failed to stage FIT BIN payload")?;
            let binary = prepare_runtime_artifacts(
                context,
                RuntimeArtifactOptions {
                    elf_path: prepared.elf().to_path_buf(),
                    to_bin: true,
                    bin_dir: Some(temporary.path().to_path_buf()),
                    debug: false,
                    cargo_artifact_dir: None,
                    strip_elf: false,
                },
            )
            .with_context(|| {
                format!(
                    "failed to derive FIT BIN payload with llvm-objcopy from {}",
                    prepared.elf().display()
                )
            })?;
            fs::read(binary.bin().context("BIN conversion produced no path")?)
                .await
                .context("failed to read derived FIT BIN payload")?
        }
    };
    if kernel_data.is_empty() {
        anyhow::bail!("cannot build FIT with an empty kernel payload");
    }
    // Resolve entry after payload selection so format-specific header rules see
    // the exact bytes that will be stored in the FIT image.
    let entry = config.resolve_entry(&metadata, load, &kernel_data)?;
    // Replace the output directory entry only after success. In particular, a
    // hard link to an input must not truncate that input's shared inode.
    let output_dir = output
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    #[cfg(unix)]
    let output_permissions = match fs::metadata(&output).await {
        Ok(metadata) => Some(metadata.permissions()),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => {
            return Err(error).with_path("failed to read FIT output permissions", &output);
        }
    };
    let builder = tempfile::Builder::new();
    #[cfg(unix)]
    let builder = {
        use std::os::unix::fs::PermissionsExt;
        let mut builder = builder;
        // New outputs follow normal file creation (0666 masked by umask).
        // Keep replacements private until their existing mode is restored.
        if output_permissions.is_none() {
            builder.permissions(std::fs::Permissions::from_mode(0o666));
        }
        builder
    };
    let temporary = builder
        .tempfile_in(output_dir)
        .with_path("failed to stage FIT output", &output)?;
    write_fit_image(
        FitInput {
            kernel_path: prepared.elf().to_path_buf(),
            dtb_path: config.fdt.as_ref().map(|fdt| fdt.path.clone()),
            initramfs_path: None,
            arch: metadata.arch,
            kernel_load_addr: load,
            kernel_entry_addr: entry,
            fdt_load_addr: config.fdt.as_ref().and_then(|fdt| fdt.load),
            output_path: Some(temporary.path().to_path_buf()),
        },
        kernel_data,
        config.os,
    )
    .await?;
    #[cfg(unix)]
    if let Some(permissions) = output_permissions {
        temporary
            .as_file()
            .set_permissions(permissions)
            .with_path("failed to preserve FIT output permissions", &output)?;
    }
    temporary
        .persist(&output)
        .map_err(|error| error.error)
        .with_path(errors::FIT_SAVE_ERROR, &output)?;
    info!("FIT image ok: {}", output.display());
    Ok(GeneratedFitImage { path: output })
}

// Metadata parsing reads descriptions, not the bytes they describe. Explicit
// addresses bypass inference, but cannot make truncated load data valid.
fn validate_elf_load_data(metadata: &ElfMetadata, file_size: usize) -> anyhow::Result<()> {
    let in_file = |offset: u64, size: u64| {
        offset
            .checked_add(size)
            .is_some_and(|end| end <= file_size as u64)
    };
    for segment in &metadata.load_segments {
        if segment.file_size > segment.memory_size
            || !in_file(segment.file_offset, segment.file_size)
            || segment
                .virtual_address
                .checked_add(segment.memory_size)
                .is_none()
            || segment
                .physical_address
                .checked_add(segment.memory_size)
                .is_none()
        {
            anyhow::bail!("invalid ELF PT_LOAD data range");
        }
    }
    for section in &metadata.load_sections {
        if !in_file(section.file_offset, section.size)
            || section.virtual_address.checked_add(section.size).is_none()
        {
            anyhow::bail!("invalid ELF allocated section data range");
        }
    }
    Ok(())
}

/// Existing U-Boot path: Linux payload with its historical header handling.
pub(crate) async fn generate_fit_image(mut input: FitInput) -> anyhow::Result<GeneratedFitImage> {
    info!("Making FIT image...");

    let kernel_data = fs::read(&input.kernel_path)
        .await
        .with_path(errors::KERNEL_READ_ERROR, &input.kernel_path)?;

    info!(
        "kernel: {} (size: {:.2})",
        input.kernel_path.display(),
        Byte::from(kernel_data.len())
    );

    let kernel_entry_addr =
        loongarch_image_entry(input.arch, &kernel_data, input.kernel_load_addr)?
            .unwrap_or(input.kernel_entry_addr);
    if kernel_entry_addr != input.kernel_entry_addr {
        info!("resolved LoongArch kernel entry: {kernel_entry_addr:#x}");
    }

    input.kernel_entry_addr = kernel_entry_addr;
    let image = write_fit_image(input, kernel_data, FitOs::Linux).await?;
    info!("FIT image ok: {}", image.path().display());
    Ok(image)
}

async fn write_fit_image(
    input: FitInput,
    kernel_data: Vec<u8>,
    os: FitOs,
) -> anyhow::Result<GeneratedFitImage> {
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
        Some(
            drop_stale_initrd(data)
                .with_context(|| format!("failed to prepare FIT DTB {}", dtb_path.display()))?,
        )
    } else {
        warn!("未指定 DTB 文件，将生成仅包含 kernel 的 FIT image");
        None
    };
    let initramfs_data = match input.initramfs_path.as_ref() {
        Some(path) => {
            let data = fs::read(path)
                .await
                .with_path("failed to read initramfs", path)?;
            anyhow::ensure!(!data.is_empty(), "initramfs is empty: {}", path.display());
            Some(data)
        }
        None => None,
    };

    let config = build_fit_config(
        arch_name,
        os,
        kernel_data,
        dtb_data,
        initramfs_data,
        input.kernel_load_addr,
        input.kernel_entry_addr,
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

    Ok(GeneratedFitImage { path: output_path })
}

fn drop_stale_initrd(data: Vec<u8>) -> anyhow::Result<Vec<u8>> {
    // U-Boot supplies the current FIT ramdisk range during bootm. A preset DTB
    // may still carry the range of an unrelated, earlier Linux boot.
    let mut fdt = Fdt::from_bytes(&data).context("invalid input DTB")?;
    let Some(chosen_id) = fdt.get_by_path("/chosen").map(|node| node.id()) else {
        return Ok(data);
    };
    let chosen = fdt.node_mut(chosen_id).expect("chosen node belongs to FDT");
    let start_property = chosen.remove_property("linux,initrd-start");
    let end_property = chosen.remove_property("linux,initrd-end");
    if start_property.is_none() && end_property.is_none() {
        return Ok(data);
    }
    let start = start_property
        .as_ref()
        .and_then(|property| decode_initrd_address(&property.data));
    let end = end_property
        .as_ref()
        .and_then(|property| decode_initrd_address(&property.data));
    if let (Some(start), Some(end)) = (start, end)
        && let Some(size) = end.checked_sub(start)
    {
        fdt.memory_reservations
            .retain(|reservation| reservation.address != start || reservation.size != size);
    }
    Ok(fdt.encode().as_ref().to_vec())
}

fn decode_initrd_address(data: &[u8]) -> Option<u64> {
    match data {
        [a, b, c, d] => Some(u32::from_be_bytes([*a, *b, *c, *d]) as u64),
        [a, b, c, d, e, f, g, h] => Some(u64::from_be_bytes([*a, *b, *c, *d, *e, *f, *g, *h])),
        _ => None,
    }
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

fn default_output_path(kernel_path: &Path) -> anyhow::Result<PathBuf> {
    let output_dir = kernel_path
        .parent()
        .and_then(|p| p.to_str())
        .ok_or_else(|| anyhow!("{}: {}", errors::DIR_ERROR, kernel_path.display()))?;
    Ok(Path::new(output_dir).join(FIT_IMAGE_NAME))
}

#[allow(clippy::too_many_arguments)]
fn build_fit_config(
    arch_name: &'static str,
    os: FitOs,
    kernel_data: Vec<u8>,
    dtb_data: Option<Vec<u8>>,
    initramfs_data: Option<Vec<u8>>,
    kernel_load_addr: u64,
    kernel_entry_addr: u64,
    fdt_load_addr: Option<u64>,
) -> FitImageConfig {
    let mut config = FitImageConfig::new(FIT_DESCRIPTION).with_kernel(
        ComponentConfig::new(KERNEL_COMPONENT_NAME, kernel_data)
            .with_description("This kernel")
            .with_type("kernel")
            .with_arch(arch_name)
            .with_os(os.as_str())
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

    let ramdisk_name = if let Some(data) = initramfs_data {
        config = config.with_ramdisk(
            ComponentConfig::new(RAMDISK_COMPONENT_NAME, data)
                .with_type("ramdisk")
                .with_arch(arch_name)
                .with_os("linux")
                .with_compression(false)
                // Rockchip FIT post-processing requires a load property. Zero
                // keeps FIT_LOAD_OPTIONAL_NON_ZERO ramdisks inside the FIT.
                .with_load_address(0),
        );
        Some(RAMDISK_COMPONENT_NAME)
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
            ramdisk_name,
        )
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::{
        FitInput, FitOs, build_fit_config, default_output_path, fit_arch_name,
        generate_configured_fit, loongarch_image_entry,
    };
    use crate::{
        artifact::runtime::{
            PreparedRuntimeArtifacts, RuntimeArtifactOptions, prepare_runtime_artifacts,
        },
        boot::fit_config::{FitAddress, FitConfig, FitFormat},
        process::ProcessContext,
        project::variables::VariableScope,
    };
    use fdt_edit::{Fdt, Node, Property};
    use object::Architecture;

    fn write_u16(data: &mut [u8], offset: usize, value: u16) {
        data[offset..offset + 2].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u32(data: &mut [u8], offset: usize, value: u32) {
        data[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
    }

    fn write_u64(data: &mut [u8], offset: usize, value: u64) {
        data[offset..offset + 8].copy_from_slice(&value.to_le_bytes());
    }

    fn minimal_aarch64_elf() -> Vec<u8> {
        const DATA_OFFSET: usize = 0x100;
        const STRING_OFFSET: usize = 0x180;
        const SECTION_OFFSET: usize = 0x200;
        const SECTION_SIZE: usize = 64;
        const SECTION_COUNT: usize = 3;
        let mut data = vec![0_u8; SECTION_OFFSET + SECTION_COUNT * SECTION_SIZE];
        data[..4].copy_from_slice(b"\x7fELF");
        data[4] = 2; // ELFCLASS64
        data[5] = 1; // ELFDATA2LSB
        data[6] = 1; // EV_CURRENT
        write_u16(&mut data, 16, 2); // ET_EXEC
        write_u16(&mut data, 18, 183); // EM_AARCH64
        write_u32(&mut data, 20, 1); // EV_CURRENT
        write_u64(&mut data, 24, 0x8000_0000); // e_entry
        write_u64(&mut data, 32, 64); // e_phoff
        write_u64(&mut data, 40, SECTION_OFFSET as u64); // e_shoff
        write_u16(&mut data, 52, 64); // e_ehsize
        write_u16(&mut data, 54, 56); // e_phentsize
        write_u16(&mut data, 56, 1); // e_phnum
        write_u16(&mut data, 58, SECTION_SIZE as u16); // e_shentsize
        write_u16(&mut data, 60, SECTION_COUNT as u16); // e_shnum
        write_u16(&mut data, 62, 2); // e_shstrndx

        // One file-backed executable PT_LOAD containing the section below.
        write_u32(&mut data, 64, 1); // PT_LOAD
        write_u32(&mut data, 68, 5); // PF_R | PF_X
        write_u64(&mut data, 72, DATA_OFFSET as u64); // p_offset
        write_u64(&mut data, 80, 0x8000_0000); // p_vaddr
        write_u64(&mut data, 88, 0x8000_0000); // p_paddr
        write_u64(&mut data, 96, 4); // p_filesz
        write_u64(&mut data, 104, 4); // p_memsz
        write_u64(&mut data, 112, 0x1000); // p_align

        data[DATA_OFFSET..DATA_OFFSET + 4].copy_from_slice(&[0x00, 0x00, 0x00, 0x14]);
        data[STRING_OFFSET..STRING_OFFSET + 17].copy_from_slice(b"\0.text\0.shstrtab\0");
        // A .text-like allocated section; section zero stays all zero.
        let section = SECTION_OFFSET + SECTION_SIZE;
        write_u32(&mut data, section, 1); // sh_name = ".text"
        write_u32(&mut data, section + 4, 1); // SHT_PROGBITS
        write_u64(&mut data, section + 8, 0x6); // SHF_ALLOC | SHF_EXECINSTR
        write_u64(&mut data, section + 16, 0x8000_0000); // sh_addr
        write_u64(&mut data, section + 24, DATA_OFFSET as u64); // sh_offset
        write_u64(&mut data, section + 32, 4); // sh_size
        write_u64(&mut data, section + 48, 4); // sh_addralign
        let strings = SECTION_OFFSET + 2 * SECTION_SIZE;
        write_u32(&mut data, strings, 7); // sh_name = ".shstrtab"
        write_u32(&mut data, strings + 4, 3); // SHT_STRTAB
        write_u64(&mut data, strings + 24, STRING_OFFSET as u64); // sh_offset
        write_u64(&mut data, strings + 32, 17); // sh_size
        write_u64(&mut data, strings + 48, 1); // sh_addralign
        data
    }

    fn test_process_context(root: &Path) -> ProcessContext {
        ProcessContext::new(
            root.to_path_buf(),
            root.to_path_buf(),
            VariableScope::new(root.to_path_buf(), root.to_path_buf(), root.to_path_buf()),
            None,
        )
    }

    fn fit_kernel_property(fit: &Fdt, name: &str) -> Vec<u8> {
        fit.get_by_path("/images/kernel")
            .unwrap()
            .as_node()
            .get_property(name)
            .unwrap()
            .data
            .to_vec()
    }

    fn fit_u64_property(fit: &Fdt, name: &str) -> u64 {
        u64::from_be_bytes(fit_kernel_property(fit, name).try_into().unwrap())
    }

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

        let entry = loongarch_image_entry(
            Architecture::LoongArch64,
            &kernel_data,
            0x9000_0000_9800_0000,
        )
        .unwrap()
        .unwrap();

        assert_eq!(entry, 0x9000_0000_983c_8b40);
    }

    #[tokio::test]
    async fn generates_configured_fit_from_a_real_elf_for_elf_and_bin_payloads() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("kernel.elf");
        let source_bytes = minimal_aarch64_elf();
        fs::write(&source, &source_bytes).unwrap();
        let context = test_process_context(temp.path());
        let prepared = prepare_runtime_artifacts(
            &context,
            RuntimeArtifactOptions {
                elf_path: source,
                to_bin: false,
                bin_dir: None,
                debug: false,
                cargo_artifact_dir: None,
                strip_elf: false,
            },
        )
        .unwrap();

        let elf_output = temp.path().join("kernel-elf.fit");
        let elf_config = FitConfig {
            format: FitFormat::Elf,
            output: Some(elf_output),
            ..FitConfig::default()
        };
        let elf_image = generate_configured_fit(&context, &prepared, &elf_config)
            .await
            .unwrap();
        let elf_fit = Fdt::from_bytes(&fs::read(elf_image.path()).unwrap()).unwrap();
        assert_eq!(fit_kernel_property(&elf_fit, "os"), b"linux\0");
        assert_eq!(fit_u64_property(&elf_fit, "load"), 0x7fff_ff00);
        assert_eq!(fit_u64_property(&elf_fit, "entry"), 0x8000_0000);
        assert_eq!(fit_kernel_property(&elf_fit, "data"), source_bytes);

        let bin_output = temp.path().join("kernel-bin.fit");
        let bin_config = FitConfig {
            format: FitFormat::Bin,
            output: Some(bin_output),
            ..FitConfig::default()
        };
        let bin_image = generate_configured_fit(&context, &prepared, &bin_config)
            .await
            .unwrap();
        let bin_fit = Fdt::from_bytes(&fs::read(bin_image.path()).unwrap()).unwrap();
        assert_eq!(fit_kernel_property(&bin_fit, "os"), b"linux\0");
        assert_eq!(fit_u64_property(&bin_fit, "load"), 0x8000_0000);
        assert_eq!(fit_u64_property(&bin_fit, "entry"), 0x8000_0000);
        assert_eq!(
            fit_kernel_property(&bin_fit, "data"),
            &[0x00, 0x00, 0x00, 0x14]
        );

        let elf_os_output = temp.path().join("kernel-bootelf.fit");
        let elf_os_config = FitConfig {
            format: FitFormat::Elf,
            os: FitOs::Elf,
            load: FitAddress::Explicit(0x4000_0000),
            output: Some(elf_os_output),
            ..FitConfig::default()
        };
        let elf_os_image = generate_configured_fit(&context, &prepared, &elf_os_config)
            .await
            .unwrap();
        let elf_os_fit = Fdt::from_bytes(&fs::read(elf_os_image.path()).unwrap()).unwrap();
        assert_eq!(fit_kernel_property(&elf_os_fit, "os"), b"elf\0");
        assert_eq!(fit_u64_property(&elf_os_fit, "load"), 0x4000_0000);
        assert_eq!(fit_u64_property(&elf_os_fit, "entry"), 0x4000_0000);
        assert_eq!(fit_kernel_property(&elf_os_fit, "data"), source_bytes);

        let runtime = temp.path().join("kernel-runtime.elf");
        fs::copy(prepared.source_elf(), &runtime).unwrap();
        let separate_runtime = PreparedRuntimeArtifacts::for_test(
            prepared.source_elf().to_path_buf(),
            runtime.clone(),
        );
        let separate_output = temp.path().join("kernel-separate-runtime.fit");
        let separate_config = FitConfig {
            format: FitFormat::Elf,
            output: Some(separate_output),
            ..FitConfig::default()
        };
        let separate_image = generate_configured_fit(&context, &separate_runtime, &separate_config)
            .await
            .unwrap();
        let separate_fit = Fdt::from_bytes(&fs::read(separate_image.path()).unwrap()).unwrap();
        assert_eq!(
            fit_kernel_property(&separate_fit, "data"),
            fs::read(runtime).unwrap()
        );
    }

    #[test]
    fn default_output_path_uses_kernel_directory() {
        let output_path = default_output_path(std::path::Path::new("/tmp/kernel.bin")).unwrap();

        assert_eq!(output_path, std::path::Path::new("/tmp/image.fit"));
    }

    #[test]
    fn default_fit_config_keeps_linux_kernel_defaults_without_dtb() {
        let config = build_fit_config(
            "arm64",
            FitOs::Linux,
            vec![1, 2, 3],
            None,
            None,
            0x80000,
            0x80000,
            None,
        );
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
        let config = build_fit_config(
            "riscv",
            FitOs::Linux,
            vec![1, 2, 3],
            Some(vec![4, 5, 6]),
            None,
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

    #[test]
    fn fit_links_initramfs_without_relocation_for_target_architecture() {
        let config = build_fit_config(
            "riscv",
            FitOs::Linux,
            vec![1, 2, 3],
            None,
            Some(vec![4, 5, 6]),
            0x8020_0000,
            0x8020_0000,
            None,
        );
        let bytes = fitimage::FitImageBuilder::new().build(config).unwrap();
        let fit = Fdt::from_bytes(&bytes).unwrap();
        let ramdisk = fit.get_by_path("/images/ramdisk").unwrap().as_node();
        assert_eq!(ramdisk.get_property("data").unwrap().data, [4, 5, 6]);
        assert_eq!(ramdisk.get_property("arch").unwrap().data, b"riscv\0");
        assert_eq!(ramdisk.get_property("compression").unwrap().data, b"none\0");
        assert_eq!(
            ramdisk.get_property("load").unwrap().data,
            0_u64.to_be_bytes()
        );
        let selected = fit
            .get_by_path("/configurations/config-ostool")
            .unwrap()
            .as_node();
        assert_eq!(selected.get_property("ramdisk").unwrap().data, b"ramdisk\0");
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
            initramfs_path: None,
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
        tokio::fs::write(&dtb_path, Fdt::new().encode().as_ref())
            .await
            .unwrap();

        let generated = super::generate_fit_image(FitInput {
            kernel_path,
            dtb_path: Some(dtb_path),
            initramfs_path: None,
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

    #[tokio::test]
    async fn fit_without_ramdisk_drops_stale_initrd_properties() {
        let temp = tempfile::tempdir().unwrap();
        let kernel_path = temp.path().join("kernel.bin");
        let dtb_path = temp.path().join("board.dtb");
        tokio::fs::write(&kernel_path, [1_u8, 2, 3, 4])
            .await
            .unwrap();

        let mut board = Fdt::new();
        board.memory_reservations.push(fdt_edit::MemoryReservation {
            address: 0x4610_0000,
            size: 0x05f0_0000,
        });
        board.memory_reservations.push(fdt_edit::MemoryReservation {
            address: 0x8000_0000,
            size: 0x1000,
        });
        let chosen = board.add_node(board.root_id(), Node::new("chosen"));
        let node = board.node_mut(chosen).unwrap();
        node.set_property(Property::new(
            "linux,initrd-start",
            0x4610_0000_u64.to_be_bytes().to_vec(),
        ));
        node.set_property(Property::new(
            "linux,initrd-end",
            0x4c00_0000_u64.to_be_bytes().to_vec(),
        ));
        node.set_property(Property::new("bootargs", b"root=/dev/mmcblk0p2\0".to_vec()));
        tokio::fs::write(&dtb_path, board.encode().as_ref())
            .await
            .unwrap();

        let generated = super::generate_fit_image(FitInput {
            kernel_path,
            dtb_path: Some(dtb_path),
            initramfs_path: None,
            arch: Architecture::Riscv64,
            kernel_load_addr: 0x8020_0000,
            kernel_entry_addr: 0x8020_0000,
            fdt_load_addr: None,
            output_path: None,
        })
        .await
        .unwrap();
        let fit_bytes = tokio::fs::read(generated.path()).await.unwrap();
        let fit = Fdt::from_bytes(&fit_bytes).unwrap();
        let fdt_bytes = &fit
            .get_by_path("/images/fdt")
            .unwrap()
            .as_node()
            .get_property("data")
            .unwrap()
            .data;
        let boot_fdt = Fdt::from_bytes(fdt_bytes).unwrap();
        let chosen = boot_fdt.get_by_path("/chosen").unwrap().as_node();
        assert!(chosen.get_property("linux,initrd-start").is_none());
        assert!(chosen.get_property("linux,initrd-end").is_none());
        assert_eq!(
            chosen.get_property("bootargs").unwrap().data,
            b"root=/dev/mmcblk0p2\0"
        );
        assert_eq!(boot_fdt.memory_reservations.len(), 1);
        assert_eq!(boot_fdt.memory_reservations[0].address, 0x8000_0000);
    }
}
