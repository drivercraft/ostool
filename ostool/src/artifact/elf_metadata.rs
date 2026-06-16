//! ELF-derived boot metadata.

use std::{fs, path::Path};

use anyhow::Context as _;
use object::{Object, ObjectSegment, ObjectSymbol};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::utils::PathResultExt;

/// Boot-relevant metadata derived from the prepared kernel ELF.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ElfBootMetadata {
    /// Object crate architecture name for the ELF file.
    pub architecture: String,
    /// ELF entry point.
    pub entry: u64,
    /// Load address used by boot artifact generation.
    pub load: u64,
    /// Address of `__executable_start`, when present.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub executable_start: Option<u64>,
    /// First loadable segment, when the object exposes one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub first_load_segment: Option<ElfLoadSegment>,
}

/// Loadable segment summary used as boot metadata evidence.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct ElfLoadSegment {
    pub address: u64,
    pub size: u64,
}

pub(crate) fn read_elf_boot_metadata(path: &Path) -> anyhow::Result<ElfBootMetadata> {
    let data = fs::read(path).with_path("failed to read ELF file", path)?;
    let file = object::File::parse(data.as_slice())
        .with_context(|| format!("failed to parse ELF file: {}", path.display()))?;
    let entry = file.entry();
    let executable_start = find_executable_start(&file);
    let first_load_segment = file
        .segments()
        .filter(|segment| segment.size() > 0)
        .min_by_key(ObjectSegment::address)
        .map(|segment| ElfLoadSegment {
            address: segment.address(),
            size: segment.size(),
        });
    let load = executable_start
        .or_else(|| first_load_segment.as_ref().map(|segment| segment.address))
        .unwrap_or(entry);

    Ok(ElfBootMetadata {
        architecture: format!("{:?}", file.architecture()),
        entry,
        load,
        executable_start,
        first_load_segment,
    })
}

fn find_executable_start(file: &object::File<'_>) -> Option<u64> {
    file.symbols().find_map(|symbol| {
        let name = symbol.name().ok()?;
        (name == "__executable_start").then_some(symbol.address())
    })
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::artifact::elf_metadata::read_elf_boot_metadata;

    #[test]
    fn reads_boot_metadata_from_current_executable() {
        let temp = tempfile::tempdir().unwrap();
        let source = std::env::current_exe().unwrap();
        let elf = temp.path().join("sample-elf");
        fs::copy(source, &elf).unwrap();

        let metadata = read_elf_boot_metadata(&elf).unwrap();

        assert!(!metadata.architecture.is_empty());
        assert_eq!(
            metadata.load,
            metadata
                .executable_start
                .or_else(|| metadata
                    .first_load_segment
                    .as_ref()
                    .map(|segment| segment.address))
                .unwrap_or(metadata.entry)
        );
    }
}
