use core::ffi::c_void;

use crate::uefi::abi::{
    EFI_HTTP_PROTOCOL_GUID, EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID, EFI_LOCATE_BY_PROTOCOL,
    EFI_NOT_READY, EFI_UNSUPPORTED, EVT_NOTIFY_SIGNAL, EfiBootServices, EfiEvent, EfiGuid,
    EfiHandle, EfiHttpConfigAccessPoint, EfiHttpConfigData, EfiHttpMessage, EfiHttpMessageData,
    EfiHttpProtocol, EfiHttpRequestData, EfiHttpToken, EfiHttpv4AccessPoint,
    EfiServiceBindingProtocol, EfiSimpleTextOutputProtocol, EfiStatus, EfiSystemTable,
    HTTP_METHOD_GET, HTTP_VERSION_11, TPL_CALLBACK, boot_services_from_system_table,
};
use crate::uefi::console::{write_console, write_status, write_usize, write_utf16_nul};

const UTF16_URL_BUFFER_SIZE: usize = 1024;
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
