use std::sync::Arc;

use anyhow::Context;
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::http::{HeaderMap, HeaderValue, header};
use chrono::{Duration, Utc};
use rand_core::OsRng;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

use crate::storage::{AuthSession, DynStorage, NewUser, Permission, Role, User, UserProfile};

pub const AUTH_COOKIE_NAME: &str = "ostool_server_admin";
const AUTH_SESSION_TTL: Duration = Duration::hours(8);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrentUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub nickname: Option<String>,
    pub avatar_url: Option<String>,
    pub email: String,
    pub phone: Option<String>,
    pub department: Option<String>,
    pub title: Option<String>,
    pub last_login_at: Option<chrono::DateTime<Utc>>,
    pub roles: Vec<Role>,
    pub permissions: Vec<Permission>,
}

#[derive(Clone)]
pub struct AuthService {
    storage: DynStorage,
}

impl AuthService {
    pub fn new(storage: DynStorage) -> Self {
        Self { storage }
    }

    pub async fn create_user(
        &self,
        username: String,
        display_name: String,
        email: String,
        password: String,
        profile: UserProfile,
        role_names: Vec<String>,
    ) -> anyhow::Result<User> {
        let password_hash = hash_password(&password)?;
        self.storage
            .create_user(NewUser {
                username,
                display_name,
                email,
                password_hash,
                profile,
                role_names,
            })
            .await
    }

    async fn current_user_from_user(&self, user: User) -> anyhow::Result<CurrentUser> {
        let roles = self.storage.user_roles(&user.id).await?;
        let permissions = self.storage.user_permissions(&user.id).await?;
        Ok(CurrentUser {
            id: user.id,
            username: user.username,
            display_name: user.display_name,
            nickname: user.nickname,
            avatar_url: user.avatar_url,
            email: user.email,
            phone: user.phone,
            department: user.department,
            title: user.title,
            last_login_at: user.last_login_at,
            roles,
            permissions,
        })
    }

    pub async fn login(
        &self,
        username: &str,
        password: &str,
    ) -> anyhow::Result<(CurrentUser, String)> {
        let Some(user) = self.storage.find_user_by_username(username).await? else {
            anyhow::bail!("invalid username or password");
        };
        if user.disabled {
            anyhow::bail!("user is disabled");
        }
        verify_password(password, &user.password_hash)?;
        self.storage
            .delete_expired_auth_sessions(Utc::now())
            .await?;
        let now = Utc::now();
        let token = Uuid::new_v4().to_string();
        let session = AuthSession {
            id: Uuid::new_v4().to_string(),
            user_id: user.id.clone(),
            token_hash: token_hash(&token),
            ip_address: None,
            user_agent: None,
            expires_at: now + AUTH_SESSION_TTL,
            last_seen_at: Some(now),
            revoked_at: None,
            created_at: now,
        };
        self.storage.create_auth_session(session).await?;
        self.storage.mark_user_login(&user.id, now).await?;
        Ok((self.current_user_from_user(user).await?, token))
    }

    pub async fn logout(&self, token: &str) -> anyhow::Result<()> {
        self.storage
            .delete_auth_session_by_token_hash(&token_hash(token))
            .await
    }

    pub async fn user_for_token(&self, token: &str) -> anyhow::Result<Option<CurrentUser>> {
        let Some(user) = self
            .storage
            .find_user_by_auth_token_hash(&token_hash(token), Utc::now())
            .await?
        else {
            return Ok(None);
        };
        if user.disabled {
            return Ok(None);
        }
        Ok(Some(self.current_user_from_user(user).await?))
    }

    pub async fn user_count(&self) -> anyhow::Result<i64> {
        self.storage.user_count().await
    }
}

pub fn hash_password(password: &str) -> anyhow::Result<String> {
    let salt = SaltString::generate(&mut OsRng);
    Ok(Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map_err(|err| anyhow::anyhow!("failed to hash password: {err}"))?
        .to_string())
}

fn verify_password(password: &str, hash: &str) -> anyhow::Result<()> {
    let parsed = PasswordHash::new(hash)
        .map_err(|err| anyhow::anyhow!("failed to parse password hash: {err}"))?;
    Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .map_err(|_| anyhow::anyhow!("invalid username or password"))
}

pub fn token_hash(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    format!("{:x}", hasher.finalize())
}

pub fn cookie_value(token: &str) -> String {
    format!("{AUTH_COOKIE_NAME}={token}; Path=/; HttpOnly; SameSite=Lax")
}

pub fn clear_cookie_value() -> &'static str {
    "ostool_server_admin=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0"
}

pub fn token_from_headers(headers: &HeaderMap) -> Option<String> {
    let cookie = headers.get(header::COOKIE)?.to_str().ok()?;
    cookie
        .split(';')
        .filter_map(|part| part.trim().split_once('='))
        .find_map(|(name, value)| {
            if name == AUTH_COOKIE_NAME {
                Some(value.to_string())
            } else {
                None
            }
        })
}

pub fn set_cookie_header(headers: &mut HeaderMap, value: impl AsRef<str>) -> anyhow::Result<()> {
    headers.insert(
        header::SET_COOKIE,
        HeaderValue::from_str(value.as_ref()).context("failed to build Set-Cookie header")?,
    );
    Ok(())
}

pub type SharedAuthService = Arc<AuthService>;
