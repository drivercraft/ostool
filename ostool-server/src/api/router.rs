use std::{
    collections::{BTreeMap, BTreeSet},
    net::SocketAddr,
};

use axum::{
    Router,
    body::{Bytes, to_bytes},
    extract::{ConnectInfo, Path, Request, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use futures_util::future::join_all;
use httpboot_protocol::{BootArch, ImageFormat};
use mime_guess::from_path;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::{
    fs::{self, File},
    io::{AsyncReadExt, AsyncSeekExt, SeekFrom},
};

use crate::{
    api::{
        dto::{
            ActionResponse, AdminBoardUpsertRequest, AdminLeaseCreateRequest,
            AdminLeaseUpdateRequest, AdminOverviewResponse, AdminPasswordResetRequest,
            AdminPermissionResponse, AdminPermissionsResponse, AdminRoleCreateRequest,
            AdminRoleResponse, AdminRoleUpdateRequest, AdminRolesResponse,
            AdminServerConfigEditable, AdminServerConfigReadonly, AdminServerConfigResponse,
            AdminSessionResponse, AdminSessionsResponse, AdminTftpConfigResponse,
            AdminTftpStatusResponse, AdminUserCreateRequest, AdminUserResponse,
            AdminUserRolesResponse, AdminUserRolesUpdateRequest, AdminUserUpdateRequest,
            AdminUsersResponse, BoardPowerAction, BoardPowerStatusResponse,
            BoardRuntimeStatusResponse, BoardTypeSummary, BootProfileResponse, CreateLeaseRequest,
            CreateSessionRequest, CurrentUserResponse, DtbFileResponse, FileResponse,
            HttpBootFileResponse, KernelPublishResponse, LeaseResponse, LeasesResponse,
            LoginRequest, NetworkInterfaceSummary, SerialPortSummary, SerialStatusResponse,
            SessionCreatedResponse, SessionDetailResponse, SessionDtbResponse,
            SiteSettingsResponse, SiteSettingsUpdateRequest, TftpSessionResponse,
            UpdateServerConfigRequest,
        },
        error::ApiError,
    },
    auth::{
        CurrentUser, clear_cookie_value, cookie_value, hash_password, set_cookie_header,
        token_from_headers,
    },
    board_pool::BoardAllocationStatus,
    config::{
        BoardConfig, BootConfig, PowerManagementConfig, ServerConfig, TftpConfig, UbootNetworkMode,
    },
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
    state::{AppState, BoardLeaseState, TouchSessionError},
    storage::{Lease, NewAuditLog, NewRole, Role, SiteSettings, UpsertDtbMetadata, UserProfile},
    tftp::{
        files::{TftpFileRef, normalize_relative_path},
        service::build_tftp_manager,
        status::resolve_interface_ipv4,
    },
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
        .route("/api/v1/auth/logout", post(logout))
        .route("/api/v1/auth/me", get(get_current_user))
        .route("/api/v1/user/profile", get(get_user_profile))
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
            delete(delete_admin_session),
        )
        .route(
            "/api/v1/admin/tftp",
            get(get_tftp_config).put(update_tftp_config),
        )
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
        .with_state(state)
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
    } else if path.starts_with("/api/v1/user/") {
        let _ = current_user_from_headers(&state, &headers).await?;
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
        .map(|role| AdminRoleResponse::new(role, Vec::new()))
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
        || user
            .permissions
            .iter()
            .any(|permission| permission.code == "settings.manage")
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

fn request_profile(
    nickname: Option<String>,
    avatar_url: Option<String>,
    phone: Option<String>,
    department: Option<String>,
    title: Option<String>,
) -> UserProfile {
    UserProfile {
        nickname: clean_optional(nickname),
        avatar_url: clean_optional(avatar_url),
        phone: clean_optional(phone),
        department: clean_optional(department),
        title: clean_optional(title),
    }
}

async fn admin_role_response(state: &AppState, role: Role) -> Result<AdminRoleResponse, ApiError> {
    let permissions = state.storage.role_permissions(&role.id).await?;
    Ok(AdminRoleResponse::new(role, permissions))
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

async fn login(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<LoginRequest>,
) -> Result<Response, ApiError> {
    let (user, token) = state
        .auth
        .login(request.username.trim(), &request.password)
        .await
        .map_err(|_| ApiError::unauthorized("invalid username or password"))?;
    let mut response = axum::Json(user_response(user)).into_response();
    set_cookie_header(response.headers_mut(), cookie_value(&token)).map_err(ApiError::from)?;
    Ok(response)
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

async fn create_lease(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    headers: HeaderMap,
    axum::Json(request): axum::Json<CreateLeaseRequest>,
) -> Result<(StatusCode, axum::Json<LeaseResponse>), ApiError> {
    let user = current_user_from_headers(&state, &headers).await?;
    if request.board_type.trim().is_empty() {
        return Err(ApiError::bad_request("board_type must not be empty"));
    }
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
    if request.username.trim().is_empty() || request.password.is_empty() {
        return Err(ApiError::bad_request("username and password are required"));
    }
    let user = state
        .auth
        .create_user(
            request.username.trim().to_string(),
            request.display_name.trim().to_string(),
            request.email.trim().to_string(),
            request.password,
            request_profile(
                request.nickname,
                request.avatar_url,
                request.phone,
                request.department,
                request.title,
            ),
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
    let user = state
        .storage
        .update_user(
            &user_id,
            request.display_name,
            request.email,
            request_profile(
                request.nickname,
                request.avatar_url,
                request.phone,
                request.department,
                request.title,
            ),
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
    if request.password.is_empty() {
        return Err(ApiError::bad_request("password must not be empty"));
    }
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
    let mut roles = Vec::new();
    for role in state.storage.list_roles().await? {
        roles.push(admin_role_response(&state, role).await?);
    }
    Ok(axum::Json(AdminRolesResponse { roles }))
}

async fn create_admin_role(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<AdminRoleCreateRequest>,
) -> Result<(StatusCode, axum::Json<AdminRoleResponse>), ApiError> {
    let name = request.name.trim();
    if name.is_empty() {
        return Err(ApiError::bad_request("role name must not be empty"));
    }
    if !name
        .chars()
        .all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-')
    {
        return Err(ApiError::bad_request(
            "role name must contain only lowercase letters, numbers, '_' or '-'",
        ));
    }
    let role = state
        .storage
        .create_role(NewRole {
            name: name.to_string(),
            display_name: request.display_name.trim().to_string(),
            description: request.description.trim().to_string(),
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
    let role = state
        .storage
        .update_role(
            &role_id,
            request.display_name.trim().to_string(),
            request.description.trim().to_string(),
            request.permission_ids,
        )
        .await?
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
    let user = state
        .storage
        .find_user_by_id(request.user_id.trim())
        .await?
        .ok_or_else(|| ApiError::not_found("user not found"))?;
    if user.disabled {
        return Err(ApiError::conflict("user is disabled"));
    }
    validate_lease_window(request.starts_at, request.expires_at)?;
    let board_id = request.board_id.trim();
    if board_id.is_empty() {
        return Err(ApiError::bad_request("board_id must not be empty"));
    }
    let board = state
        .boards
        .read()
        .await
        .get(board_id)
        .cloned()
        .ok_or_else(|| ApiError::not_found(format!("board `{board_id}` not found")))?;
    if board.disabled {
        return Err(ApiError::conflict(format!(
            "board `{board_id}` is disabled"
        )));
    }
    ensure_lease_window_available(
        &state,
        board_id,
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

async fn delete_admin_lease(
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
    let boards = state.boards.read().await;
    let runtimes = state.board_runtimes.read().await;
    let board_types = summarize_board_types(&boards, &runtimes);
    let board_count_total = boards.len();
    let disabled_board_count = boards.values().filter(|board| board.disabled).count();
    let board_count_available = boards
        .values()
        .filter(|board| !board.disabled)
        .filter(|board| {
            runtimes
                .get(&board.id)
                .map(|runtime| runtime.lease_state == BoardLeaseState::Idle)
                .unwrap_or(false)
        })
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
    tftp_status.resolved_server_ip =
        resolve_server_network(&config)?.and_then(|network| network.server_ip);
    tftp_status.resolved_netmask =
        resolve_server_network(&config)?.and_then(|network| network.netmask);

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
    normalize_optional_string(&mut request.notes);
    normalize_tags(&mut request.tags);
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

fn normalize_serial_config(
    serial: Option<&mut crate::config::SerialConfig>,
) -> Result<(), ApiError> {
    let Some(serial) = serial else {
        return Ok(());
    };

    normalize_serial_key_value(&mut serial.key, "serial.key.value")?;
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
        }
        PowerManagementConfig::ZhongshengRelay(relay) => {
            normalize_serial_key_value(&mut relay.key, "power_management.key.value")?;
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
            normalize_dtb_name(value).map_err(|err| ApiError::bad_request(err.to_string()))
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

async fn delete_admin_session(
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

async fn get_tftp_config(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminTftpConfigResponse>, ApiError> {
    let config = state.config.read().await.clone();
    Ok(axum::Json(AdminTftpConfigResponse { tftp: config.tftp }))
}

async fn update_tftp_config(
    State(state): State<AppState>,
    axum::Json(tftp): axum::Json<TftpConfig>,
) -> Result<axum::Json<AdminTftpConfigResponse>, ApiError> {
    tokio::fs::create_dir_all(tftp.root_dir())
        .await
        .map_err(|err| ApiError::internal(err.to_string()))?;
    let new_manager = build_tftp_manager(&tftp);
    new_manager.start_if_needed().await.map_err(|err| {
        ApiError::service_unavailable(format!("failed to start TFTP provider: {err}"))
    })?;
    if matches!(tftp, TftpConfig::SystemTftpdHpa(_))
        && let Err(err) = new_manager.reconcile().await
    {
        return Err(ApiError::service_unavailable(format!(
            "failed to reconcile TFTP provider: {err}"
        )));
    }

    {
        let mut config = state.config.write().await;
        config.tftp = tftp.clone();
    }
    state.save_config().await?;
    *state.tftp_manager.write().await = new_manager;

    Ok(axum::Json(AdminTftpConfigResponse { tftp }))
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
    let config = state.config.read().await.clone();
    status.resolved_server_ip =
        resolve_server_network(&config)?.and_then(|network| network.server_ip);
    status.resolved_netmask = resolve_server_network(&config)?.and_then(|network| network.netmask);
    Ok(axum::Json(AdminTftpStatusResponse { status }))
}

async fn get_server_config(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminServerConfigResponse>, ApiError> {
    let config = state.config.read().await.clone();
    let site = state.storage.get_site_settings().await?;
    Ok(axum::Json(server_config_response(&config, site)))
}

async fn update_server_config(
    headers: HeaderMap,
    State(state): State<AppState>,
    axum::Json(request): axum::Json<UpdateServerConfigRequest>,
) -> Result<axum::Json<AdminServerConfigResponse>, ApiError> {
    if request.network.interface.trim().is_empty() {
        return Err(ApiError::bad_request("network.interface must not be empty"));
    }
    if request.upload_limits.session_file_max_mib == 0 {
        return Err(ApiError::bad_request(
            "upload_limits.session_file_max_mib must be greater than 0",
        ));
    }

    {
        let mut config = state.config.write().await;
        config.network = request.network;
        config.upload_limits = request.upload_limits;
    }
    state.save_config().await?;
    let current_user = current_user_from_headers(&state, &headers).await?;
    let site = state
        .storage
        .update_site_settings(
            site_settings_from_request(request.site)?,
            Some(current_user.id),
        )
        .await?;

    let config = state.config.read().await.clone();
    Ok(axum::Json(server_config_response(&config, site)))
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
    let boards = state.boards.read().await;
    let runtimes = state.board_runtimes.read().await;
    let result = summarize_board_types(&boards, &runtimes);
    Ok(axum::Json(result))
}

async fn create_session(
    State(state): State<AppState>,
    ConnectInfo(addr): ConnectInfo<SocketAddr>,
    axum::Json(request): axum::Json<CreateSessionRequest>,
) -> Result<(StatusCode, axum::Json<SessionCreatedResponse>), ApiError> {
    if request.board_type.trim().is_empty() {
        return Err(ApiError::bad_request("board_type must not be empty"));
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
    let response = http_boot_file_response(&config, &session_id, &relative_path, file)?;
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
    let kernel_response = http_boot_file_response(&config, &session_id, &remote_name, kernel_file)?;
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
        boot_architecture: optional_clean_header(headers, "X-Dtb-Architecture")?,
        compatible: optional_clean_header(headers, "X-Dtb-Compatible")?,
        description: optional_clean_header(headers, "X-Dtb-Description")?,
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
    let max_mib = state.config.read().await.upload_limits.session_file_max_mib;
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
    session_id: &str,
    relative_path: &str,
    file: TftpFileRef,
) -> Result<HttpBootFileResponse, ApiError> {
    let http_url = http_boot_url(config, session_id, relative_path)?;
    Ok(HttpBootFileResponse::from_file(file, http_url))
}

fn http_boot_url(
    config: &ServerConfig,
    session_id: &str,
    relative_path: &str,
) -> Result<String, ApiError> {
    let relative_path = normalize_relative_path(relative_path)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let base_url = http_boot_public_base_url(config)?;
    let base_url = base_url.trim_end_matches('/');
    Ok(format!(
        "{base_url}/boot/sessions/{session_id}/{relative_path}"
    ))
}

fn http_boot_public_base_url(config: &ServerConfig) -> Result<String, ApiError> {
    if let Some(public_base_url) = config.http_boot.public_base_url.as_deref()
        && !public_base_url.trim().is_empty()
    {
        return Ok(public_base_url.trim().to_string());
    }

    if let Some(network) = resolve_server_network(config)?
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
        if runtimes
            .get(&board.id)
            .map(|runtime| runtime.lease_state == BoardLeaseState::Idle)
            .unwrap_or(false)
        {
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
    site: SiteSettings,
) -> AdminServerConfigResponse {
    AdminServerConfigResponse {
        readonly: readonly_server_config(config),
        editable: AdminServerConfigEditable {
            network: config.network.clone(),
            upload_limits: config.upload_limits.clone(),
        },
        site: site_settings_response(site),
    }
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
    let mut settings = SiteSettings {
        site_name: request.site_name,
        site_subtitle: request.site_subtitle,
        logo_url: request.logo_url,
        favicon_url: request.favicon_url,
        announcement: request.announcement,
        maintenance_mode: request.maintenance_mode,
        self_service_enabled: request.self_service_enabled,
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

fn resolve_server_network(config: &ServerConfig) -> Result<Option<ResolvedNetwork>, ApiError> {
    let interface = if config.network.interface.trim().is_empty() {
        default_non_loopback_interface_name()
    } else {
        Some(config.network.interface.trim().to_string())
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

    let config = state.config.read().await.clone();
    let mut network = resolve_server_network(&config)?;
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
    normalize_dtb_name(value).map_err(|err| ApiError::bad_request(err.to_string()))
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
