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
    database::{AccessGrant, Database, DevicePoll, User},
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
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/login", post(login))
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
    if is_public_path(path) {
        return next.run(request).await;
    }
    let config = state.config.read().await.auth.clone();
    let identity_required = path == "/api/v1/auth/me"
        || path == "/api/v1/auth/device/authorize"
        || path.starts_with("/api/v1/auth/personal-access-tokens");
    if !config.enabled && !identity_required {
        // Authentication remains optional for legacy board APIs, but a valid
        // token is still attached so OAuth device approval and account routes
        // work when an installation has not yet enforced authentication.
        if let (Some(database), Some(token)) = (
            state.database.as_ref(),
            bearer_token(request.headers()).or_else(|| {
                request.uri().query().and_then(|query| {
                    query
                        .split('&')
                        .find_map(|part| part.strip_prefix("access_token="))
                })
            }),
        ) && let Ok(Some(user)) = database.authenticate_token(token).await
        {
            request.extensions_mut().insert(CurrentUser(user));
        }
        return next.run(request).await;
    }
    let Some(database) = state.database.as_ref() else {
        return ApiError::service_unavailable("authentication database is not configured")
            .into_response();
    };
    let Some(token) = bearer_token(request.headers()).or_else(|| {
        request.uri().query().and_then(|query| {
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
    request.extensions_mut().insert(CurrentUser(user));
    next.run(request).await
}

fn is_public_path(path: &str) -> bool {
    path == "/"
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
        || path == "/api/v1/auth/register"
        || path == "/api/v1/auth/login"
        || path.starts_with("/api/v1/loaders/")
        || path == "/healthz"
        || path.starts_with("/admin/assets/")
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
    let database = state
        .database
        .as_ref()
        .ok_or_else(|| ApiError::service_unavailable("PostgreSQL is not configured"))?;
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
    let base = format!("/login?user_code={user_code}");
    Ok(Json(DeviceCodeResponse {
        device_code,
        user_code,
        verification_uri: "/login".into(),
        verification_uri_complete: base,
        expires_in: (expires_at - Utc::now()).num_seconds().max(1),
        interval,
    }))
}

async fn token(
    State(state): State<AppState>,
    Form(form): Form<TokenForm>,
) -> Result<Json<TokenResponse>, Response> {
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
    Ok(Json(token_response(grant)))
}

async fn revoke(
    State(state): State<AppState>,
    Form(form): Form<RevokeForm>,
) -> Result<StatusCode, Response> {
    let (database, _) = database(&state)
        .await
        .map_err(IntoResponse::into_response)?;
    database.revoke_refresh(&form.token).await.map_err(|e| {
        oauth_error(
            StatusCode::SERVICE_UNAVAILABLE,
            "server_error",
            &e.to_string(),
        )
    })?;
    Ok(StatusCode::NO_CONTENT)
}

async fn register(
    State(state): State<AppState>,
    Json(request): Json<RegisterRequest>,
) -> Result<(StatusCode, Json<TokenResponse>), ApiError> {
    let (database, config) = database(&state).await?;
    if !config.allow_registration {
        return Err(ApiError::forbidden("registration is disabled"));
    }
    let user = database
        .create_user(
            &request.email,
            &request.password,
            request.display_name.as_deref(),
            false,
        )
        .await
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let grant = database
        .issue_grant(user, "web", "board:operate offline_access", &config)
        .await?;
    Ok((StatusCode::CREATED, Json(token_response(grant))))
}

async fn login(
    State(state): State<AppState>,
    Json(request): Json<LoginRequest>,
) -> Result<Json<TokenResponse>, ApiError> {
    let (database, config) = database(&state).await?;
    let user = database
        .authenticate_password(&request.email, &request.password)
        .await?
        .ok_or_else(|| ApiError::unauthorized("email or password is incorrect"))?;
    Ok(Json(token_response(
        database
            .issue_grant(user, "web", "board:operate offline_access", &config)
            .await?,
    )))
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
    let expires_at = request
        .expires_in_days
        .map(|days| Utc::now() + chrono::Duration::days(i64::from(days.max(1))));
    let token = database
        .create_personal_access_token(user.id, &request.name, expires_at)
        .await?;
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
