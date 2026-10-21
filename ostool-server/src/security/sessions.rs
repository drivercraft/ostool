use super::web::{self, CurrentCredential};
use crate::{
    AppState,
    api::{auth::CurrentUser, error::ApiError},
};
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    routing::{delete, get, post},
};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/security/sessions", get(list))
        .route("/api/v1/auth/security/sessions/others", post(revoke_others))
        .route("/api/v1/auth/security/sessions/{id}", delete(revoke))
}
async fn family(state: &AppState, id: Option<Uuid>) -> Result<Uuid, ApiError> {
    let id = id.ok_or_else(|| ApiError::forbidden("请使用浏览器会话管理登录设备"))?;
    sqlx::query_scalar("SELECT family_id FROM oauth_sessions WHERE id=$1 AND revoked_at IS NULL")
        .bind(id)
        .fetch_optional(state.database.pool())
        .await?
        .ok_or_else(|| ApiError::unauthorized("登录已失效"))
}
async fn list(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentCredential(credential)): Extension<CurrentCredential>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(
        projection(&state, user.id, credential.session_id).await?,
    ))
}
pub async fn projection(
    state: &AppState,
    user: Uuid,
    current_session: Option<Uuid>,
) -> Result<Value, ApiError> {
    let current = family(state, current_session).await?;
    let rows=sqlx::query("SELECT DISTINCT ON (family_id) id,family_id,client_id,created_at,last_seen_at,absolute_expires_at,ip,user_agent FROM oauth_sessions WHERE user_id=$1 AND revoked_at IS NULL AND absolute_expires_at>now() AND last_seen_at>now()-interval '1 day' ORDER BY family_id,created_at DESC").bind(user).fetch_all(state.database.pool()).await?;
    let entries=rows.iter().map(|row|Ok(json!({"id":row.try_get::<Uuid,_>("id")?,"client":row.try_get::<String,_>("client_id")?,"created_at":row.try_get::<chrono::DateTime<chrono::Utc>,_>("created_at")?,"last_seen_at":row.try_get::<chrono::DateTime<chrono::Utc>,_>("last_seen_at")?,"expires_at":row.try_get::<chrono::DateTime<chrono::Utc>,_>("absolute_expires_at")?,"ip":row.try_get::<Option<String>,_>("ip")?,"user_agent":row.try_get::<Option<String>,_>("user_agent")?,"current":row.try_get::<Uuid,_>("family_id")?==current}))).collect::<Result<Vec<Value>,sqlx::Error>>()?;
    Ok(json!({"sessions":entries}))
}
async fn revoke_others(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentCredential(credential)): Extension<CurrentCredential>,
) -> Result<Json<Value>, ApiError> {
    web::require_step_up(&credential)?;
    let current = family(&state, credential.session_id).await?;
    let result=sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE user_id=$1 AND family_id<>$2 AND revoked_at IS NULL").bind(user.id).bind(current).execute(state.database.pool()).await?;
    Ok(Json(json!({"revoked":result.rows_affected()})))
}
async fn revoke(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentCredential(credential)): Extension<CurrentCredential>,
    Path(id): Path<Uuid>,
) -> Result<Json<Value>, ApiError> {
    web::require_step_up(&credential)?;
    let current = family(&state, credential.session_id).await?;
    let target: Option<Uuid> =
        sqlx::query_scalar("SELECT family_id FROM oauth_sessions WHERE id=$1 AND user_id=$2")
            .bind(id)
            .bind(user.id)
            .fetch_optional(state.database.pool())
            .await?;
    let target = target.ok_or_else(|| ApiError::not_found("登录会话不存在"))?;
    sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE family_id=$1 AND user_id=$2")
        .bind(target)
        .bind(user.id)
        .execute(state.database.pool())
        .await?;
    Ok(Json(json!({"revoked":true,"current":target==current})))
}
