use std::{
    collections::{BTreeMap, BTreeSet},
    time::{Duration, Instant},
};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{Html, Redirect, Response},
    routing::{delete, get, post, put},
};
use serde_json::json;

use crate::{
    api::{
        error::ApiError,
        models::{
            ActionResponse, AdminSessionsResponse, AdminTftpConfigResponse,
            AdminTftpStatusResponse, BoardTypeSummary, BootProfileResponse, CreateSessionRequest,
            FileResponse, SerialStatusResponse, SessionCreatedResponse, SessionDetailResponse,
            TftpSessionResponse,
        },
    },
    config::{BoardConfig, BootConfig, TftpConfig},
    process::run_shell_command,
    serial::ws::run_serial_ws,
    state::AppState,
    tftp::{
        files::{FileSlot, TftpFileRef},
        service::build_tftp_manager,
        status::resolve_interface_ipv4,
    },
    web::assets::layout,
};

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/", get(|| async { Redirect::temporary("/admin/boards") }))
        .route(
            "/admin",
            get(|| async { Redirect::temporary("/admin/boards") }),
        )
        .route("/admin/boards", get(admin_boards_page))
        .route("/admin/boards/{board_id}", get(admin_board_detail_page))
        .route("/admin/sessions", get(admin_sessions_page))
        .route("/admin/tftp", get(admin_tftp_page))
        .route("/api/v1/admin/boards", get(list_boards).post(create_board))
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

async fn admin_boards_page() -> Html<String> {
    Html(layout(
        "Boards",
        r#"<div class="panel">
<p class="muted">Single-file board configs are stored under <code>.ostool-server/boards/</code>.</p>
<pre id="payload">Loading boards...</pre>
</div>
<script>
fetchJson('/api/v1/admin/boards')
  .then(data => renderJson('payload', data))
  .catch(err => renderJson('payload', { error: err.message }));
</script>"#,
    ))
}

async fn admin_board_detail_page(Path(board_id): Path<String>) -> Html<String> {
    Html(layout(
        "Board Detail",
        &format!(
            r#"<div class="panel">
<p class="muted">Board file: <code>{board_id}.toml</code></p>
<pre id="payload">Loading board...</pre>
</div>
<script>
fetchJson('/api/v1/admin/boards/{board_id}')
  .then(data => renderJson('payload', data))
  .catch(err => renderJson('payload', {{ error: err.message }}));
</script>"#
        ),
    ))
}

async fn admin_sessions_page() -> Html<String> {
    Html(layout(
        "Sessions",
        r#"<div class="panel">
<p class="muted">Active leases in the board pool.</p>
<pre id="payload">Loading sessions...</pre>
</div>
<script>
fetchJson('/api/v1/admin/sessions')
  .then(data => renderJson('payload', data))
  .catch(err => renderJson('payload', { error: err.message }));
</script>"#,
    ))
}

async fn admin_tftp_page() -> Html<String> {
    Html(layout(
        "TFTP",
        r#"<div class="panel">
<p class="muted">Service-wide TFTP configuration and health.</p>
<pre id="config">Loading TFTP config...</pre>
<pre id="status">Loading TFTP status...</pre>
</div>
<script>
Promise.all([
  fetchJson('/api/v1/admin/tftp'),
  fetchJson('/api/v1/admin/tftp/status')
]).then(([cfg, status]) => {
  renderJson('config', cfg);
  renderJson('status', status);
}).catch(err => {
  renderJson('config', { error: err.message });
  renderJson('status', { error: err.message });
});
</script>"#,
    ))
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
    Ok(axum::Json(AdminTftpConfigResponse {
        tftp: state.config.read().await.tftp.clone(),
    }))
}

async fn update_tftp_config(
    State(state): State<AppState>,
    axum::Json(tftp): axum::Json<TftpConfig>,
) -> Result<axum::Json<AdminTftpConfigResponse>, ApiError> {
    tokio::fs::create_dir_all(tftp.root_dir())
        .await
        .map_err(|err| ApiError::internal(err.to_string()))?;
    let new_manager = build_tftp_manager(&tftp);
    new_manager.start_if_needed().map_err(|err| {
        ApiError::service_unavailable(format!("failed to start TFTP provider: {err}"))
    })?;
    if matches!(tftp, TftpConfig::SystemTftpdHpa(_))
        && let Err(err) = new_manager.reconcile()
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
    let status = state.tftp_manager.read().await.status().map_err(|err| {
        ApiError::service_unavailable(format!("failed to get TFTP status: {err}"))
    })?;
    Ok(axum::Json(AdminTftpStatusResponse { status }))
}

async fn reconcile_tftp(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminTftpStatusResponse>, ApiError> {
    {
        let manager = state.tftp_manager.read().await;
        manager.reconcile().map_err(|err| {
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
    let leased = sessions
        .values()
        .map(|session| session.board_id.as_str())
        .collect::<BTreeSet<_>>();

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

    let result = aggregate
        .into_iter()
        .map(|(board_type, (tags, total, available))| BoardTypeSummary {
            board_type,
            tags: tags.into_iter().collect(),
            total,
            available,
        })
        .collect::<Vec<_>>();
    Ok(axum::Json(result))
}

async fn create_session(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<CreateSessionRequest>,
) -> Result<(StatusCode, axum::Json<SessionCreatedResponse>), ApiError> {
    if request.board_type.trim().is_empty() {
        return Err(ApiError::bad_request("board_type must not be empty"));
    }

    let deadline = request
        .timeout_ms
        .map(|timeout_ms| Instant::now() + Duration::from_millis(timeout_ms));

    let session = loop {
        if let Some(session) = state
            .create_session(
                &request.board_type,
                &request.required_tags,
                request.client_name.clone(),
            )
            .await
        {
            break session;
        }

        if !request.wait {
            return Err(ApiError::conflict(format!(
                "no available board for type `{}`",
                request.board_type
            )));
        }

        if let Some(deadline) = deadline
            && Instant::now() >= deadline
        {
            return Err(ApiError::conflict(format!(
                "timed out waiting for board type `{}`",
                request.board_type
            )));
        }

        tokio::time::sleep(Duration::from_millis(200)).await;
    };

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
    Ok(axum::Json(BootProfileResponse { boot: board.boot }))
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
        .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?;
    let response = file_response_for_board(&board, file).await?;
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
        .get_session_file(&session_id, slot)?
        .ok_or_else(|| ApiError::not_found(format!("no file for slot `{slot}`")))?;
    Ok(axum::Json(file_response_for_board(&board, file).await?))
}

async fn delete_session_file(
    Path((session_id, slot)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let slot = parse_slot(&slot)?;
    get_session_or_404(&state, &session_id).await?;
    let manager = state.tftp_manager.read().await.clone();
    manager.remove_session_file(&session_id, slot)?;
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
    let status = state.tftp_manager.read().await.status()?;
    let server_ip = resolve_tftp_server_ip(&board).await?;
    let files = session_file_responses(&state, &session_id, &board).await?;

    Ok(axum::Json(TftpSessionResponse {
        available: status.enabled && status.healthy && status.writable && server_ip.is_some(),
        provider: status.provider,
        server_ip,
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
    let files = manager.list_session_files(session_id)?;
    let mut responses = Vec::with_capacity(files.len());
    for file in files {
        responses.push(file_response_for_board(board, file).await?);
    }
    Ok(responses)
}

async fn file_response_for_board(
    board: &BoardConfig,
    file: TftpFileRef,
) -> Result<FileResponse, ApiError> {
    let tftp_url = resolve_tftp_server_ip(board)
        .await?
        .map(|server_ip| format!("tftp://{server_ip}/{}", file.relative_path));
    Ok(FileResponse::from_file(file, tftp_url))
}

async fn resolve_tftp_server_ip(board: &BoardConfig) -> Result<Option<String>, ApiError> {
    let BootConfig::Uboot(profile) = &board.boot else {
        return Ok(None);
    };
    let Some(net) = &profile.net else {
        return Ok(None);
    };
    if let Some(server_ip) = net
        .server_ip_override
        .as_ref()
        .filter(|value| !value.trim().is_empty())
    {
        return Ok(Some(server_ip.clone()));
    }
    resolve_interface_ipv4(&net.interface).map_err(|err| {
        ApiError::service_unavailable(format!("failed to resolve interface IP: {err}"))
    })
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
