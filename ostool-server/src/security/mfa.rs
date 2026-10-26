use super::{
    crypto,
    store::{self, ClientContext},
    web::{self, CurrentCredential, Surface},
};
use crate::{
    AppState,
    api::{auth::CurrentUser, error::ApiError},
    persistence::postgres::User,
};
use axum::{
    Json, Router,
    extract::{Extension, State},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use base64::{Engine, engine::general_purpose::STANDARD};
use chrono::{DateTime, Utc};
use rand::RngCore;
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::{Postgres, Row, Transaction};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct MfaLogin {
    challenge_id: Uuid,
    code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Confirmation {
    challenge_id: Uuid,
    code: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StepUp {
    password: String,
    #[serde(default)]
    code: String,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/mfa/verify", post(complete_login))
        .route("/api/v1/auth/step-up", post(step_up))
        .route("/api/v1/auth/security", get(status))
        .route(
            "/api/v1/auth/security/events",
            get(crate::admin_events::subscribe_security),
        )
        .route("/api/v1/auth/security/enrollment", post(enroll))
        .route("/api/v1/auth/security/confirm", post(confirm))
        .route("/api/v1/auth/security/mfa", delete(disable))
        .route("/api/v1/auth/security/recovery-codes", post(recovery))
}
fn key(state: &crate::security::config::SecurityConfig) -> Result<&str, ApiError> {
    state
        .mfa_encryption_key
        .as_deref()
        .ok_or_else(|| ApiError::service_unavailable("双重验证尚未配置服务端加密密钥"))
}
fn invalid() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::UNAUTHORIZED,
        "mfa_invalid",
        "验证码或恢复码错误、已使用或已过期",
    )
}
async fn enabled(state: &AppState, user: Uuid) -> Result<bool, ApiError> {
    Ok(
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM mfa_credentials WHERE user_id=$1)")
            .bind(user)
            .fetch_one(state.database.pool())
            .await?,
    )
}
pub async fn begin_login(
    state: &AppState,
    user: User,
    surface: Surface,
    client: &ClientContext,
) -> Result<Response, ApiError> {
    let config = state.config.read().await.clone();
    if enabled(state, user.id).await? {
        key(&config.security)?;
        let id = store::challenge(
            state.database.pool(),
            "mfa-login",
            &client.binding(),
            "",
            json!({"version":user.security_version,"surface":surface}),
            Some(user.id),
            300,
        )
        .await?;
        return Ok((
            axum::http::StatusCode::ACCEPTED,
            Json(json!({"mfa_required":true,"challenge_id":id,"expires_in":300})),
        )
            .into_response());
    }
    let scope = if surface == Surface::Admin && config.security.require_admin_mfa {
        "mfa:enroll"
    } else {
        "board:operate offline_access"
    };
    let grant = state
        .database
        .issue_grant(user, surface.client(), scope, &config.auth)
        .await?;
    web::browser_grant(state, grant, surface, client).await
}
async fn verify_mfa(
    tx: &mut Transaction<'_, Postgres>,
    encryption: &str,
    user: Uuid,
    code: &str,
) -> Result<bool, ApiError> {
    let row=sqlx::query("SELECT secret_ciphertext,last_step,recovery_hashes FROM mfa_credentials WHERE user_id=$1 FOR UPDATE").bind(user).fetch_optional(&mut **tx).await?;
    let Some(row) = row else { return Ok(false) };
    let secret = crypto::open(
        encryption,
        &format!("mfa:{user}"),
        &row.try_get::<String, _>("secret_ciphertext")?,
    )
    .map_err(|error| {
        log::error!("MFA decryption failed: {error}");
        ApiError::service_unavailable("双重验证暂时不可用")
    })?;
    if let Some(step) = crypto::verify_totp(
        &secret,
        code,
        Utc::now().timestamp(),
        row.try_get("last_step")?,
    ) {
        sqlx::query("UPDATE mfa_credentials SET last_step=$2 WHERE user_id=$1")
            .bind(user)
            .bind(step)
            .execute(&mut **tx)
            .await?;
        return Ok(true);
    }
    let normalized = normalize_recovery(code);
    let hash = crypto::hash(&format!("recovery:{user}:{normalized}"));
    let mut hashes: Vec<String> = row.try_get("recovery_hashes")?;
    if let Some(index) = hashes.iter().position(|value| crypto::equal(value, &hash)) {
        hashes.remove(index);
        sqlx::query("UPDATE mfa_credentials SET recovery_hashes=$2 WHERE user_id=$1")
            .bind(user)
            .bind(hashes)
            .execute(&mut **tx)
            .await?;
        return Ok(true);
    }
    Ok(false)
}
async fn complete_login(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
    Json(input): Json<MfaLogin>,
) -> Result<Response, ApiError> {
    if input.code.len() > 128 {
        return Err(invalid());
    }
    store::limit(&state.security_cache, "mfa-ip", &client.ip, 12, 60).await?;
    let user: Option<Uuid> = sqlx::query_scalar(
        "SELECT user_id FROM security_challenges WHERE id=$1 AND purpose='mfa-login'",
    )
    .bind(input.challenge_id)
    .fetch_optional(state.database.pool())
    .await?
    .flatten();
    let user = user.ok_or_else(invalid)?;
    store::limit(&state.security_cache, "mfa-user", &user.to_string(), 6, 900).await?;
    let config = state.config.read().await.clone();
    let encryption = key(&config.security)?;
    let mut tx = state.database.pool().begin().await?;
    let version: Option<i64> = sqlx::query_scalar(
        "SELECT security_version FROM users WHERE id=$1 AND NOT disabled FOR UPDATE",
    )
    .bind(user)
    .fetch_optional(&mut *tx)
    .await?;
    let Some(version) = version else {
        return Err(invalid());
    };
    let challenge=sqlx::query("SELECT subject,payload,expires_at,attempts FROM security_challenges WHERE id=$1 AND purpose='mfa-login' FOR UPDATE").bind(input.challenge_id).fetch_optional(&mut *tx).await?.ok_or_else(invalid)?;
    let payload: Value = challenge.try_get("payload")?;
    let valid = challenge.try_get::<DateTime<Utc>, _>("expires_at")? > Utc::now()
        && challenge.try_get::<i32, _>("attempts")? < 5
        && challenge.try_get::<String, _>("subject")? == client.binding()
        && payload["version"].as_i64() == Some(version);
    sqlx::query("UPDATE security_challenges SET attempts=attempts+1 WHERE id=$1")
        .bind(input.challenge_id)
        .execute(&mut *tx)
        .await?;
    if !valid || !verify_mfa(&mut tx, encryption, user, input.code.trim()).await? {
        tx.commit().await?;
        return Err(invalid());
    }
    sqlx::query("DELETE FROM security_challenges WHERE id=$1")
        .bind(input.challenge_id)
        .execute(&mut *tx)
        .await?;
    let surface: Surface =
        serde_json::from_value(payload["surface"].clone()).map_err(|_| invalid())?;
    tx.commit().await?;
    store::clear_limit(&state.security_cache, "mfa-user", &user.to_string()).await?;
    let user = state
        .database
        .user_by_id(user)
        .await?
        .filter(|u| u.security_version == version)
        .ok_or_else(invalid)?;
    if surface == Surface::Admin
        && !crate::security::permissions::access(&state.database, &user)
            .await?
            .allows("console.access")
    {
        return Err(ApiError::forbidden("后台访问权限已撤销"));
    }
    let grant = state
        .database
        .issue_grant(
            user,
            surface.client(),
            "board:operate offline_access",
            &config.auth,
        )
        .await?;
    web::browser_grant(&state, grant, surface, &client).await
}
async fn step_up(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentCredential(credential)): Extension<CurrentCredential>,
    Extension(client): Extension<ClientContext>,
    Json(input): Json<StepUp>,
) -> Result<Response, ApiError> {
    store::limit(
        &state.security_cache,
        "step-up",
        &format!("{}:{}", user.id, client.ip),
        6,
        900,
    )
    .await?;
    let id = credential
        .session_id
        .ok_or_else(|| ApiError::forbidden("请使用浏览器登录会话验证身份"))?;
    let authenticated = state
        .database
        .authenticate_password(&user.email, &input.password)
        .await?
        .filter(|u| u.security_version == user.security_version);
    if authenticated.is_none() {
        return Err(ApiError::unauthorized("身份验证失败"));
    }
    let config = state.config.read().await.security.clone();
    let mut tx = state.database.pool().begin().await?;
    sqlx::query(
        "SELECT id FROM users WHERE id=$1 AND security_version=$2 AND NOT disabled FOR UPDATE",
    )
    .bind(user.id)
    .bind(user.security_version)
    .fetch_one(&mut *tx)
    .await?;
    let mfa_enabled: bool =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM mfa_credentials WHERE user_id=$1)")
            .bind(user.id)
            .fetch_one(&mut *tx)
            .await?;
    if mfa_enabled && !verify_mfa(&mut tx, key(&config)?, user.id, input.code.trim()).await? {
        return Err(invalid());
    }
    let count=sqlx::query("UPDATE oauth_sessions SET step_up_until=now()+interval '3 minutes' WHERE id=$1 AND user_id=$2 AND security_version=$3 AND revoked_at IS NULL AND access_expires_at>now()").bind(id).bind(user.id).bind(user.security_version).execute(&mut *tx).await?.rows_affected();
    tx.commit().await?;
    if count != 1 {
        return Err(ApiError::unauthorized("登录已失效"));
    }
    Ok(Json(json!({"verified":true,"expires_in":180})).into_response())
}
async fn status(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> Result<Json<Value>, ApiError> {
    Ok(Json(status_projection(&state, user.id).await?))
}
pub async fn status_projection(state: &AppState, user: Uuid) -> Result<Value, ApiError> {
    let row=sqlx::query("SELECT (SELECT cardinality(recovery_hashes) FROM mfa_credentials WHERE user_id=$1) AS recovery_count,(SELECT email_verified_at IS NOT NULL FROM users WHERE id=$1) AS email_verified").bind(user).fetch_one(state.database.pool()).await?;
    let count: Option<i32> = row.try_get("recovery_count")?;
    let config = state.config.read().await.security.clone();
    Ok(
        json!({"enabled":count.is_some(),"available":config.mfa_encryption_key.is_some(),"recovery_codes_remaining":count.unwrap_or(0),"email_verified":row.try_get::<bool,_>("email_verified")?,"email_verification_available":config.smtp_available(),"require_admin_mfa":config.require_admin_mfa}),
    )
}
async fn enroll(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentCredential(credential)): Extension<CurrentCredential>,
    Extension(client): Extension<ClientContext>,
) -> Result<Json<Value>, ApiError> {
    web::require_step_up(&credential)?;
    if enabled(&state, user.id).await? {
        return Err(ApiError::conflict("双重验证已经启用"));
    }
    let config = state.config.read().await.security.clone();
    let encryption = key(&config)?;
    let mut secret = [0u8; 20];
    rand::rng().fill_bytes(&mut secret);
    let encoded = data_encoding::BASE32_NOPAD.encode(&secret);
    let mut uri = url::Url::parse("otpauth://totp/").expect("fixed URI");
    uri.path_segments_mut()
        .expect("totp path")
        .push(&format!("ostool:{}", user.email));
    uri.query_pairs_mut()
        .append_pair("secret", &encoded)
        .append_pair("issuer", "ostool")
        .append_pair("algorithm", "SHA1")
        .append_pair("digits", "6")
        .append_pair("period", "30");
    let ciphertext = crypto::seal(encryption, &format!("mfa:{}", user.id), &secret)?;
    sqlx::query("DELETE FROM security_challenges WHERE user_id=$1 AND purpose='mfa-enroll'")
        .bind(user.id)
        .execute(state.database.pool())
        .await?;
    let subject = format!(
        "{}:{}",
        credential.session_id.ok_or_else(invalid)?,
        client.binding()
    );
    let id = store::challenge(
        state.database.pool(),
        "mfa-enroll",
        &subject,
        "",
        json!({"secret":ciphertext,"version":user.security_version}),
        Some(user.id),
        600,
    )
    .await?;
    let svg = qrcode::QrCode::new(uri.as_str())
        .map_err(anyhow::Error::from)?
        .render::<qrcode::render::svg::Color>()
        .min_dimensions(180, 180)
        .build();
    Ok(Json(
        json!({"challenge_id":id,"secret":encoded,"otpauth_uri":uri,"qr_image":format!("data:image/svg+xml;base64,{}",STANDARD.encode(svg)),"expires_in":600}),
    ))
}
fn normalize_recovery(value: &str) -> String {
    value
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '-')
        .collect::<String>()
        .to_ascii_uppercase()
}
fn recovery_codes(user: Uuid) -> (Vec<String>, Vec<String>) {
    let codes: Vec<String> = (0..8)
        .map(|_| {
            let mut bytes = [0u8; 10];
            rand::rng().fill_bytes(&mut bytes);
            let hex = bytes.iter().map(|b| format!("{b:02X}")).collect::<String>();
            hex.as_bytes()
                .chunks(4)
                .map(|c| std::str::from_utf8(c).expect("hex"))
                .collect::<Vec<_>>()
                .join("-")
        })
        .collect();
    let hashes = codes
        .iter()
        .map(|code| crypto::hash(&format!("recovery:{user}:{}", normalize_recovery(code))))
        .collect();
    (codes, hashes)
}
async fn confirm(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentCredential(credential)): Extension<CurrentCredential>,
    Extension(client): Extension<ClientContext>,
    Json(input): Json<Confirmation>,
) -> Result<Json<Value>, ApiError> {
    web::require_step_up(&credential)?;
    store::limit(
        &state.security_cache,
        "mfa-enroll",
        &user.id.to_string(),
        6,
        900,
    )
    .await?;
    let config = state.config.read().await.security.clone();
    let encryption = key(&config)?;
    let mut tx = state.database.pool().begin().await?;
    let version: i64 = sqlx::query_scalar(
        "SELECT security_version FROM users WHERE id=$1 AND NOT disabled FOR UPDATE",
    )
    .bind(user.id)
    .fetch_one(&mut *tx)
    .await?;
    let challenge=sqlx::query("SELECT subject,payload,expires_at,attempts FROM security_challenges WHERE id=$1 AND purpose='mfa-enroll' AND user_id=$2 FOR UPDATE").bind(input.challenge_id).bind(user.id).fetch_optional(&mut *tx).await?.ok_or_else(invalid)?;
    let payload: Value = challenge.try_get("payload")?;
    let subject = format!(
        "{}:{}",
        credential.session_id.ok_or_else(invalid)?,
        client.binding()
    );
    let valid = challenge.try_get::<DateTime<Utc>, _>("expires_at")? > Utc::now()
        && challenge.try_get::<i32, _>("attempts")? < 5
        && challenge.try_get::<String, _>("subject")? == subject
        && payload["version"].as_i64() == Some(version);
    sqlx::query("UPDATE security_challenges SET attempts=attempts+1 WHERE id=$1")
        .bind(input.challenge_id)
        .execute(&mut *tx)
        .await?;
    let ciphertext = payload["secret"].as_str().ok_or_else(invalid)?;
    let secret = crypto::open(encryption, &format!("mfa:{}", user.id), ciphertext)?;
    let step = crypto::verify_totp(&secret, input.code.trim(), Utc::now().timestamp(), -1);
    if !valid || step.is_none() {
        tx.commit().await?;
        return Err(invalid());
    }
    let (codes, hashes) = recovery_codes(user.id);
    sqlx::query("INSERT INTO mfa_credentials(user_id,secret_ciphertext,last_step,recovery_hashes) VALUES($1,$2,$3,$4)").bind(user.id).bind(ciphertext).bind(step).bind(hashes).execute(&mut *tx).await?;
    let current = credential.session_id.ok_or_else(invalid)?;
    let alive: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM oauth_sessions WHERE id=$1 AND revoked_at IS NULL)",
    )
    .bind(current)
    .fetch_one(&mut *tx)
    .await?;
    if !alive {
        return Err(ApiError::unauthorized("登录会话已变化，请重新尝试"));
    }
    sqlx::query("UPDATE users SET security_version=security_version+1 WHERE id=$1")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let current = credential.session_id.ok_or_else(invalid)?;
    sqlx::query("UPDATE oauth_sessions SET revoked_at=NULL,security_version=(SELECT security_version FROM users WHERE id=$2),scope='board:operate offline_access',step_up_until=now()+interval '3 minutes' WHERE id=$1 AND user_id=$2")
        .bind(current).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"recovery_codes":codes,"reauthenticate":false})))
}
async fn disable(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentCredential(credential)): Extension<CurrentCredential>,
) -> Result<Response, ApiError> {
    web::require_step_up(&credential)?;
    let mut tx = state.database.pool().begin().await?;
    sqlx::query(
        "SELECT id FROM users WHERE id=$1 AND security_version=$2 AND NOT disabled FOR UPDATE",
    )
    .bind(user.id)
    .bind(user.security_version)
    .fetch_one(&mut *tx)
    .await?;
    let config = state.config.read().await.security.clone();
    if crate::security::permissions::access(&state.database, &user)
        .await?
        .allows("console.access")
        && config.require_admin_mfa
    {
        return Err(ApiError::forbidden("管理员必须保留双重验证"));
    }
    sqlx::query("DELETE FROM mfa_credentials WHERE user_id=$1")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let current = credential.session_id.ok_or_else(invalid)?;
    let alive: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM oauth_sessions WHERE id=$1 AND revoked_at IS NULL)",
    )
    .bind(current)
    .fetch_one(&mut *tx)
    .await?;
    if !alive {
        return Err(ApiError::unauthorized("登录会话已变化，请重新尝试"));
    }
    sqlx::query("UPDATE users SET security_version=security_version+1 WHERE id=$1")
        .bind(user.id)
        .execute(&mut *tx)
        .await?;
    let current = credential.session_id.ok_or_else(invalid)?;
    sqlx::query("UPDATE oauth_sessions SET revoked_at=NULL,security_version=(SELECT security_version FROM users WHERE id=$2),step_up_until=now()+interval '3 minutes' WHERE id=$1 AND user_id=$2")
        .bind(current).bind(user.id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Json(json!({"reauthenticate":false})).into_response())
}
async fn recovery(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(CurrentCredential(credential)): Extension<CurrentCredential>,
) -> Result<Json<Value>, ApiError> {
    web::require_step_up(&credential)?;
    let mut tx = state.database.pool().begin().await?;
    sqlx::query(
        "SELECT id FROM users WHERE id=$1 AND security_version=$2 AND NOT disabled FOR UPDATE",
    )
    .bind(user.id)
    .bind(user.security_version)
    .fetch_one(&mut *tx)
    .await?;
    let (codes, hashes) = recovery_codes(user.id);
    let count = sqlx::query("UPDATE mfa_credentials SET recovery_hashes=$2 WHERE user_id=$1")
        .bind(user.id)
        .bind(hashes)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if count != 1 {
        return Err(ApiError::conflict("请先启用双重验证"));
    }
    tx.commit().await?;
    Ok(Json(json!({"recovery_codes":codes})))
}
