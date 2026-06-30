use chrono::{DateTime, Utc};
pub use httpboot_protocol::KernelPublishResponse;
use serde::{Deserialize, Serialize};

use crate::{
    config::{
        BoardConfig, BootConfig, PowerManagementConfig, SerialConfig, SerialPortKeyKind,
        TftpConfig, TftpNetworkConfig, UploadLimitsConfig,
    },
    dtb_store::DtbFile,
    session::Session,
    state::BoardLeaseState,
    storage::{DtbMetadata, Lease, Permission, Role, SessionRecord},
    tftp::{files::TftpFileRef, status::TftpStatus},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
    pub captcha_token: String,
    pub captcha_answer: String,
}

/// Self-service registration payload. Submitted by the public `/register` page.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegisterRequest {
    pub username: String,
    pub display_name: Option<String>,
    pub email: String,
    pub password: String,
    pub confirm_password: String,
    pub captcha_token: String,
    pub captcha_answer: String,
    /// Optional profile fields mirrors the admin create form.
    #[serde(default)]
    pub phone: Option<String>,
    #[serde(default)]
    pub department: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
}

/// Returned by `/api/v1/auth/register`. Tells the client what happened so the
/// UI can show the right next-step message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "outcome")]
pub enum RegisterResponse {
    /// Self-registration is disabled on this platform.
    Closed,
    /// Account is active and the user may log in now.
    Active {
        username: String,
        display_name: String,
    },
    /// Account was created but is pending admin approval.
    Pending {
        username: String,
        display_name: String,
    },
}

/// Public endpoint that tells the register/login pages which flow to render.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrationPolicyResponse {
    /// `closed` | `auto` | `approval`
    pub mode: String,
    pub self_service_enabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CaptchaResponse {
    pub token: String,
    pub image_svg: String,
    pub expires_in_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentUserResponse {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub email: String,
    pub phone: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
    pub last_login_at: Option<DateTime<Utc>>,
    pub roles: Vec<AdminRoleResponse>,
    pub permissions: Vec<AdminPermissionResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateLeaseRequest {
    pub board_type: String,
    #[serde(default)]
    pub required_tags: Vec<String>,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseResponse {
    pub lease: Lease,
    pub session: Option<Session>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeasesResponse {
    pub leases: Vec<LeaseResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminLeaseCreateRequest {
    pub user_id: String,
    pub board_id: String,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub client_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminLeaseUpdateRequest {
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUserResponse {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub email: String,
    pub phone: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
    pub disabled: bool,
    /// `active` | `pending` | `rejected` | `disabled`
    pub status: String,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUsersResponse {
    pub users: Vec<AdminUserResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUserCreateRequest {
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub phone: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
    pub password: String,
    #[serde(default)]
    pub role_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUserUpdateRequest {
    pub display_name: String,
    pub email: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub phone: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminPasswordResetRequest {
    pub password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserPasswordUpdateRequest {
    pub current_password: String,
    pub new_password: String,
    pub confirm_new_password: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminPermissionResponse {
    pub id: String,
    pub code: String,
    pub name: String,
    pub description: String,
}

impl From<Permission> for AdminPermissionResponse {
    fn from(value: Permission) -> Self {
        Self {
            id: value.id,
            code: value.code,
            name: value.name,
            description: value.description,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminRoleResponse {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub system: bool,
    pub disabled: bool,
    pub user_count: u64,
    pub permissions: Vec<AdminPermissionResponse>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

impl AdminRoleResponse {
    pub fn new(role: Role, permissions: Vec<Permission>, user_count: u64) -> Self {
        Self {
            id: role.id,
            name: role.name,
            display_name: role.display_name,
            description: role.description,
            system: role.system,
            disabled: role.disabled,
            user_count,
            permissions: permissions
                .into_iter()
                .map(AdminPermissionResponse::from)
                .collect(),
            created_at: role.created_at,
            updated_at: role.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminRolesResponse {
    pub roles: Vec<AdminRoleResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminPermissionsResponse {
    pub permissions: Vec<AdminPermissionResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminRoleCreateRequest {
    pub name: String,
    pub display_name: String,
    pub description: String,
    #[serde(default)]
    pub permission_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminRoleUpdateRequest {
    pub display_name: String,
    pub description: String,
    #[serde(default)]
    pub permission_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminRoleDisableRequest {
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUserRolesResponse {
    pub roles: Vec<AdminRoleResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminUserRolesUpdateRequest {
    pub role_ids: Vec<String>,
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
pub struct AdminSessionResponse {
    pub session: SessionRecord,
    pub lease: Option<Lease>,
    pub user_id: Option<String>,
    pub source_ip: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSessionUpdateRequest {
    pub client_name: Option<String>,
    pub failure_message: Option<String>,
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
    pub current_device_path: String,
    pub port_type: String,
    pub label: String,
    pub primary_key_kind: Option<SerialPortKeyKind>,
    pub primary_key_value: Option<String>,
    pub usb_path: Option<String>,
    pub stable_identity: bool,
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
    pub storage_path: Option<String>,
    pub sha256: Option<String>,
    pub boot_architecture: Option<String>,
    pub compatible: Option<String>,
    pub description: Option<String>,
    pub disabled: bool,
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
            storage_path: None,
            sha256: None,
            boot_architecture: None,
            compatible: None,
            description: None,
            disabled: false,
        }
    }

    pub fn from_dtb_with_metadata(file: DtbFile, metadata: Option<DtbMetadata>) -> Self {
        let mut response = Self::from_dtb(file);
        if let Some(metadata) = metadata {
            response.storage_path = Some(metadata.storage_path);
            response.sha256 = Some(metadata.sha256);
            response.boot_architecture = metadata.boot_architecture;
            response.compatible = metadata.compatible;
            response.description = metadata.description;
            response.disabled = metadata.disabled;
        }
        response
    }
}

impl From<DtbMetadata> for DtbFileResponse {
    fn from(metadata: DtbMetadata) -> Self {
        let name = metadata.name;
        Self {
            relative_tftp_path_template: format!("boot/dtb/{name}"),
            name,
            size: metadata.size_bytes.max(0) as u64,
            updated_at: metadata.updated_at,
            storage_path: Some(metadata.storage_path),
            sha256: Some(metadata.sha256),
            boot_architecture: metadata.boot_architecture,
            compatible: metadata.compatible,
            disabled: metadata.disabled,
            description: metadata.description,
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
pub struct HttpBootFileResponse {
    pub filename: String,
    pub relative_path: String,
    pub http_url: String,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

impl HttpBootFileResponse {
    pub fn from_file(file: TftpFileRef, http_url: String) -> Self {
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
    pub sessions: Vec<AdminSessionResponse>,
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
    pub http_boot_public_base_url: Option<String>,
    pub dtb_upload_max_mib: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminServerConfigEditable {
    pub network: TftpNetworkConfig,
    pub upload_limits: UploadLimitsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminServerConfigResponse {
    pub readonly: AdminServerConfigReadonly,
    pub editable: AdminServerConfigEditable,
    pub site: SiteSettingsResponse,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpdateServerConfigRequest {
    pub editable: AdminServerConfigEditable,
    pub site: SiteSettingsUpdateRequest,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettingsResponse {
    pub site_name: String,
    pub site_subtitle: String,
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
    pub announcement: Option<String>,
    pub maintenance_mode: bool,
    pub self_service_enabled: bool,
    pub registration_mode: String,
    pub default_lease_minutes: i64,
    pub max_lease_minutes: i64,
    pub support_email: Option<String>,
    pub support_url: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettingsUpdateRequest {
    pub site_name: String,
    pub site_subtitle: String,
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
    pub announcement: Option<String>,
    pub maintenance_mode: bool,
    pub self_service_enabled: bool,
    pub registration_mode: String,
    pub default_lease_minutes: i64,
    pub max_lease_minutes: i64,
    pub support_email: Option<String>,
    pub support_url: Option<String>,
}
