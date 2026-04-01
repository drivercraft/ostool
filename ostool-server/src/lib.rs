//! Server crate for managing development boards, serial sessions, and TFTP files.

pub mod api;
pub mod board_pool;
pub mod board_store;
pub mod config;
pub mod dtb_store;
pub mod power;
pub mod process;
pub mod serial;
pub mod session;
pub mod state;
pub mod tftp;
pub mod web;

pub use api::router::build_router;
pub use config::{
    BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement, PowerManagementConfig,
    PxeProfile, SerialConfig, ServerConfig, SystemTftpdHpaConfig, TftpConfig, TftpNetworkConfig,
    UbootProfile, ZhongshengRelayPowerManagement,
};
pub use dtb_store::{DtbFile, DtbStore};
pub use state::{AppState, build_app_state};
