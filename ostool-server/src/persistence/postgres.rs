//! PostgreSQL authority shared by local and public deployments.
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
    pub security_version: i64,
}

#[derive(Debug, Clone)]
pub struct AccessGrant {
    pub session_id: Uuid,
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
    pub async fn initialize_record<T: serde::Serialize + serde::de::DeserializeOwned>(
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
        self.record(name)
            .await?
            .context("missing initialized record")
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
    pub async fn connect(config: &DatabaseConfig) -> anyhow::Result<Self> {
        crate::security::crypto::initialize_tls();
        let Some(url) = config.url.as_deref().filter(|url| !url.trim().is_empty()) else {
            bail!("PostgreSQL is required (OSTOOL_DATABASE_URL)");
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
        anyhow::ensure!(
            config.max_connections > 0,
            "database.max_connections must be greater than 0"
        );
        let schema = config.schema.clone();
        let migrate = config.migrate;
        let pool = PgPoolOptions::new()
            .max_connections(config.max_connections)
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
        Ok(Self { pool })
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
        let email = normalize_email(&email)?;
        let exists: bool = sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM users WHERE email=$1)")
            .bind(&email)
            .fetch_one(&self.pool)
            .await?;
        if !exists {
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
        crate::security::crypto::validate_password(password)?;
        let password_hash = hash_password_async(password.to_owned()).await?;
        let row = sqlx::query(
            "INSERT INTO users (email, password_hash, display_name, is_admin, role_id) VALUES ($1,$2,$3,$4,CASE WHEN $4 THEN 'admin' ELSE 'member' END) RETURNING id,email,display_name,is_admin,security_version",
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
            "SELECT id,email,display_name,is_admin,security_version FROM users WHERE email=$1 AND disabled=false",
        )
        .bind(email)
        .fetch_optional(&self.pool)
        .await
        .context("failed to query user")?;
        Ok(row.as_ref().map(user_from_row))
    }

    pub async fn user_by_id(&self, id: Uuid) -> anyhow::Result<Option<User>> {
        let row = sqlx::query(
            "SELECT id,email,display_name,is_admin,security_version FROM users WHERE id=$1 AND disabled=false",
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
        const DUMMY: &str = "$argon2id$v=19$m=19456,p=1,t=2$JQJyPZZI+mzpA7cCcrNBZg$vJ/LwJ+M2FACLdPhhSHhklaajIlQueOj55VYf7nL604";
        if password.len() > 1024 {
            return Ok(None);
        }
        let email = normalize_email(email).unwrap_or_default();
        let row=sqlx::query("SELECT id,email,display_name,is_admin,security_version,password_hash FROM users WHERE email=$1 AND NOT disabled").bind(email).fetch_optional(&self.pool).await?;
        let hash = row
            .as_ref()
            .map(|row| row.try_get::<String, _>("password_hash"))
            .transpose()?
            .unwrap_or_else(|| DUMMY.into());
        let valid = verify_password_async(password.to_owned(), hash).await?;
        Ok(if valid {
            row.as_ref().map(user_from_row)
        } else {
            None
        })
    }

    pub async fn issue_grant(
        &self,
        user: User,
        client_id: &str,
        scope: &str,
        auth: &AuthConfig,
    ) -> anyhow::Result<AccessGrant> {
        anyhow::ensure!(
            (1..=100).contains(&auth.max_active_sessions),
            "invalid session limit"
        );
        let access_token = random_token();
        let refresh_token = random_token();
        let now = Utc::now();
        let access_expires_at = now + chrono::Duration::seconds(auth.access_token_ttl_secs as i64);
        let refresh_expires_at =
            now + chrono::Duration::seconds(auth.refresh_token_ttl_secs as i64);
        let mut tx = self.pool.begin().await?;
        let exists: Option<Uuid> = sqlx::query_scalar(
            "SELECT id FROM users WHERE id=$1 AND security_version=$2 AND NOT disabled FOR UPDATE",
        )
        .bind(user.id)
        .bind(user.security_version)
        .fetch_optional(&mut *tx)
        .await?;
        anyhow::ensure!(
            exists.is_some(),
            "credentials changed during authentication"
        );
        sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE id IN (SELECT id FROM oauth_sessions WHERE user_id=$1 AND client_id=$2 AND revoked_at IS NULL AND refresh_expires_at>now() ORDER BY created_at DESC,id OFFSET $3)").bind(user.id).bind(client_id).bind((auth.max_active_sessions - 1) as i64).execute(&mut *tx).await?;
        let session_id:Uuid=sqlx::query_scalar("INSERT INTO oauth_sessions(user_id,client_id,scope,access_token_hash,refresh_token_hash,access_expires_at,refresh_expires_at,absolute_expires_at,security_version) VALUES($1,$2,$3,$4,$5,$6,$7,$7,$8) RETURNING id")
            .bind(user.id).bind(client_id).bind(scope).bind(token_hash(&access_token)).bind(token_hash(&refresh_token)).bind(access_expires_at).bind(refresh_expires_at).bind(user.security_version).fetch_one(&mut *tx).await?;
        tx.commit().await?;
        Ok(AccessGrant {
            session_id,
            user,
            access_token,
            refresh_token,
            access_expires_at,
            refresh_expires_at,
            scope: scope.to_owned(),
        })
    }
    pub async fn authenticate_token(&self, token: &str) -> anyhow::Result<Option<User>> {
        Ok(self
            .authenticate_credential(token)
            .await?
            .map(|credential| credential.user))
    }
    pub async fn authenticate_credential(&self, token: &str) -> anyhow::Result<Option<Credential>> {
        if token.len() > 256 {
            return Ok(None);
        }
        let row=sqlx::query("SELECT u.id,u.email,u.display_name,u.is_admin,u.security_version,s.id AS session_id,s.client_id,s.scope,s.step_up_until FROM oauth_sessions s JOIN users u ON u.id=s.user_id WHERE s.access_token_hash=$1 AND s.revoked_at IS NULL AND s.access_expires_at>now() AND s.absolute_expires_at>now() AND s.last_seen_at>now()-interval '1 day' AND s.security_version=u.security_version AND NOT u.disabled UNION ALL SELECT u.id,u.email,u.display_name,u.is_admin,u.security_version,NULL::uuid,'pat','board:operate',NULL::timestamptz FROM personal_access_tokens p JOIN users u ON u.id=p.user_id WHERE p.token_hash=$1 AND (p.expires_at IS NULL OR p.expires_at>now()) AND NOT u.disabled LIMIT 1")
            .bind(token_hash(token)).fetch_optional(&self.pool).await?;
        let Some(row) = row else { return Ok(None) };
        let session_id: Option<Uuid> = row.try_get("session_id")?;
        if let Some(id) = session_id {
            sqlx::query("UPDATE oauth_sessions SET last_seen_at=now() WHERE id=$1 AND last_seen_at<now()-interval '1 minute'").bind(id).execute(&self.pool).await?;
        }
        if session_id.is_none() {
            sqlx::query("UPDATE personal_access_tokens SET last_used_at=now() WHERE token_hash=$1 AND (last_used_at IS NULL OR last_used_at<now()-interval '1 minute')")
                .bind(token_hash(token)).execute(&self.pool).await?;
        }
        Ok(Some(Credential {
            user: user_from_row(&row),
            session_id,
            client_id: row.try_get("client_id")?,
            scope: row.try_get("scope")?,
            step_up_until: row.try_get("step_up_until")?,
            token_hash: token_hash(token),
        }))
    }

    pub async fn credential_active(&self, credential: &Credential) -> anyhow::Result<bool> {
        if let Some(session) = credential.session_id {
            // A rotated family keeps an established socket alive only while its
            // current access grant is valid. An unrefreshed expired token must not
            // become a day-long credential merely by holding a WebSocket open.
            Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM oauth_sessions s JOIN users u ON u.id=s.user_id WHERE s.family_id=(SELECT family_id FROM oauth_sessions WHERE id=$1) AND s.revoked_at IS NULL AND s.access_expires_at>now() AND s.absolute_expires_at>now() AND s.last_seen_at>now()-interval '1 day' AND s.security_version=u.security_version AND NOT u.disabled)").bind(session).fetch_one(&self.pool).await?)
        } else {
            Ok(sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM personal_access_tokens p JOIN users u ON u.id=p.user_id WHERE p.token_hash=$1 AND (p.expires_at IS NULL OR p.expires_at>now()) AND NOT u.disabled)").bind(&credential.token_hash).fetch_one(&self.pool).await?)
        }
    }
    pub async fn create_personal_access_token(
        &self,
        user_id: Uuid,
        name: &str,
        expires_at: Option<DateTime<Utc>>,
    ) -> anyhow::Result<PersonalAccessToken> {
        let expires_at =
            Some(expires_at.unwrap_or_else(|| Utc::now() + chrono::Duration::days(90)));
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
        self.refresh_for_client(token, auth, None).await
    }
    pub async fn refresh_for_client(
        &self,
        token: &str,
        auth: &AuthConfig,
        client: Option<&str>,
    ) -> anyhow::Result<Option<AccessGrant>> {
        let mut tx = self.pool.begin().await?;
        let owner: Option<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM oauth_sessions WHERE refresh_token_hash=$1")
                .bind(token_hash(token))
                .fetch_optional(&mut *tx)
                .await?;
        let Some(owner) = owner else { return Ok(None) };
        let alive: Option<Uuid> =
            sqlx::query_scalar("SELECT id FROM users WHERE id=$1 AND NOT disabled FOR UPDATE")
                .bind(owner)
                .fetch_optional(&mut *tx)
                .await?;
        if alive.is_none() {
            return Ok(None);
        }
        let row=sqlx::query("SELECT s.*,u.email,u.display_name,u.is_admin,u.security_version AS current_version FROM oauth_sessions s JOIN users u ON u.id=s.user_id WHERE s.refresh_token_hash=$1 AND NOT u.disabled FOR UPDATE OF s").bind(token_hash(token)).fetch_optional(&mut *tx).await?;
        let Some(row) = row else { return Ok(None) };
        if client.is_some_and(|client| {
            row.try_get::<String, _>("client_id")
                .is_ok_and(|value| value != client)
        }) {
            return Ok(None);
        }
        let family: Uuid = row.try_get("family_id")?;
        if row
            .try_get::<Option<DateTime<Utc>>, _>("revoked_at")?
            .is_some()
        {
            sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE family_id=$1 AND revoked_at IS NULL").bind(family).execute(&mut *tx).await?;
            tx.commit().await?;
            return Ok(None);
        }
        let now = Utc::now();
        if row.try_get::<DateTime<Utc>, _>("refresh_expires_at")? <= now
            || row.try_get::<DateTime<Utc>, _>("absolute_expires_at")? <= now
            || row.try_get::<DateTime<Utc>, _>("last_seen_at")? < now - chrono::Duration::days(1)
            || row.try_get::<i64, _>("security_version")?
                != row.try_get::<i64, _>("current_version")?
        {
            return Ok(None);
        }
        let user = User {
            id: row.try_get("user_id")?,
            email: row.try_get("email")?,
            display_name: row.try_get("display_name")?,
            is_admin: row.try_get("is_admin")?,
            security_version: row.try_get("current_version")?,
        };
        let scope: String = row.try_get("scope")?;
        let access_token = random_token();
        let refresh_token = random_token();
        let access_expires_at = now + chrono::Duration::seconds(auth.access_token_ttl_secs as i64);
        let refresh_expires_at: DateTime<Utc> = row.try_get("absolute_expires_at")?;
        sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE id=$1")
            .bind(row.try_get::<Uuid, _>("id")?)
            .execute(&mut *tx)
            .await?;
        let session_id:Uuid=sqlx::query_scalar("INSERT INTO oauth_sessions(user_id,client_id,scope,access_token_hash,refresh_token_hash,access_expires_at,refresh_expires_at,absolute_expires_at,family_id,security_version,ip,user_agent,step_up_until,created_at) SELECT user_id,client_id,scope,$1,$2,$3,absolute_expires_at,absolute_expires_at,family_id,security_version,ip,user_agent,step_up_until,created_at FROM oauth_sessions WHERE id=$4 RETURNING id")
            .bind(token_hash(&access_token)).bind(token_hash(&refresh_token)).bind(access_expires_at).bind(row.try_get::<Uuid,_>("id")?).fetch_one(&mut *tx).await?;
        sqlx::query("UPDATE oauth_sessions SET last_refreshed_at=now() WHERE id=$1")
            .bind(session_id)
            .execute(&mut *tx)
            .await?;
        tx.commit().await?;
        Ok(Some(AccessGrant {
            session_id,
            user,
            scope,
            access_token,
            refresh_token,
            access_expires_at,
            refresh_expires_at,
        }))
    }

    pub async fn revoke_refresh(&self, token: &str) -> anyhow::Result<()> {
        sqlx::query("UPDATE oauth_sessions SET revoked_at=now() WHERE family_id IN (SELECT family_id FROM oauth_sessions WHERE refresh_token_hash=$1)")
            .bind(token_hash(token))
            .execute(&self.pool)
            .await
            .context("failed to revoke OAuth session")?;
        Ok(())
    }
    pub async fn refresh_owner(&self, token: &str) -> anyhow::Result<Option<User>> {
        let id: Option<Uuid> =
            sqlx::query_scalar("SELECT user_id FROM oauth_sessions WHERE refresh_token_hash=$1")
                .bind(token_hash(token))
                .fetch_optional(&self.pool)
                .await?;
        match id {
            Some(id) => self.user_by_id(id).await,
            None => Ok(None),
        }
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
        let row = sqlx::query("SELECT d.*,u.id as uid,u.email,u.display_name,u.is_admin,u.security_version FROM oauth_device_codes d LEFT JOIN users u ON u.id=d.user_id WHERE d.device_code_hash=$1")
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
            security_version: row.try_get("security_version")?,
        };
        let scope: String = row.try_get("scope")?;
        Ok(DevicePoll::Authorized(
            self.issue_grant(user, &row.try_get::<String, _>("client_id")?, &scope, auth)
                .await?,
        ))
    }
}

fn user_from_row(row: &sqlx::postgres::PgRow) -> User {
    User {
        id: row.try_get("id").expect("user id"),
        email: row.try_get("email").expect("user email"),
        display_name: row.try_get("display_name").ok(),
        is_admin: row.try_get("is_admin").unwrap_or(false),
        security_version: row
            .try_get("security_version")
            .expect("user security version"),
    }
}

pub(crate) fn normalize_email(email: &str) -> anyhow::Result<String> {
    let email = email.trim().to_lowercase();
    if email.len() > 320
        || email.chars().any(char::is_whitespace)
        || email.split('@').count() != 2
        || email.split('@').any(str::is_empty)
        || email.parse::<lettre::Address>().is_err()
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
    crate::security::crypto::random_token()
}
fn token_hash(token: &str) -> String {
    Sha256::digest(token.as_bytes())
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) async fn hash_password_async(password: String) -> anyhow::Result<String> {
    crate::security::crypto::validate_password(&password)?;
    let permit = password_slot().await?;
    tokio::task::spawn_blocking(move || {
        // Keep the permit even if the HTTP request is cancelled while Argon2 runs.
        let _permit = permit;
        hash_password(&password)
    })
    .await?
}

pub(crate) async fn verify_password_async(password: String, hash: String) -> anyhow::Result<bool> {
    let permit = password_slot().await?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        verify_password(&password, &hash)
    })
    .await
    .map_err(Into::into)
}

#[derive(Clone, Debug)]
pub struct Credential {
    pub user: User,
    pub session_id: Option<Uuid>,
    pub client_id: String,
    pub scope: String,
    pub step_up_until: Option<DateTime<Utc>>,
    pub token_hash: String,
}
async fn password_slot() -> anyhow::Result<tokio::sync::SemaphorePermit<'static>> {
    static SLOTS: std::sync::LazyLock<tokio::sync::Semaphore> =
        std::sync::LazyLock::new(|| tokio::sync::Semaphore::new(4));
    Ok(tokio::time::timeout(std::time::Duration::from_secs(3), SLOTS.acquire()).await??)
}
