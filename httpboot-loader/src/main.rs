#![cfg_attr(target_os = "uefi", no_main)]
#![cfg_attr(target_os = "uefi", no_std)]

#[cfg(not(target_os = "uefi"))]
compile_error!("the uefi-app feature must be built with a *-unknown-uefi target");

#[cfg(target_os = "uefi")]
use core::{ffi::c_void, panic::PanicInfo};

#[cfg(target_os = "uefi")]
type EfiHandle = *mut c_void;

#[cfg(target_os = "uefi")]
#[repr(transparent)]
#[derive(Clone, Copy)]
pub struct EfiStatus(usize);

#[cfg(target_os = "uefi")]
const EFI_SUCCESS: EfiStatus = EfiStatus(0);
#[cfg(target_os = "uefi")]
const EFI_UNSUPPORTED: EfiStatus = EfiStatus(3);

#[cfg(target_os = "uefi")]
#[repr(C)]
struct EfiTableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    reserved: u32,
}

#[cfg(target_os = "uefi")]
#[repr(C)]
struct EfiSimpleTextOutputProtocol {
    reset: usize,
    output_string:
        extern "efiapi" fn(this: *mut EfiSimpleTextOutputProtocol, string: *const u16) -> EfiStatus,
}

#[cfg(target_os = "uefi")]
#[repr(C)]
pub struct EfiSystemTable {
    hdr: EfiTableHeader,
    firmware_vendor: *mut u16,
    firmware_revision: u32,
    console_in_handle: EfiHandle,
    con_in: *mut c_void,
    console_out_handle: EfiHandle,
    con_out: *mut EfiSimpleTextOutputProtocol,
}

#[cfg(target_os = "uefi")]
#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(_image: EfiHandle, system_table: *mut EfiSystemTable) -> EfiStatus {
    let Some(console) = (unsafe { system_table.as_mut() }).and_then(|table| {
        let console = table.con_out;
        unsafe { console.as_mut() }
    }) else {
        return EFI_UNSUPPORTED;
    };

    write_console(console, "ostool HTTP Boot loader\r\n");
    write_console(console, "manifest parser core linked\r\n");
    write_console(
        console,
        "HTTP download and architecture jump backend are not enabled in this build\r\n",
    );

    EFI_SUCCESS
}

#[cfg(target_os = "uefi")]
fn write_console(console: *mut EfiSimpleTextOutputProtocol, message: &str) {
    let Some(console_ref) = (unsafe { console.as_mut() }) else {
        return;
    };

    let mut buffer = [0u16; 192];
    let mut index = 0;
    for unit in message.encode_utf16() {
        if index + 1 >= buffer.len() {
            break;
        }
        buffer[index] = unit;
        index += 1;
    }
    buffer[index] = 0;

    (console_ref.output_string)(console, buffer.as_ptr());
}

#[cfg(target_os = "uefi")]
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
