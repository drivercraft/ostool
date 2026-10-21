//! Scheduled exclusive board access. All mutations and session admission share
//! a process gate, with a PostgreSQL advisory lock serializing overlap checks.
use crate::{AppState, api::error::ApiError, session::SessionStopReason};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::{get, put},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::Row;
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rental {
    pub id: Uuid,
    pub board_id: String,
    pub user_id: Uuid,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    pub notes: String,
    pub cancelled_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub status: String,
}
#[derive(Debug, Deserialize)]
pub struct RentalInput {
    pub board_id: String,
    pub user_id: Uuid,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    #[serde(default)]
    pub notes: String,
}
fn database(state: &AppState) -> Result<&crate::database::Database, ApiError> {
    state
        .database
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("租赁管理需要 PostgreSQL"))
}
fn db_error(error: sqlx::Error) -> ApiError {
    log::error!("rental database operation failed: {error}");
    ApiError::service_unavailable("租赁数据库暂时不可用")
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/rentals", get(list).post(create))
        .route("/api/v1/admin/rentals/{id}", put(update).delete(cancel))
}
fn status(
    starts: DateTime<Utc>,
    ends: DateTime<Utc>,
    cancelled: bool,
    now: DateTime<Utc>,
) -> &'static str {
    if cancelled {
        "cancelled"
    } else if now >= ends {
        "expired"
    } else if now < starts {
        "scheduled"
    } else {
        "active"
    }
}
pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<Rental>>, ApiError> {
    list_for(&state, None).await.map(Json)
}
pub(crate) async fn list_for(
    state: &AppState,
    user: Option<Uuid>,
) -> Result<Vec<Rental>, ApiError> {
    let rows = sqlx::query("SELECT * FROM board_rentals WHERE ($1::uuid IS NULL OR user_id=$1) ORDER BY created_at DESC,id")
        .bind(user).fetch_all(database(state)?.pool())
        .await
        .map_err(db_error)?;
    let now = Utc::now();
    rows.iter()
        .map(|row| {
            let starts_at = row.try_get("starts_at")?;
            let ends_at = row.try_get("ends_at")?;
            let cancelled_at: Option<DateTime<Utc>> = row.try_get("cancelled_at")?;
            Ok(Rental {
                id: row.try_get("id")?,
                board_id: row.try_get("board_id")?,
                user_id: row.try_get("user_id")?,
                starts_at,
                ends_at,
                notes: row.try_get("notes")?,
                cancelled_at,
                created_at: row.try_get("created_at")?,
                status: status(starts_at, ends_at, cancelled_at.is_some(), now).into(),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()
        .map_err(db_error)
}
async fn create(
    State(state): State<AppState>,
    Json(input): Json<RentalInput>,
) -> Result<StatusCode, ApiError> {
    save(&state, None, input).await?;
    Ok(StatusCode::CREATED)
}
async fn update(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Json(input): Json<RentalInput>,
) -> Result<StatusCode, ApiError> {
    save(&state, Some(id), input).await?;
    Ok(StatusCode::NO_CONTENT)
}
async fn save(state: &AppState, id: Option<Uuid>, input: RentalInput) -> Result<Uuid, ApiError> {
    save_internal(state, id, input, None).await
}

pub(crate) async fn save_internal(
    state: &AppState,
    id: Option<Uuid>,
    input: RentalInput,
    approval: Option<(Uuid, Uuid, String)>,
) -> Result<Uuid, ApiError> {
    if input.starts_at >= input.ends_at || input.ends_at <= Utc::now() {
        return Err(ApiError::bad_request(
            "开始时间必须早于结束时间，且结束时间必须在未来",
        ));
    }
    if input.notes.len() > 2000 {
        return Err(ApiError::bad_request("备注不能超过 2000 字符"));
    }
    let _gate = state.rental_gate.lock().await;
    let _inventory = state.board_inventory_gate.lock().await;
    let board = state
        .boards
        .read()
        .await
        .get(&input.board_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found("开发板不存在"))?;
    if board.disabled {
        return Err(ApiError::conflict("已禁用的开发板不能分配租赁"));
    }
    let mut tx = database(state)?.pool().begin().await.map_err(db_error)?;
    if let Some((request_id, _, _)) = &approval {
        let pending: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM (SELECT * FROM rental_requests WHERE id=$1 FOR UPDATE) r WHERE status='pending' AND user_id=$2 AND board_id=$3 AND starts_at=$4 AND ends_at=$5)")
            .bind(request_id).bind(input.user_id).bind(&input.board_id).bind(input.starts_at).bind(input.ends_at)
            .fetch_one(&mut *tx).await.map_err(db_error)?;
        if !pending {
            return Err(ApiError::conflict("申请已处理或已撤回，请检查最新状态"));
        }
    }
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,2999))")
        .bind(&input.board_id)
        .execute(&mut *tx)
        .await
        .map_err(db_error)?;
    if let Some(id) = id {
        let existing=sqlx::query("SELECT board_id,user_id,cancelled_at,ends_at FROM board_rentals WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await.map_err(db_error)?.ok_or_else(||ApiError::not_found("租赁不存在"))?;
        if existing
            .try_get::<String, _>("board_id")
            .map_err(db_error)?
            != input.board_id
            || existing.try_get::<Uuid, _>("user_id").map_err(db_error)? != input.user_id
        {
            return Err(ApiError::bad_request(
                "修改用户或开发板请先取消原租赁，再新增",
            ));
        }
        if existing
            .try_get::<Option<DateTime<Utc>>, _>("cancelled_at")
            .map_err(db_error)?
            .is_some()
            || existing
                .try_get::<DateTime<Utc>, _>("ends_at")
                .map_err(db_error)?
                <= Utc::now()
        {
            return Err(ApiError::conflict("已取消或已结束的租赁不能编辑"));
        }
    }
    let enabled: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE id=$1 AND NOT disabled)")
            .bind(input.user_id)
            .fetch_one(&mut *tx)
            .await
            .map_err(db_error)?;
    if !enabled {
        return Err(ApiError::bad_request("用户不存在或已禁用"));
    }
    let overlap:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM board_rentals WHERE board_id=$1 AND cancelled_at IS NULL AND starts_at<$3 AND ends_at>$2 AND ($4::uuid IS NULL OR id<>$4))").bind(&input.board_id).bind(input.starts_at).bind(input.ends_at).bind(id).fetch_one(&mut *tx).await.map_err(db_error)?;
    if overlap {
        return Err(ApiError::conflict(
            "该开发板在指定时间段已有租赁，请调整时间",
        ));
    }
    let rental_id = id.unwrap_or_else(Uuid::new_v4);
    match id {
        Some(id) => {
            sqlx::query("UPDATE board_rentals SET starts_at=$1,ends_at=$2,notes=$3 WHERE id=$4")
                .bind(input.starts_at)
                .bind(input.ends_at)
                .bind(input.notes)
                .bind(id)
                .execute(&mut *tx)
                .await
                .map_err(db_error)?;
        }
        None => {
            sqlx::query("INSERT INTO board_rentals(id,board_id,user_id,starts_at,ends_at,notes) VALUES($1,$2,$3,$4,$5,$6)").bind(rental_id).bind(input.board_id).bind(input.user_id).bind(input.starts_at).bind(input.ends_at).bind(input.notes).execute(&mut *tx).await.map_err(db_error)?;
        }
    }
    if let Some((request_id, reviewer, note)) = approval {
        sqlx::query("UPDATE rental_requests SET status='approved',reviewed_by=$2,reviewed_at=now(),decision_note=$3,rental_id=$4 WHERE id=$1")
            .bind(request_id).bind(reviewer).bind(note).bind(rental_id).execute(&mut *tx).await.map_err(db_error)?;
    }
    tx.commit().await.map_err(db_error)?;
    // New/shortened time ranges immediately revoke invalid member sessions.
    enforce(state).await?;
    state
        .admin_events
        .invalidate(&["rentals", "rental_requests"]);
    state.rental_changed.notify_one();
    Ok(rental_id)
}
async fn cancel(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let _gate = state.rental_gate.lock().await;
    let affected = sqlx::query(
        "UPDATE board_rentals SET cancelled_at=COALESCE(cancelled_at,now()) WHERE id=$1",
    )
    .bind(id)
    .execute(database(&state)?.pool())
    .await
    .map_err(db_error)?;
    if affected.rows_affected() == 0 {
        return Err(ApiError::not_found("租赁不存在"));
    }
    enforce(&state).await?;
    state.admin_events.invalidate(&["rentals"]);
    state.rental_changed.notify_one();
    Ok(StatusCode::NO_CONTENT)
}

/// Returns active, enabled-user allocations for a member at database time.
pub async fn active(
    state: &AppState,
    user: Uuid,
) -> Result<BTreeMap<String, (Uuid, DateTime<Utc>)>, ApiError> {
    let rows=sqlx::query("SELECT r.id,r.board_id,r.ends_at FROM board_rentals r JOIN users u ON r.user_id=u.id WHERE r.user_id=$1 AND NOT u.disabled AND r.cancelled_at IS NULL AND r.starts_at<=now() AND r.ends_at>now()")
        .bind(user).fetch_all(database(state)?.pool()).await.map_err(db_error)?;
    rows.iter()
        .map(|r| {
            Ok((
                r.try_get("board_id")?,
                (r.try_get("id")?, r.try_get("ends_at")?),
            ))
        })
        .collect::<Result<_, sqlx::Error>>()
        .map_err(db_error)
}

/// Hardware release uses the existing coordinator, including power-off and
/// serial cleanup. Failure to validate a public member's grant fails closed.
pub async fn enforce(state: &AppState) -> Result<(), ApiError> {
    let rows=sqlx::query("SELECT s.session_id,r.ends_at FROM session_owners s JOIN users u ON u.id=s.user_id JOIN board_rentals r ON r.id=s.rental_id WHERE NOT u.disabled AND r.cancelled_at IS NULL AND r.starts_at<=now() AND r.ends_at>now()")
        .fetch_all(database(state)?.pool()).await.map_err(db_error)?;
    let valid = rows
        .iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("session_id")?,
                row.try_get::<DateTime<Utc>, _>("ends_at")?,
            ))
        })
        .collect::<Result<BTreeMap<_, _>, sqlx::Error>>()
        .map_err(db_error)?;
    let sessions = state
        .sessions
        .read()
        .await
        .values()
        .cloned()
        .collect::<Vec<_>>();
    for session in sessions {
        if session.rental_deadline.read().await.is_none() {
            continue;
        }
        let snapshot = session.snapshot().await;
        if let Some(deadline) = valid.get(&snapshot.id) {
            session.bind_rental_deadline(*deadline).await;
        } else {
            session.request_stop(SessionStopReason::Expired);
        }
    }
    Ok(())
}

pub fn start_deadlines(state: AppState) {
    tokio::spawn(async move {
        loop {
            let next=sqlx::query_scalar::<_,Option<DateTime<Utc>>>("SELECT min(boundary) FROM (SELECT starts_at AS boundary FROM board_rentals WHERE cancelled_at IS NULL AND starts_at>now() UNION ALL SELECT ends_at FROM board_rentals WHERE cancelled_at IS NULL AND ends_at>now()) t")
                .fetch_one(state.database.as_ref().expect("public database").pool()).await;
            let delay = match next {
                Ok(Some(time)) => (time - Utc::now()).to_std().unwrap_or_default(),
                Ok(None) => std::time::Duration::from_secs(3600),
                Err(error) => {
                    log::warn!("rental deadline lookup failed: {error}");
                    std::time::Duration::from_secs(1)
                }
            };
            tokio::select! {_=tokio::time::sleep(delay)=>{},_=state.rental_changed.notified()=>{}}
            let _gate = state.rental_gate.lock().await;
            if let Err(error) = enforce(&state).await {
                log::warn!("rental enforcement failed: {}", error.message);
            }
            state.admin_events.invalidate(&["rentals"]);
        }
    });
}

pub async fn protect_board(state: &AppState, board: &str) -> Result<(), ApiError> {
    if !state.config.read().await.edition.is_public() {
        return Ok(());
    }
    let reserved:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM board_rentals WHERE board_id=$1 AND cancelled_at IS NULL AND ends_at>now())").bind(board).fetch_one(database(state)?.pool()).await.map_err(db_error)?;
    if reserved {
        return Err(ApiError::conflict("开发板有未结束的租赁，请先取消租赁"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn status_uses_half_open_intervals() {
        let start = Utc::now();
        let end = start + chrono::Duration::hours(1);
        assert_eq!(status(start, end, false, start), "active");
        assert_eq!(status(start, end, false, end), "expired");
        assert_eq!(status(start, end, true, start), "cancelled");
    }
}
