//! Administrator user and role management. Read projections share admin SSE.
use super::{auth::CurrentUser, error::ApiError};
use crate::AppState;
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::{get, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Role {
    pub revision: i64,
    pub id: String,
    pub code: String,
    pub enabled: bool,
    pub name: String,
    pub description: String,
    pub administrator: bool,
    pub permissions: Vec<String>,
    pub allocation_policy: crate::security::permissions::AllocationPolicy,
    pub builtin: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub user_count: i64,
}
#[derive(Deserialize)]
pub struct UserInput {
    pub username: String,
    pub email: String,
    pub display_name: Option<String>,
    pub role_id: String,
    pub disabled: bool,
    pub password: Option<String>,
}
#[derive(Deserialize)]
pub struct RoleInput {
    #[serde(default)]
    pub revision: Option<i64>,
    pub code: String,
    pub enabled: bool,
    pub name: String,
    pub description: String,
    pub permissions: Vec<String>,
    #[serde(default)]
    pub allocation_policy: crate::security::permissions::AllocationPolicy,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/users", get(list_users).post(create_user))
        .route(
            "/api/v1/admin/users/{id}",
            put(update_user).delete(delete_user),
        )
        .route("/api/v1/admin/roles", get(list_roles).post(create_role))
        .route(
            "/api/v1/admin/roles/{id}",
            put(update_role).delete(delete_role),
        )
        .route("/api/v1/admin/deployment", get(deployment))
        .route(
            "/api/v1/admin/permissions",
            get(crate::security::permissions::catalog),
        )
}

fn sql_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation() || e.is_foreign_key_violation())
    {
        ApiError::conflict("用户名、邮箱、角色编码或名称已存在，或该角色仍在使用")
    } else {
        log::error!("user management database error: {error}");
        ApiError::service_unavailable("用户数据库操作失败")
    }
}

pub async fn list_users(
    State(state): State<AppState>,
    axum::extract::Query(mut filters): axum::extract::Query<super::listing::Filters>,
) -> Result<Json<serde_json::Value>, ApiError> {
    filters.view = "users".into();
    super::listing::page(&state, &filters).await.map(Json)
}
pub async fn list_roles(State(state): State<AppState>) -> Result<Json<Vec<Role>>, ApiError> {
    let db = &state.database;
    let rows = sqlx::query("SELECT r.*,(SELECT count(*) FROM users u WHERE u.role_id=r.id) AS user_count FROM roles r ORDER BY builtin DESC,id")
        .fetch_all(db.pool())
        .await
        .map_err(sql_error)?;
    rows.iter()
        .map(|r| {
            Ok(Role {
                revision: r.try_get("revision")?,
                id: r.try_get("id")?,
                code: r.try_get("code")?,
                enabled: r.try_get("enabled")?,
                name: r.try_get("name")?,
                description: r.try_get("description")?,
                administrator: r.try_get("administrator")?,
                permissions: r.try_get("permissions")?,
                allocation_policy: serde_json::from_value(r.try_get("allocation_policy")?)
                    .map_err(|e| sqlx::Error::Decode(Box::new(e)))?,
                builtin: r.try_get("builtin")?,
                user_count: r.try_get("user_count")?,
                created_at: r.try_get("created_at")?,
                updated_at: r.try_get("updated_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map(Json)
        .map_err(sql_error)
}
fn validate(input: &UserInput) -> Result<(), ApiError> {
    crate::persistence::postgres::normalize_username(&input.username)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    crate::persistence::postgres::normalize_email(&input.email)
        .map_err(|_| ApiError::bad_request("邮箱格式无效"))?;
    crate::security::names::normalize_display_name(input.display_name.as_deref())?;
    if let Some(password) = input.password.as_deref() {
        crate::security::crypto::validate_password(password)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
    }
    Ok(())
}
async fn create_user(
    State(state): State<AppState>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
    Json(input): Json<UserInput>,
) -> Result<(StatusCode, axum::extract::Extension<crate::audit::Target>), ApiError> {
    validate(&input)?;
    state
        .config
        .read()
        .await
        .security
        .name_policy
        .check_change(&input.username, input.display_name.as_deref(), None)?;
    let display_name =
        crate::security::names::normalize_display_name(input.display_name.as_deref())?;
    let password = input
        .password
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("新用户需要初始密码"))?;
    let hash = crate::persistence::postgres::hash_password_async(password.to_owned()).await?;
    let pool = state.database.pool();
    ensure_assignable(&state, &actor, &input.role_id, None).await?;
    let result = sqlx::query_scalar::<_,Uuid>("INSERT INTO users(email,display_name,password_hash,role_id,is_admin,disabled,username) SELECT $1,$2,$3,id,administrator,$5,$6 FROM roles WHERE id=$4 AND enabled RETURNING users.id")
        .bind(input.email.trim().to_lowercase()).bind(display_name).bind(hash).bind(input.role_id).bind(input.disabled).bind(input.username.trim().to_ascii_lowercase()).fetch_optional(pool).await.map_err(sql_error)?;
    let id = result.ok_or_else(|| ApiError::bad_request("角色不存在"))?;
    state.admin_events.invalidate(&["users"]);
    Ok((
        StatusCode::CREATED,
        axum::extract::Extension(crate::audit::Target(id.to_string())),
    ))
}
async fn update_user(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
    Json(input): Json<UserInput>,
) -> Result<StatusCode, ApiError> {
    validate(&input)?;
    ensure_assignable(&state, &actor, &input.role_id, Some(id)).await?;
    let hash = match input.password {
        Some(ref p) => Some(crate::persistence::postgres::hash_password_async(p.clone()).await?),
        None => None,
    };
    let mut tx = state.database.pool().begin().await.map_err(sql_error)?;
    // Serializes changes to administrators, including concurrent demotions.
    sqlx::query("LOCK TABLE users IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await
        .map_err(sql_error)?;
    let previous = sqlx::query("SELECT username,display_name FROM users WHERE id=$1")
        .bind(id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| ApiError::not_found("用户不存在"))?;
    let previous_username: String = previous.try_get("username").map_err(sql_error)?;
    let previous_name: Option<String> = previous.try_get("display_name").map_err(sql_error)?;
    state
        .config
        .read()
        .await
        .security
        .name_policy
        .check_change(
            &input.username,
            input.display_name.as_deref(),
            Some((&previous_username, previous_name.as_deref())),
        )?;
    let display_name =
        crate::security::names::normalize_display_name(input.display_name.as_deref())?;
    let role = sqlx::query("SELECT administrator FROM roles WHERE id=$1 AND enabled")
        .bind(&input.role_id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| ApiError::bad_request("角色不存在"))?;
    let administrator: bool = role.try_get("administrator").map_err(sql_error)?;
    if id == actor.id && (input.disabled || !administrator) {
        return Err(ApiError::conflict("不能禁用自己或移除自己的管理权限"));
    }
    let result=sqlx::query("UPDATE users SET email=$1,display_name=$2,role_id=$3,is_admin=$4,disabled=$5,password_hash=COALESCE($6,password_hash),username=$8 WHERE id=$7")
        .bind(input.email.trim().to_lowercase()).bind(display_name).bind(input.role_id).bind(administrator).bind(input.disabled).bind(hash.as_deref()).bind(id).bind(input.username.trim().to_ascii_lowercase()).execute(&mut *tx).await.map_err(sql_error)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::not_found("用户不存在"));
    }
    ensure_admin(&mut tx).await?;
    // The database trigger revokes credentials for identity, role, status and
    // password changes. Ordinary display-name edits keep valid sessions alive.
    tx.commit().await.map_err(sql_error)?;
    state.admin_events.invalidate(&["users"]);
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_user(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
) -> Result<StatusCode, ApiError> {
    if id == actor.id {
        return Err(ApiError::conflict("不能删除当前账号"));
    }
    ensure_assignable(&state, &actor, "member", Some(id)).await?;
    let mut tx = state.database.pool().begin().await.map_err(sql_error)?;
    sqlx::query("LOCK TABLE users IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await
        .map_err(sql_error)?;
    let result = sqlx::query("DELETE FROM users WHERE id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(sql_error)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::not_found("用户不存在"));
    }
    ensure_admin(&mut tx).await?;
    tx.commit().await.map_err(sql_error)?;
    state.admin_events.invalidate(&["users"]);
    Ok(StatusCode::NO_CONTENT)
}
async fn ensure_assignable(
    state: &AppState,
    actor: &crate::persistence::postgres::User,
    role: &str,
    target: Option<Uuid>,
) -> Result<(), ApiError> {
    let enabled: Option<bool> = sqlx::query_scalar("SELECT enabled FROM roles WHERE id=$1")
        .bind(role)
        .fetch_optional(state.database.pool())
        .await?;
    match enabled {
        Some(true) => {}
        Some(false) => return Err(ApiError::conflict("不能分配已停用的角色")),
        None => return Err(ApiError::bad_request("角色不存在")),
    }
    let access = crate::security::permissions::access(&state.database, actor).await?;
    if access.allows("system.admin") {
        return Ok(());
    }
    let row = sqlx::query("SELECT permissions,allocation_policy FROM roles WHERE id=$1")
        .bind(role)
        .fetch_optional(state.database.pool())
        .await?
        .ok_or_else(|| ApiError::bad_request("角色不存在"))?;
    let permissions: Vec<String> = row.try_get("permissions")?;
    let policy =
        serde_json::from_value(row.try_get("allocation_policy")?).map_err(anyhow::Error::from)?;
    crate::security::permissions::ensure_delegable(&access, &permissions, &policy)?;
    if let Some(target) = target {
        let other=sqlx::query("SELECT r.permissions,r.allocation_policy,u.is_admin FROM users u JOIN roles r ON r.id=u.role_id WHERE u.id=$1")
            .bind(target).fetch_optional(state.database.pool()).await?.ok_or_else(||ApiError::not_found("用户不存在"))?;
        let permissions: Vec<String> = other.try_get("permissions")?;
        let policy = serde_json::from_value(other.try_get("allocation_policy")?)
            .map_err(anyhow::Error::from)?;
        crate::security::permissions::ensure_delegable(&access, &permissions, &policy)?;
        if other.try_get::<bool, _>("is_admin")? || permissions.iter().any(|p| !access.allows(p)) {
            return Err(ApiError::forbidden("不能维护权限高于自己的用户"));
        }
    }
    Ok(())
}
async fn ensure_admin(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), ApiError> {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users u JOIN roles r ON r.id=u.role_id WHERE u.is_admin AND NOT u.disabled AND r.enabled")
            .fetch_one(&mut **tx)
            .await
            .map_err(sql_error)?;
    if count == 0 {
        return Err(ApiError::conflict("至少需要保留一位启用的管理员"));
    }
    Ok(())
}
fn validate_role(input: &RoleInput) -> Result<(), ApiError> {
    let code = input.code.as_bytes();
    if !(3..=64).contains(&code.len())
        || !code[0].is_ascii_lowercase()
        || !code
            .iter()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'_' | b'-'))
    {
        return Err(ApiError::bad_request(
            "角色编码为 3–64 位小写字母、数字、下划线或连字符，以字母开头",
        ));
    }
    if input.name.trim().is_empty() || input.name.len() > 80 || input.description.len() > 500 {
        return Err(ApiError::bad_request(
            "角色名称需要 1 至 80 个字符，描述不超过 500 个字符",
        ));
    }
    Ok(())
}
async fn create_role(
    State(state): State<AppState>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
    Json(mut input): Json<RoleInput>,
) -> Result<(StatusCode, Extension<crate::audit::Target>), ApiError> {
    validate_role(&input)?;
    input.allocation_policy = input.allocation_policy.normalized()?;
    let permissions = crate::security::permissions::expand(&input.permissions)?;
    let actor_access = crate::security::permissions::access(&state.database, &actor).await?;
    crate::security::permissions::ensure_delegable(
        &actor_access,
        &permissions,
        &input.allocation_policy,
    )?;
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO roles(id,name,description,administrator,permissions,allocation_policy,code,enabled) VALUES($1,$2,$3,$4,$5,$6,$7,$8)")
        .bind(&id).bind(input.name.trim()).bind(input.description).bind(permissions.iter().any(|p|p=="system.admin"))
        .bind(permissions).bind(serde_json::to_value(input.allocation_policy).map_err(anyhow::Error::from)?)
        .bind(input.code).bind(input.enabled)
        .execute(state.database.pool()).await.map_err(sql_error)?;
    state.admin_events.invalidate(&["roles"]);
    Ok((StatusCode::CREATED, Extension(crate::audit::Target(id))))
}
async fn update_role(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
    Json(mut input): Json<RoleInput>,
) -> Result<StatusCode, ApiError> {
    validate_role(&input)?;
    input.allocation_policy = input.allocation_policy.normalized()?;
    let permissions = crate::security::permissions::expand(&input.permissions)?;
    let access = crate::security::permissions::access(&state.database, &actor).await?;
    crate::security::permissions::ensure_delegable(
        &access,
        &permissions,
        &input.allocation_policy,
    )?;
    let _gate = state.rental_gate.lock().await;
    let mut tx = state.database.pool().begin().await?;
    sqlx::query("LOCK TABLE users IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await?;
    let row = sqlx::query(
        "SELECT code,enabled,builtin,permissions,allocation_policy,revision FROM roles WHERE id=$1 FOR UPDATE",
    )
    .bind(&id)
    .fetch_optional(&mut *tx)
    .await?
    .ok_or_else(|| ApiError::not_found("角色不存在"))?;
    if input.revision != Some(row.try_get::<i64, _>("revision")?) {
        return Err(ApiError::conflict(
            "角色已被其他管理员修改，请保留草稿并载入最新版本",
        ));
    }
    if input.code != row.try_get::<String, _>("code")? {
        return Err(ApiError::conflict("角色编码创建后不可修改"));
    }
    let old: Vec<String> = row.try_get("permissions")?;
    let policy: serde_json::Value = row.try_get("allocation_policy")?;
    let previous_policy = serde_json::from_value(policy.clone()).map_err(anyhow::Error::from)?;
    crate::security::permissions::ensure_delegable(&access, &old, &previous_policy)?;
    let new_policy = serde_json::to_value(&input.allocation_policy).map_err(anyhow::Error::from)?;
    let changed = old != permissions
        || policy != new_policy
        || input.enabled != row.try_get::<bool, _>("enabled")?;
    if changed && row.try_get::<bool, _>("builtin")? {
        return Err(ApiError::conflict(
            "内置角色的状态、权限和使用限制不可修改，请创建自定义角色",
        ));
    }
    if !access.allows("system.admin") && old.iter().any(|p| !access.allows(p)) {
        return Err(ApiError::forbidden("不能编辑权限高于自己的角色"));
    }
    let own_role: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=$1 AND role_id=$2)")
            .bind(actor.id)
            .bind(&id)
            .fetch_one(&mut *tx)
            .await?;
    if changed && own_role {
        return Err(ApiError::conflict(
            "不能修改自己正在使用的角色状态或权限，请使用其他管理员操作",
        ));
    }
    sqlx::query(
        "UPDATE roles SET name=$1,description=$2,permissions=$3,allocation_policy=$4,enabled=$6 WHERE id=$5",
    )
    .bind(input.name.trim())
    .bind(input.description)
    .bind(permissions)
    .bind(new_policy)
    .bind(&id)
    .bind(input.enabled)
    .execute(&mut *tx)
    .await?;
    ensure_admin(&mut tx).await?;
    tx.commit().await?;
    super::rentals::enforce(&state).await?;
    state
        .admin_events
        .invalidate(&["roles", "users", "rentals", "security_revision"]);
    state.rental_changed.notify_one();
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_role(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
) -> Result<StatusCode, ApiError> {
    let actor_access = crate::security::permissions::access(&state.database, &actor).await?;
    let row = sqlx::query("SELECT permissions,allocation_policy FROM roles WHERE id=$1")
        .bind(&id)
        .fetch_optional(state.database.pool())
        .await?
        .ok_or_else(|| ApiError::not_found("角色不存在"))?;
    let permissions: Vec<String> = row.try_get("permissions")?;
    let policy =
        serde_json::from_value(row.try_get("allocation_policy")?).map_err(anyhow::Error::from)?;
    crate::security::permissions::ensure_delegable(&actor_access, &permissions, &policy)?;
    let result = sqlx::query("DELETE FROM roles WHERE id=$1 AND NOT builtin")
        .bind(id)
        .execute(state.database.pool())
        .await
        .map_err(sql_error)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::conflict("内置角色不能删除，或角色不存在"));
    }
    state.admin_events.invalidate(&["roles"]);
    Ok(StatusCode::NO_CONTENT)
}
pub async fn deployment(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.read().await;
    let public = config.edition.is_public();
    let redis_connected = match &state.security_cache {
        Some(cache) => cache.ping().await.is_ok(),
        None => false,
    };
    let security = if public {
        serde_json::json!({
            "图形验证方式":config.security.captcha_provider,
            "注册要求邮箱验证":config.security.registration_email_verification,
            "登录失败阈值":config.security.login_max_failures,
            "登录锁定秒数":config.security.login_lock_secs,
            "管理员强制双重验证":config.security.require_admin_mfa,
            "SMTP 密码已配置":config.security.smtp_password.is_some(),
            "双重验证加密密钥已配置":config.security.mfa_encryption_key.is_some()
        })
    } else {
        serde_json::json!({})
    };
    let persistent = "PostgreSQL";
    let mut storage = serde_json::json!({"boards":persistent,"dtbs":persistent,"ota":persistent,"configuration":"PostgreSQL（部署启动参数仍来自 TOML / .env）","active_sessions":"进程状态（重启释放，不恢复硬件连接）","artifacts":"临时文件（随租约清理）"});
    if public {
        for key in ["users", "roles", "rentals"] {
            storage[key] = serde_json::json!("PostgreSQL");
        }
    }
    Json(
        serde_json::json!({"edition":config.edition,"database_connected":true,"redis_connected":redis_connected,"security":security,"auth_enabled":public,"allow_registration":public&&config.auth.allow_registration,"public_base_url":config.auth.public_base_url,"access_token_ttl_secs":config.auth.access_token_ttl_secs,"storage":storage}),
    )
}
