use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Redirect, Response},
    routing::{delete, get, post, put},
};
use futures_util::future::join_all;
use serde_json::json;

use crate::{
    api::{
        error::ApiError,
        models::{
            ActionResponse, AdminBoardUpsertRequest, AdminOverviewResponse,
            AdminServerConfigEditable, AdminServerConfigReadonly, AdminServerConfigResponse,
            AdminSessionsResponse, AdminTftpConfigResponse, AdminTftpStatusResponse,
            BoardTypeSummary, BootProfileResponse, CreateSessionRequest, DtbFileResponse,
            FileResponse, NetworkInterfaceSummary, SerialPortSummary, SerialStatusResponse,
            SessionCreatedResponse, SessionDetailResponse, SessionDtbResponse, TftpSessionResponse,
            UpdateServerConfigRequest,
        },
    },
    board_pool::BoardAllocationStatus,
    config::{BoardConfig, BootConfig, PowerManagementConfig, ServerConfig, TftpConfig},
    dtb_store::normalize_dtb_name,
    power::{PowerAction, PowerActionError, execute_power_action_for_board},
    serial::{
        discovery::list_serial_ports as discover_serial_ports,
        network::{
            default_non_loopback_interface_name,
            list_network_interfaces as discover_network_interfaces,
        },
        ws::run_serial_ws,
    },
    state::AppState,
    tftp::{
        files::{TftpFileRef, normalize_relative_path},
        service::build_tftp_manager,
        status::resolve_interface_ipv4,
    },
    web::{serve_admin_asset, serve_admin_history, serve_admin_index},
};

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route(
            "/",
            get(|| async { Redirect::temporary("/admin/overview") }),
        )
        .route("/admin", get(serve_admin_index))
        .route("/admin/", get(serve_admin_index))
        .route("/admin/assets/{*path}", get(serve_admin_asset))
        .route("/admin/{*path}", get(serve_admin_history))
        .route("/api/v1/admin/overview", get(get_admin_overview))
        .route("/api/v1/admin/boards", get(list_boards).post(create_board))
        .route("/api/v1/admin/dtbs", get(list_dtbs).post(create_dtb))
        .route("/api/v1/admin/serial-ports", get(list_serial_ports))
        .route(
            "/api/v1/admin/network-interfaces",
            get(list_network_interfaces),
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
        .with_state(state)
}

async fn get_admin_overview(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminOverviewResponse>, ApiError> {
    let boards = state.boards.read().await;
    let sessions = session_snapshots(&state).await;
    let board_types = summarize_board_types(&boards, &sessions);
    let leased = leased_board_ids(&sessions);
    let board_count_total = boards.len();
    let disabled_board_count = boards.values().filter(|board| board.disabled).count();
    let board_count_available = boards
        .values()
        .filter(|board| !board.disabled)
        .filter(|board| !leased.contains(board.id.as_str()))
        .count();
    drop(sessions);
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
        active_session_count: state.sessions.read().await.len(),
        board_types,
        tftp_status,
        server: readonly_server_config(&config),
    }))
}

async fn list_boards(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<BoardConfig>>, ApiError> {
    Ok(axum::Json(
        state
            .boards
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>(),
    ))
}

async fn list_dtbs(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<DtbFileResponse>>, ApiError> {
    let files = state.dtb_store.list_all().await?;
    Ok(axum::Json(
        files.into_iter().map(DtbFileResponse::from_dtb).collect(),
    ))
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
    Ok(axum::Json(board))
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
    Ok(axum::Json(DtbFileResponse::from_dtb(file)))
}

async fn create_dtb(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, axum::Json<DtbFileResponse>), ApiError> {
    let dtb_name = dtb_name_header(&headers, "X-Dtb-Name")?;
    if body.is_empty() {
        return Err(ApiError::bad_request("DTB upload body must not be empty"));
    }
    if state.dtb_store.get(&dtb_name).await?.is_some() {
        return Err(ApiError::conflict(format!(
            "DTB `{dtb_name}` already exists"
        )));
    }

    let file = state.dtb_store.write(&dtb_name, &body).await?;
    Ok((
        StatusCode::CREATED,
        axum::Json(DtbFileResponse::from_dtb(file)),
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

    state.board_store.write_board(&board).await?;
    state
        .boards
        .write()
        .await
        .insert(board.id.clone(), board.clone());
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

    if board.id != board_id {
        let sessions = state.sessions.read().await;
        if sessions
            .values()
            .any(|session| session.board().id == board_id)
        {
            return Err(ApiError::conflict(format!(
                "board `{board_id}` is leased by an active session"
            )));
        }
    }

    state.board_store.write_board(&board).await?;
    if board.id != board_id {
        state.board_store.delete_board(&board_id).await?;
    }

    {
        let mut boards = state.boards.write().await;
        boards.remove(&board_id);
        boards.insert(board.id.clone(), board.clone());
    }

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
    normalize_boot_config(&mut request.boot);

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

    let trimmed = serial.port.trim();
    if trimmed.is_empty() {
        return Err(ApiError::bad_request(
            "serial.port must not be empty when serial is configured",
        ));
    }
    if trimmed.len() != serial.port.len() {
        serial.port = trimmed.to_string();
    }
    if serial.baud_rate == 0 {
        return Err(ApiError::bad_request(
            "serial.baud_rate must be > 0 when serial is configured",
        ));
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
            normalize_required_string(&mut relay.serial_port, "power_management.serial_port")?;
        }
    }

    Ok(())
}

fn normalize_boot_config(boot: &mut BootConfig) {
    match boot {
        BootConfig::Uboot(profile) => {
            normalize_optional_string(&mut profile.dtb_name);
        }
        BootConfig::Pxe(profile) => {
            normalize_optional_string(&mut profile.notes);
        }
    }
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
    headers: HeaderMap,
    body: Bytes,
) -> Result<axum::Json<DtbFileResponse>, ApiError> {
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

    if requested_name.as_deref() == Some(current_name.as_str()) && body.is_empty() {
        let file = state
            .dtb_store
            .get(&current_name)
            .await?
            .ok_or_else(|| ApiError::not_found(format!("DTB `{current_name}` not found")))?;
        return Ok(axum::Json(DtbFileResponse::from_dtb(file)));
    }

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
        effective_name = new_name.to_string();
    }

    if !body.is_empty() {
        state.dtb_store.write(&effective_name, &body).await?;
    } else if requested_name.is_none() {
        return Err(ApiError::bad_request(
            "DTB update requires a new name or replacement file body",
        ));
    }

    let file = state
        .dtb_store
        .get(&effective_name)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("DTB `{effective_name}` not found")))?;
    Ok(axum::Json(DtbFileResponse::from_dtb(file)))
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
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_board(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    {
        let sessions = state.sessions.read().await;
        if sessions
            .values()
            .any(|session| session.board().id == board_id)
        {
            return Err(ApiError::conflict(format!(
                "board `{board_id}` is leased by an active session"
            )));
        }
    }

    {
        let mut boards = state.boards.write().await;
        if boards.remove(&board_id).is_none() {
            return Err(ApiError::not_found(format!("board `{board_id}` not found")));
        }
    }

    state.board_store.delete_board(&board_id).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_admin_sessions(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminSessionsResponse>, ApiError> {
    Ok(axum::Json(AdminSessionsResponse {
        sessions: session_snapshots(&state).await,
    }))
}

async fn delete_admin_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let removed = state.remove_session(&session_id).await?;
    if removed.is_none() {
        return Err(ApiError::not_found(format!(
            "session `{session_id}` not found"
        )));
    }
    Ok(StatusCode::NO_CONTENT)
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
    Ok(axum::Json(server_config_response(&config)))
}

async fn update_server_config(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<UpdateServerConfigRequest>,
) -> Result<axum::Json<AdminServerConfigResponse>, ApiError> {
    if request.network.interface.trim().is_empty() {
        return Err(ApiError::bad_request("network.interface must not be empty"));
    }

    {
        let mut config = state.config.write().await;
        config.network = request.network;
    }
    state.save_config().await?;

    let config = state.config.read().await.clone();
    Ok(axum::Json(server_config_response(&config)))
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
    let sessions = session_snapshots(&state).await;
    let result = summarize_board_types(&boards, &sessions);
    Ok(axum::Json(result))
}

async fn create_session(
    State(state): State<AppState>,
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
        .touch_session(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("session `{session_id}` not found")))?;
    Ok(axum::Json(json!({
        "session_id": session.id,
        "lease_expires_at": session.expires_at
    })))
}

async fn delete_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let removed = state.remove_session(&session_id).await?;
    if removed.is_none() {
        return Err(ApiError::not_found(format!(
            "session `{session_id}` not found"
        )));
    }
    Ok(StatusCode::NO_CONTENT)
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
        boot: board.boot,
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
        SerialStatusResponse {
            available: true,
            connected,
            port: Some(serial.port),
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
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, axum::Json<FileResponse>), ApiError> {
    let _session = get_session_or_404(&state, &session_id).await?;
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let relative_path = headers
        .get("X-File-Path")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("missing X-File-Path header"))?;
    let relative_path = parse_relative_path(relative_path)?;

    if !state.config.read().await.tftp.enabled() {
        return Err(ApiError::conflict("TFTP provider is disabled"));
    }

    let manager = state.tftp_manager.read().await.clone();
    let file = manager
        .put_session_file(&session_id, &relative_path, &body)
        .await
        .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?;
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
    get_session_or_404(&state, &session_id).await?;
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
    let board = state
        .session_board(session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;

    let action = if power_on {
        PowerAction::On
    } else {
        PowerAction::Off
    };
    let message = execute_power_action_for_board(&board, action)
        .await
        .map_err(|err| match err {
            PowerActionError::NotConfigured | PowerActionError::InvalidConfig(_) => {
                ApiError::bad_request(err.to_string())
            }
            PowerActionError::Execution(err) => ApiError::from(err),
        })?;

    Ok(axum::Json(ActionResponse { ok: true, message }))
}

fn parse_relative_path(raw: &str) -> Result<String, ApiError> {
    normalize_relative_path(raw).map_err(|err| ApiError::bad_request(err.to_string()))
}

fn leased_board_ids(sessions: &[crate::session::Session]) -> BTreeSet<&str> {
    sessions
        .iter()
        .map(|session| session.board_id.as_str())
        .collect::<BTreeSet<_>>()
}

fn summarize_board_types(
    boards: &BTreeMap<String, BoardConfig>,
    sessions: &[crate::session::Session],
) -> Vec<BoardTypeSummary> {
    let leased = leased_board_ids(sessions);
    let mut aggregate = BTreeMap::<String, (BTreeSet<String>, usize, usize)>::new();
    for board in boards.values().filter(|board| !board.disabled) {
        let entry = aggregate
            .entry(board.board_type.clone())
            .or_insert_with(|| (BTreeSet::new(), 0, 0));
        for tag in &board.tags {
            entry.0.insert(tag.clone());
        }
        entry.1 += 1;
        if !leased.contains(board.id.as_str()) {
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
    }
}

fn server_config_response(config: &crate::config::ServerConfig) -> AdminServerConfigResponse {
    AdminServerConfigResponse {
        readonly: readonly_server_config(config),
        editable: AdminServerConfigEditable {
            network: config.network.clone(),
        },
    }
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

    let config = state.config.read().await.clone();
    resolve_server_network(&config)
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
        state.board_store.write_board(board).await?;
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
    use std::future;
    use std::sync::Arc;

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use serde_json::json;
    #[cfg(unix)]
    use serialport::{SerialPort, TTYPort};
    use tempfile::tempdir;
    use tokio::sync::{mpsc, oneshot};
    #[cfg(unix)]
    use tokio_modbus::{
        ExceptionCode, Request as ModbusRequest, Response as ModbusResponse, SlaveRequest,
        server::{Service, rtu::Server},
    };
    use tower::util::ServiceExt;

    use super::{build_router, resolve_server_network};
    use crate::{
        build_app_state,
        config::{
            BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement,
            PowerManagementConfig, ServerConfig, TftpConfig, ZhongshengRelayPowerManagement,
        },
        tftp::service::{TftpManager, build_tftp_manager},
        web::first_asset_path,
    };

    async fn test_router() -> Router {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        std::mem::forget(temp);
        let config_path = root.join(".ostool-server.toml");
        let mut config = ServerConfig::default();
        config.listen_addr = "127.0.0.1:0".parse().unwrap();
        config.network.interface = "lo".into();
        config.data_dir = root.join("data");
        config.board_dir = root.join("boards");
        config.dtb_dir = root.join("dtbs");
        config.tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(root.join("tftp")));
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();
        state.ensure_data_dirs().await.unwrap();
        build_router(state)
    }

    async fn create_board(app: &Router, request: serde_json::Value) -> StatusCode {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    async fn create_session(app: &Router, board_type: &str) -> String {
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "board_type": board_type,
                            "required_tags": [],
                            "client_name": "test",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        value["session_id"].as_str().unwrap().to_string()
    }

    async fn upload_dtb(app: &Router, name: &str, body: &'static str) -> StatusCode {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/dtbs")
                    .header("X-Dtb-Name", name)
                    .body(Body::from(body))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    fn sample_board(board_id: &str) -> BoardConfig {
        BoardConfig {
            id: board_id.into(),
            board_type: "rk3568".into(),
            tags: vec!["lab".into(), "usb".into()],
            serial: Some(crate::config::SerialConfig {
                port: "/dev/ttyUSB0".into(),
                baud_rate: 115_200,
            }),
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "echo on".into(),
                power_off_cmd: "echo off".into(),
            }),
            boot: BootConfig::Uboot(crate::config::UbootProfile {
                use_tftp: true,
                ..Default::default()
            }),
            notes: Some("rack-a".into()),
            disabled: false,
        }
    }

    #[cfg(unix)]
    #[derive(Clone)]
    struct RecordingRelayService {
        requests: mpsc::UnboundedSender<(u8, u16, bool)>,
    }

    #[cfg(unix)]
    impl Service for RecordingRelayService {
        type Request = SlaveRequest<'static>;
        type Response = ModbusResponse;
        type Exception = ExceptionCode;
        type Future = future::Ready<std::result::Result<Self::Response, Self::Exception>>;

        fn call(&self, req: Self::Request) -> Self::Future {
            match req.request {
                ModbusRequest::WriteSingleCoil(address, coil) => {
                    self.requests.send((req.slave, address, coil)).unwrap();
                    future::ready(Ok(ModbusResponse::WriteSingleCoil(address, coil)))
                }
                _ => future::ready(Err(ExceptionCode::IllegalFunction)),
            }
        }
    }

    #[cfg(unix)]
    fn spawn_relay_test_server() -> (
        String,
        TTYPort,
        tokio::task::JoinHandle<std::io::Result<tokio_modbus::server::Terminated>>,
        mpsc::UnboundedReceiver<(u8, u16, bool)>,
        oneshot::Sender<()>,
    ) {
        let (master, mut slave) = TTYPort::pair().unwrap();
        slave.set_exclusive(false).unwrap();
        let slave_path = slave.name().unwrap();

        let server_stream = tokio_serial::SerialStream::try_from(master).unwrap();
        let (request_tx, request_rx) = mpsc::unbounded_channel();
        let (stop_tx, stop_rx) = oneshot::channel();
        let task = tokio::spawn(async move {
            Server::new(server_stream)
                .serve_until(
                    RecordingRelayService {
                        requests: request_tx,
                    },
                    async move {
                        let _ = stop_rx.await;
                    },
                )
                .await
        });

        (slave_path, slave, task, request_rx, stop_tx)
    }

    #[tokio::test]
    async fn admin_route_serves_embedded_index() {
        let app: Router = test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html"
        );

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("ostool-server 管理台"));
    }

    #[tokio::test]
    async fn admin_asset_route_serves_embedded_asset() {
        let asset_path = first_asset_path().expect("missing built frontend asset");
        let app: Router = test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri(format!("/admin/{asset_path}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.headers().contains_key(header::CONTENT_TYPE));
    }

    #[tokio::test]
    async fn admin_history_fallback_serves_index() {
        let app: Router = test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/admin/boards/demo-board")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("id=\"app\""));
    }

    #[tokio::test]
    async fn server_config_endpoint_updates_only_network() {
        let app: Router = test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/server-config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"network":{"interface":"lo"}}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["editable"]["network"]["interface"], "lo");
        assert!(value["readonly"]["listen_addr"].is_string());
    }

    #[test]
    fn resolve_server_network_uses_configured_interface() {
        let mut config = ServerConfig::default();
        config.network.interface = "lo".into();

        let resolved = resolve_server_network(&config).unwrap().unwrap();
        assert_eq!(resolved.interface.as_deref(), Some("lo"));
    }

    #[test]
    fn board_config_new_uboot_profile_supports_use_tftp() {
        let board = BoardConfig {
            id: "demo".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "echo on".into(),
                power_off_cmd: "echo off".into(),
            }),
            boot: BootConfig::Uboot(crate::config::UbootProfile {
                use_tftp: true,
                ..Default::default()
            }),
            notes: None,
            disabled: false,
        };

        match board.boot {
            BootConfig::Uboot(profile) => assert!(profile.use_tftp),
            BootConfig::Pxe(_) => panic!("expected uboot"),
        }
    }

    #[tokio::test]
    async fn get_serial_ports_endpoint_returns_ok() {
        let app = test_router().await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/serial-ports")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let _: Vec<crate::api::models::SerialPortSummary> = serde_json::from_slice(&body).unwrap();
    }

    #[tokio::test]
    async fn get_board_returns_board_config() {
        let app = test_router().await;
        let board = sample_board("demo-board");
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/boards/demo-board")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let returned: BoardConfig = serde_json::from_slice(&body).unwrap();
        assert_eq!(returned.id, "demo-board");
        assert_eq!(returned.board_type, "rk3568");
    }

    #[tokio::test]
    async fn create_board_persists_request_payload_and_returns_board_config() {
        let app = test_router().await;
        let request = serde_json::to_value(sample_board("create-me")).unwrap();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let board: BoardConfig = serde_json::from_slice(&body).unwrap();
        assert_eq!(board.id, "create-me");
        assert_eq!(board.serial.unwrap().port, "/dev/ttyUSB0");
    }

    #[tokio::test]
    async fn create_board_assigns_next_available_id_when_id_is_blank() {
        let app = test_router().await;
        let board = sample_board("demo-board");
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );
        assert_eq!(
            create_board(
                &app,
                json!({
                    "id": "rk3568-1",
                    "board_type": "rk3568",
                    "tags": [],
                    "serial": null,
                    "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
                    "boot": { "kind": "pxe", "notes": null },
                    "notes": null,
                    "disabled": false
                }),
            )
            .await,
            StatusCode::CREATED
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "id": "",
                            "board_type": "rk3568",
                            "tags": [" lab "],
                            "serial": null,
                            "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
                            "boot": { "kind": "pxe", "notes": null },
                            "notes": " ",
                            "disabled": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let board: BoardConfig = serde_json::from_slice(&body).unwrap();
        assert_eq!(board.id, "rk3568-2");
        assert_eq!(board.tags, vec!["lab"]);
        assert!(board.notes.is_none());
    }

    #[tokio::test]
    async fn update_board_keeps_original_id_when_request_id_is_blank() {
        let app = test_router().await;
        let board = sample_board("demo-board");
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/boards/demo-board")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "id": " ",
                            "board_type": "rk3568",
                            "tags": ["usb"],
                            "serial": null,
                            "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
                            "boot": { "kind": "uboot", "use_tftp": false },
                            "notes": "updated",
                            "disabled": true
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let updated: BoardConfig = serde_json::from_slice(&body).unwrap();
        assert_eq!(updated.id, "demo-board");
        assert!(updated.serial.is_none());
        assert!(updated.disabled);
    }

    #[tokio::test]
    async fn update_board_allows_explicit_rename() {
        let app = test_router().await;
        let board = sample_board("demo-board");
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/boards/demo-board")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "id": "demo-board-renamed",
                            "board_type": "rk3568",
                            "tags": ["lab"],
                            "serial": null,
                            "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
                            "boot": { "kind": "pxe", "notes": "pxe" },
                            "notes": null,
                            "disabled": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let updated: BoardConfig = serde_json::from_slice(&body).unwrap();
        assert_eq!(updated.id, "demo-board-renamed");

        let boards_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/boards")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let boards_body = to_bytes(boards_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let boards: Vec<BoardConfig> = serde_json::from_slice(&boards_body).unwrap();
        assert_eq!(boards[0].id, "demo-board-renamed");
    }

    #[tokio::test]
    async fn power_actions_execute_custom_power_management_commands() {
        let app = test_router().await;
        let mut board = sample_board("power-board");
        board.power_management = PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd: "printf power-on >/dev/null".into(),
            power_off_cmd: "printf power-off >/dev/null".into(),
        });
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );

        let session = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "board_type": "rk3568",
                            "required_tags": [],
                            "client_name": "test",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let session_body = to_bytes(session.into_body(), usize::MAX).await.unwrap();
        let session_value: serde_json::Value = serde_json::from_slice(&session_body).unwrap();
        let session_id = session_value["session_id"].as_str().unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/sessions/{session_id}/board/power-on"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["message"], "executed `printf power-on >/dev/null`");
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn power_actions_execute_zhongsheng_relay_via_modbus_rtu() {
        let app = test_router().await;
        let (relay_port, _relay_handle, server, mut requests, stop_tx) = spawn_relay_test_server();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let mut board = sample_board("relay-board");
        board.power_management =
            PowerManagementConfig::ZhongshengRelay(ZhongshengRelayPowerManagement {
                serial_port: relay_port.clone(),
            });
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );

        let session = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "board_type": "rk3568",
                            "required_tags": [],
                            "client_name": "test",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        let session_body = to_bytes(session.into_body(), usize::MAX).await.unwrap();
        let session_value: serde_json::Value = serde_json::from_slice(&session_body).unwrap();
        let session_id = session_value["session_id"].as_str().unwrap();

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(format!("/api/v1/sessions/{session_id}/board/power-off"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(status, StatusCode::OK, "{}", String::from_utf8_lossy(&body));
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(
            value["message"],
            format!("executed Zhongsheng relay power-off via {relay_port}")
        );

        let request = tokio::time::timeout(std::time::Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request, (1, 0, false));

        let _ = stop_tx.send(());
        let _ = server.await.unwrap();
    }

    #[tokio::test]
    async fn create_board_rejects_duplicate_ids_and_missing_required_fields() {
        let app = test_router().await;
        let board = sample_board("demo-board");
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );

        let duplicate_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&board).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(duplicate_response.status(), StatusCode::CONFLICT);

        let invalid_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "id": null,
                            "board_type": " ",
                            "tags": [],
                            "serial": null,
                            "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
                            "boot": { "kind": "pxe", "notes": null },
                            "notes": null,
                            "disabled": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(invalid_response.status(), StatusCode::BAD_REQUEST);

        let missing_power_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "id": null,
                            "board_type": "rk3568",
                            "tags": [],
                            "serial": null,
                            "power_management": null,
                            "boot": { "kind": "pxe", "notes": null },
                            "notes": null,
                            "disabled": false
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            missing_power_response.status(),
            StatusCode::UNPROCESSABLE_ENTITY
        );
    }

    #[tokio::test]
    async fn admin_dtb_endpoints_support_create_rename_replace_and_delete() {
        let app = test_router().await;

        assert_eq!(
            upload_dtb(&app, "board.dtb", "dtb-v1").await,
            StatusCode::CREATED
        );

        let rename_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/dtbs/board.dtb")
                    .header("X-Dtb-Name", "board-v2.dtb")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rename_response.status(), StatusCode::OK);

        let replace_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/dtbs/board-v2.dtb")
                    .body(Body::from("dtb-v2"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replace_response.status(), StatusCode::OK);
        let replace_body = to_bytes(replace_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let file: crate::api::models::DtbFileResponse =
            serde_json::from_slice(&replace_body).unwrap();
        assert_eq!(file.name, "board-v2.dtb");
        assert_eq!(file.size, 6);
        assert_eq!(file.relative_tftp_path_template, "boot/dtb/board-v2.dtb");

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/admin/dtbs/board-v2.dtb")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);
    }

    #[tokio::test]
    async fn renaming_dtb_updates_board_references_and_referenced_dtb_cannot_be_deleted() {
        let app = test_router().await;
        assert_eq!(
            upload_dtb(&app, "board.dtb", "dtb").await,
            StatusCode::CREATED
        );
        assert_eq!(
            create_board(
                &app,
                json!({
                    "id": "rk3568-dtb",
                    "board_type": "rk3568",
                    "tags": [],
                    "serial": null,
                    "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
                    "boot": { "kind": "uboot", "use_tftp": true, "dtb_name": "board.dtb" },
                    "notes": null,
                    "disabled": false
                }),
            )
            .await,
            StatusCode::CREATED
        );

        let delete_referenced = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/admin/dtbs/board.dtb")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_referenced.status(), StatusCode::CONFLICT);

        let rename_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/dtbs/board.dtb")
                    .header("X-Dtb-Name", "board-renamed.dtb")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(rename_response.status(), StatusCode::OK);

        let board_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/boards/rk3568-dtb")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let board_body = to_bytes(board_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let board: BoardConfig = serde_json::from_slice(&board_body).unwrap();
        match board.boot {
            BootConfig::Uboot(profile) => {
                assert_eq!(profile.dtb_name.as_deref(), Some("board-renamed.dtb"))
            }
            BootConfig::Pxe(_) => panic!("expected uboot"),
        }
    }

    #[tokio::test]
    async fn session_dtb_endpoint_stages_preset_file_and_supports_download() {
        let app = test_router().await;
        assert_eq!(
            upload_dtb(&app, "board.dtb", "dtb-bytes").await,
            StatusCode::CREATED
        );
        assert_eq!(
            create_board(
                &app,
                json!({
                    "id": "rk3568-dtb",
                    "board_type": "rk3568",
                    "tags": [],
                    "serial": { "port": "/dev/ttyUSB0", "baud_rate": 115200 },
                    "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
                    "boot": { "kind": "uboot", "use_tftp": true, "dtb_name": "board.dtb" },
                    "notes": null,
                    "disabled": false
                }),
            )
            .await,
            StatusCode::CREATED
        );
        let session_id = create_session(&app, "rk3568").await;

        let dtb_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{session_id}/dtb"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(dtb_response.status(), StatusCode::OK);
        let dtb_body = to_bytes(dtb_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let dtb: crate::api::models::SessionDtbResponse =
            serde_json::from_slice(&dtb_body).unwrap();
        assert_eq!(dtb.dtb_name.as_deref(), Some("board.dtb"));
        assert_eq!(
            dtb.relative_path.as_deref(),
            Some(format!("ostool/sessions/{session_id}/boot/dtb/board.dtb").as_str())
        );
        assert_eq!(dtb.session_file_path.as_deref(), Some("boot/dtb/board.dtb"));

        let download_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{session_id}/dtb/download"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(download_response.status(), StatusCode::OK);
        let download_body = to_bytes(download_response.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(download_body.as_ref(), b"dtb-bytes");
    }

    #[tokio::test]
    async fn board_types_endpoint_returns_aggregated_counts() {
        let app = test_router().await;
        let board_a = json!({
            "id": "rk3568-01",
            "board_type": "rk3568",
            "tags": ["lab-a", "usbboot"],
            "serial": { "port": "/dev/ttyUSB0", "baud_rate": 115200 },
            "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
            "boot": { "kind": "uboot", "use_tftp": false },
            "notes": null,
            "disabled": false
        });
        let board_b = json!({
            "id": "rk3568-02",
            "board_type": "rk3568",
            "tags": ["lab-b"],
            "serial": { "port": "/dev/ttyUSB1", "baud_rate": 115200 },
            "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
            "boot": { "kind": "uboot", "use_tftp": false },
            "notes": null,
            "disabled": false
        });

        assert_eq!(create_board(&app, board_a).await, StatusCode::CREATED);
        assert_eq!(create_board(&app, board_b).await, StatusCode::CREATED);

        let session_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "board_type": "rk3568",
                            "required_tags": [],
                            "client_name": "test",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(session_response.status(), StatusCode::CREATED);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/board-types")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value[0]["board_type"], "rk3568");
        assert_eq!(value[0]["total"], 2);
        assert_eq!(value[0]["available"], 1);
        assert_eq!(value[0]["tags"], json!(["lab-a", "lab-b", "usbboot"]));
    }

    #[tokio::test]
    async fn session_file_endpoints_support_nested_paths() {
        let app = test_router().await;
        assert_eq!(
            create_board(
                &app,
                serde_json::to_value(sample_board("nested-files")).unwrap()
            )
            .await,
            StatusCode::CREATED
        );
        let session_id = create_session(&app, "rk3568").await;

        let upload = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{session_id}/files"))
                    .header("X-File-Path", "boot/Image")
                    .body(Body::from("kernel-image"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::CREATED);
        let upload_body = to_bytes(upload.into_body(), usize::MAX).await.unwrap();
        let uploaded: serde_json::Value = serde_json::from_slice(&upload_body).unwrap();
        assert_eq!(uploaded["filename"], "Image");
        assert_eq!(
            uploaded["relative_path"],
            format!("ostool/sessions/{session_id}/boot/Image")
        );
        assert_eq!(
            uploaded["tftp_url"],
            format!("tftp://127.0.0.1/ostool/sessions/{session_id}/boot/Image")
        );

        let get_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{session_id}/files/boot/Image"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(get_response.status(), StatusCode::OK);

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/sessions/{session_id}/files/boot/Image"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::NO_CONTENT);

        let missing_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{session_id}/files/boot/Image"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn session_file_list_supports_multiple_paths_and_overwrite() {
        let app = test_router().await;
        assert_eq!(
            create_board(
                &app,
                serde_json::to_value(sample_board("list-files")).unwrap()
            )
            .await,
            StatusCode::CREATED
        );
        let session_id = create_session(&app, "rk3568").await;

        for (path, body) in [
            ("boot/Image", "v1"),
            ("boot/dtb/board.dtb", "dtb"),
            ("boot/Image", "updated-kernel"),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("PUT")
                        .uri(format!("/api/v1/sessions/{session_id}/files"))
                        .header("X-File-Path", path)
                        .body(Body::from(body))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
        }

        let list_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{session_id}/files"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(list_response.status(), StatusCode::OK);
        let list_body = to_bytes(list_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let files: serde_json::Value = serde_json::from_slice(&list_body).unwrap();
        assert_eq!(files.as_array().unwrap().len(), 2);
        assert_eq!(
            files[0]["relative_path"],
            format!("ostool/sessions/{session_id}/boot/Image")
        );
        assert_eq!(files[0]["size"], "updated-kernel".len());
        assert_eq!(
            files[1]["relative_path"],
            format!("ostool/sessions/{session_id}/boot/dtb/board.dtb")
        );
    }

    #[tokio::test]
    async fn legacy_slot_file_route_is_removed() {
        let app = test_router().await;
        assert_eq!(
            create_board(
                &app,
                serde_json::to_value(sample_board("legacy-files")).unwrap()
            )
            .await,
            StatusCode::CREATED
        );
        let session_id = create_session(&app, "rk3568").await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{session_id}/files/kernel"))
                    .body(Body::from("legacy"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn create_session_returns_created_when_board_is_available() {
        let app = test_router().await;
        let board = json!({
            "id": "demo-01",
            "board_type": "demo",
            "tags": [],
            "serial": { "port": "/dev/ttyUSB0", "baud_rate": 115200 },
            "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
            "boot": { "kind": "uboot", "use_tftp": false },
            "notes": null,
            "disabled": false
        });
        assert_eq!(create_board(&app, board).await, StatusCode::CREATED);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "board_type": "demo",
                            "required_tags": [],
                            "client_name": "test",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["board_id"], "demo-01");
        assert_eq!(value["serial_available"], true);
    }

    #[tokio::test]
    async fn create_session_returns_conflict_without_waiting_when_pool_is_busy() {
        let app = test_router().await;
        let board = json!({
            "id": "demo-01",
            "board_type": "demo",
            "tags": [],
            "serial": { "port": "/dev/ttyUSB0", "baud_rate": 115200 },
            "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
            "boot": { "kind": "uboot", "use_tftp": false },
            "notes": null,
            "disabled": false
        });
        assert_eq!(create_board(&app, board).await, StatusCode::CREATED);

        let first = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "board_type": "demo",
                            "required_tags": [],
                            "client_name": "first",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::CREATED);

        let second = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "board_type": "demo",
                            "required_tags": [],
                            "client_name": "second",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(second.status(), StatusCode::CONFLICT);
        let body = to_bytes(second.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], "conflict");
        assert_eq!(value["message"], "no available board for type `demo`");
    }

    #[tokio::test]
    async fn create_session_returns_not_found_when_board_type_is_missing() {
        let app = test_router().await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "board_type": "missing-demo",
                            "required_tags": [],
                            "client_name": "test",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], "not_found");
        assert_eq!(value["message"], "board type `missing-demo` not found");
    }

    #[tokio::test]
    async fn create_session_rejects_empty_board_type() {
        let app = test_router().await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "board_type": "",
                            "required_tags": [],
                            "client_name": "test",
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["code"], "bad_request");
        assert_eq!(value["message"], "board_type must not be empty");
    }
}
