//! Redis holds only transient risk counters. PostgreSQL remains the credential authority.
use crate::{ServerConfig, api::error::ApiError};
use anyhow::Context;
use redis::aio::{ConnectionManager, ConnectionManagerConfig};
#[derive(Clone)]
pub struct SecurityCache {
    connection: ConnectionManager,
    prefix: String,
}
impl SecurityCache {
    pub async fn connect(config: &ServerConfig) -> anyhow::Result<Self> {
        super::crypto::initialize_tls();
        let url = config
            .security
            .redis_url
            .as_deref()
            .context("public edition requires OSTOOL_REDIS_URL")?;
        let client = redis::Client::open(url)
            .map_err(|_| anyhow::anyhow!("invalid Redis connection settings"))?;
        let settings = ConnectionManagerConfig::new()
            .set_connection_timeout(Some(std::time::Duration::from_secs(3)))
            .set_response_timeout(Some(std::time::Duration::from_secs(3)))
            .set_number_of_retries(1);
        let connection = ConnectionManager::new_with_config(client, settings)
            .await
            .map_err(|_| anyhow::anyhow!("unable to connect to security Redis"))?;
        let database = url::Url::parse(
            config
                .database
                .url
                .as_deref()
                .context("database URL required")?,
        )?;
        let identity = format!(
            "{}:{}:{}:{}",
            database.host_str().unwrap_or(""),
            database.port().unwrap_or(5432),
            database.path(),
            config
                .database
                .schema
                .as_deref()
                .unwrap_or_else(|| database.query().unwrap_or("public"))
        );
        let result = Self {
            connection,
            prefix: format!(
                "{}:{}:",
                config.security.redis_prefix,
                &super::crypto::hash(&identity)[..24]
            ),
        };
        result.ping().await?;
        Ok(result)
    }
    pub async fn ping(&self) -> anyhow::Result<()> {
        let pong: String = redis::cmd("PING")
            .query_async(&mut self.connection.clone())
            .await?;
        anyhow::ensure!(pong == "PONG", "invalid Redis reply");
        Ok(())
    }
    fn key(&self, scope: &str, identity: &str) -> String {
        format!(
            "{}{}",
            self.prefix,
            super::crypto::hash(&format!("{scope}:{identity}"))
        )
    }
    pub async fn clear(&self, scope: &str, identity: &str) -> Result<(), ApiError> {
        let _: i64 = redis::cmd("DEL")
            .arg(self.key(scope, identity))
            .query_async(&mut self.connection.clone())
            .await
            .map_err(unavailable)?;
        Ok(())
    }
    pub async fn limit(
        &self,
        scope: &str,
        identity: &str,
        max: i32,
        seconds: i64,
    ) -> Result<(), ApiError> {
        // Never extend a window on rejected attempts. A command failure denies the operation.
        let (count, ttl): (i64, i64) = redis::Script::new(
            r#"
local n = tonumber(redis.call('GET',KEYS[1]) or '0')
if n < tonumber(ARGV[1])+1 then n=redis.call('INCR',KEYS[1]) end
local ttl=redis.call('TTL',KEYS[1])
if ttl < 0 then redis.call('EXPIRE',KEYS[1],ARGV[2]); ttl=tonumber(ARGV[2]) end
return {n,ttl}
"#,
        )
        .key(self.key(scope, identity))
        .arg(max)
        .arg(seconds)
        .invoke_async(&mut self.connection.clone())
        .await
        .map_err(unavailable)?;
        if count > i64::from(max) {
            return Err(ApiError::new(
                axum::http::StatusCode::TOO_MANY_REQUESTS,
                "rate_limited",
                "操作过于频繁，请稍后重试",
            )
            .with_retry_after(ttl.max(1) as u32));
        }
        Ok(())
    }
}
fn unavailable(error: redis::RedisError) -> ApiError {
    log::error!("security Redis operation failed: {:?}", error.kind());
    ApiError::service_unavailable("验证服务暂时不可用，请稍后重试")
}
