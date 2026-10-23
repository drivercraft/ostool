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
    pub id: String,
    pub name: String,
    pub description: String,
    pub administrator: bool,
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
    pub name: String,
    pub description: String,
    pub administrator: bool,
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
}

fn sql_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation() || e.is_foreign_key_violation())
    {
        ApiError::conflict("用户名、邮箱或角色名称已存在，或该角色仍在使用")
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
                id: r.try_get("id")?,
                name: r.try_get("name")?,
                description: r.try_get("description")?,
                administrator: r.try_get("administrator")?,
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
    if input.display_name.as_ref().is_some_and(|s| s.len() > 120) {
        return Err(ApiError::bad_request("显示名称过长"));
    }
    if let Some(password) = input.password.as_deref() {
        crate::security::crypto::validate_password(password)
            .map_err(|error| ApiError::bad_request(error.to_string()))?;
    }
    Ok(())
}
async fn create_user(
    State(state): State<AppState>,
    Json(input): Json<UserInput>,
) -> Result<(StatusCode, axum::extract::Extension<crate::audit::Target>), ApiError> {
    validate(&input)?;
    let password = input
        .password
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("新用户需要初始密码"))?;
    let hash = crate::persistence::postgres::hash_password_async(password.to_owned()).await?;
    let pool = state.database.pool();
    let result = sqlx::query_scalar::<_,Uuid>("INSERT INTO users(email,display_name,password_hash,role_id,is_admin,disabled,username) SELECT $1,$2,$3,id,administrator,$5,$6 FROM roles WHERE id=$4 RETURNING users.id")
        .bind(input.email.trim().to_lowercase()).bind(input.display_name).bind(hash).bind(input.role_id).bind(input.disabled).bind(input.username.trim().to_ascii_lowercase()).fetch_optional(pool).await.map_err(sql_error)?;
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
    let role = sqlx::query("SELECT administrator FROM roles WHERE id=$1")
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
        .bind(input.email.trim().to_lowercase()).bind(input.display_name).bind(input.role_id).bind(administrator).bind(input.disabled).bind(hash.as_deref()).bind(id).bind(input.username.trim().to_ascii_lowercase()).execute(&mut *tx).await.map_err(sql_error)?;
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
async fn ensure_admin(tx: &mut sqlx::Transaction<'_, sqlx::Postgres>) -> Result<(), ApiError> {
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM users WHERE is_admin AND NOT disabled")
            .fetch_one(&mut **tx)
            .await
            .map_err(sql_error)?;
    if count == 0 {
        return Err(ApiError::conflict("至少需要保留一位启用的管理员"));
    }
    Ok(())
}
fn validate_role(input: &RoleInput) -> Result<(), ApiError> {
    if input.name.trim().is_empty() || input.name.len() > 80 || input.description.len() > 500 {
        return Err(ApiError::bad_request(
            "角色名称需要 1 至 80 个字符，描述不超过 500 个字符",
        ));
    }
    Ok(())
}
async fn create_role(
    State(state): State<AppState>,
    Json(input): Json<RoleInput>,
) -> Result<(StatusCode, Extension<crate::audit::Target>), ApiError> {
    validate_role(&input)?;
    let id = Uuid::new_v4().to_string();
    sqlx::query("INSERT INTO roles(id,name,description,administrator) VALUES($1,$2,$3,$4)")
        .bind(&id)
        .bind(input.name.trim())
        .bind(input.description)
        .bind(input.administrator)
        .execute(state.database.pool())
        .await
        .map_err(sql_error)?;
    state.admin_events.invalidate(&["roles"]);
    Ok((StatusCode::CREATED, Extension(crate::audit::Target(id))))
}
async fn update_role(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(input): Json<RoleInput>,
) -> Result<StatusCode, ApiError> {
    validate_role(&input)?;
    let mut tx = state.database.pool().begin().await.map_err(sql_error)?;
    sqlx::query("LOCK TABLE users IN SHARE ROW EXCLUSIVE MODE")
        .execute(&mut *tx)
        .await
        .map_err(sql_error)?;
    let row = sqlx::query("SELECT builtin,administrator FROM roles WHERE id=$1 FOR UPDATE")
        .bind(&id)
        .fetch_optional(&mut *tx)
        .await
        .map_err(sql_error)?
        .ok_or_else(|| ApiError::not_found("角色不存在"))?;
    if row.try_get::<bool, _>("builtin").map_err(sql_error)?
        && input.administrator != row.try_get::<bool, _>("administrator").map_err(sql_error)?
    {
        return Err(ApiError::conflict("内置角色的权限不可修改"));
    }
    // Permission changes require explicit reassignment, avoiding mass escalation.
    if input.administrator != row.try_get::<bool, _>("administrator").map_err(sql_error)? {
        let count: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE role_id=$1")
            .bind(&id)
            .fetch_one(&mut *tx)
            .await
            .map_err(sql_error)?;
        if count > 0 {
            return Err(ApiError::conflict(
                "先将此角色的用户转移至其他角色，再修改权限",
            ));
        }
    }
    sqlx::query("UPDATE roles SET name=$1,description=$2,administrator=$3 WHERE id=$4")
        .bind(input.name.trim())
        .bind(input.description)
        .bind(input.administrator)
        .bind(&id)
        .execute(&mut *tx)
        .await
        .map_err(sql_error)?;
    tx.commit().await.map_err(sql_error)?;
    state.admin_events.invalidate(&["roles"]);
    Ok(StatusCode::NO_CONTENT)
}
async fn delete_role(
    Path(id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
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
            "SMTP 主机":config.security.smtp_host,
            "SMTP 端口":config.security.smtp_port,
            "SMTP TLS":config.security.smtp_tls,
            "发件人":config.security.smtp_from,
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
