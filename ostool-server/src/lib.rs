//! Server crate for managing development boards, serial sessions, and TFTP files.

pub mod admin_events;
pub mod admin_monitor;
pub mod admin_power;
pub mod api;
pub mod board_pool;
pub mod board_store;
pub mod config;
pub mod device;
pub mod dtb_store;
pub mod http_boot;
pub mod loader;
pub mod network_test;
pub mod ota;
pub mod power;
pub mod process;
pub mod serial;
pub mod session;
pub mod state;
pub mod tftp;
pub mod virtual_lab;
pub mod virtual_qemu;
pub mod web;

pub use api::router::build_router;
pub use config::{
    AxloaderSerialFlowControl, AxloaderSerialParameters, AxloaderSerialParity,
    AxloaderSerialStopBits, BoardConfig, BoardNetworkIdentity, BootConfig, BuiltinTftpConfig,
    CustomPowerManagement, LoaderNetworkConfig, PowerManagementConfig, PxeProfile, SerialConfig,
    SerialPortKey, SerialPortKeyKind, ServerConfig, SystemTftpdHpaConfig, TftpConfig,
    TftpNetworkConfig, UbootNetworkMode, UbootProfile, UefiBootArch, UefiHttpProfile,
    UploadLimitsConfig, VirtualQemuConfig, ZhongshengRelayPowerManagement,
};
pub use dtb_store::{DtbFile, DtbStore};
pub use state::{AppState, BoardLeaseState, build_app_state};
