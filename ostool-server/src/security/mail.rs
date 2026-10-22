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
async fn send(
    config: &SecurityConfig,
    email: &str,
    code: &str,
    ttl_secs: u32,
) -> Result<(), ApiError> {
    let host = config
        .smtp_host
        .as_deref()
        .ok_or_else(|| ApiError::service_unavailable("邮箱验证尚未配置"))?;
    let from = config
        .smtp_from
        .as_deref()
        .ok_or_else(|| ApiError::service_unavailable("邮箱验证尚未配置"))?;
    let message=Message::builder().from(from.parse().map_err(|_|ApiError::service_unavailable("邮件发送配置无效"))?).to(email.parse().map_err(|_|ApiError::bad_request("邮箱格式无效"))?).subject("ostool 账号安全验证码").body(format!("你的 ostool 验证码是：{code}\n验证码 {ttl_secs} 秒内有效，只能使用一次。请勿向任何人提供验证码。\n如果这不是你的操作，请忽略本邮件。" )).map_err(anyhow::Error::from)?;
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
    if !config.smtp_available() {
        return Err(ApiError::service_unavailable("邮箱验证尚未配置"));
    }
    if purpose == "email:reset" && !config.password_reset_enabled {
        return Err(ApiError::forbidden("密码找回已关闭"));
    }
    let registration = purpose == "email:register";
    if registration && !state.config.read().await.auth.allow_registration {
        return Err(ApiError::forbidden("注册已关闭"));
    }
    let policy = if registration {
        &config.registration_email_policy
    } else {
        &config.email_policy
    };
    let ttl_secs = if purpose == "email:reset" {
        config.password_reset_ttl_secs
    } else {
        policy.ttl_secs
    };
    let prefix = if registration {
        "registration-email"
    } else {
        "email"
    };
    store::limit(
        &state.security_cache,
        &format!("{prefix}-ip"),
        &client.ip,
        policy.ip_limit as i32,
        policy.window_secs as i64,
    )
    .await?;
    store::limit(
        &state.security_cache,
        &format!("{prefix}-resend"),
        email,
        1,
        policy.resend_secs as i64,
    )
    .await?;
    store::limit(
        &state.security_cache,
        &format!("{prefix}-hour"),
        email,
        policy.email_limit as i32,
        policy.window_secs as i64,
    )
    .await?;
    let code = format!("{:06}", rand::random_range(0..1_000_000));
    let subject = crypto::hash(email);
    let table = proof_table(purpose);
    let id = Uuid::new_v4();
    let mut tx = state.database.pool().begin().await?;
    sqlx::query("SELECT pg_advisory_xact_lock(29992027)")
        .execute(&mut *tx)
        .await?;
    let active:i64=sqlx::query_scalar("SELECT (SELECT count(*) FROM email_verification_tokens WHERE consumed_at IS NULL AND invalidated_at IS NULL AND expires_at>now())+(SELECT count(*) FROM password_reset_tokens WHERE consumed_at IS NULL AND invalidated_at IS NULL AND expires_at>now())")
        .fetch_one(&mut *tx).await?;
    if active >= 10000 {
        return Err(ApiError::service_unavailable("验证服务繁忙，请稍后重试"));
    }
    sqlx::query(&format!("UPDATE {table} SET invalidated_at=now() WHERE purpose=$1 AND email_hash=$2 AND consumed_at IS NULL AND invalidated_at IS NULL"))
        .bind(purpose).bind(&subject).execute(&mut *tx).await?;
    sqlx::query(&format!("INSERT INTO {table}(id,purpose,user_id,email_hash,code_hash,security_version,request_ip_hash,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,now()+make_interval(secs=>$8::double precision))"))
        .bind(id).bind(purpose).bind(user.as_ref().map(|u|u.id)).bind(&subject)
        .bind(crypto::proof_hash(purpose,id,&code)).bind(user.as_ref().map(|u|u.security_version))
        .bind(crypto::hash(&client.ip)).bind(ttl_secs as f64).execute(&mut *tx).await?;
    tx.commit().await?;
    if purpose != "email:reset" || user.is_some() {
        let delivery:Uuid=sqlx::query_scalar("INSERT INTO mail_deliveries(challenge_id,user_id,purpose,recipient_email,recipient_hash) VALUES($1,$2,$3,$4,$5) RETURNING id")
            .bind(id).bind(user.as_ref().map(|u|u.id)).bind(purpose).bind(email).bind(subject).fetch_one(state.database.pool()).await?;
        let result = send(&config, email, &code, ttl_secs).await;
        sqlx::query(
            "UPDATE mail_deliveries SET status=$2,error_code=$3,completed_at=now() WHERE id=$1",
        )
        .bind(delivery)
        .bind(if result.is_ok() { "sent" } else { "failed" })
        .bind(if result.is_ok() {
            None
        } else {
            Some("smtp_delivery_failed")
        })
        .execute(state.database.pool())
        .await?;
        if let Err(error) = result {
            sqlx::query(&format!(
                "UPDATE {table} SET invalidated_at=now() WHERE id=$1"
            ))
            .bind(id)
            .execute(state.database.pool())
            .await?;
            state.admin_events.invalidate(&["mail_deliveries"]);
            return Err(error);
        }
        sqlx::query(&format!("UPDATE {table} SET sent_at=now() WHERE id=$1"))
            .bind(id)
            .execute(state.database.pool())
            .await?;
    }
    state.admin_events.invalidate(&["mail_deliveries"]);
    Ok(Json(
        json!({"challenge_id":id,"expires_in":ttl_secs,"resend_after":policy.resend_secs,"message":"如该邮箱可以接收验证码，邮件将很快送达"}),
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
fn proof_table(purpose: &str) -> &'static str {
    if purpose == "email:reset" {
        "password_reset_tokens"
    } else {
        "email_verification_tokens"
    }
}
/// Each proof retains its lifecycle metadata; only code hashes are persisted.
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
    let config = state.config.read().await.security.clone();
    let max_attempts = if purpose == "email:register" {
        config.registration_email_policy.max_attempts
    } else {
        config.email_policy.max_attempts
    } as i32;
    let table = proof_table(purpose);
    let mut tx = state.database.pool().begin().await?;
    let row = sqlx::query(&format!("SELECT * FROM {table} WHERE id=$1 FOR UPDATE"))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let valid = row.try_get::<String, _>("purpose")? == purpose
        && row.try_get::<DateTime<Utc>, _>("expires_at")? > Utc::now()
        && row
            .try_get::<Option<DateTime<Utc>>, _>("consumed_at")?
            .is_none()
        && row
            .try_get::<Option<DateTime<Utc>>, _>("invalidated_at")?
            .is_none()
        && row.try_get::<i32, _>("attempts")? < max_attempts
        && email.is_none_or(|value| {
            row.try_get::<String, _>("email_hash")
                .is_ok_and(|h| h == crypto::hash(value))
        })
        && crypto::equal(
            &row.try_get::<String, _>("code_hash")?,
            &crypto::proof_hash(purpose, id, code),
        );
    sqlx::query(&format!("UPDATE {table} SET attempts=LEAST(attempts+1,5),consumed_at=CASE WHEN $2 THEN now() ELSE consumed_at END,invalidated_at=CASE WHEN NOT $2 AND attempts+1 >= $3 THEN COALESCE(invalidated_at,now()) ELSE invalidated_at END WHERE id=$1"))
        .bind(id).bind(valid).bind(max_attempts).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(if valid {
        Some((
            row.try_get("user_id")?,
            json!({"version":row.try_get::<Option<i64>,_>("security_version")?}),
        ))
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
    if !state.config.read().await.security.password_reset_enabled {
        return Err(ApiError::forbidden("密码找回已关闭"));
    }
    store::limit(&state.security_cache, "reset-confirm", &client.ip, 10, 900).await?;
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
    let (owner, payload) = consume(
        &state,
        input.challenge_id,
        "email:verify",
        Some(&user.email),
        input.code.trim(),
    )
    .await?
    .ok_or_else(|| ApiError::bad_request("验证码错误或已过期"))?;
    if owner != Some(user.id) || payload["version"].as_i64() != Some(user.security_version) {
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
