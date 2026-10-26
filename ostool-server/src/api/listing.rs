//! Bounded database-backed management views shared by HTTP and SSE.
use super::error::ApiError;
use crate::AppState;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{Postgres, QueryBuilder};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct Filters {
    pub view: String,
    pub q: String,
    pub status: String,
    pub role: String,
    pub board: String,
    pub user: String,
    pub page: Option<i64>,
    pub page_size: Option<i64>,
}
impl Filters {
    pub fn from_stream(query: &BTreeMap<String, String>) -> Result<Option<Self>, ApiError> {
        if !query.contains_key("list_view") {
            return Ok(None);
        }
        let values = query
            .iter()
            .filter_map(|(key, value)| key.strip_prefix("list_").map(|key| (key, value.clone())))
            .collect::<BTreeMap<_, _>>();
        let values: serde_json::Map<String, Value> = values
            .into_iter()
            .map(|(key, value)| {
                let value = if matches!(key, "page" | "page_size") {
                    value
                        .parse::<i64>()
                        .map(Value::from)
                        .map_err(|_| ApiError::bad_request("分页参数无效"))?
                } else {
                    Value::String(value)
                };
                Ok((key.to_string(), value))
            })
            .collect::<Result<_, ApiError>>()?;
        let filters: Self = serde_json::from_value(values.into())
            .map_err(|_| ApiError::bad_request("列表参数无效"))?;
        filters.validate()?;
        Ok(Some(filters))
    }
    pub fn validate(&self) -> Result<(), ApiError> {
        if !matches!(self.view.as_str(), "users" | "rentals" | "rental_requests")
            || self.page.is_some_and(|p| !(1..=1_000_000).contains(&p))
            || self.page_size.is_some_and(|s| !(1..=100).contains(&s))
            || [&self.q, &self.status, &self.role, &self.board, &self.user]
                .iter()
                .any(|s| s.len() > 256)
        {
            return Err(ApiError::bad_request("列表筛选或分页参数无效"));
        }
        let statuses: &[&str] = match self.view.as_str() {
            "users" => &["", "true", "false"],
            "rentals" => &["", "active", "scheduled", "expired", "cancelled"],
            _ => &[
                "",
                "pending",
                "approved",
                "auto_approved",
                "rejected",
                "cancelled",
            ],
        };
        if !statuses.contains(&self.status.as_str()) {
            return Err(ApiError::bad_request("未知列表状态"));
        }
        Ok(())
    }
    pub fn affected(&self, data: &BTreeMap<String, Value>) -> bool {
        data.contains_key(&self.view) || data.contains_key("users")
    }
    fn sql(&self, select: &str) -> QueryBuilder<'_, Postgres> {
        let mut sql = QueryBuilder::new(select.to_owned());
        match self.view.as_str() {
            "users" => {
                sql.push(" FROM users u WHERE true");
                if !self.role.is_empty() {
                    sql.push(" AND u.role_id=").push_bind(&self.role);
                }
                if !self.status.is_empty() {
                    sql.push(" AND u.disabled=")
                        .push_bind(self.status == "true");
                }
                if !self.q.is_empty() {
                    sql.push(" AND strpos(lower(concat_ws(' ',u.username,u.email,u.display_name)),lower(").push_bind(&self.q).push("))>0");
                }
            }
            _ => {
                sql.push(if self.view == "rentals" {
                    " FROM board_rentals r"
                } else {
                    " FROM rental_requests r"
                });
                sql.push(" JOIN users u ON u.id=r.user_id WHERE true");
                if !self.user.is_empty() {
                    sql.push(" AND (r.user_id::text=").push_bind(&self.user).push(" OR strpos(lower(concat_ws(' ',u.username,u.email,u.display_name)),lower(").push_bind(&self.user).push("))>0)");
                }
                if !self.board.is_empty() {
                    sql.push(" AND r.board_id=").push_bind(&self.board);
                }
                if !self.q.is_empty() {
                    sql.push(" AND strpos(lower(concat_ws(' ',r.board_id,u.username,u.email,u.display_name,r.notes)),lower(").push_bind(&self.q).push("))>0");
                }
                if self.view == "rentals" {
                    sql.push(match self.status.as_str() {
                        "cancelled" => " AND r.cancelled_at IS NOT NULL",
                        "expired" => " AND r.cancelled_at IS NULL AND r.ends_at<=now()",
                        "scheduled" => " AND r.cancelled_at IS NULL AND r.starts_at>now()",
                        "active" => {
                            " AND r.cancelled_at IS NULL AND r.starts_at<=now() AND r.ends_at>now()"
                        }
                        _ => "",
                    });
                } else if !self.status.is_empty() {
                    sql.push(" AND r.status=").push_bind(&self.status);
                }
            }
        }
        sql
    }
}

pub async fn page(state: &AppState, filters: &Filters) -> Result<Value, ApiError> {
    scoped_page(state, filters, None).await
}
pub async fn scoped_page(
    state: &AppState,
    filters: &Filters,
    owner: Option<Uuid>,
) -> Result<Value, ApiError> {
    filters.validate()?;
    if owner.is_some() && filters.view == "users" {
        return Err(ApiError::forbidden("不能查询其他用户"));
    }
    let mut tx = state.database.pool().begin().await?;
    // Count and rows must describe the same snapshot when other users mutate data.
    sqlx::query("SET TRANSACTION ISOLATION LEVEL REPEATABLE READ READ ONLY")
        .execute(&mut *tx)
        .await?;
    sqlx::query("SET LOCAL statement_timeout='5s'")
        .execute(&mut *tx)
        .await?;
    let mut count = filters.sql("SELECT count(*)");
    if let Some(owner) = owner {
        count.push(" AND r.user_id=").push_bind(owner);
    }
    let total: i64 = count.build_query_scalar().fetch_one(&mut *tx).await?;
    let size = filters.page_size.unwrap_or(20);
    let page = filters
        .page
        .unwrap_or(1)
        .min(((total + size - 1) / size).max(1));
    let select = match filters.view.as_str() {
        "users" => {
            "SELECT u.id,u.username,u.email,u.display_name,u.role_id,u.disabled,u.created_at,u.updated_at,u.email_verified_at,u.last_login_at,u.last_login_ip,u.password_changed_at,EXISTS(SELECT 1 FROM mfa_credentials m WHERE m.user_id=u.id) AS mfa_enabled"
        }
        "rentals" => {
            "SELECT r.*,u.username AS user_username,u.email AS user_email,u.display_name AS user_display_name,CASE WHEN r.cancelled_at IS NOT NULL THEN 'cancelled' WHEN r.ends_at<=now() THEN 'expired' WHEN r.starts_at>now() THEN 'scheduled' ELSE 'active' END AS status"
        }
        _ => "SELECT r.*,u.email AS user_email",
    };
    let mut sql = filters.sql(&format!("SELECT to_jsonb(item) FROM ({select}"));
    if let Some(owner) = owner {
        sql.push(" AND r.user_id=").push_bind(owner);
    }
    sql.push(if filters.view == "users" {
        " ORDER BY lower(u.username),u.id"
    } else {
        " ORDER BY r.created_at DESC,r.id DESC"
    })
    .push(" LIMIT ")
    .push_bind(size)
    .push(" OFFSET ")
    .push_bind((page - 1) * size)
    .push(") item");
    let items: Vec<Value> = sql.build_query_scalar().fetch_all(&mut *tx).await?;
    tx.commit().await?;
    Ok(
        json!({"active":true,"query":filters,"items":items,"total":total,"page":page,"page_size":size}),
    )
}

#[derive(Clone, Default, Deserialize, Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct EditorFilters {
    pub id: Option<Uuid>,
    pub user: Option<Uuid>,
    pub q: String,
    pub board: String,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
}
impl EditorFilters {
    pub fn from_stream(query: &BTreeMap<String, String>) -> Result<Option<Self>, ApiError> {
        if !query.contains_key("editor_view") {
            return Ok(None);
        }
        let values = query
            .iter()
            .filter_map(|(key, value)| {
                key.strip_prefix("editor_")
                    .filter(|key| *key != "view")
                    .map(|key| (key, value.clone()))
            })
            .collect::<BTreeMap<_, _>>();
        let values: serde_json::Map<String, Value> = values
            .into_iter()
            .map(|(key, value)| (key.to_string(), Value::String(value)))
            .collect();
        let filters: Self = serde_json::from_value(values.into())
            .map_err(|_| ApiError::bad_request("日历参数无效"))?;
        filters.validate()?;
        Ok(Some(filters))
    }
    pub fn validate(&self) -> Result<(), ApiError> {
        if self.from.is_some() != self.to.is_some()
            || self.q.len() > 256
            || self.board.len() > 256
            || self
                .from
                .zip(self.to)
                .is_some_and(|(a, b)| a >= b || b - a > chrono::Duration::days(370))
        {
            return Err(ApiError::bad_request("日历最多查询一年的有效时间范围"));
        }
        Ok(())
    }
}
pub async fn editor(state: &AppState, filters: &EditorFilters) -> Result<Value, ApiError> {
    let user_rows: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',id,'username',username,'email',email,'display_name',display_name,'disabled',disabled) FROM users WHERE (NOT disabled AND strpos(lower(concat_ws(' ',username,email,display_name)),lower($1))>0) OR id=$2 ORDER BY (id=$2) DESC NULLS LAST,username LIMIT 51")
        .bind(&filters.q).bind(filters.user).fetch_all(state.database.pool()).await?;
    let rental: Option<Value> = sqlx::query_scalar("SELECT to_jsonb(item) FROM (SELECT r.*,CASE WHEN cancelled_at IS NOT NULL THEN 'cancelled' WHEN ends_at<=now() THEN 'expired' WHEN starts_at>now() THEN 'scheduled' ELSE 'active' END AS status FROM board_rentals r WHERE id=$1) item")
        .bind(filters.id).fetch_optional(state.database.pool()).await?;
    let from = filters.from.unwrap_or_else(Utc::now);
    let to = filters.to.unwrap_or(from + chrono::Duration::days(32));
    let bookings: Vec<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id',r.id,'board_id',r.board_id,'starts_at',r.starts_at,'ends_at',r.ends_at,'label',coalesce(u.display_name,u.username),'status','reserved') FROM board_rentals r JOIN users u ON u.id=r.user_id WHERE r.board_id=$1 AND r.cancelled_at IS NULL AND r.starts_at<$3 AND r.ends_at>$2 ORDER BY r.starts_at LIMIT 10001")
        .bind(&filters.board).bind(from).bind(to).fetch_all(state.database.pool()).await?;
    if bookings.len() > 10000 {
        return Err(ApiError::bad_request(
            "该日历范围的租赁过多，请缩小时间范围",
        ));
    }
    Ok(
        json!({"query":filters,"users":user_rows.iter().take(50).collect::<Vec<_>>(),"more_users":user_rows.len()>50,"rental":rental,"bookings":bookings}),
    )
}
