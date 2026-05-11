#![cfg_attr(target_os = "uefi", no_main)]
#![cfg_attr(target_os = "uefi", no_std)]

#[cfg(not(target_os = "uefi"))]
compile_error!("the uefi-app feature must be built with a *-unknown-uefi target");

#[cfg(target_os = "uefi")]
use core::{ffi::c_void, panic::PanicInfo};

#[cfg(target_os = "uefi")]
use httpboot_loader::{uri_from_device_path, write_sibling_manifest_url};

#[cfg(target_os = "uefi")]
type EfiHandle = *mut c_void;

#[cfg(target_os = "uefi")]
#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EfiStatus(usize);

#[cfg(target_os = "uefi")]
const EFI_SUCCESS: EfiStatus = EfiStatus(0);
#[cfg(target_os = "uefi")]
const EFI_UNSUPPORTED: EfiStatus = EfiStatus(3);
#[cfg(target_os = "uefi")]
const EFI_LOADED_IMAGE_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x5b1b31a1,
    data2: 0x9562,
    data3: 0x11d2,
    data4: [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};
#[cfg(target_os = "uefi")]
const EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0xbdc8e6af,
    data2: 0xd9bc,
    data3: 0x4379,
    data4: [0xa7, 0x2a, 0xe0, 0xc4, 0xe7, 0x5d, 0xae, 0x1c],
};
#[cfg(target_os = "uefi")]
const EFI_HTTP_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x7a59b29b,
    data2: 0x910b,
    data3: 0x4171,
    data4: [0x82, 0x42, 0xa8, 0x5a, 0x0d, 0xf2, 0x5b, 0x5b],
};

#[cfg(target_os = "uefi")]
const DEVICE_PATH_BUFFER_SIZE: usize = 1024;
#[cfg(target_os = "uefi")]
const URL_BUFFER_SIZE: usize = 1024;

#[cfg(target_os = "uefi")]
#[repr(C)]
struct EfiGuid {
    data1: u32,
    data2: u16,
    data3: u16,
    data4: [u8; 8],
}

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
struct EfiBootServices {
    hdr: EfiTableHeader,
    _raise_tpl: usize,
    _restore_tpl: usize,
    _allocate_pages: usize,
    _free_pages: usize,
    _get_memory_map: usize,
    _allocate_pool: usize,
    free_pool: extern "efiapi" fn(buffer: *mut c_void) -> EfiStatus,
    _create_event: usize,
    _set_timer: usize,
    _wait_for_event: usize,
    _signal_event: usize,
    _close_event: usize,
    _check_event: usize,
    _install_protocol_interface: usize,
    _reinstall_protocol_interface: usize,
    _uninstall_protocol_interface: usize,
    handle_protocol: extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        interface: *mut *mut c_void,
    ) -> EfiStatus,
    _reserved: usize,
    _register_protocol_notify: usize,
    _locate_handle: usize,
    _locate_device_path: usize,
    _install_configuration_table: usize,
    _load_image: usize,
    _start_image: usize,
    _exit: usize,
    _unload_image: usize,
    _exit_boot_services: usize,
    _get_next_monotonic_count: usize,
    _stall: usize,
    _set_watchdog_timer: usize,
    _connect_controller: usize,
    _disconnect_controller: usize,
    _open_protocol: usize,
    _close_protocol: usize,
    _open_protocol_information: usize,
    _protocols_per_handle: usize,
    locate_handle_buffer: extern "efiapi" fn(
        search_type: EfiLocateSearchType,
        protocol: *const EfiGuid,
        search_key: *mut c_void,
        no_handles: *mut usize,
        buffer: *mut *mut EfiHandle,
    ) -> EfiStatus,
}

#[cfg(target_os = "uefi")]
#[repr(C)]
struct EfiDevicePathProtocol {
    node_type: u8,
    node_subtype: u8,
    length: [u8; 2],
}

#[cfg(target_os = "uefi")]
#[repr(C)]
struct EfiLoadedImageProtocol {
    revision: u32,
    parent_handle: EfiHandle,
    system_table: *mut EfiSystemTable,
    device_handle: EfiHandle,
    file_path: *const EfiDevicePathProtocol,
}

#[cfg(target_os = "uefi")]
#[repr(C)]
struct EfiServiceBindingProtocol {
    create_child: extern "efiapi" fn(
        this: *mut EfiServiceBindingProtocol,
        child_handle: *mut EfiHandle,
    ) -> EfiStatus,
    destroy_child: extern "efiapi" fn(
        this: *mut EfiServiceBindingProtocol,
        child_handle: EfiHandle,
    ) -> EfiStatus,
}

#[cfg(target_os = "uefi")]
#[repr(C)]
struct EfiHttpProtocol {
    get_mode_data: usize,
    configure: usize,
    request: usize,
    cancel: usize,
    response: usize,
    poll: usize,
}

#[cfg(target_os = "uefi")]
type EfiLocateSearchType = u32;
#[cfg(target_os = "uefi")]
const EFI_LOCATE_BY_PROTOCOL: EfiLocateSearchType = 2;

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
    standard_error_handle: EfiHandle,
    std_err: *mut EfiSimpleTextOutputProtocol,
    runtime_services: *mut c_void,
    boot_services: *mut EfiBootServices,
}

#[cfg(target_os = "uefi")]
#[unsafe(no_mangle)]
pub extern "efiapi" fn efi_main(image: EfiHandle, system_table: *mut EfiSystemTable) -> EfiStatus {
    let Some(console) = (unsafe { system_table.as_mut() }).and_then(|table| {
        let console = table.con_out;
        unsafe { console.as_mut() }
    }) else {
        return EFI_UNSUPPORTED;
    };

    write_console(console, "ostool HTTP Boot loader\r\n");
    write_console(console, "manifest parser core linked\r\n");

    let mut device_path_buffer = [0u8; DEVICE_PATH_BUFFER_SIZE];
    let mut manifest_url_buffer = [0u8; URL_BUFFER_SIZE];
    match loader_url_from_loaded_image(image, system_table, &mut device_path_buffer) {
        Ok(loader_url) => {
            write_console(console, "loader_url: ");
            write_console(console, loader_url);
            write_console(console, "\r\n");

            match write_sibling_manifest_url(loader_url, &mut manifest_url_buffer) {
                Ok(manifest_url) => {
                    write_console(console, "manifest_url: ");
                    write_console(console, manifest_url);
                    write_console(console, "\r\n");
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
    print_http_protocol_probe(console, system_table);

    EFI_SUCCESS
}

#[cfg(target_os = "uefi")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoaderError {
    ProtocolUnavailable,
    MissingFilePath,
    DevicePathTooLarge,
    InvalidDevicePath,
}

#[cfg(target_os = "uefi")]
fn print_http_protocol_probe(
    console: *mut EfiSimpleTextOutputProtocol,
    system_table: *mut EfiSystemTable,
) {
    let Some(boot_services) = boot_services_from_system_table(system_table) else {
        write_console(console, "failed to access Boot Services for HTTP probe\r\n");
        return;
    };

    match count_protocol_handles(boot_services, &EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID) {
        Ok(count) => {
            write_console(console, "http_service_binding_handles: ");
            write_usize(console, count);
            write_console(console, "\r\n");
        }
        Err(_) => write_console(
            console,
            "failed to locate HTTP Service Binding Protocol\r\n",
        ),
    }

    match count_protocol_handles(boot_services, &EFI_HTTP_PROTOCOL_GUID) {
        Ok(count) => {
            write_console(console, "http_protocol_handles: ");
            write_usize(console, count);
            write_console(console, "\r\n");
        }
        Err(_) => write_console(console, "failed to locate HTTP Protocol\r\n"),
    }

    probe_http_child(console, boot_services);
}

#[cfg(target_os = "uefi")]
fn boot_services_from_system_table(
    system_table: *mut EfiSystemTable,
) -> Option<&'static mut EfiBootServices> {
    (unsafe { system_table.as_mut() })
        .map(|table| table.boot_services)
        .and_then(|boot_services| unsafe { boot_services.as_mut() })
}

#[cfg(target_os = "uefi")]
fn count_protocol_handles(
    boot_services: &mut EfiBootServices,
    protocol: &EfiGuid,
) -> Result<usize, EfiStatus> {
    let mut handle_count = 0usize;
    let mut handles = core::ptr::null_mut();
    let status = (boot_services.locate_handle_buffer)(
        EFI_LOCATE_BY_PROTOCOL,
        protocol,
        core::ptr::null_mut(),
        &mut handle_count,
        &mut handles,
    );
    if status.is_error() {
        return Err(status);
    }
    if !handles.is_null() {
        let _ = (boot_services.free_pool)(handles as *mut c_void);
    }
    Ok(handle_count)
}

#[cfg(target_os = "uefi")]
fn first_protocol_handle(
    boot_services: &mut EfiBootServices,
    protocol: &EfiGuid,
) -> Result<EfiHandle, EfiStatus> {
    let mut handle_count = 0usize;
    let mut handles = core::ptr::null_mut();
    let status = (boot_services.locate_handle_buffer)(
        EFI_LOCATE_BY_PROTOCOL,
        protocol,
        core::ptr::null_mut(),
        &mut handle_count,
        &mut handles,
    );
    if status.is_error() {
        return Err(status);
    }

    let first = if handle_count > 0 && !handles.is_null() {
        Some(unsafe { *handles })
    } else {
        None
    };
    if !handles.is_null() {
        let _ = (boot_services.free_pool)(handles as *mut c_void);
    }
    first.ok_or(EFI_UNSUPPORTED)
}

#[cfg(target_os = "uefi")]
fn open_protocol_on_handle<T>(
    boot_services: &mut EfiBootServices,
    handle: EfiHandle,
    protocol: &EfiGuid,
) -> Result<*mut T, EfiStatus> {
    let mut interface = core::ptr::null_mut();
    let status = (boot_services.handle_protocol)(handle, protocol, &mut interface);
    if status.is_error() || interface.is_null() {
        return Err(status);
    }
    Ok(interface as *mut T)
}

#[cfg(target_os = "uefi")]
fn probe_http_child(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
) {
    let service_handle =
        match first_protocol_handle(boot_services, &EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID) {
            Ok(handle) => handle,
            Err(status) => {
                write_console(
                    console,
                    "http_create_child_skipped: service binding not found ",
                );
                write_status(console, status);
                write_console(console, "\r\n");
                return;
            }
        };

    let service_binding = match open_protocol_on_handle::<EfiServiceBindingProtocol>(
        boot_services,
        service_handle,
        &EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID,
    ) {
        Ok(service_binding) => service_binding,
        Err(status) => {
            write_console(console, "http_service_binding_open_failed: ");
            write_status(console, status);
            write_console(console, "\r\n");
            return;
        }
    };

    let mut child_handle = core::ptr::null_mut();
    let create_status =
        unsafe { ((*service_binding).create_child)(service_binding, &mut child_handle) };
    write_console(console, "http_create_child_status: ");
    write_status(console, create_status);
    write_console(console, "\r\n");
    if create_status.is_error() || child_handle.is_null() {
        return;
    }

    let http_status = match open_protocol_on_handle::<EfiHttpProtocol>(
        boot_services,
        child_handle,
        &EFI_HTTP_PROTOCOL_GUID,
    ) {
        Ok(_) => EFI_SUCCESS,
        Err(status) => status,
    };
    write_console(console, "http_child_protocol_status: ");
    write_status(console, http_status);
    write_console(console, "\r\n");

    let destroy_status =
        unsafe { ((*service_binding).destroy_child)(service_binding, child_handle) };
    write_console(console, "http_destroy_child_status: ");
    write_status(console, destroy_status);
    write_console(console, "\r\n");
}

#[cfg(target_os = "uefi")]
fn loader_url_from_loaded_image<'a>(
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

#[cfg(target_os = "uefi")]
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

#[cfg(target_os = "uefi")]
impl EfiStatus {
    fn is_error(self) -> bool {
        self.0 & (1usize << (usize::BITS - 1)) != 0
    }
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
fn write_usize(console: *mut EfiSimpleTextOutputProtocol, mut value: usize) {
    let mut digits = [0u8; 20];
    let mut len = 0;

    if value == 0 {
        write_console(console, "0");
        return;
    }

    while value > 0 && len < digits.len() {
        digits[len] = b'0' + (value % 10) as u8;
        value /= 10;
        len += 1;
    }

    let mut output = [0u8; 20];
    for index in 0..len {
        output[index] = digits[len - index - 1];
    }
    let text = core::str::from_utf8(&output[..len]).unwrap_or("?");
    write_console(console, text);
}

#[cfg(target_os = "uefi")]
fn write_status(console: *mut EfiSimpleTextOutputProtocol, status: EfiStatus) {
    write_console(console, "0x");
    write_hex_usize(console, status.0);
}

#[cfg(target_os = "uefi")]
fn write_hex_usize(console: *mut EfiSimpleTextOutputProtocol, mut value: usize) {
    let mut digits = [0u8; 16];
    let mut len = 0;

    if value == 0 {
        write_console(console, "0");
        return;
    }

    while value > 0 && len < digits.len() {
        let digit = (value & 0xf) as u8;
        digits[len] = match digit {
            0..=9 => b'0' + digit,
            _ => b'a' + (digit - 10),
        };
        value >>= 4;
        len += 1;
    }

    let mut output = [0u8; 16];
    for index in 0..len {
        output[index] = digits[len - index - 1];
    }
    let text = core::str::from_utf8(&output[..len]).unwrap_or("?");
    write_console(console, text);
}

#[cfg(target_os = "uefi")]
#[panic_handler]
fn panic(_info: &PanicInfo<'_>) -> ! {
    loop {
        core::hint::spin_loop();
    }
}
