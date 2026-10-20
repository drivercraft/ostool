//! Administrator user and role management. Read projections share admin SSE.
use super::{auth::CurrentUser, error::ApiError};
use crate::{AppState, database::Database};
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
}
#[derive(Debug, Serialize, Deserialize)]
pub struct ManagedUser {
    pub id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub role_id: String,
    pub disabled: bool,
    pub created_at: DateTime<Utc>,
}
#[derive(Deserialize)]
pub struct UserInput {
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

fn db(state: &AppState) -> Result<&Database, ApiError> {
    state
        .database
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("PostgreSQL 未配置"))
}
fn sql_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|e| e.is_unique_violation() || e.is_foreign_key_violation())
    {
        ApiError::conflict("邮箱或角色名称已存在，或该角色仍在使用")
    } else {
        log::error!("user management database error: {error}");
        ApiError::service_unavailable("用户数据库操作失败")
    }
}

pub async fn list_users(State(state): State<AppState>) -> Result<Json<Vec<ManagedUser>>, ApiError> {
    let Some(db) = &state.database else {
        return Ok(Json(Vec::new()));
    };
    let rows = sqlx::query("SELECT id,email,display_name,role_id,disabled,created_at FROM users ORDER BY created_at,id")
        .fetch_all(db.pool()).await.map_err(sql_error)?;
    rows.iter()
        .map(|r| {
            Ok(ManagedUser {
                id: r.try_get("id")?,
                email: r.try_get("email")?,
                display_name: r.try_get("display_name")?,
                role_id: r.try_get("role_id")?,
                disabled: r.try_get("disabled")?,
                created_at: r.try_get("created_at")?,
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map(Json)
        .map_err(sql_error)
}
pub async fn list_roles(State(state): State<AppState>) -> Result<Json<Vec<Role>>, ApiError> {
    let Some(db) = &state.database else {
        return Ok(Json(Vec::new()));
    };
    let rows = sqlx::query("SELECT * FROM roles ORDER BY builtin DESC,id")
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
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map(Json)
        .map_err(sql_error)
}
fn validate(input: &UserInput) -> Result<(), ApiError> {
    crate::database::normalize_email(&input.email)
        .map_err(|_| ApiError::bad_request("邮箱格式无效"))?;
    if input.display_name.as_ref().is_some_and(|s| s.len() > 120) {
        return Err(ApiError::bad_request("显示名称过长"));
    }
    if input
        .password
        .as_ref()
        .is_some_and(|s| s.len() < 12 || s.len() > 1024)
    {
        return Err(ApiError::bad_request("密码需要 12 至 1024 个字符"));
    }
    Ok(())
}
async fn create_user(
    State(state): State<AppState>,
    Json(input): Json<UserInput>,
) -> Result<StatusCode, ApiError> {
    validate(&input)?;
    let password = input
        .password
        .as_deref()
        .ok_or_else(|| ApiError::bad_request("新用户需要初始密码"))?;
    let hash = crate::database::hash_password_async(password.to_owned()).await?;
    let pool = db(&state)?.pool();
    let result = sqlx::query("INSERT INTO users(email,display_name,password_hash,role_id,is_admin,disabled) SELECT $1,$2,$3,id,administrator,$5 FROM roles WHERE id=$4")
        .bind(input.email.trim().to_lowercase()).bind(input.display_name).bind(hash).bind(input.role_id).bind(input.disabled).execute(pool).await.map_err(sql_error)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::bad_request("角色不存在"));
    }
    state.admin_events.invalidate(&["users"]);
    Ok(StatusCode::CREATED)
}
async fn update_user(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Extension(CurrentUser(actor)): Extension<CurrentUser>,
    Json(input): Json<UserInput>,
) -> Result<StatusCode, ApiError> {
    validate(&input)?;
    let hash = match input.password {
        Some(ref p) => Some(crate::database::hash_password_async(p.clone()).await?),
        None => None,
    };
    let mut tx = db(&state)?.pool().begin().await.map_err(sql_error)?;
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
    let result=sqlx::query("UPDATE users SET email=$1,display_name=$2,role_id=$3,is_admin=$4,disabled=$5,password_hash=COALESCE($6,password_hash) WHERE id=$7")
        .bind(input.email.trim().to_lowercase()).bind(input.display_name).bind(input.role_id).bind(administrator).bind(input.disabled).bind(hash.as_deref()).bind(id).execute(&mut *tx).await.map_err(sql_error)?;
    if result.rows_affected() != 1 {
        return Err(ApiError::not_found("用户不存在"));
    }
    ensure_admin(&mut tx).await?;
    // Role/status/password edits invalidate previously issued credentials.
    sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(sql_error)?;
    sqlx::query("DELETE FROM personal_access_tokens WHERE user_id=$1")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(sql_error)?;
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
    let mut tx = db(&state)?.pool().begin().await.map_err(sql_error)?;
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
) -> Result<StatusCode, ApiError> {
    validate_role(&input)?;
    sqlx::query("INSERT INTO roles(id,name,description,administrator) VALUES($1,$2,$3,$4)")
        .bind(Uuid::new_v4().to_string())
        .bind(input.name.trim())
        .bind(input.description)
        .bind(input.administrator)
        .execute(db(&state)?.pool())
        .await
        .map_err(sql_error)?;
    state.admin_events.invalidate(&["roles"]);
    Ok(StatusCode::CREATED)
}
async fn update_role(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(input): Json<RoleInput>,
) -> Result<StatusCode, ApiError> {
    validate_role(&input)?;
    let mut tx = db(&state)?.pool().begin().await.map_err(sql_error)?;
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
        .execute(db(&state)?.pool())
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
    let mut storage = serde_json::json!({"boards":if state.database.is_some(){"PostgreSQL"}else{"TOML"},"dtbs":if state.database.is_some(){"文件 + PostgreSQL 元数据镜像"}else{"文件"},"ota":"文件","configuration":"TOML + 环境变量","active_sessions":"内存（重启失效）","artifacts":"文件（随租约清理）"});
    if public {
        for key in ["users", "roles", "rentals"] {
            storage[key] = serde_json::json!("PostgreSQL");
        }
    }
    Json(
        serde_json::json!({"edition":config.edition,"database_connected":state.database.is_some(),"auth_enabled":public,"allow_registration":public&&config.auth.allow_registration,"public_base_url":config.auth.public_base_url,"access_token_ttl_secs":config.auth.access_token_ttl_secs,"storage":storage}),
    )
}
