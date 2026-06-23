//! Server crate for managing development boards, serial sessions, and TFTP files.

pub mod api;
pub mod auth;
pub mod board_pool;
pub mod config;
pub mod dtb_store;
pub mod http_boot;
pub mod lease;
pub mod power;
pub mod process;
pub mod seed;
pub mod serial;
pub mod session;
pub mod state;
pub mod storage;
pub mod tftp;
pub mod web;

pub use api::router::build_router;
pub use config::{
    AdminSeedConfig, BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement,
    DatabaseConfig, PowerManagementConfig, PxeProfile, SampleDataConfig, SerialConfig,
    SerialPortKey, SerialPortKeyKind, ServerConfig, SystemTftpdHpaConfig, TftpConfig,
    TftpNetworkConfig, UbootNetworkMode, UbootProfile, UefiBootArch, UefiHttpProfile,
    UploadLimitsConfig, ZhongshengRelayPowerManagement,
};
pub use dtb_store::{DtbFile, DtbStore};
pub use state::{AppState, BoardLeaseState, build_app_state};
