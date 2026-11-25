//! Automatic approval applies the same requested rental window and ownership checks.
use crate::{BoardConfig, api::error::ApiError, security::permissions::Access};
use chrono::{DateTime, Duration, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(crate) fn eligible(access: &Access, board: &BoardConfig) -> bool {
    access.allows("board.instant")
        && board.allow_instant_use
        && !board.disabled
        && (access.allocation_policy.board_types.is_empty()
            || access
                .allocation_policy
                .board_types
                .contains(&board.board_type))
}
/// The caller serializes user and board mutations and holds the transaction locks.
/// Compare the peak simultaneous reservations, rather than all intersecting rows:
/// two adjacent reservations must not count as two concurrent devices.
pub(crate) async fn approval_allowed(
    tx: &mut Transaction<'_, Postgres>,
    access: &Access,
    board: &BoardConfig,
    user: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<bool, ApiError> {
    let policy = &access.allocation_policy;
    let duration = end - start;
    if !eligible(access, board)
        || duration > Duration::minutes(i64::from(policy.max_duration_minutes))
        || duration < Duration::minutes(i64::from(policy.min_available_minutes))
    {
        return Ok(false);
    }
    let peak: i64 = sqlx::query_scalar("WITH windows AS (SELECT greatest(starts_at,$2) AS start,least(ends_at,$3) AS finish FROM board_rentals WHERE user_id=$1 AND cancelled_at IS NULL AND starts_at<$3 AND ends_at>$2),events AS (SELECT start AS at,1 AS delta FROM windows UNION ALL SELECT finish,-1 FROM windows),levels AS (SELECT sum(sum(delta)) OVER (ORDER BY at) AS concurrent FROM events GROUP BY at) SELECT coalesce(max(concurrent),0)::bigint FROM levels")
        .bind(user).bind(start).bind(end).fetch_one(&mut **tx).await?;
    Ok(peak < i64::from(policy.max_concurrent_sessions))
}
