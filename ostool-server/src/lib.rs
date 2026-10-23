//! Server crate for managing development boards, serial sessions, and TFTP files.

pub mod admin_events;
pub mod admin_monitor;
pub mod admin_power;
pub mod api;
pub mod audit;
pub mod board_pool;
pub mod config;
pub mod dashboard;
pub mod device;
pub mod environment;
pub mod hardware;
pub mod http_boot;
pub mod loader;
pub mod network_test;
pub mod ota;
pub mod persistence;
pub mod power;
pub mod process;
pub mod security;
pub mod serial;
pub mod session;
pub mod state;
pub mod static_assets;
pub mod tftp;
pub mod uploads;
pub mod virtual_lab;
pub mod virtual_qemu;

pub use api::router::build_router;
pub use config::{
    AuthConfig, AxloaderSerialFlowControl, AxloaderSerialParameters, AxloaderSerialParity,
    AxloaderSerialStopBits, BoardConfig, BoardNetworkIdentity, BootConfig, BuiltinTftpConfig,
    CustomPowerManagement, DatabaseConfig, LoaderNetworkConfig, PowerManagementConfig, PxeProfile,
    SerialConfig, SerialPortKey, SerialPortKeyKind, ServerConfig, SystemTftpdHpaConfig, TftpConfig,
    TftpNetworkConfig, UbootNetworkMode, UbootProfile, UefiBootArch, UefiHttpProfile,
    UploadLimitsConfig, VirtualQemuConfig, ZhongshengRelayPowerManagement,
};
pub use persistence::{BoardStore, DtbFile, DtbStore};
pub use state::{AppState, BoardLeaseState, build_app_state};

#[cfg(test)]
extern crate self as ostool_server;
#[cfg(test)]
#[path = "../tests/support/database.rs"]
pub(crate) mod test_support;
