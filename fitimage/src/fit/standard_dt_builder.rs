//! Standard FDT Builder for FIT images
//!
//! Creates U-Boot compatible FIT images using proper FDT structure.

use crate::error::Result;
use crate::fit::config::{ComponentConfig, FitImageConfig};
use crate::fit::{FdtHeader, FdtToken, FdtTokenUtils, MemReserveEntry, StringTable};

const FDT_DATA_ALIGNMENT: usize = 8;
const PROPERTY_PREFIX_SIZE: usize = 3 * size_of::<u32>();

/// Standard FDT builder that creates U-Boot compatible FIT images
pub struct StandardFdtBuilder {
    /// FDT header
    header: FdtHeader,
    /// String table for property names
    string_table: StringTable,
    /// Structure block buffer
    struct_buffer: Vec<u8>,
    /// Memory reserve map entries
    mem_reserve: Vec<MemReserveEntry>,
}

impl StandardFdtBuilder {
    /// Create a new standard FDT builder
    pub fn new() -> Result<Self> {
        Ok(Self {
            header: FdtHeader::new(),
            string_table: StringTable::new(),
            struct_buffer: Vec::new(),
            mem_reserve: Vec::new(),
        })
    }

    /// Build a FIT device tree from configuration
    pub fn build_fit_tree(&mut self, config: &FitImageConfig) -> Result<()> {
        // Add memory reserve entries (typically empty for FIT images)
        self.add_default_memory_reserve();

        // Build the structure block
        self.build_structure_block(config)?;

        Ok(())
    }

    /// Add default memory reserve entries
    fn add_default_memory_reserve(&mut self) {
        // For FIT images, we typically don't need memory reservations
        // Just add the terminator
        self.mem_reserve.push(MemReserveEntry::new(0, 0));
    }

    /// Build the main structure block
    fn build_structure_block(&mut self, config: &FitImageConfig) -> Result<()> {
        // Start with root node (empty name for root)
        self.begin_node("")?;

        // Add root properties to match mkimage standard
        self.add_property_u32(
            "timestamp",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as u32,
        )?;
        self.add_property_string("description", &config.description)?;
        self.add_property_u32("#address-cells", 2)?;
        self.add_property_u32("#size-cells", 1)?;

        // Add images node
        self.begin_node("images")?;
        self.add_images(config)?;
        self.end_node()?; // End images node

        // Add configurations node
        self.begin_node("configurations")?;
        self.add_configurations(config)?;
        self.end_node()?; // End configurations node

        // End root node
        self.end_node()?;

        // Add END token
        FdtToken::End.write_to_buffer(&mut self.struct_buffer);

        Ok(())
    }

    /// Add images to the structure block
    fn add_images(&mut self, config: &FitImageConfig) -> Result<()> {
        let mut component_names = Vec::new();

        // Add kernel
        if let Some(ref kernel) = config.kernel {
            // Use standard naming without prefix to match mkimage
            let node_name = kernel.name.clone();
            self.add_kernel_image(&node_name, kernel)?;
            component_names.push(("kernel", node_name));
        }

        // Add FDT
        if let Some(ref fdt) = config.fdt {
            // Use standard naming without prefix to match mkimage
            let node_name = fdt.name.clone();
            self.add_fdt_image(&node_name, fdt)?;
            component_names.push(("fdt", node_name));
        }

        // Add ramdisk
        if let Some(ref ramdisk) = config.ramdisk {
            // Use standard naming without prefix to match mkimage
            let node_name = ramdisk.name.clone();
            self.add_ramdisk_image(&node_name, ramdisk)?;
            component_names.push(("ramdisk", node_name));
        }

        Ok(())
    }

    /// Add configurations to the structure block
    fn add_configurations(&mut self, config: &FitImageConfig) -> Result<()> {
        // Add configurations from the config
        if config.configurations.is_empty() {
            // Create default configuration if none specified
            self.begin_node("config-1")?;
            self.add_property_string("description", "Default configuration")?;

            // Add component references using standard naming
            if let Some(ref kernel) = config.kernel {
                self.add_property_string("kernel", &kernel.name)?;
            }

            if let Some(ref fdt) = config.fdt {
                self.add_property_string("fdt", &fdt.name)?;
            }

            if let Some(ref ramdisk) = config.ramdisk {
                self.add_property_string("ramdisk", &ramdisk.name)?;
            }

            self.end_node()?;

            // Set default configuration reference
            self.add_property_string("default", "config-1")?;
        } else {
            // Set default configuration reference first (before config nodes)
            if let Some(ref default_config) = config.default_config {
                self.add_property_string("default", default_config)?;
            }

            // Add specified configurations
            for (config_name, val) in &config.configurations {
                self.begin_node(config_name)?;
                self.add_property_string("description", &val.description)?;

                // Add component references
                if let Some(ref kernel_ref) = val.kernel {
                    self.add_property_string("kernel", kernel_ref)?;
                }

                if let Some(ref fdt_ref) = val.fdt {
                    self.add_property_string("fdt", fdt_ref)?;
                }

                if let Some(ref ramdisk_ref) = val.ramdisk {
                    self.add_property_string("ramdisk", ramdisk_ref)?;
                }

                self.end_node()?;
            }
        }

        Ok(())
    }

    /// Add kernel image node
    fn add_kernel_image(&mut self, name: &str, component: &ComponentConfig) -> Result<()> {
        self.begin_node(name)?;

        // Use custom description if provided, otherwise default
        if let Some(ref desc) = component.description {
            self.add_property_string("description", desc)?;
        } else {
            self.add_property_string("description", "Linux Kernel")?;
        }

        // Use custom type if provided, otherwise default
        if let Some(ref type_str) = component.component_type {
            self.add_property_string("type", type_str)?;
        } else {
            self.add_property_string("type", "kernel")?;
        }

        // Use custom arch if provided, otherwise default
        if let Some(ref arch_str) = component.arch {
            self.add_property_string("arch", arch_str)?;
        } else {
            self.add_property_string("arch", "arm64")?;
        }

        // Use custom os if provided, otherwise default
        if let Some(ref os_str) = component.os {
            self.add_property_string("os", os_str)?;
        } else {
            self.add_property_string("os", "linux")?;
        }

        // Use custom compression if provided, otherwise default
        if component.compression {
            self.add_property_string("compression", "gzip")?;
        } else {
            self.add_property_string("compression", "none")?;
        }

        if let Some(load_addr) = component.load_address {
            // Use 32-bit address format for arm64 to match mkimage standard
            self.add_property_u64("load", load_addr)?;
        }

        if let Some(entry_addr) = component.entry_point {
            // Use 32-bit address format for arm64 to match mkimage standard
            self.add_property_u64("entry", entry_addr)?;
        }

        self.add_property_data("data", &component.data)?;

        // Add hash nodes to match mkimage standard
        // self.add_hash_nodes(&component.data)?;

        self.end_node()?;
        Ok(())
    }

    /// Add FDT image node
    fn add_fdt_image(&mut self, name: &str, component: &ComponentConfig) -> Result<()> {
        self.begin_node(name)?;

        // Use custom description if provided, otherwise default
        if let Some(ref desc) = component.description {
            self.add_property_string("description", desc)?;
        } else {
            self.add_property_string("description", "Device Tree Blob")?;
        }

        // Use custom type if provided, otherwise default
        if let Some(ref type_str) = component.component_type {
            self.add_property_string("type", type_str)?;
        } else {
            self.add_property_string("type", "flat_dt")?;
        }

        // Use custom arch if provided, otherwise default
        if let Some(ref arch_str) = component.arch {
            self.add_property_string("arch", arch_str)?;
        } else {
            self.add_property_string("arch", "arm64")?;
        }

        // Use custom compression if provided, otherwise default
        if component.compression {
            self.add_property_string("compression", "gzip")?;
        } else {
            self.add_property_string("compression", "none")?;
        }

        if let Some(load_addr) = component.load_address {
            // Use 32-bit address format for arm64 to match mkimage standard
            self.add_property_u64("load", load_addr)?;
        }

        self.add_fdt_property_data("data", &component.data)?;

        // Add hash nodes to match mkimage standard
        // self.add_hash_nodes(&component.data)?;

        self.end_node()?;
        Ok(())
    }

    /// Add ramdisk image node
    fn add_ramdisk_image(&mut self, name: &str, component: &ComponentConfig) -> Result<()> {
        self.begin_node(name)?;
        self.add_property_string("description", "Ramdisk Image")?;
        self.add_property_string("type", "ramdisk")?;
        self.add_property_string("arch", "arm64")?;
        self.add_property_string("os", "linux")?;
        // Use custom compression if provided, otherwise default
        if component.compression {
            self.add_property_string("compression", "gzip")?;
        } else {
            self.add_property_string("compression", "none")?;
        }

        if let Some(load_addr) = component.load_address {
            // Use 32-bit address format for arm64 to match mkimage standard
            self.add_property_u64("load", load_addr)?;
        }

        self.add_property_data("data", &component.data)?;

        // Add hash nodes to match mkimage standard
        // self.add_hash_nodes(&component.data)?;

        self.end_node()?;
        Ok(())
    }

    /// Begin a node
    fn begin_node(&mut self, name: &str) -> Result<()> {
        FdtToken::BeginNode.write_to_buffer(&mut self.struct_buffer);
        FdtTokenUtils::write_string(&mut self.struct_buffer, name)?;
        Ok(())
    }

    /// End a node
    fn end_node(&mut self) -> Result<()> {
        FdtToken::EndNode.write_to_buffer(&mut self.struct_buffer);
        Ok(())
    }

    /// Add string property
    fn add_property_string(&mut self, name: &str, value: &str) -> Result<()> {
        let name_offset = self.string_table.add_string(name);

        FdtToken::Prop.write_to_buffer(&mut self.struct_buffer);
        FdtTokenUtils::write_prop_header(
            &mut self.struct_buffer,
            value.len() as u32 + 1,
            name_offset,
        )?;
        FdtTokenUtils::write_string(&mut self.struct_buffer, value)?;
        Ok(())
    }

    /// Add u32 property
    fn add_property_u32(&mut self, name: &str, value: u32) -> Result<()> {
        let name_offset = self.string_table.add_string(name);

        FdtToken::Prop.write_to_buffer(&mut self.struct_buffer);
        FdtTokenUtils::write_prop_header(&mut self.struct_buffer, 4, name_offset)?;
        self.struct_buffer.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// Add u64 property
    fn add_property_u64(&mut self, name: &str, value: u64) -> Result<()> {
        let name_offset = self.string_table.add_string(name);

        FdtToken::Prop.write_to_buffer(&mut self.struct_buffer);
        FdtTokenUtils::write_prop_header(&mut self.struct_buffer, 8, name_offset)?;
        self.struct_buffer.extend_from_slice(&value.to_be_bytes());
        Ok(())
    }

    /// Add data property
    fn add_property_data(&mut self, name: &str, data: &[u8]) -> Result<()> {
        let name_offset = self.string_table.add_string(name);

        FdtToken::Prop.write_to_buffer(&mut self.struct_buffer);
        FdtTokenUtils::write_prop_header(&mut self.struct_buffer, data.len() as u32, name_offset)?;
        FdtTokenUtils::write_prop_data(&mut self.struct_buffer, data)?;
        Ok(())
    }

    /// Add an FDT payload at the alignment required for a nested devicetree blob.
    fn add_fdt_property_data(&mut self, name: &str, data: &[u8]) -> Result<()> {
        self.align_fdt_data();
        self.add_property_data(name, data)
    }

    fn align_fdt_data(&mut self) {
        let data_offset = FdtHeader::size()
            + self.mem_reserve.len() * MemReserveEntry::size()
            + self.struct_buffer.len()
            + PROPERTY_PREFIX_SIZE;

        if !data_offset.is_multiple_of(FDT_DATA_ALIGNMENT) {
            FdtToken::Nop.write_to_buffer(&mut self.struct_buffer);
        }
    }

    /// Finalize and return the complete FDT
    pub fn finalize(mut self) -> Result<Vec<u8>> {
        // Calculate all offsets and sizes
        let header_size = FdtHeader::size() as u32;
        let mem_rsvmap_size = (self.mem_reserve.len() * MemReserveEntry::size()) as u32;
        let struct_size = self.struct_buffer.len() as u32;
        let strings_size = self.string_table.size() as u32;

        // Calculate offsets to match mkimage layout: [Header][Mem Reserve Map][FDT Structure][String Table]
        let off_dt_struct = header_size + mem_rsvmap_size;
        let off_mem_rsvmap = header_size; // Memory reserve map comes right after header
        let off_dt_strings = off_dt_struct + struct_size;
        let total_size = off_dt_strings + strings_size;

        // Finalize header
        self.header.finalize(
            total_size,
            off_dt_struct,
            off_dt_strings,
            off_mem_rsvmap,
            strings_size,
            struct_size,
        );

        // Build final FDT with mkimage-compatible layout
        let mut result = Vec::with_capacity(total_size as usize);

        // Write header
        self.header.write_to_buffer(&mut result);

        // Write memory reserve map (comes right after header in mkimage)
        for entry in &self.mem_reserve {
            entry.write_to_buffer(&mut result);
        }

        // Write structure block
        result.extend_from_slice(&self.struct_buffer);

        // Write strings block
        result.extend_from_slice(self.string_table.data());

        Ok(result)
    }
}

impl Default for StandardFdtBuilder {
    fn default() -> Self {
        Self::new().expect("Failed to create default StandardFdtBuilder")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fit::{
        config::{ComponentConfig, FitImageConfig},
        FDT_MAGIC,
    };

    #[test]
    fn test_standard_fdt_builder() {
        let config = FitImageConfig::new("Test FIT Image")
            .with_kernel(
                ComponentConfig::new("test-kernel", vec![1, 2, 3, 4])
                    .with_load_address(0x80080000)
                    .with_entry_point(0x80080000),
            )
            .with_fdt(
                ComponentConfig::new("test-fdt", vec![5, 6, 7, 8, 9]).with_load_address(0x82000000),
            );

        let mut builder = StandardFdtBuilder::new().unwrap();
        builder.build_fit_tree(&config).unwrap();
        let fdt_data = builder.finalize().unwrap();

        // Verify we got a valid FDT
        assert!(!fdt_data.is_empty());

        // Check magic number
        assert_eq!(&fdt_data[0..4], b"\xd0\x0d\xfe\xed");

        // Verify header
        let mut header_bytes = [0u8; 40];
        header_bytes.copy_from_slice(&fdt_data[0..40]);

        // Extract magic number (big-endian)
        let magic = u32::from_be_bytes([
            header_bytes[0],
            header_bytes[1],
            header_bytes[2],
            header_bytes[3],
        ]);
        assert_eq!(magic, FDT_MAGIC);

        // Extract total size
        let total_size = u32::from_be_bytes([
            header_bytes[4],
            header_bytes[5],
            header_bytes[6],
            header_bytes[7],
        ]);
        assert_eq!(total_size as usize, fdt_data.len());
    }

    #[test]
    fn test_empty_config() {
        let config = FitImageConfig::new("Empty FIT");

        let mut builder = StandardFdtBuilder::new().unwrap();
        builder.build_fit_tree(&config).unwrap();
        let fdt_data = builder.finalize().unwrap();

        // Should still create a valid FDT structure
        assert!(!fdt_data.is_empty());
        assert_eq!(&fdt_data[0..4], b"\xd0\x0d\xfe\xed");
    }

    #[test]
    fn test_string_table_deduplication() {
        let config = FitImageConfig::new("Test FIT")
            .with_kernel(ComponentConfig::new("test", vec![1, 2, 3]).with_load_address(0x80000000))
            .with_fdt(ComponentConfig::new("test", vec![4, 5, 6]).with_load_address(0x82000000));

        let mut builder = StandardFdtBuilder::new().unwrap();
        builder.build_fit_tree(&config).unwrap();
        let fdt_data = builder.finalize().unwrap();

        // Should be valid
        assert!(!fdt_data.is_empty());
        assert_eq!(&fdt_data[0..4], b"\xd0\x0d\xfe\xed");
    }

    #[test]
    fn only_fdt_component_data_is_eight_byte_aligned() {
        let kernel_data = vec![0xa5; 17];
        let fdt_data = vec![0x5a; 19];
        let config = FitImageConfig::new("Aligned FIT")
            .with_kernel(
                ComponentConfig::new("kernel", kernel_data.clone())
                    .with_description("Kernel")
                    .with_type("kernel")
                    .with_arch("loongarch")
                    .with_os("linux")
                    .with_compression(false)
                    .with_load_address(0x9000_0000_9800_0000)
                    .with_entry_point(0x9000_0000_9800_0000),
            )
            .with_fdt(
                ComponentConfig::new("fdt", fdt_data.clone())
                    .with_description("FDT")
                    .with_type("flat_dt")
                    .with_arch("loongarch"),
            );

        let mut builder = StandardFdtBuilder::new().unwrap();
        builder.build_fit_tree(&config).unwrap();
        let fit_data = builder.finalize().unwrap();
        let kernel_offset = fit_data
            .windows(kernel_data.len())
            .position(|window| window == kernel_data)
            .unwrap();
        let fdt_offset = fit_data
            .windows(fdt_data.len())
            .position(|window| window == fdt_data)
            .unwrap();

        assert_eq!(
            kernel_offset % 8,
            4,
            "kernel data offset: {kernel_offset:#x}"
        );
        assert_eq!(fdt_offset % 8, 0, "FDT data offset: {fdt_offset:#x}");
    }
}
