//! On-demand allocations reuse rental ownership and every existing session check.
use crate::{
    AppState, BoardLeaseState, api::error::ApiError, persistence::postgres::User,
    security::permissions,
};
use chrono::{DateTime, Duration, Utc};
use sqlx::Row;
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;

/// The caller holds rental_gate when claiming an allocation. Listing only previews
/// candidates; final admission always repeats these checks under the same gate.
pub async fn candidates(
    state: &AppState,
    user: &User,
    board_type: Option<&str>,
    board_id: Option<&str>,
    tags: &[String],
) -> Result<BTreeMap<String, DateTime<Utc>>, ApiError> {
    let access = permissions::access(&state.database, user).await?;
    if !access.allows("board.instant") {
        return Ok(BTreeMap::new());
    }
    let now = Utc::now();
    let end = now + Duration::minutes(i64::from(access.allocation_policy.max_duration_minutes));
    let min_end =
        now + Duration::minutes(i64::from(access.allocation_policy.min_available_minutes));
    let boards = state.boards.read().await;
    let ids = boards
        .values()
        .filter(|b| {
            b.allow_instant_use
                && !b.disabled
                && board_type.is_none_or(|t| b.board_type == t)
                && board_id.is_none_or(|id| b.id == id)
                && tags.iter().all(|t| b.tags.contains(t))
                && (access.allocation_policy.board_types.is_empty()
                    || access.allocation_policy.board_types.contains(&b.board_type))
        })
        .map(|b| b.id.clone())
        .collect::<Vec<_>>();
    drop(boards);
    if ids.is_empty() {
        return Ok(BTreeMap::new());
    }
    let rows=sqlx::query("SELECT board_id,starts_at,ends_at FROM board_rentals WHERE board_id=ANY($1) AND cancelled_at IS NULL AND ends_at>now() AND starts_at<$2 ORDER BY starts_at")
  .bind(&ids).bind(end+Duration::seconds(60)).fetch_all(state.database.pool()).await?;
    let mut result = ids
        .into_iter()
        .map(|id| (id, end))
        .collect::<BTreeMap<_, _>>();
    for row in rows {
        let id: String = row.try_get("board_id")?;
        let start: DateTime<Utc> = row.try_get("starts_at")?;
        if start <= now {
            result.remove(&id);
        } else if let Some(limit) = result.get_mut(&id) {
            *limit = (*limit).min(start - Duration::seconds(60));
        }
    }
    result.retain(|_, end| *end >= min_end);
    Ok(result)
}
pub async fn assign(
    state: &AppState,
    user: &User,
    board_type: &str,
    board_id: Option<&str>,
    tags: &[String],
) -> Result<(String, Uuid, DateTime<Utc>), ApiError> {
    let access = permissions::access(&state.database, user).await?;
    if !access.allows("board.instant") {
        return Err(ApiError::forbidden("当前角色没有即时分配权限"));
    }
    let count: i64 = sqlx::query_scalar(
        "SELECT count(DISTINCT board_id) FROM (SELECT board_id FROM session_history WHERE user_id=$1 AND ended_at IS NULL UNION SELECT board_id FROM board_rentals WHERE user_id=$1 AND source='instant' AND cancelled_at IS NULL AND starts_at<=now() AND ends_at>now()) allocations",
    )
    .bind(user.id)
    .fetch_one(state.database.pool())
    .await?;
    if count >= i64::from(access.allocation_policy.max_concurrent_sessions) {
        return Err(ApiError::conflict("已达到当前角色允许的设备并发使用上限"));
    }
    let candidates = candidates(state, user, Some(board_type), board_id, tags).await?;
    let boards = state.boards.read().await;
    let runtimes = state.board_runtimes.read().await;
    let unavailable = runtimes
        .iter()
        .filter(|(_, r)| r.lease_state != BoardLeaseState::Idle)
        .map(|(id, _)| id.clone())
        .collect::<BTreeSet<_>>();
    let selected = candidates.into_iter().find(|(id, _)| {
        !unavailable.contains(id)
            && boards
                .get(id)
                .is_some_and(|b| !state.admin_power.is_busy(&b.power_management))
    });
    drop(runtimes);
    drop(boards);
    let (id, end) = selected.ok_or_else(|| {
        ApiError::conflict(match board_id {
            Some(id) => format!("board `{id}` is not available"),
            None => format!("no available board for type `{board_type}`"),
        })
    })?;
    let rental = Uuid::new_v4();
    let mut tx = state.database.begin().await?;
    sqlx::query("INSERT INTO board_rentals(id,board_id,user_id,starts_at,ends_at,notes,source) VALUES($1,$2,$3,now(),$4,'即时使用自动分配','instant')")
  .bind(rental).bind(&id).bind(user.id).bind(end).execute(&mut *tx).await?;
    sqlx::query("INSERT INTO rental_requests(user_id,board_id,starts_at,ends_at,notes,status,decision_note,reviewed_at,rental_id) SELECT user_id,board_id,starts_at,ends_at,notes,'auto_approved','自动放行',now(),id FROM board_rentals WHERE id=$1")
        .bind(rental).execute(&mut *tx).await?;
    tx.commit().await?;
    state
        .admin_events
        .invalidate(&["rentals", "rental_requests"]);
    state.rental_changed.notify_one();
    Ok((id, rental, end))
}
pub async fn cancel_failed(state: &AppState, id: Uuid) {
    if let Err(error) = sqlx::query(
        "WITH cancelled AS (UPDATE board_rentals SET cancelled_at=now() WHERE id=$1 AND source='instant' RETURNING id) UPDATE rental_requests SET status='cancelled',decision_note='自动放行后分配失败，已取消' WHERE rental_id IN (SELECT id FROM cancelled)",
    )
            .bind(id)
            .execute(state.database.pool())
            .await
    {
        log::error!("failed to cancel unclaimed instant allocation {id}: {error}");
    }
    state
        .admin_events
        .invalidate(&["rentals", "rental_requests"]);
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllocationInput {
    pub board_id: String,
}

pub async fn claim(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Extension(crate::api::auth::CurrentUser(user)): axum::extract::Extension<
        crate::api::auth::CurrentUser,
    >,
    axum::Json(input): axum::Json<AllocationInput>,
) -> Result<
    (
        axum::http::StatusCode,
        axum::extract::Extension<crate::audit::Target>,
        axum::Json<serde_json::Value>,
    ),
    ApiError,
> {
    if input.board_id.is_empty() || input.board_id.len() > 256 {
        return Err(ApiError::bad_request("请选择开发板"));
    }
    let _gate = state.rental_gate.lock().await;
    let _inventory = state.board_inventory_gate.lock().await;
    let kind = state
        .boards
        .read()
        .await
        .get(&input.board_id)
        .map(|b| b.board_type.clone())
        .ok_or_else(|| ApiError::not_found("开发板不存在"))?;
    let (board, id, end) = assign(&state, &user, &kind, Some(&input.board_id), &[]).await?;
    Ok((
        axum::http::StatusCode::CREATED,
        axum::extract::Extension(crate::audit::Target(id.to_string())),
        axum::Json(serde_json::json!({"id":id,"board_id":board,"ends_at":end})),
    ))
}
pub async fn release(
    axum::extract::State(state): axum::extract::State<AppState>,
    axum::extract::Extension(crate::api::auth::CurrentUser(user)): axum::extract::Extension<
        crate::api::auth::CurrentUser,
    >,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<axum::http::StatusCode, ApiError> {
    let _gate = state.rental_gate.lock().await;
    let changed=sqlx::query("UPDATE board_rentals SET cancelled_at=now() WHERE id=$1 AND user_id=$2 AND source='instant' AND cancelled_at IS NULL AND ends_at>now()")
        .bind(id).bind(user.id).execute(state.database.pool()).await?;
    if changed.rows_affected() == 0 {
        return Err(ApiError::not_found("即时分配记录不存在或已结束"));
    }
    sqlx::query("UPDATE rental_requests SET status='cancelled',decision_note='用户主动归还' WHERE rental_id=$1 AND status='auto_approved'").bind(id).execute(state.database.pool()).await?;
    crate::api::rentals::enforce(&state).await?;
    state
        .admin_events
        .invalidate(&["rentals", "rental_requests"]);
    state.rental_changed.notify_one();
    Ok(axum::http::StatusCode::NO_CONTENT)
}
