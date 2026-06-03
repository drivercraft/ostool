#![cfg_attr(target_os = "uefi", no_main)]
#![cfg_attr(target_os = "uefi", no_std)]

#[cfg(not(target_os = "uefi"))]
compile_error!("the uefi-app feature must be built with a *-unknown-uefi target");

#[cfg(target_os = "uefi")]
use core::panic::PanicInfo;

#[cfg(target_os = "uefi")]
use httpboot::write_sibling_manifest_url;

#[cfg(target_os = "uefi")]
mod uefi;

#[cfg(target_os = "uefi")]
use uefi::abi::{EFI_UNSUPPORTED, EfiHandle, EfiStatus, EfiSystemTable};
#[cfg(target_os = "uefi")]
use uefi::console::write_console;
#[cfg(target_os = "uefi")]
use uefi::http::run_http_boot_loader;
#[cfg(target_os = "uefi")]
use uefi::loaded_image::{LoaderError, loader_url_from_loaded_image};

#[cfg(target_os = "uefi")]
const DEVICE_PATH_BUFFER_SIZE: usize = 1024;
#[cfg(target_os = "uefi")]
const URL_BUFFER_SIZE: usize = 1024;
#[cfg(target_os = "uefi")]
const EMBEDDED_MANIFEST_URL_ENV: &str = "OSTOOL_HTTPBOOT_MANIFEST_URL";

#[cfg(target_os = "uefi")]
#[unsafe(no_mangle)]
/// # Safety
///
/// The UEFI firmware must pass a valid image handle and system table pointer.
pub unsafe extern "efiapi" fn efi_main(
    image: EfiHandle,
    system_table: *mut EfiSystemTable,
) -> EfiStatus {
    let Some(console) = (unsafe { system_table.as_mut() }).and_then(|table| {
        let console = table.con_out;
        unsafe { console.as_mut() }
    }) else {
        return EFI_UNSUPPORTED;
    };

    write_console(console, "ostool HTTP Boot\r\n");

    let mut device_path_buffer = [0u8; DEVICE_PATH_BUFFER_SIZE];
    let mut manifest_url_buffer = [0u8; URL_BUFFER_SIZE];
    let mut manifest_url = None;
    let loaded_image_error =
        match loader_url_from_loaded_image(image, system_table, &mut device_path_buffer) {
            Ok(loader_url) => {
                match write_sibling_manifest_url(loader_url, &mut manifest_url_buffer) {
                    Ok(manifest_url_text) => {
                        manifest_url = Some(manifest_url_text);
                        None
                    }
                    Err(_) => Some("failed to build manifest URL"),
                }
            }
            Err(LoaderError::ProtocolUnavailable) => Some("failed to open Loaded Image Protocol"),
            Err(LoaderError::MissingFilePath) => Some("loaded image has no file path"),
            Err(LoaderError::DevicePathTooLarge) => Some("loaded image device path is too large"),
            Err(LoaderError::InvalidDevicePath) => Some("loaded image device path has no URI"),
        };

    if manifest_url.is_none() {
        match embedded_manifest_url() {
            Some(url) => {
                manifest_url = Some(url);
            }
            None => {
                write_console(console, "manifest: unavailable\r\n");
                if let Some(error) = loaded_image_error {
                    write_console(console, "reason: ");
                    write_console(console, error);
                    write_console(console, "\r\n");
                }
                write_console(console, "hint: set ");
                write_console(console, EMBEDDED_MANIFEST_URL_ENV);
                write_console(console, "\r\n");
            }
        }
    }

    if let Some(url) = manifest_url {
        write_console(console, "manifest: ");
        write_console(console, url);
        write_console(console, "\r\n");
    }

    run_http_boot_loader(console, image, system_table, manifest_url)
}

#[cfg(target_os = "uefi")]
fn embedded_manifest_url() -> Option<&'static str> {
    let url = option_env!("OSTOOL_HTTPBOOT_MANIFEST_URL")?.trim();
    if url.is_empty() { None } else { Some(url) }
}

#[cfg(target_os = "uefi")]
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
