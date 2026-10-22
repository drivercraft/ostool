//! Site configuration and versioned legal consent. Public projections contain no secrets.
use super::{auth::CurrentUser, error::ApiError};
use crate::{
    AppState,
    security::{crypto, store::ClientContext},
};
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    routing::get,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct SiteSettings {
    pub name: String,
    pub short_name: String,
    pub tagline: String,
    pub description: String,
    pub logo_url: String,
    pub favicon_url: String,
    pub support_email: String,
    pub support_phone: String,
    #[serde(default)]
    pub support_work_hours: String,
    #[serde(default)]
    pub privacy_contact_email: String,
    pub support_url: String,
    pub operator: String,
    pub address: String,
    pub copyright: String,
    pub icp_number: String,
    pub icp_url: String,
    pub police_number: String,
    pub police_url: String,
    pub github_url: String,
    pub maintenance: bool,
    pub maintenance_message: String,
    pub terms_title: String,
    pub terms_body: String,
    pub privacy_title: String,
    pub privacy_body: String,
}
impl Default for SiteSettings {
    fn default() -> Self {
        Self {
        name:"ostool".into(),short_name:"ostool".into(),tagline:"BOARD PLATFORM".into(),
        description:"为嵌入式团队打造的开发板协作平台。".into(),
        logo_url:String::new(),favicon_url:String::new(),support_email:String::new(),
        support_phone:String::new(),support_work_hours:"工作日 09:00–18:00".into(),privacy_contact_email:String::new(),support_url:String::new(),operator:String::new(),address:String::new(),
        copyright:"ostool. Built for hardware teams.".into(),icp_number:String::new(),icp_url:String::new(),
        police_number:String::new(),police_url:String::new(),github_url:"https://github.com/drivercraft/ostool".into(),
        maintenance:false,maintenance_message:"系统正在维护，部分服务可能暂时不可用。".into(),
        terms_title:"用户协议".into(),
        terms_body:"本平台提供开发板访问、串口调试和租赁申请服务。\n请使用真实可联系的邮箱注册，妥善保管密码、验证器和访问令牌。不得共享他人的凭据或访问未获分配的开发板。\n使用开发板时应遵守运营者公布的使用规则与租赁时段，不得执行破坏设备、攻击网络或侵害他人权益的操作。\n维护、网络和硬件故障可能影响使用。遇到故障或账号问题，请通过站点公布的联系方式联系运营者。".into(),
        privacy_title:"隐私政策".into(),
        privacy_body:"本平台为账号管理保存用户名、邮箱、显示名称和密码摘要，不保存明文密码。\n为验证登录和排查异常，平台处理登录时间、来源 IP、浏览器标识、安全事件及操作审计。租赁记录用于开发板分配与使用管理。\n人机验证、邮箱验证和双重验证所需的证明会设置有效期；验证码不以明文保存，验证器密钥加密保存。浏览器会使用必要的登录 Cookie 和界面偏好存储。\n数据由本站运营者管理。请通过站点公布的联系方式咨询数据访问、更正及删除事宜。".into(),
    }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SiteView {
    pub revision: i64,
    pub terms_version: i64,
    pub privacy_version: i64,
    pub terms_published_at: chrono::DateTime<chrono::Utc>,
    pub privacy_published_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
    pub settings: SiteSettings,
    pub auth: Value,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteUpdate {
    pub revision: i64,
    pub settings: SiteSettings,
}
#[derive(Debug, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct LegalConsent {
    pub accepted: bool,
    pub terms_version: i64,
    pub privacy_version: i64,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/site", get(public))
        .route("/api/v1/site/devices", get(devices))
        .route(
            "/api/v1/site/events",
            get(crate::admin_events::subscribe_site),
        )
        .route("/api/v1/site/legal/{kind}/{version}", get(document))
        .route("/api/v1/admin/site-settings", get(public).put(update))
        .route("/api/v1/admin/legal-history", get(history))
        .route(
            "/api/v1/admin/account-settings",
            get(account_settings).put(update_account_settings),
        )
        .route("/api/v1/admin/mail-deliveries", get(mail_deliveries))
}
pub async fn initialize(database: &crate::persistence::Database) -> anyhow::Result<()> {
    let settings = SiteSettings::default();
    let mut tx = database.pool().begin().await?;
    sqlx::query("INSERT INTO site_settings(id,content) VALUES(1,$1) ON CONFLICT DO NOTHING")
        .bind(serde_json::to_value(&settings)?)
        .execute(&mut *tx)
        .await?;
    for (kind, title, body) in [
        ("terms", &settings.terms_title, &settings.terms_body),
        ("privacy", &settings.privacy_title, &settings.privacy_body),
    ] {
        sqlx::query("INSERT INTO legal_document_versions(kind,version,title,body,content_hash) VALUES($1,1,$2,$3,$4) ON CONFLICT DO NOTHING")
            .bind(kind).bind(title).bind(body).bind(crypto::hash(&format!("{title}\n{body}"))).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}
pub async fn view(state: &AppState) -> Result<SiteView, ApiError> {
    let row=sqlx::query("SELECT s.*, (SELECT max(version) FROM legal_document_versions WHERE kind='terms') AS terms_version, (SELECT max(version) FROM legal_document_versions WHERE kind='privacy') AS privacy_version, (SELECT max(published_at) FROM legal_document_versions WHERE kind='terms') AS terms_published_at, (SELECT max(published_at) FROM legal_document_versions WHERE kind='privacy') AS privacy_published_at FROM site_settings s WHERE id=1")
        .fetch_one(state.database.pool()).await?;
    Ok(SiteView {
        revision: row.try_get("revision")?,
        terms_version: row.try_get("terms_version")?,
        privacy_version: row.try_get("privacy_version")?,
        terms_published_at: row.try_get("terms_published_at")?,
        privacy_published_at: row.try_get("privacy_published_at")?,
        updated_at: row.try_get("updated_at")?,
        settings: serde_json::from_value(row.try_get("content")?).map_err(anyhow::Error::from)?,
        auth: public_auth(&*state.config.read().await),
    })
}
async fn public(State(state): State<AppState>) -> Result<Json<SiteView>, ApiError> {
    Ok(Json(view(&state).await?))
}

pub(crate) async fn device_catalog(state: &AppState) -> Vec<super::models::PublicDevice> {
    state
        .boards
        .read()
        .await
        .values()
        .map(super::models::PublicDevice::from)
        .collect()
}

async fn devices(State(state): State<AppState>) -> Json<Vec<super::models::PublicDevice>> {
    Json(device_catalog(&state).await)
}
fn validate(s: &SiteSettings) -> Result<(), ApiError> {
    for (text, max) in [
        (&s.name, 80),
        (&s.short_name, 20),
        (&s.tagline, 80),
        (&s.description, 500),
        (&s.support_phone, 40),
        (&s.support_work_hours, 80),
        (&s.operator, 120),
        (&s.address, 300),
        (&s.copyright, 200),
        (&s.icp_number, 64),
        (&s.police_number, 64),
        (&s.maintenance_message, 500),
        (&s.terms_title, 80),
        (&s.privacy_title, 80),
    ] {
        if text.chars().count() > max || text.contains(char::from(0)) {
            return Err(ApiError::bad_request(format!(
                "配置文本不得超过 {max} 个字符或包含非法字符"
            )));
        }
    }
    if s.terms_title.trim().is_empty() || s.privacy_title.trim().is_empty() {
        return Err(ApiError::bad_request("协议和隐私政策标题不能为空"));
    }
    if s.maintenance && s.maintenance_message.trim().is_empty() {
        return Err(ApiError::bad_request("开启维护提示时请填写提示内容"));
    }
    if s.name.trim().is_empty() || s.short_name.trim().is_empty() {
        return Err(ApiError::bad_request("站点名称和简称不能为空"));
    }
    for body in [&s.terms_body, &s.privacy_body] {
        if body.trim().is_empty() || body.len() > 100_000 || body.contains('\0') {
            return Err(ApiError::bad_request("协议正文不能为空，且不超过 100 KB"));
        }
    }
    for value in [&s.logo_url, &s.favicon_url] {
        if !value.is_empty()
            && (!value.starts_with('/')
                || value.starts_with("//")
                || value.contains(['\\', '\r', '\n'])
                || value.len() > 1000)
        {
            return Err(ApiError::bad_request("品牌图片请使用同源的绝对路径"));
        }
    }
    for value in [&s.support_url, &s.icp_url, &s.police_url, &s.github_url] {
        if !value.is_empty() {
            let url = url::Url::parse(value)
                .map_err(|_| ApiError::bad_request("链接必须是 HTTP(S) 地址"))?;
            if !matches!(url.scheme(), "http" | "https")
                || !url.username().is_empty()
                || url.password().is_some()
                || value.len() > 1000
            {
                return Err(ApiError::bad_request("链接必须是无凭据的 HTTP(S) 地址"));
            }
        }
    }
    for email in [&s.support_email, &s.privacy_contact_email] {
        if !email.is_empty() {
            crate::persistence::postgres::normalize_email(email)
                .map_err(|_| ApiError::bad_request("邮箱格式无效"))?;
        }
    }
    Ok(())
}
async fn update(
    State(state): State<AppState>,
    actor: Option<Extension<CurrentUser>>,
    Json(input): Json<SiteUpdate>,
) -> Result<Json<Value>, ApiError> {
    validate(&input.settings)?;
    let actor = actor.map(|Extension(CurrentUser(user))| user.id);
    let mut tx = state.database.pool().begin().await?;
    let row = sqlx::query("SELECT revision,content FROM site_settings WHERE id=1 FOR UPDATE")
        .fetch_one(&mut *tx)
        .await?;
    if input.revision != row.try_get::<i64, _>("revision")? {
        return Err(ApiError::conflict(
            "配置已被其他管理员修改，请载入最新版本后重试",
        ));
    }
    let old: SiteSettings =
        serde_json::from_value(row.try_get("content")?).map_err(anyhow::Error::from)?;
    for (kind, title, body, old_title, old_body) in [
        (
            "terms",
            &input.settings.terms_title,
            &input.settings.terms_body,
            &old.terms_title,
            &old.terms_body,
        ),
        (
            "privacy",
            &input.settings.privacy_title,
            &input.settings.privacy_body,
            &old.privacy_title,
            &old.privacy_body,
        ),
    ] {
        if title != old_title || body != old_body {
            sqlx::query("INSERT INTO legal_document_versions(kind,version,title,body,content_hash,published_by) SELECT $1,max(version)+1,$2,$3,$4,$5 FROM legal_document_versions WHERE kind=$1")
                .bind(kind).bind(title).bind(body).bind(crypto::hash(&format!("{title}\n{body}"))).bind(actor).execute(&mut *tx).await?;
        }
    }
    sqlx::query("UPDATE site_settings SET revision=revision+1,content=$1,updated_at=now(),updated_by=$2 WHERE id=1")
        .bind(serde_json::to_value(input.settings).map_err(anyhow::Error::from)?).bind(actor).execute(&mut *tx).await?;
    tx.commit().await?;
    state.admin_events.invalidate(&["site", "legal_history"]);
    Ok(Json(json!({"saved":true,"revision":input.revision+1})))
}
async fn document(
    State(state): State<AppState>,
    Path((kind, version)): Path<(String, i64)>,
) -> Result<Json<Value>, ApiError> {
    let row=sqlx::query("SELECT kind,version,title,body,content_hash,published_at FROM legal_document_versions WHERE kind=$1 AND version=$2")
        .bind(kind).bind(version).fetch_optional(state.database.pool()).await?.ok_or_else(||ApiError::not_found("文档版本不存在"))?;
    Ok(Json(
        json!({"kind":row.try_get::<String,_>("kind")?,"version":row.try_get::<i64,_>("version")?,
        "title":row.try_get::<String,_>("title")?,"body":row.try_get::<String,_>("body")?,
        "content_hash":row.try_get::<String,_>("content_hash")?,"published_at":row.try_get::<chrono::DateTime<chrono::Utc>,_>("published_at")?}),
    ))
}
pub async fn history(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows=sqlx::query("SELECT kind,version,title,content_hash,published_at FROM (SELECT *,row_number() OVER(PARTITION BY kind ORDER BY version DESC) AS position FROM legal_document_versions) versions WHERE position<=100 ORDER BY published_at DESC,kind")
        .fetch_all(state.database.pool()).await?;
    Ok(Json(Value::Array(rows.into_iter().map(|r|Ok(json!({"kind":r.try_get::<String,_>("kind")?,"version":r.try_get::<i64,_>("version")?,"title":r.try_get::<String,_>("title")?,"content_hash":r.try_get::<String,_>("content_hash")?,"published_at":r.try_get::<chrono::DateTime<chrono::Utc>,_>("published_at")?}))).collect::<Result<Vec<_>,sqlx::Error>>()?)))
}
pub async fn mail_deliveries(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let rows=sqlx::query("SELECT id,purpose,recipient_email,status,error_code,created_at,completed_at FROM mail_deliveries ORDER BY created_at DESC,id LIMIT 100")
        .fetch_all(state.database.pool()).await?;
    Ok(Json(Value::Array(rows.into_iter().map(|r|Ok(json!({"id":r.try_get::<Uuid,_>("id")?,"purpose":r.try_get::<String,_>("purpose")?,"recipient_email":r.try_get::<String,_>("recipient_email")?,"status":r.try_get::<String,_>("status")?,"error_code":r.try_get::<Option<String>,_>("error_code")?,"created_at":r.try_get::<chrono::DateTime<chrono::Utc>,_>("created_at")?,"completed_at":r.try_get::<Option<chrono::DateTime<chrono::Utc>>,_>("completed_at")?}))).collect::<Result<Vec<_>,sqlx::Error>>()?)))
}
pub async fn register(
    state: &AppState,
    request: &super::auth::RegisterRequest,
    client: &ClientContext,
    email: &str,
    verified: bool,
) -> Result<crate::persistence::postgres::User, ApiError> {
    let consent = &request.legal;
    if !consent.accepted {
        return Err(ApiError::bad_request("请阅读并同意用户协议和隐私政策"));
    }
    let username = crate::persistence::postgres::normalize_username(&request.username)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let hash = crate::persistence::postgres::hash_password_async(request.password.clone()).await?;
    let mut tx = state.database.pool().begin().await?;
    sqlx::query("SELECT id FROM site_settings WHERE id=1 FOR SHARE")
        .fetch_one(&mut *tx)
        .await?;
    for (kind, version) in [
        ("terms", consent.terms_version),
        ("privacy", consent.privacy_version),
    ] {
        let latest: i64 =
            sqlx::query_scalar("SELECT max(version) FROM legal_document_versions WHERE kind=$1")
                .bind(kind)
                .fetch_one(&mut *tx)
                .await?;
        if latest != version {
            return Err(ApiError::new(
                axum::http::StatusCode::CONFLICT,
                "legal_changed",
                "协议已更新，请重新阅读并同意后注册",
            ));
        }
    }
    let user:Uuid=sqlx::query_scalar("INSERT INTO users(email,password_hash,display_name,email_verified_at,username) VALUES($1,$2,$3,CASE WHEN $4 THEN now() ELSE NULL END,$5) RETURNING id")
        .bind(email).bind(hash).bind(request.display_name.as_deref()).bind(verified).bind(username).fetch_one(&mut *tx).await.map_err(|error| {
            if error.as_database_error().is_some_and(|error| error.is_unique_violation()) {
                ApiError::conflict("用户名或邮箱已被使用")
            } else {
                log::error!("registration failed: {error}");
                ApiError::service_unavailable("注册服务暂时不可用")
            }
        })?;
    for (kind, version) in [
        ("terms", consent.terms_version),
        ("privacy", consent.privacy_version),
    ] {
        sqlx::query("INSERT INTO user_legal_acceptances(user_id,kind,version,ip_hash,user_agent_hash) VALUES($1,$2,$3,$4,$5)")
            .bind(user).bind(kind).bind(version).bind(crypto::hash(&client.ip)).bind(crypto::hash(&client.user_agent)).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    state
        .database
        .user_by_id(user)
        .await?
        .ok_or_else(|| ApiError::unauthorized("账号状态已变化"))
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AccountSettingsRecord {
    pub revision: i64,
    pub settings: crate::security::config::AccountSettings,
}
pub async fn load_account_settings(
    database: &crate::persistence::Database,
    config: &mut crate::ServerConfig,
) -> anyhow::Result<()> {
    if let Some(stored) = database
        .record::<AccountSettingsRecord>("account-settings")
        .await?
    {
        return stored.settings.apply(config);
    }
    let initial = AccountSettingsRecord {
        revision: 1,
        settings: crate::security::config::AccountSettings::from_config(config),
    };
    initial.settings.apply(config)?;
    let stored = database
        .initialize_record("account-settings", &initial)
        .await?;
    stored.settings.apply(config)
}
pub fn public_auth(config: &crate::ServerConfig) -> Value {
    let public = config.edition.is_public();
    let email = public && config.security.smtp_available();
    json!({"allow_registration":public && config.auth.allow_registration,
        "registration_email_verification":public && config.security.registration_email_verification,
        "email_security_available":email,
        "password_reset_available":email && config.security.password_reset_enabled})
}
pub async fn account_settings(State(state): State<AppState>) -> Result<Json<Value>, ApiError> {
    let record: AccountSettingsRecord = state
        .database
        .record("account-settings")
        .await?
        .ok_or_else(|| ApiError::not_found("本地版不提供账号策略配置"))?;
    let config = state.config.read().await;
    if !config.edition.is_public() {
        return Err(ApiError::not_found("本地版不提供账号策略配置"));
    }
    Ok(Json(
        json!({"revision":record.revision,"settings":crate::security::config::AccountSettings::from_config(&config),
        "secrets":{"smtp_password":config.security.smtp_password.is_some(),"turnstile_secret":config.security.turnstile_secret.is_some(),"mfa_key":config.security.mfa_encryption_key.is_some()}}),
    ))
}
async fn update_account_settings(
    State(state): State<AppState>,
    Json(input): Json<AccountSettingsRecord>,
) -> Result<Json<Value>, ApiError> {
    let mut config = state.config.write().await;
    if !config.edition.is_public() {
        return Err(ApiError::not_found("本地版不提供账号策略配置"));
    }
    let mut next = config.clone();
    input
        .settings
        .apply(&mut next)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let mut tx = state.database.pool().begin().await?;
    let payload: Value = sqlx::query_scalar(
        "SELECT payload FROM server_records WHERE name='account-settings' FOR UPDATE",
    )
    .fetch_one(&mut *tx)
    .await?;
    let current: AccountSettingsRecord =
        serde_json::from_value(payload).map_err(anyhow::Error::from)?;
    if current.revision != input.revision {
        return Err(ApiError::conflict("配置已更新，请保留草稿后载入最新版本"));
    }
    let saved = AccountSettingsRecord {
        revision: current.revision + 1,
        settings: input.settings,
    };
    sqlx::query(
        "UPDATE server_records SET payload=$1,updated_at=now() WHERE name='account-settings'",
    )
    .bind(serde_json::to_value(&saved).map_err(anyhow::Error::from)?)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    *config = next;
    drop(config);
    state
        .admin_events
        .invalidate(&["account_settings", "site", "deployment"]);
    Ok(Json(json!({"saved":true,"revision":saved.revision})))
}
