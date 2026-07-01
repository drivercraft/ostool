use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
};

use axum::{
    Router,
    body::{Bytes, to_bytes},
    extract::{ConnectInfo, Path, Query, Request, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderName, HeaderValue, Method, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use chrono::Utc;
use futures_util::future::join_all;
use httpboot_protocol::{BootArch, ImageFormat};
use mime_guess::from_path;
use rand_core::{OsRng, RngCore};
use serde::Deserialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
};
use uuid::Uuid;

use crate::{
    api::{
        dto::{
            ActionResponse, AdminAuditLogResponse, AdminAuditLogsResponse, AdminBoardUpsertRequest,
            AdminLeaseCreateRequest, AdminLeaseUpdateRequest, AdminOverviewResponse,
            AdminPasswordResetRequest, AdminPermissionResponse, AdminPermissionsResponse,
            AdminRoleCreateRequest, AdminRoleDisableRequest, AdminRoleResponse,
            AdminRoleUpdateRequest, AdminRolesResponse, AdminServerConfigEditable,
            AdminServerConfigReadonly, AdminServerConfigResponse, AdminSessionResponse,
            AdminSessionUpdateRequest, AdminSessionsResponse, AdminTftpConfigResponse,
            AdminTftpStatusResponse, AdminUserCreateRequest, AdminUserResponse,
            AdminUserRolesResponse, AdminUserRolesUpdateRequest, AdminUserUpdateRequest,
            AdminUsersResponse, BoardPowerAction, BoardPowerStatusResponse,
            BoardRuntimeStatusResponse, BoardTypeSummary, BootProfileResponse, CaptchaResponse,
            CreateLeaseRequest, CreateSessionRequest, CurrentUserResponse, DtbFileResponse,
            FileResponse, HttpBootFileResponse, KernelPublishResponse, LeaseResponse,
            LeasesResponse, LoginRequest, NetworkInterfaceSummary, RegisterRequest,
            RegisterResponse, RegistrationPolicyResponse, SerialPortSummary, SerialStatusResponse,
            SessionCreatedResponse, SessionDetailResponse, SessionDtbResponse,
            SiteSettingsResponse, SiteSettingsUpdateRequest, TftpSessionResponse,
            UpdateServerConfigRequest, UserPasswordUpdateRequest,
        },
        error::ApiError,
    },
    auth::{
        CurrentUser, clear_cookie_value, cookie_value, hash_password, set_cookie_header,
        token_from_headers, verify_password,
    },
    board_pool::BoardAllocationStatus,
    config::{BoardConfig, BootConfig, PowerManagementConfig, ServerConfig, UbootNetworkMode},
    dtb_store::normalize_dtb_name,
    http_boot::publish::{KernelPublishInput, publish_kernel},
    lease::{create_user_lease, release_lease},
    power::{PowerAction, PowerActionError},
    serial::{
        discovery::list_serial_ports as discover_serial_ports,
        discovery::resolve_serial_config,
        network::{
            default_non_loopback_interface_name,
            list_network_interfaces as discover_network_interfaces,
        },
        ws::run_serial_ws,
    },
    session::SessionState,
    session::SessionStopReason,
    state::{AppState, BoardLeaseState, CaptchaChallenge, RateLimitWindow, TouchSessionError},
    storage::{
        Lease, NewAuditLog, NewRole, Role, RuntimeSettings, SiteSettings, UpsertDtbMetadata,
        UserProfile, default_user_permission,
    },
    tftp::{
        files::{TftpFileRef, normalize_relative_path},
        status::resolve_interface_ipv4,
    },
    validation,
    web::{
        serve_admin_asset, serve_admin_history, serve_admin_index, serve_asset,
        serve_history_fallback, serve_index,
    },
};

const DTB_UPLOAD_MAX_MIB: u32 = 10;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(serve_index))
        .route("/assets/{*path}", get(serve_asset))
        .route("/admin", get(serve_admin_index))
        .route("/admin/", get(serve_admin_index))
        .route("/admin/assets/{*path}", get(serve_admin_asset))
        .route("/admin/{*path}", get(serve_admin_history))
        .route(
            "/boot/sessions/{session_id}/{*path}",
            get(get_http_boot_file),
        )
        .route("/api/v1/admin/overview", get(get_admin_overview))
        .route("/api/v1/auth/login", post(login))
        .route("/api/v1/auth/captcha", get(get_captcha))
        .route("/api/v1/auth/register", post(register))
        .route("/api/v1/auth/registration-policy", get(registration_policy))
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(get_current_user))
        .route("/api/v1/user/profile", get(get_user_profile))
        .route("/api/v1/user/password", post(update_user_password))
        .route(
            "/api/v1/user/leases/availability",
            get(list_user_lease_availability),
        )
        .route(
            "/api/v1/user/leases",
            get(list_user_leases).post(create_lease),
        )
        .route("/api/v1/user/leases/{lease_id}", delete(delete_user_lease))
        .route(
            "/api/v1/user/leases/{lease_id}/heartbeat",
            post(heartbeat_user_lease),
        )
        .route(
            "/api/v1/admin/users",
            get(list_admin_users).post(create_admin_user),
        )
        .route(
            "/api/v1/admin/users/{user_id}",
            get(get_admin_user)
                .put(update_admin_user)
                .delete(delete_admin_user),
        )
        .route(
            "/api/v1/admin/users/{user_id}/roles",
            get(get_admin_user_roles).put(update_admin_user_roles),
        )
        .route(
            "/api/v1/admin/users/{user_id}/reset-password",
            post(reset_admin_user_password),
        )
        .route(
            "/api/v1/admin/users/{user_id}/disable",
            post(disable_admin_user),
        )
        .route(
            "/api/v1/admin/users/{user_id}/approve",
            post(approve_admin_user),
        )
        .route(
            "/api/v1/admin/users/{user_id}/reject",
            post(reject_admin_user),
        )
        .route("/api/v1/admin/users/pending", get(list_pending_admin_users))
        .route("/api/v1/admin/audit-logs", get(list_admin_audit_logs))
        .route(
            "/api/v1/admin/leases",
            get(list_admin_leases).post(create_admin_lease),
        )
        .route(
            "/api/v1/admin/leases/{lease_id}",
            get(get_admin_lease)
                .put(update_admin_lease)
                .delete(delete_admin_lease),
        )
        .route(
            "/api/v1/admin/leases/{lease_id}/session",
            post(start_admin_lease_session),
        )
        .route(
            "/api/v1/admin/leases/{lease_id}/release",
            post(release_admin_lease),
        )
        .route("/api/v1/admin/permissions", get(list_admin_permissions))
        .route(
            "/api/v1/admin/roles",
            get(list_admin_roles).post(create_admin_role),
        )
        .route(
            "/api/v1/admin/roles/{role_id}",
            get(get_admin_role)
                .put(update_admin_role)
                .delete(delete_admin_role),
        )
        .route(
            "/api/v1/admin/roles/{role_id}/disable",
            post(disable_admin_role),
        )
        .route("/api/v1/admin/boards", get(list_boards).post(create_board))
        .route("/api/v1/admin/dtbs", get(list_dtbs).post(create_dtb))
        .route("/api/v1/admin/serial-ports", get(list_serial_ports))
        .route(
            "/api/v1/admin/network-interfaces",
            get(list_network_interfaces),
        )
        .route(
            "/api/v1/admin/boards/{board_id}/power-status",
            get(get_board_power_status),
        )
        .route(
            "/api/v1/admin/boards/{board_id}/runtime-status",
            get(get_board_runtime_status),
        )
        .route(
            "/api/v1/admin/boards/{board_id}",
            get(get_board).put(update_board).delete(delete_board),
        )
        .route(
            "/api/v1/admin/dtbs/{dtb_name}",
            get(get_dtb).put(update_dtb).delete(delete_dtb),
        )
        .route("/api/v1/admin/sessions", get(list_admin_sessions))
        .route(
            "/api/v1/admin/sessions/{session_id}",
            put(update_admin_session).delete(delete_admin_session),
        )
        .route(
            "/api/v1/admin/sessions/{session_id}/close",
            post(close_admin_session),
        )
        .route("/api/v1/admin/tftp", get(get_tftp_config))
        .route("/api/v1/admin/tftp/status", get(get_tftp_status))
        .route("/api/v1/admin/tftp/reconcile", post(reconcile_tftp))
        .route(
            "/api/v1/admin/server-config",
            get(get_server_config).put(update_server_config),
        )
        .route(
            "/api/v1/admin/site-settings",
            get(get_site_settings).put(update_site_settings),
        )
        .route("/api/v1/board-types", get(list_board_types))
        .route("/api/v1/sessions", post(create_session))
        .route(
            "/api/v1/sessions/{session_id}",
            get(get_session).delete(delete_session),
        )
        .route(
            "/api/v1/sessions/{session_id}/heartbeat",
            post(heartbeat_session),
        )
        .route(
            "/api/v1/sessions/{session_id}/boot-profile",
            get(get_boot_profile),
        )
        .route(
            "/api/v1/sessions/{session_id}/http-boot/files",
            put(put_http_boot_file),
        )
        .route(
            "/api/v1/sessions/{session_id}/http-boot/kernel",
            put(put_http_boot_kernel),
        )
        .route("/api/v1/sessions/{session_id}/dtb", get(get_session_dtb))
        .route(
            "/api/v1/sessions/{session_id}/dtb/download",
            get(download_session_dtb),
        )
        .route(
            "/api/v1/sessions/{session_id}/serial",
            get(get_serial_status),
        )
        .route("/api/v1/sessions/{session_id}/serial/ws", get(serial_ws))
        .route(
            "/api/v1/sessions/{session_id}/board/power-on",
            post(power_on_board),
        )
        .route(
            "/api/v1/sessions/{session_id}/board/power-off",
            post(power_off_board),
        )
        .route(
            "/api/v1/sessions/{session_id}/files",
            get(list_session_files).put(put_session_file),
        )
        .route(
            "/api/v1/sessions/{session_id}/files/{*path}",
            put(reject_legacy_put_session_file)
                .get(get_session_file)
                .delete(delete_session_file),
        )
        .route(
            "/api/v1/sessions/{session_id}/tftp",
            get(get_session_tftp_status),
        )
        .route("/{*path}", get(serve_history_fallback))
        .layer(middleware::from_fn_with_state(state.clone(), auth_gate))
        .layer(middleware::from_fn_with_state(state.clone(), security_gate))
        .with_state(state)
}

async fn security_gate(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    enforce_same_origin(&headers, &request)?;
    enforce_rate_limit(
        &state,
        addr,
        &headers,
        request.method(),
        request.uri().path(),
    )
    .await?;
    let mut response = next.run(request).await;
    set_security_headers(response.headers_mut());
    Ok(response)
}

#[derive(Clone, Copy)]
struct RateLimitRule {
    name: &'static str,
    max_requests: u32,
    window_seconds: i64,
}

fn rate_limit_rule(method: &Method, path: &str) -> Option<RateLimitRule> {
    if !path.starts_with("/api/") {
        return None;
    }
    if method == Method::GET && path == "/api/v1/auth/captcha" {
        return Some(RateLimitRule {
            name: "captcha",
            max_requests: 20,
            window_seconds: 60,
        });
    }
    if method == Method::POST && path == "/api/v1/auth/login" {
        return Some(RateLimitRule {
            name: "login",
            max_requests: 10,
            window_seconds: 60,
        });
    }
    if method == Method::POST && path == "/api/v1/user/password" {
        return Some(RateLimitRule {
            name: "password",
            max_requests: 10,
            window_seconds: 60,
        });
    }
    if matches!(method.as_str(), "POST" | "PUT")
        && (path.starts_with("/api/v1/admin/dtbs")
            || path.contains("/http-boot/")
            || path.contains("/files"))
    {
        return Some(RateLimitRule {
            name: "upload",
            max_requests: 20,
            window_seconds: 60,
        });
    }
    Some(RateLimitRule {
        name: "api",
        max_requests: 300,
        window_seconds: 60,
    })
}

async fn enforce_rate_limit(
    state: &AppState,
    addr: SocketAddr,
    headers: &HeaderMap,
    method: &Method,
    path: &str,
) -> Result<(), ApiError> {
    let Some(rule) = rate_limit_rule(method, path) else {
        return Ok(());
    };
    let key = format!(
        "{}:{}:{}",
        addr.ip(),
        rule.name,
        token_from_headers(headers).unwrap_or_default()
    );
    let now = Utc::now();
    let mut limits = state.rate_limits.write().await;
    limits.retain(|_, window| window.reset_at > now);
    let window = limits.entry(key).or_insert_with(|| RateLimitWindow {
        count: 0,
        reset_at: now + chrono::Duration::seconds(rule.window_seconds),
    });
    if window.count >= rule.max_requests {
        return Err(ApiError::too_many_requests("请求过于频繁，请稍后再试"));
    }
    window.count += 1;
    Ok(())
}

fn enforce_same_origin(headers: &HeaderMap, request: &Request) -> Result<(), ApiError> {
    if matches!(
        *request.method(),
        Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE
    ) {
        return Ok(());
    }
    let Some(host) = headers
        .get(header::HOST)
        .and_then(|value| value.to_str().ok())
    else {
        return Ok(());
    };
    if let Some(origin) = headers
        .get(header::ORIGIN)
        .and_then(|value| value.to_str().ok())
    {
        if host_matches_origin(host, origin) {
            return Ok(());
        }
        return Err(ApiError::forbidden("跨站请求被拒绝"));
    }
    if let Some(referer) = headers
        .get(header::REFERER)
        .and_then(|value| value.to_str().ok())
        && !host_matches_origin(host, referer)
    {
        return Err(ApiError::forbidden("跨站请求被拒绝"));
    }
    Ok(())
}

fn host_matches_origin(host: &str, value: &str) -> bool {
    let origin_host = value
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(value)
        .split('/')
        .next()
        .unwrap_or_default();
    origin_host.eq_ignore_ascii_case(host)
}

fn set_security_headers(headers: &mut HeaderMap) {
    let values = [
        (
            HeaderName::from_static("x-content-type-options"),
            HeaderValue::from_static("nosniff"),
        ),
        (
            HeaderName::from_static("x-frame-options"),
            HeaderValue::from_static("DENY"),
        ),
        (
            HeaderName::from_static("referrer-policy"),
            HeaderValue::from_static("strict-origin-when-cross-origin"),
        ),
        (
            HeaderName::from_static("permissions-policy"),
            HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
        ),
        (
            HeaderName::from_static("content-security-policy"),
            HeaderValue::from_static(
                "default-src 'self'; img-src 'self' data:; style-src 'self' 'unsafe-inline'; script-src 'self'; connect-src 'self' ws: wss:; base-uri 'self'; frame-ancestors 'none'",
            ),
        ),
    ];
    for (name, value) in values {
        headers.entry(name).or_insert(value);
    }
}

async fn auth_gate(
    State(state): State<AppState>,
    headers: HeaderMap,
    request: Request,
    next: Next,
) -> Result<Response, ApiError> {
    let path = request.uri().path();
    if path.starts_with("/api/v1/admin/") {
        if state.auth.user_count().await.map_err(ApiError::from)? == 0 {
            return Ok(next.run(request).await);
        }
        let user = current_user_from_headers(&state, &headers).await?;
        if !current_user_is_admin(&user) {
            return Err(ApiError::forbidden("administrator role required"));
        }
        if let Some(permission_code) = admin_permission_for_request(request.method(), path) {
            require_permission(&user, permission_code)?;
        }
    } else if path.starts_with("/api/v1/user/") {
        let user = current_user_from_headers(&state, &headers).await?;
        if let Some(permission_code) = user_permission_for_request(request.method(), path) {
            require_permission(&user, permission_code)?;
        }
    }
    Ok(next.run(request).await)
}

async fn current_user_from_headers(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<CurrentUser, ApiError> {
    let token = token_from_headers(headers)
        .ok_or_else(|| ApiError::unauthorized("authentication required"))?;
    state
        .auth
        .user_for_token(&token)
        .await
        .map_err(ApiError::from)?
        .ok_or_else(|| ApiError::unauthorized("authentication required"))
}

fn user_response(user: CurrentUser) -> CurrentUserResponse {
    let permissions = user
        .permissions
        .into_iter()
        .map(AdminPermissionResponse::from)
        .collect();
    let roles = user
        .roles
        .into_iter()
        .map(|role| AdminRoleResponse::new(role, Vec::new(), 0))
        .collect();
    CurrentUserResponse {
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
    }
}

fn current_user_is_admin(user: &CurrentUser) -> bool {
    user.roles.iter().any(|role| role.name == "admin")
        || user.permissions.iter().any(|permission| {
            is_admin_permission(&permission.code) && !default_user_permission(&permission.code)
        })
}

fn is_admin_permission(permission_code: &str) -> bool {
    permission_code.split_once('.').is_some_and(|(module, _)| {
        matches!(
            module,
            "overview"
                | "users"
                | "roles"
                | "boards"
                | "dtbs"
                | "leases"
                | "sessions"
                | "tftp"
                | "server"
                | "site"
                | "serial_ports"
                | "network_interfaces"
                | "permissions"
        )
    })
}

fn user_has_permission(user: &CurrentUser, permission_code: &str) -> bool {
    if user.roles.iter().any(|role| role.name == "admin") {
        return true;
    }
    user.permissions
        .iter()
        .any(|permission| permission.code == permission_code)
}

fn require_permission(user: &CurrentUser, permission_code: &str) -> Result<(), ApiError> {
    if user_has_permission(user, permission_code) {
        Ok(())
    } else {
        Err(ApiError::forbidden(format!(
            "permission `{permission_code}` required"
        )))
    }
}

fn admin_permission_for_request(method: &Method, path: &str) -> Option<&'static str> {
    let segments = path
        .trim_start_matches("/api/v1/admin/")
        .split('/')
        .collect::<Vec<_>>();
    let first = *segments.first()?;
    match first {
        "overview" if method == Method::GET => Some("overview.read"),
        "permissions" if method == Method::GET => Some("permissions.read"),
        "users" => admin_users_permission(method, &segments),
        "roles" => admin_roles_permission(method, &segments),
        "leases" => admin_leases_permission(method, &segments),
        "sessions" => admin_sessions_permission(method, &segments),
        "boards" => admin_boards_permission(method, &segments),
        "dtbs" => crud_permission(method, "dtbs"),
        "serial-ports" if method == Method::GET => Some("serial_ports.read"),
        "network-interfaces" if method == Method::GET => Some("network_interfaces.read"),
        "tftp" => admin_tftp_permission(method, &segments),
        "server-config" => read_update_permission(method, "server"),
        "site-settings" => read_update_permission(method, "site"),
        "audit-logs" if method == Method::GET => Some("audit.read"),
        _ => None,
    }
}

fn user_permission_for_request(method: &Method, path: &str) -> Option<&'static str> {
    let segments = path
        .trim_start_matches("/api/v1/user/")
        .split('/')
        .collect::<Vec<_>>();
    match (method.as_str(), segments.as_slice()) {
        ("POST", ["password"]) => Some("profile.update"),
        ("GET", ["leases"]) => Some("leases.read"),
        ("GET", ["leases", "availability"]) => Some("leases.read"),
        ("POST", ["leases"]) => Some("leases.create"),
        ("DELETE", ["leases", _]) => Some("leases.release"),
        ("POST", ["leases", _, "heartbeat"]) => Some("leases.heartbeat"),
        _ => None,
    }
}

fn crud_permission(method: &Method, resource: &'static str) -> Option<&'static str> {
    match method.as_str() {
        "GET" => Some(match resource {
            "roles" => "roles.read",
            "dtbs" => "dtbs.read",
            _ => return None,
        }),
        "POST" => Some(match resource {
            "roles" => "roles.create",
            "dtbs" => "dtbs.create",
            _ => return None,
        }),
        "PUT" => Some(match resource {
            "roles" => "roles.update",
            "dtbs" => "dtbs.update",
            _ => return None,
        }),
        "DELETE" => Some(match resource {
            "roles" => "roles.delete",
            "dtbs" => "dtbs.delete",
            _ => return None,
        }),
        _ => None,
    }
}

fn read_update_permission(method: &Method, resource: &'static str) -> Option<&'static str> {
    match method.as_str() {
        "GET" => Some(match resource {
            "server" => "server.read",
            "site" => "site.read",
            _ => return None,
        }),
        "PUT" => Some(match resource {
            "server" => "server.update",
            "site" => "site.update",
            _ => return None,
        }),
        _ => None,
    }
}

fn admin_users_permission(method: &Method, segments: &[&str]) -> Option<&'static str> {
    match (method.as_str(), segments) {
        ("GET", ["users"]) => Some("users.read"),
        ("GET", ["users", "pending"]) => Some("users.read"),
        ("GET", ["users", _]) => Some("users.read"),
        ("POST", ["users"]) => Some("users.create"),
        ("PUT", ["users", _]) => Some("users.update"),
        ("DELETE", ["users", _]) => Some("users.delete"),
        ("GET", ["users", _, "roles"]) => Some("users.read"),
        ("PUT", ["users", _, "roles"]) => Some("users.update"),
        ("POST", ["users", _, "reset-password"]) => Some("users.password.update"),
        ("POST", ["users", _, "disable"]) => Some("users.update"),
        ("POST", ["users", _, "approve"]) => Some("users.update"),
        ("POST", ["users", _, "reject"]) => Some("users.update"),
        _ => None,
    }
}

fn admin_leases_permission(method: &Method, segments: &[&str]) -> Option<&'static str> {
    match (method.as_str(), segments) {
        ("GET", ["leases"]) | ("GET", ["leases", _]) => Some("leases.read"),
        ("POST", ["leases"]) => Some("leases.create"),
        ("PUT", ["leases", _]) => Some("leases.update"),
        ("DELETE", ["leases", _]) => Some("leases.delete"),
        ("POST", ["leases", _, "session"]) => Some("leases.start"),
        ("POST", ["leases", _, "release"]) => Some("leases.release"),
        _ => None,
    }
}

fn admin_roles_permission(method: &Method, segments: &[&str]) -> Option<&'static str> {
    match (method.as_str(), segments) {
        ("GET", ["roles"]) | ("GET", ["roles", _]) => Some("roles.read"),
        ("POST", ["roles"]) => Some("roles.create"),
        ("PUT", ["roles", _]) => Some("roles.update"),
        ("POST", ["roles", _, "disable"]) => Some("roles.update"),
        ("DELETE", ["roles", _]) => Some("roles.delete"),
        _ => None,
    }
}

fn admin_sessions_permission(method: &Method, segments: &[&str]) -> Option<&'static str> {
    match (method.as_str(), segments) {
        ("GET", ["sessions"]) | ("GET", ["sessions", _]) => Some("sessions.read"),
        ("PUT", ["sessions", _]) => Some("sessions.update"),
        ("POST", ["sessions", _, "close"]) => Some("sessions.delete"),
        ("DELETE", ["sessions", _]) => Some("sessions.delete"),
        _ => None,
    }
}

fn admin_boards_permission(method: &Method, segments: &[&str]) -> Option<&'static str> {
    match (method.as_str(), segments) {
        ("GET", ["boards"])
        | ("GET", ["boards", _])
        | ("GET", ["boards", _, "power-status"])
        | ("GET", ["boards", _, "runtime-status"]) => Some("boards.read"),
        ("POST", ["boards"]) => Some("boards.create"),
        ("PUT", ["boards", _]) => Some("boards.update"),
        ("DELETE", ["boards", _]) => Some("boards.delete"),
        _ => None,
    }
}

fn admin_tftp_permission(method: &Method, segments: &[&str]) -> Option<&'static str> {
    match (method.as_str(), segments) {
        ("GET", ["tftp"]) | ("GET", ["tftp", "status"]) => Some("tftp.read"),
        ("POST", ["tftp", "reconcile"]) => Some("tftp.reconcile"),
        _ => None,
    }
}

fn admin_user_response(user: crate::storage::User) -> AdminUserResponse {
    AdminUserResponse {
        id: user.id,
        username: user.username,
        display_name: user.display_name,
        nickname: user.nickname,
        avatar_url: user.avatar_url,
        email: user.email,
        phone: user.phone,
        department: user.department,
        title: user.title,
        disabled: user.disabled,
        status: user.status.as_str().to_string(),
        last_login_at: user.last_login_at,
        created_at: user.created_at,
        updated_at: user.updated_at,
    }
}

fn clean_optional(value: Option<String>) -> Option<String> {
    value.and_then(|value| {
        let trimmed = value.trim().to_string();
        (!trimmed.is_empty()).then_some(trimmed)
    })
}

fn validate_len(value: &str, field_name: &str, min: usize, max: usize) -> Result<(), ApiError> {
    let len = validation::char_len(value);
    if len < min || len > max {
        return Err(ApiError::bad_request(format!(
            "{field_name} length must be between {min} and {max} characters"
        )));
    }
    Ok(())
}

fn validate_max_len(value: &str, field_name: &str, max: usize) -> Result<(), ApiError> {
    let len = validation::char_len(value);
    if len > max {
        return Err(ApiError::bad_request(format!(
            "{field_name} length must be at most {max} characters"
        )));
    }
    Ok(())
}

fn clean_required_len(
    value: String,
    field_name: &str,
    min: usize,
    max: usize,
) -> Result<String, ApiError> {
    let value = value.trim().to_string();
    validate_len(&value, field_name, min, max)?;
    Ok(value)
}

fn clean_optional_len(
    value: Option<String>,
    field_name: &str,
    max: usize,
) -> Result<Option<String>, ApiError> {
    let value = clean_optional(value);
    if let Some(value) = value.as_deref() {
        validate_max_len(value, field_name, max)?;
    }
    Ok(value)
}

fn validate_username(value: &str) -> Result<(), ApiError> {
    validate_len(
        value,
        "username",
        validation::USERNAME_MIN_LEN,
        validation::USERNAME_MAX_LEN,
    )?;
    if !validation::valid_username(value) {
        return Err(ApiError::bad_request(
            "username must contain only letters, numbers, '_' or '-'",
        ));
    }
    Ok(())
}

fn validate_email(value: &str) -> Result<(), ApiError> {
    validate_len(
        value,
        "email",
        validation::EMAIL_MIN_LEN,
        validation::EMAIL_MAX_LEN,
    )?;
    let Some((local, domain)) = value.split_once('@') else {
        return Err(ApiError::bad_request("email must be a valid email address"));
    };
    if local.is_empty() || !domain.contains('.') || domain.ends_with('.') {
        return Err(ApiError::bad_request("email must be a valid email address"));
    }
    Ok(())
}

fn validate_password(value: &str) -> Result<(), ApiError> {
    validate_len(
        value,
        "password",
        validation::PASSWORD_MIN_LEN,
        validation::PASSWORD_MAX_LEN,
    )
}

fn request_profile_checked(
    nickname: Option<String>,
    avatar_url: Option<String>,
    phone: Option<String>,
    department: Option<String>,
    title: Option<String>,
) -> Result<UserProfile, ApiError> {
    Ok(UserProfile {
        nickname: clean_optional_len(nickname, "nickname", validation::DISPLAY_NAME_MAX_LEN)?,
        avatar_url: clean_optional_len(avatar_url, "avatar_url", validation::URL_MAX_LEN)?,
        phone: clean_optional_len(phone, "phone", validation::PHONE_MAX_LEN)?,
        department: clean_optional_len(department, "department", validation::DISPLAY_NAME_MAX_LEN)?,
        title: clean_optional_len(title, "title", validation::DISPLAY_NAME_MAX_LEN)?,
    })
}

#[derive(Clone, Default)]
struct DtbMetadataInput {
    boot_architecture: Option<String>,
    compatible: Option<String>,
    description: Option<String>,
    disabled: Option<bool>,
}

impl DtbMetadataInput {
    fn has_updates(&self) -> bool {
        self.boot_architecture.is_some()
            || self.compatible.is_some()
            || self.description.is_some()
            || self.disabled.is_some()
    }
}

async fn admin_role_response(state: &AppState, role: Role) -> Result<AdminRoleResponse, ApiError> {
    let permissions = state.storage.role_permissions(&role.id).await?;
    let user_count = state
        .storage
        .role_user_counts()
        .await?
        .get(&role.id)
        .copied()
        .unwrap_or(0);
    Ok(AdminRoleResponse::new(role, permissions, user_count))
}

async fn role_names_for_ids(
    state: &AppState,
    role_ids: Vec<String>,
) -> Result<Vec<String>, ApiError> {
    if role_ids.is_empty() {
        return Ok(vec!["user".to_string()]);
    }
    let roles = state.storage.list_roles().await?;
    let mut names = Vec::new();
    for role_id in role_ids {
        let role = roles
            .iter()
            .find(|item| item.id == role_id)
            .ok_or_else(|| ApiError::bad_request("unknown role id"))?;
        names.push(role.name.clone());
    }
    Ok(names)
}

async fn lease_response(state: &AppState, lease: Lease) -> LeaseResponse {
    let session = match lease.session_id.as_deref() {
        Some(session_id) => state.get_session(session_id).await,
        None => None,
    };
    LeaseResponse { lease, session }
}

const CAPTCHA_EXPIRES_IN_SECONDS: i64 = 300;

async fn get_captcha(
    State(state): State<AppState>,
) -> Result<axum::Json<CaptchaResponse>, ApiError> {
    remove_expired_captchas(&state).await;
    let token = Uuid::new_v4().to_string();
    let answer = captcha_answer();
    let image_svg = captcha_svg(&answer);
    state.captchas.write().await.insert(
        token.clone(),
        CaptchaChallenge {
            answer_hash: captcha_answer_hash(&answer),
            expires_at: Utc::now() + chrono::Duration::seconds(CAPTCHA_EXPIRES_IN_SECONDS),
        },
    );
    Ok(axum::Json(CaptchaResponse {
        token,
        image_svg,
        expires_in_seconds: CAPTCHA_EXPIRES_IN_SECONDS as u64,
    }))
}

async fn login(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<LoginRequest>,
) -> Result<Response, ApiError> {
    verify_captcha(&state, &request.captcha_token, &request.captcha_answer).await?;
    let (user, token) = state
        .auth
        .login(request.username.trim(), &request.password)
        .await
        .map_err(|err| {
            // Distinguish "pending/rejected" so the UI can guide the user.
            let message = err.to_string();
            if message.contains("status is `pending`") {
                ApiError::forbidden("账号正在等待管理员审核，审核通过后即可登录")
            } else if message.contains("status is `rejected`") {
                ApiError::forbidden("账号注册申请已被拒绝，请联系管理员")
            } else if message.contains("disabled") {
                ApiError::forbidden("账号已被禁用，请联系管理员")
            } else {
                ApiError::unauthorized("invalid username or password")
            }
        })?;
    let mut response = axum::Json(user_response(user)).into_response();
    set_cookie_header(response.headers_mut(), cookie_value(&token)).map_err(ApiError::from)?;
    Ok(response)
}

async fn registration_policy(
    State(state): State<AppState>,
) -> Result<axum::Json<RegistrationPolicyResponse>, ApiError> {
    let site = state.storage.get_site_settings().await?;
    Ok(axum::Json(RegistrationPolicyResponse {
        mode: site.registration_mode.as_str().to_string(),
        self_service_enabled: site.self_service_enabled,
    }))
}

async fn register(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<RegisterRequest>,
) -> Result<(StatusCode, axum::Json<RegisterResponse>), ApiError> {
    verify_captcha(&state, &request.captcha_token, &request.captcha_answer).await?;
    let username = clean_required_len(
        request.username,
        "username",
        validation::USERNAME_MIN_LEN,
        validation::USERNAME_MAX_LEN,
    )?;
    validate_username(&username)?;
    let display_name = clean_optional_len(
        request.display_name,
        "display_name",
        validation::DISPLAY_NAME_MAX_LEN,
    )?
    .unwrap_or_else(|| username.clone());
    validate_len(
        &display_name,
        "display_name",
        validation::DISPLAY_NAME_MIN_LEN,
        validation::DISPLAY_NAME_MAX_LEN,
    )?;
    let email = clean_required_len(
        request.email,
        "email",
        validation::EMAIL_MIN_LEN,
        validation::EMAIL_MAX_LEN,
    )?;
    validate_email(&email)?;
    validate_password(&request.password)?;
    if request.password != request.confirm_password {
        return Err(ApiError::bad_request("两次输入的密码不一致"));
    }
    let profile =
        request_profile_checked(None, None, request.phone, request.department, request.title)?;
    let outcome = state
        .auth
        .register_user(
            username.clone(),
            display_name.clone(),
            email,
            request.password,
            profile,
        )
        .await
        .map_err(|err| {
            // username uniqueness / DB errors → 409 conflict
            ApiError::conflict(err.to_string())
        })?;
    let (status_code, response) = match outcome {
        crate::auth::RegistrationOutcome::Closed => (StatusCode::OK, RegisterResponse::Closed),
        crate::auth::RegistrationOutcome::Active { .. } => (
            StatusCode::CREATED,
            RegisterResponse::Active {
                username,
                display_name,
            },
        ),
        crate::auth::RegistrationOutcome::Pending { .. } => (
            StatusCode::ACCEPTED,
            RegisterResponse::Pending {
                username,
                display_name,
            },
        ),
    };
    Ok((status_code, axum::Json(response)))
}

async fn remove_expired_captchas(state: &AppState) {
    let now = Utc::now();
    state
        .captchas
        .write()
        .await
        .retain(|_, challenge| challenge.expires_at > now);
}

async fn verify_captcha(state: &AppState, token: &str, answer: &str) -> Result<(), ApiError> {
    remove_expired_captchas(state).await;
    let Some(challenge) = state.captchas.write().await.remove(token) else {
        return Err(ApiError::bad_request("验证码已过期，请刷新后重试"));
    };
    if captcha_answer_hash(answer) == challenge.answer_hash {
        Ok(())
    } else {
        Err(ApiError::bad_request("验证码不正确，请刷新后重试"))
    }
}

fn captcha_answer() -> String {
    const ALPHABET: &[u8] = b"23456789ABCDEFGHJKLMNPQRSTUVWXYZ";
    let mut bytes = [0_u8; 6];
    OsRng.fill_bytes(&mut bytes);
    bytes
        .into_iter()
        .map(|byte| ALPHABET[(byte as usize) % ALPHABET.len()] as char)
        .collect()
}

fn captcha_answer_hash(answer: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(answer.trim().to_ascii_uppercase().as_bytes());
    format!("{:x}", hasher.finalize())
}

fn captcha_svg(answer: &str) -> String {
    let chars = answer.chars().collect::<Vec<_>>();
    let mut text = String::new();
    for (index, ch) in chars.iter().enumerate() {
        let x = 20 + index * 20;
        let y = 31 + ((index % 2) * 4);
        text.push_str(&format!(
            r#"<text x="{x}" y="{y}" transform="rotate({rotate} {x} {y})">{ch}</text>"#,
            rotate = if index % 2 == 0 { -8 } else { 7 },
        ));
    }
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="148" height="46" viewBox="0 0 148 46" role="img" aria-label="验证码">
<rect width="148" height="46" rx="8" fill="#f8fafc"/>
<path d="M8 32 C38 16, 74 42, 140 17" fill="none" stroke="#93c5fd" stroke-width="2" opacity=".65"/>
<path d="M12 15 C42 38, 88 8, 136 31" fill="none" stroke="#c4b5fd" stroke-width="2" opacity=".55"/>
<g fill="#0f172a" font-family="ui-monospace, SFMono-Regular, Menlo, Consolas, monospace" font-size="22" font-weight="800" letter-spacing="3">{text}</g>
</svg>"##
    )
}

async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    if let Some(token) = token_from_headers(&headers) {
        state.auth.logout(&token).await.map_err(ApiError::from)?;
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    set_cookie_header(response.headers_mut(), clear_cookie_value()).map_err(ApiError::from)?;
    Ok(response)
}

async fn get_current_user(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::Json<CurrentUserResponse>, ApiError> {
    Ok(axum::Json(user_response(
        current_user_from_headers(&state, &headers).await?,
    )))
}

async fn get_user_profile(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::Json<CurrentUserResponse>, ApiError> {
    get_current_user(State(state), headers).await
}

async fn update_user_password(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<UserPasswordUpdateRequest>,
) -> Result<StatusCode, ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    if request.new_password != request.confirm_new_password {
        return Err(ApiError::bad_request(
            "password confirmation does not match",
        ));
    }
    validate_password(&request.new_password)?;
    let stored_user = state
        .storage
        .find_user_by_id(&user.id)
        .await?
        .ok_or_else(|| ApiError::unauthorized("authentication required"))?;
    verify_password(&request.current_password, &stored_user.password_hash)
        .map_err(|_| ApiError::unauthorized("current password is incorrect"))?;
    let password_hash = hash_password(&request.new_password).map_err(ApiError::from)?;
    state
        .storage
        .update_password_hash(&user.id, password_hash)
        .await?;
    if let Some(token) = token_from_headers(&headers) {
        state
            .auth
            .revoke_other_sessions(&user.id, &token)
            .await
            .map_err(ApiError::from)?;
    }
    Ok(StatusCode::NO_CONTENT)
}

async fn list_user_leases(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::Json<LeasesResponse>, ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    let leases = state.storage.list_leases_for_user(&user.id).await?;
    let mut responses = Vec::new();
    for lease in leases {
        responses.push(lease_response(&state, lease).await);
    }
    Ok(axum::Json(LeasesResponse { leases: responses }))
}

async fn list_user_lease_availability(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<axum::Json<LeasesResponse>, ApiError> {
    current_user_from_headers(&state, &headers).await?;
    let leases = state.storage.list_leases().await?;
    let mut leased_session_ids = BTreeSet::new();
    let mut responses = Vec::new();
    for lease in leases
        .into_iter()
        .filter(|lease| lease.state == crate::storage::LeaseState::Active)
    {
        if let Some(session_id) = lease.session_id.as_deref() {
            leased_session_ids.insert(session_id.to_string());
        }
        responses.push(lease_response(&state, lease).await);
    }

    let boards = state.boards.read().await.clone();
    for session in session_snapshots(&state).await {
        if leased_session_ids.contains(&session.id) {
            continue;
        }
        let Some(board) = boards.get(&session.board_id) else {
            continue;
        };
        responses.push(LeaseResponse {
            lease: Lease {
                id: format!("runtime-session-{}", session.id),
                user_id: "runtime-session".to_string(),
                session_id: Some(session.id.clone()),
                board_id: session.board_id.clone(),
                board_type: board.board_type.clone(),
                required_tags: board.tags.clone(),
                state: crate::storage::LeaseState::Active,
                created_at: session.created_at,
                updated_at: session.last_heartbeat_at,
                starts_at: session.created_at,
                expires_at: session.expires_at,
                released_at: None,
                failure_message: None,
            },
            session: Some(session),
        });
    }
    Ok(axum::Json(LeasesResponse { leases: responses }))
}

async fn create_lease(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<CreateLeaseRequest>,
) -> Result<(StatusCode, axum::Json<LeaseResponse>), ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    validate_len(
        request.board_type.trim(),
        "board_type",
        1,
        validation::BOARD_TYPE_MAX_LEN,
    )?;
    for tag in &request.required_tags {
        validate_max_len(tag.trim(), "required_tags", validation::TAG_MAX_LEN)?;
    }
    validate_lease_window(request.starts_at, request.expires_at)?;
    let lease = create_user_lease(
        &state,
        &user.id,
        &user.username,
        request,
        Some(addr.ip().to_string()),
    )
    .await
    .map_err(|err| ApiError::conflict(err.to_string()))?;
    let response = lease_response(&state, lease).await;
    Ok((StatusCode::CREATED, axum::Json(response)))
}

async fn delete_user_lease(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(lease_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    let lease = state
        .storage
        .find_lease(&lease_id)
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))?;
    if lease.user_id != user.id {
        return Err(ApiError::not_found("lease not found"));
    }
    release_lease(&state, lease, None).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn heartbeat_user_lease(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(lease_id): Path<String>,
) -> Result<axum::Json<LeaseResponse>, ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    let lease = state
        .storage
        .find_lease(&lease_id)
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))?;
    if lease.user_id != user.id {
        return Err(ApiError::not_found("lease not found"));
    }
    let Some(session_id) = lease.session_id.as_deref() else {
        return Err(ApiError::conflict("lease session has not started"));
    };
    let session = state
        .heartbeat_session(session_id)
        .await
        .map_err(|_| ApiError::not_found("session not found"))?;
    state
        .storage
        .update_lease_expiry(&lease.id, session.expires_at)
        .await?;
    let lease = state
        .storage
        .find_lease(&lease_id)
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))?;
    Ok(axum::Json(lease_response(&state, lease).await))
}

async fn list_admin_users(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminUsersResponse>, ApiError> {
    let users = state
        .storage
        .list_users()
        .await?
        .into_iter()
        .map(admin_user_response)
        .collect();
    Ok(axum::Json(AdminUsersResponse { users }))
}

async fn create_admin_user(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<AdminUserCreateRequest>,
) -> Result<(StatusCode, axum::Json<AdminUserResponse>), ApiError> {
    let username = clean_required_len(
        request.username,
        "username",
        validation::USERNAME_MIN_LEN,
        validation::USERNAME_MAX_LEN,
    )?;
    validate_username(&username)?;
    let display_name = clean_optional_len(
        Some(request.display_name),
        "display_name",
        validation::DISPLAY_NAME_MAX_LEN,
    )?
    .unwrap_or_else(|| username.clone());
    validate_len(
        &display_name,
        "display_name",
        validation::DISPLAY_NAME_MIN_LEN,
        validation::DISPLAY_NAME_MAX_LEN,
    )?;
    let email = clean_required_len(
        request.email,
        "email",
        validation::EMAIL_MIN_LEN,
        validation::EMAIL_MAX_LEN,
    )?;
    validate_email(&email)?;
    validate_password(&request.password)?;
    let user = state
        .auth
        .create_user(
            username,
            display_name,
            email,
            request.password,
            request_profile_checked(
                request.nickname,
                request.avatar_url,
                request.phone,
                request.department,
                request.title,
            )?,
            role_names_for_ids(&state, request.role_ids).await?,
        )
        .await
        .map_err(|err| ApiError::conflict(err.to_string()))?;
    Ok((StatusCode::CREATED, axum::Json(admin_user_response(user))))
}

async fn get_admin_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<axum::Json<AdminUserResponse>, ApiError> {
    let user = state
        .storage
        .find_user_by_id(&user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    Ok(axum::Json(admin_user_response(user)))
}

async fn update_admin_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    axum::Json(request): axum::Json<AdminUserUpdateRequest>,
) -> Result<axum::Json<AdminUserResponse>, ApiError> {
    let display_name = clean_required_len(
        request.display_name,
        "display_name",
        validation::DISPLAY_NAME_MIN_LEN,
        validation::DISPLAY_NAME_MAX_LEN,
    )?;
    let email = clean_required_len(
        request.email,
        "email",
        validation::EMAIL_MIN_LEN,
        validation::EMAIL_MAX_LEN,
    )?;
    validate_email(&email)?;
    let user = state
        .storage
        .update_user(
            &user_id,
            display_name,
            email,
            request_profile_checked(
                request.nickname,
                request.avatar_url,
                request.phone,
                request.department,
                request.title,
            )?,
            request.disabled,
        )
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    Ok(axum::Json(admin_user_response(user)))
}

async fn reset_admin_user_password(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    axum::Json(request): axum::Json<AdminPasswordResetRequest>,
) -> Result<StatusCode, ApiError> {
    validate_password(&request.password)?;
    let password_hash = hash_password(&request.password).map_err(ApiError::from)?;
    state
        .storage
        .update_password_hash(&user_id, password_hash)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn disable_admin_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state.storage.set_user_disabled(&user_id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn approve_admin_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<axum::Json<AdminUserResponse>, ApiError> {
    let user = ensure_pending_user_for_review(&state, &user_id).await?;
    state
        .auth
        .approve_user(&user_id)
        .await
        .map_err(|err| ApiError::not_found(err.to_string()))?;
    write_audit_log(
        &state,
        "user.approve",
        "user",
        Some(user_id.clone()),
        json!({ "username": user.username }),
    )
    .await?;
    let updated = state
        .storage
        .find_user_by_id(&user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    Ok(axum::Json(admin_user_response(updated)))
}

async fn reject_admin_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<axum::Json<AdminUserResponse>, ApiError> {
    let user = ensure_pending_user_for_review(&state, &user_id).await?;
    state
        .auth
        .reject_user(&user_id)
        .await
        .map_err(|err| ApiError::not_found(err.to_string()))?;
    write_audit_log(
        &state,
        "user.reject",
        "user",
        Some(user_id.clone()),
        json!({ "username": user.username }),
    )
    .await?;
    let updated = state
        .storage
        .find_user_by_id(&user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    Ok(axum::Json(admin_user_response(updated)))
}

async fn ensure_user_for_review(
    state: &AppState,
    user_id: &str,
) -> Result<crate::storage::User, ApiError> {
    state
        .storage
        .find_user_by_id(user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))
}

async fn ensure_pending_user_for_review(
    state: &AppState,
    user_id: &str,
) -> Result<crate::storage::User, ApiError> {
    let user = ensure_user_for_review(state, user_id).await?;
    if user.status != crate::storage::UserStatus::Pending {
        return Err(ApiError::conflict(
            "only pending registration users can be reviewed",
        ));
    }
    Ok(user)
}

async fn list_pending_admin_users(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminUsersResponse>, ApiError> {
    let users = state
        .storage
        .list_users_with_status(crate::storage::UserStatus::Pending)
        .await?;
    Ok(axum::Json(AdminUsersResponse {
        users: users.into_iter().map(admin_user_response).collect(),
    }))
}

async fn delete_admin_user(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    if state.storage.find_user_by_id(&user_id).await?.is_none() {
        return Err(ApiError::not_found("user not found"));
    }
    state.storage.set_user_disabled(&user_id, true).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Deserialize)]
struct AdminAuditLogsQuery {
    limit: Option<i64>,
}

async fn list_admin_audit_logs(
    State(state): State<AppState>,
    Query(query): Query<AdminAuditLogsQuery>,
) -> Result<axum::Json<AdminAuditLogsResponse>, ApiError> {
    let limit = query.limit.unwrap_or(500).clamp(1, 1000);
    let logs = state
        .storage
        .list_audit_logs(limit)
        .await?
        .into_iter()
        .map(AdminAuditLogResponse::from)
        .collect();
    Ok(axum::Json(AdminAuditLogsResponse { logs }))
}

async fn list_admin_permissions(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminPermissionsResponse>, ApiError> {
    let permissions = state
        .storage
        .list_permissions()
        .await?
        .into_iter()
        .map(AdminPermissionResponse::from)
        .collect();
    Ok(axum::Json(AdminPermissionsResponse { permissions }))
}

async fn list_admin_roles(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminRolesResponse>, ApiError> {
    let user_counts = state.storage.role_user_counts().await?;
    let mut roles = Vec::new();
    for role in state.storage.list_roles().await? {
        let permissions = state.storage.role_permissions(&role.id).await?;
        let user_count = user_counts.get(&role.id).copied().unwrap_or(0);
        roles.push(AdminRoleResponse::new(role, permissions, user_count));
    }
    Ok(axum::Json(AdminRolesResponse { roles }))
}

async fn create_admin_role(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<AdminRoleCreateRequest>,
) -> Result<(StatusCode, axum::Json<AdminRoleResponse>), ApiError> {
    let name = clean_required_len(
        request.name,
        "role name",
        validation::ROLE_NAME_MIN_LEN,
        validation::ROLE_NAME_MAX_LEN,
    )?;
    if !validation::valid_role_name(&name) {
        return Err(ApiError::bad_request(
            "role name must contain only lowercase letters, numbers, '_' or '-'",
        ));
    }
    let display_name = clean_required_len(
        request.display_name,
        "role display_name",
        validation::DISPLAY_NAME_MIN_LEN,
        validation::DISPLAY_NAME_MAX_LEN,
    )?;
    let description = clean_optional_len(
        Some(request.description),
        "role description",
        validation::DESCRIPTION_MAX_LEN,
    )?
    .unwrap_or_default();
    let role = state
        .storage
        .create_role(NewRole {
            name,
            display_name,
            description,
            permission_ids: request.permission_ids,
        })
        .await
        .map_err(|err| ApiError::conflict(err.to_string()))?;
    Ok((
        StatusCode::CREATED,
        axum::Json(admin_role_response(&state, role).await?),
    ))
}

async fn get_admin_role(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
) -> Result<axum::Json<AdminRoleResponse>, ApiError> {
    let role = state
        .storage
        .find_role_by_id(&role_id)
        .await?
        .ok_or_else(|| ApiError::not_found("role not found"))?;
    Ok(axum::Json(admin_role_response(&state, role).await?))
}

async fn update_admin_role(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
    axum::Json(request): axum::Json<AdminRoleUpdateRequest>,
) -> Result<axum::Json<AdminRoleResponse>, ApiError> {
    let display_name = clean_required_len(
        request.display_name,
        "role display_name",
        validation::DISPLAY_NAME_MIN_LEN,
        validation::DISPLAY_NAME_MAX_LEN,
    )?;
    let description = clean_optional_len(
        Some(request.description),
        "role description",
        validation::DESCRIPTION_MAX_LEN,
    )?
    .unwrap_or_default();
    let role = state
        .storage
        .update_role(&role_id, display_name, description, request.permission_ids)
        .await?
        .ok_or_else(|| ApiError::not_found("role not found"))?;
    Ok(axum::Json(admin_role_response(&state, role).await?))
}

async fn disable_admin_role(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
    axum::Json(request): axum::Json<AdminRoleDisableRequest>,
) -> Result<axum::Json<AdminRoleResponse>, ApiError> {
    let role = state
        .storage
        .set_role_disabled(&role_id, request.disabled)
        .await
        .map_err(|err| ApiError::conflict(err.to_string()))?
        .ok_or_else(|| ApiError::not_found("role not found"))?;
    Ok(axum::Json(admin_role_response(&state, role).await?))
}

async fn delete_admin_role(
    State(state): State<AppState>,
    Path(role_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    state
        .storage
        .delete_role(&role_id)
        .await
        .map_err(|err| ApiError::conflict(err.to_string()))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_admin_user_roles(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
) -> Result<axum::Json<AdminUserRolesResponse>, ApiError> {
    if state.storage.find_user_by_id(&user_id).await?.is_none() {
        return Err(ApiError::not_found("user not found"));
    }
    let mut roles = Vec::new();
    for role in state.storage.user_roles(&user_id).await? {
        roles.push(admin_role_response(&state, role).await?);
    }
    Ok(axum::Json(AdminUserRolesResponse { roles }))
}

async fn update_admin_user_roles(
    State(state): State<AppState>,
    Path(user_id): Path<String>,
    axum::Json(request): axum::Json<AdminUserRolesUpdateRequest>,
) -> Result<axum::Json<AdminUserRolesResponse>, ApiError> {
    if state.storage.find_user_by_id(&user_id).await?.is_none() {
        return Err(ApiError::not_found("user not found"));
    }
    state
        .storage
        .set_user_roles(&user_id, request.role_ids)
        .await?;
    get_admin_user_roles(State(state), Path(user_id)).await
}

async fn list_admin_leases(
    State(state): State<AppState>,
) -> Result<axum::Json<LeasesResponse>, ApiError> {
    let leases = state.storage.list_leases().await?;
    let mut responses = Vec::new();
    for lease in leases {
        responses.push(lease_response(&state, lease).await);
    }
    Ok(axum::Json(LeasesResponse { leases: responses }))
}

async fn create_admin_lease(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<AdminLeaseCreateRequest>,
) -> Result<(StatusCode, axum::Json<LeaseResponse>), ApiError> {
    let user_id = clean_required_len(request.user_id, "user_id", 1, validation::ID_MAX_LEN)?;
    let board_id = clean_required_len(request.board_id, "board_id", 1, validation::ID_MAX_LEN)?;
    if let Some(client_name) = request.client_name.as_deref() {
        validate_max_len(
            client_name.trim(),
            "client_name",
            validation::CLIENT_NAME_MAX_LEN,
        )?;
    }
    let user = state
        .storage
        .find_user_by_id(&user_id)
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    if user.disabled {
        return Err(ApiError::conflict("user is disabled"));
    }
    validate_lease_window(request.starts_at, request.expires_at)?;
    let board = state
        .boards
        .read()
        .await
        .get(&board_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("board `{board_id}` not found")))?;
    if board.disabled {
        return Err(ApiError::conflict(format!(
            "board `{board_id}` is disabled"
        )));
    }
    ensure_lease_window_available(
        &state,
        &board_id,
        request.starts_at,
        request.expires_at,
        None,
    )
    .await?;
    let lease = state
        .storage
        .create_lease(crate::storage::NewLease {
            user_id: user.id,
            session_id: None,
            board_id: board.id,
            board_type: board.board_type,
            required_tags: Vec::new(),
            starts_at: request.starts_at,
            expires_at: request.expires_at,
        })
        .await?;
    let response = lease_response(&state, lease).await;
    Ok((StatusCode::CREATED, axum::Json(response)))
}

async fn get_admin_lease(
    State(state): State<AppState>,
    Path(lease_id): Path<String>,
) -> Result<axum::Json<LeaseResponse>, ApiError> {
    let lease = state
        .storage
        .find_lease(&lease_id)
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))?;
    Ok(axum::Json(lease_response(&state, lease).await))
}

async fn update_admin_lease(
    State(state): State<AppState>,
    Path(lease_id): Path<String>,
    axum::Json(request): axum::Json<AdminLeaseUpdateRequest>,
) -> Result<axum::Json<LeaseResponse>, ApiError> {
    if let Some(failure_message) = request.failure_message.as_deref() {
        validate_max_len(
            failure_message.trim(),
            "failure_message",
            validation::LONG_DESCRIPTION_MAX_LEN,
        )?;
    }
    let existing = state
        .storage
        .find_lease(&lease_id)
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))?;
    if existing.state != crate::storage::LeaseState::Active {
        return Err(ApiError::conflict("only active leases can be updated"));
    }
    validate_lease_window(request.starts_at, request.expires_at)?;
    ensure_lease_window_available(
        &state,
        &existing.board_id,
        request.starts_at,
        request.expires_at,
        Some(&existing.id),
    )
    .await?;
    let updated = state
        .storage
        .update_lease(
            &lease_id,
            request.starts_at,
            request.expires_at,
            clean_optional(request.failure_message),
        )
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))?;
    if let Some(session_id) = updated.session_id.as_deref() {
        state
            .update_session_expiry(session_id, updated.expires_at)
            .await;
    }
    Ok(axum::Json(lease_response(&state, updated).await))
}

async fn start_admin_lease_session(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    Path(lease_id): Path<String>,
) -> Result<axum::Json<LeaseResponse>, ApiError> {
    let lease = state
        .storage
        .find_lease(&lease_id)
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))?;
    let updated = start_lease_session(&state, lease, Some(addr.ip().to_string())).await?;
    Ok(axum::Json(lease_response(&state, updated).await))
}

async fn release_admin_lease(
    State(state): State<AppState>,
    Path(lease_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let lease = state
        .storage
        .find_lease(&lease_id)
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))?;
    release_lease(&state, lease, None).await?;
    Ok(StatusCode::ACCEPTED)
}

async fn delete_admin_lease(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(lease_id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    require_permission(&user, "leases.delete")?;
    let lease = state
        .storage
        .find_lease(&lease_id)
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))?;
    if let Some(session_id) = lease.session_id.as_deref() {
        let _ = state
            .request_session_stop(session_id, SessionStopReason::ApiDelete)
            .await;
    }
    state.storage.delete_lease(&lease_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn validate_lease_window(
    starts_at: chrono::DateTime<chrono::Utc>,
    expires_at: chrono::DateTime<chrono::Utc>,
) -> Result<(), ApiError> {
    if expires_at <= starts_at {
        return Err(ApiError::bad_request("expires_at must be after starts_at"));
    }
    if expires_at <= chrono::Utc::now() {
        return Err(ApiError::bad_request("expires_at must be in the future"));
    }
    Ok(())
}

async fn ensure_lease_window_available(
    state: &AppState,
    board_id: &str,
    starts_at: chrono::DateTime<chrono::Utc>,
    expires_at: chrono::DateTime<chrono::Utc>,
    exclude_lease_id: Option<&str>,
) -> Result<(), ApiError> {
    let leases = state.storage.list_leases().await?;
    let overlaps = leases.into_iter().any(|lease| {
        lease.board_id == board_id
            && lease.state == crate::storage::LeaseState::Active
            && exclude_lease_id != Some(lease.id.as_str())
            && starts_at < lease.expires_at
            && expires_at > lease.starts_at
    });
    if overlaps {
        return Err(ApiError::conflict(format!(
            "board `{board_id}` already has a lease in that time window"
        )));
    }
    Ok(())
}

async fn start_lease_session(
    state: &AppState,
    lease: Lease,
    source_ip: Option<String>,
) -> Result<Lease, ApiError> {
    if lease.state != crate::storage::LeaseState::Active {
        return Err(ApiError::conflict("only active leases can start a session"));
    }
    let now = chrono::Utc::now();
    if now < lease.starts_at {
        return Err(ApiError::conflict("lease time window has not started"));
    }
    if now >= lease.expires_at {
        return Err(ApiError::conflict("lease time window has expired"));
    }
    if let Some(session_id) = lease.session_id.as_deref()
        && state.get_session(session_id).await.is_some()
    {
        return Ok(lease);
    }

    let session = state
        .create_session_for_board(&lease.board_id, Some(lease.user_id.clone()), source_ip)
        .await
        .map_err(|err| match err {
            BoardAllocationStatus::BoardTypeNotFound => {
                ApiError::not_found(format!("board `{}` not found", lease.board_id))
            }
            BoardAllocationStatus::NoAvailableBoard => {
                ApiError::conflict(format!("board `{}` is not available", lease.board_id))
            }
        })?;
    state
        .update_session_expiry(&session.id, lease.expires_at)
        .await;
    state
        .storage
        .bind_lease_session(&lease.id, &session.id)
        .await?;
    state
        .storage
        .find_lease(&lease.id)
        .await?
        .ok_or_else(|| ApiError::not_found("lease not found"))
}

async fn get_admin_overview(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminOverviewResponse>, ApiError> {
    let current_leased_board_ids = current_leased_board_ids(&state).await?;
    let boards = state.boards.read().await;
    let runtimes = state.board_runtimes.read().await;
    let board_types = summarize_board_types(&boards, &runtimes, &current_leased_board_ids);
    let board_count_total = boards.len();
    let disabled_board_count = boards.values().filter(|board| board.disabled).count();
    let board_count_available = boards
        .values()
        .filter(|board| !board.disabled)
        .filter(|board| !current_leased_board_ids.contains(&board.id))
        .count();
    drop(runtimes);
    drop(boards);

    let mut tftp_status = state
        .tftp_manager
        .read()
        .await
        .status()
        .await
        .map_err(|err| {
            ApiError::service_unavailable(format!("failed to get TFTP status: {err}"))
        })?;
    let config = state.config.read().await.clone();
    let runtime = state.runtime_settings.read().await.clone();
    tftp_status.resolved_server_ip =
        resolve_server_network(&runtime)?.and_then(|network| network.server_ip);
    tftp_status.resolved_netmask =
        resolve_server_network(&runtime)?.and_then(|network| network.netmask);

    Ok(axum::Json(AdminOverviewResponse {
        board_count_total,
        board_count_available,
        disabled_board_count,
        active_session_count: session_snapshots(&state)
            .await
            .into_iter()
            .filter(|session| session.state == crate::session::SessionLifecycleState::Active)
            .count(),
        board_types,
        tftp_status,
        server: readonly_server_config(&config),
    }))
}

async fn list_boards(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<BoardConfig>>, ApiError> {
    let boards = state
        .boards
        .read()
        .await
        .values()
        .cloned()
        .map(with_resolved_serial_config)
        .collect::<Vec<_>>();
    Ok(axum::Json(boards))
}

async fn list_dtbs(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<DtbFileResponse>>, ApiError> {
    let metadata = state.storage.list_dtb_metadata().await?;
    if !metadata.is_empty() {
        return Ok(axum::Json(metadata.into_iter().map(Into::into).collect()));
    }
    let files = state.dtb_store.list_all().await?;
    let mut responses = Vec::new();
    for file in files {
        let metadata = state.storage.find_dtb_metadata_by_name(&file.name).await?;
        responses.push(DtbFileResponse::from_dtb_with_metadata(file, metadata));
    }
    Ok(axum::Json(responses))
}

async fn list_network_interfaces() -> Result<axum::Json<Vec<NetworkInterfaceSummary>>, ApiError> {
    Ok(axum::Json(discover_network_interfaces().map_err(
        |err| {
            ApiError::service_unavailable(format!(
                "failed to enumerate network interfaces: {err:#}"
            ))
        },
    )?))
}

async fn list_serial_ports() -> Result<axum::Json<Vec<SerialPortSummary>>, ApiError> {
    Ok(axum::Json(discover_serial_ports().map_err(|err| {
        ApiError::service_unavailable(format!("failed to enumerate serial ports: {err:#}"))
    })?))
}

async fn get_board(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<BoardConfig>, ApiError> {
    let board = state
        .boards
        .read()
        .await
        .get(&board_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("board `{board_id}` not found")))?;
    Ok(axum::Json(with_resolved_serial_config(board)))
}

async fn get_board_power_status(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<BoardPowerStatusResponse>, ApiError> {
    let status = state
        .board_power_status(&board_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("board `{board_id}` not found")))?;
    Ok(axum::Json(BoardPowerStatusResponse {
        available: status.available,
        powered: status.powered,
        last_action: status.last_action.map(board_power_action),
        updated_at: status.updated_at,
    }))
}

async fn get_board_runtime_status(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<BoardRuntimeStatusResponse>, ApiError> {
    let status = state
        .board_runtime_status(&board_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("board `{board_id}` not found")))?;
    Ok(axum::Json(BoardRuntimeStatusResponse {
        lease_state: status.lease_state,
        active_session_id: status.active_session_id,
        last_release_error: status.last_release_error,
        updated_at: status.updated_at,
    }))
}

async fn get_dtb(
    Path(dtb_name): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<DtbFileResponse>, ApiError> {
    let dtb_name =
        normalize_dtb_name(&dtb_name).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let file = state
        .dtb_store
        .get(&dtb_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("DTB `{dtb_name}` not found")))?;
    let metadata = state.storage.find_dtb_metadata_by_name(&dtb_name).await?;
    Ok(axum::Json(DtbFileResponse::from_dtb_with_metadata(
        file, metadata,
    )))
}

async fn create_dtb(
    State(state): State<AppState>,
    request: Request,
) -> Result<(StatusCode, axum::Json<DtbFileResponse>), ApiError> {
    let headers = request.headers();
    let dtb_name = dtb_name_header(headers, "X-Dtb-Name")?;
    let dtb_metadata = dtb_metadata_headers(headers)?;
    let body = read_limited_body(request, DTB_UPLOAD_MAX_MIB, "DTB").await?;
    if body.is_empty() {
        return Err(ApiError::bad_request("DTB upload body must not be empty"));
    }
    if state.dtb_store.get(&dtb_name).await?.is_some() {
        return Err(ApiError::conflict(format!(
            "DTB `{dtb_name}` already exists"
        )));
    }

    let file = state.dtb_store.write(&dtb_name, &body).await?;
    let metadata = sync_dtb_metadata(&state, &file, &body, dtb_metadata, None).await?;
    write_audit_log(
        &state,
        "dtb.create",
        "dtb_file",
        Some(dtb_name.clone()),
        json!({
            "name": dtb_name,
            "size_bytes": file.size,
            "sha256": metadata.sha256,
        }),
    )
    .await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(DtbFileResponse::from_dtb_with_metadata(
            file,
            Some(metadata),
        )),
    ))
}

async fn create_board(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<AdminBoardUpsertRequest>,
) -> Result<(StatusCode, axum::Json<BoardConfig>), ApiError> {
    let board = build_board_config_for_create(&state, request).await?;

    {
        let boards = state.boards.read().await;
        if boards.contains_key(&board.id) {
            return Err(ApiError::conflict(format!(
                "board `{}` already exists",
                board.id
            )));
        }
    }

    state.storage.create_board_config(board.clone()).await?;
    state
        .boards
        .write()
        .await
        .insert(board.id.clone(), board.clone());
    state.sync_board_runtime_states().await;
    Ok((StatusCode::CREATED, axum::Json(board)))
}

async fn update_board(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
    axum::Json(request): axum::Json<AdminBoardUpsertRequest>,
) -> Result<axum::Json<BoardConfig>, ApiError> {
    let board = build_board_config_for_update(&state, &board_id, request).await?;

    {
        let boards = state.boards.read().await;
        if !boards.contains_key(&board_id) {
            return Err(ApiError::not_found(format!("board `{board_id}` not found")));
        }
        if board.id != board_id && boards.contains_key(&board.id) {
            return Err(ApiError::conflict(format!(
                "board `{}` already exists",
                board.id
            )));
        }
    }

    let runtime = state
        .board_runtime_status(&board_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("board `{board_id}` not found")))?;
    if runtime.lease_state != BoardLeaseState::Idle {
        return Err(ApiError::conflict(format!(
            "board `{board_id}` is not idle"
        )));
    }

    state
        .storage
        .update_board_config(&board_id, board.clone())
        .await?;

    {
        let mut boards = state.boards.write().await;
        boards.remove(&board_id);
        boards.insert(board.id.clone(), board.clone());
    }
    state.sync_board_runtime_states().await;

    Ok(axum::Json(board))
}

async fn build_board_config_for_create(
    state: &AppState,
    request: AdminBoardUpsertRequest,
) -> Result<BoardConfig, ApiError> {
    let mut request = normalize_board_upsert_request(request)?;
    let boards = state.boards.read().await;
    let board_id = request
        .id
        .take()
        .unwrap_or_else(|| allocate_board_id(&boards, &request.board_type));
    Ok(request.into_board_config(board_id))
}

async fn build_board_config_for_update(
    _state: &AppState,
    current_board_id: &str,
    request: AdminBoardUpsertRequest,
) -> Result<BoardConfig, ApiError> {
    let mut request = normalize_board_upsert_request(request)?;
    let board_id = request
        .id
        .take()
        .unwrap_or_else(|| current_board_id.to_string());
    Ok(request.into_board_config(board_id))
}

fn normalize_board_upsert_request(
    mut request: AdminBoardUpsertRequest,
) -> Result<AdminBoardUpsertRequest, ApiError> {
    normalize_optional_string(&mut request.id);
    normalize_required_string(&mut request.board_type, "board_type")?;
    validate_max_len(
        &request.board_type,
        "board_type",
        validation::BOARD_TYPE_MAX_LEN,
    )?;
    normalize_optional_string(&mut request.notes);
    if let Some(id) = request.id.as_deref() {
        validate_max_len(id, "board id", validation::ID_MAX_LEN)?;
    }
    if let Some(notes) = request.notes.as_deref() {
        validate_max_len(notes, "notes", validation::LONG_DESCRIPTION_MAX_LEN)?;
    }
    normalize_tags(&mut request.tags);
    validate_tags(&request.tags)?;
    normalize_serial_config(request.serial.as_mut())?;
    normalize_power_management_config(&mut request.power_management)?;
    normalize_boot_config(&mut request.boot)?;

    if let Some(id) = request.id.as_ref()
        && (id.contains('/') || id.contains('\\'))
    {
        return Err(ApiError::bad_request(
            "board id must not contain path separators",
        ));
    }
    if let BootConfig::Uboot(profile) = &request.boot
        && let Some(dtb_name) = profile.dtb_name.as_deref()
    {
        normalize_dtb_name(dtb_name).map_err(|err| ApiError::bad_request(err.to_string()))?;
    }

    Ok(request)
}

fn normalize_required_string(value: &mut String, field_name: &str) -> Result<(), ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(format!(
            "{field_name} must not be empty"
        )));
    }
    if trimmed.len() != value.len() {
        *value = trimmed.to_string();
    }
    Ok(())
}

fn normalize_optional_string(value: &mut Option<String>) {
    if let Some(raw) = value {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            *value = None;
        } else if trimmed.len() != raw.len() {
            *raw = trimmed.to_string();
        }
    }
}

fn normalize_tags(tags: &mut Vec<String>) {
    *tags = tags
        .iter()
        .map(|tag| tag.trim())
        .filter(|tag| !tag.is_empty())
        .map(ToOwned::to_owned)
        .collect();
}

fn validate_tags(tags: &[String]) -> Result<(), ApiError> {
    let joined_len = tags
        .iter()
        .map(|tag| validation::char_len(tag))
        .sum::<usize>()
        + tags.len().saturating_sub(1);
    if joined_len > validation::TAGS_TEXT_MAX_LEN {
        return Err(ApiError::bad_request(format!(
            "tags length must be at most {} characters",
            validation::TAGS_TEXT_MAX_LEN
        )));
    }
    for tag in tags {
        validate_max_len(tag, "tag", validation::TAG_MAX_LEN)?;
    }
    Ok(())
}

fn normalize_serial_config(
    serial: Option<&mut crate::config::SerialConfig>,
) -> Result<(), ApiError> {
    let Some(serial) = serial else {
        return Ok(());
    };

    normalize_serial_key_value(&mut serial.key, "serial.key.value")?;
    validate_max_len(
        &serial.key.value,
        "serial.key.value",
        validation::SERIAL_KEY_MAX_LEN,
    )?;
    if serial.baud_rate == 0 {
        return Err(ApiError::bad_request(
            "serial.baud_rate must be > 0 when serial is configured",
        ));
    }
    serial.resolved_device_path = None;
    serial.resolved_usb_path = None;
    Ok(())
}

fn normalize_serial_key_value(
    key: &mut crate::config::SerialPortKey,
    field: &str,
) -> Result<(), ApiError> {
    let trimmed = key.value.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(format!("{field} must not be empty")));
    }
    if trimmed.len() != key.value.len() {
        key.value = trimmed.to_string();
    }
    Ok(())
}

fn normalize_power_management_config(
    power_management: &mut PowerManagementConfig,
) -> Result<(), ApiError> {
    match power_management {
        PowerManagementConfig::Custom(custom) => {
            normalize_required_string(&mut custom.power_on_cmd, "power_management.power_on_cmd")?;
            normalize_required_string(&mut custom.power_off_cmd, "power_management.power_off_cmd")?;
            validate_max_len(
                &custom.power_on_cmd,
                "power_management.power_on_cmd",
                validation::COMMAND_MAX_LEN,
            )?;
            validate_max_len(
                &custom.power_off_cmd,
                "power_management.power_off_cmd",
                validation::COMMAND_MAX_LEN,
            )?;
        }
        PowerManagementConfig::ZhongshengRelay(relay) => {
            normalize_serial_key_value(&mut relay.key, "power_management.key.value")?;
            validate_max_len(
                &relay.key.value,
                "power_management.key.value",
                validation::SERIAL_KEY_MAX_LEN,
            )?;
        }
    }

    Ok(())
}

fn normalize_boot_config(boot: &mut BootConfig) -> Result<(), ApiError> {
    match boot {
        BootConfig::Uboot(profile) => {
            normalize_optional_string(&mut profile.dtb_name);
            normalize_optional_string(&mut profile.kernel_load_addr);
            normalize_optional_string(&mut profile.fit_load_addr);
            normalize_optional_string(&mut profile.bootm_addr);
            normalize_optional_string(&mut profile.board_ip);
            normalize_optional_string(&mut profile.server_ip);
            normalize_optional_string(&mut profile.netmask);
            normalize_optional_string(&mut profile.gatewayip);
            if let Some(dtb_name) = profile.dtb_name.as_deref() {
                validate_max_len(dtb_name, "boot.dtb_name", validation::DTB_NAME_MAX_LEN)?;
            }
            for (field, value, max) in [
                (
                    "boot.kernel_load_addr",
                    profile.kernel_load_addr.as_deref(),
                    validation::LOAD_ADDR_MAX_LEN,
                ),
                (
                    "boot.fit_load_addr",
                    profile.fit_load_addr.as_deref(),
                    validation::LOAD_ADDR_MAX_LEN,
                ),
                (
                    "boot.bootm_addr",
                    profile.bootm_addr.as_deref(),
                    validation::LOAD_ADDR_MAX_LEN,
                ),
                (
                    "boot.board_ip",
                    profile.board_ip.as_deref(),
                    validation::IP_MAX_LEN,
                ),
                (
                    "boot.server_ip",
                    profile.server_ip.as_deref(),
                    validation::IP_MAX_LEN,
                ),
                (
                    "boot.netmask",
                    profile.netmask.as_deref(),
                    validation::IP_MAX_LEN,
                ),
                (
                    "boot.gatewayip",
                    profile.gatewayip.as_deref(),
                    validation::IP_MAX_LEN,
                ),
            ] {
                if let Some(value) = value {
                    validate_max_len(value, field, max)?;
                }
            }
            if !profile.use_tftp {
                profile.network_mode = UbootNetworkMode::Dhcp;
            }
            if profile.network_mode == UbootNetworkMode::Dhcp {
                profile.board_ip = None;
                profile.server_ip = None;
                profile.netmask = None;
                profile.gatewayip = None;
            }
            profile
                .validate()
                .map_err(|err| ApiError::bad_request(format!("{err:#}")))?;
        }
        BootConfig::Pxe(profile) => {
            normalize_optional_string(&mut profile.notes);
            if let Some(notes) = profile.notes.as_deref() {
                validate_max_len(notes, "boot.notes", validation::LONG_DESCRIPTION_MAX_LEN)?;
            }
        }
        BootConfig::UefiHttp(profile) => {
            profile.mac = None;
        }
    }
    Ok(())
}

fn allocate_board_id(boards: &BTreeMap<String, BoardConfig>, board_type: &str) -> String {
    let mut num = 1usize;
    loop {
        let candidate = format!("{board_type}-{num}");
        if !boards.contains_key(&candidate) {
            return candidate;
        }
        num += 1;
    }
}

impl AdminBoardUpsertRequest {
    fn into_board_config(self, board_id: String) -> BoardConfig {
        BoardConfig {
            id: board_id,
            board_type: self.board_type,
            tags: self.tags,
            serial: self.serial,
            power_management: self.power_management,
            boot: self.boot,
            notes: self.notes,
            disabled: self.disabled,
        }
    }
}

async fn update_dtb(
    Path(dtb_name): Path<String>,
    State(state): State<AppState>,
    request: Request,
) -> Result<axum::Json<DtbFileResponse>, ApiError> {
    let headers = request.headers();
    let dtb_metadata = dtb_metadata_headers(headers)?;
    let current_name =
        normalize_dtb_name(&dtb_name).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let requested_name = headers
        .get("X-Dtb-Name")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| {
            let value =
                normalize_dtb_name(value).map_err(|err| ApiError::bad_request(err.to_string()))?;
            validate_max_len(&value, "X-Dtb-Name", validation::DTB_NAME_MAX_LEN)?;
            Ok::<String, ApiError>(value)
        })
        .transpose()?;
    let mut effective_name = current_name.clone();
    let body = read_limited_body(request, DTB_UPLOAD_MAX_MIB, "DTB").await?;

    if let Some(new_name) = requested_name.as_deref()
        && new_name != current_name
    {
        state
            .dtb_store
            .rename(&current_name, new_name)
            .await
            .map_err(|err| {
                let message = err.to_string();
                if message.contains("already exists") {
                    ApiError::conflict(message)
                } else if message.contains("not found") {
                    ApiError::not_found(message)
                } else {
                    ApiError::from(err)
                }
            })?;
        rewrite_board_dtb_references(&state, &current_name, new_name).await?;
        state
            .storage
            .rename_dtb_metadata(&current_name, new_name)
            .await?;
        write_audit_log(
            &state,
            "dtb.rename",
            "dtb_file",
            Some(new_name.to_string()),
            json!({
                "from": current_name,
                "to": new_name,
            }),
        )
        .await?;
        effective_name = new_name.to_string();
    }

    if !body.is_empty() {
        let file = state.dtb_store.write(&effective_name, &body).await?;
        let metadata = sync_dtb_metadata(&state, &file, &body, dtb_metadata, None).await?;
        write_audit_log(
            &state,
            "dtb.update",
            "dtb_file",
            Some(effective_name.clone()),
            json!({
                "name": effective_name,
                "size_bytes": file.size,
                "sha256": metadata.sha256,
            }),
        )
        .await?;
    } else if dtb_metadata.has_updates() {
        let file = state
            .dtb_store
            .get(&effective_name)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("DTB `{effective_name}` not found")))?;
        let bytes = state
            .dtb_store
            .read(&effective_name)
            .await
            .map_err(ApiError::from)?;
        let metadata = sync_dtb_metadata(&state, &file, &bytes, dtb_metadata, None).await?;
        write_audit_log(
            &state,
            "dtb.update_metadata",
            "dtb_file",
            Some(effective_name.clone()),
            json!({
                "name": effective_name,
                "sha256": metadata.sha256,
            }),
        )
        .await?;
    } else if requested_name.is_none() {
        return Err(ApiError::bad_request(
            "DTB update requires a new name, metadata, or replacement file body",
        ));
    }

    let file = state
        .dtb_store
        .get(&effective_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("DTB `{effective_name}` not found")))?;
    let metadata = state
        .storage
        .find_dtb_metadata_by_name(&effective_name)
        .await?;
    Ok(axum::Json(DtbFileResponse::from_dtb_with_metadata(
        file, metadata,
    )))
}

async fn delete_dtb(
    Path(dtb_name): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let dtb_name =
        normalize_dtb_name(&dtb_name).map_err(|err| ApiError::bad_request(err.to_string()))?;
    let boards = state.boards.read().await;
    let referenced_by = boards_referencing_dtb(&boards, &dtb_name);
    drop(boards);
    if !referenced_by.is_empty() {
        return Err(ApiError::conflict(format!(
            "DTB `{dtb_name}` is referenced by boards: {}",
            referenced_by.join(", ")
        )));
    }
    if state.dtb_store.get(&dtb_name).await?.is_none() {
        return Err(ApiError::not_found(format!("DTB `{dtb_name}` not found")));
    }
    state.dtb_store.delete(&dtb_name).await?;
    state.storage.delete_dtb_metadata_by_name(&dtb_name).await?;
    write_audit_log(
        &state,
        "dtb.delete",
        "dtb_file",
        Some(dtb_name.clone()),
        json!({
            "name": dtb_name,
        }),
    )
    .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_board(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let runtime = state
        .board_runtime_status(&board_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("board `{board_id}` not found")))?;
    if runtime.lease_state != BoardLeaseState::Idle {
        return Err(ApiError::conflict(format!(
            "board `{board_id}` is not idle"
        )));
    }

    {
        let mut boards = state.boards.write().await;
        if boards.remove(&board_id).is_none() {
            return Err(ApiError::not_found(format!("board `{board_id}` not found")));
        }
    }
    state.sync_board_runtime_states().await;

    state.storage.delete_board_config(&board_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_admin_sessions(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminSessionsResponse>, ApiError> {
    let active_sessions = session_snapshots(&state)
        .await
        .into_iter()
        .map(|session| (session.id.clone(), session))
        .collect::<BTreeMap<_, _>>();
    let sessions = state
        .storage
        .list_session_records()
        .await?
        .into_iter()
        .map(|mut record| {
            if let Some(active) = active_sessions.get(&record.id) {
                record.state = active.state.as_str().to_string();
                record.last_heartbeat_at = active.last_heartbeat_at;
                record.expires_at = active.expires_at;
                record.source_ip = active.source_ip.clone();
            }
            record
        })
        .collect::<Vec<_>>();
    let leases = state.storage.list_leases().await?;
    let lease_by_session_id = leases
        .into_iter()
        .filter_map(|lease| {
            lease
                .session_id
                .clone()
                .map(|session_id| (session_id, lease))
        })
        .collect::<BTreeMap<_, _>>();

    Ok(axum::Json(AdminSessionsResponse {
        sessions: sessions
            .into_iter()
            .map(|session| {
                let lease = lease_by_session_id.get(&session.id).cloned();
                AdminSessionResponse {
                    user_id: lease.as_ref().map(|item| item.user_id.clone()),
                    lease,
                    source_ip: session.source_ip.clone(),
                    session,
                }
            })
            .collect(),
    }))
}

async fn update_admin_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<AdminSessionUpdateRequest>,
) -> Result<axum::Json<AdminSessionResponse>, ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    require_permission(&user, "sessions.update")?;

    if let Some(client_name) = request.client_name.as_deref() {
        validate_max_len(
            client_name.trim(),
            "client_name",
            validation::CLIENT_NAME_MAX_LEN,
        )?;
    }
    if let Some(failure_message) = request.failure_message.as_deref() {
        validate_max_len(
            failure_message.trim(),
            "failure_message",
            validation::LONG_DESCRIPTION_MAX_LEN,
        )?;
    }

    let session = state
        .storage
        .update_session_record(
            &session_id,
            clean_optional(request.client_name),
            clean_optional(request.failure_message),
        )
        .await?
        .ok_or_else(|| ApiError::not_found("session record not found"))?;

    let lease = state
        .storage
        .list_leases()
        .await?
        .into_iter()
        .find(|lease| lease.session_id.as_deref() == Some(&session.id));
    Ok(axum::Json(AdminSessionResponse {
        user_id: lease.as_ref().map(|item| item.user_id.clone()),
        source_ip: session.source_ip.clone(),
        lease,
        session,
    }))
}

async fn close_admin_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    require_permission(&user, "sessions.delete")?;
    state
        .request_session_stop(&session_id, SessionStopReason::ApiDelete)
        .await
        .ok_or_else(|| ApiError::not_found("active session not found"))?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_admin_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    require_permission(&user, "sessions.delete")?;
    let _ = state
        .request_session_stop(&session_id, SessionStopReason::ApiDelete)
        .await;
    state.storage.delete_session_record(&session_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_tftp_config(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminTftpConfigResponse>, ApiError> {
    let config = state.config.read().await.clone();
    Ok(axum::Json(AdminTftpConfigResponse { tftp: config.tftp }))
}

async fn get_tftp_status(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminTftpStatusResponse>, ApiError> {
    let mut status = state
        .tftp_manager
        .read()
        .await
        .status()
        .await
        .map_err(|err| {
            ApiError::service_unavailable(format!("failed to get TFTP status: {err}"))
        })?;
    let runtime = state.runtime_settings.read().await.clone();
    status.resolved_server_ip =
        resolve_server_network(&runtime)?.and_then(|network| network.server_ip);
    status.resolved_netmask = resolve_server_network(&runtime)?.and_then(|network| network.netmask);
    Ok(axum::Json(AdminTftpStatusResponse { status }))
}

async fn get_server_config(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminServerConfigResponse>, ApiError> {
    let config = state.config.read().await.clone();
    let site = state.storage.get_site_settings().await?;
    let runtime = state.storage.get_runtime_settings().await?;
    *state.runtime_settings.write().await = runtime.clone();
    Ok(axum::Json(server_config_response(&config, runtime, site)))
}

async fn update_server_config(
    headers: HeaderMap,
    State(state): State<AppState>,
    axum::Json(request): axum::Json<UpdateServerConfigRequest>,
) -> Result<axum::Json<AdminServerConfigResponse>, ApiError> {
    let current_user = current_user_from_headers(&state, &headers).await?;
    let runtime = state
        .storage
        .update_runtime_settings(
            runtime_settings_from_request(request.editable)?,
            Some(current_user.id.clone()),
        )
        .await?;
    *state.runtime_settings.write().await = runtime.clone();
    let site = state
        .storage
        .update_site_settings(
            site_settings_from_request(request.site)?,
            Some(current_user.id),
        )
        .await?;

    let config = state.config.read().await.clone();
    Ok(axum::Json(server_config_response(&config, runtime, site)))
}

async fn get_site_settings(
    State(state): State<AppState>,
) -> Result<axum::Json<SiteSettingsResponse>, ApiError> {
    let settings = state.storage.get_site_settings().await?;
    Ok(axum::Json(site_settings_response(settings)))
}

async fn update_site_settings(
    headers: HeaderMap,
    State(state): State<AppState>,
    axum::Json(request): axum::Json<SiteSettingsUpdateRequest>,
) -> Result<axum::Json<SiteSettingsResponse>, ApiError> {
    let current_user = current_user_from_headers(&state, &headers).await?;
    let settings = state
        .storage
        .update_site_settings(site_settings_from_request(request)?, Some(current_user.id))
        .await?;
    Ok(axum::Json(site_settings_response(settings)))
}

async fn reconcile_tftp(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminTftpStatusResponse>, ApiError> {
    {
        let manager = state.tftp_manager.read().await;
        manager.reconcile().await.map_err(|err| {
            ApiError::service_unavailable(format!("failed to reconcile TFTP: {err}"))
        })?;
    }
    get_tftp_status(State(state)).await
}

async fn list_board_types(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<BoardTypeSummary>>, ApiError> {
    let current_leased_board_ids = current_leased_board_ids(&state).await?;
    let boards = state.boards.read().await;
    let runtimes = state.board_runtimes.read().await;
    let result = summarize_board_types(&boards, &runtimes, &current_leased_board_ids);
    Ok(axum::Json(result))
}

async fn create_session(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::Json(request): axum::Json<CreateSessionRequest>,
) -> Result<(StatusCode, axum::Json<SessionCreatedResponse>), ApiError> {
    validate_len(
        request.board_type.trim(),
        "board_type",
        1,
        validation::BOARD_TYPE_MAX_LEN,
    )?;
    for tag in &request.required_tags {
        validate_max_len(tag.trim(), "required_tags", validation::TAG_MAX_LEN)?;
    }
    if let Some(client_name) = request.client_name.as_deref() {
        validate_max_len(
            client_name.trim(),
            "client_name",
            validation::CLIENT_NAME_MAX_LEN,
        )?;
    }

    let session = state
        .create_session(
            &request.board_type,
            &request.required_tags,
            request.client_name.clone(),
            Some(addr.ip().to_string()),
        )
        .await
        .map_err(|err| match err {
            BoardAllocationStatus::BoardTypeNotFound => {
                ApiError::not_found(format!("board type `{}` not found", request.board_type))
            }
            BoardAllocationStatus::NoAvailableBoard => ApiError::conflict(format!(
                "no available board for type `{}`",
                request.board_type
            )),
        })?;

    let board = state
        .session_board(&session.id)
        .await
        .ok_or_else(|| ApiError::not_found("allocated board disappeared"))?;
    let ws_url = board
        .serial
        .as_ref()
        .map(|_| format!("/api/v1/sessions/{}/serial/ws", session.id));

    Ok((
        StatusCode::CREATED,
        axum::Json(SessionCreatedResponse {
            session_id: session.id,
            board_id: board.id,
            lease_expires_at: session.expires_at,
            serial_available: board.serial.is_some(),
            boot_mode: board.boot.kind_name().to_string(),
            ws_url,
        }),
    ))
}

async fn get_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<SessionDetailResponse>, ApiError> {
    let session = get_session_or_404(&state, &session_id).await?;
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let files = session_file_responses(&state, &session_id, &board).await?;
    let connected = session.serial_connected;

    Ok(axum::Json(SessionDetailResponse {
        session,
        board: board.clone(),
        serial_available: board.serial.is_some(),
        serial_connected: connected,
        files,
    }))
}

async fn heartbeat_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<serde_json::Value>, ApiError> {
    let session = state
        .heartbeat_session(&session_id)
        .await
        .map_err(|err| match err {
            TouchSessionError::NotFound => {
                ApiError::not_found(format!("session `{session_id}` not found"))
            }
            TouchSessionError::Releasing => {
                ApiError::conflict(format!("session `{session_id}` is releasing"))
            }
        })?;
    Ok(axum::Json(json!({
        "session_id": session.id,
        "lease_expires_at": session.expires_at
    })))
}

async fn delete_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let removed = state
        .request_session_stop(&session_id, SessionStopReason::ApiDelete)
        .await;
    if removed.is_none() {
        return Err(ApiError::not_found(format!(
            "session `{session_id}` not found"
        )));
    }
    Ok(StatusCode::ACCEPTED)
}

async fn get_boot_profile(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<BootProfileResponse>, ApiError> {
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let network = resolved_board_network(&state, &board).await?;
    Ok(axum::Json(BootProfileResponse {
        boot: boot_profile_with_resolved_network(board.boot, network.as_ref()),
        server_ip: network.as_ref().and_then(|item| item.server_ip.clone()),
        netmask: network.as_ref().and_then(|item| item.netmask.clone()),
        interface: network.as_ref().and_then(|item| item.interface.clone()),
    }))
}

async fn get_session_dtb(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<SessionDtbResponse>, ApiError> {
    get_session_or_404(&state, &session_id).await?;
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let Some(dtb_name) = board_preset_dtb_name(&board).map(str::to_string) else {
        return Ok(axum::Json(SessionDtbResponse {
            dtb_name: None,
            relative_path: None,
            session_file_path: None,
            tftp_url: None,
        }));
    };

    let file = ensure_session_preset_dtb_file(&state, &session_id, &board).await?;
    let tftp_url = if let Some(file) = file {
        file_response_for_board(&state, &board, file)
            .await?
            .tftp_url
    } else {
        None
    };

    Ok(axum::Json(SessionDtbResponse {
        dtb_name: Some(dtb_name.clone()),
        relative_path: Some(session_dtb_relative_path(&session_id, &dtb_name)),
        session_file_path: Some(session_dtb_file_path(&dtb_name)),
        tftp_url,
    }))
}

async fn download_session_dtb(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Response, ApiError> {
    get_session_or_404(&state, &session_id).await?;
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let dtb_name = board_preset_dtb_name(&board)
        .ok_or_else(|| ApiError::not_found("board has no preset DTB configured"))?;
    let bytes = state.dtb_store.read(dtb_name).await.map_err(|err| {
        let message = err.to_string();
        if message.contains("No such file") || message.contains("not found") {
            ApiError::not_found(format!("preset DTB `{dtb_name}` not found"))
        } else {
            ApiError::from(err)
        }
    })?;

    Ok((
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            ),
            (
                header::CONTENT_DISPOSITION,
                HeaderValue::from_str(&format!("attachment; filename=\"{dtb_name}\""))
                    .unwrap_or_else(|_| HeaderValue::from_static("attachment")),
            ),
        ],
        bytes,
    )
        .into_response())
}

async fn get_serial_status(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<SerialStatusResponse>, ApiError> {
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let connected = state
        .get_session(&session_id)
        .await
        .map(|session| session.serial_connected)
        .unwrap_or(false);
    let response = if let Some(serial) = board.serial {
        let resolved = resolve_serial_config(&serial)
            .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?;
        SerialStatusResponse {
            available: true,
            connected,
            port: Some(resolved.current_device_path),
            baud_rate: Some(serial.baud_rate),
            ws_url: Some(format!("/api/v1/sessions/{session_id}/serial/ws")),
        }
    } else {
        SerialStatusResponse {
            available: false,
            connected: false,
            port: None,
            baud_rate: None,
            ws_url: None,
        }
    };
    Ok(axum::Json(response))
}

fn with_resolved_serial_config(mut board: BoardConfig) -> BoardConfig {
    if let Some(serial) = board.serial.as_mut()
        && let Ok(resolved) = resolve_serial_config(serial)
    {
        serial.resolved_device_path = Some(resolved.current_device_path);
        serial.resolved_usb_path = resolved.usb_path;
    }

    board
}

async fn serial_ws(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let session = state
        .session_state(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let board = session.board().clone();
    let Some(_serial) = board.serial.clone() else {
        return Err(ApiError::conflict("board has no serial configuration"));
    };
    if session.is_releasing() {
        return Err(ApiError::conflict("session is releasing"));
    }

    if !session.try_set_serial_connected() {
        return Err(ApiError::conflict("serial websocket already connected"));
    }

    Ok(ws.on_upgrade(move |socket| run_serial_ws(socket, state, session)))
}

async fn power_on_board(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<ActionResponse>, ApiError> {
    run_board_power_action(&state, &session_id, true).await
}

async fn power_off_board(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<ActionResponse>, ApiError> {
    run_board_power_action(&state, &session_id, false).await
}

async fn get_http_boot_file(
    Path((session_id, path)): Path<(String, String)>,
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, ApiError> {
    let relative_path = parse_relative_path(&path)?;
    let session = active_session_state_or_404(&state, &session_id).await?;
    ensure_httpboot_board(session.board())?;
    read_http_boot_session_file(&state, &session_id, &relative_path, request.headers()).await
}

async fn read_http_boot_session_file(
    state: &AppState,
    session_id: &str,
    relative_path: &str,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let config = state.config.read().await.clone();
    if !config.http_boot.enabled {
        return Err(ApiError::not_found("HTTP Boot is disabled"));
    }
    let manager = state.tftp_manager.read().await.clone();
    let file = manager
        .get_session_file(session_id, relative_path)
        .await
        .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?
        .ok_or_else(|| {
            ApiError::not_found(format!("HTTP Boot file `{relative_path}` not found"))
        })?;
    let disk_path = file.disk_path;
    let metadata = fs::metadata(&disk_path)
        .await
        .map_err(|err| match err.kind() {
            std::io::ErrorKind::NotFound => {
                ApiError::not_found(format!("HTTP Boot file `{relative_path}` not found"))
            }
            _ => ApiError::from(anyhow::Error::from(err)),
        })?;
    let file_len = usize::try_from(metadata.len()).map_err(|_| {
        ApiError::service_unavailable(format!("HTTP Boot file `{relative_path}` is too large"))
    })?;
    let content_type = from_path(&disk_path).first_or_octet_stream().to_string();
    let content_type = HeaderValue::from_str(&content_type).map_err(|err| {
        ApiError::service_unavailable(format!("invalid content type `{content_type}`: {err}"))
    })?;
    if let Some(range) = headers
        .get(header::RANGE)
        .and_then(|value| value.to_str().ok())
        && let Some((start, end)) = parse_single_byte_range(range, file_len)
    {
        let chunk = read_file_range(&disk_path, start, end).await?;
        return Ok((
            StatusCode::PARTIAL_CONTENT,
            [
                (header::CONTENT_TYPE, content_type),
                (
                    header::CONTENT_LENGTH,
                    HeaderValue::from_str(&chunk.len().to_string()).map_err(|err| {
                        ApiError::service_unavailable(format!(
                            "invalid content length header: {err}"
                        ))
                    })?,
                ),
                (
                    header::CONTENT_RANGE,
                    HeaderValue::from_str(&format!("bytes {start}-{end}/{file_len}")).map_err(
                        |err| {
                            ApiError::service_unavailable(format!(
                                "invalid content range header: {err}"
                            ))
                        },
                    )?,
                ),
                (header::ACCEPT_RANGES, HeaderValue::from_static("bytes")),
            ],
            chunk,
        )
            .into_response());
    }

    let body = fs::read(&disk_path)
        .await
        .map_err(|err| ApiError::from(anyhow::Error::from(err)))?;
    Ok(([(header::CONTENT_TYPE, content_type)], body).into_response())
}

async fn read_file_range(
    path: &std::path::Path,
    start: usize,
    end: usize,
) -> Result<Vec<u8>, ApiError> {
    let len = end
        .checked_sub(start)
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| ApiError::bad_request("invalid byte range"))?;
    let mut file = File::open(path)
        .await
        .map_err(|err| ApiError::from(anyhow::Error::from(err)))?;
    file.seek(SeekFrom::Start(start as u64))
        .await
        .map_err(|err| ApiError::from(anyhow::Error::from(err)))?;
    let mut chunk = vec![0; len];
    file.read_exact(&mut chunk)
        .await
        .map_err(|err| ApiError::from(anyhow::Error::from(err)))?;
    Ok(chunk)
}

fn parse_single_byte_range(range: &str, len: usize) -> Option<(usize, usize)> {
    let value = range.strip_prefix("bytes=")?;
    if value.contains(',') {
        return None;
    }
    let (start, end) = value.split_once('-')?;
    if start.is_empty() {
        return None;
    }
    let start = start.parse::<usize>().ok()?;
    let mut end = if end.is_empty() {
        len.checked_sub(1)?
    } else {
        end.parse::<usize>().ok()?
    };
    if start >= len {
        return None;
    }
    end = end.min(len - 1);
    if start > end {
        return None;
    }
    Some((start, end))
}

async fn put_http_boot_file(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    request: Request,
) -> Result<(StatusCode, axum::Json<HttpBootFileResponse>), ApiError> {
    let session = active_session_state_or_404(&state, &session_id).await?;
    ensure_httpboot_board(session.board())?;
    if !state.config.read().await.http_boot.enabled {
        return Err(ApiError::conflict("HTTP Boot is disabled"));
    }

    let (relative_path, file) =
        put_session_file_from_request(&state, &session_id, request, "HTTP Boot file").await?;
    let config = state.config.read().await.clone();
    let runtime = state.runtime_settings.read().await.clone();
    let response = http_boot_file_response(&config, &runtime, &session_id, &relative_path, file)?;
    Ok((StatusCode::CREATED, axum::Json(response)))
}

async fn put_http_boot_kernel(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    request: Request,
) -> Result<(StatusCode, axum::Json<KernelPublishResponse>), ApiError> {
    let session = active_session_state_or_404(&state, &session_id).await?;
    let board = session.board().clone();
    ensure_httpboot_board(&board)?;
    let headers = request.headers();
    let remote_name = optional_header(headers, "X-HttpBoot-Remote-Name")?
        .unwrap_or_else(|| "kernel.elf".to_string());
    let remote_name = parse_relative_path(&remote_name)?;
    let _arch = parse_httpboot_arch_header(headers)?;
    let _image_format = parse_httpboot_image_format_header(headers)?;
    let _entry_symbol = optional_header(headers, "X-HttpBoot-Entry-Symbol")?
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if !state.config.read().await.http_boot.enabled {
        return Err(ApiError::conflict("HTTP Boot is disabled"));
    }

    let (kernel_file, kernel_size, kernel_sha256) = put_session_file_bytes_from_request(
        &state,
        &session_id,
        &remote_name,
        request,
        "HTTP Boot kernel",
    )
    .await?;
    let config = state.config.read().await.clone();
    let runtime = state.runtime_settings.read().await.clone();
    let kernel_response =
        http_boot_file_response(&config, &runtime, &session_id, &remote_name, kernel_file)?;
    let response = publish_kernel(KernelPublishInput {
        kernel_url: kernel_response.http_url,
        kernel_size,
        kernel_sha256: Some(kernel_sha256),
    });

    Ok((StatusCode::CREATED, axum::Json(response)))
}

fn parse_httpboot_arch_header(headers: &HeaderMap) -> Result<BootArch, ApiError> {
    match required_header(headers, "X-HttpBoot-Arch")?.trim() {
        "x86_64" => Ok(BootArch::X86_64),
        "aarch64" => Ok(BootArch::Aarch64),
        "loongarch64" => Ok(BootArch::Loongarch64),
        "riscv64" => Ok(BootArch::Riscv64),
        "other" => Ok(BootArch::Other),
        other => Err(ApiError::bad_request(format!(
            "unsupported X-HttpBoot-Arch `{other}`"
        ))),
    }
}

fn parse_httpboot_image_format_header(headers: &HeaderMap) -> Result<ImageFormat, ApiError> {
    match optional_header(headers, "X-HttpBoot-Image-Format")?
        .unwrap_or_else(|| "elf64".to_string())
        .trim()
    {
        "elf64" => Ok(ImageFormat::Elf64),
        other => Err(ApiError::bad_request(format!(
            "unsupported X-HttpBoot-Image-Format `{other}`"
        ))),
    }
}

fn required_header(headers: &HeaderMap, name: &'static str) -> Result<String, ApiError> {
    optional_header(headers, name)?
        .ok_or_else(|| ApiError::bad_request(format!("missing {name} header")))
}

fn optional_header(headers: &HeaderMap, name: &'static str) -> Result<Option<String>, ApiError> {
    headers
        .get(name)
        .map(|value| {
            value
                .to_str()
                .map(str::to_string)
                .map_err(|_| ApiError::bad_request(format!("invalid {name} header")))
        })
        .transpose()
}

fn optional_clean_header(
    headers: &HeaderMap,
    name: &'static str,
) -> Result<Option<String>, ApiError> {
    Ok(clean_optional(optional_header(headers, name)?))
}

fn optional_clean_header_len(
    headers: &HeaderMap,
    name: &'static str,
    max: usize,
) -> Result<Option<String>, ApiError> {
    let value = optional_clean_header(headers, name)?;
    if let Some(value) = value.as_deref() {
        validate_max_len(value, name, max)?;
    }
    Ok(value)
}

fn optional_bool_header(headers: &HeaderMap, name: &'static str) -> Result<Option<bool>, ApiError> {
    optional_header(headers, name)?
        .map(|value| match value.trim().to_ascii_lowercase().as_str() {
            "true" | "1" | "yes" | "on" => Ok(true),
            "false" | "0" | "no" | "off" => Ok(false),
            _ => Err(ApiError::bad_request(format!("invalid {name} header"))),
        })
        .transpose()
}

fn dtb_metadata_headers(headers: &HeaderMap) -> Result<DtbMetadataInput, ApiError> {
    Ok(DtbMetadataInput {
        boot_architecture: optional_clean_header_len(
            headers,
            "X-Dtb-Architecture",
            validation::BOOT_ARCH_MAX_LEN,
        )?,
        compatible: optional_clean_header_len(
            headers,
            "X-Dtb-Compatible",
            validation::COMPATIBLE_MAX_LEN,
        )?,
        description: optional_clean_header_len(
            headers,
            "X-Dtb-Description",
            validation::LONG_DESCRIPTION_MAX_LEN,
        )?,
        disabled: optional_bool_header(headers, "X-Dtb-Disabled")?,
    })
}

async fn list_session_files(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<FileResponse>>, ApiError> {
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    Ok(axum::Json(
        session_file_responses(&state, &session_id, &board).await?,
    ))
}

async fn put_session_file(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    request: Request,
) -> Result<(StatusCode, axum::Json<FileResponse>), ApiError> {
    let session = state
        .session_state(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("session `{session_id}` not found")))?;
    if session.is_releasing() || session.is_stop_requested() {
        return Err(ApiError::conflict(format!(
            "session `{session_id}` is releasing"
        )));
    }
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    if !state.config.read().await.tftp.enabled() {
        return Err(ApiError::conflict("TFTP provider is disabled"));
    }

    let (_relative_path, file) =
        put_session_file_from_request(&state, &session_id, request, "session file").await?;
    let response = file_response_for_board(&state, &board, file).await?;
    Ok((StatusCode::CREATED, axum::Json(response)))
}

async fn get_session_file(
    Path((session_id, path)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<axum::Json<FileResponse>, ApiError> {
    let relative_path = parse_relative_path(&path)?;
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let manager = state.tftp_manager.read().await.clone();
    let file = manager
        .get_session_file(&session_id, &relative_path)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("file `{relative_path}` not found")))?;
    Ok(axum::Json(
        file_response_for_board(&state, &board, file).await?,
    ))
}

async fn reject_legacy_put_session_file(
    Path((_session_id, _path)): Path<(String, String)>,
) -> Result<StatusCode, ApiError> {
    Err(ApiError::not_found(
        "upload files via PUT /api/v1/sessions/{session_id}/files with X-File-Path",
    ))
}

async fn delete_session_file(
    Path((session_id, path)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let relative_path = parse_relative_path(&path)?;
    let session = state
        .session_state(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("session `{session_id}` not found")))?;
    if session.is_releasing() || session.is_stop_requested() {
        return Err(ApiError::conflict(format!(
            "session `{session_id}` is releasing"
        )));
    }
    let manager = state.tftp_manager.read().await.clone();
    manager
        .remove_session_file(&session_id, &relative_path)
        .await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_session_tftp_status(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<TftpSessionResponse>, ApiError> {
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let status = state.tftp_manager.read().await.status().await?;
    let server_ip = resolved_board_network(&state, &board)
        .await?
        .and_then(|network| network.server_ip);
    let files = session_file_responses(&state, &session_id, &board).await?;

    Ok(axum::Json(TftpSessionResponse {
        available: status.enabled && status.healthy && status.writable && server_ip.is_some(),
        provider: status.provider,
        server_ip,
        netmask: resolved_board_network(&state, &board)
            .await?
            .and_then(|network| network.netmask),
        writable: status.writable,
        files,
    }))
}

async fn get_session_or_404(
    state: &AppState,
    session_id: &str,
) -> Result<crate::session::Session, ApiError> {
    state
        .get_session(session_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("session `{session_id}` not found")))
}

async fn put_session_file_from_request(
    state: &AppState,
    session_id: &str,
    request: Request,
    body_label: &'static str,
) -> Result<(String, TftpFileRef), ApiError> {
    let relative_path = required_relative_path_header(request.headers(), "X-File-Path")?;
    let (file, _, _) =
        put_session_file_bytes_from_request(state, session_id, &relative_path, request, body_label)
            .await?;
    Ok((relative_path, file))
}

async fn put_session_file_bytes_from_request(
    state: &AppState,
    session_id: &str,
    relative_path: &str,
    request: Request,
    body_label: &'static str,
) -> Result<(TftpFileRef, u64, String), ApiError> {
    let max_mib = state
        .runtime_settings
        .read()
        .await
        .upload_limits
        .session_file_max_mib;
    let body = read_limited_body(request, max_mib, body_label).await?;
    let size = body.len() as u64;
    let sha256 = hex_sha256(&body);
    let manager = state.tftp_manager.read().await.clone();
    let file = manager
        .put_session_file(session_id, relative_path, &body)
        .await
        .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?;
    Ok((file, size, sha256))
}

fn required_relative_path_header(
    headers: &HeaderMap,
    name: &'static str,
) -> Result<String, ApiError> {
    let relative_path = headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request(format!("missing {name} header")))?;
    parse_relative_path(relative_path)
}

async fn session_file_responses(
    state: &AppState,
    session_id: &str,
    board: &BoardConfig,
) -> Result<Vec<FileResponse>, ApiError> {
    let manager = state.tftp_manager.read().await.clone();
    let files = manager.list_session_files(session_id).await?;
    let mut responses = Vec::with_capacity(files.len());
    for file in files {
        responses.push(file_response_for_board(state, board, file).await?);
    }
    Ok(responses)
}

async fn file_response_for_board(
    state: &AppState,
    board: &BoardConfig,
    file: TftpFileRef,
) -> Result<FileResponse, ApiError> {
    let tftp_url = resolved_board_network(state, board)
        .await?
        .and_then(|network| network.server_ip)
        .map(|server_ip| format!("tftp://{server_ip}/{}", file.relative_path));
    Ok(FileResponse::from_file(file, tftp_url))
}

async fn run_board_power_action(
    state: &AppState,
    session_id: &str,
    power_on: bool,
) -> Result<axum::Json<ActionResponse>, ApiError> {
    let session = state
        .session_state(session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    if session.is_releasing() {
        return Err(ApiError::conflict(format!(
            "session `{session_id}` is releasing"
        )));
    }
    let board = state
        .session_board(session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;

    let action = if power_on {
        PowerAction::On
    } else {
        PowerAction::Off
    };
    let message = state
        .execute_board_power_action(&board, action)
        .await
        .map_err(|err| match err {
            PowerActionError::NotConfigured | PowerActionError::InvalidConfig(_) => {
                ApiError::bad_request(err.to_string())
            }
            PowerActionError::Execution(err) => ApiError::from(err),
        })?;

    Ok(axum::Json(ActionResponse { ok: true, message }))
}

fn board_power_action(action: PowerAction) -> BoardPowerAction {
    match action {
        PowerAction::On => BoardPowerAction::PowerOn,
        PowerAction::Off => BoardPowerAction::PowerOff,
    }
}

fn parse_relative_path(raw: &str) -> Result<String, ApiError> {
    normalize_relative_path(raw).map_err(|err| ApiError::bad_request(err.to_string()))
}

async fn active_session_state_or_404(
    state: &AppState,
    session_id: &str,
) -> Result<Arc<SessionState>, ApiError> {
    let session = state
        .session_state(session_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("session `{session_id}` not found")))?;
    if session.is_releasing() || session.is_stop_requested() {
        return Err(ApiError::conflict(format!(
            "session `{session_id}` is releasing"
        )));
    }
    Ok(session)
}

fn ensure_httpboot_board(board: &BoardConfig) -> Result<(), ApiError> {
    if matches!(board.boot, BootConfig::UefiHttp(_)) {
        Ok(())
    } else {
        Err(ApiError::bad_request(format!(
            "board `{}` does not use `httpboot` boot",
            board.id
        )))
    }
}

fn http_boot_file_response(
    config: &ServerConfig,
    runtime: &RuntimeSettings,
    session_id: &str,
    relative_path: &str,
    file: TftpFileRef,
) -> Result<HttpBootFileResponse, ApiError> {
    let http_url = http_boot_url(config, runtime, session_id, relative_path)?;
    Ok(HttpBootFileResponse::from_file(file, http_url))
}

fn http_boot_url(
    config: &ServerConfig,
    runtime: &RuntimeSettings,
    session_id: &str,
    relative_path: &str,
) -> Result<String, ApiError> {
    let relative_path = normalize_relative_path(relative_path)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let base_url = http_boot_public_base_url(config, runtime)?;
    let base_url = base_url.trim_end_matches('/');
    Ok(format!(
        "{base_url}/boot/sessions/{session_id}/{relative_path}"
    ))
}

fn http_boot_public_base_url(
    config: &ServerConfig,
    runtime: &RuntimeSettings,
) -> Result<String, ApiError> {
    if let Some(public_base_url) = config.http_boot.public_base_url.as_deref()
        && !public_base_url.trim().is_empty()
    {
        return Ok(public_base_url.trim().to_string());
    }

    if let Some(network) = resolve_server_network(runtime)?
        && let Some(server_ip) = network.server_ip
    {
        return Ok(format!(
            "http://{}:{}",
            server_ip,
            config.listen_addr.port()
        ));
    }

    Ok(format!("http://{}", config.listen_addr))
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    let mut hex = String::with_capacity(digest.len() * 2);
    for byte in digest {
        use std::fmt::Write as _;
        let _ = write!(&mut hex, "{byte:02x}");
    }
    hex
}

async fn sync_dtb_metadata(
    state: &AppState,
    file: &crate::dtb_store::DtbFile,
    bytes: &[u8],
    metadata: DtbMetadataInput,
    uploaded_by: Option<String>,
) -> Result<crate::storage::DtbMetadata, ApiError> {
    let existing = state.storage.find_dtb_metadata_by_name(&file.name).await?;
    Ok(state
        .storage
        .upsert_dtb_metadata(UpsertDtbMetadata {
            name: file.name.clone(),
            storage_path: file.name.clone(),
            size_bytes: file.size as i64,
            sha256: hex_sha256(bytes),
            boot_architecture: metadata.boot_architecture,
            compatible: metadata.compatible,
            description: metadata.description,
            disabled: metadata
                .disabled
                .unwrap_or_else(|| existing.as_ref().is_some_and(|item| item.disabled)),
            uploaded_by,
        })
        .await?)
}

async fn write_audit_log(
    state: &AppState,
    action: &str,
    target_type: &str,
    target_id: Option<String>,
    metadata: serde_json::Value,
) -> Result<(), ApiError> {
    state
        .storage
        .create_audit_log(NewAuditLog {
            actor_user_id: None,
            actor_username: None,
            action: action.to_string(),
            target_type: target_type.to_string(),
            target_id,
            outcome: "success".to_string(),
            ip_address: None,
            user_agent: None,
            request_id: None,
            metadata_json: metadata.to_string(),
        })
        .await?;
    Ok(())
}

fn summarize_board_types(
    boards: &BTreeMap<String, BoardConfig>,
    runtimes: &BTreeMap<String, crate::state::BoardRuntimeState>,
    current_leased_board_ids: &BTreeSet<String>,
) -> Vec<BoardTypeSummary> {
    let mut aggregate = BTreeMap::<String, (BTreeSet<String>, usize, usize)>::new();
    for board in boards.values().filter(|board| !board.disabled) {
        let entry = aggregate
            .entry(board.board_type.clone())
            .or_insert_with(|| (BTreeSet::new(), 0, 0));
        for tag in &board.tags {
            entry.0.insert(tag.clone());
        }
        entry.1 += 1;
        let has_active_runtime = runtimes
            .get(&board.id)
            .is_some_and(|runtime| runtime.lease_state != BoardLeaseState::Idle);
        if !current_leased_board_ids.contains(&board.id) && !has_active_runtime {
            entry.2 += 1;
        }
    }

    aggregate
        .into_iter()
        .map(|(board_type, (tags, total, available))| BoardTypeSummary {
            board_type,
            tags: tags.into_iter().collect(),
            total,
            available,
        })
        .collect::<Vec<_>>()
}

async fn current_leased_board_ids(state: &AppState) -> Result<BTreeSet<String>, ApiError> {
    let now = chrono::Utc::now();
    Ok(state
        .storage
        .list_leases()
        .await?
        .into_iter()
        .filter(|lease| {
            matches!(
                lease.state,
                crate::storage::LeaseState::Active | crate::storage::LeaseState::Releasing
            ) && lease.starts_at <= now
                && now < lease.expires_at
        })
        .map(|lease| lease.board_id)
        .collect())
}

async fn session_snapshots(state: &AppState) -> Vec<crate::session::Session> {
    let sessions = state
        .sessions
        .read()
        .await
        .values()
        .cloned()
        .collect::<Vec<_>>();
    join_all(
        sessions
            .into_iter()
            .map(|session| async move { session.snapshot().await }),
    )
    .await
}

fn boards_referencing_dtb(boards: &BTreeMap<String, BoardConfig>, dtb_name: &str) -> Vec<String> {
    boards
        .values()
        .filter(|board| board_preset_dtb_name(board) == Some(dtb_name))
        .map(|board| board.id.clone())
        .collect()
}

fn readonly_server_config(config: &crate::config::ServerConfig) -> AdminServerConfigReadonly {
    AdminServerConfigReadonly {
        listen_addr: config.listen_addr.to_string(),
        data_dir: config.data_dir.display().to_string(),
        board_dir: config.board_dir.display().to_string(),
        dtb_dir: config.dtb_dir.display().to_string(),
        http_boot_public_base_url: config.http_boot.public_base_url.clone(),
        dtb_upload_max_mib: DTB_UPLOAD_MAX_MIB,
    }
}

fn server_config_response(
    config: &crate::config::ServerConfig,
    runtime: RuntimeSettings,
    site: SiteSettings,
) -> AdminServerConfigResponse {
    AdminServerConfigResponse {
        readonly: readonly_server_config(config),
        editable: AdminServerConfigEditable {
            network: runtime.network,
            upload_limits: runtime.upload_limits,
        },
        site: site_settings_response(site),
    }
}

fn runtime_settings_from_request(
    request: AdminServerConfigEditable,
) -> Result<RuntimeSettings, ApiError> {
    let mut settings = RuntimeSettings {
        network: request.network,
        upload_limits: request.upload_limits,
        updated_at: chrono::Utc::now(),
    };
    settings
        .validate()
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(settings)
}

fn site_settings_response(settings: SiteSettings) -> SiteSettingsResponse {
    SiteSettingsResponse {
        site_name: settings.site_name,
        site_subtitle: settings.site_subtitle,
        logo_url: settings.logo_url,
        favicon_url: settings.favicon_url,
        announcement: settings.announcement,
        maintenance_mode: settings.maintenance_mode,
        self_service_enabled: settings.self_service_enabled,
        registration_mode: settings.registration_mode.as_str().to_string(),
        default_lease_minutes: settings.default_lease_minutes,
        max_lease_minutes: settings.max_lease_minutes,
        support_email: settings.support_email,
        support_url: settings.support_url,
        updated_at: settings.updated_at,
    }
}

fn site_settings_from_request(
    request: SiteSettingsUpdateRequest,
) -> Result<SiteSettings, ApiError> {
    let registration_mode = crate::storage::RegistrationMode::parse(&request.registration_mode)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let mut settings = SiteSettings {
        site_name: request.site_name,
        site_subtitle: request.site_subtitle,
        logo_url: request.logo_url,
        favicon_url: request.favicon_url,
        announcement: request.announcement,
        maintenance_mode: request.maintenance_mode,
        self_service_enabled: request.self_service_enabled,
        registration_mode,
        default_lease_minutes: request.default_lease_minutes,
        max_lease_minutes: request.max_lease_minutes,
        support_email: request.support_email,
        support_url: request.support_url,
        updated_at: chrono::Utc::now(),
    };
    settings
        .validate()
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    Ok(settings)
}

fn mib_to_bytes(limit_mib: u32) -> usize {
    (limit_mib as usize) * 1024 * 1024
}

fn payload_too_large_message(label: &str, max_mib: u32) -> String {
    format!("{label} upload body exceeds limit of {max_mib} MiB")
}

async fn read_limited_body(request: Request, max_mib: u32, label: &str) -> Result<Bytes, ApiError> {
    let limit_bytes = mib_to_bytes(max_mib);
    let headers = request.headers();
    if let Some(content_length) = headers
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        && content_length > limit_bytes as u64
    {
        return Err(ApiError::payload_too_large(payload_too_large_message(
            label, max_mib,
        )));
    }

    to_bytes(request.into_body(), limit_bytes)
        .await
        .map_err(|err| {
            let message = err.to_string();
            if message.contains("length limit exceeded") {
                ApiError::payload_too_large(payload_too_large_message(label, max_mib))
            } else {
                ApiError::bad_request(format!("failed to read {label} upload body: {message}"))
            }
        })
}

#[derive(Debug, Clone)]
struct ResolvedNetwork {
    interface: Option<String>,
    server_ip: Option<String>,
    netmask: Option<String>,
}

fn resolve_server_network(settings: &RuntimeSettings) -> Result<Option<ResolvedNetwork>, ApiError> {
    let interface = if settings.network.interface.trim().is_empty() {
        default_non_loopback_interface_name()
    } else {
        Some(settings.network.interface.trim().to_string())
    };
    if interface.as_deref() == Some("lo") {
        return Ok(Some(ResolvedNetwork {
            interface,
            server_ip: Some("127.0.0.1".into()),
            netmask: Some("255.0.0.0".into()),
        }));
    }
    let interfaces = discover_network_interfaces().map_err(|err| {
        ApiError::service_unavailable(format!("failed to enumerate network interfaces: {err:#}"))
    })?;
    let matched = interfaces
        .into_iter()
        .find(|item| interface.as_deref() == Some(item.name.as_str()));
    let server_ip = if let Some(interface_name) = interface.as_deref() {
        resolve_interface_ipv4(interface_name).map_err(|err| {
            ApiError::service_unavailable(format!("failed to resolve interface IP: {err}"))
        })?
    } else {
        None
    };
    let netmask = matched.and_then(|item| item.netmask);

    Ok(Some(ResolvedNetwork {
        interface,
        server_ip,
        netmask,
    }))
}

async fn resolved_board_network(
    state: &AppState,
    board: &BoardConfig,
) -> Result<Option<ResolvedNetwork>, ApiError> {
    let BootConfig::Uboot(profile) = &board.boot else {
        return Ok(None);
    };
    if !profile.use_tftp {
        return Ok(None);
    }

    if profile.network_mode == UbootNetworkMode::StaticIp
        && profile.server_ip.is_some()
        && profile.netmask.is_some()
    {
        return Ok(Some(ResolvedNetwork {
            interface: None,
            server_ip: profile.server_ip.clone(),
            netmask: profile.netmask.clone(),
        }));
    }

    let runtime = state.runtime_settings.read().await.clone();
    let mut network = resolve_server_network(&runtime)?;
    if profile.network_mode == UbootNetworkMode::StaticIp
        && let Some(network) = network.as_mut()
    {
        if let Some(server_ip) = profile.server_ip.as_ref() {
            network.server_ip = Some(server_ip.clone());
        }
        if let Some(netmask) = profile.netmask.as_ref() {
            network.netmask = Some(netmask.clone());
        }
    }
    Ok(network)
}

fn boot_profile_with_resolved_network(
    boot: BootConfig,
    network: Option<&ResolvedNetwork>,
) -> BootConfig {
    let BootConfig::Uboot(mut profile) = boot else {
        return boot;
    };
    if profile.network_mode == UbootNetworkMode::StaticIp {
        if profile.server_ip.is_none() {
            profile.server_ip = network.and_then(|item| item.server_ip.clone());
        }
        if profile.netmask.is_none() {
            profile.netmask = network.and_then(|item| item.netmask.clone());
        }
    }
    BootConfig::Uboot(profile)
}

fn dtb_name_header(headers: &HeaderMap, name: &str) -> Result<String, ApiError> {
    let value = headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request(format!("missing {name} header")))?;
    let value = normalize_dtb_name(value).map_err(|err| ApiError::bad_request(err.to_string()))?;
    validate_max_len(&value, name, validation::DTB_NAME_MAX_LEN)?;
    Ok(value)
}

fn board_preset_dtb_name(board: &BoardConfig) -> Option<&str> {
    let BootConfig::Uboot(profile) = &board.boot else {
        return None;
    };
    profile.dtb_name.as_deref()
}

fn session_dtb_file_path(dtb_name: &str) -> String {
    format!("boot/dtb/{dtb_name}")
}

fn session_dtb_relative_path(session_id: &str, dtb_name: &str) -> String {
    format!(
        "ostool/sessions/{session_id}/{}",
        session_dtb_file_path(dtb_name)
    )
}

async fn ensure_session_preset_dtb_file(
    state: &AppState,
    session_id: &str,
    board: &BoardConfig,
) -> Result<Option<TftpFileRef>, ApiError> {
    let Some(dtb_name) = board_preset_dtb_name(board) else {
        return Ok(None);
    };
    let file_path = session_dtb_file_path(dtb_name);
    let manager = state.tftp_manager.read().await.clone();
    if let Some(existing) = manager.get_session_file(session_id, &file_path).await? {
        return Ok(Some(existing));
    }

    let bytes = state.dtb_store.read(dtb_name).await.map_err(|err| {
        let message = err.to_string();
        if message.contains("No such file") || message.contains("not found") {
            ApiError::not_found(format!("preset DTB `{dtb_name}` not found"))
        } else {
            ApiError::from(err)
        }
    })?;
    let file = manager
        .put_session_file(session_id, &file_path, &bytes)
        .await
        .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?;
    Ok(Some(file))
}

async fn rewrite_board_dtb_references(
    state: &AppState,
    old_name: &str,
    new_name: &str,
) -> Result<(), ApiError> {
    let affected = {
        let boards = state.boards.read().await;
        boards
            .values()
            .filter_map(|board| {
                let mut next = board.clone();
                let BootConfig::Uboot(profile) = &mut next.boot else {
                    return None;
                };
                if profile.dtb_name.as_deref() != Some(old_name) {
                    return None;
                }
                profile.dtb_name = Some(new_name.to_string());
                Some(next)
            })
            .collect::<Vec<_>>()
    };

    for board in &affected {
        state
            .storage
            .update_board_config(&board.id, board.clone())
            .await?;
    }

    if !affected.is_empty() {
        let mut boards = state.boards.write().await;
        for board in affected {
            boards.insert(board.id.clone(), board);
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::storage::{Permission, Role};

    fn permission(code: &str) -> Permission {
        Permission {
            id: format!("perm-{code}"),
            code: code.to_string(),
            name: code.to_string(),
            description: String::new(),
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn role(name: &str) -> Role {
        Role {
            id: format!("role-{name}"),
            name: name.to_string(),
            display_name: name.to_string(),
            description: String::new(),
            system: false,
            disabled: false,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    fn current_user(roles: Vec<Role>, permissions: Vec<Permission>) -> CurrentUser {
        CurrentUser {
            id: "user-1".to_string(),
            username: "operator".to_string(),
            display_name: "Operator".to_string(),
            nickname: None,
            avatar_url: None,
            email: "operator@example.com".to_string(),
            phone: None,
            department: None,
            title: None,
            last_login_at: None,
            roles,
            permissions,
        }
    }

    #[test]
    fn scoped_admin_permission_allows_admin_area_access() {
        let user = current_user(vec![role("operator")], vec![permission("leases.delete")]);

        assert!(current_user_is_admin(&user));
        assert!(user_has_permission(&user, "leases.delete"));
        assert!(!user_has_permission(&user, "sessions.delete"));
        assert!(!user_has_permission(&user, "users.delete"));
    }

    #[test]
    fn exact_permission_does_not_cover_other_actions() {
        let user = current_user(vec![role("operator")], vec![permission("leases.update")]);

        assert!(current_user_is_admin(&user));
        assert!(user_has_permission(&user, "leases.update"));
        assert!(!user_has_permission(&user, "leases.read"));
        assert!(!user_has_permission(&user, "leases.delete"));
        assert!(!user_has_permission(&user, "leases.release"));
    }

    #[test]
    fn overview_only_user_has_no_admin_area_access() {
        let user = current_user(vec![role("user")], Vec::new());

        assert!(!current_user_is_admin(&user));
        assert!(!user_has_permission(&user, "leases.delete"));
    }

    #[test]
    fn default_user_permissions_do_not_allow_admin_area_access() {
        let user = current_user(
            vec![role("user")],
            vec![
                permission("leases.read"),
                permission("leases.create"),
                permission("leases.start"),
                permission("leases.release"),
                permission("leases.heartbeat"),
                permission("sessions.read"),
                permission("sessions.create"),
                permission("sessions.update"),
            ],
        );

        assert!(!current_user_is_admin(&user));
        assert!(user_has_permission(&user, "leases.create"));
        assert!(user_has_permission(&user, "sessions.create"));
        assert!(!user_has_permission(&user, "sessions.delete"));
    }

    #[test]
    fn admin_request_permissions_are_resource_scoped() {
        assert_eq!(
            admin_permission_for_request(&Method::GET, "/api/v1/admin/boards"),
            Some("boards.read")
        );
        assert_eq!(
            admin_permission_for_request(&Method::POST, "/api/v1/admin/boards"),
            Some("boards.create")
        );
        assert_eq!(
            admin_permission_for_request(&Method::PUT, "/api/v1/admin/boards/board-1"),
            Some("boards.update")
        );
        assert_eq!(
            admin_permission_for_request(&Method::DELETE, "/api/v1/admin/boards/board-1"),
            Some("boards.delete")
        );
        assert_eq!(
            admin_permission_for_request(&Method::POST, "/api/v1/admin/leases/lease-1/session"),
            Some("leases.start")
        );
        assert_eq!(
            admin_permission_for_request(&Method::POST, "/api/v1/admin/roles/role-1/disable"),
            Some("roles.update")
        );
        assert_eq!(
            admin_permission_for_request(&Method::POST, "/api/v1/admin/leases/lease-1/release"),
            Some("leases.release")
        );
        assert_eq!(
            admin_permission_for_request(&Method::DELETE, "/api/v1/admin/leases/lease-1"),
            Some("leases.delete")
        );
        assert_eq!(
            admin_permission_for_request(&Method::DELETE, "/api/v1/admin/sessions/session-1"),
            Some("sessions.delete")
        );
        assert_eq!(
            admin_permission_for_request(&Method::PUT, "/api/v1/admin/sessions/session-1"),
            Some("sessions.update")
        );
        assert_eq!(
            admin_permission_for_request(&Method::POST, "/api/v1/admin/sessions/session-1/close"),
            Some("sessions.delete")
        );
        assert_eq!(
            admin_permission_for_request(&Method::POST, "/api/v1/admin/tftp/reconcile"),
            Some("tftp.reconcile")
        );
        assert_eq!(
            admin_permission_for_request(&Method::GET, "/api/v1/admin/audit-logs"),
            Some("audit.read")
        );
    }

    #[test]
    fn user_request_permissions_use_resource_actions() {
        assert_eq!(
            user_permission_for_request(&Method::POST, "/api/v1/user/password"),
            Some("profile.update")
        );
        assert_eq!(
            user_permission_for_request(&Method::GET, "/api/v1/user/leases"),
            Some("leases.read")
        );
        assert_eq!(
            user_permission_for_request(&Method::GET, "/api/v1/user/leases/availability"),
            Some("leases.read")
        );
        assert_eq!(
            user_permission_for_request(&Method::POST, "/api/v1/user/leases"),
            Some("leases.create")
        );
        assert_eq!(
            user_permission_for_request(&Method::DELETE, "/api/v1/user/leases/lease-1"),
            Some("leases.release")
        );
        assert_eq!(
            user_permission_for_request(&Method::POST, "/api/v1/user/leases/lease-1/heartbeat"),
            Some("leases.heartbeat")
        );
    }

    #[test]
    fn captcha_hash_ignores_case_and_surrounding_whitespace() {
        assert_eq!(
            captcha_answer_hash(" AB12CD "),
            captcha_answer_hash("ab12cd")
        );
    }

    #[test]
    fn rate_limit_rules_cover_sensitive_endpoints() {
        let captcha = rate_limit_rule(&Method::GET, "/api/v1/auth/captcha").unwrap();
        assert_eq!(captcha.name, "captcha");
        assert_eq!(captcha.max_requests, 20);

        let login = rate_limit_rule(&Method::POST, "/api/v1/auth/login").unwrap();
        assert_eq!(login.name, "login");
        assert_eq!(login.max_requests, 10);

        let password = rate_limit_rule(&Method::POST, "/api/v1/user/password").unwrap();
        assert_eq!(password.name, "password");
        assert_eq!(password.max_requests, 10);

        let asset = rate_limit_rule(&Method::GET, "/assets/app.js");
        assert!(asset.is_none());
    }

    #[test]
    fn host_origin_matching_uses_request_host() {
        assert!(host_matches_origin(
            "127.0.0.1:2999",
            "http://127.0.0.1:2999"
        ));
        assert!(host_matches_origin(
            "example.com",
            "https://example.com/dashboard"
        ));
        assert!(!host_matches_origin("example.com", "https://evil.example"));
    }
}
