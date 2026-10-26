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
        "SELECT count(*) FROM session_history WHERE user_id=$1 AND ended_at IS NULL",
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
    sqlx::query("INSERT INTO board_rentals(id,board_id,user_id,starts_at,ends_at,notes,source) VALUES($1,$2,$3,now(),$4,'即时使用自动分配','instant')")
  .bind(rental).bind(&id).bind(user.id).bind(end).execute(state.database.pool()).await?;
    state.admin_events.invalidate(&["rentals"]);
    state.rental_changed.notify_one();
    Ok((id, rental, end))
}
pub async fn cancel_failed(state: &AppState, id: Uuid) {
    if let Err(error) =
        sqlx::query("UPDATE board_rentals SET cancelled_at=now() WHERE id=$1 AND source='instant'")
            .bind(id)
            .execute(state.database.pool())
            .await
    {
        log::error!("failed to cancel unclaimed instant allocation {id}: {error}");
    }
    state.admin_events.invalidate(&["rentals"]);
}
