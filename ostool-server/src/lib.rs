//! Server crate for managing development boards, serial sessions, and TFTP files.

pub mod admin_events;
pub mod admin_monitor;
pub mod admin_power;
pub mod api;
pub mod audit;
pub mod board_pool;
pub mod config;
pub mod dashboard;
pub mod demo;
pub mod device;
pub mod environment;
pub mod http_boot;
pub mod loader;
pub mod network_test;
pub mod ota;
pub mod persistence;
pub mod power;
pub mod process;
pub mod serial;
pub mod session;
pub mod state;
pub mod static_assets;
pub mod tftp;
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
#[doc(hidden)]
pub use state::build_fixture_app_state;
pub use state::{AppState, BoardLeaseState, build_app_state};

// Retain earlier public Rust import paths while new code uses persistence/.
#[deprecated(note = "use persistence::postgres")]
pub mod database {
    pub use crate::persistence::postgres::*;
}
#[deprecated(note = "use persistence::boards or persistence::legacy::boards")]
pub mod board_store {
    pub mod fs {
        pub use crate::persistence::boards::{BoardStore as FileBoardStore, QuarantinedBoard};
    }
}
#[deprecated(note = "use persistence::dtbs")]
pub mod dtb_store {
    pub use crate::persistence::dtbs::*;
}

#[deprecated(note = "use static_assets")]
pub mod web {
    pub use crate::static_assets::*;
}
