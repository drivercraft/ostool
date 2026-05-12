use core::ffi::c_void;

pub type EfiHandle = *mut c_void;
pub type EfiEvent = *mut c_void;
pub type EfiLocateSearchType = u32;
pub type EfiEventNotify = Option<extern "efiapi" fn(event: EfiEvent, context: *mut c_void)>;
pub type EfiMemoryType = u32;
pub type EfiAllocateType = u32;
pub type EfiPhysicalAddress = u64;

pub const EFI_LOCATE_BY_PROTOCOL: EfiLocateSearchType = 2;
pub const EFI_ALLOCATE_ADDRESS: EfiAllocateType = 0;
pub const EFI_LOADER_DATA: EfiMemoryType = 2;

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub struct EfiStatus(pub usize);

pub const EFI_SUCCESS: EfiStatus = EfiStatus(0);
pub const EFI_ERROR_BIT: usize = 1usize << (usize::BITS - 1);
pub const EFI_UNSUPPORTED: EfiStatus = EfiStatus(EFI_ERROR_BIT | 3);
pub const EFI_NOT_READY: EfiStatus = EfiStatus(EFI_ERROR_BIT | 6);

pub const EFI_LOADED_IMAGE_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x5b1b31a1,
    data2: 0x9562,
    data3: 0x11d2,
    data4: [0x8e, 0x3f, 0x00, 0xa0, 0xc9, 0x69, 0x72, 0x3b],
};

pub const EFI_HTTP_SERVICE_BINDING_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0xbdc8e6af,
    data2: 0xd9bc,
    data3: 0x4379,
    data4: [0xa7, 0x2a, 0xe0, 0xc4, 0xe7, 0x5d, 0xae, 0x1c],
};

pub const EFI_HTTP_PROTOCOL_GUID: EfiGuid = EfiGuid {
    data1: 0x7a59b29b,
    data2: 0x910b,
    data3: 0x4171,
    data4: [0x82, 0x42, 0xa8, 0x5a, 0x0d, 0xf2, 0x5b, 0x5b],
};

pub const EVT_NOTIFY_SIGNAL: u32 = 0x0000_0200;
pub const TPL_CALLBACK: usize = 8;
pub const HTTP_VERSION_11: u32 = 1;
pub const HTTP_METHOD_GET: u32 = 0;
pub const HTTP_STATUS_200_OK: u32 = 3;

#[repr(C)]
pub struct EfiGuid {
    pub data1: u32,
    pub data2: u16,
    pub data3: u16,
    pub data4: [u8; 8],
}

#[repr(C)]
pub struct EfiTableHeader {
    signature: u64,
    revision: u32,
    header_size: u32,
    crc32: u32,
    reserved: u32,
}

#[repr(C)]
pub struct EfiSimpleTextOutputProtocol {
    pub reset: usize,
    pub output_string:
        extern "efiapi" fn(this: *mut EfiSimpleTextOutputProtocol, string: *const u16) -> EfiStatus,
}

#[repr(C)]
pub struct EfiBootServices {
    pub hdr: EfiTableHeader,
    pub _raise_tpl: usize,
    pub _restore_tpl: usize,
    pub allocate_pages: extern "efiapi" fn(
        allocate_type: EfiAllocateType,
        memory_type: EfiMemoryType,
        pages: usize,
        memory: *mut EfiPhysicalAddress,
    ) -> EfiStatus,
    pub free_pages: extern "efiapi" fn(memory: EfiPhysicalAddress, pages: usize) -> EfiStatus,
    pub get_memory_map: extern "efiapi" fn(
        memory_map_size: *mut usize,
        memory_map: *mut EfiMemoryDescriptor,
        map_key: *mut usize,
        descriptor_size: *mut usize,
        descriptor_version: *mut u32,
    ) -> EfiStatus,
    pub allocate_pool: extern "efiapi" fn(
        pool_type: EfiMemoryType,
        size: usize,
        buffer: *mut *mut c_void,
    ) -> EfiStatus,
    pub free_pool: extern "efiapi" fn(buffer: *mut c_void) -> EfiStatus,
    pub create_event: extern "efiapi" fn(
        event_type: u32,
        notify_tpl: usize,
        notify_function: EfiEventNotify,
        notify_context: *mut c_void,
        event: *mut EfiEvent,
    ) -> EfiStatus,
    pub _set_timer: usize,
    pub _wait_for_event: usize,
    pub _signal_event: usize,
    pub close_event: extern "efiapi" fn(event: EfiEvent) -> EfiStatus,
    pub _check_event: usize,
    pub _install_protocol_interface: usize,
    pub _reinstall_protocol_interface: usize,
    pub _uninstall_protocol_interface: usize,
    pub handle_protocol: extern "efiapi" fn(
        handle: EfiHandle,
        protocol: *const EfiGuid,
        interface: *mut *mut c_void,
    ) -> EfiStatus,
    pub _reserved: usize,
    pub _register_protocol_notify: usize,
    pub _locate_handle: usize,
    pub _locate_device_path: usize,
    pub _install_configuration_table: usize,
    pub _load_image: usize,
    pub _start_image: usize,
    pub _exit: usize,
    pub _unload_image: usize,
    pub exit_boot_services:
        extern "efiapi" fn(image_handle: EfiHandle, map_key: usize) -> EfiStatus,
    pub _get_next_monotonic_count: usize,
    pub _stall: usize,
    pub _set_watchdog_timer: usize,
    pub _connect_controller: usize,
    pub _disconnect_controller: usize,
    pub _open_protocol: usize,
    pub _close_protocol: usize,
    pub _open_protocol_information: usize,
    pub _protocols_per_handle: usize,
    pub locate_handle_buffer: extern "efiapi" fn(
        search_type: EfiLocateSearchType,
        protocol: *const EfiGuid,
        search_key: *mut c_void,
        no_handles: *mut usize,
        buffer: *mut *mut EfiHandle,
    ) -> EfiStatus,
}

#[repr(C)]
pub struct EfiMemoryDescriptor {
    pub memory_type: u32,
    pub physical_start: EfiPhysicalAddress,
    pub virtual_start: u64,
    pub number_of_pages: u64,
    pub attribute: u64,
}

#[repr(C)]
pub struct EfiDevicePathProtocol {
    pub node_type: u8,
    pub node_subtype: u8,
    pub length: [u8; 2],
}

#[repr(C)]
pub struct EfiLoadedImageProtocol {
    pub revision: u32,
    pub parent_handle: EfiHandle,
    pub system_table: *mut EfiSystemTable,
    pub device_handle: EfiHandle,
    pub file_path: *const EfiDevicePathProtocol,
}

#[repr(C)]
pub struct EfiServiceBindingProtocol {
    pub create_child: extern "efiapi" fn(
        this: *mut EfiServiceBindingProtocol,
        child_handle: *mut EfiHandle,
    ) -> EfiStatus,
    pub destroy_child: extern "efiapi" fn(
        this: *mut EfiServiceBindingProtocol,
        child_handle: EfiHandle,
    ) -> EfiStatus,
}

#[repr(C)]
pub struct EfiHttpProtocol {
    pub get_mode_data: usize,
    pub configure: extern "efiapi" fn(
        this: *mut EfiHttpProtocol,
        http_config_data: *mut EfiHttpConfigData,
    ) -> EfiStatus,
    pub request:
        extern "efiapi" fn(this: *mut EfiHttpProtocol, token: *mut EfiHttpToken) -> EfiStatus,
    pub cancel: usize,
    pub response:
        extern "efiapi" fn(this: *mut EfiHttpProtocol, token: *mut EfiHttpToken) -> EfiStatus,
    pub poll: extern "efiapi" fn(this: *mut EfiHttpProtocol) -> EfiStatus,
}

#[repr(C)]
pub struct EfiHttpConfigData {
    pub http_version: u32,
    pub timeout_millisec: u32,
    pub local_address_is_ipv6: u8,
    pub _padding: [u8; 7],
    pub access_point: EfiHttpConfigAccessPoint,
}

#[repr(C)]
pub union EfiHttpConfigAccessPoint {
    pub ipv4_node: *mut EfiHttpv4AccessPoint,
    pub ipv6_node: *mut c_void,
}

#[repr(C)]
pub struct EfiHttpv4AccessPoint {
    pub use_default_address: u8,
    pub local_address: [u8; 4],
    pub local_subnet: [u8; 4],
    pub local_port: u16,
}

#[repr(C)]
pub union EfiHttpMessageData {
    pub request: *mut EfiHttpRequestData,
    pub response: *mut EfiHttpResponseData,
}

#[repr(C)]
pub struct EfiHttpRequestData {
    pub method: u32,
    pub url: *mut u16,
}

#[repr(C)]
pub struct EfiHttpResponseData {
    pub status_code: u32,
}

#[repr(C)]
pub struct EfiHttpHeader {
    pub field_name: *mut u8,
    pub field_value: *mut u8,
}

#[repr(C)]
pub struct EfiHttpMessage {
    pub data: EfiHttpMessageData,
    pub header_count: usize,
    pub headers: *mut EfiHttpHeader,
    pub body_length: usize,
    pub body: *mut c_void,
}

#[repr(C)]
pub struct EfiHttpToken {
    pub event: EfiEvent,
    pub status: EfiStatus,
    pub message: *mut EfiHttpMessage,
}

#[repr(C)]
pub struct EfiSystemTable {
    pub hdr: EfiTableHeader,
    pub firmware_vendor: *mut u16,
    pub firmware_revision: u32,
    pub console_in_handle: EfiHandle,
    pub con_in: *mut c_void,
    pub console_out_handle: EfiHandle,
    pub con_out: *mut EfiSimpleTextOutputProtocol,
    pub standard_error_handle: EfiHandle,
    pub std_err: *mut EfiSimpleTextOutputProtocol,
    pub runtime_services: *mut c_void,
    pub boot_services: *mut EfiBootServices,
}

impl EfiStatus {
    pub fn is_error(self) -> bool {
        self.0 & EFI_ERROR_BIT != 0
    }
}

pub fn boot_services_from_system_table(
    system_table: *mut EfiSystemTable,
) -> Option<&'static mut EfiBootServices> {
    (unsafe { system_table.as_mut() })
        .map(|table| table.boot_services)
        .and_then(|boot_services| unsafe { boot_services.as_mut() })
}
