use super::{
    crypto,
    store::{self, ClientContext},
};
use crate::{
    AppState,
    api::{auth::CurrentUser, error::ApiError},
    config::ServerConfig,
    persistence::postgres::{AccessGrant, Credential},
};
use axum::{
    body::Body,
    extract::{Extension, State},
    http::{HeaderMap, HeaderValue, Method, Request, header},
    middleware::Next,
    response::{IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    Admin,
    Website,
}
impl Surface {
    pub fn client(self) -> &'static str {
        match self {
            Self::Admin => "admin-web",
            Self::Website => "web",
        }
    }
    pub fn from_request(headers: &HeaderMap, path: &str, query: Option<&str>) -> Self {
        if path.starts_with("/api/v1/admin/")
            || headers
                .get("x-ostool-surface")
                .is_some_and(|v| v == "admin")
            || query.is_some_and(|q| q.split('&').any(|p| p == "surface=admin"))
        {
            Self::Admin
        } else {
            Self::Website
        }
    }
    pub fn cookie_name(self, refresh: bool, secure: bool) -> String {
        format!(
            "{}ostool-{}-{}",
            if secure { "__Host-" } else { "" },
            if self == Self::Admin {
                "admin"
            } else {
                "website"
            },
            if refresh { "refresh" } else { "access" }
        )
    }
}
#[derive(Clone, Debug)]
pub struct CurrentCredential(pub Credential);
#[derive(Clone)]
pub struct CspNonce(pub String);
pub fn secure(config: &ServerConfig) -> bool {
    config
        .auth
        .public_base_url
        .as_deref()
        .is_some_and(|url| url.starts_with("https://"))
}
pub fn cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    let mut found = None;
    for value in headers.get_all(header::COOKIE) {
        for pair in value.to_str().ok()?.split(';') {
            let (key, value) = pair.trim().split_once('=')?;
            if key == name {
                if found.is_some() {
                    return None;
                }
                found = Some(value);
            }
        }
    }
    found
}
pub async fn browser_grant(
    state: &AppState,
    grant: AccessGrant,
    surface: Surface,
    client: &ClientContext,
) -> Result<Response, ApiError> {
    sqlx::query("UPDATE oauth_sessions SET ip=$2,user_agent=$3,step_up_until=now()+interval '3 minutes' WHERE id=$1").bind(grant.session_id).bind(&client.ip).bind(&client.user_agent).execute(state.database.pool()).await?;
    sqlx::query("UPDATE users SET last_login_at=now(),last_login_ip=$2 WHERE id=$1")
        .bind(grant.user.id)
        .bind(&client.ip)
        .execute(state.database.pool())
        .await?;
    cookie_grant(state, grant, surface).await
}
pub async fn refreshed_grant(
    state: &AppState,
    grant: AccessGrant,
    surface: Surface,
    _client: &ClientContext,
) -> Result<Response, ApiError> {
    cookie_grant(state, grant, surface).await
}
async fn cookie_grant(
    state: &AppState,
    grant: AccessGrant,
    surface: Surface,
) -> Result<Response, ApiError> {
    let config = state.config.read().await;
    let mut response = json_response(
        json!({"authenticated":true,"enrollment_required":grant.scope=="mfa:enroll"}),
    );
    let secure = secure(&config);
    set_cookie(
        &mut response,
        &surface.cookie_name(false, secure),
        &grant.access_token,
        (grant.access_expires_at - chrono::Utc::now())
            .num_seconds()
            .max(1),
        secure,
    );
    set_cookie(
        &mut response,
        &surface.cookie_name(true, secure),
        &grant.refresh_token,
        (grant.refresh_expires_at - chrono::Utc::now())
            .num_seconds()
            .max(1),
        secure,
    );
    response.extensions_mut().insert(CurrentUser(grant.user));
    Ok(response)
}
pub fn json_response(value: serde_json::Value) -> Response {
    axum::Json(value).into_response()
}
pub fn set_cookie(response: &mut Response, name: &str, value: &str, seconds: i64, secure: bool) {
    let cookie = format!(
        "{name}={value}; Path=/; HttpOnly; SameSite=Strict; Max-Age={seconds}{}",
        if secure { "; Secure" } else { "" }
    );
    response.headers_mut().append(
        header::SET_COOKIE,
        cookie.parse().expect("server-generated cookie"),
    );
}
pub async fn clear_cookies(state: &AppState, surface: Surface) -> Response {
    let mut response = axum::http::StatusCode::NO_CONTENT.into_response();
    let secure = secure(&*state.config.read().await);
    for refresh in [false, true] {
        set_cookie(
            &mut response,
            &surface.cookie_name(refresh, secure),
            "",
            0,
            secure,
        );
    }
    response
}
pub fn require_step_up(credential: &Credential) -> Result<(), ApiError> {
    // The admin console relies on its authenticated administrator session.
    // Use the stored client identity, never the request's surface header.
    if credential.user.is_admin
        && credential.client_id == Surface::Admin.client()
        && credential.session_id.is_some()
    {
        return Ok(());
    }
    if credential.session_id.is_some()
        && credential
            .step_up_until
            .is_some_and(|until| until > chrono::Utc::now())
    {
        Ok(())
    } else {
        Err(ApiError::new(
            axum::http::StatusCode::FORBIDDEN,
            "reauth_required",
            "敏感操作需要重新验证当前账号",
        ))
    }
}
pub fn sensitive(method: &Method, path: &str) -> bool {
    (method == Method::POST
        && matches!(
            path,
            "/api/v1/account/password"
                | "/api/v1/auth/personal-access-tokens"
                | "/api/v1/auth/device/authorize"
        ))
        || (path.starts_with("/api/v1/auth/security/") && method != Method::GET)
}
/// Context/security headers are common; all identity restrictions remain public-only.
pub async fn transport(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Response {
    let trusted = state.config.read().await.audit.trusted_proxies.clone();
    let peer = request
        .extensions()
        .get::<axum::extract::ConnectInfo<std::net::SocketAddr>>()
        .map(|peer| peer.0.ip());
    let client = ClientContext::new(
        crate::audit::client_ip(peer, request.headers(), &trusted)
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "unknown".into()),
        request.headers(),
    );
    request.extensions_mut().insert(client);
    if (request.uri().path().starts_with("/api/v1/auth/")
        || request.uri().path().starts_with("/api/v1/admin/auth/")
        || request.uri().path().starts_with("/oauth/"))
        && !matches!(*request.method(), Method::GET | Method::HEAD)
    {
        let (parts, body) = request.into_parts();
        request = match axum::body::to_bytes(body, 16 * 1024).await {
            Ok(bytes) => Request::from_parts(parts, Body::from(bytes)),
            Err(_) => return ApiError::payload_too_large("认证请求过大").into_response(),
        };
    }
    let nonce = crypto::random_token();
    request.extensions_mut().insert(CspNonce(nonce.clone()));
    let path = request.uri().path().to_owned();
    let mut response = next.run(request).await;
    let config = state.config.read().await;
    let cloudflare = config.security.captcha_provider == super::config::CaptchaProvider::Turnstile;
    let third = if cloudflare {
        " https://challenges.cloudflare.com"
    } else {
        ""
    };
    let websocket = config
        .auth
        .public_base_url
        .iter()
        .chain(&config.auth.allowed_origins)
        .filter_map(|value| url::Url::parse(value).ok())
        .map(|mut url| {
            let _ = url.set_scheme(if url.scheme() == "https" { "wss" } else { "ws" });
            url.origin().ascii_serialization()
        })
        .collect::<Vec<_>>()
        .join(" ");
    let policy = if path.starts_with("/share/") || path.starts_with("/boot/") {
        "sandbox; default-src 'none'".to_owned()
    } else {
        format!(
            "default-src 'self'; script-src 'self' 'nonce-{nonce}'{third}; style-src 'self' 'unsafe-inline'; img-src 'self' data:; font-src 'self' data:; connect-src 'self' {websocket}{third}; frame-src{frames}; frame-ancestors 'none'; base-uri 'self'; form-action 'self'; object-src 'none'",
            frames = if cloudflare {
                " https://challenges.cloudflare.com"
            } else {
                " 'none'"
            }
        )
    };
    for (name, value) in [
        ("content-security-policy", policy),
        ("x-content-type-options", "nosniff".into()),
        ("x-frame-options", "DENY".into()),
        ("referrer-policy", "no-referrer".into()),
        (
            "permissions-policy",
            "camera=(), microphone=(), geolocation=(), payment=(), usb=()".into(),
        ),
        ("cross-origin-opener-policy", "same-origin".into()),
    ] {
        response.headers_mut().insert(
            name,
            HeaderValue::from_str(&value).expect("fixed response policy"),
        );
    }
    if secure(&config) {
        response.headers_mut().insert(
            "strict-transport-security",
            HeaderValue::from_static("max-age=31536000"),
        );
    }
    if path.starts_with("/api/") || path.starts_with("/oauth/") {
        response
            .headers_mut()
            .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
        response
            .headers_mut()
            .insert("pragma", HeaderValue::from_static("no-cache"));
    }
    response
}
pub async fn gate(
    State(state): State<AppState>,
    Extension(client): Extension<ClientContext>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let config = state.config.read().await.clone();
    if !config.edition.is_public() {
        return next.run(request).await;
    }
    let path = request.uri().path();
    if path.starts_with("/api/v1/loaders/") || path.starts_with("/api/v1/loader-updates/") {
        return ApiError::forbidden("装载器接口仅通过独立设备入口访问").into_response();
    }
    if (path.starts_with("/admin") || path.starts_with("/api/v1/admin/"))
        && !config.security.admin_allowed_networks.is_empty()
        && !client.ip.parse::<std::net::IpAddr>().is_ok_and(|ip| {
            config
                .security
                .admin_allowed_networks
                .iter()
                .filter_map(|n| n.parse::<ipnet::IpNet>().ok())
                .any(|n| n.contains(&ip))
        })
    {
        return ApiError::forbidden("管理后台访问地址不在允许范围内").into_response();
    }
    if matches!(
        *request.method(),
        Method::POST | Method::PUT | Method::PATCH | Method::DELETE
    ) || request.headers().contains_key("upgrade")
    {
        if let Some(origin) = request.headers().get(header::ORIGIN) {
            let allowed = config
                .auth
                .public_base_url
                .iter()
                .chain(&config.auth.allowed_origins)
                .filter_map(|value| url::Url::parse(value).ok())
                .any(|value| {
                    Some(value.origin().ascii_serialization().as_str()) == origin.to_str().ok()
                });
            if !allowed {
                return ApiError::forbidden("请求来源不受信任").into_response();
            }
        } else if request
            .headers()
            .get(header::COOKIE)
            .is_some_and(|value| value.as_bytes().windows(7).any(|part| part == b"ostool-"))
            && !request.headers().contains_key(header::AUTHORIZATION)
        {
            return ApiError::forbidden("缺少请求来源验证").into_response();
        } else if request
            .headers()
            .get("sec-fetch-site")
            .is_some_and(|value| value != "same-origin" && value != "none")
        {
            return ApiError::forbidden("请求来源不受信任").into_response();
        }
    }
    if (path.starts_with("/api/v1/auth/")
        || path.starts_with("/api/v1/admin/auth/")
        || path.starts_with("/oauth/"))
        && !matches!(*request.method(), Method::GET | Method::HEAD)
        && let Err(error) = store::limit(&state.security_cache, "auth-ip", &client.ip, 90, 60).await
    {
        return error.into_response();
    }
    next.run(request).await
}
