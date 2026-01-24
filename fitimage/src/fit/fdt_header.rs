//! FDT (Flattened Device Tree) Header implementation
//!
//! Implements the standard FDT header according to the Device Tree specification.

use crate::error::Result;
use std::mem;

/// Standard FDT magic number
pub const FDT_MAGIC: u32 = 0xd00dfeed;

/// Current FDT version we support
pub const FDT_VERSION: u32 = 0x11;

/// Last compatible version
pub const FDT_LAST_COMP_VERSION: u32 = 0x10;

/// FDT header structure as defined in the Device Tree specification
/// Extended to match mkimage's 56-byte header format
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct FdtHeader {
    /// Magic number: FDT_MAGIC
    pub magic: u32,
    /// Total size of the FDT structure
    pub totalsize: u32,
    /// Offset in bytes to the structure block
    pub off_dt_struct: u32,
    /// Offset in bytes to the strings block
    pub off_dt_strings: u32,
    /// Offset in bytes to the memory reserve map
    pub off_mem_rsvmap: u32,
    /// Version of the device tree format
    pub version: u32,
    /// Last compatible version
    pub last_comp_version: u32,
    /// Boot CPU ID (physical)
    pub boot_cpuid_phys: u32,
    /// Size in bytes of the strings block
    pub size_dt_strings: u32,
    /// Size in bytes of the structure block
    pub size_dt_struct: u32,
    /// Additional fields to match mkimage's 56-byte header
    pub reserved0: u32, // Reserved/padding field
    pub reserved1: u32, // Reserved/padding field
    pub reserved2: u32, // Reserved/padding field
    pub reserved3: u32, // Reserved/padding field
}

impl FdtHeader {
    /// Create a new FDT header with placeholder values
    pub fn new() -> Self {
        Self {
            magic: FDT_MAGIC,
            totalsize: size_of::<Self>() as _, // Will be calculated later
            off_dt_struct: 0,                  // Will be calculated later
            off_dt_strings: 0,                 // Will be calculated later
            off_mem_rsvmap: 0,                 // Will be calculated later
            version: FDT_VERSION,
            last_comp_version: FDT_LAST_COMP_VERSION,
            boot_cpuid_phys: 0,
            size_dt_strings: 0, // Will be calculated later
            size_dt_struct: 0,  // Will be calculated later
            // Initialize reserved fields to zero to match mkimage
            reserved0: 0,
            reserved1: 0,
            reserved2: 0,
            reserved3: 0,
        }
    }

    /// Calculate the header size in bytes
    pub fn size() -> usize {
        mem::size_of::<FdtHeader>()
    }

    /// Write header to buffer in big-endian format
    pub fn write_to_buffer(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(&self.magic.to_be_bytes());
        buffer.extend_from_slice(&self.totalsize.to_be_bytes());
        buffer.extend_from_slice(&self.off_dt_struct.to_be_bytes());
        buffer.extend_from_slice(&self.off_dt_strings.to_be_bytes());
        buffer.extend_from_slice(&self.off_mem_rsvmap.to_be_bytes());
        buffer.extend_from_slice(&self.version.to_be_bytes());
        buffer.extend_from_slice(&self.last_comp_version.to_be_bytes());
        buffer.extend_from_slice(&self.boot_cpuid_phys.to_be_bytes());
        buffer.extend_from_slice(&self.size_dt_strings.to_be_bytes());
        buffer.extend_from_slice(&self.size_dt_struct.to_be_bytes());
        // Write reserved fields to match mkimage's 56-byte header
        buffer.extend_from_slice(&self.reserved0.to_be_bytes());
        buffer.extend_from_slice(&self.reserved1.to_be_bytes());
        buffer.extend_from_slice(&self.reserved2.to_be_bytes());
        buffer.extend_from_slice(&self.reserved3.to_be_bytes());
    }

    /// Update final values after all components are built
    pub fn finalize(
        &mut self,
        totalsize: u32,
        off_dt_struct: u32,
        off_dt_strings: u32,
        off_mem_rsvmap: u32,
        size_dt_strings: u32,
        size_dt_struct: u32,
    ) {
        self.totalsize = totalsize;
        self.off_dt_struct = off_dt_struct;
        self.off_dt_strings = off_dt_strings;
        self.off_mem_rsvmap = off_mem_rsvmap;
        self.size_dt_strings = size_dt_strings;
        self.size_dt_struct = size_dt_struct;
    }

    /// Validate header fields
    pub fn validate(&self) -> Result<()> {
        if self.magic != FDT_MAGIC {
            return Err(crate::error::MkImageError::invalid_image_data(format!(
                "Invalid magic number: expected 0x{:08x}, got 0x{:08x}",
                FDT_MAGIC, self.magic
            )));
        }

        if self.version < FDT_LAST_COMP_VERSION {
            return Err(crate::error::MkImageError::invalid_image_data(format!(
                "Unsupported version: {}, minimum supported: {}",
                self.version, FDT_LAST_COMP_VERSION
            )));
        }

        if self.totalsize < Self::size() as u32 {
            return Err(crate::error::MkImageError::invalid_image_data(
                "Total size smaller than header size".to_string(),
            ));
        }

        Ok(())
    }
}

impl Default for FdtHeader {
    fn default() -> Self {
        Self::new()
    }
}

/// Memory reserve map entry for FDT
#[repr(C)]
#[derive(Debug, Clone, Copy)]
pub struct MemReserveEntry {
    /// Physical address of reserved region
    pub address: u64,
    /// Size of reserved region
    pub size: u64,
}

impl MemReserveEntry {
    /// Create a new memory reserve entry
    pub fn new(address: u64, size: u64) -> Self {
        Self { address, size }
    }

    /// Size of the entry in bytes
    pub fn size() -> usize {
        mem::size_of::<MemReserveEntry>()
    }

    /// Write entry to buffer in big-endian format
    pub fn write_to_buffer(&self, buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(&self.address.to_be_bytes());
        buffer.extend_from_slice(&self.size.to_be_bytes());
    }

    /// Write terminator (zero address, zero size) to buffer
    pub fn write_terminator(buffer: &mut Vec<u8>) {
        buffer.extend_from_slice(&[0u8; 16]); // Two u64 zeros
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fdt_header_constants() {
        assert_eq!(FDT_MAGIC, 0xd00dfeed);
        assert_eq!(FDT_VERSION, 0x11);
        assert_eq!(FDT_LAST_COMP_VERSION, 0x10);
    }

    #[test]
    fn test_fdt_header_creation() {
        let header = FdtHeader::new();
        assert_eq!(header.magic, FDT_MAGIC);
        assert_eq!(header.version, FDT_VERSION);
        assert_eq!(header.last_comp_version, FDT_LAST_COMP_VERSION);
    }

    #[test]
    fn test_fdt_header_size() {
        assert_eq!(FdtHeader::size(), 56); // 14 * 4 bytes to match mkimage
    }

    #[test]
    fn test_fdt_header_write() {
        let header = FdtHeader::new();
        let mut buffer = Vec::new();
        header.write_to_buffer(&mut buffer);

        assert_eq!(buffer.len(), 56); // 56 bytes to match mkimage

        // Check magic number is first 4 bytes in big-endian
        assert_eq!(buffer[0..4], [0xd0, 0x0d, 0xfe, 0xed]);
    }

    #[test]
    fn test_fdt_header_finalization() {
        let mut header = FdtHeader::new();
        header.finalize(
            1000, // totalsize
            40,   // off_dt_struct (after header)
            500,  // off_dt_strings
            40,   // off_mem_rsvmap (after header)
            200,  // size_dt_strings
            400,  // size_dt_struct
        );

        assert_eq!(header.totalsize, 1000);
        assert_eq!(header.off_dt_struct, 40);
        assert_eq!(header.off_dt_strings, 500);
        assert_eq!(header.size_dt_strings, 200);
        assert_eq!(header.size_dt_struct, 400);
    }

    #[test]
    fn test_fdt_header_validation() {
        let mut header = FdtHeader::new();

        // Valid header should pass
        header.validate().unwrap();

        // Invalid magic should fail
        header.magic = 0x12345678;
        assert!(header.validate().is_err());

        // Too small total size should fail
        header.magic = FDT_MAGIC;
        header.totalsize = 20;
        assert!(header.validate().is_err());
    }

    #[test]
    fn test_mem_reserve_entry() {
        let entry = MemReserveEntry::new(0x80000000, 0x1000);

        assert_eq!(entry.address, 0x80000000);
        assert_eq!(entry.size, 0x1000);
        assert_eq!(MemReserveEntry::size(), 16); // 2 * 8 bytes
    }

    // #[test]
    // fn test_mem_reserve_entry_write() {
    //     let entry = MemReserveEntry::new(0x12345678, 0xABCDEF00);
    //     let mut buffer = Vec::new();
    //     entry.write_to_buffer(&mut buffer);

    //     assert_eq!(buffer.len(), 16);
    //     // Check big-endian format
    //     assert_eq!(buffer[0..4], [0x12, 0x34, 0x56, 0x78]);
    //     assert_eq!(buffer[8..12], [0xAB, 0xCD, 0xEF, 0x00]);
    // }
    #[test]
    fn test_mem_reserve_terminator() {
        let mut buffer = Vec::new();
        MemReserveEntry::write_terminator(&mut buffer);

        assert_eq!(buffer.len(), 16);
        assert_eq!(buffer, [0u8; 16]);
    }
}
