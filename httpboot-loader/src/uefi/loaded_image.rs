use httpboot_loader::uri_from_device_path;

use crate::uefi::abi::{
    EFI_LOADED_IMAGE_PROTOCOL_GUID, EfiDevicePathProtocol, EfiHandle, EfiLoadedImageProtocol,
    EfiSystemTable, boot_services_from_system_table,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoaderError {
    ProtocolUnavailable,
    MissingFilePath,
    DevicePathTooLarge,
    InvalidDevicePath,
}

pub fn loader_url_from_loaded_image<'a>(
    image: EfiHandle,
    system_table: *mut EfiSystemTable,
    buffer: &'a mut [u8],
) -> Result<&'a str, LoaderError> {
    let boot_services =
        boot_services_from_system_table(system_table).ok_or(LoaderError::ProtocolUnavailable)?;

    let mut interface = core::ptr::null_mut();
    let status =
        (boot_services.handle_protocol)(image, &EFI_LOADED_IMAGE_PROTOCOL_GUID, &mut interface);
    if status.is_error() || interface.is_null() {
        return Err(LoaderError::ProtocolUnavailable);
    }

    let loaded_image = unsafe { (interface as *mut EfiLoadedImageProtocol).as_ref() }
        .ok_or(LoaderError::ProtocolUnavailable)?;
    if loaded_image.file_path.is_null() {
        return Err(LoaderError::MissingFilePath);
    }

    let size = copy_device_path_to_buffer(loaded_image.file_path, buffer)?;
    uri_from_device_path(&buffer[..size]).map_err(|_| LoaderError::InvalidDevicePath)
}

fn copy_device_path_to_buffer(
    device_path: *const EfiDevicePathProtocol,
    buffer: &mut [u8],
) -> Result<usize, LoaderError> {
    let mut offset = 0;
    let mut current = device_path;

    loop {
        let node = unsafe { current.as_ref() }.ok_or(LoaderError::MissingFilePath)?;
        let node_len = u16::from_le_bytes(node.length) as usize;
        if node_len < 4 {
            return Err(LoaderError::InvalidDevicePath);
        }
        if offset + node_len > buffer.len() {
            return Err(LoaderError::DevicePathTooLarge);
        }

        unsafe {
            core::ptr::copy_nonoverlapping(
                current as *const u8,
                buffer.as_mut_ptr().add(offset),
                node_len,
            );
        }
        offset += node_len;

        if node.node_type == 0x7f && node.node_subtype == 0xff {
            return Ok(offset);
        }

        current = unsafe { (current as *const u8).add(node_len) as *const EfiDevicePathProtocol };
    }
}
