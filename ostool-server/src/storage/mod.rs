pub mod mysql;
pub mod sqlite;

use std::{collections::BTreeMap, sync::Arc};

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::{BoardConfig, TftpNetworkConfig, UploadLimitsConfig};

/// Account lifecycle status. Drives the registration / approval workflow.
///
/// - `Active`   — user can log in normally (admin-created or approved self-register)
/// - `Pending`  — self-registered while `registration_mode = approval`, awaiting admin review
/// - `Rejected` — admin rejected a pending registration; cannot log in
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UserStatus {
    Active,
    Pending,
    Rejected,
}

impl UserStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Pending => "pending",
            Self::Rejected => "rejected",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "pending" => Ok(Self::Pending),
            "rejected" => Ok(Self::Rejected),
            other => anyhow::bail!("unknown user status `{other}`"),
        }
    }

    /// A user in this status is allowed to pass the login gate.
    pub fn can_login(self) -> bool {
        matches!(self, Self::Active)
    }
}

impl Default for UserStatus {
    fn default() -> Self {
        Self::Active
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub email: String,
    pub phone: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
    pub password_hash: String,
    pub disabled: bool,
    pub status: UserStatus,
    pub last_login_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewUser {
    pub username: String,
    pub display_name: String,
    pub email: String,
    pub password_hash: String,
    pub profile: UserProfile,
    pub role_names: Vec<String>,
    pub status: UserStatus,
}

#[derive(Debug, Clone, Default)]
pub struct UserProfile {
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub phone: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Permission {
    pub id: String,
    pub code: String,
    pub name: String,
    pub description: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

pub const BUILTIN_PERMISSIONS: &[(&str, &str, &str)] = &[
    ("overview.read", "查看概览", "查看站点运行情况和统计数据"),
    ("users.read", "查看用户", "查看用户列表、用户详情和用户角色"),
    ("users.create", "新增用户", "创建用户账号"),
    ("users.update", "编辑用户", "编辑用户资料、状态和分配角色"),
    ("users.delete", "删除用户", "删除或停用用户账号"),
    (
        "users.password.update",
        "重置用户密码",
        "为用户重置登录密码",
    ),
    ("roles.read", "查看角色权限", "查看角色、权限和分配情况"),
    ("roles.create", "新增角色", "创建角色并分配权限"),
    ("roles.update", "编辑角色", "修改角色信息和权限"),
    ("roles.delete", "删除角色", "删除自定义角色"),
    ("boards.read", "查看开发板", "查看开发板配置和运行状态"),
    ("boards.create", "新增开发板", "新增开发板配置"),
    (
        "boards.update",
        "编辑开发板",
        "编辑开发板配置、串口、电源和启动参数",
    ),
    ("boards.delete", "删除开发板", "删除开发板配置"),
    ("dtbs.read", "查看 DTB", "查看 DTB 文件和元数据"),
    ("dtbs.create", "上传 DTB", "上传新的 DTB 文件"),
    (
        "dtbs.update",
        "编辑 DTB",
        "编辑 DTB 元数据、重命名或替换文件",
    ),
    ("dtbs.delete", "删除 DTB", "删除 DTB 文件"),
    ("leases.read", "查看租赁", "查看租赁情况"),
    ("leases.create", "新增租赁", "为用户创建开发板租赁"),
    ("leases.update", "编辑租赁", "修改租赁时间段和状态信息"),
    ("leases.start", "启用租赁", "为有效租赁启动会话"),
    ("leases.release", "释放租赁", "释放租赁占用的会话"),
    ("leases.heartbeat", "租赁心跳", "续约自己持有的活跃租赁"),
    ("leases.delete", "删除租赁", "删除租赁记录"),
    ("sessions.read", "查看租约会话", "查看租约会话和历史记录"),
    (
        "sessions.create",
        "新增租约会话",
        "为可用开发板创建租约会话",
    ),
    (
        "sessions.update",
        "更新租约会话",
        "更新租约会话心跳和运行状态",
    ),
    ("sessions.delete", "删除租约会话", "删除租约会话记录"),
    ("issues.read", "查看问题会话", "查看用户反馈的问题会话"),
    ("issues.create", "提交问题会话", "提交用户反馈的问题会话"),
    (
        "issues.update",
        "处理问题会话",
        "更新问题会话状态和处理备注",
    ),
    ("issues.delete", "删除问题会话", "删除问题会话记录"),
    ("announcements.read", "查看公告", "查看公告管理和公告内容"),
    ("announcements.create", "新增公告", "创建平台公告"),
    ("announcements.update", "编辑公告", "编辑平台公告内容和状态"),
    ("announcements.delete", "删除公告", "删除平台公告"),
    ("tftp.read", "查看 TFTP 配置", "查看 TFTP 配置和运行状态"),
    (
        "tftp.reconcile",
        "同步 TFTP 配置",
        "同步 TFTP provider 的运行配置",
    ),
    ("server.read", "查看服务器配置", "查看服务器运行期设置"),
    ("server.update", "编辑服务器配置", "修改服务器运行期设置"),
    ("site.read", "查看站点设置", "查看站点展示和租赁策略设置"),
    ("site.update", "编辑站点设置", "修改站点展示和租赁策略设置"),
    ("audit.read", "查看审计日志", "查看系统审计日志和行为轨迹"),
    ("serial_ports.read", "查看串口", "查看服务器可用串口"),
    (
        "network_interfaces.read",
        "查看网络接口",
        "查看服务器网络接口",
    ),
    ("permissions.read", "查看权限", "查看系统内置权限列表"),
];

pub fn default_user_permission(code: &str) -> bool {
    matches!(
        code,
        "leases.read"
            | "leases.create"
            | "leases.start"
            | "leases.release"
            | "leases.heartbeat"
            | "sessions.read"
            | "sessions.create"
            | "sessions.update"
            | "issues.read"
            | "issues.create"
    )
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub system: bool,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewRole {
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub permission_ids: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AuthSession {
    pub id: String,
    pub user_id: String,
    pub token_hash: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub expires_at: DateTime<Utc>,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub revoked_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LeaseState {
    Active,
    Releasing,
    Released,
    Expired,
    Failed,
}

impl LeaseState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Releasing => "releasing",
            Self::Released => "released",
            Self::Expired => "expired",
            Self::Failed => "failed",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "active" => Ok(Self::Active),
            "releasing" => Ok(Self::Releasing),
            "released" => Ok(Self::Released),
            "expired" => Ok(Self::Expired),
            "failed" => Ok(Self::Failed),
            other => anyhow::bail!("unknown lease state `{other}`"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Lease {
    pub id: String,
    pub user_id: String,
    pub session_id: Option<String>,
    pub board_id: String,
    pub board_type: String,
    pub required_tags: Vec<String>,
    pub state: LeaseState,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub released_at: Option<DateTime<Utc>>,
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewLease {
    pub user_id: String,
    pub session_id: Option<String>,
    pub board_id: String,
    pub board_type: String,
    pub required_tags: Vec<String>,
    pub starts_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub id: String,
    pub board_id: String,
    pub client_name: Option<String>,
    pub source_ip: Option<String>,
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub failure_message: Option<String>,
}

#[derive(Debug, Clone)]
pub struct NewSessionRecord {
    pub id: String,
    pub board_id: String,
    pub client_name: Option<String>,
    pub source_ip: Option<String>,
    pub state: String,
    pub created_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IssueSessionState {
    Open,
    InProgress,
    Resolved,
    Closed,
}

impl IssueSessionState {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::InProgress => "in_progress",
            Self::Resolved => "resolved",
            Self::Closed => "closed",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "open" => Ok(Self::Open),
            "in_progress" => Ok(Self::InProgress),
            "resolved" => Ok(Self::Resolved),
            "closed" => Ok(Self::Closed),
            other => anyhow::bail!("unknown issue session state `{other}`"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IssueSessionPriority {
    Low,
    Normal,
    High,
    Urgent,
}

impl IssueSessionPriority {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Normal => "normal",
            Self::High => "high",
            Self::Urgent => "urgent",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "low" => Ok(Self::Low),
            "normal" => Ok(Self::Normal),
            "high" => Ok(Self::High),
            "urgent" => Ok(Self::Urgent),
            other => anyhow::bail!("unknown issue session priority `{other}`"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IssueSession {
    pub id: String,
    pub user_id: String,
    pub lease_id: Option<String>,
    pub session_id: Option<String>,
    pub title: String,
    pub category: String,
    pub description: String,
    pub state: IssueSessionState,
    pub priority: IssueSessionPriority,
    pub handler_user_id: Option<String>,
    pub resolution: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct NewIssueSession {
    pub user_id: String,
    pub lease_id: Option<String>,
    pub session_id: Option<String>,
    pub title: String,
    pub category: String,
    pub description: String,
    pub priority: IssueSessionPriority,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnnouncementStatus {
    Draft,
    Published,
    Hidden,
}

impl AnnouncementStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Draft => "draft",
            Self::Published => "published",
            Self::Hidden => "hidden",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "draft" => Ok(Self::Draft),
            "published" => Ok(Self::Published),
            "hidden" => Ok(Self::Hidden),
            other => anyhow::bail!("unknown announcement status `{other}`"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AnnouncementKind {
    System,
    Activity,
}

impl AnnouncementKind {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Activity => "activity",
        }
    }

    pub fn from_str(value: &str) -> anyhow::Result<Self> {
        match value {
            "system" => Ok(Self::System),
            "activity" => Ok(Self::Activity),
            other => anyhow::bail!("unknown announcement kind `{other}`"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Announcement {
    pub id: String,
    pub title: String,
    pub content: String,
    pub kind: AnnouncementKind,
    pub status: AnnouncementStatus,
    pub pinned: bool,
    pub created_by: Option<String>,
    pub updated_by: Option<String>,
    pub published_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewAnnouncement {
    pub title: String,
    pub content: String,
    pub kind: AnnouncementKind,
    pub status: AnnouncementStatus,
    pub pinned: bool,
    pub created_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DtbMetadata {
    pub id: String,
    pub name: String,
    pub storage_path: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub boot_architecture: Option<String>,
    pub compatible: Option<String>,
    pub description: Option<String>,
    pub disabled: bool,
    pub uploaded_by: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct UpsertDtbMetadata {
    pub name: String,
    pub storage_path: String,
    pub size_bytes: i64,
    pub sha256: String,
    pub boot_architecture: Option<String>,
    pub compatible: Option<String>,
    pub description: Option<String>,
    pub disabled: bool,
    pub uploaded_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditLog {
    pub id: String,
    pub actor_user_id: Option<String>,
    pub actor_username: Option<String>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub outcome: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub request_id: Option<String>,
    pub metadata_json: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone)]
pub struct NewAuditLog {
    pub actor_user_id: Option<String>,
    pub actor_username: Option<String>,
    pub action: String,
    pub target_type: String,
    pub target_id: Option<String>,
    pub outcome: String,
    pub ip_address: Option<String>,
    pub user_agent: Option<String>,
    pub request_id: Option<String>,
    pub metadata_json: String,
}

/// Self-service registration policy. Stored as `registration.mode` site setting.
///
/// - `Closed`  — self-registration disabled; only admins can create accounts
/// - `Auto`    — self-registered accounts activate immediately
/// - `Approval` — self-registered accounts land in `Pending` status until an admin
///   approves (`Active`) or rejects (`Rejected`) them
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RegistrationMode {
    Closed,
    Auto,
    Approval,
}

impl RegistrationMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Auto => "auto",
            Self::Approval => "approval",
        }
    }

    pub fn parse(value: &str) -> anyhow::Result<Self> {
        match value {
            "closed" => Ok(Self::Closed),
            "auto" => Ok(Self::Auto),
            "approval" => Ok(Self::Approval),
            other => anyhow::bail!("unknown registration mode `{other}`"),
        }
    }
}

impl Default for RegistrationMode {
    fn default() -> Self {
        Self::Closed
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettings {
    pub site_name: String,
    pub site_subtitle: String,
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
    pub announcement: Option<String>,
    pub maintenance_mode: bool,
    pub self_service_enabled: bool,
    pub registration_mode: RegistrationMode,
    pub default_lease_minutes: i64,
    pub max_lease_minutes: i64,
    pub support_email: Option<String>,
    pub support_url: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimeSettings {
    pub network: TftpNetworkConfig,
    pub upload_limits: UploadLimitsConfig,
    pub updated_at: DateTime<Utc>,
}

impl Default for RuntimeSettings {
    fn default() -> Self {
        Self {
            network: TftpNetworkConfig::default(),
            upload_limits: UploadLimitsConfig::default(),
            updated_at: Utc::now(),
        }
    }
}

impl RuntimeSettings {
    pub fn validate(&mut self) -> anyhow::Result<()> {
        self.network.interface = self.network.interface.trim().to_string();
        if self.upload_limits.session_file_max_mib == 0 {
            anyhow::bail!("session file upload limit must be greater than 0");
        }
        Ok(())
    }
}

impl Default for SiteSettings {
    fn default() -> Self {
        Self {
            site_name: "ostool-server".to_string(),
            site_subtitle: "开发板租赁平台".to_string(),
            logo_url: None,
            favicon_url: None,
            announcement: None,
            maintenance_mode: false,
            self_service_enabled: true,
            registration_mode: RegistrationMode::Closed,
            default_lease_minutes: 120,
            max_lease_minutes: 480,
            support_email: None,
            support_url: None,
            updated_at: Utc::now(),
        }
    }
}

impl SiteSettings {
    pub fn validate(&mut self) -> anyhow::Result<()> {
        self.site_name = trim_or_default(&self.site_name, "ostool-server");
        self.site_subtitle = trim_or_default(&self.site_subtitle, "开发板租赁平台");
        self.logo_url = clean_optional(self.logo_url.take());
        self.favicon_url = clean_optional(self.favicon_url.take());
        self.announcement = clean_optional(self.announcement.take());
        self.support_email = clean_optional(self.support_email.take());
        self.support_url = clean_optional(self.support_url.take());
        if self.default_lease_minutes <= 0 {
            anyhow::bail!("default lease minutes must be greater than 0");
        }
        if self.max_lease_minutes < self.default_lease_minutes {
            anyhow::bail!(
                "max lease minutes must be greater than or equal to default lease minutes"
            );
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub(crate) struct SiteSettingDefinition {
    pub key: &'static str,
    pub value_json: String,
    pub value_type: &'static str,
    pub group_name: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub readonly: bool,
    pub sensitive: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone)]
pub(crate) struct SiteSettingValue {
    pub key: String,
    pub value_json: String,
    pub updated_at: String,
}

pub(crate) fn default_site_setting_rows() -> Vec<SiteSettingDefinition> {
    let settings = SiteSettings::default();
    let runtime = RuntimeSettings::default();
    let now = settings.updated_at.to_rfc3339();
    vec![
        site_setting_definition(
            "site.name",
            &settings.site_name,
            "string",
            "site",
            "站点名称",
            "展示在页面标题和导航栏中的站点名称",
            &now,
        ),
        site_setting_definition(
            "site.subtitle",
            &settings.site_subtitle,
            "string",
            "site",
            "站点副标题",
            "用于说明平台定位的短文本",
            &now,
        ),
        site_setting_definition(
            "site.logo_url",
            &settings.logo_url,
            "string",
            "site",
            "Logo URL",
            "可选的站点 Logo 图片地址",
            &now,
        ),
        site_setting_definition(
            "site.favicon_url",
            &settings.favicon_url,
            "string",
            "site",
            "Favicon URL",
            "可选的浏览器标签页图标地址",
            &now,
        ),
        site_setting_definition(
            "site.announcement",
            &settings.announcement,
            "string",
            "site",
            "平台公告",
            "展示给用户的站点公告",
            &now,
        ),
        site_setting_definition(
            "site.maintenance_mode",
            &settings.maintenance_mode,
            "boolean",
            "site",
            "维护模式",
            "打开后可用于前端提示平台处于维护状态",
            &now,
        ),
        site_setting_definition(
            "rental.self_service_enabled",
            &settings.self_service_enabled,
            "boolean",
            "rental",
            "自助租赁",
            "是否允许注册用户自助创建开发板租赁",
            &now,
        ),
        site_setting_definition(
            "registration.mode",
            &settings.registration_mode.as_str().to_string(),
            "string",
            "registration",
            "注册策略",
            "自助注册策略：closed（关闭）/ auto（自动生效）/ approval（管理员审核）",
            &now,
        ),
        site_setting_definition(
            "rental.default_lease_minutes",
            &settings.default_lease_minutes,
            "integer",
            "rental",
            "默认租赁时长",
            "创建租赁时的默认时长，单位分钟",
            &now,
        ),
        site_setting_definition(
            "rental.max_lease_minutes",
            &settings.max_lease_minutes,
            "integer",
            "rental",
            "最大租赁时长",
            "允许配置的最大租赁时长，单位分钟",
            &now,
        ),
        site_setting_definition(
            "support.email",
            &settings.support_email,
            "string",
            "support",
            "支持邮箱",
            "平台支持联系邮箱",
            &now,
        ),
        site_setting_definition(
            "support.url",
            &settings.support_url,
            "string",
            "support",
            "支持链接",
            "平台支持或工单系统链接",
            &now,
        ),
        site_setting_definition(
            "runtime.network_interface",
            &runtime.network.interface,
            "string",
            "runtime",
            "网络接口",
            "用于计算 TFTP/HTTP Boot server_ip 的网络接口，空值表示自动选择",
            &now,
        ),
        site_setting_definition(
            "runtime.session_file_max_mib",
            &runtime.upload_limits.session_file_max_mib,
            "integer",
            "runtime",
            "Session 文件上传上限",
            "Session TFTP/HTTP 文件上传体积上限，单位 MiB",
            &now,
        ),
    ]
}

pub(crate) fn site_settings_to_values(
    settings: &SiteSettings,
) -> anyhow::Result<Vec<(&'static str, String)>> {
    Ok(vec![
        ("site.name", setting_json(&settings.site_name)?),
        ("site.subtitle", setting_json(&settings.site_subtitle)?),
        ("site.logo_url", setting_json(&settings.logo_url)?),
        ("site.favicon_url", setting_json(&settings.favicon_url)?),
        ("site.announcement", setting_json(&settings.announcement)?),
        (
            "site.maintenance_mode",
            setting_json(&settings.maintenance_mode)?,
        ),
        (
            "rental.self_service_enabled",
            setting_json(&settings.self_service_enabled)?,
        ),
        (
            "registration.mode",
            setting_json(&settings.registration_mode.as_str().to_string())?,
        ),
        (
            "rental.default_lease_minutes",
            setting_json(&settings.default_lease_minutes)?,
        ),
        (
            "rental.max_lease_minutes",
            setting_json(&settings.max_lease_minutes)?,
        ),
        ("support.email", setting_json(&settings.support_email)?),
        ("support.url", setting_json(&settings.support_url)?),
    ])
}

pub(crate) fn runtime_settings_to_values(
    settings: &RuntimeSettings,
) -> anyhow::Result<Vec<(&'static str, String)>> {
    Ok(vec![
        (
            "runtime.network_interface",
            setting_json(&settings.network.interface)?,
        ),
        (
            "runtime.session_file_max_mib",
            setting_json(&settings.upload_limits.session_file_max_mib)?,
        ),
    ])
}

pub(crate) fn site_settings_from_values(
    values: Vec<SiteSettingValue>,
) -> anyhow::Result<SiteSettings> {
    let mut settings = SiteSettings::default();
    let mut updated_at = settings.updated_at;
    let mut map = std::collections::HashMap::new();
    for value in values {
        if let Ok(parsed) = parse_time(&value.updated_at)
            && parsed > updated_at
        {
            updated_at = parsed;
        }
        map.insert(value.key, value.value_json);
    }

    settings.site_name = setting_value(&map, "site.name", settings.site_name)?;
    settings.site_subtitle = setting_value(&map, "site.subtitle", settings.site_subtitle)?;
    settings.logo_url = setting_value(&map, "site.logo_url", settings.logo_url)?;
    settings.favicon_url = setting_value(&map, "site.favicon_url", settings.favicon_url)?;
    settings.announcement = setting_value(&map, "site.announcement", settings.announcement)?;
    settings.maintenance_mode =
        setting_value(&map, "site.maintenance_mode", settings.maintenance_mode)?;
    settings.self_service_enabled = setting_value(
        &map,
        "rental.self_service_enabled",
        settings.self_service_enabled,
    )?;
    settings.registration_mode = match map.get("registration.mode") {
        Some(raw) => {
            let value: String = serde_json::from_str(raw)
                .context("failed to parse site setting `registration.mode`")?;
            RegistrationMode::parse(&value)?
        }
        None => settings.registration_mode,
    };
    settings.default_lease_minutes = setting_value(
        &map,
        "rental.default_lease_minutes",
        settings.default_lease_minutes,
    )?;
    settings.max_lease_minutes =
        setting_value(&map, "rental.max_lease_minutes", settings.max_lease_minutes)?;
    settings.support_email = setting_value(&map, "support.email", settings.support_email)?;
    settings.support_url = setting_value(&map, "support.url", settings.support_url)?;
    settings.updated_at = updated_at;
    settings.validate()?;
    Ok(settings)
}

pub(crate) fn runtime_settings_from_values(
    values: Vec<SiteSettingValue>,
) -> anyhow::Result<RuntimeSettings> {
    let mut settings = RuntimeSettings::default();
    let mut updated_at = settings.updated_at;
    let mut map = std::collections::HashMap::new();
    for value in values {
        if let Ok(parsed) = parse_time(&value.updated_at)
            && parsed > updated_at
        {
            updated_at = parsed;
        }
        map.insert(value.key, value.value_json);
    }

    settings.network.interface = setting_value(
        &map,
        "runtime.network_interface",
        settings.network.interface,
    )?;
    settings.upload_limits.session_file_max_mib = setting_value(
        &map,
        "runtime.session_file_max_mib",
        settings.upload_limits.session_file_max_mib,
    )?;
    settings.updated_at = updated_at;
    settings.validate()?;
    Ok(settings)
}

fn site_setting_definition<T: Serialize>(
    key: &'static str,
    value: &T,
    value_type: &'static str,
    group_name: &'static str,
    name: &'static str,
    description: &'static str,
    now: &str,
) -> SiteSettingDefinition {
    SiteSettingDefinition {
        key,
        value_json: setting_json(value).expect("site setting defaults serialize"),
        value_type,
        group_name,
        name,
        description,
        readonly: false,
        sensitive: false,
        created_at: now.to_string(),
        updated_at: now.to_string(),
    }
}

fn setting_json<T: Serialize>(value: &T) -> anyhow::Result<String> {
    serde_json::to_string(value).context("failed to serialize site setting value")
}

fn setting_value<T: DeserializeOwned>(
    values: &std::collections::HashMap<String, String>,
    key: &str,
    default: T,
) -> anyhow::Result<T> {
    let Some(value) = values.get(key) else {
        return Ok(default);
    };
    serde_json::from_str(value).with_context(|| format!("failed to parse site setting `{key}`"))
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn trim_or_default(value: &str, default: &str) -> String {
    let value = value.trim();
    if value.is_empty() {
        default.to_string()
    } else {
        value.to_string()
    }
}

#[async_trait]
pub trait UserRepository: Send + Sync {
    async fn create_user(&self, user: NewUser) -> anyhow::Result<User>;
    async fn list_users(&self) -> anyhow::Result<Vec<User>>;
    async fn list_users_with_status(&self, status: UserStatus) -> anyhow::Result<Vec<User>>;
    async fn find_user_by_id(&self, user_id: &str) -> anyhow::Result<Option<User>>;
    async fn find_user_by_username(&self, username: &str) -> anyhow::Result<Option<User>>;
    async fn update_user(
        &self,
        user_id: &str,
        display_name: String,
        email: String,
        profile: UserProfile,
        disabled: bool,
    ) -> anyhow::Result<Option<User>>;
    async fn mark_user_login(&self, user_id: &str, at: DateTime<Utc>) -> anyhow::Result<()>;
    async fn update_password_hash(
        &self,
        user_id: &str,
        password_hash: String,
    ) -> anyhow::Result<()>;
    async fn set_user_disabled(&self, user_id: &str, disabled: bool) -> anyhow::Result<()>;
    async fn set_user_status(&self, user_id: &str, status: UserStatus) -> anyhow::Result<()>;
    async fn user_count(&self) -> anyhow::Result<i64>;
}

#[async_trait]
pub trait AuthSessionRepository: Send + Sync {
    async fn create_auth_session(&self, session: AuthSession) -> anyhow::Result<()>;
    async fn find_auth_session_by_token_hash(
        &self,
        token_hash: &str,
    ) -> anyhow::Result<Option<AuthSession>>;
    async fn find_user_by_auth_token_hash(
        &self,
        token_hash: &str,
        now: DateTime<Utc>,
    ) -> anyhow::Result<Option<User>>;
    async fn delete_auth_session_by_token_hash(&self, token_hash: &str) -> anyhow::Result<()>;
    async fn delete_auth_sessions_for_user_except(
        &self,
        user_id: &str,
        token_hash: &str,
    ) -> anyhow::Result<()>;
    async fn delete_expired_auth_sessions(&self, now: DateTime<Utc>) -> anyhow::Result<()>;
}

#[async_trait]
pub trait LeaseRepository: Send + Sync {
    async fn create_lease(&self, lease: NewLease) -> anyhow::Result<Lease>;
    async fn list_leases(&self) -> anyhow::Result<Vec<Lease>>;
    async fn list_leases_for_user(&self, user_id: &str) -> anyhow::Result<Vec<Lease>>;
    async fn find_lease(&self, lease_id: &str) -> anyhow::Result<Option<Lease>>;
    async fn mark_lease_state(
        &self,
        lease_id: &str,
        state: LeaseState,
        released_at: Option<DateTime<Utc>>,
        failure_message: Option<String>,
    ) -> anyhow::Result<()>;
    async fn expire_leases_before(&self, now: DateTime<Utc>) -> anyhow::Result<u64>;
    async fn update_lease_expiry(
        &self,
        lease_id: &str,
        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<()>;
    async fn bind_lease_session(&self, lease_id: &str, session_id: &str) -> anyhow::Result<()>;
    async fn delete_lease(&self, lease_id: &str) -> anyhow::Result<()>;
    async fn update_lease(
        &self,
        lease_id: &str,
        starts_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        failure_message: Option<String>,
    ) -> anyhow::Result<Option<Lease>>;
}

#[async_trait]
pub trait SessionRecordRepository: Send + Sync {
    async fn create_session_record(
        &self,
        record: NewSessionRecord,
    ) -> anyhow::Result<SessionRecord>;
    async fn list_session_records(&self) -> anyhow::Result<Vec<SessionRecord>>;
    async fn update_session_record_runtime(
        &self,
        session_id: &str,
        state: String,
        last_heartbeat_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<()>;
    async fn finish_session_record(
        &self,
        session_id: &str,
        state: String,
        ended_at: DateTime<Utc>,
        failure_message: Option<String>,
    ) -> anyhow::Result<()>;
    async fn update_session_record(
        &self,
        session_id: &str,
        client_name: Option<String>,
        failure_message: Option<String>,
    ) -> anyhow::Result<Option<SessionRecord>>;
    async fn delete_session_record(&self, session_id: &str) -> anyhow::Result<()>;
}

#[async_trait]
pub trait IssueSessionRepository: Send + Sync {
    async fn create_issue_session(&self, issue: NewIssueSession) -> anyhow::Result<IssueSession>;
    async fn list_issue_sessions(&self) -> anyhow::Result<Vec<IssueSession>>;
    async fn list_issue_sessions_for_user(
        &self,
        user_id: &str,
    ) -> anyhow::Result<Vec<IssueSession>>;
    async fn find_issue_session(&self, issue_id: &str) -> anyhow::Result<Option<IssueSession>>;
    async fn update_issue_session(
        &self,
        issue_id: &str,
        state: IssueSessionState,
        priority: IssueSessionPriority,
        handler_user_id: Option<String>,
        resolution: Option<String>,
    ) -> anyhow::Result<Option<IssueSession>>;
    async fn delete_issue_session(&self, issue_id: &str) -> anyhow::Result<()>;
}

#[async_trait]
pub trait AnnouncementRepository: Send + Sync {
    async fn create_announcement(
        &self,
        announcement: NewAnnouncement,
    ) -> anyhow::Result<Announcement>;
    async fn list_announcements(&self) -> anyhow::Result<Vec<Announcement>>;
    async fn list_published_announcements(&self) -> anyhow::Result<Vec<Announcement>>;
    async fn find_announcement(
        &self,
        announcement_id: &str,
    ) -> anyhow::Result<Option<Announcement>>;
    async fn update_announcement(
        &self,
        announcement_id: &str,
        title: String,
        content: String,
        kind: AnnouncementKind,
        status: AnnouncementStatus,
        pinned: bool,
        updated_by: Option<String>,
    ) -> anyhow::Result<Option<Announcement>>;
    async fn delete_announcement(&self, announcement_id: &str) -> anyhow::Result<()>;
}

#[async_trait]
pub trait BoardConfigRepository: Send + Sync {
    async fn create_board_config(&self, board: BoardConfig) -> anyhow::Result<BoardConfig>;
    async fn list_board_configs(&self) -> anyhow::Result<Vec<BoardConfig>>;
    async fn update_board_config(
        &self,
        current_board_id: &str,
        board: BoardConfig,
    ) -> anyhow::Result<BoardConfig>;
    async fn delete_board_config(&self, board_id: &str) -> anyhow::Result<()>;
}

#[async_trait]
pub trait RbacRepository: Send + Sync {
    async fn list_permissions(&self) -> anyhow::Result<Vec<Permission>>;
    async fn list_roles(&self) -> anyhow::Result<Vec<Role>>;
    async fn find_role_by_id(&self, role_id: &str) -> anyhow::Result<Option<Role>>;
    async fn create_role(&self, role: NewRole) -> anyhow::Result<Role>;
    async fn update_role(
        &self,
        role_id: &str,
        display_name: String,
        description: String,
        permission_ids: Vec<String>,
    ) -> anyhow::Result<Option<Role>>;
    async fn set_role_disabled(
        &self,
        role_id: &str,
        disabled: bool,
    ) -> anyhow::Result<Option<Role>>;
    async fn delete_role(&self, role_id: &str) -> anyhow::Result<()>;
    async fn role_permissions(&self, role_id: &str) -> anyhow::Result<Vec<Permission>>;
    async fn role_user_counts(&self) -> anyhow::Result<BTreeMap<String, u64>>;
    async fn user_roles(&self, user_id: &str) -> anyhow::Result<Vec<Role>>;
    async fn set_user_roles(&self, user_id: &str, role_ids: Vec<String>) -> anyhow::Result<()>;
    async fn user_permissions(&self, user_id: &str) -> anyhow::Result<Vec<Permission>>;
}

#[async_trait]
pub trait DtbMetadataRepository: Send + Sync {
    async fn upsert_dtb_metadata(&self, metadata: UpsertDtbMetadata)
    -> anyhow::Result<DtbMetadata>;
    async fn list_dtb_metadata(&self) -> anyhow::Result<Vec<DtbMetadata>>;
    async fn find_dtb_metadata_by_name(&self, name: &str) -> anyhow::Result<Option<DtbMetadata>>;
    async fn rename_dtb_metadata(&self, current_name: &str, new_name: &str) -> anyhow::Result<()>;
    async fn delete_dtb_metadata_by_name(&self, name: &str) -> anyhow::Result<()>;
}

#[async_trait]
pub trait AuditLogRepository: Send + Sync {
    async fn create_audit_log(&self, log: NewAuditLog) -> anyhow::Result<AuditLog>;
    async fn list_audit_logs(&self, limit: i64) -> anyhow::Result<Vec<AuditLog>>;
}

#[async_trait]
pub trait SiteSettingsRepository: Send + Sync {
    async fn get_site_settings(&self) -> anyhow::Result<SiteSettings>;
    async fn update_site_settings(
        &self,
        settings: SiteSettings,
        updated_by: Option<String>,
    ) -> anyhow::Result<SiteSettings>;
}

#[async_trait]
pub trait RuntimeSettingsRepository: Send + Sync {
    async fn get_runtime_settings(&self) -> anyhow::Result<RuntimeSettings>;
    async fn update_runtime_settings(
        &self,
        settings: RuntimeSettings,
        updated_by: Option<String>,
    ) -> anyhow::Result<RuntimeSettings>;
}

pub trait Storage:
    UserRepository
    + AuthSessionRepository
    + LeaseRepository
    + SessionRecordRepository
    + IssueSessionRepository
    + AnnouncementRepository
    + BoardConfigRepository
    + RbacRepository
    + DtbMetadataRepository
    + AuditLogRepository
    + SiteSettingsRepository
    + RuntimeSettingsRepository
    + Send
    + Sync
    + 'static
{
}

impl<T> Storage for T where
    T: UserRepository
        + AuthSessionRepository
        + LeaseRepository
        + SessionRecordRepository
        + IssueSessionRepository
        + AnnouncementRepository
        + BoardConfigRepository
        + RbacRepository
        + DtbMetadataRepository
        + AuditLogRepository
        + SiteSettingsRepository
        + RuntimeSettingsRepository
        + Send
        + Sync
        + 'static
{
}

pub type DynStorage = Arc<dyn Storage>;

pub fn parse_time(value: &str) -> anyhow::Result<DateTime<Utc>> {
    Ok(DateTime::parse_from_rfc3339(value)
        .with_context(|| format!("failed to parse timestamp `{value}`"))?
        .with_timezone(&Utc))
}
