use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{Redirect, Response},
    routing::{delete, get, post, put},
};
use serde_json::json;

use crate::{
    api::{
        error::ApiError,
        models::{
            ActionResponse, AdminOverviewResponse, AdminServerConfigEditable,
            AdminServerConfigReadonly, AdminServerConfigResponse, AdminSessionsResponse,
            AdminTftpConfigResponse, AdminTftpStatusResponse, BoardTypeSummary,
            BootProfileResponse, CreateSessionRequest, FileResponse, NetworkInterfaceSummary,
            SerialPortSummary, SerialStatusResponse, SessionCreatedResponse, SessionDetailResponse,
            TftpSessionResponse, UpdateServerConfigRequest,
        },
    },
    config::{BoardConfig, BootConfig, ServerConfig, TftpConfig},
    process::run_shell_command,
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
        files::{FileSlot, TftpFileRef},
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
        .route("/api/v1/admin/serial-ports", get(list_serial_ports))
        .route(
            "/api/v1/admin/network-interfaces",
            get(list_network_interfaces),
        )
        .route(
            "/api/v1/admin/boards/{board_id}",
            get(get_board).put(update_board).delete(delete_board),
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
        .route(
            "/api/v1/sessions/{session_id}/serial",
            get(get_serial_status),
        )
        .route("/api/v1/sessions/{session_id}/serial/ws", get(serial_ws))
        .route(
            "/api/v1/sessions/{session_id}/board/reset",
            post(reset_board),
        )
        .route(
            "/api/v1/sessions/{session_id}/board/power-off",
            post(power_off_board),
        )
        .route(
            "/api/v1/sessions/{session_id}/files",
            get(list_session_files),
        )
        .route(
            "/api/v1/sessions/{session_id}/files/{slot}",
            put(put_session_file)
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
    let sessions = state.sessions.read().await;
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
        resolve_server_tftp_network(&config)?.and_then(|network| network.server_ip);
    tftp_status.resolved_netmask =
        resolve_server_tftp_network(&config)?.and_then(|network| network.netmask);

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

async fn list_serial_ports() -> Result<axum::Json<Vec<SerialPortSummary>>, ApiError> {
    Ok(axum::Json(discover_serial_ports().map_err(|err| {
        ApiError::service_unavailable(format!("failed to enumerate serial ports: {err:#}"))
    })?))
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

async fn create_board(
    State(state): State<AppState>,
    axum::Json(board): axum::Json<BoardConfig>,
) -> Result<(StatusCode, axum::Json<BoardConfig>), ApiError> {
    validate_board_id(&board.id)?;

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
    axum::Json(board): axum::Json<BoardConfig>,
) -> Result<axum::Json<BoardConfig>, ApiError> {
    validate_board_id(&board_id)?;
    if board.id != board_id {
        return Err(ApiError::bad_request(
            "board id in path and body must match",
        ));
    }

    {
        let boards = state.boards.read().await;
        if !boards.contains_key(&board_id) {
            return Err(ApiError::not_found(format!("board `{board_id}` not found")));
        }
    }

    state.board_store.write_board(&board).await?;
    state
        .boards
        .write()
        .await
        .insert(board.id.clone(), board.clone());
    Ok(axum::Json(board))
}

async fn delete_board(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    {
        let sessions = state.sessions.read().await;
        if sessions
            .values()
            .any(|session| session.board_id == board_id)
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
        sessions: state.sessions.read().await.values().cloned().collect(),
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
        resolve_server_tftp_network(&config)?.and_then(|network| network.server_ip);
    status.resolved_netmask =
        resolve_server_tftp_network(&config)?.and_then(|network| network.netmask);
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
    if request.lease.default_ttl_secs == 0 {
        return Err(ApiError::bad_request("lease.default_ttl_secs must be > 0"));
    }
    if request.lease.max_ttl_secs < request.lease.default_ttl_secs {
        return Err(ApiError::bad_request(
            "lease.max_ttl_secs must be >= lease.default_ttl_secs",
        ));
    }
    if request.lease.gc_interval_secs == 0 {
        return Err(ApiError::bad_request("lease.gc_interval_secs must be > 0"));
    }
    if request.tftp_network.interface.trim().is_empty() {
        return Err(ApiError::bad_request(
            "tftp_network.interface must not be empty",
        ));
    }

    {
        let mut config = state.config.write().await;
        config.lease = request.lease;
        config.tftp_network = request.tftp_network;
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
    let sessions = state.sessions.read().await;
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
        .ok_or_else(|| {
            ApiError::conflict(format!(
                "no available board for type `{}`",
                request.board_type
            ))
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
    let connected = state
        .active_serial_sessions
        .read()
        .await
        .contains(&session_id);

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
    let network = resolved_board_tftp_network(&state, &board).await?;
    Ok(axum::Json(BootProfileResponse {
        boot: board.boot,
        server_ip: network.as_ref().and_then(|item| item.server_ip.clone()),
        netmask: network.as_ref().and_then(|item| item.netmask.clone()),
        interface: network.as_ref().and_then(|item| item.interface.clone()),
    }))
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
        .active_serial_sessions
        .read()
        .await
        .contains(&session_id);
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
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let Some(serial) = board.serial.clone() else {
        return Err(ApiError::conflict("board has no serial configuration"));
    };

    {
        let mut active = state.active_serial_sessions.write().await;
        if !active.insert(session_id.clone()) {
            return Err(ApiError::conflict("serial websocket already connected"));
        }
    }

    Ok(ws.on_upgrade(move |socket| run_serial_ws(socket, state, session_id, serial)))
}

async fn reset_board(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<ActionResponse>, ApiError> {
    run_board_command(&state, &session_id, true).await
}

async fn power_off_board(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<ActionResponse>, ApiError> {
    run_board_command(&state, &session_id, false).await
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
    Path((session_id, slot)): Path<(String, String)>,
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<(StatusCode, axum::Json<FileResponse>), ApiError> {
    let slot = parse_slot(&slot)?;
    let _session = get_session_or_404(&state, &session_id).await?;
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let filename = headers
        .get("X-File-Name")
        .and_then(|value| value.to_str().ok())
        .ok_or_else(|| ApiError::bad_request("missing X-File-Name header"))?;

    if !state.config.read().await.tftp.enabled() {
        return Err(ApiError::conflict("TFTP provider is disabled"));
    }

    let manager = state.tftp_manager.read().await.clone();
    let file = manager
        .put_session_file(&session_id, slot, filename, &body)
        .await
        .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?;
    let response = file_response_for_board(&state, &board, file).await?;
    Ok((StatusCode::CREATED, axum::Json(response)))
}

async fn get_session_file(
    Path((session_id, slot)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<axum::Json<FileResponse>, ApiError> {
    let slot = parse_slot(&slot)?;
    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let manager = state.tftp_manager.read().await.clone();
    let file = manager
        .get_session_file(&session_id, slot)
        .await?
        .ok_or_else(|| ApiError::not_found(format!("no file for slot `{slot}`")))?;
    Ok(axum::Json(
        file_response_for_board(&state, &board, file).await?,
    ))
}

async fn delete_session_file(
    Path((session_id, slot)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let slot = parse_slot(&slot)?;
    get_session_or_404(&state, &session_id).await?;
    let manager = state.tftp_manager.read().await.clone();
    manager.remove_session_file(&session_id, slot).await?;
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
    let server_ip = resolved_board_tftp_network(&state, &board)
        .await?
        .and_then(|network| network.server_ip);
    let files = session_file_responses(&state, &session_id, &board).await?;

    Ok(axum::Json(TftpSessionResponse {
        available: status.enabled && status.healthy && status.writable && server_ip.is_some(),
        provider: status.provider,
        server_ip,
        netmask: resolved_board_tftp_network(&state, &board)
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
    let tftp_url = resolved_board_tftp_network(state, board)
        .await?
        .and_then(|network| network.server_ip)
        .map(|server_ip| format!("tftp://{server_ip}/{}", file.relative_path));
    Ok(FileResponse::from_file(file, tftp_url))
}

async fn run_board_command(
    state: &AppState,
    session_id: &str,
    reset: bool,
) -> Result<axum::Json<ActionResponse>, ApiError> {
    let board = state
        .session_board(session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let BootConfig::Uboot(profile) = &board.boot else {
        return Err(ApiError::bad_request("board boot mode is not U-Boot"));
    };
    let command = if reset {
        profile.board_reset_cmd.as_deref()
    } else {
        profile.board_power_off_cmd.as_deref()
    }
    .filter(|cmd| !cmd.trim().is_empty())
    .ok_or_else(|| ApiError::bad_request("requested board action is not configured"))?;

    run_shell_command(command).map_err(ApiError::from)?;
    Ok(axum::Json(ActionResponse {
        ok: true,
        message: format!("executed `{command}`"),
    }))
}

fn parse_slot(raw: &str) -> Result<FileSlot, ApiError> {
    raw.parse::<FileSlot>()
        .map_err(|err| ApiError::bad_request(err.to_string()))
}

fn validate_board_id(board_id: &str) -> Result<(), ApiError> {
    if board_id.trim().is_empty() {
        return Err(ApiError::bad_request("board id must not be empty"));
    }
    if board_id.contains('/') || board_id.contains('\\') {
        return Err(ApiError::bad_request(
            "board id must not contain path separators",
        ));
    }
    Ok(())
}

fn leased_board_ids<'a>(
    sessions: &'a BTreeMap<String, crate::session::Session>,
) -> BTreeSet<&'a str> {
    sessions
        .values()
        .map(|session| session.board_id.as_str())
        .collect::<BTreeSet<_>>()
}

fn summarize_board_types(
    boards: &BTreeMap<String, BoardConfig>,
    sessions: &BTreeMap<String, crate::session::Session>,
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

fn readonly_server_config(config: &crate::config::ServerConfig) -> AdminServerConfigReadonly {
    AdminServerConfigReadonly {
        listen_addr: config.listen_addr.to_string(),
        data_dir: config.data_dir.display().to_string(),
        board_dir: config.board_dir.display().to_string(),
    }
}

fn server_config_response(config: &crate::config::ServerConfig) -> AdminServerConfigResponse {
    AdminServerConfigResponse {
        readonly: readonly_server_config(config),
        editable: AdminServerConfigEditable {
            lease: config.lease.clone(),
            tftp_network: config.tftp_network.clone(),
        },
    }
}

#[derive(Debug, Clone)]
struct ResolvedTftpNetwork {
    interface: Option<String>,
    server_ip: Option<String>,
    netmask: Option<String>,
}

fn resolve_server_tftp_network(
    config: &ServerConfig,
) -> Result<Option<ResolvedTftpNetwork>, ApiError> {
    let interface = if config.tftp_network.interface.trim().is_empty() {
        default_non_loopback_interface_name()
    } else {
        Some(config.tftp_network.interface.trim().to_string())
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

    Ok(Some(ResolvedTftpNetwork {
        interface,
        server_ip,
        netmask,
    }))
}

async fn resolved_board_tftp_network(
    state: &AppState,
    board: &BoardConfig,
) -> Result<Option<ResolvedTftpNetwork>, ApiError> {
    let BootConfig::Uboot(profile) = &board.boot else {
        return Ok(None);
    };
    if !profile.use_tftp {
        return Ok(None);
    }

    let config = state.config.read().await.clone();
    resolve_server_tftp_network(&config)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use serde_json::json;
    use tempfile::tempdir;
    use tower::util::ServiceExt;

    use super::{build_router, resolve_server_tftp_network};
    use crate::{
        build_app_state,
        config::{BoardConfig, BootConfig, BuiltinTftpConfig, ServerConfig, TftpConfig},
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
        config.data_dir = root.join("data");
        config.board_dir = root.join("boards");
        config.tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(root.join("tftp")));
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();
        state.ensure_data_dirs().await.unwrap();
        build_router(state)
    }

    async fn create_board(app: &Router, board: serde_json::Value) -> StatusCode {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(board.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
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
    async fn server_config_endpoint_updates_only_lease() {
        let app: Router = test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/server-config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"lease":{"default_ttl_secs":120,"max_ttl_secs":240,"gc_interval_secs":10},"tftp_network":{"interface":"lo"}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["editable"]["lease"]["default_ttl_secs"], 120);
        assert_eq!(value["editable"]["tftp_network"]["interface"], "lo");
        assert!(value["readonly"]["listen_addr"].is_string());
    }

    #[test]
    fn resolve_server_tftp_network_uses_configured_interface() {
        let mut config = ServerConfig::default();
        config.tftp_network.interface = "lo".into();

        let resolved = resolve_server_tftp_network(&config).unwrap().unwrap();
        assert_eq!(resolved.interface.as_deref(), Some("lo"));
    }

    #[test]
    fn board_config_new_uboot_profile_supports_use_tftp() {
        let board = BoardConfig {
            id: "demo".into(),
            name: "demo".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
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
    async fn board_types_endpoint_returns_aggregated_counts() {
        let app = test_router().await;
        let board_a = json!({
            "id": "rk3568-01",
            "name": "rk3568-01",
            "board_type": "rk3568",
            "tags": ["lab-a", "usbboot"],
            "serial": { "port": "/dev/ttyUSB0", "baud_rate": 115200 },
            "boot": { "kind": "uboot", "use_tftp": false },
            "notes": null,
            "disabled": false
        });
        let board_b = json!({
            "id": "rk3568-02",
            "name": "rk3568-02",
            "board_type": "rk3568",
            "tags": ["lab-b"],
            "serial": { "port": "/dev/ttyUSB1", "baud_rate": 115200 },
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
    async fn create_session_returns_created_when_board_is_available() {
        let app = test_router().await;
        let board = json!({
            "id": "demo-01",
            "name": "demo-01",
            "board_type": "demo",
            "tags": [],
            "serial": { "port": "/dev/ttyUSB0", "baud_rate": 115200 },
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
            "name": "demo-01",
            "board_type": "demo",
            "tags": [],
            "serial": { "port": "/dev/ttyUSB0", "baud_rate": 115200 },
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
