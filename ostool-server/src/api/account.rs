//! Member workspace. Every personal query is scoped to the authenticated identity.
use super::{
    auth::CurrentUser,
    error::ApiError,
    rentals::{self, RentalInput},
};
use crate::AppState;
use axum::{
    Json, Router,
    extract::{Extension, Path, State},
    http::StatusCode,
    routing::{get, post, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Serialize, sqlx::FromRow)]
pub struct RentalRequest {
    pub id: Uuid,
    pub user_id: Uuid,
    pub user_email: String,
    pub board_id: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub notes: String,
    pub status: String,
    pub decision_note: String,
    pub reviewed_at: Option<DateTime<Utc>>,
    pub rental_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RequestInput {
    board_id: String,
    starts_at: DateTime<Utc>,
    ends_at: DateTime<Utc>,
    #[serde(default)]
    notes: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ReviewInput {
    decision: String,
    #[serde(default)]
    note: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileInput {
    email: String,
    display_name: Option<String>,
    current_password: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PasswordInput {
    current_password: String,
    new_password: String,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/account", get(snapshot))
        .route("/api/v1/account/rentals", get(member_rentals))
        .route("/api/v1/account/availability", get(member_availability))
        .route(
            "/api/v1/account/events",
            get(crate::admin_events::subscribe_account),
        )
        .route("/api/v1/account/profile", put(update_profile))
        .route("/api/v1/account/password", post(change_password))
        .route(
            "/api/v1/account/rental-requests",
            get(member_requests).post(submit),
        )
        .route(
            "/api/v1/account/rental-requests/{id}",
            axum::routing::delete(withdraw),
        )
        .route("/api/v1/admin/rental-requests", get(admin_list))
        .route("/api/v1/admin/rental-requests/{id}/review", post(review))
}

fn db_error(error: sqlx::Error) -> ApiError {
    if error
        .as_database_error()
        .is_some_and(|error| error.is_unique_violation())
    {
        return ApiError::conflict("该邮箱已被使用");
    }
    log::error!("account database operation failed: {error}");
    ApiError::service_unavailable("账号数据库暂时不可用")
}

async fn admin_list(
    State(state): State<AppState>,
    axum::extract::Query(mut filters): axum::extract::Query<super::listing::Filters>,
) -> Result<Json<Value>, ApiError> {
    filters.view = "rental_requests".into();
    super::listing::page(&state, &filters).await.map(Json)
}
async fn snapshot(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> Result<Json<Value>, ApiError> {
    projection(&state, user.id).await.map(Json)
}
async fn member_rentals(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    axum::extract::Query(mut filters): axum::extract::Query<super::listing::Filters>,
) -> Result<Json<Value>, ApiError> {
    filters.view = "rentals".into();
    super::listing::scoped_page(&state, &filters, Some(user.id))
        .await
        .map(Json)
}
async fn member_requests(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    axum::extract::Query(mut filters): axum::extract::Query<super::listing::Filters>,
) -> Result<Json<Value>, ApiError> {
    filters.view = "rental_requests".into();
    super::listing::scoped_page(&state, &filters, Some(user.id))
        .await
        .map(Json)
}

async fn member_availability(
    State(state): State<AppState>,
    axum::extract::Query(filters): axum::extract::Query<super::listing::EditorFilters>,
) -> Result<Json<Value>, ApiError> {
    availability(&state, &filters).await.map(Json)
}
pub(crate) async fn availability(
    state: &AppState,
    filters: &super::listing::EditorFilters,
) -> Result<Value, ApiError> {
    filters.validate()?;
    let from = filters.from.unwrap_or_else(Utc::now);
    let to = filters.to.unwrap_or(from + chrono::Duration::days(32));
    let visible = state
        .boards
        .read()
        .await
        .get(&filters.board)
        .is_some_and(|board| !board.disabled);
    let rows: Vec<Value> = if visible {
        sqlx::query_scalar("SELECT jsonb_build_object('id',id,'board_id',board_id,'starts_at',starts_at,'ends_at',ends_at,'status','reserved') FROM board_rentals WHERE board_id=$1 AND cancelled_at IS NULL AND starts_at<$3 AND ends_at>$2 ORDER BY starts_at,id LIMIT 10001")
            .bind(&filters.board).bind(from).bind(to).fetch_all(state.database.pool()).await.map_err(db_error)?
    } else {
        Vec::new()
    };
    if rows.len() > 10000 {
        return Err(ApiError::bad_request(
            "该日历范围的租赁过多，请缩小时间范围",
        ));
    }
    Ok(json!({"query":filters,"items":rows}))
}

pub(crate) async fn projection(state: &AppState, user: Uuid) -> Result<Value, ApiError> {
    let db = &state.database;
    let profile = db
        .user_by_id(user)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::unauthorized("账号不可用，请重新登录"))?;
    let catalog: Vec<_> = state
        .boards
        .read()
        .await
        .values()
        .filter(|b| !b.disabled)
        .map(|b| json!({"id":b.id,"board_type":b.board_type,"tags":b.tags}))
        .collect();
    let tokens = sqlx::query("SELECT id,name,expires_at,created_at FROM personal_access_tokens WHERE user_id=$1 ORDER BY created_at DESC")
        .bind(user).fetch_all(db.pool()).await.map_err(db_error)?.into_iter().map(|r| Ok(json!({"id":r.try_get::<Uuid,_>("id")?,"name":r.try_get::<String,_>("name")?,"expires_at":r.try_get::<Option<DateTime<Utc>>,_>("expires_at")?,"created_at":r.try_get::<DateTime<Utc>,_>("created_at")?}))).collect::<Result<Vec<_>,sqlx::Error>>().map_err(db_error)?;
    let summary: Value = sqlx::query_scalar("SELECT jsonb_build_object('active',(SELECT count(*) FROM board_rentals WHERE user_id=$1 AND cancelled_at IS NULL AND starts_at<=now() AND ends_at>now()),'scheduled',(SELECT count(*) FROM board_rentals WHERE user_id=$1 AND cancelled_at IS NULL AND starts_at>now()),'pending',(SELECT count(*) FROM rental_requests WHERE user_id=$1 AND status='pending'))").bind(user).fetch_one(db.pool()).await.map_err(db_error)?;
    let access = crate::security::permissions::access(db, &profile).await?;
    Ok(
        json!({"profile":{"permissions":access.permissions,"can_access_admin":access.allows("console.access"),"allocation_policy":access.allocation_policy,"id":profile.id,"username":profile.username,"email":profile.email,"display_name":profile.display_name,"is_admin":profile.is_admin},"catalog":catalog,"account_summary":summary,"personal_tokens":tokens}),
    )
}

async fn update_profile(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(crate::security::web::CurrentCredential(credential)): Extension<
        crate::security::web::CurrentCredential,
    >,
    Json(input): Json<ProfileInput>,
) -> Result<StatusCode, ApiError> {
    let email = crate::persistence::postgres::normalize_email(&input.email)
        .map_err(|_| ApiError::bad_request("邮箱格式无效"))?;
    let display_name =
        crate::security::names::normalize_display_name(input.display_name.as_deref())?;
    if input.email.trim().to_lowercase() != user.email {
        crate::security::web::require_step_up(&credential)?;
    }
    let db = &state.database;
    let mut tx = db.pool().begin().await.map_err(db_error)?;
    let current = sqlx::query(
        "SELECT email,password_hash,username,display_name FROM users WHERE id=$1 AND NOT disabled FOR UPDATE",
    )
    .bind(user.id)
    .fetch_optional(&mut *tx)
    .await
    .map_err(db_error)?
    .ok_or_else(|| ApiError::unauthorized("账号不可用"))?;
    let current_username: String = current.try_get("username").map_err(db_error)?;
    let current_name: Option<String> = current.try_get("display_name").map_err(db_error)?;
    state
        .config
        .read()
        .await
        .security
        .name_policy
        .check_change(
            &current_username,
            display_name.as_deref(),
            Some((&current_username, current_name.as_deref())),
        )?;
    if email != current.try_get::<String, _>("email").map_err(db_error)? {
        let hash = current
            .try_get::<String, _>("password_hash")
            .map_err(db_error)?;
        let password = input.current_password.unwrap_or_default();
        if password.len() > 1024
            || !crate::persistence::postgres::verify_password_async(password, hash)
                .await
                .map_err(ApiError::from)?
        {
            return Err(ApiError::bad_request("修改邮箱需要正确的当前密码"));
        }
    }
    sqlx::query("UPDATE users SET email=$2,display_name=$3 WHERE id=$1")
        .bind(user.id)
        .bind(email)
        .bind(display_name)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    state.admin_events.invalidate(&["users", "rental_requests"]);
    Ok(StatusCode::NO_CONTENT)
}
async fn change_password(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(input): Json<PasswordInput>,
) -> Result<StatusCode, ApiError> {
    crate::security::crypto::validate_password(&input.new_password)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    if input.current_password.len() > 1024 {
        return Err(ApiError::bad_request("当前密码过长"));
    }
    let db = &state.database;
    let mut tx = db.pool().begin().await.map_err(db_error)?;
    let hash: String = sqlx::query_scalar(
        "SELECT password_hash FROM users WHERE id=$1 AND NOT disabled FOR UPDATE",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await
    .map_err(db_error)?;
    if !crate::persistence::postgres::verify_password_async(input.current_password, hash)
        .await
        .map_err(ApiError::from)?
    {
        return Err(ApiError::bad_request("当前密码不正确"));
    }
    let new_hash = crate::persistence::postgres::hash_password_async(input.new_password)
        .await
        .map_err(ApiError::from)?;
    sqlx::query("UPDATE users SET password_hash=$2 WHERE id=$1")
        .bind(user.id)
        .bind(new_hash)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    sqlx::query(
        "UPDATE oauth_sessions SET revoked_at=now() WHERE user_id=$1 AND revoked_at IS NULL",
    )
    .bind(user.id)
    .execute(&mut *tx)
    .await
    .map_err(db_error)?;
    sqlx::query("DELETE FROM personal_access_tokens WHERE user_id=$1")
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    sqlx::query("DELETE FROM oauth_device_codes WHERE user_id=$1")
        .bind(user.id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    tx.commit().await.map_err(db_error)?;
    state.admin_events.invalidate(&["users"]);
    Ok(StatusCode::NO_CONTENT)
}
async fn submit(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(input): Json<RequestInput>,
) -> Result<(StatusCode, Extension<crate::audit::Target>), ApiError> {
    if input.starts_at >= input.ends_at || input.ends_at <= Utc::now() || input.notes.len() > 2000 {
        return Err(ApiError::bad_request(
            "请选择有效的起止时间，申请说明不能超过 2000 字符",
        ));
    }
    let _gate = state.rental_gate.lock().await;
    let _inventory = state.board_inventory_gate.lock().await;
    if state
        .boards
        .read()
        .await
        .get(&input.board_id)
        .is_none_or(|b| b.disabled)
    {
        return Err(ApiError::bad_request("开发板不存在或已禁用"));
    }
    let overlap:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM board_rentals WHERE board_id=$1 AND cancelled_at IS NULL AND starts_at<$3 AND ends_at>$2)").bind(&input.board_id).bind(input.starts_at).bind(input.ends_at).fetch_one(state.database.pool()).await.map_err(db_error)?;
    if overlap {
        return Err(ApiError::conflict("该时段已有租赁，请调整申请时间"));
    }
    let duplicate:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM rental_requests WHERE user_id=$1 AND board_id=$2 AND status='pending' AND starts_at=$3 AND ends_at=$4)").bind(user.id).bind(&input.board_id).bind(input.starts_at).bind(input.ends_at).fetch_one(state.database.pool()).await.map_err(db_error)?;
    if duplicate {
        return Err(ApiError::conflict("相同申请正在审核中，请勿重复提交"));
    }
    let id: Uuid = sqlx::query_scalar("INSERT INTO rental_requests (user_id,board_id,starts_at,ends_at,notes) VALUES ($1,$2,$3,$4,$5) RETURNING id").bind(user.id).bind(input.board_id).bind(input.starts_at).bind(input.ends_at).bind(input.notes).fetch_one(state.database.pool()).await.map_err(db_error)?;
    state.admin_events.invalidate(&["rental_requests"]);
    Ok((
        StatusCode::CREATED,
        Extension(crate::audit::Target(id.to_string())),
    ))
}
async fn withdraw(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
) -> Result<StatusCode, ApiError> {
    let count=sqlx::query("UPDATE rental_requests SET status='cancelled' WHERE id=$1 AND user_id=$2 AND status='pending'").bind(id).bind(user.id).execute(state.database.pool()).await.map_err(db_error)?.rows_affected();
    if count == 0 {
        return Err(ApiError::not_found("申请不存在或已处理"));
    }
    state.admin_events.invalidate(&["rental_requests"]);
    Ok(StatusCode::NO_CONTENT)
}
async fn review(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Path(id): Path<Uuid>,
    Json(input): Json<ReviewInput>,
) -> Result<StatusCode, ApiError> {
    if input.note.len() > 2000 {
        return Err(ApiError::bad_request("审核说明不能超过 2000 字符"));
    }
    match input.decision.as_str() {
        "approve" => {
            let request: RentalRequest=sqlx::query_as("SELECT r.*,u.email AS user_email FROM rental_requests r JOIN users u ON r.user_id=u.id WHERE r.id=$1").bind(id).fetch_optional(state.database.pool()).await.map_err(db_error)?.ok_or_else(|| ApiError::not_found("申请不存在"))?;
            rentals::save_internal(
                &state,
                None,
                RentalInput {
                    board_id: request.board_id,
                    user_id: request.user_id,
                    starts_at: request.starts_at,
                    ends_at: request.ends_at,
                    notes: request.notes,
                },
                Some((id, user.id, input.note)),
            )
            .await?;
        }
        "reject" => {
            let count=sqlx::query("UPDATE rental_requests SET status='rejected',decision_note=$2,reviewed_by=$3,reviewed_at=now() WHERE id=$1 AND status='pending'").bind(id).bind(input.note).bind(user.id).execute(state.database.pool()).await.map_err(db_error)?.rows_affected();
            if count == 0 {
                return Err(ApiError::conflict("申请不存在或已处理"));
            }
            state.admin_events.invalidate(&["rental_requests"]);
        }
        _ => return Err(ApiError::bad_request("审核结果必须为 approve 或 reject")),
    }
    Ok(StatusCode::NO_CONTENT)
}
