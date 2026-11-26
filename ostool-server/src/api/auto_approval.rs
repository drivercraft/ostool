//! Automatic approval applies the same requested rental window and ownership checks.
use crate::{BoardConfig, api::error::ApiError, security::permissions::Access};
use chrono::{DateTime, Duration, Utc};
use sqlx::{Postgres, Transaction};
use uuid::Uuid;

pub(crate) struct Decision {
    pub approved: bool,
    pub note: String,
}
impl Decision {
    fn manual(reason: impl AsRef<str>) -> Self {
        Self {
            approved: false,
            note: format!("{}，等待管理员审核", reason.as_ref()),
        }
    }
}
fn scope_reason(access: &Access, board: &BoardConfig) -> Option<&'static str> {
    if !access.allows("board.instant") {
        Some("申请已提交")
    } else if board.disabled {
        Some("设备已禁用")
    } else if !board.allow_instant_use {
        Some("该设备未开启允许自动审批")
    } else if !access.allocation_policy.board_types.is_empty()
        && !access
            .allocation_policy
            .board_types
            .contains(&board.board_type)
    {
        Some("设备板型不在当前角色的免审批范围")
    } else {
        None
    }
}
pub(crate) fn eligible(access: &Access, board: &BoardConfig) -> bool {
    scope_reason(access, board).is_none()
}
/// The caller serializes user and board mutations and holds the transaction locks.
/// Compare the peak simultaneous reservations, rather than all intersecting rows:
/// two adjacent reservations must not count as two concurrent devices.
pub(crate) async fn decide(
    tx: &mut Transaction<'_, Postgres>,
    access: &Access,
    board: &BoardConfig,
    user: Uuid,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Decision, ApiError> {
    let policy = &access.allocation_policy;
    let duration = end - start;
    if let Some(reason) = scope_reason(access, board) {
        return Ok(Decision::manual(reason));
    }
    if duration > Duration::minutes(i64::from(policy.max_duration_minutes)) {
        return Ok(Decision::manual(format!(
            "申请时长超过免审批上限（{} 分钟）",
            policy.max_duration_minutes
        )));
    }
    if duration < Duration::minutes(i64::from(policy.min_available_minutes)) {
        return Ok(Decision::manual(format!(
            "申请时长不足免审批下限（{} 分钟）",
            policy.min_available_minutes
        )));
    }
    let peak: i64 = sqlx::query_scalar("WITH windows AS (SELECT greatest(starts_at,$2) AS start,least(ends_at,$3) AS finish FROM board_rentals WHERE user_id=$1 AND cancelled_at IS NULL AND starts_at<$3 AND ends_at>$2),events AS (SELECT start AS at,1 AS delta FROM windows UNION ALL SELECT finish,-1 FROM windows),levels AS (SELECT sum(sum(delta)) OVER (ORDER BY at) AS concurrent FROM events GROUP BY at) SELECT coalesce(max(concurrent),0)::bigint FROM levels")
        .bind(user).bind(start).bind(end).fetch_one(&mut **tx).await?;
    if peak >= i64::from(policy.max_concurrent_sessions) {
        return Ok(Decision::manual(format!(
            "所选时段内已达到免审批并发上限（{} 台）",
            policy.max_concurrent_sessions
        )));
    }
    Ok(Decision {
        approved: true,
        note: "符合免审批策略，系统自动通过".into(),
    })
}
