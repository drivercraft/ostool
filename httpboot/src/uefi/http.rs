use core::ffi::c_void;

use crate::uefi::abi::{
    EFI_ALLOCATE_ADDRESS, EFI_CONVENTIONAL_MEMORY, EFI_HTTP_PROTOCOL_GUID,
    EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID, EFI_LOADER_DATA, EFI_LOCATE_BY_PROTOCOL,
    EFI_NO_MAPPING, EFI_NOT_READY, EFI_SUCCESS, EFI_TIMEOUT, EFI_UNSUPPORTED, EVT_NOTIFY_SIGNAL,
    EfiBootServices, EfiEvent, EfiGuid, EfiHandle, EfiHttpConfigAccessPoint, EfiHttpConfigData,
    EfiHttpHeader, EfiHttpMessage, EfiHttpMessageData, EfiHttpProtocol, EfiHttpRequestData,
    EfiHttpResponseData, EfiHttpToken, EfiHttpv4AccessPoint, EfiMemoryDescriptor,
    EfiPhysicalAddress, EfiServiceBindingProtocol, EfiSimpleTextOutputProtocol, EfiStatus,
    EfiSystemTable, HTTP_METHOD_GET, HTTP_STATUS_200_OK, HTTP_STATUS_206_PARTIAL_CONTENT,
    HTTP_VERSION_11, TPL_CALLBACK, boot_services_from_system_table,
};
use crate::uefi::console::{
    set_progress_cursor_visible, write_console, write_usize, write_utf16_nul,
};
use crate::uefi::entry::{EntryPlan, call_entry_point, print_entry_plan, target_matches_manifest};
use httpboot::parse_downloaded_manifest;

const UTF16_URL_BUFFER_SIZE: usize = 1024;
const HTTP_HOST_BUFFER_SIZE: usize = 256;
const MANIFEST_BODY_BUFFER_SIZE: usize = 4096;
// Keep kernel downloads range-based because the ASUS NUC15 UEFI HTTP stack can
// time out when one large response body is drained through repeated Response()
// calls. The NUC15 firmware currently completes 1 KiB range responses reliably,
// while larger ranges may require draining one HTTP response with multiple
// Response() calls and can fail in firmware.
const KERNEL_RANGE_CHUNK_SIZE: usize = 1024;
const HTTP_COMPLETION_POLL_LIMIT: usize = 100_000;
const HTTP_REQUEST_RETRY_LIMIT: usize = 8;
const HTTP_REQUEST_RETRY_STALL_US: usize = 250_000;
const HTTP_BOOT_ROUND_RETRY_STALL_US: usize = 3_000_000;
const KERNEL_PROGRESS_STEP_PERCENT: usize = 1;
const KERNEL_PROGRESS_BAR_WIDTH: usize = 80;
const MAX_KERNEL_DOWNLOAD_SIZE: usize = 256 * 1024 * 1024;
const EFI_PAGE_SIZE: usize = 4096;
const MEMORY_MAP_BUFFER_SIZE: usize = 64 * 1024;
const OSTOOL_BOOT_INFO_MAGIC: u64 = 0x4f53_544f_4f4c_4249;
const OSTOOL_BOOT_INFO_VERSION: u32 = 1;
const OSTOOL_BOOT_INFO_MAX_RAM_REGIONS: usize = 32;
const ENABLE_BOOT_JUMP: bool = cfg!(feature = "boot-jump");

#[repr(C)]
#[derive(Clone, Copy)]
struct OstoolRamRegion {
    start: u64,
    size: u64,
}

#[repr(C)]
struct OstoolBootInfo {
    magic: u64,
    version: u32,
    region_count: u32,
    regions: [OstoolRamRegion; OSTOOL_BOOT_INFO_MAX_RAM_REGIONS],
}

impl OstoolBootInfo {
    fn new() -> Self {
        Self {
            magic: OSTOOL_BOOT_INFO_MAGIC,
            version: OSTOOL_BOOT_INFO_VERSION,
            region_count: 0,
            regions: [OstoolRamRegion { start: 0, size: 0 }; OSTOOL_BOOT_INFO_MAX_RAM_REGIONS],
        }
    }
}

pub fn run_http_boot_loader(
    console: *mut EfiSimpleTextOutputProtocol,
    image: EfiHandle,
    system_table: *mut EfiSystemTable,
    manifest_url: Option<&str>,
) -> EfiStatus {
    let Some(boot_services) = boot_services_from_system_table(system_table) else {
        write_console(console, "error: Boot Services unavailable\r\n");
        return EFI_SUCCESS;
    };

    let mut round = 0usize;
    loop {
        round += 1;
        run_http_child(console, boot_services, image, manifest_url, round > 3);
        let _ = (boot_services.stall)(HTTP_BOOT_ROUND_RETRY_STALL_US);
    }
}

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

fn run_http_child(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    image: EfiHandle,
    manifest_url: Option<&str>,
    show_errors: bool,
) {
    let service_handle =
        match first_protocol_handle(boot_services, &EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID) {
            Ok(handle) => handle,
            Err(_) => {
                if show_errors {
                    write_console(console, "error: HTTP service binding unavailable\r\n");
                }
                return;
            }
        };

    let service_binding = match open_protocol_on_handle::<EfiServiceBindingProtocol>(
        boot_services,
        service_handle,
        &EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID,
    ) {
        Ok(service_binding) => service_binding,
        Err(_) => {
            if show_errors {
                write_console(console, "error: failed to open HTTP service binding\r\n");
            }
            return;
        }
    };

    let mut child_handle = core::ptr::null_mut();
    let create_status =
        unsafe { ((*service_binding).create_child)(service_binding, &mut child_handle) };
    if create_status.is_error() || child_handle.is_null() {
        if show_errors {
            write_console(console, "error: failed to create HTTP child\r\n");
        }
        return;
    }

    let http_protocol = match open_protocol_on_handle::<EfiHttpProtocol>(
        boot_services,
        child_handle,
        &EFI_HTTP_PROTOCOL_GUID,
    ) {
        Ok(http_protocol) => http_protocol,
        Err(_) => {
            if show_errors {
                write_console(console, "error: failed to open HTTP protocol\r\n");
            }
            destroy_http_child(console, service_binding, child_handle);
            return;
        }
    };

    let configure_status = configure_http_ipv4_default(http_protocol);

    if !configure_status.is_error() {
        if let Some(manifest_url) = manifest_url {
            request_manifest(
                console,
                boot_services,
                image,
                http_protocol,
                manifest_url,
                show_errors,
            );
        } else {
            write_console(console, "error: manifest URL unavailable\r\n");
        }

        let _ = unsafe { ((*http_protocol).configure)(http_protocol, core::ptr::null_mut()) };
    } else {
        if show_errors {
            write_console(console, "error: failed to configure HTTP IPv4\r\n");
        }
    }

    destroy_http_child(console, service_binding, child_handle);
}

fn request_manifest(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    image: EfiHandle,
    http_protocol: *mut EfiHttpProtocol,
    manifest_url: &str,
    show_errors: bool,
) {
    let mut url_buffer = [0u16; UTF16_URL_BUFFER_SIZE];
    let url = match write_utf16_nul(manifest_url, &mut url_buffer) {
        Ok(url) => url,
        Err(_) => {
            if show_errors {
                write_console(console, "error: manifest URL too long\r\n");
            }
            return;
        }
    };
    let mut host_name = *b"Host\0";
    let mut host_value = [0u8; HTTP_HOST_BUFFER_SIZE];
    match write_url_host_header_value(manifest_url, &mut host_value) {
        Ok(_) => {}
        Err(()) => {
            if show_errors {
                write_console(console, "error: invalid manifest URL host\r\n");
            }
            return;
        }
    };

    let mut event = core::ptr::null_mut();
    let event_status = (boot_services.create_event)(
        EVT_NOTIFY_SIGNAL,
        TPL_CALLBACK,
        Some(noop_event_notify),
        core::ptr::null_mut(),
        &mut event,
    );
    if event_status.is_error() || event.is_null() {
        if show_errors {
            write_console(
                console,
                "error: failed to create manifest request event\r\n",
            );
        }
        return;
    }

    let mut headers = [EfiHttpHeader {
        field_name: host_name.as_mut_ptr(),
        field_value: host_value.as_mut_ptr(),
    }];
    let mut request_data = EfiHttpRequestData {
        method: HTTP_METHOD_GET,
        url,
    };
    let mut message = EfiHttpMessage {
        data: EfiHttpMessageData {
            request: &mut request_data,
        },
        header_count: headers.len(),
        headers: headers.as_mut_ptr(),
        body_length: 0,
        body: core::ptr::null_mut(),
    };
    let mut token = EfiHttpToken {
        event,
        status: EFI_NOT_READY,
        message: &mut message,
    };

    let request_status = submit_request_with_retries(boot_services, http_protocol, &mut token);

    if !request_status.is_error() {
        let completion = poll_http_token(http_protocol, &token);
        if !completion.is_error() {
            receive_manifest_response(console, boot_services, image, http_protocol);
        } else if show_errors {
            write_console(console, "error: manifest request did not complete\r\n");
        }
    } else if show_errors {
        write_console(console, "error: failed to send manifest request\r\n");
    }

    let _ = (boot_services.close_event)(event);
}

extern "efiapi" fn noop_event_notify(_event: EfiEvent, _context: *mut c_void) {}

fn submit_request_with_retries(
    boot_services: &mut EfiBootServices,
    http_protocol: *mut EfiHttpProtocol,
    token: &mut EfiHttpToken,
) -> EfiStatus {
    let mut status = EFI_NOT_READY;
    for attempt in 0..HTTP_REQUEST_RETRY_LIMIT {
        token.status = EFI_NOT_READY;
        status = unsafe { ((*http_protocol).request)(http_protocol, token) };
        if !is_transient_http_submit_status(status) {
            return status;
        }

        for _ in 0..8 {
            let _ = unsafe { ((*http_protocol).poll)(http_protocol) };
        }
        let _ = (boot_services.stall)(HTTP_REQUEST_RETRY_STALL_US * (attempt + 1));
    }
    status
}

fn is_transient_http_submit_status(status: EfiStatus) -> bool {
    status == EFI_NO_MAPPING || status == EFI_NOT_READY || status == EFI_TIMEOUT
}

fn poll_http_token(http_protocol: *mut EfiHttpProtocol, token: &EfiHttpToken) -> EfiStatus {
    for _ in 0..HTTP_COMPLETION_POLL_LIMIT {
        let status = unsafe { core::ptr::read_volatile(&token.status) };
        if status != EFI_NOT_READY {
            return status;
        }
        let _ = unsafe { ((*http_protocol).poll)(http_protocol) };
    }
    unsafe { core::ptr::read_volatile(&token.status) }
}

fn write_url_host_header_value<'a>(url: &str, output: &'a mut [u8]) -> Result<&'a str, ()> {
    let scheme_end = url.find("://").ok_or(())?;
    let authority = &url[scheme_end + 3..];
    let host_end = authority.find('/').unwrap_or(authority.len());
    if host_end == 0 {
        return Err(());
    }

    let host = &authority[..host_end];
    if host.len() + 1 > output.len() {
        return Err(());
    }
    output[..host.len()].copy_from_slice(host.as_bytes());
    output[host.len()] = 0;
    core::str::from_utf8(&output[..host.len()]).map_err(|_| ())
}

fn receive_manifest_response(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    image: EfiHandle,
    http_protocol: *mut EfiHttpProtocol,
) {
    let mut event = core::ptr::null_mut();
    let event_status = (boot_services.create_event)(
        EVT_NOTIFY_SIGNAL,
        TPL_CALLBACK,
        Some(noop_event_notify),
        core::ptr::null_mut(),
        &mut event,
    );
    if event_status.is_error() || event.is_null() {
        write_console(
            console,
            "error: failed to create manifest response event\r\n",
        );
        return;
    }

    let mut response_data = EfiHttpResponseData { status_code: 0 };
    let mut body = [0u8; MANIFEST_BODY_BUFFER_SIZE];
    let mut message = EfiHttpMessage {
        data: EfiHttpMessageData {
            response: &mut response_data,
        },
        header_count: 0,
        headers: core::ptr::null_mut(),
        body_length: body.len(),
        body: body.as_mut_ptr() as *mut c_void,
    };
    let mut token = EfiHttpToken {
        event,
        status: EFI_NOT_READY,
        message: &mut message,
    };

    let response_status = unsafe { ((*http_protocol).response)(http_protocol, &mut token) };

    if !response_status.is_error() {
        let completion = poll_http_token(http_protocol, &token);
        if !completion.is_error() {
            print_manifest_response(
                console,
                boot_services,
                image,
                http_protocol,
                &response_data,
                &message,
                &body,
            );
        } else {
            write_console(console, "error: manifest response did not complete\r\n");
        }
    } else {
        write_console(console, "error: failed to receive manifest response\r\n");
    }

    let _ = (boot_services.close_event)(event);
}

fn print_manifest_response(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    image: EfiHandle,
    http_protocol: *mut EfiHttpProtocol,
    response_data: &EfiHttpResponseData,
    message: &EfiHttpMessage,
    body: &[u8],
) {
    if response_data.status_code != HTTP_STATUS_200_OK {
        write_console(console, "error: manifest HTTP status ");
        write_http_status_code(console, response_data.status_code);
        write_console(console, "\r\n");
        free_response_headers(boot_services, message);
        return;
    }

    if message.body_length > body.len() {
        write_console(console, "error: manifest body too large\r\n");
        free_response_headers(boot_services, message);
        return;
    }

    match parse_downloaded_manifest(&body[..message.body_length], body.len()) {
        Ok(manifest) => {
            if !target_matches_manifest(manifest.arch) {
                write_console(console, "error: manifest arch mismatch\r\n");
                free_response_headers(boot_services, message);
                return;
            }
            write_console(console, "kernel: ");
            write_usize(console, manifest.kernel_size as usize);
            write_console(console, " bytes load=0x");
            write_hex_u64(console, manifest.kernel_load_addr);
            write_console(console, " entry=0x");
            write_hex_u64(console, manifest.entry_point);
            write_console(console, "\r\n");
            request_kernel_probe(
                console,
                boot_services,
                image,
                http_protocol,
                manifest.kernel_url,
                manifest.kernel_size,
                manifest.kernel_load_addr,
                manifest.entry_point,
                manifest.arch,
            );
        }
        Err(_) => write_console(console, "error: failed to parse manifest\r\n"),
    }

    free_response_headers(boot_services, message);
}

fn request_kernel_probe(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    image: EfiHandle,
    http_protocol: *mut EfiHttpProtocol,
    kernel_url: &str,
    kernel_size: u64,
    kernel_load_addr: u64,
    entry_point: u64,
    arch: &str,
) {
    download_kernel_to_load_addr(
        console,
        boot_services,
        image,
        http_protocol,
        kernel_url,
        kernel_size,
        kernel_load_addr,
        entry_point,
        arch,
    );
}

fn request_kernel_range(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    http_protocol: *mut EfiHttpProtocol,
    kernel_url: &str,
    range_start: usize,
    range_end: usize,
    dst: *mut u8,
    expected_len: usize,
    first: bool,
) -> Option<usize> {
    let mut url_buffer = [0u16; UTF16_URL_BUFFER_SIZE];
    let url = match write_utf16_nul(kernel_url, &mut url_buffer) {
        Ok(url) => url,
        Err(_) => {
            write_console(console, "error: kernel URL too long\r\n");
            return None;
        }
    };
    let mut host_name = *b"Host\0";
    let mut host_value = [0u8; HTTP_HOST_BUFFER_SIZE];
    match write_url_host_header_value(kernel_url, &mut host_value) {
        Ok(_) => {}
        Err(()) => {
            write_console(console, "error: invalid kernel URL host\r\n");
            return None;
        }
    };
    let mut range_name = *b"Range\0";
    let mut range_value = [0u8; 64];
    write_range_header_value(range_start, range_end, &mut range_value)?;

    let mut event = core::ptr::null_mut();
    let event_status = (boot_services.create_event)(
        EVT_NOTIFY_SIGNAL,
        TPL_CALLBACK,
        Some(noop_event_notify),
        core::ptr::null_mut(),
        &mut event,
    );
    if event_status.is_error() || event.is_null() {
        write_console(console, "error: failed to create kernel request event\r\n");
        return None;
    }

    let mut headers = [
        EfiHttpHeader {
            field_name: host_name.as_mut_ptr(),
            field_value: host_value.as_mut_ptr(),
        },
        EfiHttpHeader {
            field_name: range_name.as_mut_ptr(),
            field_value: range_value.as_mut_ptr(),
        },
    ];
    let mut request_data = EfiHttpRequestData {
        method: HTTP_METHOD_GET,
        url,
    };
    let mut message = EfiHttpMessage {
        data: EfiHttpMessageData {
            request: &mut request_data,
        },
        header_count: headers.len(),
        headers: headers.as_mut_ptr(),
        body_length: 0,
        body: core::ptr::null_mut(),
    };
    let mut token = EfiHttpToken {
        event,
        status: EFI_NOT_READY,
        message: &mut message,
    };

    let request_status = submit_request_with_retries(boot_services, http_protocol, &mut token);

    if !request_status.is_error() {
        let completion = poll_http_token(http_protocol, &token);
        if completion.is_error() {
            write_console(console, "error: kernel request did not complete\r\n");
            let _ = (boot_services.close_event)(event);
            return None;
        }
        let received = receive_kernel_range_body(
            console,
            boot_services,
            http_protocol,
            dst,
            expected_len,
            first,
        );
        let _ = (boot_services.close_event)(event);
        return received;
    }

    write_console(console, "error: failed to send kernel request\r\n");
    let _ = (boot_services.close_event)(event);
    None
}

fn download_kernel_to_load_addr(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    image: EfiHandle,
    http_protocol: *mut EfiHttpProtocol,
    kernel_url: &str,
    expected_kernel_size: u64,
    kernel_load_addr: u64,
    entry_point: u64,
    arch: &str,
) {
    let Some(expected_size) = checked_kernel_size(console, expected_kernel_size) else {
        return;
    };

    let Some(page_count) = kernel_page_count(console, kernel_load_addr, expected_size) else {
        return;
    };

    let mut target = kernel_load_addr as EfiPhysicalAddress;
    let allocate_status = (boot_services.allocate_pages)(
        EFI_ALLOCATE_ADDRESS,
        EFI_LOADER_DATA,
        page_count,
        &mut target,
    );
    if allocate_status.is_error() || target != kernel_load_addr {
        write_console(console, "error: failed to allocate kernel pages\r\n");
        return;
    }

    set_progress_cursor_visible(console, false);
    print_download_progress(console, 0, expected_size, 0);
    let received = download_kernel_ranges(
        console,
        boot_services,
        http_protocol,
        kernel_url,
        kernel_load_addr,
        expected_size,
    );
    let complete = received == expected_size;

    if complete {
        set_progress_cursor_visible(console, true);
        write_console(console, "\r\n");
        print_jump_readiness(
            console,
            boot_services,
            image,
            kernel_load_addr,
            entry_point,
            arch,
            expected_size,
            page_count,
        );
    } else {
        set_progress_cursor_visible(console, true);
        write_console(console, "\r\n");
        write_console(console, "error: kernel download incomplete\r\n");
        let _ = (boot_services.free_pages)(kernel_load_addr, page_count);
    }
}

fn checked_kernel_size(
    console: *mut EfiSimpleTextOutputProtocol,
    expected_kernel_size: u64,
) -> Option<usize> {
    if expected_kernel_size == 0 {
        write_console(console, "kernel_download_skipped: zero size\r\n");
        return None;
    }
    if expected_kernel_size > MAX_KERNEL_DOWNLOAD_SIZE as u64 {
        write_console(console, "kernel_download_skipped: size too large\r\n");
        return None;
    }
    Some(expected_kernel_size as usize)
}

fn kernel_page_count(
    console: *mut EfiSimpleTextOutputProtocol,
    kernel_load_addr: u64,
    expected_size: usize,
) -> Option<usize> {
    if kernel_load_addr as usize as u64 != kernel_load_addr {
        write_console(
            console,
            "kernel_download_skipped: load address too large\r\n",
        );
        return None;
    }
    if kernel_load_addr as usize % EFI_PAGE_SIZE != 0 {
        write_console(
            console,
            "kernel_download_skipped: load address is not page aligned\r\n",
        );
        return None;
    }
    expected_size
        .checked_add(EFI_PAGE_SIZE - 1)
        .map(|size| size / EFI_PAGE_SIZE)
        .filter(|pages| *pages > 0)
        .or_else(|| {
            write_console(console, "kernel_download_skipped: page count overflow\r\n");
            None
        })
}

fn download_kernel_ranges(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    http_protocol: *mut EfiHttpProtocol,
    kernel_url: &str,
    kernel_load_addr: u64,
    expected_size: usize,
) -> usize {
    let mut downloaded = 0usize;
    let mut next_progress_percent = KERNEL_PROGRESS_STEP_PERCENT;

    while downloaded < expected_size {
        let chunk_len = (expected_size - downloaded).min(KERNEL_RANGE_CHUNK_SIZE);
        let range_start = downloaded;
        let range_end = downloaded + chunk_len - 1;
        let dst = unsafe { (kernel_load_addr as *mut u8).add(downloaded) };
        let first = downloaded == 0;
        let Some(received) = request_kernel_range(
            console,
            boot_services,
            http_protocol,
            kernel_url,
            range_start,
            range_end,
            dst,
            chunk_len,
            first,
        ) else {
            write_console(console, "\r\n");
            write_console(console, "error: kernel download stopped at ");
            write_usize(console, downloaded);
            write_console(console, "\r\n");
            break;
        };
        if received == 0 {
            write_console(console, "\r\n");
            write_console(console, "error: zero length kernel chunk\r\n");
            break;
        }
        downloaded += received;
        let percent = download_percent(downloaded, expected_size);
        if percent >= next_progress_percent || downloaded == expected_size {
            print_download_progress(console, downloaded, expected_size, percent);
            while next_progress_percent <= percent {
                next_progress_percent += KERNEL_PROGRESS_STEP_PERCENT;
            }
        }
    }

    downloaded
}

fn download_percent(downloaded: usize, expected_size: usize) -> usize {
    if expected_size == 0 {
        return 0;
    }
    downloaded.saturating_mul(100) / expected_size
}

fn print_download_progress(
    console: *mut EfiSimpleTextOutputProtocol,
    downloaded: usize,
    expected_size: usize,
    percent: usize,
) {
    write_console(console, "\rdownload: [");
    let filled = percent.saturating_mul(KERNEL_PROGRESS_BAR_WIDTH) / 100;
    for index in 0..KERNEL_PROGRESS_BAR_WIDTH {
        write_console(console, if index < filled { "#" } else { "-" });
    }
    write_console(console, "] ");
    write_usize(console, percent);
    write_console(console, "% ");
    write_human_size(console, downloaded);
    write_console(console, "/");
    write_human_size(console, expected_size);
    write_console(console, "    ");
}

fn write_human_size(console: *mut EfiSimpleTextOutputProtocol, bytes: usize) {
    const KIB: usize = 1024;
    const MIB: usize = 1024 * 1024;

    if bytes >= MIB {
        write_fixed_2(console, bytes, MIB);
        write_console(console, " MiB");
    } else if bytes >= KIB {
        write_fixed_2(console, bytes, KIB);
        write_console(console, " KiB");
    } else {
        write_usize(console, bytes);
        write_console(console, " B");
    }
}

fn write_fixed_2(console: *mut EfiSimpleTextOutputProtocol, value: usize, unit: usize) {
    let whole = value / unit;
    let frac = value % unit;
    let hundredths = frac.saturating_mul(100) / unit;
    write_usize(console, whole);
    write_console(console, ".");
    if hundredths < 10 {
        write_console(console, "0");
    }
    write_usize(console, hundredths);
}

fn write_range_header_value(start: usize, end: usize, output: &mut [u8]) -> Option<()> {
    let mut writer = ByteWriter::new(output);
    writer.write_bytes(b"bytes=")?;
    writer.write_usize(start)?;
    writer.write_byte(b'-')?;
    writer.write_usize(end)?;
    writer.finish()
}

struct ByteWriter<'a> {
    output: &'a mut [u8],
    len: usize,
}

impl<'a> ByteWriter<'a> {
    fn new(output: &'a mut [u8]) -> Self {
        Self { output, len: 0 }
    }

    fn write_byte(&mut self, byte: u8) -> Option<()> {
        if self.len + 1 >= self.output.len() {
            return None;
        }
        self.output[self.len] = byte;
        self.len += 1;
        Some(())
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> Option<()> {
        for byte in bytes {
            self.write_byte(*byte)?;
        }
        Some(())
    }

    fn write_usize(&mut self, mut value: usize) -> Option<()> {
        let mut digits = [0u8; 20];
        let mut len = 0usize;
        if value == 0 {
            return self.write_byte(b'0');
        }
        while value > 0 && len < digits.len() {
            digits[len] = b'0' + (value % 10) as u8;
            value /= 10;
            len += 1;
        }
        while len > 0 {
            len -= 1;
            self.write_byte(digits[len])?;
        }
        Some(())
    }

    fn finish(&mut self) -> Option<()> {
        if self.len >= self.output.len() {
            return None;
        }
        self.output[self.len] = 0;
        Some(())
    }
}

fn receive_kernel_range_body(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    http_protocol: *mut EfiHttpProtocol,
    body: *mut u8,
    body_len: usize,
    first: bool,
) -> Option<usize> {
    let response =
        receive_kernel_stream_chunk(console, boot_services, http_protocol, body, body_len, first)?;
    if response.http_status != HTTP_STATUS_206_PARTIAL_CONTENT {
        write_console(console, "error: kernel HTTP status ");
        write_http_status_code(console, response.http_status);
        write_console(console, "\r\n");
        return None;
    }
    Some(response.received)
}

struct KernelChunkResponse {
    received: usize,
    http_status: u32,
}

fn receive_kernel_stream_chunk(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    http_protocol: *mut EfiHttpProtocol,
    body: *mut u8,
    body_len: usize,
    _first: bool,
) -> Option<KernelChunkResponse> {
    let mut event = core::ptr::null_mut();
    let event_status = (boot_services.create_event)(
        EVT_NOTIFY_SIGNAL,
        TPL_CALLBACK,
        Some(noop_event_notify),
        core::ptr::null_mut(),
        &mut event,
    );
    if event_status.is_error() || event.is_null() {
        write_console(console, "error: failed to create kernel response event\r\n");
        return None;
    }

    let mut response_data = EfiHttpResponseData {
        status_code: HTTP_STATUS_200_OK,
    };
    let mut message = EfiHttpMessage {
        data: EfiHttpMessageData {
            response: &mut response_data,
        },
        header_count: 0,
        headers: core::ptr::null_mut(),
        body_length: body_len,
        body: body as *mut c_void,
    };
    let mut token = EfiHttpToken {
        event,
        status: EFI_NOT_READY,
        message: &mut message,
    };

    let response_status = unsafe { ((*http_protocol).response)(http_protocol, &mut token) };

    if response_status.is_error() {
        write_console(console, "error: failed to receive kernel response\r\n");
        let _ = (boot_services.close_event)(event);
        return None;
    }

    let completion = poll_http_token(http_protocol, &token);
    if !completion.is_error() {
        free_response_headers(boot_services, &message);
    } else {
        write_console(console, "error: kernel response did not complete\r\n");
        free_response_headers(boot_services, &message);
    }

    let _ = (boot_services.close_event)(event);

    if completion.is_error() {
        return None;
    }
    Some(KernelChunkResponse {
        received: message.body_length,
        http_status: response_data.status_code,
    })
}

fn print_jump_readiness(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    image: EfiHandle,
    kernel_load_addr: u64,
    entry_point: u64,
    arch: &str,
    kernel_size: usize,
    page_count: usize,
) {
    print_entry_plan(
        console,
        &EntryPlan {
            arch,
            load_addr: kernel_load_addr,
            entry_point,
            kernel_size,
            boot_info: 0,
        },
    );

    maybe_exit_boot_services(
        console,
        boot_services,
        image,
        &EntryPlan {
            arch,
            load_addr: kernel_load_addr,
            entry_point,
            kernel_size,
            boot_info: 0,
        },
    );
    let _ = (boot_services.free_pages)(kernel_load_addr, page_count);
    write_console(console, "error: jump returned unexpectedly\r\n");
}

struct MemoryMapProbe {
    memory_map_size: usize,
    map_key: usize,
    descriptor_size: usize,
    descriptor_version: u32,
}

impl MemoryMapProbe {
    fn new() -> Self {
        Self {
            memory_map_size: 0,
            map_key: 0,
            descriptor_size: 0,
            descriptor_version: 0,
        }
    }
}

fn get_memory_map(
    boot_services: &mut EfiBootServices,
    probe: &mut MemoryMapProbe,
    memory_map: *mut EfiMemoryDescriptor,
    memory_map_capacity: usize,
) -> EfiStatus {
    probe.memory_map_size = memory_map_capacity;
    (boot_services.get_memory_map)(
        &mut probe.memory_map_size,
        memory_map,
        &mut probe.map_key,
        &mut probe.descriptor_size,
        &mut probe.descriptor_version,
    )
}

fn maybe_exit_boot_services(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    image: EfiHandle,
    entry_plan: &EntryPlan<'_>,
) {
    if !ENABLE_BOOT_JUMP {
        write_console(console, "error: boot jump disabled\r\n");
        return;
    }

    let mut memory_map = [0u8; MEMORY_MAP_BUFFER_SIZE];
    let mut probe = MemoryMapProbe::new();
    let map_status = get_memory_map(
        boot_services,
        &mut probe,
        memory_map.as_mut_ptr() as *mut EfiMemoryDescriptor,
        memory_map.len(),
    );
    if !map_status.is_error() {
        let mut boot_info = OstoolBootInfo::new();
        populate_boot_info_from_memory_map(&mut boot_info, &memory_map, &probe);
        let entry_plan = EntryPlan {
            boot_info: core::ptr::addr_of!(boot_info) as usize,
            ..*entry_plan
        };
        let exit_status = (boot_services.exit_boot_services)(image, probe.map_key);
        if !exit_status.is_error() {
            unsafe { call_entry_point(&entry_plan) };
        }
        retry_exit_boot_services(console, boot_services, image, &entry_plan);
        return;
    }

    write_console(console, "error: memory map unavailable before jump\r\n");
}

fn retry_exit_boot_services(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    image: EfiHandle,
    entry_plan: &EntryPlan<'_>,
) {
    let mut memory_map = [0u8; MEMORY_MAP_BUFFER_SIZE];
    let mut probe = MemoryMapProbe::new();
    let map_status = get_memory_map(
        boot_services,
        &mut probe,
        memory_map.as_mut_ptr() as *mut EfiMemoryDescriptor,
        memory_map.len(),
    );
    if map_status.is_error() {
        write_console(console, "error: memory map retry failed\r\n");
        return;
    }

    let mut boot_info = OstoolBootInfo::new();
    populate_boot_info_from_memory_map(&mut boot_info, &memory_map, &probe);
    let entry_plan = EntryPlan {
        boot_info: core::ptr::addr_of!(boot_info) as usize,
        ..*entry_plan
    };
    let exit_status = (boot_services.exit_boot_services)(image, probe.map_key);
    if !exit_status.is_error() {
        unsafe { call_entry_point(&entry_plan) };
    }
    write_console(console, "error: ExitBootServices failed\r\n");
}

fn populate_boot_info_from_memory_map(
    boot_info: &mut OstoolBootInfo,
    memory_map: &[u8],
    probe: &MemoryMapProbe,
) {
    if probe.descriptor_size < core::mem::size_of::<EfiMemoryDescriptor>() {
        return;
    }

    let mut offset = 0usize;
    while offset + core::mem::size_of::<EfiMemoryDescriptor>() <= probe.memory_map_size
        && offset + core::mem::size_of::<EfiMemoryDescriptor>() <= memory_map.len()
    {
        let descriptor = unsafe {
            core::ptr::read_unaligned(memory_map.as_ptr().add(offset) as *const EfiMemoryDescriptor)
        };
        if descriptor.memory_type == EFI_CONVENTIONAL_MEMORY
            && boot_info.region_count < OSTOOL_BOOT_INFO_MAX_RAM_REGIONS as u32
        {
            let index = boot_info.region_count as usize;
            boot_info.regions[index] = OstoolRamRegion {
                start: descriptor.physical_start,
                size: descriptor
                    .number_of_pages
                    .saturating_mul(EFI_PAGE_SIZE as u64),
            };
            boot_info.region_count += 1;
        }
        offset += probe.descriptor_size;
    }
}

fn free_response_headers(boot_services: &mut EfiBootServices, message: &EfiHttpMessage) {
    if !message.headers.is_null() {
        let _ = (boot_services.free_pool)(message.headers as *mut c_void);
    }
}

fn write_hex_u64(console: *mut EfiSimpleTextOutputProtocol, value: u64) {
    let mut output = [0u8; 16];
    let mut shift = 60u32;
    for byte in &mut output {
        let digit = ((value >> shift) & 0xf) as u8;
        *byte = match digit {
            0..=9 => b'0' + digit,
            _ => b'a' + (digit - 10),
        };
        shift = shift.saturating_sub(4);
    }
    let text = core::str::from_utf8(&output).unwrap_or("????????????????");
    write_console(console, text);
}

fn write_http_status_code(console: *mut EfiSimpleTextOutputProtocol, status_code: u32) {
    let numeric = match status_code {
        1 => Some(100),
        2 => Some(101),
        3 => Some(200),
        4 => Some(201),
        5 => Some(202),
        6 => Some(203),
        7 => Some(204),
        8 => Some(205),
        9 => Some(206),
        10 => Some(300),
        11 => Some(301),
        12 => Some(302),
        13 => Some(303),
        14 => Some(304),
        15 => Some(305),
        16 => Some(307),
        17 => Some(400),
        18 => Some(401),
        19 => Some(402),
        20 => Some(403),
        21 => Some(404),
        22 => Some(405),
        23 => Some(406),
        24 => Some(407),
        25 => Some(408),
        26 => Some(409),
        27 => Some(410),
        28 => Some(411),
        29 => Some(412),
        30 => Some(413),
        31 => Some(414),
        32 => Some(415),
        33 => Some(416),
        34 => Some(417),
        35 => Some(500),
        36 => Some(501),
        37 => Some(502),
        38 => Some(503),
        39 => Some(504),
        40 => Some(505),
        41 => Some(308),
        42 => Some(429),
        _ => None,
    };

    if let Some(numeric) = numeric {
        write_usize(console, numeric);
    } else {
        write_console(console, "unknown");
    }
}

fn configure_http_ipv4_default(http_protocol: *mut EfiHttpProtocol) -> EfiStatus {
    let mut ipv4 = EfiHttpv4AccessPoint {
        use_default_address: 1,
        local_address: [0; 4],
        local_subnet: [0; 4],
        local_port: 0,
    };
    let mut config = EfiHttpConfigData {
        http_version: HTTP_VERSION_11,
        timeout_millisec: 10_000,
        local_address_is_ipv6: 0,
        _padding: [0; 7],
        access_point: EfiHttpConfigAccessPoint {
            ipv4_node: &mut ipv4,
        },
    };

    unsafe { ((*http_protocol).configure)(http_protocol, &mut config) }
}

fn destroy_http_child(
    _console: *mut EfiSimpleTextOutputProtocol,
    service_binding: *mut EfiServiceBindingProtocol,
    child_handle: EfiHandle,
) {
    let _ = unsafe { ((*service_binding).destroy_child)(service_binding, child_handle) };
}
