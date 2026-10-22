//! Durable hardware usage records; live connections remain owned by the controller.
use super::Database;
use crate::{config::BoardConfig, session::Session};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::{Postgres, QueryBuilder};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize, sqlx::FromRow)]
pub struct LeaseRecord {
    pub session_id: String,
    pub board_id: String,
    pub board_type: String,
    pub user_id: Option<Uuid>,
    pub username: Option<String>,
    pub rental_id: Option<Uuid>,
    pub client_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    pub stop_requested_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    pub stop_reason: Option<String>,
    pub end_reason: Option<String>,
    pub release_failures: i32,
    pub last_release_error: Option<String>,
}
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct LeaseFilters {
    pub q: Option<String>,
    pub board: Option<String>,
    pub state: Option<String>,
    pub reason: Option<String>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}
impl LeaseFilters {
    pub fn from_stream(query: &BTreeMap<String, String>) -> anyhow::Result<Self> {
        let mut values = serde_json::Map::new();
        for (key, value) in query {
            if let Some(key) = key.strip_prefix("lease_")
                && key != "view"
                && !value.is_empty()
            {
                let value = if matches!(key, "page" | "page_size") {
                    serde_json::json!(value.parse::<i64>()?)
                } else {
                    serde_json::Value::String(value.clone())
                };
                values.insert(key.into(), value);
            }
        }
        let filters: Self = serde_json::from_value(values.into())?;
        filters.validate()?;
        Ok(filters)
    }
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(self.page.is_none_or(|p| p >= 1), "页码必须大于零");
        anyhow::ensure!(
            self.page_size.is_none_or(|s| (1..=100).contains(&s)),
            "每页数量须为 1–100"
        );
        anyhow::ensure!(
            self.from.zip(self.to).is_none_or(|(a, b)| a < b),
            "起始时间必须早于截止时间"
        );
        anyhow::ensure!(
            [&self.q, &self.board, &self.reason]
                .iter()
                .all(|v| v.as_ref().is_none_or(|s| s.len() <= 256)),
            "筛选条件过长"
        );
        anyhow::ensure!(
            self.state
                .as_deref()
                .is_none_or(|s| matches!(s, "" | "active" | "releasing" | "ended")),
            "未知租约状态"
        );
        Ok(())
    }
    fn sql(&self, select: &str) -> QueryBuilder<'_, Postgres> {
        let mut query = QueryBuilder::new(format!("{select} FROM session_history WHERE true"));
        if let Some(q) = self.q.as_ref().filter(|q| !q.is_empty()) {
            // strpos treats %, _ and backslashes as ordinary search characters.
            query.push(" AND strpos(lower(concat_ws(' ',session_id,board_id,board_type,username,client_name)),lower(").push_bind(q).push("))>0");
        }
        if let Some(board) = self.board.as_ref().filter(|s| !s.is_empty()) {
            query.push(" AND board_id=").push_bind(board);
        }
        match self.state.as_deref() {
            Some("active") => {
                query.push(" AND ended_at IS NULL AND stop_requested_at IS NULL");
            }
            Some("releasing") => {
                query.push(" AND ended_at IS NULL AND stop_requested_at IS NOT NULL");
            }
            Some("ended") => {
                query.push(" AND ended_at IS NOT NULL");
            }
            _ => {}
        }
        if let Some(reason) = self.reason.as_ref().filter(|s| !s.is_empty()) {
            query.push(" AND end_reason=").push_bind(reason);
        }
        if let Some(from) = self.from {
            query.push(" AND created_at>=").push_bind(from);
        }
        if let Some(to) = self.to {
            query.push(" AND created_at<").push_bind(to);
        }
        query
    }
}
#[derive(Debug, Serialize)]
pub struct LeasePage {
    pub active: bool,
    pub query: LeaseFilters,
    pub items: Vec<LeaseRecord>,
    pub total: i64,
    pub page: i64,
    pub page_size: i64,
}
impl Database {
    pub async fn start_lease(&self, session: &Session, board: &BoardConfig) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO session_history(session_id,board_id,board_type,client_name,created_at,last_heartbeat_at,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7)")
            .bind(&session.id).bind(&session.board_id).bind(&board.board_type).bind(&session.client_name)
            .bind(session.created_at).bind(session.last_heartbeat_at).bind(session.expires_at)
            .execute(self.pool()).await?;
        Ok(())
    }
    pub async fn touch_lease(&self, session: &Session) -> anyhow::Result<()> {
        self.touch_leases(std::slice::from_ref(session)).await?;
        Ok(())
    }
    /// The existing lease watchdog checkpoints serial activity in one batch, away
    /// from the data plane. Unchanged heartbeats do not write rows.
    pub async fn touch_leases(&self, sessions: &[Session]) -> anyhow::Result<u64> {
        if sessions.is_empty() {
            return Ok(0);
        }
        let ids = sessions.iter().map(|s| s.id.clone()).collect::<Vec<_>>();
        let heartbeats = sessions
            .iter()
            .map(|s| s.last_heartbeat_at)
            .collect::<Vec<_>>();
        let deadlines = sessions.iter().map(|s| s.expires_at).collect::<Vec<_>>();
        let changed=sqlx::query("UPDATE session_history h SET last_heartbeat_at=v.heartbeat,expires_at=v.deadline FROM unnest($1::text[],$2::timestamptz[],$3::timestamptz[]) AS v(id,heartbeat,deadline) WHERE h.session_id=v.id AND h.ended_at IS NULL AND (h.last_heartbeat_at<v.heartbeat OR (h.last_heartbeat_at=v.heartbeat AND h.expires_at<>v.deadline))")
            .bind(ids).bind(heartbeats).bind(deadlines).execute(self.pool()).await?;
        Ok(changed.rows_affected())
    }
    pub async fn stopping_lease(&self, id: &str, reason: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE session_history SET stop_requested_at=COALESCE(stop_requested_at,now()),stop_reason=COALESCE(stop_reason,$2) WHERE session_id=$1 AND ended_at IS NULL")
            .bind(id).bind(reason).execute(self.pool()).await?;
        Ok(())
    }
    pub async fn lease_release_failed(&self, id: &str, error: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE session_history SET release_failures=release_failures+1,last_release_error=$2 WHERE session_id=$1 AND ended_at IS NULL")
            .bind(id).bind(error).execute(self.pool()).await?;
        Ok(())
    }
    pub async fn finish_lease(&self, id: &str, reason: &str) -> anyhow::Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("UPDATE session_history SET ended_at=GREATEST(now(),created_at),stop_requested_at=COALESCE(stop_requested_at,now()),stop_reason=COALESCE(stop_reason,$2),end_reason=$2 WHERE session_id=$1 AND ended_at IS NULL")
            .bind(id).bind(reason).execute(&mut *tx).await?;
        sqlx::query("DELETE FROM session_owners WHERE session_id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    /// One controller owns each inventory. On restart, former connections cannot be restored.
    /// ended_at is detection time; last_heartbeat_at is the last confirmed activity.
    pub async fn recover_interrupted_leases(&self) -> anyhow::Result<()> {
        let mut tx = self.pool().begin().await?;
        sqlx::query("UPDATE session_history SET ended_at=GREATEST(now(),created_at),end_reason='server_interrupted' WHERE ended_at IS NULL")
            .execute(&mut *tx).await?;
        sqlx::query("DELETE FROM session_owners")
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(())
    }
    pub async fn lease_page(&self, filters: &LeaseFilters) -> anyhow::Result<LeasePage> {
        filters.validate()?;
        let mut tx = self.pool().begin().await?;
        sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ, READ ONLY")
            .execute(&mut *tx)
            .await?;
        let total: i64 = filters
            .sql("SELECT count(*)")
            .build_query_scalar()
            .fetch_one(&mut *tx)
            .await?;
        let page_size = filters.page_size.unwrap_or(20);
        let page = filters
            .page
            .unwrap_or(1)
            .min(((total + page_size - 1) / page_size).max(1));
        let items = filters
            .sql("SELECT *")
            .push(" ORDER BY created_at DESC,session_id DESC LIMIT ")
            .push_bind(page_size)
            .push(" OFFSET ")
            .push_bind((page - 1) * page_size)
            .build_query_as()
            .fetch_all(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(LeasePage {
            active: true,
            query: filters.clone(),
            items,
            total,
            page,
            page_size,
        })
    }
}
