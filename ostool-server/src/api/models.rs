use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::{
        BoardConfig, BootConfig, PowerManagementConfig, SerialConfig, TftpConfig, TftpNetworkConfig,
    },
    dtb_store::DtbFile,
    session::Session,
    state::BoardLeaseState,
    tftp::{files::TftpFileRef, status::TftpStatus},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardTypeSummary {
    pub board_type: String,
    pub tags: Vec<String>,
    pub total: usize,
    pub available: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub board_type: String,
    #[serde(default)]
    pub required_tags: Vec<String>,
    pub client_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCreatedResponse {
    pub session_id: String,
    pub board_id: String,
    pub lease_expires_at: DateTime<Utc>,
    pub serial_available: bool,
    pub boot_mode: String,
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetailResponse {
    pub session: Session,
    pub board: BoardConfig,
    pub serial_available: bool,
    pub serial_connected: bool,
    pub files: Vec<FileResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialStatusResponse {
    pub available: bool,
    pub connected: bool,
    pub port: Option<String>,
    pub baud_rate: Option<u32>,
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialPortSummary {
    pub port_name: String,
    pub port_type: String,
    pub label: String,
    pub usb_vendor_id: Option<u16>,
    pub usb_product_id: Option<u16>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminBoardUpsertRequest {
    pub id: Option<String>,
    pub board_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub notes: Option<String>,
    #[serde(default)]
    pub disabled: bool,
    pub serial: Option<SerialConfig>,
    pub power_management: PowerManagementConfig,
    pub boot: BootConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DtbFileResponse {
    pub name: String,
    pub size: u64,
    pub updated_at: DateTime<Utc>,
    pub relative_tftp_path_template: String,
}

impl DtbFileResponse {
    pub fn from_dtb(file: DtbFile) -> Self {
        let name = file.name;
        Self {
            relative_tftp_path_template: format!("boot/dtb/{name}"),
            name,
            size: file.size,
            updated_at: file.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkInterfaceSummary {
    pub name: String,
    pub label: String,
    pub ipv4_addresses: Vec<String>,
    pub netmask: Option<String>,
    pub loopback: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResponse {
    pub filename: String,
    pub relative_path: String,
    pub tftp_url: Option<String>,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

impl FileResponse {
    pub fn from_file(file: TftpFileRef, tftp_url: Option<String>) -> Self {
        Self {
            filename: file.filename,
            relative_path: file.relative_path,
            tftp_url,
            size: file.size,
            uploaded_at: file.uploaded_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TftpSessionResponse {
    pub available: bool,
    pub provider: String,
    pub server_ip: Option<String>,
    pub netmask: Option<String>,
    pub writable: bool,
    pub files: Vec<FileResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDtbResponse {
    pub dtb_name: Option<String>,
    pub relative_path: Option<String>,
    pub session_file_path: Option<String>,
    pub tftp_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResponse {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardPowerAction {
    PowerOn,
    PowerOff,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardPowerStatusResponse {
    pub available: bool,
    pub powered: Option<bool>,
    pub last_action: Option<BoardPowerAction>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardRuntimeStatusResponse {
    pub lease_state: BoardLeaseState,
    pub active_session_id: Option<String>,
    pub last_release_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminTftpConfigResponse {
    pub tftp: TftpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminTftpStatusResponse {
    pub status: TftpStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSessionsResponse {
    pub sessions: Vec<Session>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootProfileResponse {
    pub boot: BootConfig,
    pub server_ip: Option<String>,
    pub netmask: Option<String>,
    pub interface: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminOverviewResponse {
    pub board_count_total: usize,
    pub board_count_available: usize,
    pub disabled_board_count: usize,
    pub active_session_count: usize,
    pub board_types: Vec<BoardTypeSummary>,
    pub tftp_status: TftpStatus,
    pub server: AdminServerConfigReadonly,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminServerConfigReadonly {
    pub listen_addr: String,
    pub data_dir: String,
    pub board_dir: String,
    pub dtb_dir: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminServerConfigEditable {
    pub network: TftpNetworkConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminServerConfigResponse {
    pub readonly: AdminServerConfigReadonly,
    pub editable: AdminServerConfigEditable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateServerConfigRequest {
    pub network: TftpNetworkConfig,
}
