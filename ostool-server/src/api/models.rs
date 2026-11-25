use chrono::{DateTime, Utc};
pub use httpboot_protocol::{KernelPublishResponse, LoaderStatusResponse};
use httpboot_protocol::{LoaderHardwareInfo, MacAddress};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::{
    config::{
        BoardConfig, BoardNetworkIdentity, BootConfig, PowerManagementConfig, SerialConfig,
        SerialPortKey, SerialPortKeyKind, TftpConfig, TftpNetworkConfig, UploadLimitsConfig,
    },
    persistence::dtbs::DtbFile,
    session::Session,
    state::BoardLeaseState,
    tftp::{files::TftpFileRef, status::TftpStatus},
};

/// Anonymous device directory. Never serialize BoardConfig into this response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PublicDevice {
    pub id: String,
    pub board_type: String,
    pub tags: Vec<String>,
    pub disabled: bool,
}

impl From<&BoardConfig> for PublicDevice {
    fn from(board: &BoardConfig) -> Self {
        Self {
            id: board.id.clone(),
            board_type: board.board_type.clone(),
            tags: board.tags.clone(),
            disabled: board.disabled,
        }
    }
}

/// Inventory projection. Operational commands and boot-network configuration
/// belong only in the explicitly selected administrator editor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminBoardSummary {
    pub allow_instant_use: bool,
    pub id: String,
    pub board_type: String,
    pub tags: Vec<String>,
    pub disabled: bool,
    pub serial: Option<SerialConfig>,
    pub network_identity: Option<BoardNetworkIdentity>,
    pub boot: AdminBootSummary,
    pub power_management: AdminPowerSummary,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AdminPowerSummary {
    Custom,
    ZhongshengRelay {
        key: SerialPortKey,
        resolved_device_path: Option<String>,
    },
    Qemu {
        virtual_device_id: String,
    },
}
impl From<&PowerManagementConfig> for AdminPowerSummary {
    fn from(power: &PowerManagementConfig) -> Self {
        match power {
            PowerManagementConfig::Custom(_) => Self::Custom,
            PowerManagementConfig::ZhongshengRelay(relay) => Self::ZhongshengRelay {
                key: relay.key.clone(),
                resolved_device_path: crate::serial::discovery::resolve_serial_key(&relay.key)
                    .ok()
                    .map(|p| p.current_device_path),
            },
            PowerManagementConfig::Qemu { virtual_device_id } => Self::Qemu {
                virtual_device_id: virtual_device_id.clone(),
            },
        }
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminBootSummary {
    pub kind: String,
}
impl From<BoardConfig> for AdminBoardSummary {
    fn from(board: BoardConfig) -> Self {
        Self {
            power_management: AdminPowerSummary::from(&board.power_management),
            allow_instant_use: board.allow_instant_use,
            id: board.id,
            board_type: board.board_type,
            tags: board.tags,
            disabled: board.disabled,
            serial: board.serial,
            network_identity: board.network_identity,
            boot: AdminBootSummary {
                kind: board.boot.kind_name().into(),
            },
        }
    }
}

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub board_id: Option<String>,
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
    /// Operational configuration is available only through the management API.
    pub board: PublicDevice,
    pub serial_available: bool,
    pub serial_connected: bool,
    pub files: Vec<SharedSessionFileResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialStatusResponse {
    pub runtime: crate::serial::runtime::SerialRuntimeStatus,
    pub manager: ostool_serial::ManagerSnapshot,
    pub available: bool,
    pub connected: bool,
    pub port: Option<String>,
    pub baud_rate: Option<u32>,
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialPortSummary {
    pub current_device_path: String,
    pub port_type: String,
    pub label: String,
    pub primary_key_kind: Option<SerialPortKeyKind>,
    pub primary_key_value: Option<String>,
    pub usb_path: Option<String>,
    pub usb_id: Option<String>,
    pub stable_identity: bool,
    pub usb_vendor_id: Option<u16>,
    pub usb_product_id: Option<u16>,
    pub manufacturer: Option<String>,
    pub product: Option<String>,
    pub serial_number: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetDisabledRequest {
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminBoardUpsertRequest {
    #[serde(default)]
    pub allow_instant_use: bool,
    #[serde(default)]
    pub reset_command: Option<String>,
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
    #[serde(default)]
    pub network_identity: Option<BoardNetworkIdentity>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoaderDeviceSummary {
    pub mac_address: MacAddress,
    pub current_mac_address: MacAddress,
    pub ip_address: String,
    pub arch: String,
    pub loader_version: String,
    pub hardware: LoaderHardwareInfo,
    pub last_seen_at: DateTime<Utc>,
    pub online: bool,
    pub conflict: bool,
    pub bound_board_id: Option<String>,
    pub boot_epoch: String,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateVirtualDeviceRequest {
    #[serde(default)]
    pub mac_address: Option<MacAddress>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualDeviceSummary {
    pub id: String,
    pub mac_address: MacAddress,
    pub tap: String,
    pub powered: bool,
    pub serial_connected: bool,
    pub generation: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VirtualDevicesResponse {
    pub enabled: bool,
    pub devices: Vec<VirtualDeviceSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DtbFileResponse {
    pub name: String,
    pub size: u64,
    pub disabled: bool,
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
            disabled: file.disabled,
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
pub struct SharedSessionFileResponse {
    pub filename: String,
    pub relative_path: String,
    pub tftp_url: Option<Url>,
    pub http_url: Option<Url>,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

impl SharedSessionFileResponse {
    pub fn from_file(file: TftpFileRef, tftp_url: Option<Url>, http_url: Option<Url>) -> Self {
        Self {
            filename: file.filename,
            relative_path: file.relative_path,
            tftp_url,
            http_url,
            size: file.size,
            uploaded_at: file.uploaded_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBootFileResponse {
    pub filename: String,
    pub relative_path: String,
    pub http_url: Url,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

impl HttpBootFileResponse {
    pub fn from_file(file: TftpFileRef, http_url: Url) -> Self {
        Self {
            filename: file.filename,
            relative_path: file.relative_path,
            http_url,
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
    pub files: Vec<SharedSessionFileResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeartbeatResponse {
    pub session_id: String,
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDtbResponse {
    pub dtb_name: Option<String>,
    pub relative_path: Option<String>,
    pub session_file_path: Option<String>,
    pub tftp_url: Option<Url>,
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
    pub sessions: Vec<AdminSession>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSession {
    #[serde(flatten)]
    pub session: Session,
    pub owner_username: Option<String>,
    pub owner_user_id: Option<uuid::Uuid>,
    pub rental_id: Option<uuid::Uuid>,
    pub rental_starts_at: Option<chrono::DateTime<chrono::Utc>>,
    pub rental_ends_at: Option<chrono::DateTime<chrono::Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootProfileResponse {
    pub server_side_operations: bool,
    pub boot: BootConfig,
    pub server_ip: Option<String>,
    pub netmask: Option<String>,
    pub interface: Option<String>,
    pub http_base_url: Option<Url>,
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
    pub http_boot_public_base_url: Option<String>,
    pub dtb_upload_max_mib: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminServerConfigEditable {
    pub audit_retention_days: u32,
    pub network: TftpNetworkConfig,
    pub upload_limits: UploadLimitsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminServerConfigResponse {
    pub readonly: AdminServerConfigReadonly,
    pub editable: AdminServerConfigEditable,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateServerConfigRequest {
    #[serde(default)]
    pub audit_retention_days: Option<u32>,
    pub network: TftpNetworkConfig,
    pub upload_limits: UploadLimitsConfig,
}
