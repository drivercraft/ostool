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
pub struct RegisterRequest {
    pub email: String,
    pub password: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct LoginRequest {
    pub email: String,
    pub password: String,
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
    if is_public_path(path) {
        return next.run(request).await;
    }
    let database = &state.database;
    let Some(token) = bearer_token(request.headers()).or_else(|| {
        (matches!(path, "/api/v1/admin/events" | "/api/v1/account/events"))
            .then(|| request.uri().query())
            .flatten()
            .and_then(|query| {
                query
                    .split('&')
                    .find_map(|part| part.strip_prefix("access_token="))
            })
    }) else {
        return ApiError::unauthorized("Bearer access token is required").into_response();
    };
    let user = match database.authenticate_token(token).await {
        Ok(Some(user)) => user,
        Ok(None) => {
            return ApiError::unauthorized("access token is invalid or expired").into_response();
        }
        Err(error) => {
            return ApiError::service_unavailable(format!("authentication unavailable: {error:#}"))
                .into_response();
        }
    };
    if path.starts_with("/api/v1/admin") && !user.is_admin {
        return ApiError::forbidden("administrator role is required").into_response();
    }
    if !user.is_admin
        && let Some(tail) = path.strip_prefix("/api/v1/sessions/")
    {
        let session_id = tail.split('/').next().unwrap_or_default();
        match sqlx::query_scalar::<_, bool>(
            "SELECT EXISTS(SELECT 1 FROM session_owners s JOIN board_rentals r ON s.rental_id=r.id WHERE s.session_id=$1 AND s.user_id=$2 AND r.cancelled_at IS NULL AND r.starts_at<=now() AND r.ends_at>now())",
        )
        .bind(session_id)
        .bind(user.id)
        .fetch_one(database.pool())
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
    request.extensions_mut().insert(CurrentUser(user.clone()));
    let mut response = next.run(request).await;
    response.extensions_mut().insert(CurrentUser(user));
    response
}

fn is_public_path(path: &str) -> bool {
    path == "/"
        || path == "/register"
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
        || path == "/api/v1/auth/config"
        || path == "/api/v1/auth/register"
        || path == "/api/v1/auth/login"
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
    let (device_code, user_code, expires_at, interval) = database
        .create_device_code(&form.client_id, &scope, &config)
        .await
        .map_err(|error| {
            oauth_error(
                StatusCode::SERVICE_UNAVAILABLE,
                "server_error",
                &error.to_string(),
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
                .map_err(|e| {
                    oauth_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "server_error",
                        &e.to_string(),
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
                DevicePoll::Authorized(grant) => grant,
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
                .refresh(refresh, &config)
                .await
                .map_err(|e| {
                    oauth_error(
                        StatusCode::SERVICE_UNAVAILABLE,
                        "server_error",
                        &e.to_string(),
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
    database.revoke_refresh(&form.token).await.map_err(|e| {
        oauth_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            &e.to_string(),
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
    Json(request): Json<RegisterRequest>,
) -> Result<(StatusCode, Extension<CurrentUser>, Json<TokenResponse>), ApiError> {
    let (database, config) = database(&state).await?;
    if !config.allow_registration {
        return Err(ApiError::forbidden("registration is disabled"));
    }
    if request
        .display_name
        .as_ref()
        .is_some_and(|name| name.len() > 120)
    {
        return Err(ApiError::bad_request("显示名称不超过 120 个字符"));
    }
    let user = database
        .create_user(
            &request.email,
            &request.password,
            request.display_name.as_deref(),
            false,
        )
        .await
        .map_err(|error| {
            if error
                .chain()
                .filter_map(|e| e.downcast_ref::<sqlx::Error>())
                .any(|e| {
                    e.as_database_error()
                        .is_some_and(|e| e.is_unique_violation())
                })
            {
                ApiError::conflict("该邮箱已注册，请直接登录")
            } else if request.password.len() < 12 || request.password.len() > 1024 {
                ApiError::bad_request("密码长度需要 12 至 1024 个字符")
            } else if crate::persistence::postgres::normalize_email(&request.email).is_err() {
                ApiError::bad_request("邮箱格式无效")
            } else {
                log::error!("registration failed: {error:#}");
                ApiError::service_unavailable("注册暂时不可用，请稍后重试")
            }
        })?;
    state.admin_events.invalidate(&["users"]);
    let grant = database
        .issue_grant(user, "web", "board:operate offline_access", &config)
        .await?;
    Ok((
        StatusCode::CREATED,
        Extension(CurrentUser(grant.user.clone())),
        Json(token_response(grant)),
    ))
}

async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<(Extension<CurrentUser>, Json<TokenResponse>), ApiError> {
    let (database, config) = database(&state).await?;
    let user = database
        .authenticate_password(&request.email, &request.password)
        .await?
        .ok_or_else(|| ApiError::unauthorized("email or password is incorrect"))?;
    let grant = database
        .issue_grant(user, "web", "board:operate offline_access", &config)
        .await?;
    Ok((
        Extension(CurrentUser(grant.user.clone())),
        Json(token_response(grant)),
    ))
}

async fn admin_login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<(Extension<CurrentUser>, Json<TokenResponse>), ApiError> {
    let (database, config) = database(&state).await?;
    let user = database
        .authenticate_password(&request.email, &request.password)
        .await?
        .ok_or_else(|| ApiError::unauthorized("邮箱或密码不正确"))?;
    if !user.is_admin {
        return Err(ApiError::forbidden("该账号没有管理后台权限"));
    }
    let grant = database
        .issue_grant(user, "admin-web", "board:operate offline_access", &config)
        .await?;
    Ok((
        Extension(CurrentUser(grant.user.clone())),
        Json(token_response(grant)),
    ))
}

async fn me(Extension(CurrentUser(user)): Extension<CurrentUser>) -> Json<UserResponse> {
    Json(UserResponse {
        id: user.id,
        email: user.email,
        display_name: user.display_name,
        is_admin: user.is_admin,
    })
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
            .is_some_and(|days| !(1..=3650).contains(&days))
    {
        return Err(ApiError::bad_request(
            "令牌名称为 1 至 120 个字符，有效期为 1 至 3650 天",
        ));
    }
    let expires_at = request
        .expires_in_days
        .map(|days| Utc::now() + chrono::Duration::days(i64::from(days.max(1))));
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
            .await
            .map_err(|error| ApiError::service_unavailable(error.to_string()))?;
    let Some(row) = status else {
        return Err(ApiError::not_found("device code not found"));
    };
    let expires_at: DateTime<Utc> = sqlx::Row::try_get(&row, "expires_at")
        .map_err(|error| ApiError::service_unavailable(error.to_string()))?;
    let authorized = sqlx::Row::try_get::<Option<DateTime<Utc>>, _>(&row, "authorized_at")
        .map_err(|error| ApiError::service_unavailable(error.to_string()))?
        .is_some();
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

async fn auth_config(State(state): State<AppState>) -> Json<serde_json::Value> {
    let config = state.config.read().await;
    Json(
        serde_json::json!({"edition":config.edition,"enabled":config.edition.is_public(),"allow_registration":config.edition.is_public() && config.auth.allow_registration,"database_available":true}),
    )
}
