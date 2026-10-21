use super::crypto;
use crate::api::error::ApiError;
use axum::http::HeaderMap;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::{PgPool, Row};
use uuid::Uuid;

#[derive(Clone, Debug)]
pub struct ClientContext {
    pub ip: String,
    pub user_agent: String,
}
impl ClientContext {
    pub fn new(ip: String, headers: &HeaderMap) -> Self {
        Self {
            ip,
            user_agent: headers
                .get("user-agent")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .chars()
                .take(512)
                .collect(),
        }
    }
    pub fn binding(&self) -> String {
        crypto::hash(&serde_json::to_string(&(&self.ip, &self.user_agent)).expect("string tuple"))
    }
}
/// Redis scripts atomically set and increment TTL-bound distributed limits.
pub async fn limit(
    cache: &Option<super::cache::SecurityCache>,
    scope: &str,
    identity: &str,
    max: i32,
    seconds: i64,
) -> Result<(), ApiError> {
    let cache = cache
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("验证服务未配置"))?;
    cache.limit(scope, identity, max, seconds).await
}
pub async fn clear_limit(
    cache: &Option<super::cache::SecurityCache>,
    scope: &str,
    identity: &str,
) -> Result<(), ApiError> {
    cache
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("验证服务未配置"))?
        .clear(scope, identity)
        .await
}
pub async fn challenge(
    pool: &PgPool,
    purpose: &str,
    subject: &str,
    code: &str,
    payload: Value,
    user: Option<Uuid>,
    ttl: i64,
) -> Result<Uuid, ApiError> {
    let id = Uuid::new_v4();
    let mut tx = pool.begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(29992026)")
        .execute(&mut *tx)
        .await?;
    sqlx::query("DELETE FROM security_challenges WHERE expires_at<=now()")
        .execute(&mut *tx)
        .await?;
    // Security state is bounded; capacity exhaustion fails closed, never evicts active proofs.
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM security_challenges")
        .fetch_one(&mut *tx)
        .await?;
    if count >= 10000 {
        return Err(ApiError::service_unavailable("验证服务繁忙，请稍后重试"));
    }
    sqlx::query("INSERT INTO security_challenges(id,purpose,subject,secret_hash,user_id,payload,expires_at) VALUES($1,$2,$3,$4,$5,$6,now()+make_interval(secs=>$7::double precision))")
        .bind(id).bind(purpose).bind(subject).bind(crypto::proof_hash(purpose,id,code)).bind(user).bind(payload).bind(ttl as f64).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(id)
}
/// Every CAPTCHA attempt consumes its proof, including wrong action/IP/answer.
pub async fn consume(
    pool: &PgPool,
    id: Uuid,
    purpose: &str,
    subject: &str,
    code: &str,
) -> Result<Option<Value>, ApiError> {
    let row=sqlx::query("DELETE FROM security_challenges WHERE id=$1 RETURNING purpose,subject,secret_hash,payload,expires_at").bind(id).fetch_optional(pool).await?;
    let Some(row) = row else { return Ok(None) };
    let expires: DateTime<Utc> = row.try_get("expires_at")?;
    let valid = expires > Utc::now()
        && row.try_get::<String, _>("purpose")? == purpose
        && row.try_get::<String, _>("subject")? == subject
        && crypto::equal(
            &row.try_get::<String, _>("secret_hash")?,
            &crypto::proof_hash(purpose, id, code),
        );
    if valid {
        Ok(Some(row.try_get("payload")?))
    } else {
        Ok(None)
    }
}
