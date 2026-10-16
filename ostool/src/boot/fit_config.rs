//! Typed internal configuration and address resolution for FIT images.

use std::{mem::size_of, path::PathBuf};

use anyhow::{Context, Result, anyhow, bail};
use object::Architecture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::artifact::elf_metadata::{ElfMetadata, LoadSection, LoadSegment};

const LOONGARCH_IMAGE_MAGIC: &[u8] = b"MZ";
const LOONGARCH_IMAGE_ENTRY_OFFSET: usize = 8;
const LOONGARCH_IMAGE_LOAD_OFFSET: usize = 24;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FitFormat {
    Elf,
    #[default]
    Bin,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FitOs {
    #[default]
    Linux,
    Elf,
}

impl FitOs {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Linux => "linux",
            Self::Elf => "elf",
        }
    }
}

/// An address supplied directly or derived from ELF metadata.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum FitAddress {
    #[default]
    Auto,
    Explicit(u64),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct FitFdt {
    pub(crate) path: PathBuf,
    pub(crate) load: Option<u64>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default, deny_unknown_fields)]
pub(crate) struct FitConfig {
    pub(crate) format: FitFormat,
    pub(crate) os: FitOs,
    pub(crate) load: FitAddress,
    pub(crate) entry: FitAddress,
    pub(crate) fdt: Option<FitFdt>,
    pub(crate) output: Option<PathBuf>,
}

impl Default for FitConfig {
    fn default() -> Self {
        Self {
            format: FitFormat::Bin,
            os: FitOs::Linux,
            load: FitAddress::Auto,
            entry: FitAddress::Auto,
            fdt: None,
            output: None,
        }
    }
}

impl FitConfig {
    fn validate(&self) -> Result<()> {
        if self.os == FitOs::Elf && self.format != FitFormat::Elf {
            bail!("FIT OS elf requires format elf and U-Boot CONFIG_BOOTM_ELF");
        }
        Ok(())
    }

    /// Resolves the payload load address from the original, unstripped ELF.
    pub(crate) fn resolve_load(&self, metadata: &ElfMetadata) -> Result<u64> {
        self.validate()?;
        match self.load {
            FitAddress::Explicit(address) => Ok(address),
            // An ELF container needs staging memory, independent of its runtime
            // sections. Metadata cannot establish a safe region on the board.
            FitAddress::Auto if self.os == FitOs::Elf => {
                bail!("FIT OS elf requires an explicit load address in safe staging memory")
            }
            FitAddress::Auto if self.format == FitFormat::Bin => {
                let base = bin_load_base(metadata)
                    .context("cannot derive BIN FIT load address; set load explicitly")?;
                if metadata
                    .executable_start
                    .is_some_and(|address| address != base)
                {
                    bail!(
                        "cannot derive BIN FIT load address: __executable_start differs from the first payload LMA; set load explicitly"
                    );
                }
                Ok(base)
            }
            FitAddress::Auto => {
                let base = elf_load_base(metadata).context("cannot derive ELF FIT load address")?;
                if metadata
                    .executable_start
                    .is_some_and(|address| address != base)
                {
                    bail!(
                        "cannot derive ELF FIT load address: __executable_start differs from the ELF container base; set load explicitly"
                    );
                }
                Ok(base)
            }
        }
    }

    /// FIT OS elf passes the ELF container address to bootelf, which reads e_entry
    /// itself. Other OS handlers consume an execution address. For `format = elf`
    /// with a Linux OS, the payload is a staged ELF container: the entry must be in
    /// a file-backed section whose LMA equals its VMA. Other sections may rely on
    /// the kernel's own early mapping or relocation code.
    pub(crate) fn resolve_entry(
        &self,
        metadata: &ElfMetadata,
        load: u64,
        payload: &[u8],
    ) -> Result<u64> {
        self.validate()?;
        if self.os == FitOs::Elf {
            if let FitAddress::Explicit(address) = self.entry
                && address != load
            {
                bail!("FIT OS elf entry must equal the ELF payload load address; use entry auto");
            }
            return Ok(load);
        }
        match self.entry {
            FitAddress::Explicit(address) => Ok(address),
            FitAddress::Auto => {
                if self.format == FitFormat::Bin
                    && self.os == FitOs::Linux
                    && let Some(entry) = loongarch_image_entry(metadata.arch, payload, load)?
                {
                    // LoongArch Linux BIN images carry their execution offset in
                    // the MZ image header. That header is authoritative for this
                    // format and intentionally replaces ELF e_entry validation.
                    return Ok(entry);
                }
                direct_entry(metadata, load, self.format)
                    .context("cannot derive FIT entry; set entry explicitly")
            }
        }
    }
}

/// Returns the execution address encoded in a LoongArch Linux image header.
/// `None` means the payload is not a LoongArch MZ image and should use the
/// normal ELF metadata rules.
pub(crate) fn loongarch_image_entry(
    arch: Architecture,
    payload: &[u8],
    load: u64,
) -> Result<Option<u64>> {
    if arch != Architecture::LoongArch64 || !payload.starts_with(LOONGARCH_IMAGE_MAGIC) {
        return Ok(None);
    }

    let image_entry = read_image_header_u64(payload, LOONGARCH_IMAGE_ENTRY_OFFSET)?;
    let image_load_addr = read_image_header_u64(payload, LOONGARCH_IMAGE_LOAD_OFFSET)?;
    let entry_offset = image_entry.checked_sub(image_load_addr).ok_or_else(|| {
        anyhow!(
            "invalid LoongArch image header: entry {image_entry:#x} precedes load address {image_load_addr:#x}"
        )
    })?;
    if entry_offset >= payload.len() as u64 {
        bail!(
            "invalid LoongArch image header: entry offset {entry_offset:#x} exceeds image size {:#x}",
            payload.len()
        );
    }

    Ok(Some(load.checked_add(entry_offset).ok_or_else(|| {
        anyhow!("LoongArch kernel entry address overflow")
    })?))
}

fn read_image_header_u64(payload: &[u8], offset: usize) -> Result<u64> {
    let bytes = payload
        .get(offset..offset + size_of::<u64>())
        .ok_or_else(|| anyhow!("truncated LoongArch image header at offset {offset:#x}"))?;
    let bytes = <[u8; size_of::<u64>()]>::try_from(bytes)
        .map_err(|_| anyhow!("invalid LoongArch image header field at offset {offset:#x}"))?;
    Ok(u64::from_le_bytes(bytes))
}

fn direct_entry(metadata: &ElfMetadata, load: u64, format: FitFormat) -> Result<u64> {
    // Only infer an entry that directly addresses its file-backed bytes at load time.
    // Startup code may map or relocate other sections, so they need not have VMA == LMA.
    // This validates the entry mapping, not the complete runtime memory layout.
    let base = match format {
        FitFormat::Bin => bin_load_base(metadata)?,
        FitFormat::Elf => elf_load_base(metadata)?,
    };
    if load != base {
        bail!("payload load address differs from its original base");
    }
    let mut found = false;
    for section in &metadata.load_sections {
        if metadata.entry >= section.virtual_address
            && metadata.entry
                < checked_end(
                    section.virtual_address,
                    section.size,
                    "section virtual range",
                )?
        {
            if section_lma(section, &metadata.load_segments)? != section.virtual_address {
                bail!("ELF entry VMA differs from its LMA");
            }
            found = true;
        }
    }
    if !found {
        bail!("ELF entry is outside the file-backed payload");
    }
    Ok(metadata.entry)
}

fn elf_load_base(metadata: &ElfMetadata) -> Result<u64> {
    // An ELF payload used as a Linux image is staged as one contiguous byte
    // container. Every file-backed PT_LOAD therefore needs the same
    // `p_paddr - p_offset` base; an explicit address cannot rearrange bytes.
    let mut segments = metadata
        .load_segments
        .iter()
        .filter(|segment| segment.file_size != 0);
    let first_segment = segments
        .next()
        .ok_or_else(|| anyhow!("no file-backed PT_LOAD segment"))?;
    let first = elf_container_base(first_segment)?;
    for segment in segments {
        let base = elf_container_base(segment)?;
        if base != first {
            bail!(
                "PT_LOAD at file offset {:#x} requires base {first:#x}, but PT_LOAD at file offset {:#x} requires base {base:#x}; no single load address preserves this layout; overriding load/entry does not rearrange payload bytes",
                first_segment.file_offset,
                segment.file_offset,
            );
        }
    }
    Ok(first)
}

fn elf_container_base(segment: &LoadSegment) -> Result<u64> {
    valid_segment_range(segment)?;
    segment
        .physical_address
        .checked_sub(segment.file_offset)
        .ok_or_else(|| {
            anyhow!(
                "PT_LOAD physical address {:#x} precedes file offset {:#x}",
                segment.physical_address,
                segment.file_offset
            )
        })
}

fn bin_load_base(metadata: &ElfMetadata) -> Result<u64> {
    metadata
        .load_sections
        .iter()
        .map(|section| section_lma(section, &metadata.load_segments))
        .try_fold(None::<u64>, |minimum, address| {
            let address = address?;
            Ok::<_, anyhow::Error>(Some(
                minimum.map_or(address, |current| current.min(address)),
            ))
        })?
        .ok_or_else(|| anyhow!("no file-backed allocated section"))
}

fn section_lma(section: &LoadSection, segments: &[LoadSegment]) -> Result<u64> {
    let section_file_end = checked_end(section.file_offset, section.size, "section file range")?;
    let section_virtual_end = checked_end(
        section.virtual_address,
        section.size,
        "section virtual range",
    )?;
    let mut candidates = Vec::new();

    for segment in segments.iter().filter(|segment| segment.file_size != 0) {
        valid_segment_range(segment)?;
        let segment_file_end =
            checked_end(segment.file_offset, segment.file_size, "PT_LOAD file range")?;
        let segment_virtual_end = checked_end(
            segment.virtual_address,
            segment.file_size,
            "PT_LOAD virtual range",
        )?;
        if section.file_offset < segment.file_offset
            || section_file_end > segment_file_end
            || section.virtual_address < segment.virtual_address
            || section_virtual_end > segment_virtual_end
        {
            continue;
        }

        let file_delta = section.file_offset - segment.file_offset;
        let virtual_delta = section.virtual_address - segment.virtual_address;
        if file_delta != virtual_delta {
            continue;
        }
        candidates.push(
            segment
                .physical_address
                .checked_add(file_delta)
                .ok_or_else(|| anyhow!("PT_LOAD LMA overflow"))?,
        );
    }

    let first = candidates.first().copied().ok_or_else(|| {
        anyhow!("an allocated file-backed section has no matching PT_LOAD segment")
    })?;
    if candidates.iter().any(|candidate| *candidate != first) {
        bail!("an allocated file-backed section has ambiguous PT_LOAD LMA mappings");
    }
    Ok(first)
}

fn valid_segment_range(segment: &LoadSegment) -> Result<()> {
    checked_end(segment.file_offset, segment.file_size, "PT_LOAD file range")?;
    checked_end(
        segment.virtual_address,
        segment.file_size,
        "PT_LOAD virtual range",
    )?;
    checked_end(
        segment.physical_address,
        segment.file_size,
        "PT_LOAD physical range",
    )?;
    Ok(())
}

fn checked_end(start: u64, size: u64, kind: &str) -> Result<u64> {
    start
        .checked_add(size)
        .ok_or_else(|| anyhow!("{kind} overflows"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use object::Architecture;

    #[test]
    fn resolves_payload_addresses_and_rejects_ambiguous_auto_values() {
        let mut metadata = ElfMetadata {
            arch: Architecture::Riscv64,
            entry: 0x8000_0100,
            executable_start: None,
            load_segments: vec![LoadSegment {
                virtual_address: 0x8000_0000,
                physical_address: 0x8000_0000,
                file_offset: 0x100,
                file_size: 0x200,
                memory_size: 0x200,
                alignment: 1,
                flags: 0,
            }],
            load_sections: vec![LoadSection {
                virtual_address: 0x8000_0100,
                file_offset: 0x200,
                size: 0x20,
            }],
        };
        let mut config = FitConfig::default();
        let load = config.resolve_load(&metadata).unwrap();
        assert_eq!(load, 0x8000_0100);
        assert_eq!(
            config.resolve_entry(&metadata, load, &[]).unwrap(),
            metadata.entry
        );
        metadata.executable_start = Some(0x9000_0100);
        assert!(config.resolve_load(&metadata).is_err());
        metadata.load_segments[0].virtual_address += 0x1000_0000;
        metadata.load_sections[0].virtual_address += 0x1000_0000;
        metadata.entry += 0x1000_0000;
        assert!(config.resolve_entry(&metadata, load, &[]).is_err());
        config.entry = FitAddress::Explicit(0);
        config.load = FitAddress::Explicit(0);
        assert_eq!(config.resolve_load(&metadata).unwrap(), 0);
        assert_eq!(config.resolve_entry(&metadata, 0, &[]).unwrap(), 0);
        config.load = FitAddress::Auto;
        metadata.executable_start = None;
        config.format = FitFormat::Elf;
        let load = config.resolve_load(&metadata).unwrap();
        assert_eq!(load, 0x7fff_ff00);
        // A VMA symbol cannot override the file base, even if it names entry.
        metadata.executable_start = Some(metadata.entry);
        assert!(config.resolve_load(&metadata).is_err());
        metadata.executable_start = Some(load);
        assert_eq!(config.resolve_load(&metadata).unwrap(), load);
        config.entry = FitAddress::Auto;
        assert!(config.resolve_entry(&metadata, load, &[]).is_err());
        config.entry = FitAddress::Explicit(metadata.entry);
        assert_eq!(
            config.resolve_entry(&metadata, load, &[]).unwrap(),
            metadata.entry
        );
        config.os = FitOs::Elf;
        assert!(config.resolve_load(&metadata).is_err());
        config.load = FitAddress::Explicit(load);
        config.entry = FitAddress::Auto;
        assert_eq!(config.resolve_entry(&metadata, load, &[]).unwrap(), load);
        config.entry = FitAddress::Explicit(metadata.entry);
        assert!(config.resolve_entry(&metadata, load, &[]).is_err());
        config.format = FitFormat::Bin;
        assert!(config.resolve_load(&metadata).is_err());

        // The same section mapping failure belongs to entry for ELF, load for BIN.
        metadata.load_sections[0].file_offset = 0x400;
        config = FitConfig {
            format: FitFormat::Elf,
            ..FitConfig::default()
        };
        assert_eq!(config.resolve_load(&metadata).unwrap(), load);
        let entry_error = config.resolve_entry(&metadata, load, &[]).unwrap_err();
        assert_eq!(
            entry_error.to_string(),
            "cannot derive FIT entry; set entry explicitly"
        );
        assert!(!format!("{entry_error:#}").contains("BIN FIT load"));
        assert!(!format!("{entry_error:#}").contains("set load explicitly"));
        config.format = FitFormat::Bin;
        let load_error = config.resolve_load(&metadata).unwrap_err();
        assert_eq!(
            load_error.to_string(),
            "cannot derive BIN FIT load address; set load explicitly"
        );
        assert_eq!(
            entry_error.root_cause().to_string(),
            load_error.root_cause().to_string()
        );
    }

    #[test]
    fn rejects_incompatible_os_and_format_from_both_resolvers() {
        let metadata = ElfMetadata {
            arch: Architecture::Aarch64,
            entry: 0,
            executable_start: None,
            load_segments: Vec::new(),
            load_sections: Vec::new(),
        };
        let config = FitConfig {
            format: FitFormat::Bin,
            os: FitOs::Elf,
            load: FitAddress::Explicit(0x4000_0000),
            ..FitConfig::default()
        };

        let load_error = config.resolve_load(&metadata).unwrap_err();
        assert_eq!(
            load_error.to_string(),
            "FIT OS elf requires format elf and U-Boot CONFIG_BOOTM_ELF"
        );
        let entry_error = config
            .resolve_entry(&metadata, 0x4000_0000, &[])
            .unwrap_err();
        assert_eq!(entry_error.to_string(), load_error.to_string());
    }

    #[test]
    fn resolves_loongarch_image_entry_in_the_configured_entry_path() {
        let metadata = ElfMetadata {
            arch: Architecture::LoongArch64,
            entry: 0x1000,
            executable_start: None,
            load_segments: Vec::new(),
            load_sections: Vec::new(),
        };
        let mut payload = vec![0_u8; 0x40_0000];
        payload[..2].copy_from_slice(b"MZ");
        payload[8..16].copy_from_slice(&0x5c_8b40_u64.to_le_bytes());
        payload[24..32].copy_from_slice(&0x20_0000_u64.to_le_bytes());
        let config = FitConfig {
            format: FitFormat::Bin,
            load: FitAddress::Explicit(0x9000_0000_9800_0000),
            ..FitConfig::default()
        };

        assert_eq!(
            config
                .resolve_entry(&metadata, 0x9000_0000_9800_0000, &payload)
                .unwrap(),
            0x9000_0000_983c_8b40
        );
    }

    #[test]
    fn allows_elf_container_gaps_with_one_file_base() {
        let metadata = ElfMetadata {
            arch: Architecture::Aarch64,
            entry: 0x8000_0100,
            executable_start: None,
            load_segments: vec![
                LoadSegment {
                    virtual_address: 0x8000_0000,
                    physical_address: 0x8000_0000,
                    file_offset: 0x100,
                    file_size: 0x200,
                    memory_size: 0x200,
                    alignment: 0x1000,
                    flags: 5,
                },
                LoadSegment {
                    virtual_address: 0x9000_0000,
                    physical_address: 0x8000_0300,
                    file_offset: 0x400,
                    file_size: 0x100,
                    memory_size: 0x100,
                    alignment: 0x1000,
                    flags: 4,
                },
            ],
            load_sections: vec![
                LoadSection {
                    virtual_address: 0x8000_0100,
                    file_offset: 0x200,
                    size: 0x20,
                },
                LoadSection {
                    virtual_address: 0x9000_0000,
                    file_offset: 0x400,
                    size: 0x20,
                },
            ],
        };
        let config = FitConfig {
            format: FitFormat::Elf,
            ..FitConfig::default()
        };

        let load = config.resolve_load(&metadata).unwrap();
        assert_eq!(load, 0x7fff_ff00);
        assert_eq!(
            config.resolve_entry(&metadata, load, &[]).unwrap(),
            metadata.entry
        );
    }

    #[test]
    fn rejects_elf_segments_with_different_container_bases() {
        let mut metadata = ElfMetadata {
            arch: Architecture::Aarch64,
            entry: 0,
            executable_start: None,
            load_segments: vec![
                LoadSegment {
                    virtual_address: 0x8000_0000,
                    physical_address: 0x8000_0100,
                    file_offset: 0x100,
                    file_size: 0x100,
                    memory_size: 0x100,
                    alignment: 0x1000,
                    flags: 5,
                },
                LoadSegment {
                    virtual_address: 0x9000_0000,
                    physical_address: 0x8000_0400,
                    file_offset: 0x400,
                    file_size: 0x100,
                    memory_size: 0x100,
                    alignment: 0x1000,
                    flags: 4,
                },
            ],
            load_sections: Vec::new(),
        };
        let config = FitConfig {
            format: FitFormat::Elf,
            ..FitConfig::default()
        };
        assert!(config.resolve_load(&metadata).is_ok());

        metadata.load_segments[1].physical_address += 0x1000;
        let error = config.resolve_load(&metadata).unwrap_err();
        assert!(format!("{error:#}").contains("no single load address preserves this layout"));
    }

    #[test]
    fn config_round_trips_json_and_toml_with_explicit_zero_and_high_addresses() {
        let config = FitConfig {
            format: FitFormat::Elf,
            os: FitOs::Elf,
            load: FitAddress::Explicit(0),
            entry: FitAddress::Explicit(0x9000_0000_8000_0000),
            fdt: Some(FitFdt {
                path: PathBuf::from("board.dtb"),
                load: Some(0),
            }),
            output: Some(PathBuf::from("boot.fit")),
        };

        let json = serde_json::to_string(&config).unwrap();
        assert_eq!(serde_json::from_str::<FitConfig>(&json).unwrap(), config);

        let toml = toml::to_string(&config).unwrap();
        assert_eq!(toml::from_str::<FitConfig>(&toml).unwrap(), config);
    }
}
