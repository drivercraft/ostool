pub mod mysql;
pub mod sqlite;

use std::sync::Arc;

use anyhow::Context;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::config::BoardConfig;

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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub id: String,
    pub name: String,
    pub display_name: String,
    pub description: String,
    pub system: bool,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SiteSettings {
    pub site_name: String,
    pub site_subtitle: String,
    pub logo_url: Option<String>,
    pub favicon_url: Option<String>,
    pub announcement: Option<String>,
    pub maintenance_mode: bool,
    pub self_service_enabled: bool,
    pub default_lease_minutes: i64,
    pub max_lease_minutes: i64,
    pub support_email: Option<String>,
    pub support_url: Option<String>,
    pub updated_at: DateTime<Utc>,
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
            "是否允许普通用户自助创建开发板租赁",
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
    async fn update_lease_expiry(
        &self,
        lease_id: &str,
        expires_at: DateTime<Utc>,
    ) -> anyhow::Result<()>;
    async fn bind_lease_session(&self, lease_id: &str, session_id: &str) -> anyhow::Result<()>;
    async fn update_lease(
        &self,
        lease_id: &str,
        starts_at: DateTime<Utc>,
        expires_at: DateTime<Utc>,
        failure_message: Option<String>,
    ) -> anyhow::Result<Option<Lease>>;
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
    async fn delete_role(&self, role_id: &str) -> anyhow::Result<()>;
    async fn role_permissions(&self, role_id: &str) -> anyhow::Result<Vec<Permission>>;
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

pub trait Storage:
    UserRepository
    + AuthSessionRepository
    + LeaseRepository
    + BoardConfigRepository
    + RbacRepository
    + DtbMetadataRepository
    + AuditLogRepository
    + SiteSettingsRepository
    + Send
    + Sync
    + 'static
{
}

impl<T> Storage for T where
    T: UserRepository
        + AuthSessionRepository
        + LeaseRepository
        + BoardConfigRepository
        + RbacRepository
        + DtbMetadataRepository
        + AuditLogRepository
        + SiteSettingsRepository
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
