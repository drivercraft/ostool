use core::ffi::c_void;

use crate::uefi::abi::{
    EFI_HTTP_PROTOCOL_GUID, EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID, EFI_LOCATE_BY_PROTOCOL,
    EFI_NOT_READY, EFI_UNSUPPORTED, EVT_NOTIFY_SIGNAL, EfiBootServices, EfiEvent, EfiGuid,
    EfiHandle, EfiHttpConfigAccessPoint, EfiHttpConfigData, EfiHttpMessage, EfiHttpMessageData,
    EfiHttpProtocol, EfiHttpRequestData, EfiHttpResponseData, EfiHttpToken, EfiHttpv4AccessPoint,
    EfiServiceBindingProtocol, EfiSimpleTextOutputProtocol, EfiStatus, EfiSystemTable,
    HTTP_METHOD_GET, HTTP_STATUS_200_OK, HTTP_VERSION_11, TPL_CALLBACK,
    boot_services_from_system_table,
};
use crate::uefi::console::{write_console, write_status, write_usize, write_utf16_nul};
use httpboot_loader::parse_downloaded_manifest;

const UTF16_URL_BUFFER_SIZE: usize = 1024;
const MANIFEST_BODY_BUFFER_SIZE: usize = 4096;
const KERNEL_PROBE_BODY_BUFFER_SIZE: usize = 16 * 1024;
const HTTP_COMPLETION_POLL_LIMIT: usize = 100_000;

pub fn print_http_protocol_probe(
    console: *mut EfiSimpleTextOutputProtocol,
    system_table: *mut EfiSystemTable,
    manifest_url: Option<&str>,
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

    probe_http_child(console, boot_services, manifest_url);
}

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

fn probe_http_child(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    manifest_url: Option<&str>,
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

    let http_protocol = match open_protocol_on_handle::<EfiHttpProtocol>(
        boot_services,
        child_handle,
        &EFI_HTTP_PROTOCOL_GUID,
    ) {
        Ok(http_protocol) => {
            write_console(console, "http_child_protocol_status: 0x0\r\n");
            http_protocol
        }
        Err(status) => {
            write_console(console, "http_child_protocol_status: ");
            write_status(console, status);
            write_console(console, "\r\n");
            destroy_http_child(console, service_binding, child_handle);
            return;
        }
    };

    let configure_status = configure_http_ipv4_default(http_protocol);
    write_console(console, "http_configure_status: ");
    write_status(console, configure_status);
    write_console(console, "\r\n");

    if !configure_status.is_error() {
        if let Some(manifest_url) = manifest_url {
            let mut url_buffer = [0u16; UTF16_URL_BUFFER_SIZE];
            match write_utf16_nul(manifest_url, &mut url_buffer) {
                Ok(url) => request_manifest(console, boot_services, http_protocol, url),
                Err(_) => write_console(console, "http_request_skipped: URL too long\r\n"),
            }
        } else {
            write_console(
                console,
                "http_request_skipped: manifest URL unavailable\r\n",
            );
        }

        let reset_status =
            unsafe { ((*http_protocol).configure)(http_protocol, core::ptr::null_mut()) };
        write_console(console, "http_reset_status: ");
        write_status(console, reset_status);
        write_console(console, "\r\n");
    }

    destroy_http_child(console, service_binding, child_handle);
}

fn request_manifest(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    http_protocol: *mut EfiHttpProtocol,
    url: *mut u16,
) {
    let mut event = core::ptr::null_mut();
    let event_status = (boot_services.create_event)(
        EVT_NOTIFY_SIGNAL,
        TPL_CALLBACK,
        Some(noop_event_notify),
        core::ptr::null_mut(),
        &mut event,
    );
    write_console(console, "http_request_event_status: ");
    write_status(console, event_status);
    write_console(console, "\r\n");
    if event_status.is_error() || event.is_null() {
        return;
    }

    let mut request_data = EfiHttpRequestData {
        method: HTTP_METHOD_GET,
        url,
    };
    let mut message = EfiHttpMessage {
        data: EfiHttpMessageData {
            request: &mut request_data,
        },
        header_count: 0,
        headers: core::ptr::null_mut(),
        body_length: 0,
        body: core::ptr::null_mut(),
    };
    let mut token = EfiHttpToken {
        event,
        status: EFI_NOT_READY,
        message: &mut message,
    };

    let request_status = unsafe { ((*http_protocol).request)(http_protocol, &mut token) };
    write_console(console, "http_request_status: ");
    write_status(console, request_status);
    write_console(console, "\r\n");

    if !request_status.is_error() {
        let completion = poll_http_token(http_protocol, &token);
        write_console(console, "http_request_completion: ");
        write_status(console, completion);
        write_console(console, "\r\n");
        if !completion.is_error() {
            receive_manifest_response(console, boot_services, http_protocol);
        }
    }

    let close_status = (boot_services.close_event)(event);
    write_console(console, "http_request_close_event_status: ");
    write_status(console, close_status);
    write_console(console, "\r\n");
}

extern "efiapi" fn noop_event_notify(_event: EfiEvent, _context: *mut c_void) {}

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

fn receive_manifest_response(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
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
    write_console(console, "http_response_event_status: ");
    write_status(console, event_status);
    write_console(console, "\r\n");
    if event_status.is_error() || event.is_null() {
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
    write_console(console, "http_response_status: ");
    write_status(console, response_status);
    write_console(console, "\r\n");

    if !response_status.is_error() {
        let completion = poll_http_token(http_protocol, &token);
        write_console(console, "http_response_completion: ");
        write_status(console, completion);
        write_console(console, "\r\n");
        if !completion.is_error() {
            print_manifest_response(
                console,
                boot_services,
                http_protocol,
                &response_data,
                &message,
                &body,
            );
        }
    }

    let close_status = (boot_services.close_event)(event);
    write_console(console, "http_response_close_event_status: ");
    write_status(console, close_status);
    write_console(console, "\r\n");
}

fn print_manifest_response(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    http_protocol: *mut EfiHttpProtocol,
    response_data: &EfiHttpResponseData,
    message: &EfiHttpMessage,
    body: &[u8],
) {
    write_console(console, "http_response_status_enum: ");
    write_usize(console, response_data.status_code as usize);
    write_console(console, "\r\n");
    write_console(console, "http_response_status_code: ");
    write_http_status_code(console, response_data.status_code);
    write_console(console, "\r\n");

    write_console(console, "http_response_header_count: ");
    write_usize(console, message.header_count);
    write_console(console, "\r\n");

    write_console(console, "http_response_body_length: ");
    write_usize(console, message.body_length);
    write_console(console, "\r\n");

    if response_data.status_code != HTTP_STATUS_200_OK {
        write_console(
            console,
            "manifest_parse_skipped: HTTP status is not 200\r\n",
        );
        free_response_headers(boot_services, message);
        return;
    }

    if message.body_length > body.len() {
        write_console(console, "manifest_parse_skipped: body buffer too small\r\n");
        free_response_headers(boot_services, message);
        return;
    }

    match parse_downloaded_manifest(&body[..message.body_length], body.len()) {
        Ok(manifest) => {
            write_console(console, "manifest_arch: ");
            write_console(console, manifest.arch);
            write_console(console, "\r\n");
            write_console(console, "manifest_kernel_url: ");
            write_console(console, manifest.kernel_url);
            write_console(console, "\r\n");
            write_console(console, "manifest_kernel_size: ");
            write_usize(console, manifest.kernel_size as usize);
            write_console(console, "\r\n");
            request_kernel_probe(
                console,
                boot_services,
                http_protocol,
                manifest.kernel_url,
                manifest.kernel_size,
            );
        }
        Err(_) => write_console(console, "manifest_parse_failed\r\n"),
    }

    free_response_headers(boot_services, message);
}

fn request_kernel_probe(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    http_protocol: *mut EfiHttpProtocol,
    kernel_url: &str,
    kernel_size: u64,
) {
    let mut url_buffer = [0u16; UTF16_URL_BUFFER_SIZE];
    let url = match write_utf16_nul(kernel_url, &mut url_buffer) {
        Ok(url) => url,
        Err(_) => {
            write_console(console, "kernel_request_skipped: URL too long\r\n");
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
    write_console(console, "kernel_request_event_status: ");
    write_status(console, event_status);
    write_console(console, "\r\n");
    if event_status.is_error() || event.is_null() {
        return;
    }

    let mut request_data = EfiHttpRequestData {
        method: HTTP_METHOD_GET,
        url,
    };
    let mut message = EfiHttpMessage {
        data: EfiHttpMessageData {
            request: &mut request_data,
        },
        header_count: 0,
        headers: core::ptr::null_mut(),
        body_length: 0,
        body: core::ptr::null_mut(),
    };
    let mut token = EfiHttpToken {
        event,
        status: EFI_NOT_READY,
        message: &mut message,
    };

    let request_status = unsafe { ((*http_protocol).request)(http_protocol, &mut token) };
    write_console(console, "kernel_request_status: ");
    write_status(console, request_status);
    write_console(console, "\r\n");

    if !request_status.is_error() {
        let completion = poll_http_token(http_protocol, &token);
        write_console(console, "kernel_request_completion: ");
        write_status(console, completion);
        write_console(console, "\r\n");
        if !completion.is_error() {
            receive_kernel_probe_response(console, boot_services, http_protocol, kernel_size);
        }
    }

    let close_status = (boot_services.close_event)(event);
    write_console(console, "kernel_request_close_event_status: ");
    write_status(console, close_status);
    write_console(console, "\r\n");
}

fn receive_kernel_probe_response(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    http_protocol: *mut EfiHttpProtocol,
    expected_kernel_size: u64,
) {
    let mut event = core::ptr::null_mut();
    let event_status = (boot_services.create_event)(
        EVT_NOTIFY_SIGNAL,
        TPL_CALLBACK,
        Some(noop_event_notify),
        core::ptr::null_mut(),
        &mut event,
    );
    write_console(console, "kernel_response_event_status: ");
    write_status(console, event_status);
    write_console(console, "\r\n");
    if event_status.is_error() || event.is_null() {
        return;
    }

    let mut response_data = EfiHttpResponseData { status_code: 0 };
    let mut body = [0u8; KERNEL_PROBE_BODY_BUFFER_SIZE];
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
    write_console(console, "kernel_response_status: ");
    write_status(console, response_status);
    write_console(console, "\r\n");

    if !response_status.is_error() {
        let completion = poll_http_token(http_protocol, &token);
        write_console(console, "kernel_response_completion: ");
        write_status(console, completion);
        write_console(console, "\r\n");
        if !completion.is_error() {
            print_kernel_probe_response(
                console,
                boot_services,
                &response_data,
                &message,
                &body,
                expected_kernel_size,
            );
        }
    }

    let close_status = (boot_services.close_event)(event);
    write_console(console, "kernel_response_close_event_status: ");
    write_status(console, close_status);
    write_console(console, "\r\n");
}

fn print_kernel_probe_response(
    console: *mut EfiSimpleTextOutputProtocol,
    boot_services: &mut EfiBootServices,
    response_data: &EfiHttpResponseData,
    message: &EfiHttpMessage,
    body: &[u8],
    expected_kernel_size: u64,
) {
    write_console(console, "kernel_response_status_enum: ");
    write_usize(console, response_data.status_code as usize);
    write_console(console, "\r\n");
    write_console(console, "kernel_response_status_code: ");
    write_http_status_code(console, response_data.status_code);
    write_console(console, "\r\n");
    write_console(console, "kernel_response_header_count: ");
    write_usize(console, message.header_count);
    write_console(console, "\r\n");
    write_console(console, "kernel_response_body_length: ");
    write_usize(console, message.body_length);
    write_console(console, "\r\n");
    write_console(console, "kernel_expected_size: ");
    write_usize(console, expected_kernel_size as usize);
    write_console(console, "\r\n");
    write_console(console, "kernel_probe_buffer_size: ");
    write_usize(console, body.len());
    write_console(console, "\r\n");

    if response_data.status_code != HTTP_STATUS_200_OK {
        write_console(console, "kernel_probe_skipped: HTTP status is not 200\r\n");
    } else if message.body_length > body.len() {
        write_console(console, "kernel_probe_skipped: body buffer too small\r\n");
    } else {
        write_console(console, "kernel_probe_received_prefix\r\n");
    }

    free_response_headers(boot_services, message);
}

fn free_response_headers(boot_services: &mut EfiBootServices, message: &EfiHttpMessage) {
    if !message.headers.is_null() {
        let _ = (boot_services.free_pool)(message.headers as *mut c_void);
    }
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
    console: *mut EfiSimpleTextOutputProtocol,
    service_binding: *mut EfiServiceBindingProtocol,
    child_handle: EfiHandle,
) {
    let destroy_status =
        unsafe { ((*service_binding).destroy_child)(service_binding, child_handle) };
    write_console(console, "http_destroy_child_status: ");
    write_status(console, destroy_status);
    write_console(console, "\r\n");
}
