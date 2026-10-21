//! PostgreSQL authority shared by local and public deployments.
//! File adapters are retained for one-time imports and isolated test fixtures.
use std::time::Duration;

use anyhow::{Context, bail};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde_json::Value;
use sha2::{Digest, Sha256};
use sqlx::{PgPool, Row, postgres::PgPoolOptions};
use uuid::Uuid;

use crate::config::{AuthConfig, DatabaseConfig};

#[derive(Clone, Debug)]
pub struct Database {
    pool: PgPool,
}

#[derive(Debug, Clone)]
pub struct User {
    pub id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub is_admin: bool,
}

#[derive(Debug, Clone)]
pub struct AccessGrant {
    pub user: User,
    pub access_token: String,
    pub refresh_token: String,
    pub access_expires_at: DateTime<Utc>,
    pub refresh_expires_at: DateTime<Utc>,
    pub scope: String,
}

#[derive(Debug, Clone)]
pub enum DevicePoll {
    Pending,
    Authorized(AccessGrant),
    Denied,
    Expired,
}

#[derive(Debug, Clone)]
pub struct PersonalAccessToken {
    pub id: Uuid,
    pub token: String,
    pub name: String,
    pub expires_at: Option<DateTime<Utc>>,
}

impl Database {
    pub async fn record<T: serde::de::DeserializeOwned>(
        &self,
        name: &str,
    ) -> anyhow::Result<Option<T>> {
        let value: Option<Value> =
            sqlx::query_scalar("SELECT payload FROM server_records WHERE name=$1")
                .bind(name)
                .fetch_optional(&self.pool)
                .await?;
        value
            .map(serde_json::from_value)
            .transpose()
            .map_err(Into::into)
    }
    pub async fn import_record<T: serde::Serialize + serde::de::DeserializeOwned>(
        &self,
        name: &str,
        initial: &T,
    ) -> anyhow::Result<T> {
        sqlx::query(
            "INSERT INTO server_records (name,payload) VALUES ($1,$2) ON CONFLICT DO NOTHING",
        )
        .bind(name)
        .bind(serde_json::to_value(initial)?)
        .execute(&self.pool)
        .await?;
        self.record(name).await?.context("missing imported record")
    }
    pub async fn save_record<T: serde::Serialize + ?Sized>(
        &self,
        name: &str,
        value: &T,
    ) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO server_records (name,payload) VALUES ($1,$2) ON CONFLICT (name) DO UPDATE SET payload=excluded.payload,updated_at=now()")
            .bind(name).bind(serde_json::to_value(value)?).execute(&self.pool).await?;
        Ok(())
    }
    pub async fn connect(config: &DatabaseConfig) -> anyhow::Result<Option<Self>> {
        let Some(url) = config.url.as_deref().filter(|url| !url.trim().is_empty()) else {
            return Ok(None);
        };
        if let Some(schema) = &config.schema
            && (schema.is_empty()
                || schema.len() > 63
                || !schema
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_'))
        {
            bail!("database schema must contain 1..63 ASCII letters, digits or underscores");
        }
        let schema = config.schema.clone();
        let migrate = config.migrate;
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections.max(1))
            .acquire_timeout(Duration::from_secs(10))
            .after_connect(move |connection, _| {
                let schema = schema.clone();
                Box::pin(async move {
                    if let Some(schema) = schema {
                        if migrate {
                            sqlx::query(&format!("CREATE SCHEMA IF NOT EXISTS \"{schema}\""))
                                .execute(&mut *connection)
                                .await?;
                        }
                        sqlx::query("SELECT set_config('search_path',$1,false)")
                            .bind(format!("\"{schema}\",public"))
                            .execute(connection)
                            .await?;
                    }
                    Ok(())
                })
            })
            .connect(url)
            .await
            .context("failed to connect to PostgreSQL")?;
        if config.migrate {
            sqlx::migrate!("./migrations")
                .run(&pool)
                .await
                .context("failed to run PostgreSQL migrations")?;
        }
        Ok(Some(Self { pool }))
    }

    pub fn pool(&self) -> &PgPool {
        &self.pool
    }

    pub async fn bootstrap_admin(&self, auth: &AuthConfig) -> anyhow::Result<()> {
        let email = auth
            .bootstrap_admin_email
            .clone()
            .or_else(|| std::env::var("OSTOOL_ADMIN_EMAIL").ok());
        let password = auth
            .bootstrap_admin_password
            .clone()
            .or_else(|| std::env::var("OSTOOL_ADMIN_PASSWORD").ok());
        let (Some(email), Some(password)) = (email, password) else {
            return Ok(());
        };
        if password.len() < 12 || password.len() > 1024 {
            bail!("bootstrap admin password must contain at least 12 characters");
        }
        if self.find_user(&email).await?.is_none() {
            self.create_user(&email, &password, Some("Administrator"), true)
                .await?;
        }
        Ok(())
    }

    pub async fn create_user(
        &self,
        email: &str,
        password: &str,
        display_name: Option<&str>,
        is_admin: bool,
    ) -> anyhow::Result<User> {
        let email = normalize_email(email)?;
        if password.len() < 12 || password.len() > 1024 {
            bail!("password must contain at least 12 characters");
        }
        let password_hash = hash_password_async(password.to_owned()).await?;
        let row = sqlx::query(
            "INSERT INTO users (email, password_hash, display_name, is_admin, role_id) VALUES ($1,$2,$3,$4,CASE WHEN $4 THEN 'admin' ELSE 'member' END) RETURNING id,email,display_name,is_admin",
        )
        .bind(email)
        .bind(password_hash)
        .bind(display_name)
        .bind(is_admin)
        .fetch_one(&self.pool)
        .await
        .context("failed to create user")?;
        Ok(user_from_row(&row))
    }

    pub async fn find_user(&self, email: &str) -> anyhow::Result<Option<User>> {
        let email = normalize_email(email)?;
        let row = sqlx::query(
            "SELECT id,email,display_name,is_admin FROM users WHERE email=$1 AND disabled=false",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .context("failed to query user")?;
        Ok(row.as_ref().map(user_from_row))
    }

    pub async fn user_by_id(&self, id: Uuid) -> anyhow::Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id,email,display_name,is_admin FROM users WHERE id=$1 AND disabled=false",
        )
        .bind(id)
        .fetch_optional(&self.pool)
        .await
        .context("failed to query user")?;
        Ok(row.as_ref().map(user_from_row))
    }

    pub async fn authenticate_password(
        &self,
        email: &str,
        password: &str,
    ) -> anyhow::Result<Option<User>> {
        let email = normalize_email(email)?;
        let row = sqlx::query("SELECT id,email,display_name,is_admin,password_hash FROM users WHERE email=$1 AND disabled=false")
            .bind(email)
            .fetch_optional(&self.pool)
            .await
            .context("failed to query credentials")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let hash: String = row.try_get("password_hash")?;
        let password = password.to_owned();
        if !tokio::task::spawn_blocking(move || verify_password(&password, &hash)).await? {
            return Ok(None);
        }
        Ok(Some(user_from_row(&row)))
    }

    pub async fn issue_grant(
        &self,
        user: User,
        client_id: &str,
        scope: &str,
        auth: &AuthConfig,
    ) -> anyhow::Result<AccessGrant> {
        let access_token = random_token();
        let refresh_token = random_token();
        let now = Utc::now();
        let access_expires_at =
            now + chrono::Duration::seconds(auth.access_token_ttl_secs.max(60) as i64);
        let refresh_expires_at =
            now + chrono::Duration::seconds(auth.refresh_token_ttl_secs.max(300) as i64);
        sqlx::query("INSERT INTO oauth_sessions (user_id,client_id,scope,access_token_hash,refresh_token_hash,access_expires_at,refresh_expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(user.id).bind(client_id).bind(scope)
            .bind(token_hash(&access_token)).bind(token_hash(&refresh_token))
            .bind(access_expires_at).bind(refresh_expires_at)
            .execute(&self.pool).await.context("failed to create OAuth session")?;
        Ok(AccessGrant {
            user,
            access_token,
            refresh_token,
            access_expires_at,
            refresh_expires_at,
            scope: scope.to_string(),
        })
    }

    pub async fn authenticate_token(&self, token: &str) -> anyhow::Result<Option<User>> {
        let row = sqlx::query("SELECT u.id,u.email,u.display_name,u.is_admin FROM oauth_sessions s JOIN users u ON u.id=s.user_id WHERE s.access_token_hash=$1 AND s.revoked_at IS NULL AND s.access_expires_at > now() AND u.disabled=false UNION ALL SELECT u.id,u.email,u.display_name,u.is_admin FROM personal_access_tokens p JOIN users u ON u.id=p.user_id WHERE p.token_hash=$1 AND (p.expires_at IS NULL OR p.expires_at > now()) AND u.disabled=false LIMIT 1")
            .bind(token_hash(token)).fetch_optional(&self.pool).await.context("failed to validate access token")?;
        Ok(row.as_ref().map(user_from_row))
    }

    pub async fn create_personal_access_token(
        &self,
        user_id: Uuid,
        name: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> anyhow::Result<PersonalAccessToken> {
        let name = name.trim();
        if name.is_empty() || name.len() > 120 {
            bail!("personal access token name must contain 1 to 120 characters");
        }
        let token = format!("ostool_pat_{}", random_token());
        let row = sqlx::query("INSERT INTO personal_access_tokens (user_id,token_hash,name,expires_at) VALUES ($1,$2,$3,$4) RETURNING id")
            .bind(user_id)
            .bind(token_hash(&token))
            .bind(name)
            .bind(expires_at)
            .fetch_one(&self.pool)
            .await
            .context("failed to create personal access token")?;
        Ok(PersonalAccessToken {
            id: row.try_get("id")?,
            token,
            name: name.to_string(),
            expires_at,
        })
    }

    pub async fn revoke_personal_access_token(
        &self,
        user_id: Uuid,
        id: Uuid,
    ) -> anyhow::Result<bool> {
        let result = sqlx::query("DELETE FROM personal_access_tokens WHERE id=$1 AND user_id=$2")
            .bind(id)
            .bind(user_id)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn refresh(
        &self,
        token: &str,
        auth: &AuthConfig,
    ) -> anyhow::Result<Option<AccessGrant>> {
        let mut tx = self
            .pool
            .begin()
            .await
            .context("failed to start token refresh")?;
        let row = sqlx::query("SELECT s.id,s.user_id,s.scope,u.id as uid,u.email,u.display_name,u.is_admin FROM oauth_sessions s JOIN users u ON u.id=s.user_id WHERE s.refresh_token_hash=$1 AND s.revoked_at IS NULL AND s.refresh_expires_at > now() AND u.disabled=false FOR UPDATE")
            .bind(token_hash(token)).fetch_optional(&mut *tx).await.context("failed to query refresh token")?;
        let Some(row) = row else {
            return Ok(None);
        };
        let session_id: Uuid = row.try_get("id")?;
        let user = User {
            id: row.try_get("uid")?,
            email: row.try_get("email")?,
            display_name: row.try_get("display_name")?,
            is_admin: row.try_get("is_admin")?,
        };
        sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE id=$1")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        let scope: String = row.try_get("scope")?;
        let access_token = random_token();
        let refresh_token = random_token();
        let now = Utc::now();
        let access_expires_at =
            now + chrono::Duration::seconds(auth.access_token_ttl_secs.max(60) as i64);
        let refresh_expires_at =
            now + chrono::Duration::seconds(auth.refresh_token_ttl_secs.max(300) as i64);
        sqlx::query("INSERT INTO oauth_sessions (user_id,client_id,scope,access_token_hash,refresh_token_hash,access_expires_at,refresh_expires_at) SELECT user_id,client_id,scope,$1,$2,$3,$4 FROM oauth_sessions WHERE id=$5")
            .bind(token_hash(&access_token)).bind(token_hash(&refresh_token)).bind(access_expires_at).bind(refresh_expires_at).bind(session_id).execute(&mut *tx).await?;
        tx.commit().await?;
        Ok(Some(AccessGrant {
            user,
            access_token,
            refresh_token,
            access_expires_at,
            refresh_expires_at,
            scope,
        }))
    }

    pub async fn revoke_refresh(&self, token: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE refresh_token_hash=$1")
            .bind(token_hash(token))
            .execute(&self.pool)
            .await
            .context("failed to revoke OAuth session")?;
        Ok(())
    }

    pub async fn create_device_code(
        &self,
        client_id: &str,
        scope: &str,
        auth: &AuthConfig,
    ) -> anyhow::Result<(String, String, DateTime<Utc>, u64)> {
        let device_code = random_token();
        let user_code = format!(
            "{}-{}",
            &random_token()[..4].to_uppercase(),
            &random_token()[..4].to_uppercase()
        );
        let expires_at =
            Utc::now() + chrono::Duration::seconds(auth.device_code_ttl_secs.max(60) as i64);
        sqlx::query("INSERT INTO oauth_device_codes (device_code_hash,user_code,client_id,scope,expires_at,interval_secs) VALUES ($1,$2,$3,$4,$5,5)")
            .bind(token_hash(&device_code)).bind(&user_code).bind(client_id).bind(scope).bind(expires_at).execute(&self.pool).await.context("failed to create device code")?;
        Ok((device_code, user_code, expires_at, 5))
    }

    pub async fn authorize_device_code(
        &self,
        user_code: &str,
        user_id: Uuid,
    ) -> anyhow::Result<bool> {
        let result = sqlx::query("UPDATE oauth_device_codes SET user_id=$1, authorized_at=now() WHERE user_code=$2 AND authorized_at IS NULL AND denied_at IS NULL AND expires_at > now()")
            .bind(user_id).bind(user_code.trim().to_uppercase()).execute(&self.pool).await?;
        Ok(result.rows_affected() == 1)
    }

    pub async fn poll_device_code(
        &self,
        device_code: &str,
        auth: &AuthConfig,
    ) -> anyhow::Result<DevicePoll> {
        let row = sqlx::query("SELECT d.*,u.id as uid,u.email,u.display_name,u.is_admin FROM oauth_device_codes d LEFT JOIN users u ON u.id=d.user_id WHERE d.device_code_hash=$1")
            .bind(token_hash(device_code)).fetch_optional(&self.pool).await?;
        let Some(row) = row else {
            return Ok(DevicePoll::Expired);
        };
        let expires_at: DateTime<Utc> = row.try_get("expires_at")?;
        if expires_at <= Utc::now() {
            return Ok(DevicePoll::Expired);
        }
        if row
            .try_get::<Option<DateTime<Utc>>, _>("denied_at")?
            .is_some()
        {
            return Ok(DevicePoll::Denied);
        }
        if row
            .try_get::<Option<DateTime<Utc>>, _>("consumed_at")?
            .is_some()
        {
            return Ok(DevicePoll::Expired);
        }
        let Some(user_id) = row.try_get::<Option<Uuid>, _>("uid")? else {
            return Ok(DevicePoll::Pending);
        };
        let consumed = sqlx::query("UPDATE oauth_device_codes SET consumed_at=now() WHERE device_code_hash=$1 AND consumed_at IS NULL")
            .bind(token_hash(device_code))
            .execute(&self.pool)
            .await?;
        if consumed.rows_affected() != 1 {
            return Ok(DevicePoll::Expired);
        }
        let user = User {
            id: user_id,
            email: row.try_get("email")?,
            display_name: row.try_get("display_name")?,
            is_admin: row.try_get("is_admin")?,
        };
        let scope: String = row.try_get("scope")?;
        Ok(DevicePoll::Authorized(
            self.issue_grant(user, &row.try_get::<String, _>("client_id")?, &scope, auth)
                .await?,
        ))
    }

    pub async fn sync_board(&self, id: &str, payload: &Value) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO board_records (id,payload) VALUES ($1,$2) ON CONFLICT (id) DO UPDATE SET payload=excluded.payload,updated_at=now()")
            .bind(id).bind(payload).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn sync_dtb(
        &self,
        name: &str,
        size: u64,
        storage_path: &str,
        sha256: &str,
    ) -> anyhow::Result<()> {
        sqlx::query("INSERT INTO dtb_records (name,size,storage_path,sha256) VALUES ($1,$2,$3,$4) ON CONFLICT (name) DO UPDATE SET size=excluded.size,storage_path=excluded.storage_path,sha256=excluded.sha256,updated_at=now()")
            .bind(name).bind(size as i64).bind(storage_path).bind(sha256).execute(&self.pool).await?;
        Ok(())
    }

    pub async fn delete_board(&self, id: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM board_records WHERE id=$1")
            .bind(id)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    pub async fn delete_dtb(&self, name: &str) -> anyhow::Result<()> {
        sqlx::query("DELETE FROM dtb_records WHERE name=$1")
            .bind(name)
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}

fn user_from_row(row: &sqlx::postgres::PgRow) -> User {
    User {
        id: row.try_get("id").expect("user id"),
        email: row.try_get("email").expect("user email"),
        display_name: row.try_get("display_name").ok(),
        is_admin: row.try_get("is_admin").unwrap_or(false),
    }
}

pub(crate) fn normalize_email(email: &str) -> anyhow::Result<String> {
    let email = email.trim().to_lowercase();
    if email.len() > 320
        || email.chars().any(char::is_whitespace)
        || email.split('@').count() != 2
        || email.split('@').any(str::is_empty)
    {
        bail!("invalid email address");
    }
    Ok(email)
}

fn hash_password(password: &str) -> anyhow::Result<String> {
    let mut bytes = [0u8; 16];
    rand::rng().fill_bytes(&mut bytes);
    let salt =
        SaltString::encode_b64(&bytes).map_err(|error| anyhow::anyhow!(error.to_string()))?;
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|error| anyhow::anyhow!(error.to_string()))?
        .to_string())
}

fn verify_password(password: &str, hash: &str) -> bool {
    PasswordHash::new(hash)
        .map(|parsed| {
            Argon2::default()
                .verify_password(password.as_bytes(), &parsed)
                .is_ok()
        })
        .unwrap_or(false)
}

fn random_token() -> String {
    Uuid::new_v4().to_string().replace('-', "")
}
fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) async fn hash_password_async(password: String) -> anyhow::Result<String> {
    tokio::task::spawn_blocking(move || hash_password(&password)).await?
}

pub(crate) async fn verify_password_async(password: String, hash: String) -> anyhow::Result<bool> {
    tokio::task::spawn_blocking(move || verify_password(&password, &hash))
        .await
        .map_err(Into::into)
}
