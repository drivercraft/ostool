//! Typed internal configuration and address resolution for FIT images.

use std::path::PathBuf;

use anyhow::{Result, anyhow, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::artifact::elf_metadata::{ElfMetadata, LoadSection, LoadSegment};

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
    /// Resolves FIT addresses from the original, unstripped ELF metadata.
    pub(crate) fn resolve_addresses(&self, metadata: &ElfMetadata) -> Result<(u64, u64)> {
        let load = match self.load {
            FitAddress::Explicit(address) => address,
            FitAddress::Auto => match metadata.executable_start {
                // The linker-provided origin describes the selected project's image base.
                Some(address) => address,
                None => match self.format {
                    FitFormat::Elf => elf_load_base(metadata)?,
                    FitFormat::Bin => bin_load_base(metadata)?,
                },
            },
        };
        let entry = match self.entry {
            FitAddress::Explicit(address) => address,
            FitAddress::Auto => metadata.entry,
        };
        Ok((load, entry))
    }
}

fn elf_load_base(metadata: &ElfMetadata) -> Result<u64> {
    let mut bases = metadata
        .load_segments
        .iter()
        .filter(|segment| segment.file_size != 0)
        .map(elf_container_base);
    let first = bases.next().ok_or_else(|| {
        anyhow!("cannot derive ELF FIT load address: no file-backed PT_LOAD segment")
    })??;
    for base in bases {
        let base = base?;
        if base != first {
            bail!(
                "cannot derive ELF FIT load address: PT_LOAD segments have different file bases; set load explicitly"
            );
        }
    }
    Ok(first)
}

fn elf_container_base(segment: &LoadSegment) -> Result<u64> {
    valid_segment_range(segment)?;
    segment.physical_address.checked_sub(segment.file_offset).ok_or_else(|| {
        anyhow!(
            "cannot derive ELF FIT load address: PT_LOAD physical address {:#x} precedes file offset {:#x}; set load explicitly",
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
            Ok::<_, anyhow::Error>(Some(minimum.map_or(address, |current| current.min(address))))
        })?
        .ok_or_else(|| {
            anyhow!(
                "cannot derive BIN FIT load address: no file-backed allocated section; set load explicitly"
            )
        })
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
                .ok_or_else(|| {
                    anyhow!("cannot derive BIN FIT load address: PT_LOAD LMA overflow")
                })?,
        );
    }

    let first = candidates.first().copied().ok_or_else(|| {
        anyhow!(
            "cannot derive BIN FIT load address: an allocated file-backed section has no matching PT_LOAD segment; set load explicitly"
        )
    })?;
    if candidates.iter().any(|candidate| *candidate != first) {
        bail!(
            "cannot derive BIN FIT load address: an allocated file-backed section has ambiguous PT_LOAD LMA mappings; set load explicitly"
        );
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
        .ok_or_else(|| anyhow!("cannot derive FIT load address: {kind} overflows"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use object::Architecture;

    #[test]
    fn auto_load_distinguishes_elf_container_and_bin_payload() {
        let metadata = ElfMetadata {
            arch: Architecture::Riscv64,
            entry: 0x8000_0100,
            executable_start: None,
            load_segments: vec![LoadSegment {
                virtual_address: 0,
                physical_address: 0x8000_0000,
                file_offset: 0x100,
                file_size: 0x200,
                memory_size: 0x200,
                alignment: 1,
                flags: 0,
            }],
            load_sections: vec![LoadSection {
                virtual_address: 0x100,
                file_offset: 0x200,
                size: 0x20,
            }],
        };
        let mut config = FitConfig::default();
        assert_eq!(config.resolve_addresses(&metadata).unwrap().0, 0x8000_0100);
        config.format = FitFormat::Elf;
        assert_eq!(config.resolve_addresses(&metadata).unwrap().0, 0x7fff_ff00);
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
