use axum::{
    Json, Router,
    extract::{Extension, Form, Path, State},
    http::{HeaderMap, Request, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{delete, get, post},
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{
    api::error::ApiError,
    persistence::postgres::{AccessGrant, Database, DevicePoll, User},
    state::AppState,
};

const DEVICE_CLIENT_ID: &str = "ostool-cli";

#[derive(Debug, Clone)]
pub struct CurrentUser(pub User);

#[derive(Debug, Deserialize)]
pub struct DeviceCodeForm {
    pub client_id: String,
    pub scope: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct TokenForm {
    pub grant_type: String,
    pub device_code: Option<String>,
    pub refresh_token: Option<String>,
    pub client_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct RevokeForm {
    pub token: String,
    pub token_type_hint: Option<String>,
    pub client_id: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegisterRequest {
    pub username: String,
    #[serde(default)]
    pub legal: super::site::LegalConsent,
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
    #[serde(default)]
    pub verification: crate::security::captcha::HumanProof,
    pub email_challenge_id: Option<Uuid>,
    pub email_code: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
    #[serde(default)]
    pub verification: crate::security::captcha::HumanProof,
}

#[derive(Debug, Deserialize)]
pub struct AuthorizeDeviceRequest {
    pub user_code: String,
}

#[derive(Debug, Deserialize)]
pub struct CreatePersonalAccessTokenRequest {
    pub name: String,
    pub expires_in_days: Option<u32>,
}

#[derive(Debug, Serialize)]
pub struct PersonalAccessTokenResponse {
    pub id: Uuid,
    pub token: String,
    pub name: String,
    pub expires_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Serialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub refresh_token: String,
    pub token_type: &'static str,
    pub expires_in: i64,
    pub scope: String,
}

#[derive(Debug, Serialize)]
pub struct DeviceCodeResponse {
    pub device_code: String,
    pub user_code: String,
    pub verification_uri: String,
    pub verification_uri_complete: String,
    pub expires_in: i64,
    pub interval: u64,
}

#[derive(Debug, Serialize)]
pub struct UserResponse {
    pub id: Uuid,
    pub email: String,
    pub display_name: Option<String>,
    pub is_admin: bool,
}

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/oauth/device/code", post(device_code))
        .route("/oauth/token", post(token))
        .route("/oauth/revoke", post(revoke))
        .route("/api/v1/auth/config", get(auth_config))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/admin/auth/login", post(admin_login))
        .route("/api/v1/auth/me", get(me))
        .route("/api/v1/auth/refresh", post(browser_refresh))
        .route("/api/v1/auth/logout", post(browser_logout))
        .merge(crate::security::captcha::routes())
        .merge(crate::security::mfa::routes())
        .merge(crate::security::mail::routes())
        .merge(crate::security::sessions::routes())
        .route("/api/v1/auth/device/authorize", post(authorize_device))
        .route("/api/v1/auth/device/{user_code}", get(device_status))
        .route(
            "/api/v1/auth/personal-access-tokens",
            post(create_personal_access_token),
        )
        .route(
            "/api/v1/auth/personal-access-tokens/{id}",
            delete(revoke_personal_access_token),
        )
}

pub async fn middleware(
    State(state): State<AppState>,
    mut request: Request<axum::body::Body>,
    next: Next,
) -> Response {
    let path = request.uri().path();
    let public = state.config.read().await.edition.is_public();
    if !public {
        if path.starts_with("/oauth/")
            || (path.starts_with("/api/v1/auth/") && path != "/api/v1/auth/config")
            || [
                "/api/v1/admin/auth",
                "/api/v1/admin/users",
                "/api/v1/admin/roles",
                "/api/v1/admin/rentals",
                "/api/v1/admin/rental-requests",
                "/api/v1/admin/mail-deliveries",
                "/api/v1/account",
            ]
            .iter()
            .any(|prefix| path.starts_with(prefix))
        {
            return ApiError::not_found("this feature is unavailable in local edition")
                .into_response();
        }
        return next.run(request).await;
    }
    let hardware_config = state.config.read().await.clone();
    if (path.starts_with("/share/sessions/")
        || path.starts_with("/boot/sessions/")
        || path.starts_with("/api/v1/loader-updates/"))
        && request
            .extensions()
            .get::<crate::security::store::ClientContext>()
            .is_some_and(|client| crate::security::web::hardware_allowed(&hardware_config, client))
    {
        return next.run(request).await;
    }
    if is_public_path(path) {
        return next.run(request).await;
    }
    let config = state.config.read().await.clone();
    let surface =
        crate::security::web::Surface::from_request(request.headers(), path, request.uri().query());
    let supplied_bearer = bearer_token(request.headers());
    let cookie_name = surface.cookie_name(false, crate::security::web::secure(&config));
    let token =
        supplied_bearer.or_else(|| crate::security::web::cookie(request.headers(), &cookie_name));
    let Some(token) = token else {
        return ApiError::unauthorized("请登录后继续操作").into_response();
    };
    let mut credential = match state.database.authenticate_credential(token).await {
        Ok(Some(credential)) => credential,
        Ok(None) => return ApiError::unauthorized("登录已过期，请重新登录").into_response(),
        Err(error) => {
            log::error!("authentication failed: {error:#}");
            return ApiError::service_unavailable("身份验证暂时不可用").into_response();
        }
    };
    if supplied_bearer.is_none() && credential.client_id != surface.client() {
        return ApiError::unauthorized("请登录当前站点").into_response();
    }
    let user = credential.user.clone();
    // A deployment may enable mandatory MFA while older administrator sessions exist.
    if config.security.require_admin_mfa && user.is_admin && credential.client_id == "admin-web" {
        let enrolled: bool = match sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM mfa_credentials WHERE user_id=$1)",
        )
        .bind(user.id)
        .fetch_one(state.database.pool())
        .await
        {
            Ok(value) => value,
            Err(error) => {
                log::error!("MFA policy lookup failed: {error}");
                return ApiError::service_unavailable("身份验证暂时不可用").into_response();
            }
        };
        if !enrolled {
            credential.scope = "mfa:enroll".into();
        }
    }
    if credential.scope == "mfa:enroll"
        && !matches!(
            path,
            "/api/v1/auth/me"
                | "/api/v1/auth/security"
                | "/api/v1/auth/security/events"
                | "/api/v1/auth/step-up"
                | "/api/v1/auth/security/enrollment"
                | "/api/v1/auth/security/confirm"
        )
    {
        return ApiError::new(
            StatusCode::FORBIDDEN,
            "mfa_enrollment_required",
            "请先启用管理员双重验证",
        )
        .into_response();
    }
    if path.starts_with("/api/v1/admin/") && (!user.is_admin || credential.client_id != "admin-web")
    {
        return ApiError::forbidden("请使用管理员账号登录管理后台").into_response();
    }
    if credential.client_id == "ostool-cli" && path != "/api/v1/auth/me" {
        let board_operation = path == "/api/v1/board-types"
            || path == "/api/v1/sessions"
            || path.starts_with("/api/v1/sessions/")
            || path.starts_with("/share/sessions/")
            || path.starts_with("/boot/sessions/");
        if !board_operation
            || !credential
                .scope
                .split_whitespace()
                .any(|scope| scope == "board:operate")
        {
            return ApiError::forbidden("CLI 凭据仅可在授权范围内操作开发板").into_response();
        }
    }
    if credential.client_id == "pat"
        && (path.starts_with("/api/v1/account")
            || (path.starts_with("/api/v1/auth/") && path != "/api/v1/auth/me"))
    {
        return ApiError::forbidden("个人访问令牌仅用于 CLI 板卡操作").into_response();
    }
    if crate::security::web::sensitive(request.method(), path)
        && let Err(error) = crate::security::web::require_step_up(&credential)
    {
        return error.into_response();
    }
    if !user.is_admin
        && let Some(tail) = path
            .strip_prefix("/api/v1/sessions/")
            .or_else(|| path.strip_prefix("/share/sessions/"))
            .or_else(|| path.strip_prefix("/boot/sessions/"))
    {
        let session_id = tail.split('/').next().unwrap_or_default();
        match sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM session_owners s JOIN board_rentals r ON s.rental_id=r.id WHERE s.session_id=$1 AND s.user_id=$2 AND r.cancelled_at IS NULL AND r.starts_at<=now() AND r.ends_at>now())",
        )
        .bind(session_id)
        .bind(user.id)
        .fetch_one(state.database.pool())
        .await
        {
            Ok(true) => {}
            Ok(false) => return ApiError::not_found("session not found").into_response(),
            Err(_) => {
                return ApiError::service_unavailable("session ownership lookup unavailable")
                    .into_response();
            }
        }
    }
    request
        .extensions_mut()
        .insert(crate::security::web::CurrentCredential(credential));
    request.extensions_mut().insert(CurrentUser(user.clone()));
    let mut response = next.run(request).await;
    response.extensions_mut().insert(CurrentUser(user));
    response
}

fn is_public_path(path: &str) -> bool {
    path == "/"
        || path == "/register"
        || path == "/password-reset"
        || path == "/account"
        || path.starts_with("/account/")
        || path.starts_with("/admin")
        || path == "/login"
        || path.starts_with("/login/")
        || path == "/devices"
        || path.starts_with("/devices/")
        || path == "/docs"
        || path.starts_with("/docs/")
        || path == "/about"
        || path.starts_with("/about/")
        || path.starts_with("/oauth/")
        || path == "/api/v1/site"
        || path.starts_with("/api/v1/site/")
        || path == "/legal/terms"
        || path == "/legal/privacy"
        || path == "/api/v1/auth/config"
        || path == "/api/v1/auth/register"
        || path == "/api/v1/auth/login"
        || path == "/api/v1/auth/captcha"
        || path == "/api/v1/auth/email-code"
        || path == "/api/v1/auth/password-reset"
        || path == "/api/v1/auth/mfa/verify"
        || path == "/api/v1/auth/refresh"
        || path == "/api/v1/auth/logout"
        || path.starts_with("/api/v1/loaders/")
        || path == "/healthz"
        || path.starts_with("/admin/assets/")
        || path.starts_with("/assets/")
        || path == "/api/v1/admin/auth/login"
}

fn bearer_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?
        .strip_prefix("Bearer ")
        .or_else(|| {
            headers
                .get(axum::http::header::AUTHORIZATION)?
                .to_str()
                .ok()?
                .strip_prefix("bearer ")
        })
}

async fn database(state: &AppState) -> Result<(&Database, crate::config::AuthConfig), ApiError> {
    let config = state.config.read().await.auth.clone();
    let database = &state.database;
    Ok((database, config))
}

async fn device_code(
    State(state): State<AppState>,
    Form(form): Form<DeviceCodeForm>,
) -> Result<Json<DeviceCodeResponse>, Response> {
    if form.client_id != DEVICE_CLIENT_ID {
        return Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client",
            "unsupported client_id",
        ));
    }
    let (database, config) = database(&state)
        .await
        .map_err(IntoResponse::into_response)?;
    let scope = form
        .scope
        .unwrap_or_else(|| "board:operate offline_access".into());
    if scope
        .split_whitespace()
        .any(|scope| !matches!(scope, "board:operate" | "offline_access"))
    {
        return Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_scope",
            "unsupported scope",
        ));
    }
    let (device_code, user_code, expires_at, interval) = database
        .create_device_code(&form.client_id, &scope, &config)
        .await
        .map_err(|error| {
            log::error!("OAuth operation failed: {error:#}");
            oauth_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                "authentication temporarily unavailable",
            )
        })?;
    let origin = config
        .public_base_url
        .clone()
        .unwrap_or_else(|| "http://127.0.0.1:2999".into());
    let verification_uri = format!("{}/login", origin.trim_end_matches('/'));
    let base = format!("{verification_uri}?user_code={user_code}");
    Ok(Json(DeviceCodeResponse {
        device_code,
        user_code,
        verification_uri,
        verification_uri_complete: base,
        expires_in: (expires_at - Utc::now()).num_seconds().max(1),
        interval,
    }))
}

async fn token(
    State(state): State<AppState>,
    Form(form): Form<TokenForm>,
) -> Result<(Extension<CurrentUser>, Json<TokenResponse>), Response> {
    let (database, config) = database(&state)
        .await
        .map_err(IntoResponse::into_response)?;
    if form
        .client_id
        .as_deref()
        .is_some_and(|client| client != DEVICE_CLIENT_ID)
    {
        return Err(oauth_error(
            StatusCode::BAD_REQUEST,
            "invalid_client",
            "unsupported client_id",
        ));
    }
    let grant = match form.grant_type.as_str() {
        "urn:ietf:params:oauth:grant-type:device_code" => {
            let Some(code) = form.device_code.as_deref() else {
                return Err(oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "device_code is required",
                ));
            };
            match database
                .poll_device_code(code, &config)
                .await
                .map_err(|error| {
                    log::error!("OAuth operation failed: {error:#}");
                    oauth_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "server_error",
                        "authentication temporarily unavailable",
                    )
                })? {
                DevicePoll::Pending => {
                    return Err(oauth_error(
                        StatusCode::BAD_REQUEST,
                        "authorization_pending",
                        "the user has not approved this login",
                    ));
                }
                DevicePoll::Denied => {
                    return Err(oauth_error(
                        StatusCode::BAD_REQUEST,
                        "access_denied",
                        "the user denied this login",
                    ));
                }
                DevicePoll::Expired => {
                    return Err(oauth_error(
                        StatusCode::BAD_REQUEST,
                        "expired_token",
                        "the device code has expired",
                    ));
                }
                DevicePoll::Authorized(grant) => *grant,
            }
        }
        "refresh_token" => {
            let Some(refresh) = form.refresh_token.as_deref() else {
                return Err(oauth_error(
                    StatusCode::BAD_REQUEST,
                    "invalid_request",
                    "refresh_token is required",
                ));
            };
            database
                .refresh_for_client(refresh, &config, Some(DEVICE_CLIENT_ID))
                .await
                .map_err(|error| {
                    log::error!("OAuth operation failed: {error:#}");
                    oauth_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "server_error",
                        "authentication temporarily unavailable",
                    )
                })?
                .ok_or_else(|| {
                    oauth_error(
                        StatusCode::BAD_REQUEST,
                        "invalid_grant",
                        "refresh token is invalid or expired",
                    )
                })?
        }
        _ => {
            return Err(oauth_error(
                StatusCode::BAD_REQUEST,
                "unsupported_grant_type",
                "grant_type is not supported",
            ));
        }
    };
    Ok((
        Extension(CurrentUser(grant.user.clone())),
        Json(token_response(grant)),
    ))
}

async fn revoke(
    State(state): State<AppState>,
    Form(form): Form<RevokeForm>,
) -> Result<Response, Response> {
    let (database, _) = database(&state)
        .await
        .map_err(IntoResponse::into_response)?;
    let actor = database.refresh_owner(&form.token).await.map_err(|_| {
        ApiError::service_unavailable("unable to resolve logout identity").into_response()
    })?;
    database
        .revoke_refresh(&form.token)
        .await
        .map_err(|error| {
            log::error!("OAuth revocation failed: {error:#}");
            oauth_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                "authentication temporarily unavailable",
            )
        })?;
    let mut response = StatusCode::NO_CONTENT.into_response();
    if let Some(actor) = actor {
        response.extensions_mut().insert(CurrentUser(actor));
    }
    Ok(response)
}

async fn register(
    State(state): State<AppState>,
    Extension(client): Extension<crate::security::store::ClientContext>,
    Json(request): Json<RegisterRequest>,
) -> Result<Response, ApiError> {
    let (_, config) = database(&state).await?;
    if !config.allow_registration {
        return Err(ApiError::forbidden("注册暂未开放"));
    }
    crate::security::store::limit(&state.security_cache, "register-ip", &client.ip, 6, 3600)
        .await?;
    crate::security::captcha::verify(
        &state,
        &client,
        crate::security::captcha::HumanAction::Register,
        &request.verification,
    )
    .await?;
    crate::security::crypto::validate_password(&request.password)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if request
        .display_name
        .as_ref()
        .is_some_and(|v| v.chars().count() > 120)
    {
        return Err(ApiError::bad_request("显示名称不超过 120 个字符"));
    }
    let email = crate::persistence::postgres::normalize_email(&request.email)
        .map_err(|_| ApiError::bad_request("邮箱格式无效"))?;
    // Consume optional one-time email proof before creating the account.
    let verified = crate::security::mail::registration_proof(
        &state,
        &email,
        request.email_challenge_id,
        request.email_code.as_deref(),
    )
    .await?;
    let user = super::site::register(&state, &request, &client, &email, verified).await?;
    state.admin_events.invalidate(&["users"]);
    let mut response = crate::security::mfa::begin_login(
        &state,
        user,
        crate::security::web::Surface::Website,
        &client,
    )
    .await?;
    *response.status_mut() = StatusCode::CREATED;
    Ok(response)
}
async fn password_login(
    state: &AppState,
    client: &crate::security::store::ClientContext,
    request: LoginRequest,
    admin: bool,
) -> Result<Response, ApiError> {
    use crate::security::{
        captcha::{self, HumanAction},
        store,
        web::Surface,
    };
    let (database, _) = database(state).await?;
    let config = state.config.read().await.security.clone();
    let account = database.login_subject(&request.email).await?;
    let identity = format!("{account}:{}", client.ip);
    store::limit(
        &state.security_cache,
        if admin { "admin-login" } else { "web-login" },
        &identity,
        config.login_max_failures as i32,
        config.login_lock_secs as i64,
    )
    .await?;
    store::limit(
        &state.security_cache,
        "login-account",
        &account,
        config.login_max_failures as i32 * 3,
        config.login_lock_secs as i64,
    )
    .await?;
    captcha::verify(
        state,
        client,
        if admin {
            HumanAction::AdminLogin
        } else {
            HumanAction::Login
        },
        &request.verification,
    )
    .await?;
    if request.email.len() > 320 || request.password.len() > 1024 {
        return Err(ApiError::unauthorized("用户名、邮箱或密码不正确"));
    }
    let user = database
        .authenticate_password(&request.email, &request.password)
        .await?
        .filter(|user| !admin || user.is_admin)
        .ok_or_else(|| ApiError::unauthorized("用户名、邮箱或密码不正确"))?;
    store::clear_limit(
        &state.security_cache,
        if admin { "admin-login" } else { "web-login" },
        &identity,
    )
    .await?;
    store::clear_limit(&state.security_cache, "login-account", &account).await?;
    crate::security::mfa::begin_login(
        state,
        user,
        if admin {
            Surface::Admin
        } else {
            Surface::Website
        },
        client,
    )
    .await
}
async fn login(
    State(state): State<AppState>,
    Extension(client): Extension<crate::security::store::ClientContext>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    password_login(&state, &client, request, false).await
}
async fn admin_login(
    State(state): State<AppState>,
    Extension(client): Extension<crate::security::store::ClientContext>,
    Json(request): Json<LoginRequest>,
) -> Result<Response, ApiError> {
    password_login(&state, &client, request, true).await
}
async fn browser_refresh(
    State(state): State<AppState>,
    headers: HeaderMap,
    Extension(client): Extension<crate::security::store::ClientContext>,
) -> Result<Response, ApiError> {
    let config = state.config.read().await.clone();
    let surface = crate::security::web::Surface::from_request(&headers, "", None);
    let name = surface.cookie_name(true, crate::security::web::secure(&config));
    let token = crate::security::web::cookie(&headers, &name)
        .ok_or_else(|| ApiError::unauthorized("请重新登录"))?;
    let grant = state
        .database
        .refresh_for_client(token, &config.auth, Some(surface.client()))
        .await?
        .ok_or_else(|| ApiError::unauthorized("请重新登录"))?;
    crate::security::web::refreshed_grant(&state, grant, surface, &client).await
}
async fn browser_logout(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<Response, ApiError> {
    let config = state.config.read().await.clone();
    let surface = crate::security::web::Surface::from_request(&headers, "", None);
    let name = surface.cookie_name(true, crate::security::web::secure(&config));
    if let Some(token) = crate::security::web::cookie(&headers, &name) {
        state.database.revoke_refresh(token).await?;
    }
    Ok(crate::security::web::clear_cookies(&state, surface).await)
}

async fn me(
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Extension(crate::security::web::CurrentCredential(credential)): Extension<
        crate::security::web::CurrentCredential,
    >,
) -> Json<serde_json::Value> {
    Json(
        serde_json::json!({"id":user.id,"username":user.username,"email":user.email,"display_name":user.display_name,"is_admin":user.is_admin,"mfa_enrollment_required":credential.scope=="mfa:enroll"}),
    )
}

async fn create_personal_access_token(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(request): Json<CreatePersonalAccessTokenRequest>,
) -> Result<(StatusCode, Json<PersonalAccessTokenResponse>), ApiError> {
    let (database, _) = database(&state).await?;
    if request.name.trim().is_empty()
        || request.name.chars().count() > 120
        || request
            .expires_in_days
            .is_some_and(|days| !(1..=365).contains(&days))
    {
        return Err(ApiError::bad_request(
            "令牌名称为 1 至 120 个字符，有效期为 1 至 365 天",
        ));
    }
    let expires_at =
        Some(Utc::now() + chrono::Duration::days(i64::from(request.expires_in_days.unwrap_or(90))));
    let token = database
        .create_personal_access_token(user.id, &request.name, expires_at)
        .await?;
    state.admin_events.invalidate(&["users"]);
    Ok((
        StatusCode::CREATED,
        Json(PersonalAccessTokenResponse {
            id: token.id,
            token: token.token,
            name: token.name,
            expires_at: token.expires_at,
        }),
    ))
}

async fn revoke_personal_access_token(
    Path(id): Path<Uuid>,
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
) -> Result<StatusCode, ApiError> {
    let (database, _) = database(&state).await?;
    if database.revoke_personal_access_token(user.id, id).await? {
        state.admin_events.invalidate(&["users"]);
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::not_found("personal access token not found"))
    }
}

async fn authorize_device(
    State(state): State<AppState>,
    Extension(CurrentUser(user)): Extension<CurrentUser>,
    Json(request): Json<AuthorizeDeviceRequest>,
) -> Result<StatusCode, ApiError> {
    let (database, _) = database(&state).await?;
    if database
        .authorize_device_code(&request.user_code, user.id)
        .await?
    {
        Ok(StatusCode::NO_CONTENT)
    } else {
        Err(ApiError::bad_request(
            "device code is invalid, expired, or already approved",
        ))
    }
}

async fn device_status(
    Path(user_code): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<serde_json::Value>, ApiError> {
    let (database, _) = database(&state).await?;
    let status =
        sqlx::query("SELECT authorized_at,expires_at FROM oauth_device_codes WHERE user_code=$1")
            .bind(user_code.to_uppercase())
            .fetch_optional(database.pool())
            .await?;
    let Some(row) = status else {
        return Err(ApiError::not_found("device code not found"));
    };
    let expires_at: DateTime<Utc> = sqlx::Row::try_get(&row, "expires_at")?;
    let authorized =
        sqlx::Row::try_get::<Option<DateTime<Utc>>, _>(&row, "authorized_at")?.is_some();
    Ok(Json(
        serde_json::json!({"authorized": authorized, "expired": expires_at <= Utc::now()}),
    ))
}

fn token_response(grant: AccessGrant) -> TokenResponse {
    TokenResponse {
        access_token: grant.access_token,
        refresh_token: grant.refresh_token,
        token_type: "Bearer",
        expires_in: (grant.access_expires_at - Utc::now()).num_seconds().max(1),
        scope: grant.scope,
    }
}

fn oauth_error(status: StatusCode, error: &str, description: &str) -> Response {
    (
        status,
        Json(serde_json::json!({"error": error, "error_description": description})),
    )
        .into_response()
}

pub(crate) fn public_config(config: &crate::ServerConfig) -> serde_json::Value {
    serde_json::json!({"edition":config.edition,"enabled":config.edition.is_public(),"allow_registration":config.edition.is_public() && config.auth.allow_registration,"database_available":true,"captcha_required":config.edition.is_public(),"registration_email_verification":config.edition.is_public() && config.security.registration_email_verification,"email_security_available":config.edition.is_public() && config.security.smtp_available(),"password_reset_available":config.edition.is_public() && config.security.smtp_available() && config.security.password_reset_enabled})
}
async fn auth_config(State(state): State<AppState>) -> Result<Json<serde_json::Value>, ApiError> {
    let site = super::site::view(&state).await?;
    let mut result = public_config(&*state.config.read().await);
    result["site"] = serde_json::to_value(site).map_err(anyhow::Error::from)?;
    Ok(Json(result))
}
