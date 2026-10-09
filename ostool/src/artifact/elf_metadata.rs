use anyhow::{Context, bail};
use object::{
    Object, ObjectSection, ObjectSegment, ObjectSymbol,
    read::elf::{ElfFile, FileHeader, ProgramHeader, SectionHeader},
};

pub(crate) struct ElfMetadata {
    pub(crate) arch: object::Architecture,
    pub(crate) entry: u64,
    pub(crate) load_segments: Vec<LoadSegment>,
    /// Allocated sections with file bytes that can appear in a raw binary image.
    pub(crate) load_sections: Vec<LoadSection>,
    pub(crate) executable_start: Option<u64>,
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LoadSegment {
    pub(crate) virtual_address: u64,
    pub(crate) physical_address: u64,
    pub(crate) file_offset: u64,
    pub(crate) file_size: u64,
    pub(crate) memory_size: u64,
    pub(crate) alignment: u64,
    pub(crate) flags: u32,
}

/// A file-backed allocated section, represented in both ELF address spaces.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct LoadSection {
    pub(crate) virtual_address: u64,
    pub(crate) file_offset: u64,
    pub(crate) size: u64,
}

impl ElfMetadata {
    pub(crate) fn parse(data: &[u8]) -> anyhow::Result<Self> {
        let file = object::File::parse(data).context("failed to parse ELF")?;

        match file {
            object::File::Elf32(file) => Self::from_elf(&file),
            object::File::Elf64(file) => Self::from_elf(&file),
            _ => bail!("expected an ELF file"),
        }
    }

    fn from_elf<Elf>(file: &ElfFile<'_, Elf>) -> anyhow::Result<Self>
    where
        Elf: FileHeader,
    {
        let endian = file.endian();
        let load_segments = file
            .segments()
            .map(|segment| {
                let header = segment.elf_program_header();
                LoadSegment {
                    virtual_address: segment.address(),
                    physical_address: header.p_paddr(endian).into(),
                    file_offset: header.p_offset(endian).into(),
                    file_size: header.p_filesz(endian).into(),
                    memory_size: header.p_memsz(endian).into(),
                    alignment: header.p_align(endian).into(),
                    flags: header.p_flags(endian),
                }
            })
            .collect();

        let load_sections = file
            .sections()
            .filter_map(|section| {
                let header = section.elf_section_header();
                let flags: u64 = header.sh_flags(endian).into();
                if flags & u64::from(object::elf::SHF_ALLOC) == 0
                    || header.sh_type(endian) == object::elf::SHT_NOBITS
                {
                    return None;
                }

                let (file_offset, size) = section.file_range()?;
                (size != 0).then_some(LoadSection {
                    virtual_address: section.address(),
                    file_offset,
                    size,
                })
            })
            .collect();

        let mut executable_start = None;
        for symbol in file.symbols() {
            if symbol.name()? == "__executable_start" && symbol.is_definition() {
                executable_start = Some(symbol.address());
                break;
            }
        }

        Ok(Self {
            arch: file.architecture(),
            entry: file.entry(),
            load_segments,
            load_sections,
            executable_start,
        })
    }
}
