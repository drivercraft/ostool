//! Optional email verification/recovery. No SMTP is required for local or image-only deployments.
use super::{
    captcha::{self, HumanAction, HumanProof},
    config::{SecurityConfig, SmtpTls},
    crypto,
    store::{self, ClientContext},
};
use crate::{
    AppState,
    api::{auth::CurrentUser, error::ApiError},
};
use axum::{
    Json, Router,
    extract::{Extension, State},
    routing::post,
};
use chrono::{DateTime, Utc};
use lettre::{
    AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor,
    transport::smtp::authentication::Credentials,
};
use serde::Deserialize;
use serde_json::{Value, json};
use sqlx::Row;
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Purpose {
    Register,
    PasswordReset,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct EmailRequest {
    email: String,
    action: Purpose,
    verification: HumanProof,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ResetRequest {
    challenge_id: Uuid,
    code: String,
    new_password: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyEmail {
    challenge_id: Uuid,
    code: String,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/auth/email-code", post(request))
        .route("/api/v1/auth/password-reset", post(reset))
        .route("/api/v1/auth/security/email", post(send_verification))
        .route("/api/v1/auth/security/email/confirm", post(confirm_email))
}
async fn send(config: &SecurityConfig, email: &str, code: &str) -> Result<(), ApiError> {
    let host = config
        .smtp_host
        .as_deref()
        .ok_or_else(|| ApiError::service_unavailable("邮箱验证尚未配置"))?;
    let from = config
        .smtp_from
        .as_deref()
        .ok_or_else(|| ApiError::service_unavailable("邮箱验证尚未配置"))?;
    let message=Message::builder().from(from.parse().map_err(|_|ApiError::service_unavailable("邮件发送配置无效"))?).to(email.parse().map_err(|_|ApiError::bad_request("邮箱格式无效"))?).subject("ostool 账号安全验证码").body(format!("你的 ostool 验证码是：{code}\n验证码 10 分钟内有效，只能使用一次。请勿向任何人提供验证码。\n如果这不是你的操作，请忽略本邮件。" )).map_err(anyhow::Error::from)?;
    let mut builder = match config.smtp_tls {
        SmtpTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
            .map_err(anyhow::Error::from)?,
        SmtpTls::Tls => {
            AsyncSmtpTransport::<Tokio1Executor>::relay(host).map_err(anyhow::Error::from)?
        }
        SmtpTls::LoopbackPlaintext => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host),
    }
    .port(config.smtp_port)
    .timeout(Some(std::time::Duration::from_secs(10)));
    if let (Some(username), Some(password)) = (&config.smtp_username, &config.smtp_password) {
        builder = builder.credentials(Credentials::new(username.clone(), password.clone()));
    }
    builder.build().send(message).await.map_err(|error| {
        log::error!("SMTP delivery failed: {error}");
        ApiError::service_unavailable("验证码发送失败，请稍后重试")
    })?;
    Ok(())
}
async fn issue(
    state: &AppState,
    client: &ClientContext,
    email: &str,
    purpose: &str,
    user: Option<crate::persistence::postgres::User>,
) -> Result<Json<Value>, ApiError> {
    let config = state.config.read().await.security.clone();
    if config.smtp_host.is_none() || config.smtp_from.is_none() {
        return Err(ApiError::service_unavailable("邮箱验证尚未配置"));
    }
    store::limit(state.database.pool(), "email-ip", &client.ip, 8, 3600).await?;
    store::limit(state.database.pool(), "email-resend", email, 1, 60).await?;
    store::limit(state.database.pool(), "email-hour", email, 5, 3600).await?;
    let code = format!("{:06}", rand::random_range(0..1_000_000));
    let subject = crypto::hash(email);
    sqlx::query("DELETE FROM security_challenges WHERE purpose=$1 AND subject=$2")
        .bind(purpose)
        .bind(&subject)
        .execute(state.database.pool())
        .await?;
    let id = store::challenge(
        state.database.pool(),
        purpose,
        &subject,
        &code,
        json!({"version":user.as_ref().map(|u|u.security_version)}),
        user.as_ref().map(|u| u.id),
        600,
    )
    .await?;
    // Recovery requests never disclose whether the recipient has an enabled account.
    if (purpose != "email:reset" || user.is_some())
        && let Err(error) = send(&config, email, &code).await
    {
        sqlx::query("DELETE FROM security_challenges WHERE id=$1")
            .bind(id)
            .execute(state.database.pool())
            .await?;
        return Err(error);
    }
    Ok(Json(
        json!({"challenge_id":id,"expires_in":600,"resend_after":60,"message":"如该邮箱可以接收验证码，邮件将很快送达"}),
    ))
}
async fn request(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
    Json(input): Json<EmailRequest>,
) -> Result<Json<Value>, ApiError> {
    let action = match input.action {
        Purpose::Register => HumanAction::RegistrationEmail,
        Purpose::PasswordReset => HumanAction::PasswordReset,
    };
    captcha::verify(&state, &client, action, &input.verification).await?;
    let email = crate::persistence::postgres::normalize_email(&input.email)
        .map_err(|_| ApiError::bad_request("邮箱格式无效"))?;
    let user = state.database.find_user(&email).await?;
    issue(
        &state,
        &client,
        &email,
        match input.action {
            Purpose::Register => "email:register",
            Purpose::PasswordReset => "email:reset",
        },
        user,
    )
    .await
}
/// Unlike CAPTCHA, a mail proof allows five attempts, all serialized in PostgreSQL.
async fn consume(
    state: &AppState,
    id: Uuid,
    purpose: &str,
    email: Option<&str>,
    code: &str,
) -> Result<Option<(Option<Uuid>, Value)>, ApiError> {
    if code.len() != 6 || !code.bytes().all(|c| c.is_ascii_digit()) {
        return Ok(None);
    }
    let mut tx = state.database.pool().begin().await?;
    let row=sqlx::query("SELECT purpose,subject,secret_hash,user_id,payload,expires_at,attempts FROM security_challenges WHERE id=$1 FOR UPDATE").bind(id).fetch_optional(&mut *tx).await?;
    let Some(row) = row else { return Ok(None) };
    let valid = row.try_get::<String, _>("purpose")? == purpose
        && row.try_get::<DateTime<Utc>, _>("expires_at")? > Utc::now()
        && row.try_get::<i32, _>("attempts")? < 5
        && email.is_none_or(|email| {
            row.try_get::<String, _>("subject")
                .is_ok_and(|s| s == crypto::hash(email))
        })
        && crypto::equal(
            &row.try_get::<String, _>("secret_hash")?,
            &crypto::proof_hash(purpose, id, code),
        );
    if valid {
        sqlx::query("DELETE FROM security_challenges WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    } else {
        sqlx::query("UPDATE security_challenges SET attempts=LEAST(attempts+1,5) WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(if valid {
        Some((row.try_get("user_id")?, row.try_get("payload")?))
    } else {
        None
    })
}
pub async fn registration_proof(
    state: &AppState,
    email: &str,
    id: Option<Uuid>,
    code: Option<&str>,
) -> Result<bool, ApiError> {
    if !state
        .config
        .read()
        .await
        .security
        .registration_email_verification
    {
        return Ok(false);
    }
    if let (Some(id), Some(code)) = (id, code)
        && consume(state, id, "email:register", Some(email), code)
            .await?
            .is_some()
    {
        return Ok(true);
    }
    Err(ApiError::bad_request("邮箱验证码错误、尝试过多或已过期"))
}
async fn reset(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
    Json(input): Json<ResetRequest>,
) -> Result<Json<Value>, ApiError> {
    store::limit(state.database.pool(), "reset-confirm", &client.ip, 10, 900).await?;
    crypto::validate_password(&input.new_password)
        .map_err(|e| ApiError::bad_request(e.to_string()))?;
    let (user, payload) = consume(
        &state,
        input.challenge_id,
        "email:reset",
        None,
        input.code.trim(),
    )
    .await?
    .ok_or_else(|| ApiError::bad_request("验证码错误、已使用或已过期"))?;
    let user = user.ok_or_else(|| ApiError::bad_request("验证码错误、已使用或已过期"))?;
    let version = payload["version"]
        .as_i64()
        .ok_or_else(|| ApiError::bad_request("验证码已失效"))?;
    let hash = crate::persistence::postgres::hash_password_async(input.new_password).await?;
    let count = sqlx::query(
        "UPDATE users SET password_hash=$2 WHERE id=$1 AND NOT disabled AND security_version=$3",
    )
    .bind(user)
    .bind(hash)
    .bind(version)
    .execute(state.database.pool())
    .await?
    .rows_affected();
    if count != 1 {
        return Err(ApiError::bad_request("验证码已失效，请重新申请"));
    }
    Ok(Json(json!({"reset":true})))
}
async fn send_verification(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(client): Extension<ClientContext>,
) -> Result<Json<Value>, ApiError> {
    issue(
        &state,
        &client,
        &user.email,
        "email:verify",
        Some(user.clone()),
    )
    .await
}
async fn confirm_email(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(input): Json<VerifyEmail>,
) -> Result<Json<Value>, ApiError> {
    let (owner, _) = consume(
        &state,
        input.challenge_id,
        "email:verify",
        Some(&user.email),
        input.code.trim(),
    )
    .await?
    .ok_or_else(|| ApiError::bad_request("验证码错误或已过期"))?;
    if owner != Some(user.id) {
        return Err(ApiError::bad_request("验证码不匹配"));
    }
    let result = sqlx::query(
        "UPDATE users SET email_verified_at=now() WHERE id=$1 AND email=$2 AND NOT disabled",
    )
    .bind(user.id)
    .bind(user.email)
    .execute(state.database.pool())
    .await?;
    if result.rows_affected() != 1 {
        return Err(ApiError::bad_request("验证码已失效，请重新申请"));
    }
    Ok(Json(json!({"verified":true})))
}
