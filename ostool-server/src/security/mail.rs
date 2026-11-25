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

// Debug previews are process-local only. PostgreSQL hashes remain the credential authority.
#[derive(Default)]
pub(crate) struct DevRegistrationCodes(
    tokio::sync::Mutex<std::collections::BTreeMap<Uuid, DevCode>>,
);
struct DevCode {
    code: String,
    expires_at: DateTime<Utc>,
}
impl DevRegistrationCodes {
    async fn insert(&self, id: Uuid, code: String, expires_at: DateTime<Utc>) {
        let mut codes = self.0.lock().await;
        codes.retain(|_, value| value.expires_at > Utc::now());
        if codes.len() >= 10000 {
            // Evict only a display copy; this never alters any authoritative proof.
            if let Some(oldest) = codes
                .iter()
                .min_by_key(|(_, value)| value.expires_at)
                .map(|(id, _)| *id)
            {
                codes.remove(&oldest);
            }
        }
        codes.insert(id, DevCode { code, expires_at });
    }
    pub(crate) async fn snapshot(&self) -> std::collections::BTreeMap<Uuid, String> {
        let mut codes = self.0.lock().await;
        codes.retain(|_, value| value.expires_at > Utc::now());
        codes
            .iter()
            .map(|(id, value)| (*id, value.code.clone()))
            .collect()
    }
    pub(crate) async fn remove(&self, id: Uuid) {
        self.0.lock().await.remove(&id);
    }
}

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
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VerifyRegistrationEmail {
    email: String,
    challenge_id: Uuid,
    code: String,
}
pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/admin/account-settings/mail/test",
            post(test_connection),
        )
        .route("/api/v1/auth/email-code", post(request))
        .route(
            "/api/v1/auth/registration-email/verify",
            post(verify_registration_email),
        )
        .route("/api/v1/auth/password-reset", post(reset))
        .route("/api/v1/auth/security/email", post(send_verification))
        .route("/api/v1/auth/security/email/confirm", post(confirm_email))
}
fn transport(config: &SecurityConfig) -> Result<AsyncSmtpTransport<Tokio1Executor>, ApiError> {
    let (host, port, tls) = config.mail_endpoint();
    let mut builder = match tls {
        SmtpTls::Starttls => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&host)
            .map_err(|_| ApiError::bad_request("SMTP 主机或 TLS 配置无效"))?,
        SmtpTls::Tls => AsyncSmtpTransport::<Tokio1Executor>::relay(&host)
            .map_err(|_| ApiError::bad_request("SMTP 主机或 TLS 配置无效"))?,
        SmtpTls::LoopbackPlaintext => {
            AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&host)
        }
    }
    .port(port)
    .timeout(Some(std::time::Duration::from_secs(10)));
    if let (Some(username), Some(password)) =
        (config.credentials().username, config.credentials().password)
    {
        builder = builder.credentials(Credentials::new(username, password));
    }
    Ok(builder.build())
}
pub async fn test_connection(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(client): Extension<ClientContext>,
    Json(input): Json<crate::api::site::AccountSettingsInput>,
) -> Result<Json<Value>, ApiError> {
    let mut next = {
        let config = state.config.read().await;
        let current: crate::api::site::AccountSettingsRecord = state
            .database
            .record("account-settings")
            .await?
            .ok_or_else(|| ApiError::not_found("账号策略尚未初始化"))?;
        if current.revision != input.revision {
            return Err(ApiError::conflict("配置已更新，请载入最新配置后测试"));
        }
        config.clone()
    };
    store::limit(
        &state.security_cache,
        "mail-test-user",
        &user.id.to_string(),
        5,
        60,
    )
    .await?;
    store::limit(&state.security_cache, "mail-test-ip", &client.ip, 10, 60).await?;
    next.security.mail_provider = input.settings.mail_provider;
    next.security.smtp_host = input.settings.smtp_host;
    next.security.smtp_port = input.settings.smtp_port;
    next.security.smtp_tls = input.settings.smtp_tls;
    next.security.smtp_from = input.settings.smtp_from;
    next.security.ses_region = input.settings.ses_region;
    next.security.smtp_enabled = true;
    next.security.registration_email_verification = false;
    let updates = crate::api::site::SecretUpdates {
        mail_credentials: input
            .secret_updates
            .mail_credentials
            .into_iter()
            .filter(|(provider, _)| *provider == next.security.mail_provider)
            .collect(),
        ..Default::default()
    };
    updates
        .apply(&mut next)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    next.security
        .validate(true)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if !next.security.smtp_available() {
        return Err(ApiError::bad_request("请填写发件人及所测服务商的完整配置"));
    }
    let ok = tokio::time::timeout(
        std::time::Duration::from_secs(15),
        transport(&next.security)?.test_connection(),
    )
    .await
    .map_err(|_| ApiError::service_unavailable("邮件连接测试超时"))?
    .map_err(|_| ApiError::bad_request("邮件连接或认证失败，请检查主机、网络及认证凭据"))?;
    if !ok {
        return Err(ApiError::bad_request("邮件服务器未接受连接测试"));
    }
    Ok(Json(json!({"message":"邮件连接及认证测试通过"})))
}
async fn send(
    config: &SecurityConfig,
    email: &str,
    code: &str,
    ttl_secs: u32,
) -> Result<(), ApiError> {
    let from = config
        .smtp_from
        .as_deref()
        .ok_or_else(|| ApiError::service_unavailable("邮箱验证尚未配置"))?;
    let message = Message::builder()
        .from(from.parse().map_err(|_| ApiError::service_unavailable("邮件发送配置无效"))?)
        .to(email.parse().map_err(|_| ApiError::bad_request("邮箱格式无效"))?)
        .subject("ostool 账号安全验证码")
        .body(format!("你的 ostool 验证码是：{code}\n验证码 {ttl_secs} 秒内有效，只能使用一次。请勿向任何人提供验证码。\n如果这不是你的操作，请忽略本邮件。"))
        .map_err(anyhow::Error::from)?;
    let transport = transport(config)?;
    tokio::time::timeout(std::time::Duration::from_secs(15), transport.send(message))
        .await
        .map_err(|_| ApiError::service_unavailable("验证码发送超时，请稍后重试"))?
        .map_err(|_| {
            // Provider diagnostics can contain addresses or credentials; keep them out of logs.
            log::warn!("SMTP delivery failed");
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
    let development = purpose == "email:register" && config.dev_registration_codes;
    if !development && !config.smtp_available() {
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
    if registration {
        store::limit(
            &state.security_cache,
            "registration-email-ip",
            &client.ip,
            policy.ip_limit as i32,
            policy.window_secs as i64,
        )
        .await?;
        store::limit(
            &state.security_cache,
            "registration-email-total",
            email,
            policy.email_limit as i32,
            policy.window_secs as i64,
        )
        .await?;
    }
    // Shared limits prevent switching purposes to bypass a recipient's cooldown/budget.
    store::limit(
        &state.security_cache,
        "mail-recipient-cooldown",
        email,
        1,
        policy.resend_secs.max(config.email_policy.resend_secs) as i64,
    )
    .await?;
    store::limit(
        &state.security_cache,
        "mail-recipient-total",
        email,
        config.email_policy.email_limit as i32,
        config.email_policy.window_secs as i64,
    )
    .await?;
    store::limit(
        &state.security_cache,
        "mail-ip-total",
        &client.ip,
        config.email_policy.ip_limit as i32,
        config.email_policy.window_secs as i64,
    )
    .await?;
    store::limit(
        &state.security_cache,
        "mail-global",
        "all",
        config.mail_hourly_limit as i32,
        3600,
    )
    .await?;
    let code = format!("{:06}", rand::random_range(0..1_000_000));
    let subject = crypto::hash(email);
    let table = proof_table(purpose);
    let id = Uuid::new_v4();
    let resend_secs = policy.resend_secs.max(config.email_policy.resend_secs);
    // Existing registrations and unknown/disabled recovery accounts receive the same response.
    // Delivery runs after acceptance so SMTP latency/failures do not disclose account existence.
    let deliver = match purpose {
        "email:register" => {
            !sqlx::query_scalar::<_, bool>("SELECT EXISTS(SELECT 1 FROM users WHERE email=$1)")
                .bind(email)
                .fetch_one(state.database.pool())
                .await?
        }
        "email:reset" => user.is_some(),
        _ => true,
    };
    let mut tx = state.database.begin().await?;
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
    let expires_at: DateTime<Utc> = sqlx::query_scalar(&format!("INSERT INTO {table}(id,purpose,user_id,email_hash,code_hash,security_version,request_ip_hash,expires_at) VALUES($1,$2,$3,$4,$5,$6,$7,now()+make_interval(secs=>$8::double precision)) RETURNING expires_at"))
        .bind(id).bind(purpose).bind(user.as_ref().map(|u|u.id)).bind(&subject)
        .bind(crypto::proof_hash(purpose,id,&code)).bind(user.as_ref().map(|u|u.security_version))
        .bind(crypto::hash(&client.ip)).bind(ttl_secs as f64).fetch_one(&mut *tx).await?;
    let delivery = if deliver {
        Some(sqlx::query_scalar::<_, Uuid>("INSERT INTO mail_deliveries(challenge_id,user_id,purpose,recipient_email,recipient_hash,transport,status,completed_at) VALUES($1,$2,$3,$4,$5,$6,$7,CASE WHEN $8 THEN now() ELSE NULL END) RETURNING id")
            .bind(id).bind(user.as_ref().map(|u|u.id)).bind(purpose).bind(email).bind(&subject)
            .bind(if development { "development" } else { "smtp" }).bind(if development { "sent" } else { "sending" }).bind(development).fetch_one(&mut *tx).await?)
    } else {
        sqlx::query(&format!(
            "UPDATE {table} SET invalidated_at=now() WHERE id=$1"
        ))
        .bind(id)
        .execute(&mut *tx)
        .await?;
        None
    };
    if development && deliver {
        sqlx::query("UPDATE email_verification_tokens SET sent_at=now() WHERE id=$1")
            .bind(id)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    if development && deliver {
        state
            .dev_registration_codes
            .insert(id, code.clone(), expires_at)
            .await;
        let state = state.clone();
        tokio::spawn(async move {
            let delay = (expires_at - Utc::now()).to_std().unwrap_or_default();
            tokio::time::sleep(delay).await;
            state.dev_registration_codes.remove(id).await;
            state.admin_events.invalidate(&["mail_deliveries"]);
        });
    }
    if let Some(delivery) = delivery.filter(|_| !development) {
        let state = state.clone();
        let email = email.to_owned();
        tokio::spawn(async move {
            let result = send(&config, &email, &code, ttl_secs).await;
            let update = async {
                let mut tx = state.database.begin().await?;
                sqlx::query("UPDATE mail_deliveries SET status=$2,error_code=$3,completed_at=now() WHERE id=$1")
                    .bind(delivery).bind(if result.is_ok() { "sent" } else { "failed" })
                    .bind(if result.is_ok() { None } else { Some("smtp_delivery_failed") })
                    .execute(&mut *tx).await?;
                sqlx::query(&format!("UPDATE {table} SET sent_at=CASE WHEN $2 THEN now() ELSE sent_at END,invalidated_at=CASE WHEN NOT $2 THEN now() ELSE invalidated_at END WHERE id=$1"))
                    .bind(id).bind(result.is_ok()).execute(&mut *tx).await?;
                tx.commit().await?;
                Ok::<_, sqlx::Error>(())
            }.await;
            if update.is_err() {
                log::error!("Failed to record mail delivery outcome: {delivery}");
            }
            state.admin_events.invalidate(&["mail_deliveries"]);
        });
    }
    state.admin_events.invalidate(&["mail_deliveries"]);
    Ok(Json(
        json!({"challenge_id":id,"expires_in":ttl_secs,"resend_after":resend_secs,"message":"如该邮箱可以接收验证码，邮件将很快送达"}),
    ))
}
async fn request(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
    Json(input): Json<EmailRequest>,
) -> Result<Json<Value>, ApiError> {
    store::limit(
        &state.security_cache,
        "mail-request-ip",
        &client.ip,
        30,
        900,
    )
    .await?;
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
fn registration_error() -> ApiError {
    ApiError::new(
        axum::http::StatusCode::BAD_REQUEST,
        "registration_email_invalid",
        "邮箱验证码错误、尝试过多或已过期，请重新验证邮箱",
    )
}
async fn verify_registration_email(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
    Json(input): Json<VerifyRegistrationEmail>,
) -> Result<Json<Value>, ApiError> {
    let config = state.config.read().await;
    if !config.auth.allow_registration || !config.security.registration_email_verification {
        return Err(ApiError::forbidden("邮箱验证注册未开放"));
    }
    drop(config);
    store::limit(
        &state.security_cache,
        "registration-email-confirm",
        &client.ip,
        10,
        900,
    )
    .await?;
    let email = crate::persistence::postgres::normalize_email(&input.email)
        .map_err(|_| ApiError::bad_request("邮箱格式无效"))?;
    check(
        &state,
        input.challenge_id,
        "email:register",
        Some(&email),
        input.code.trim(),
        false,
    )
    .await?
    .ok_or_else(registration_error)?;
    Ok(Json(json!({"verified": true})))
}
/// Final consumption shares the account-creation transaction, so failed account setup can be retried.
pub async fn consume_registration(
    connection: &mut sqlx::PgConnection,
    email: &str,
    id: Option<Uuid>,
    code: Option<&str>,
    max_attempts: u32,
) -> Result<(), ApiError> {
    let (Some(id), Some(code)) = (id, code) else {
        return Err(registration_error());
    };
    let count = sqlx::query("UPDATE email_verification_tokens SET consumed_at=clock_timestamp() WHERE id=$1 AND purpose='email:register' AND email_hash=$2 AND code_hash=$3 AND consumed_at IS NULL AND invalidated_at IS NULL AND expires_at>clock_timestamp() AND attempts<$4")
        .bind(id).bind(crypto::hash(email)).bind(crypto::proof_hash("email:register",id,code.trim()))
        .bind(max_attempts as i32).execute(connection).await?.rows_affected();
    if count != 1 {
        return Err(registration_error());
    }
    Ok(())
}
struct VerifiedEmail {
    user_id: Option<Uuid>,
    security_version: Option<i64>,
}
/// Each proof retains its lifecycle metadata; only code hashes are persisted.
async fn check(
    state: &AppState,
    id: Uuid,
    purpose: &str,
    email: Option<&str>,
    code: &str,
    consume_on_success: bool,
) -> Result<Option<VerifiedEmail>, ApiError> {
    let config = state.config.read().await.security.clone();
    let max_attempts = if purpose == "email:register" {
        config.registration_email_policy.max_attempts
    } else {
        config.email_policy.max_attempts
    } as i32;
    let table = proof_table(purpose);
    let mut tx = state.database.begin().await?;
    let row = sqlx::query(&format!("SELECT * FROM {table} WHERE id=$1 FOR UPDATE"))
        .bind(id)
        .fetch_optional(&mut *tx)
        .await?;
    let Some(row) = row else { return Ok(None) };
    let valid = code.len() == 6
        && code.bytes().all(|c| c.is_ascii_digit())
        && row.try_get::<String, _>("purpose")? == purpose
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
    sqlx::query(&format!("UPDATE {table} SET attempts=LEAST(attempts+CASE WHEN $2 THEN 0 ELSE 1 END,5),consumed_at=CASE WHEN $2 AND $4 THEN now() ELSE consumed_at END,invalidated_at=CASE WHEN NOT $2 AND attempts+1 >= $3 THEN COALESCE(invalidated_at,now()) ELSE invalidated_at END WHERE id=$1"))
        .bind(id).bind(valid).bind(max_attempts).bind(consume_on_success).execute(&mut *tx).await?;
    tx.commit().await?;
    if purpose == "email:register" && config.dev_registration_codes {
        state.admin_events.invalidate(&["mail_deliveries"]);
    }

    Ok(if valid {
        Some(VerifiedEmail {
            user_id: row.try_get("user_id")?,
            security_version: row.try_get("security_version")?,
        })
    } else {
        None
    })
}
async fn consume(
    state: &AppState,
    id: Uuid,
    purpose: &str,
    email: Option<&str>,
    code: &str,
) -> Result<Option<VerifiedEmail>, ApiError> {
    check(state, id, purpose, email, code, true).await
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
        && check(state, id, "email:register", Some(email), code.trim(), false)
            .await?
            .is_some()
    {
        return Ok(true);
    }
    Err(registration_error())
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
    let proof = consume(
        &state,
        input.challenge_id,
        "email:reset",
        None,
        input.code.trim(),
    )
    .await?
    .ok_or_else(|| ApiError::bad_request("验证码错误、已使用或已过期"))?;
    let user = proof
        .user_id
        .ok_or_else(|| ApiError::bad_request("验证码错误、已使用或已过期"))?;
    let version = proof
        .security_version
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
    Extension(client): Extension<ClientContext>,
    Json(input): Json<VerifyEmail>,
) -> Result<Json<Value>, ApiError> {
    store::limit(&state.security_cache, "email-confirm", &client.ip, 10, 900).await?;
    let proof = consume(
        &state,
        input.challenge_id,
        "email:verify",
        Some(&user.email),
        input.code.trim(),
    )
    .await?
    .ok_or_else(|| ApiError::bad_request("验证码错误或已过期"))?;
    if proof.user_id != Some(user.id) || proof.security_version != Some(user.security_version) {
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
