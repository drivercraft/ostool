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
use uefi::abi::{EFI_SUCCESS, EFI_UNSUPPORTED, EfiHandle, EfiStatus, EfiSystemTable};
#[cfg(target_os = "uefi")]
use uefi::console::write_console;
#[cfg(target_os = "uefi")]
use uefi::http::print_http_protocol_probe;
#[cfg(target_os = "uefi")]
use uefi::loaded_image::{LoaderError, loader_url_from_loaded_image};

#[cfg(target_os = "uefi")]
const DEVICE_PATH_BUFFER_SIZE: usize = 1024;
#[cfg(target_os = "uefi")]
const URL_BUFFER_SIZE: usize = 1024;

#[cfg(target_os = "uefi")]
#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(image: EfiHandle, system_table: *mut EfiSystemTable) -> EfiStatus {
    let Some(console) = (unsafe { system_table.as_mut() }).and_then(|table| {
        let console = table.con_out;
        unsafe { console.as_mut() }
    }) else {
        return EFI_UNSUPPORTED;
    };

    write_console(console, "ostool HTTP Boot\r\n");
    write_console(console, "manifest parser core linked\r\n");

    let mut device_path_buffer = [0u8; DEVICE_PATH_BUFFER_SIZE];
    let mut manifest_url_buffer = [0u8; URL_BUFFER_SIZE];
    let mut manifest_url = None;
    match loader_url_from_loaded_image(image, system_table, &mut device_path_buffer) {
        Ok(loader_url) => {
            write_console(console, "loader_url: ");
            write_console(console, loader_url);
            write_console(console, "\r\n");

            match write_sibling_manifest_url(loader_url, &mut manifest_url_buffer) {
                Ok(manifest_url_text) => {
                    write_console(console, "manifest_url: ");
                    write_console(console, manifest_url_text);
                    write_console(console, "\r\n");
                    manifest_url = Some(manifest_url_text);
                }
                Err(_) => write_console(console, "failed to build manifest URL\r\n"),
            }
        }
        Err(LoaderError::ProtocolUnavailable) => {
            write_console(console, "failed to open Loaded Image Protocol\r\n")
        }
        Err(LoaderError::MissingFilePath) => {
            write_console(console, "loaded image has no file path\r\n")
        }
        Err(LoaderError::DevicePathTooLarge) => {
            write_console(console, "loaded image device path is too large\r\n")
        }
        Err(LoaderError::InvalidDevicePath) => {
            write_console(console, "loaded image device path has no URI\r\n")
        }
    }

    write_console(
        console,
        "HTTP download backend is pending; manifest bytes parser linked\r\n",
    );
    print_http_protocol_probe(console, image, system_table, manifest_url);

    EFI_SUCCESS
}

#[cfg(target_os = "uefi")]
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
