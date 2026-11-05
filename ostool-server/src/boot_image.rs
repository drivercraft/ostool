//! Architecture validation for uploaded host kernels.
use anyhow::{Context, ensure};
use httpboot_protocol::BootArch;
use object::{Architecture, BinaryFormat, Object};

pub(crate) fn elf_arch(bytes: &[u8]) -> anyhow::Result<BootArch> {
    let file = object::File::parse(bytes).context("invalid kernel ELF")?;
    ensure!(
        file.format() == BinaryFormat::Elf && file.is_64() && file.is_little_endian(),
        "HTTP Boot requires a little-endian ELF64 kernel"
    );
    match file.architecture() {
        Architecture::X86_64 => Ok(BootArch::X86_64),
        Architecture::Riscv64 => Ok(BootArch::Riscv64),
        Architecture::LoongArch64 => Ok(BootArch::Loongarch64),
        arch => anyhow::bail!("unsupported kernel ELF architecture {arch:?}"),
    }
}

#[cfg(test)]
pub(crate) fn test_elf(machine: u16) -> Vec<u8> {
    let mut bytes = vec![0; 64];
    bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
    bytes[16..18].copy_from_slice(&2u16.to_le_bytes());
    bytes[18..20].copy_from_slice(&machine.to_le_bytes());
    bytes[20..24].copy_from_slice(&1u32.to_le_bytes());
    bytes[52..54].copy_from_slice(&64u16.to_le_bytes());
    bytes[54..56].copy_from_slice(&56u16.to_le_bytes());
    bytes[58..60].copy_from_slice(&64u16.to_le_bytes());
    bytes
}
