use std::collections::{BTreeMap, BTreeSet};

use axum::{
    Router,
    body::{Body, Bytes, to_bytes},
    extract::{Extension, Path, Request, State, WebSocketUpgrade},
    http::{HeaderMap, HeaderValue, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use futures_util::future::join_all;
use httpboot_protocol::{
    BootFile, DEVICE_PROTOCOL_VERSION_HEADER, LoaderStatusResponse, MAX_HTTP_BOOT_INITRAMFS_BYTES,
    MacAddress, UNSUPPORTED_PROTOCOL_CODE, UNSUPPORTED_PROTOCOL_MESSAGE, valid_host_cmdline,
    validate_device_protocol_version_header,
};
use mime_guess::from_path;
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::fs;
use url::Url;

use crate::{
    api::{
        error::ApiError,
        models::{
            ActionResponse, AdminBoardSummary, AdminBoardUpsertRequest, AdminOverviewResponse,
            AdminServerConfigEditable, AdminServerConfigReadonly, AdminServerConfigResponse,
            AdminSessionsResponse, AdminTftpConfigResponse, AdminTftpStatusResponse,
            BoardPowerAction, BoardPowerStatusResponse, BoardRuntimeStatusResponse,
            BoardTypeSummary, BootProfileResponse, CreateSessionRequest,
            CreateVirtualDeviceRequest, DtbFileResponse, HeartbeatResponse, HttpBootFileResponse,
            KernelPublishResponse, LoaderDeviceSummary, NetworkInterfaceSummary, SerialPortSummary,
            SerialStatusResponse, SessionCreatedResponse, SessionDetailResponse,
            SessionDtbResponse, SharedSessionFileResponse, TftpSessionResponse,
            UpdateServerConfigRequest, VirtualDeviceSummary, VirtualDevicesResponse,
        },
    },
    board_pool::{BoardAllocationStatus, BoardAllocationWithIdStatus},
    config::{
        BoardConfig, BootConfig, PowerManagementConfig, ServerConfig, TftpConfig, UbootNetworkMode,
    },
    http_boot::publish::{KernelPublishInput, publish_kernel},
    persistence::dtbs::normalize_dtb_name,
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
    session::SessionStopReason,
    session::{SessionBootCommand, SessionState},
    state::{AppState, BoardLeaseState, TouchSessionError},
    static_assets::{
        serve_admin_asset, serve_admin_history, serve_admin_index, serve_site_history,
        serve_site_index,
    },
    tftp::{
        files::{TftpFileRef, normalize_relative_path},
        service::build_tftp_manager,
        status::resolve_interface_ipv4,
    },
};

const DTB_UPLOAD_MAX_MIB: u32 = 10;

pub fn build_router(state: AppState) -> Router {
    Router::new()
        .merge(super::site::routes())
        .merge(super::documents::routes())
        .merge(crate::serial::uboot::routes())
        .merge(crate::serial::commands::routes())
        .merge(super::auth::routes())
        .merge(super::users::routes())
        .merge(super::rentals::routes())
        .merge(super::account::routes())
        .merge(crate::audit::routes())
        .route("/api/v1/admin/dashboard", get(crate::dashboard::get))
        .route("/api/v1/admin/session-history", get(list_session_history))
        .merge(super::ota::routes())
        .merge(crate::device_security::routes())
        .route("/device-files/{id}", get(crate::device_security::download))
        .route("/api/v1/admin/events", get(crate::admin_events::subscribe))
        .route(
            "/api/v1/admin/power-actions",
            post(crate::admin_power::create),
        )
        .route(
            "/api/v1/admin/power-actions/{id}",
            get(crate::admin_power::get),
        )
        .route("/", get(serve_site_index))
        .route("/admin", get(serve_admin_index))
        .route("/admin/", get(serve_admin_index))
        .route("/admin/assets/{*path}", get(serve_admin_asset))
        .route("/assets/{*path}", get(serve_admin_asset))
        .route("/admin/{*path}", get(serve_admin_history))
        .route("/register", get(serve_site_index))
        .route("/rental-request", get(serve_site_index))
        .route("/password-reset", get(serve_site_index))
        .route("/account", get(serve_site_index))
        .route("/account/{*path}", get(serve_site_history))
        .route("/login", get(serve_site_index))
        .route("/login/", get(serve_site_index))
        .route("/login/{*path}", get(serve_site_history))
        .route("/devices", get(serve_site_index))
        .route("/devices/{*path}", get(serve_site_history))
        .route("/docs", get(serve_site_index))
        .route("/docs/{*path}", get(serve_site_history))
        .route("/legal/terms", get(crate::static_assets::serve_site_index))
        .route(
            "/legal/privacy",
            get(crate::static_assets::serve_site_index),
        )
        .route("/about", get(serve_site_index))
        .route("/about/{*path}", get(serve_site_history))
        .route(
            "/boot/sessions/{session_id}/{*path}",
            get(get_http_boot_file),
        )
        .route(
            "/share/sessions/{session_id}/{*path}",
            get(get_shared_session_file),
        )
        .route("/api/v1/admin/overview", get(get_admin_overview))
        .route("/api/v1/admin/boards", get(list_boards).post(create_board))
        .route("/api/v1/admin/loader-devices", get(list_loader_devices))
        .route(
            "/api/v1/admin/virtual-devices",
            get(list_virtual_devices).post(create_virtual_device),
        )
        .route(
            "/api/v1/admin/virtual-devices/{device_id}",
            delete(delete_virtual_device),
        )
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
            "/api/v1/sessions/{session_id}/loader-status",
            get(get_loader_status),
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
            get(get_session_file).delete(delete_session_file),
        )
        .route(
            "/api/v1/sessions/{session_id}/tftp",
            get(get_session_tftp_status),
        )
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            notify_admin_changes,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            super::auth::middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::security::web::gate,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::audit::middleware,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::security::web::transport,
        ))
        .with_state(state)
}

async fn get_admin_overview(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminOverviewResponse>, ApiError> {
    let status = get_tftp_status(State(state.clone())).await?.0.status;
    Ok(axum::Json(build_admin_overview(&state, status).await))
}

pub(crate) async fn projected_overview(
    state: &AppState,
    status: serde_json::Value,
) -> Result<serde_json::Value, ApiError> {
    let status = serde_json::from_value(status).map_err(|error| {
        ApiError::service_unavailable(format!("TFTP status unavailable: {error}"))
    })?;
    serde_json::to_value(build_admin_overview(state, status).await)
        .map_err(|error| ApiError::internal(error.to_string()))
}

async fn build_admin_overview(
    state: &AppState,
    tftp_status: crate::tftp::status::TftpStatus,
) -> AdminOverviewResponse {
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

    let config = state.config.read().await.clone();
    AdminOverviewResponse {
        board_count_total,
        board_count_available,
        disabled_board_count,
        active_session_count: session_snapshots(state)
            .await
            .into_iter()
            .filter(|session| session.state == crate::session::SessionLifecycleState::Active)
            .count(),
        board_types,
        tftp_status,
        server: readonly_server_config(&config),
    }
}

async fn list_boards(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<AdminBoardSummary>>, ApiError> {
    let boards = state
        .boards
        .read()
        .await
        .values()
        .cloned()
        .map(with_resolved_serial_config)
        .map(AdminBoardSummary::from)
        .collect::<Vec<_>>();
    Ok(axum::Json(boards))
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

pub(crate) async fn get_board(
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
        last_release_error: status
            .last_release_error
            .map(|_| "设备释放失败，请检查服务器日志".into()),
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
    Ok(axum::Json(DtbFileResponse::from_dtb(file)))
}

async fn create_dtb(
    State(state): State<AppState>,
    request: Request,
) -> Result<
    (
        StatusCode,
        axum::extract::Extension<crate::audit::Target>,
        axum::Json<DtbFileResponse>,
    ),
    ApiError,
> {
    let headers = request.headers();
    let dtb_name = dtb_name_header(headers, "X-Dtb-Name")?;
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
    Ok((
        StatusCode::CREATED,
        axum::extract::Extension(crate::audit::Target(dtb_name)),
        axum::Json(DtbFileResponse::from_dtb(file)),
    ))
}

fn serial_exclusion_error(error: anyhow::Error) -> ApiError {
    if error
        .downcast_ref::<ostool_serial::BindError>()
        .is_some_and(|error| matches!(error, ostool_serial::BindError::Busy))
    {
        ApiError::conflict("relay port is already leased by a session or manual operation")
    } else {
        error.into()
    }
}

async fn create_board(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<AdminBoardUpsertRequest>,
) -> Result<
    (
        StatusCode,
        axum::extract::Extension<crate::audit::Target>,
        axum::Json<BoardConfig>,
    ),
    ApiError,
> {
    let _inventory_guard = state.board_inventory_gate.lock().await;
    let mut board = build_board_config_for_create(&state, request).await?;
    // Credentials are provisioned separately after creation. Keep public HTTPboot
    // inventory unavailable until an administrator explicitly enables it.
    if state.config.read().await.edition.is_public()
        && matches!(board.boot, BootConfig::UefiHttp(_))
    {
        board.disabled = true;
    }
    validate_board_config(&board)?;
    if state.admin_power.is_busy(&board.power_management) {
        return Err(ApiError::conflict("power resource has a running action"));
    }
    validate_virtual_binding(&state, &board).await?;

    {
        let boards = state.boards.read().await;
        if boards.contains_key(&board.id) {
            return Err(ApiError::conflict(format!(
                "board `{}` already exists",
                board.id
            )));
        }
        ensure_unique_network_identity(&boards, &board, None)?;
        ensure_unique_relay(&boards, &board, None)?;
    }

    state
        .prepare_board_serial_exclusions(&board, None)
        .await
        .map_err(serial_exclusion_error)?;
    if let Err(error) = state.board_store.write_board(&board).await {
        state.refresh_serial_exclusions().await?;
        return Err(error.into());
    }
    state
        .boards
        .write()
        .await
        .insert(board.id.clone(), board.clone());
    state.sync_board_runtime_states().await;
    state.refresh_serial_exclusions().await?;
    Ok((
        StatusCode::CREATED,
        axum::extract::Extension(crate::audit::Target(board.id.clone())),
        axum::Json(board),
    ))
}

async fn update_board(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
    axum::Json(request): axum::Json<AdminBoardUpsertRequest>,
) -> Result<axum::Json<BoardConfig>, ApiError> {
    let _inventory_guard = state.board_inventory_gate.lock().await;
    let board = build_board_config_for_update(&state, &board_id, request).await?;
    if board.id != board_id {
        super::rentals::protect_board(&state, &board_id).await?;
    }
    validate_board_config(&board)?;
    if state.admin_power.is_busy(&board.power_management) {
        return Err(ApiError::conflict("power resource has a running action"));
    }
    let existing_board = state.boards.read().await.get(&board_id).cloned();
    if existing_board
        .as_ref()
        .is_some_and(|b| state.admin_power.is_busy(&b.power_management))
    {
        return Err(ApiError::conflict("power resource has a running action"));
    }
    validate_virtual_binding(&state, &board).await?;

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
        ensure_unique_network_identity(&boards, &board, Some(&board_id))?;
        ensure_unique_relay(&boards, &board, Some(&board_id))?;
    }

    if !board.disabled
        && matches!(board.boot, BootConfig::UefiHttp(_))
        && state.config.read().await.edition.is_public()
    {
        // A rename changes the encryption context; credentials must be provisioned
        // for the new ID before enabling it.
        if board.id != board_id
            || crate::device_security::connection(&state, &board.id)
                .await
                .is_err()
        {
            return Err(ApiError::bad_request(
                "请先禁用开发板并配置设备认证，再启用 HTTPboot",
            ));
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
        .prepare_board_serial_exclusions(&board, Some(&board_id))
        .await
        .map_err(serial_exclusion_error)?;
    if let Err(error) = state.board_store.replace_board(&board_id, &board).await {
        state.refresh_serial_exclusions().await?;
        return Err(error.into());
    }

    {
        let mut boards = state.boards.write().await;
        boards.remove(&board_id);
        boards.insert(board.id.clone(), board.clone());
    }
    state.sync_board_runtime_states().await;
    state.refresh_serial_exclusions().await?;

    Ok(axum::Json(board))
}

fn ensure_unique_network_identity(
    boards: &BTreeMap<String, BoardConfig>,
    candidate: &BoardConfig,
    replaced_board_id: Option<&str>,
) -> Result<(), ApiError> {
    let Some(identity) = candidate.network_identity.as_ref() else {
        return Ok(());
    };
    if let Some((board_id, _)) = boards.iter().find(|(board_id, board)| {
        Some(board_id.as_str()) != replaced_board_id
            && board
                .network_identity
                .as_ref()
                .is_some_and(|existing| existing.mac_address == identity.mac_address)
    }) {
        return Err(ApiError::mac_already_bound(format!(
            "MAC {} is already bound to board `{board_id}`",
            identity.mac_address
        )));
    }
    Ok(())
}

fn ensure_unique_relay(
    boards: &BTreeMap<String, BoardConfig>,
    candidate: &BoardConfig,
    replaced_board_id: Option<&str>,
) -> Result<(), ApiError> {
    let PowerManagementConfig::ZhongshengRelay(relay) = &candidate.power_management else {
        return Ok(());
    };
    let resolved = crate::serial::discovery::resolve_serial_key(&relay.key).ok();
    for (id, board) in boards {
        if Some(id.as_str()) == replaced_board_id {
            continue;
        }
        let PowerManagementConfig::ZhongshengRelay(existing) = &board.power_management else {
            continue;
        };
        let same_port = relay.key == existing.key
            || resolved.as_ref().is_some_and(|port| {
                crate::serial::discovery::resolve_serial_key(&existing.key)
                    .is_ok_and(|other| other.current_device_path == port.current_device_path)
            });
        if same_port {
            return Err(ApiError::conflict(format!("继电器已绑定开发板 `{id}`")));
        }
    }
    Ok(())
}

fn validate_board_config(board: &BoardConfig) -> Result<(), ApiError> {
    if board.id.eq_ignore_ascii_case("new") || matches!(board.id.as_str(), "." | "..") {
        return Err(ApiError::bad_request("板子 ID 不能使用保留值 new、. 或 .."));
    }
    if !board.disabled && matches!(board.boot, BootConfig::Uboot(_)) && board.serial.is_none() {
        return Err(ApiError::bad_request(
            "启用 U-Boot 开发板前必须配置串口；未完成配置可先禁用保存",
        ));
    }
    board
        .validate()
        .map_err(|error| ApiError::bad_request(format!("{error:#}")))
}

async fn validate_virtual_binding(state: &AppState, board: &BoardConfig) -> Result<(), ApiError> {
    let PowerManagementConfig::Qemu { virtual_device_id } = &board.power_management else {
        return Ok(());
    };
    let device = state
        .virtual_boards
        .snapshot(virtual_device_id)
        .await
        .ok_or_else(|| {
            ApiError::bad_request(format!(
                "virtual device `{virtual_device_id}` does not exist"
            ))
        })?;
    let board_mac = board
        .network_identity
        .as_ref()
        .expect("validated QEMU board has a network identity")
        .mac_address;
    if device.mac_address != board_mac {
        return Err(ApiError::bad_request(format!(
            "virtual device `{virtual_device_id}` has MAC {}, not {board_mac}",
            device.mac_address
        )));
    }
    Ok(())
}

async fn list_loader_devices(
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<LoaderDeviceSummary>>, ApiError> {
    let boards = state.boards.read().await;
    let snapshots = state.loader_registry.snapshots().await;
    Ok(axum::Json(
        snapshots
            .into_iter()
            .map(|device| LoaderDeviceSummary {
                bound_board_id: board_id_for_mac(&boards, device.mac_address),
                mac_address: device.mac_address,
                current_mac_address: device.current_mac_address,
                ip_address: device.ip_address,
                arch: device.arch,
                loader_version: device.loader_version,
                hardware: device.hardware,
                last_seen_at: device.last_seen_at,
                online: device.online,
                conflict: device.conflict,
                boot_epoch: device.boot_epoch,
            })
            .collect(),
    ))
}

async fn list_virtual_devices(State(state): State<AppState>) -> axum::Json<VirtualDevicesResponse> {
    let devices = state
        .virtual_boards
        .snapshots()
        .await
        .into_iter()
        .map(virtual_device_summary)
        .collect();
    axum::Json(VirtualDevicesResponse {
        enabled: state.virtual_boards.enabled(),
        devices,
    })
}

async fn create_virtual_device(
    State(state): State<AppState>,
    axum::Json(request): axum::Json<CreateVirtualDeviceRequest>,
) -> Result<(StatusCode, axum::Json<VirtualDeviceSummary>), ApiError> {
    let device = state
        .virtual_boards
        .create_device(request.mac_address)
        .await
        .map_err(|error| ApiError::service_unavailable(format!("{error:#}")))?;
    Ok((
        StatusCode::CREATED,
        axum::Json(virtual_device_summary(device)),
    ))
}

async fn delete_virtual_device(
    Path(device_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let _inventory_guard = state.board_inventory_gate.lock().await;
    if state.admin_power.is_busy(&PowerManagementConfig::Qemu {
        virtual_device_id: device_id.clone(),
    }) {
        return Err(ApiError::conflict("power resource has a running action"));
    }
    let boards = state.boards.read().await;
    if let Some(board) = boards.values().find(|board| {
        matches!(
            &board.power_management,
            PowerManagementConfig::Qemu { virtual_device_id } if virtual_device_id == &device_id
        )
    }) {
        return Err(ApiError::conflict(format!(
            "virtual device `{device_id}` is bound to board `{}`",
            board.id
        )));
    }
    drop(boards);
    state
        .virtual_boards
        .delete_device(&device_id)
        .await
        .map_err(|error| ApiError::not_found(format!("{error:#}")))?;
    Ok(StatusCode::NO_CONTENT)
}

fn virtual_device_summary(
    snapshot: crate::virtual_qemu::VirtualDeviceSnapshot,
) -> VirtualDeviceSummary {
    VirtualDeviceSummary {
        id: snapshot.id,
        mac_address: snapshot.mac_address,
        tap: snapshot.tap,
        powered: snapshot.powered,
        serial_connected: snapshot.serial_connected,
        generation: snapshot.generation,
    }
}

async fn get_loader_status(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<LoaderStatusResponse>, ApiError> {
    let session = state
        .session_state(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("session `{session_id}` not found")))?;
    let mut status = session
        .loader_status()
        .await
        .ok_or_else(|| ApiError::not_found("session has no published boot command"))?;
    if let Some(httpboot_protocol::LoaderStatusPhase::Failed { message, .. }) = &mut status.status {
        *message = "设备启动失败，请联系管理员".into();
    }
    Ok(axum::Json(status))
}

pub(crate) fn board_id_for_mac(
    boards: &BTreeMap<String, BoardConfig>,
    mac: MacAddress,
) -> Option<String> {
    boards.iter().find_map(|(board_id, board)| {
        board
            .network_identity
            .as_ref()
            .filter(|identity| identity.mac_address == mac)
            .map(|_| board_id.clone())
    })
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
    if matches!(request.boot, BootConfig::UefiHttp(_)) {
        request.serial = None;
    }
    normalize_serial_config(request.serial.as_mut())?;
    normalize_power_management_config(&mut request.power_management)?;
    normalize_boot_config(&mut request.boot)?;

    if matches!(request.boot, BootConfig::UefiHttp(_)) && request.network_identity.is_none() {
        return Err(ApiError::bad_request(
            "network_identity.mac_address is required for httpboot boards",
        ));
    }

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
        PowerManagementConfig::Qemu { virtual_device_id } => {
            normalize_required_string(virtual_device_id, "power_management.virtual_device_id")?;
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
            if let Some(parameters) = profile.serial_parameters {
                parameters
                    .validate()
                    .map_err(|err| ApiError::bad_request(format!("{err:#}")))?;
            }
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
            allow_instant_use: self.allow_instant_use,
            reset_command: self.reset_command,
            id: board_id,
            board_type: self.board_type,
            tags: self.tags,
            serial: self.serial,
            power_management: self.power_management,
            boot: self.boot,
            network_identity: self.network_identity,
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
    let _inventory_guard = state.board_inventory_gate.lock().await;
    if let Some(board) = state.boards.read().await.get(&board_id)
        && state.admin_power.is_busy(&board.power_management)
    {
        return Err(ApiError::conflict("power resource has a running action"));
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

    if !state.boards.read().await.contains_key(&board_id) {
        return Err(ApiError::not_found(format!("board `{board_id}` not found")));
    }
    super::rentals::protect_board(&state, &board_id).await?;
    state.board_store.delete_board(&board_id).await?;
    state.boards.write().await.remove(&board_id);
    state.sync_board_runtime_states().await;
    state.refresh_serial_exclusions().await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_admin_sessions(
    State(state): State<AppState>,
) -> Result<axum::Json<AdminSessionsResponse>, ApiError> {
    use sqlx::Row;
    let sessions = session_snapshots(&state).await;
    let ids: Vec<_> = sessions.iter().map(|session| session.id.clone()).collect();
    let owners = sqlx::query("SELECT s.session_id,s.user_id,u.username,s.rental_id,r.starts_at,r.ends_at FROM session_owners s LEFT JOIN users u ON u.id=s.user_id LEFT JOIN board_rentals r ON r.id=s.rental_id WHERE s.session_id=ANY($1)")
        .bind(ids).fetch_all(state.database.pool()).await?;
    let sessions = sessions
        .into_iter()
        .map(|mut session| {
            session.serial_runtime = session.serial_runtime.management_view();
            let owner = owners
                .iter()
                .find(|row| row.get::<String, _>("session_id") == session.id);
            Ok(super::models::AdminSession {
                session,
                owner_username: owner
                    .map(|row| row.try_get("username"))
                    .transpose()?
                    .flatten(),
                owner_user_id: owner.map(|row| row.try_get("user_id")).transpose()?,
                rental_id: owner
                    .map(|row| row.try_get("rental_id"))
                    .transpose()?
                    .flatten(),
                rental_starts_at: owner
                    .map(|row| row.try_get("starts_at"))
                    .transpose()?
                    .flatten(),
                rental_ends_at: owner
                    .map(|row| row.try_get("ends_at"))
                    .transpose()?
                    .flatten(),
            })
        })
        .collect::<Result<Vec<_>, sqlx::Error>>()?;
    Ok(axum::Json(AdminSessionsResponse { sessions }))
}

async fn list_session_history(
    State(state): State<AppState>,
    axum::extract::Query(filters): axum::extract::Query<crate::persistence::leases::LeaseFilters>,
) -> Result<axum::Json<crate::persistence::leases::LeasePage>, ApiError> {
    filters
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    state
        .database
        .lease_page(&filters)
        .await
        .map(axum::Json)
        .map_err(ApiError::from)
}

async fn delete_admin_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    let removed = state
        .request_session_stop(&session_id, SessionStopReason::AdminRelease)
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
    let _admission = state.rental_gate.lock().await;
    if tftp.root_dir() != state.tftp_manager.read().await.root_dir()
        && !state.sessions.read().await.is_empty()
    {
        return Err(ApiError::conflict("请先释放所有会话，再修改文件存储目录"));
    }

    tftp.validate_for_edition(state.config.read().await.edition)
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
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
    *state.tftp_manager.write().await = std::sync::Arc::new(
        crate::tftp::service::DatabaseTftpManager::new(new_manager, state.session_files.clone()),
    );

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
    request
        .upload_limits
        .validate()
        .map_err(|e| ApiError::bad_request(e.to_string()))?;

    {
        let mut config = state.config.write().await;
        config.network = request.network;
        config.upload_limits = request.upload_limits;
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
    actor: Option<Extension<super::auth::CurrentUser>>,
) -> Result<axum::Json<Vec<BoardTypeSummary>>, ApiError> {
    let allowed = match actor {
        Some(Extension(super::auth::CurrentUser(user))) if !user.is_admin => {
            let mut allowed = super::rentals::active(&state, user.id).await?;
            for (id, end) in super::instant::candidates(&state, &user, None, None, &[]).await? {
                allowed.entry(id).or_insert((uuid::Uuid::nil(), end));
            }
            Some(allowed)
        }
        _ => None,
    };
    let boards = state.boards.read().await;
    let visible = boards
        .iter()
        .filter(|(id, _)| allowed.as_ref().is_none_or(|a| a.contains_key(*id)))
        .map(|(id, board)| (id.clone(), board.clone()))
        .collect();
    let runtimes = state.board_runtimes.read().await;
    Ok(axum::Json(summarize_board_types(&visible, &runtimes)))
}

async fn create_session(
    State(state): State<AppState>,
    context: Option<Extension<crate::audit::Context>>,
    actor: Option<Extension<super::auth::CurrentUser>>,
    axum::Json(request): axum::Json<CreateSessionRequest>,
) -> Result<
    (
        StatusCode,
        Extension<crate::audit::Target>,
        axum::Json<SessionCreatedResponse>,
    ),
    ApiError,
> {
    if request.board_type.trim().is_empty() {
        return Err(ApiError::bad_request("board_type must not be empty"));
    }
    let board_id = request.board_id.as_deref().map(str::trim);
    if board_id == Some("") {
        return Err(ApiError::bad_request(
            "board_id must not be empty when provided",
        ));
    }

    let _rental_guard = state.rental_gate.lock().await;
    let mut rental_access = match actor.as_ref() {
        Some(Extension(super::auth::CurrentUser(user))) if !user.is_admin => {
            Some(super::rentals::active(&state, user.id).await?)
        }
        _ => None,
    };
    let mut instant_rental = None;
    if let (Some(access), Some(Extension(super::auth::CurrentUser(user)))) =
        (&mut rental_access, actor.as_ref())
    {
        let boards = state.boards.read().await;
        let runtimes = state.board_runtimes.read().await;
        let available = access.keys().any(|id| {
            boards.get(id).is_some_and(|b| {
                b.board_type == request.board_type
                    && !b.disabled
                    && request.required_tags.iter().all(|t| b.tags.contains(t))
            }) && board_id.is_none_or(|v| id == v)
                && runtimes
                    .get(id)
                    .is_none_or(|r| r.lease_state == crate::BoardLeaseState::Idle)
        });
        drop(runtimes);
        drop(boards);
        if !available
            && crate::security::permissions::access(&state.database, user)
                .await?
                .allows("board.instant")
        {
            let (id, rental, end) = super::instant::assign(
                &state,
                user,
                &request.board_type,
                board_id,
                &request.required_tags,
            )
            .await?;
            access.clear();
            access.insert(id, (rental, end));
            instant_rental = Some(rental);
        }
    }
    let allocation = async {
        let session = if let Some(access) = &rental_access {
            let allowed = access.keys().cloned().collect::<BTreeSet<_>>();
            if allowed.is_empty() || board_id.is_some_and(|id| !allowed.contains(id)) {
                return Err(ApiError::forbidden(
                    "当前时间没有已分配的开发板，请联系管理员查看租赁",
                ));
            }
            state
                .create_session_for_rental(
                    &request.board_type,
                    board_id,
                    &request.required_tags,
                    request.client_name.clone(),
                    &allowed,
                )
                .await
                .map_err(|error| match error {
                    BoardAllocationWithIdStatus::DatabaseUnavailable => {
                        ApiError::service_unavailable("租约数据库暂时不可用")
                    }
                    BoardAllocationWithIdStatus::NoAvailableBoard => {
                        // Existing ostool clients classify these exact errors to wait
                        // for a busy board. Rental authorization must keep that contract.
                        ApiError::conflict(match board_id {
                            Some(id) => format!("board `{id}` is not available"),
                            None => format!("no available board for type `{}`", request.board_type),
                        })
                    }
                    BoardAllocationWithIdStatus::BoardTypeMismatch {
                        board_id,
                        board_type,
                        actual_board_type,
                    } => ApiError::bad_request(format!(
                        "board `{board_id}` has type `{actual_board_type}`, not `{board_type}`"
                    )),
                    BoardAllocationWithIdStatus::BoardTypeNotFound => ApiError::not_found(format!(
                        "board type `{}` not found",
                        request.board_type
                    )),
                    BoardAllocationWithIdStatus::BoardNotFound => {
                        ApiError::not_found(match board_id {
                            Some(id) => format!("board `{id}` not found"),
                            None => "board not found".into(),
                        })
                    }
                })?
        } else {
            match board_id {
                Some(board_id) => state
                    .create_session_with_board_id(
                        &request.board_type,
                        board_id,
                        &request.required_tags,
                        request.client_name.clone(),
                    )
                    .await
                    .map_err(|err| match err {
                        BoardAllocationWithIdStatus::DatabaseUnavailable => {
                            ApiError::service_unavailable("租约数据库暂时不可用")
                        }
                        BoardAllocationWithIdStatus::BoardTypeNotFound => ApiError::not_found(
                            format!("board type `{}` not found", request.board_type),
                        ),
                        BoardAllocationWithIdStatus::BoardNotFound => {
                            ApiError::not_found(format!("board `{board_id}` not found"))
                        }
                        BoardAllocationWithIdStatus::BoardTypeMismatch {
                            board_id,
                            board_type,
                            actual_board_type,
                        } => ApiError::bad_request(format!(
                            "board `{board_id}` has type `{actual_board_type}`, not `{board_type}`"
                        )),
                        BoardAllocationWithIdStatus::NoAvailableBoard => {
                            ApiError::conflict(format!("board `{board_id}` is not available"))
                        }
                    })?,
                None => state
                    .create_session(
                        &request.board_type,
                        &request.required_tags,
                        request.client_name.clone(),
                    )
                    .await
                    .map_err(|err| match err {
                        BoardAllocationStatus::DatabaseUnavailable => {
                            ApiError::service_unavailable("租约数据库暂时不可用")
                        }
                        BoardAllocationStatus::BoardTypeNotFound => ApiError::not_found(format!(
                            "board type `{}` not found",
                            request.board_type
                        )),
                        BoardAllocationStatus::NoAvailableBoard => ApiError::conflict(format!(
                            "no available board for type `{}`",
                            request.board_type
                        )),
                    })?,
            }
        };

        Ok::<_, ApiError>(session)
    }
    .await;
    let session = match allocation {
        Ok(session) => session,
        Err(error) => {
            if let Some(id) = instant_rental {
                super::instant::cancel_failed(&state, id).await;
            }
            return Err(error);
        }
    };
    let rental = rental_access
        .as_ref()
        .and_then(|access| access.get(&session.board_id))
        .copied();
    if let Some((_, deadline)) = rental
        && let Some(runtime) = state.session_state(&session.id).await
    {
        runtime.bind_rental_deadline(deadline).await;
    }
    if let Some(Extension(super::auth::CurrentUser(user))) = actor {
        let ownership = async {
            let mut tx = state.database.begin().await?;
            let updated=sqlx::query("UPDATE session_history SET user_id=$2,username=$3,rental_id=$4,expires_at=LEAST(expires_at,COALESCE($5,expires_at)) WHERE session_id=$1 AND ended_at IS NULL")
                .bind(&session.id).bind(user.id).bind(&user.username).bind(rental.map(|(id,_)|id)).bind(rental.map(|(_,deadline)|deadline))
                .execute(&mut *tx).await?;
            if updated.rows_affected()!=1 { return Err(sqlx::Error::RowNotFound); }
            sqlx::query("INSERT INTO session_owners(session_id,user_id,rental_id) VALUES($1,$2,$3)")
                .bind(&session.id).bind(user.id).bind(rental.map(|(id, _)| id))
                .execute(&mut *tx).await?;
            tx.commit().await
        }.await;
        if let Err(error) = ownership {
            state
                .request_session_stop(&session.id, SessionStopReason::AuthorizationUnavailable)
                .await;
            if let Some(id) = instant_rental {
                super::instant::cancel_failed(&state, id).await;
            }
            log::error!("failed to persist session owner: {error}");
            return Err(ApiError::service_unavailable(
                "failed to record session owner",
            ));
        }
        state
            .admin_events
            .invalidate(&["sessions", "session_history"]);
    }
    if let (Some(rental), Some(Extension(context))) = (instant_rental, context) {
        crate::audit::event(&state,&context,"rental.instant.created","即时分配设备","rentals",Some(&rental.to_string()),true,
            serde_json::json!({"board_id":session.board_id,"session_id":session.id,"ends_at":rental_access.as_ref().and_then(|a|a.get(&session.board_id)).map(|(_,end)|end)})).await;
    }
    let session = state
        .get_session(&session.id)
        .await
        .ok_or_else(|| ApiError::conflict("session expired during creation"))?;
    let board = state
        .session_board(&session.id)
        .await
        .ok_or_else(|| ApiError::not_found("allocated board disappeared"))?;
    let ws_url = board
        .serial_available()
        .then(|| format!("/api/v1/sessions/{}/serial/ws", session.id));

    Ok((
        StatusCode::CREATED,
        Extension(crate::audit::Target(session.id.clone())),
        axum::Json(SessionCreatedResponse {
            session_id: session.id,
            board_id: board.id.clone(),
            lease_expires_at: session.expires_at,
            serial_available: board.serial_available(),
            boot_mode: board.boot.kind_name().to_string(),
            ws_url,
        }),
    ))
}

async fn get_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<SessionDetailResponse>, ApiError> {
    let mut session = get_session_or_404(&state, &session_id).await?;

    session.serial_runtime = session.serial_runtime.public_view();

    let board = state
        .session_board(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session board not found"))?;
    let files = session_file_responses(&state, &session_id).await?;
    let connected = session.serial_connected;

    Ok(axum::Json(SessionDetailResponse {
        session,
        board: (&board).into(),
        serial_available: board.serial_available(),
        serial_connected: connected,
        files,
    }))
}

async fn heartbeat_session(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<HeartbeatResponse>, ApiError> {
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
    Ok(axum::Json(HeartbeatResponse {
        session_id: session.id,
        lease_expires_at: session.expires_at,
    }))
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
    let mut boot = board.boot;
    match &mut boot {
        BootConfig::UefiHttp(profile) => profile.serial_parameters = None,
        BootConfig::Uboot(profile) => {
            profile.use_tftp = false;
            profile.server_ip = None;
            profile.board_ip = None;
            profile.netmask = None;
            profile.gatewayip = None;
            profile.network_mode = UbootNetworkMode::Dhcp;
        }
        BootConfig::Pxe(profile) => profile.notes = None,
    }
    let config = state.config.read().await;
    Ok(axum::Json(BootProfileResponse {
        boot,
        server_ip: None,
        netmask: None,
        interface: None,
        http_base_url: Some(http_boot_public_base_url(&config)?),
        server_side_operations: true,
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
        file_response(&state, file).await?.tftp_url
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
    let runtime = state
        .session_state(&session_id)
        .await
        .map(|s| s.serial_runtime.snapshot())
        .unwrap_or_default();
    let manager = *state.serial_manager.subscribe().borrow();
    let mut response = if matches!(board.boot, BootConfig::UefiHttp(_)) {
        SerialStatusResponse {
            available: true,
            connected,
            port: runtime.port.clone(),
            baud_rate: runtime
                .parameters
                .and_then(|p| u32::try_from(p.baud_rate).ok()),
            ws_url: Some(format!("/api/v1/sessions/{session_id}/serial/ws")),
            runtime,
            manager,
        }
    } else if let Some(serial) = board.serial {
        let port = if serial.key.kind == crate::config::SerialPortKeyKind::Qemu {
            if state
                .virtual_boards
                .snapshot(&serial.key.value)
                .await
                .is_none()
            {
                return Err(ApiError::service_unavailable(format!(
                    "virtual device `{}` is unavailable",
                    serial.key.value
                )));
            }
            format!("qemu:{}", serial.key.value)
        } else {
            resolve_serial_config(&serial)
                .map_err(|err| {
                    log::warn!("session {session_id} serial unavailable: {err:#}");
                    ApiError::service_unavailable("设备串口暂时不可用")
                })?
                .current_device_path
        };
        SerialStatusResponse {
            runtime,
            manager,
            available: true,
            connected,
            port: Some(port),
            baud_rate: Some(serial.baud_rate),
            ws_url: Some(format!("/api/v1/sessions/{session_id}/serial/ws")),
        }
    } else {
        SerialStatusResponse {
            runtime,
            manager,
            available: false,
            connected: false,
            port: None,
            baud_rate: None,
            ws_url: None,
        }
    };
    response.port = None;
    response.baud_rate = None;
    response.runtime = response.runtime.public_view().management_view();
    response.manager = Default::default();
    Ok(axum::Json(response))
}

fn with_resolved_serial_config(mut board: BoardConfig) -> BoardConfig {
    if let Some(serial) = board.serial.as_mut() {
        if serial.key.kind == crate::config::SerialPortKeyKind::Qemu {
            serial.resolved_device_path = Some(format!("qemu:{}", serial.key.value));
            serial.resolved_usb_path = None;
        } else if let Ok(resolved) = resolve_serial_config(serial) {
            serial.resolved_device_path = Some(resolved.current_device_path);
            serial.resolved_usb_path = resolved.usb_path;
        }
    }

    board
}

async fn serial_ws(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    credential: Option<Extension<crate::security::web::CurrentCredential>>,
    ws: WebSocketUpgrade,
) -> Result<Response, ApiError> {
    let session = state
        .session_state(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let board = session.board().clone();
    if !board.serial_available() {
        return Err(ApiError::conflict("board has no serial configuration"));
    }
    if session.is_releasing()
        || session.is_stop_requested()
        || session.snapshot().await.expires_at <= chrono::Utc::now()
    {
        return Err(ApiError::conflict("session is releasing"));
    }

    if !session.try_set_serial_connected() {
        return Err(ApiError::conflict("serial websocket already connected"));
    }

    Ok(ws.on_upgrade(move |socket| async move {
        let watcher = credential.map(
            |Extension(crate::security::web::CurrentCredential(credential))| {
                let state = state.clone();
                let id = session_id.clone();
                tokio::spawn(async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                        if !state
                            .database
                            .credential_active(&credential)
                            .await
                            .unwrap_or(false)
                        {
                            state
                                .request_session_stop(&id, SessionStopReason::SerialClosed)
                                .await;
                            break;
                        }
                    }
                })
            },
        );
        run_serial_ws(socket, state, session).await;
        if let Some(watcher) = watcher {
            watcher.abort();
        }
    }))
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

async fn get_shared_session_file(
    Path((session_id, path)): Path<(String, String)>,
    State(state): State<AppState>,
    request: Request,
) -> Result<Response, ApiError> {
    let relative_path = parse_relative_path(&path)?;
    active_shared_session_or_404(&state, &session_id).await?;
    read_session_file_response(&state, &session_id, &relative_path, request.headers()).await
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
    read_session_file_response(state, session_id, relative_path, headers).await
}

pub(crate) async fn read_session_file_response(
    state: &AppState,
    session_id: &str,
    relative_path: &str,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let stored = state
        .session_files
        .get(session_id, relative_path)
        .await?
        .ok_or_else(|| ApiError::not_found("session file not found"))?;
    read_stored_file_response(state, stored, headers).await
}

pub(crate) async fn read_stored_file_response(
    state: &AppState,
    stored: crate::persistence::files::StoredFile,
    headers: &HeaderMap,
) -> Result<Response, ApiError> {
    let file_len =
        usize::try_from(stored.size).map_err(|_| ApiError::internal("session file too large"))?;
    let content_type = from_path(&stored.path).first_or_octet_stream().to_string();
    let content_type = HeaderValue::from_str(&content_type).map_err(|err| {
        ApiError::service_unavailable(format!("invalid content type `{content_type}`: {err}"))
    })?;
    let range = headers
        .get(header::RANGE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| parse_single_byte_range(v, file_len));
    let (start, length) = range
        .map(|(start, end)| (start, end - start + 1))
        .unwrap_or((0, file_len));
    let files = state.session_files.clone();
    let session = state
        .session_state(&stored.session_id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    let global = state
        .uploads
        .downloads
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            ApiError::new(
                StatusCode::TOO_MANY_REQUESTS,
                "download_busy",
                "下载并发已满",
            )
            .with_retry_after(2)
        })?;
    let own = session.downloads.clone().try_acquire_owned().map_err(|_| {
        ApiError::new(
            StatusCode::TOO_MANY_REQUESTS,
            "download_busy",
            "该会话下载并发已满",
        )
        .with_retry_after(2)
    })?;
    let mut shutdown = session.subscribe_shutdown();
    let timeout = std::time::Duration::from_secs(
        state.config.read().await.upload_limits.upload_timeout_secs as u64,
    );
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    tokio::spawn(async move {
        let _slots = (global, own);
        let transfer = async {
            let mut remaining = length;
            let mut position = start;
            while remaining > 0 {
                let chunk = files
                    .chunk(
                        stored.id,
                        (position / crate::persistence::files::CHUNK_SIZE) as u64,
                    )
                    .await
                    .map_err(std::io::Error::other)?;
                let offset = position % crate::persistence::files::CHUNK_SIZE;
                let n = remaining.min(chunk.len().saturating_sub(offset));
                if n == 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::UnexpectedEof,
                        "file changed during download",
                    ));
                }
                let bytes = Bytes::copy_from_slice(&chunk[offset..offset + n]);
                position += n;
                remaining -= n;
                sender.send(Ok(bytes)).await.map_err(|_| {
                    std::io::Error::new(std::io::ErrorKind::BrokenPipe, "download closed")
                })?;
            }
            Ok::<_, std::io::Error>(())
        };
        let result = tokio::select! {
            biased;
            _=shutdown.wait_for(|stopped|*stopped)=>Err(std::io::Error::other("session ended")),
            result=tokio::time::timeout(timeout,transfer)=>result.unwrap_or_else(|_|Err(std::io::Error::new(std::io::ErrorKind::TimedOut,"download timed out"))),
        };
        if let Err(error) = result {
            let _ = sender.try_send(Err(error));
        }
    });
    let stream = futures_util::stream::unfold(receiver, |mut receiver| async move {
        receiver.recv().await.map(|chunk| (chunk, receiver))
    });
    let mut response = Response::new(Body::from_stream(stream));
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, content_type);
    response.headers_mut().insert(
        header::CONTENT_LENGTH,
        HeaderValue::from_str(&length.to_string()).expect("numeric length"),
    );
    response
        .headers_mut()
        .insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    if let Some((start, end)) = range {
        *response.status_mut() = StatusCode::PARTIAL_CONTENT;
        response.headers_mut().insert(
            header::CONTENT_RANGE,
            HeaderValue::from_str(&format!("bytes {start}-{end}/{file_len}"))
                .expect("numeric byte range"),
        );
    }
    Ok(response)
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
    require_device_protocol_version_header(request.headers())?;
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
    require_device_protocol_version_header(request.headers())?;
    let session = active_session_state_or_404(&state, &session_id).await?;
    let board = session.board().clone();
    ensure_httpboot_board(&board)?;
    let headers = request.headers();
    let remote_name = optional_header(headers, "X-HttpBoot-Remote-Name")?
        .unwrap_or_else(|| "kernel.elf".to_string());
    let remote_name = parse_relative_path(&remote_name)?;
    let initramfs_path = optional_header(headers, "X-HttpBoot-Initramfs-Path")?
        .map(|path| parse_relative_path(&path))
        .transpose()?;
    let cmdline = optional_header(headers, "X-HttpBoot-Cmdline")?;
    if cmdline
        .as_deref()
        .is_some_and(|value| !valid_host_cmdline(value))
    {
        return Err(ApiError::bad_request("invalid host kernel command line"));
    }
    if !state.config.read().await.http_boot.enabled {
        return Err(ApiError::conflict("HTTP Boot is disabled"));
    }

    let upload = crate::uploads::receive(&state, &session_id, request, "HTTP Boot kernel").await?;
    let _boot_operation = upload
        .within_deadline(session.boot_operation.lock())
        .await?;
    // The lease can stop while this request waits for another publication.
    active_session_state_or_404(&state, &session_id).await?;

    let initramfs = upload
        .within_deadline(async {
            Ok::<_, ApiError>(if let Some(path) = initramfs_path.as_deref() {
                let manager = state.tftp_manager.read().await.clone();
                let file = manager
                    .get_session_file(&session_id, path)
                    .await
                    .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?
                    .ok_or_else(|| {
                        ApiError::bad_request("host initramfs not uploaded to this session")
                    })?;
                let size = fs::metadata(&file.disk_path)
                    .await
                    .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?
                    .len();
                if size == 0 {
                    return Err(ApiError::bad_request("host initramfs is empty"));
                }
                if size > MAX_HTTP_BOOT_INITRAMFS_BYTES as u64 {
                    return Err(ApiError::bad_request(
                        "host initramfs exceeds HTTP Boot loader limit",
                    ));
                }
                let bytes = fs::read(&file.disk_path)
                    .await
                    .map_err(|err| ApiError::service_unavailable(format!("{err:#}")))?;
                if bytes.len() as u64 != size {
                    return Err(ApiError::conflict(
                        "host initramfs changed during publication",
                    ));
                }
                let config = state.config.read().await.clone();
                let url = http_boot_url(&config, &session_id, path)?;
                Some(BootFile {
                    path: url.path().into(),
                    size: bytes.len() as u64,
                    sha256: hex_sha256(&bytes),
                })
            } else {
                None
            })
        })
        .await??;

    let (kernel_file, kernel_size, kernel_sha256) =
        upload.commit(&state, &session_id, &remote_name).await?;
    let config = state.config.read().await.clone();
    let kernel_response = http_boot_file_response(&config, &session_id, &remote_name, kernel_file)?;
    let response = publish_kernel(KernelPublishInput {
        kernel_url: kernel_response.http_url.to_string(),
        kernel_size,
        kernel_sha256: Some(kernel_sha256),
    });
    let kernel_path = kernel_response.http_url.path().to_string();
    session
        .publish_boot_command(SessionBootCommand {
            boot_id: response.boot_id.clone(),
            kernel_path,
            kernel_size,
            kernel_sha256: response
                .kernel_sha256
                .clone()
                .expect("kernel upload always computes SHA-256"),
            initramfs,
            cmdline,
        })
        .await;

    Ok((StatusCode::CREATED, axum::Json(response)))
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

fn require_device_protocol_version_header(headers: &HeaderMap) -> Result<(), ApiError> {
    let value = headers
        .get(DEVICE_PROTOCOL_VERSION_HEADER)
        .map(HeaderValue::to_str)
        .transpose()
        .map_err(|_| {
            ApiError::new(
                StatusCode::BAD_REQUEST,
                UNSUPPORTED_PROTOCOL_CODE,
                UNSUPPORTED_PROTOCOL_MESSAGE,
            )
        })?;
    validate_device_protocol_version_header(value).map_err(|error| {
        ApiError::new(
            StatusCode::BAD_REQUEST,
            UNSUPPORTED_PROTOCOL_CODE,
            error.message,
        )
    })
}

async fn list_session_files(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
) -> Result<axum::Json<Vec<SharedSessionFileResponse>>, ApiError> {
    get_session_or_404(&state, &session_id).await?;
    Ok(axum::Json(
        session_file_responses(&state, &session_id).await?,
    ))
}

async fn put_session_file(
    Path(session_id): Path<String>,
    State(state): State<AppState>,
    request: Request,
) -> Result<(StatusCode, axum::Json<SharedSessionFileResponse>), ApiError> {
    let session = state
        .session_state(&session_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("session `{session_id}` not found")))?;
    if session.is_releasing() || session.is_stop_requested() {
        return Err(ApiError::conflict(format!(
            "session `{session_id}` is releasing"
        )));
    }
    get_session_or_404(&state, &session_id).await?;
    let (_relative_path, file) =
        put_session_file_from_request(&state, &session_id, request, "session file").await?;
    let response = file_response(&state, file).await?;
    Ok((StatusCode::CREATED, axum::Json(response)))
}

async fn get_session_file(
    Path((session_id, path)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<axum::Json<SharedSessionFileResponse>, ApiError> {
    let relative_path = parse_relative_path(&path)?;
    get_session_or_404(&state, &session_id).await?;
    let manager = state.tftp_manager.read().await.clone();
    let stored = state
        .session_files
        .get(&session_id, &relative_path)
        .await?
        .ok_or_else(|| ApiError::not_found("session file not found"))?;
    let file = stored.reference(manager.root_dir())?;
    Ok(axum::Json(file_response(&state, file).await?))
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
    let _files = session.file_gate.lock().await;
    if session.is_stop_requested() {
        return Err(ApiError::conflict("会话已结束"));
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
    get_session_or_404(&state, &session_id).await?;
    let status = state.tftp_manager.read().await.status().await?;
    let files = session_file_responses(&state, &session_id).await?;
    let public = state.config.read().await.edition.is_public();
    let network = if !public && status.enabled {
        let board = state
            .session_board(&session_id)
            .await
            .ok_or_else(|| ApiError::not_found("session board not found"))?;
        resolved_board_network(&state, &board).await?
    } else {
        None
    };
    let server_ip = network
        .as_ref()
        .and_then(|network| network.server_ip.clone());

    Ok(axum::Json(TftpSessionResponse {
        available: !public
            && status.enabled
            && status.healthy
            && status.writable
            && server_ip.is_some(),
        provider: status.provider,
        server_ip,
        netmask: network.and_then(|network| network.netmask),
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
        crate::uploads::store(state, session_id, &relative_path, request, body_label).await?;
    Ok((relative_path, file))
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
) -> Result<Vec<SharedSessionFileResponse>, ApiError> {
    let manager = state.tftp_manager.read().await.clone();
    let files = manager.list_session_files(session_id).await?;
    let mut responses = Vec::with_capacity(files.len());
    for file in files {
        responses.push(file_response(state, file).await?);
    }
    Ok(responses)
}

async fn file_response(
    state: &AppState,
    file: TftpFileRef,
) -> Result<SharedSessionFileResponse, ApiError> {
    let http_url = shared_http_url(state, &file.relative_path).await?;
    let config = state.config.read().await.clone();
    let tftp_url = if !config.edition.is_public() && config.tftp.enabled() {
        let sid = file
            .relative_path
            .strip_prefix("ostool/sessions/")
            .and_then(|path| path.split('/').next())
            .ok_or_else(|| ApiError::service_unavailable("invalid stored session file path"))?;
        let board = state
            .session_board(sid)
            .await
            .ok_or_else(|| ApiError::not_found("session board not found"))?;
        if let Some(ip) = resolved_board_network(state, &board)
            .await?
            .and_then(|network| network.server_ip)
        {
            let mut url = Url::parse(&format!("tftp://{ip}/"))
                .map_err(|error| ApiError::internal(error.to_string()))?;
            if let TftpConfig::Builtin(tftp) = &config.tftp
                && tftp.bind_addr.port() != 69
            {
                url.set_port(Some(tftp.bind_addr.port()))
                    .map_err(|_| ApiError::internal("invalid TFTP port"))?;
            }
            append_url_path(&mut url, &[], &file.relative_path)?;
            Some(url)
        } else {
            None
        }
    } else {
        None
    };
    Ok(SharedSessionFileResponse::from_file(
        file,
        tftp_url,
        Some(http_url),
    ))
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
    state
        .execute_board_power_action(&board, action)
        .await
        .map_err(|err| match err {
            PowerActionError::NotConfigured | PowerActionError::InvalidConfig(_) => {
                ApiError::bad_request(err.to_string())
            }
            PowerActionError::Execution(err) => ApiError::from(err),
        })?;

    let message = format!("{} completed", action.label());
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
    if session.is_releasing()
        || session.is_stop_requested()
        || session.snapshot().await.expires_at <= chrono::Utc::now()
    {
        return Err(ApiError::conflict(format!(
            "session `{session_id}` is releasing"
        )));
    }
    Ok(session)
}

async fn active_shared_session_or_404(
    state: &AppState,
    session_id: &str,
) -> Result<Arc<SessionState>, ApiError> {
    let session = state
        .session_state(session_id)
        .await
        .ok_or_else(|| ApiError::not_found(format!("session `{session_id}` not found")))?;
    if session.is_releasing()
        || session.is_stop_requested()
        || session.snapshot().await.expires_at <= chrono::Utc::now()
    {
        return Err(ApiError::not_found(format!(
            "session `{session_id}` not found"
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
) -> Result<Url, ApiError> {
    let relative_path = normalize_relative_path(relative_path)
        .map_err(|err| ApiError::bad_request(err.to_string()))?;
    let mut url = http_boot_public_base_url(config)?;
    append_url_path(&mut url, &["boot", "sessions", session_id], &relative_path)?;
    Ok(url)
}

pub(crate) fn http_boot_public_base_url(config: &ServerConfig) -> Result<Url, ApiError> {
    if !config.edition.is_public() {
        return Url::parse(
            config
                .auth
                .public_base_url
                .as_deref()
                .unwrap_or("http://127.0.0.1:2999"),
        )
        .map_err(|_| ApiError::service_unavailable("服务访问地址无效"));
    }
    config
        .auth
        .public_url()
        .map_err(|_| ApiError::service_unavailable("服务访问地址未配置或无效"))
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
        http_boot_public_base_url: config.http_boot.public_base_url.clone(),
        dtb_upload_max_mib: DTB_UPLOAD_MAX_MIB,
    }
}

fn server_config_response(config: &crate::config::ServerConfig) -> AdminServerConfigResponse {
    AdminServerConfigResponse {
        readonly: readonly_server_config(config),
        editable: AdminServerConfigEditable {
            network: config.network.clone(),
            upload_limits: config.upload_limits.clone(),
        },
    }
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
pub(crate) struct ResolvedNetwork {
    pub(crate) server_ip: Option<String>,
    pub(crate) netmask: Option<String>,
}

fn resolve_server_network(config: &ServerConfig) -> Result<Option<ResolvedNetwork>, ApiError> {
    let interface = if config.network.interface.trim().is_empty() {
        default_non_loopback_interface_name()
    } else {
        Some(config.network.interface.trim().to_string())
    };
    if interface.as_deref() == Some("lo") {
        return Ok(Some(ResolvedNetwork {
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

    Ok(Some(ResolvedNetwork { server_ip, netmask }))
}

pub(crate) async fn resolved_board_network(
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

async fn shared_http_url(state: &AppState, stored_relative_path: &str) -> Result<Url, ApiError> {
    let relative_path = stored_relative_path
        .split_once("/sessions/")
        .map(|(_, relative)| relative)
        .ok_or_else(|| ApiError::service_unavailable("invalid stored session file path"))?;
    let mut url = http_boot_public_base_url(&*state.config.read().await)?;
    append_url_path(&mut url, &["share", "sessions"], relative_path)?;
    Ok(url)
}

fn append_url_path(
    url: &mut Url,
    prefix_segments: &[&str],
    relative_path: &str,
) -> Result<(), ApiError> {
    let mut segments = url
        .path_segments_mut()
        .map_err(|_| ApiError::service_unavailable("HTTP base URL cannot contain path segments"))?;
    segments.pop_if_empty();
    for segment in prefix_segments {
        segments.push(segment);
    }
    for segment in relative_path.split('/') {
        segments.push(segment);
    }
    Ok(())
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

async fn notify_admin_changes(
    State(state): State<AppState>,
    request: Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mutation =
        request.method() != axum::http::Method::GET && request.method() != axum::http::Method::HEAD;
    let path = request.uri().path().to_owned();
    let response = next.run(request).await;
    if mutation {
        if response.status().is_success()
            && (path.starts_with("/api/v1/auth/")
                || path.starts_with("/api/v1/admin/auth/")
                || path.starts_with("/oauth/")
                || path.starts_with("/api/v1/admin/users")
                || path.starts_with("/api/v1/admin/roles"))
        {
            state
                .security_revision
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            state.admin_events.invalidate(&["security_revision"]);
        }
        let topics: &[&str] = if path.contains("/auth/") {
            &["mail_deliveries"]
        } else if path.contains("/loaders/") {
            &["loaders"]
        } else if path.contains("/boards") {
            &["boards"]
        } else if path.contains("/sessions") {
            &["sessions", "virtual"]
        } else if path.contains("/virtual-devices") {
            &["virtual", "serial"]
        } else if path.contains("/dtbs") {
            &["dtbs"]
        } else if path.contains("/tftp") {
            &["tftp"]
        } else if path.contains("/server-config") {
            &["server"]
        } else {
            &[]
        };
        state.admin_events.invalidate(topics);
    }
    response
}

pub(crate) async fn admin_topic(
    state: &AppState,
    topic: &str,
) -> Result<serde_json::Value, ApiError> {
    if !state.config.read().await.edition.is_public()
        && matches!(
            topic,
            "account_settings"
                | "users"
                | "roles"
                | "rentals"
                | "rental_requests"
                | "mail_deliveries"
        )
    {
        return Ok(if topic == "account_settings" {
            serde_json::Value::Null
        } else {
            serde_json::json!([])
        });
    }
    let extractor = State(state.clone());
    let value = match topic {
        "account_settings" => Ok(super::site::account_settings(extractor).await?.0),
        "site" => serde_json::to_value(super::site::view(state).await?),
        "device_documents" => Ok(serde_json::json!(
            state.database.device_document_summaries().await?
        )),
        "device_catalog" => Ok(serde_json::json!(super::site::device_catalog(state).await)),
        "legal_history" => Ok(super::site::history(extractor).await?.0),
        "mail_deliveries" => Ok(super::site::mail_deliveries(extractor).await?.0),
        "security_revision" => Ok(serde_json::json!(
            state
                .security_revision
                .load(std::sync::atomic::Ordering::Relaxed)
        )),
        "boards" => serde_json::to_value(list_boards(extractor).await?.0),
        "board_revision" => Ok(serde_json::json!(state.admin_events.revision("boards"))),
        "sessions" => serde_json::to_value(list_admin_sessions(extractor).await?.0.sessions),
        "session_history" => {
            Ok(serde_json::json!({"active":false,"revision":state.admin_events.lease_revision()}))
        }
        "loaders" => serde_json::to_value(list_loader_devices(extractor).await?.0),
        "ota" => Ok(
            serde_json::json!({ "jobs": state.ota.jobs().await.into_iter().map(crate::ota::Job::management_view).collect::<Vec<_>>(), "images": state.ota.images().await? }),
        ),
        "virtual" => serde_json::to_value(list_virtual_devices(extractor).await.0),
        "dtbs" => serde_json::to_value(list_dtbs(extractor).await?.0),
        "serial" => serde_json::to_value(list_serial_ports().await?.0),
        "serial_manager" => serde_json::to_value(*state.serial_manager.subscribe().borrow()),
        "network" => serde_json::to_value(list_network_interfaces().await?.0),
        "server" => serde_json::to_value(get_server_config(extractor).await?.0),
        "tftp" => serde_json::to_value(get_tftp_config(extractor).await?.0.tftp),
        "tftp_status" => serde_json::to_value(get_tftp_status(extractor).await?.0.status),
        "overview" => serde_json::to_value(get_admin_overview(extractor).await?.0),
        "power_actions" => serde_json::to_value(state.admin_power.snapshots()),
        "users" | "rentals" | "rental_requests" => {
            Ok(serde_json::json!({"revision":state.admin_events.revision(topic)}))
        }
        "roles" => serde_json::to_value(super::users::list_roles(extractor).await?.0),
        "permissions" => Ok(serde_json::json!(crate::security::permissions::CATALOG)),
        "deployment" => Ok(super::users::deployment(extractor).await.0),
        "dashboard" => Ok(crate::dashboard::projection(state).await),
        "audit" => {
            let last: Option<uuid::Uuid> = {
                let db = &state.database;
                sqlx::query_scalar(
                    "SELECT id FROM audit_events ORDER BY created_at DESC,id DESC LIMIT 1",
                )
                .fetch_optional(db.pool())
                .await
                .map_err(|_| ApiError::service_unavailable("审计数据库暂时不可用"))?
            };
            Ok(serde_json::json!({"active":false,"revision":state.audit.revision(),"last_id":last}))
        }
        "runtimes" => {
            let mut values = BTreeMap::new();
            for (id, r) in state.board_runtimes.read().await.iter() {
                values.insert(id.clone(), serde_json::json!({"lease_state":r.lease_state,"active_session_id":r.active_session_id,"last_release_error":r.last_release_error.as_ref().map(|_|"设备释放失败，请检查服务器日志"),"updated_at":r.updated_at}));
            }
            serde_json::to_value(values)
        }
        _ => return Err(ApiError::bad_request("unknown admin topic")),
    };
    value.map_err(|e| ApiError::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::ensure_unique_relay;
    use axum::{
        Json,
        extract::{Path, State},
    };
    use std::collections::BTreeMap;
    use std::future;
    use std::sync::Arc;

    use axum::{
        Router,
        body::{Body, to_bytes},
        http::{Request, StatusCode, header},
    };
    use httpboot_protocol::{DEVICE_PROTOCOL_VERSION, DEVICE_PROTOCOL_VERSION_HEADER};
    use serde::Serialize;
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

    use super::{DTB_UPLOAD_MAX_MIB, http_boot_url, mib_to_bytes, resolve_server_network};
    use crate::{
        api::models::{
            AdminBoardUpsertRequest, BoardRuntimeStatusResponse, BootProfileResponse,
            CreateSessionRequest, SessionCreatedResponse, SessionDetailResponse,
            SharedSessionFileResponse,
        },
        config::{
            BoardConfig, BootConfig, BuiltinTftpConfig, CustomPowerManagement,
            PowerManagementConfig, SerialConfig, SerialPortKey, SerialPortKeyKind, ServerConfig,
            TftpConfig, UefiHttpProfile, UploadLimitsConfig, ZhongshengRelayPowerManagement,
        },
        session::SessionLifecycleState,
        state::BoardLeaseState,
        static_assets::first_asset_path,
        test_support::build_test_app_state as build_app_state,
        tftp::service::{TftpManager, build_tftp_manager},
    };

    #[cfg(unix)]
    type RelayServerHandle =
        tokio::task::JoinHandle<std::io::Result<tokio_modbus::server::Terminated>>;
    #[cfg(unix)]
    type RelayRequestRx = mpsc::UnboundedReceiver<(u8, u16, bool)>;
    #[cfg(unix)]
    type RelayTestServer = (
        String,
        TTYPort,
        RelayServerHandle,
        RelayRequestRx,
        oneshot::Sender<()>,
    );

    fn test_server_config(root: &std::path::Path) -> ServerConfig {
        ServerConfig {
            security: Default::default(),
            audit: Default::default(),
            edition: crate::config::Edition::Local,
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            database: crate::config::DatabaseConfig::default(),
            auth: crate::config::AuthConfig::default(),
            network_test: crate::config::NetworkTestConfig::default(),
            data_dir: root.join("data"),
            tftp: TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(root.join("tftp"))),
            http_boot: crate::config::HttpBootConfig::default_with_root(root.join("http-boot")),
            loader_network: crate::config::LoaderNetworkConfig::default(),
            virtual_qemu: crate::config::VirtualQemuConfig::default_with_runtime_dir(
                root.join("qemu"),
            ),
            network: crate::TftpNetworkConfig {
                interface: "lo".into(),
            },
            upload_limits: UploadLimitsConfig::default(),
        }
    }

    #[tokio::test]
    async fn unbound_power_action_requires_neither_board_nor_mac() {
        let (app, state) = test_router_and_state_with_config(|_| {}).await;
        let response = app.oneshot(Request::builder()
            .method("POST").uri("/api/v1/admin/power-actions")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::json!({
                "request_id": "unbound-regression", "action": "on",
                "power_management": {"kind": "custom", "power_on_cmd": "true", "power_off_cmd": "true"}
            }).to_string())).unwrap()).await.unwrap();
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        assert!(state.boards.read().await.is_empty());
        assert!(state.sessions.read().await.is_empty());
    }

    async fn post_power(app: &Router, request: serde_json::Value) -> axum::response::Response {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/power-actions")
                    .header("content-type", "application/json")
                    .body(Body::from(request.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn power_action_is_idempotent_and_completion_is_pushed_to_all_subscribers() {
        use futures_util::StreamExt;
        let (app, state) = test_router_and_state_with_config(|_| {}).await;
        let path = state.config.read().await.data_dir.join("power-count");
        let mut streams = Vec::new();
        for _ in 0..2 {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri("/api/v1/admin/events")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let mut stream = response.into_body().into_data_stream();
            let first = stream.next().await.unwrap().unwrap();
            assert!(String::from_utf8_lossy(&first).contains("snapshot"));
            streams.push(stream);
        }
        let request = json!({"request_id":"repeat", "action":"on", "power_management":{"kind":"custom","power_on_cmd":format!("printf x >> '{}'", path.display()),"power_off_cmd":"true"}});
        assert_eq!(
            post_power(&app, request.clone()).await.status(),
            StatusCode::ACCEPTED
        );
        assert_eq!(
            post_power(&app, request.clone()).await.status(),
            StatusCode::ACCEPTED
        );
        for mut stream in streams {
            tokio::time::timeout(std::time::Duration::from_secs(5), async {
                while let Some(chunk) = stream.next().await {
                    let text = String::from_utf8(chunk.unwrap().to_vec()).unwrap();
                    if text.contains("succeeded") {
                        return;
                    }
                }
                panic!("SSE ended before completion");
            })
            .await
            .expect("power completion event");
        }
        assert_eq!(tokio::fs::read(&path).await.unwrap(), b"x");
        let mut changed = request.clone();
        changed["action"] = json!("off");
        assert_eq!(
            post_power(&app, changed).await.status(),
            StatusCode::CONFLICT
        );
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/power-actions/repeat")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(body["state"], "succeeded");
        assert!(state.boards.read().await.is_empty());
    }

    #[tokio::test]
    async fn unbound_power_rejects_busy_board() {
        let (app, state) = test_router_and_state_with_config(|_| {}).await;
        let board = json!({"id":"busy", "board_type":"test", "tags":[], "disabled":false, "serial":null, "notes":null, "network_identity":null, "boot":{"kind":"pxe","notes":null}, "power_management":{"kind":"custom","power_on_cmd":"true","power_off_cmd":"true"}});
        assert_eq!(create_board(&app, board.clone()).await, StatusCode::CREATED);
        let session = state.create_session("test", &[], None).await.unwrap();
        for action in ["on", "off"] {
            let response = post_power(&app, json!({"request_id": action, "action":action, "power_management":board["power_management"]})).await;
            assert_eq!(response.status(), StatusCode::CONFLICT);
        }
        assert_eq!(state.get_session(&session.id).await.unwrap().id, session.id);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn unbound_relay_action_uses_real_modbus_transport() {
        let (app, state) = test_router_and_state_with_config(|_| {}).await;
        let (relay_port, _handle, server, mut requests, stop_tx) = spawn_relay_test_server();
        let response = post_power(&app, json!({"request_id":"relay", "action":"on", "power_management":{"kind":"zhongsheng_relay","key":{"kind":"usb_path","value":relay_port}}})).await;
        assert_eq!(response.status(), StatusCode::ACCEPTED);
        let write = tokio::time::timeout(std::time::Duration::from_secs(2), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(write, (1, 0, true));
        assert!(state.boards.read().await.is_empty());
        let _ = stop_tx.send(());
        let _ = server.await.unwrap();
    }

    #[tokio::test]
    async fn sse_reconnect_replays_and_failed_power_is_observable() {
        use futures_util::StreamExt;
        let app = test_router().await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/events")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut stream = response.into_body().into_data_stream();
        let first = String::from_utf8(stream.next().await.unwrap().unwrap().to_vec()).unwrap();
        let cursor = first
            .lines()
            .find_map(|line| line.strip_prefix("id:"))
            .unwrap()
            .trim()
            .to_string();
        drop(stream);
        assert_eq!(post_power(&app, json!({"request_id":"failure", "action":"on", "power_management":{"kind":"custom","power_on_cmd":"exit 17","power_off_cmd":"true"}})).await.status(), StatusCode::ACCEPTED);
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/events")
                    .header("last-event-id", cursor)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let mut stream = response.into_body().into_data_stream();
        tokio::time::timeout(std::time::Duration::from_secs(3), async {
            while let Some(chunk) = stream.next().await {
                let text = String::from_utf8(chunk.unwrap().to_vec()).unwrap();
                assert!(!text.contains("event: snapshot"));
                if text.contains("failed") {
                    return;
                }
            }
            panic!("stream closed before failed result");
        })
        .await
        .unwrap();
    }

    async fn test_router() -> Router {
        test_router_with_config(|_| {}).await
    }

    async fn test_router_with_config(update: impl FnOnce(&mut ServerConfig)) -> Router {
        let (router, _) = test_router_and_state_with_config(update).await;
        router
    }

    async fn test_router_and_state_with_config(
        update: impl FnOnce(&mut ServerConfig),
    ) -> (Router, crate::AppState) {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        std::mem::forget(temp);
        let config_path = root.join(".ostool-server.toml");
        let mut config = test_server_config(&root);
        update(&mut config);
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();
        state.ensure_data_dirs().await.unwrap();
        (
            crate::test_support::authenticated_router(&state)
                .await
                .unwrap(),
            state,
        )
    }

    async fn create_board(app: &Router, request: impl Serialize) -> StatusCode {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    async fn create_session(app: &Router, board_type: &str) -> String {
        let request = CreateSessionRequest {
            board_id: None,
            board_type: board_type.to_string(),
            required_tags: Vec::new(),
            client_name: Some("test".to_string()),
        };
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/sessions")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&request).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let response: SessionCreatedResponse = serde_json::from_slice(&body).unwrap();
        response.session_id
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
            allow_instant_use: false,
            reset_command: None,
            id: board_id.into(),
            board_type: "rk3568".into(),
            tags: vec!["lab".into(), "usb".into()],
            serial: Some(SerialConfig {
                key: SerialPortKey {
                    kind: SerialPortKeyKind::UsbPath,
                    value: "/dev/serial/by-path/pci-0000:00:14.0-usb-0:10.4:1.0-port0".into(),
                },
                baud_rate: 115_200,
                resolved_device_path: None,
                resolved_usb_path: None,
            }),
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "echo on".into(),
                power_off_cmd: "echo off".into(),
            }),
            boot: BootConfig::Uboot(crate::config::UbootProfile {
                use_tftp: true,
                ..Default::default()
            }),
            network_identity: None,
            notes: Some("rack-a".into()),
            disabled: false,
        }
    }

    fn sample_httpboot_board(board_id: &str) -> BoardConfig {
        let mut board = sample_board(board_id);
        board.board_type = "x86_64-uefi-http".into();
        board.tags = vec!["uefi-http".into()];
        board.boot = BootConfig::UefiHttp(UefiHttpProfile {
            serial_parameters: None,
        });
        board.network_identity = Some(crate::config::BoardNetworkIdentity {
            mac_address: "02:00:00:00:00:01".parse().unwrap(),
        });
        board
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
    fn spawn_relay_test_server() -> RelayTestServer {
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
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "no-store"
        );
        assert_eq!(
            response.headers().get(header::CONTENT_TYPE).unwrap(),
            "text/html"
        );

        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("ostool · 管理后台"));
        assert!(!body.contains("开发板协作平台"));
    }

    #[tokio::test]
    async fn website_and_admin_documents_have_independent_entries() {
        let app = test_router().await;
        let mut documents = Vec::new();
        let mut nonces = std::collections::HashSet::new();
        for path in [
            "/",
            "/devices",
            "/account/profile",
            "/login",
            "/admin/login",
            "/admin/boards",
        ] {
            let response = app
                .clone()
                .oneshot(Request::builder().uri(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            assert_eq!(response.headers()[header::CACHE_CONTROL], "no-store");
            let body = String::from_utf8(
                to_bytes(response.into_body(), usize::MAX)
                    .await
                    .unwrap()
                    .to_vec(),
            )
            .unwrap();
            if path.starts_with("/admin/") {
                assert!(body.contains("ostool · 管理后台"));
                assert!(body.contains("/assets/admin-"));
                assert!(!body.contains("/assets/website-"));
            } else {
                assert!(body.contains("开发板协作平台"));
                assert!(!body.contains("管理后台"));
                assert!(body.contains("/assets/website-"));
                assert!(!body.contains("/assets/admin-"));
            }
            let nonce = body
                .split_once("name=\"csp-nonce\" content=\"")
                .unwrap()
                .1
                .split('"')
                .next()
                .unwrap();
            assert!(
                nonces.insert(nonce.to_owned()),
                "HTML nonce must be unique per response"
            );
            documents.push(body.replace(nonce, "REQUEST_NONCE"));
        }
        assert_eq!(documents[0], documents[1]);
        assert_ne!(documents[0], documents[4]);
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
        assert_eq!(
            response.headers().get(header::CACHE_CONTROL).unwrap(),
            "public, max-age=31536000, immutable"
        );
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
                    .body(Body::from(
                        r#"{"network":{"interface":"lo"},"upload_limits":{"session_file_max_mib":32}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(value["editable"]["network"]["interface"], "lo");
        assert_eq!(
            value["editable"]["upload_limits"]["session_file_max_mib"],
            32
        );
        assert_eq!(value["readonly"]["dtb_upload_max_mib"], DTB_UPLOAD_MAX_MIB);
        assert!(value["readonly"]["listen_addr"].is_string());
    }

    #[test]
    fn resolve_server_network_uses_configured_interface() {
        let mut config = ServerConfig::default();
        config.network.interface = "lo".into();

        let resolved = resolve_server_network(&config).unwrap().unwrap();
        assert_eq!(resolved.server_ip.as_deref(), Some("127.0.0.1"));
        assert_eq!(resolved.netmask.as_deref(), Some("255.0.0.0"));
    }

    #[test]
    fn http_boot_url_uses_configured_public_base_url_first() {
        let mut config = ServerConfig {
            listen_addr: "0.0.0.0:2999".parse().unwrap(),
            ..ServerConfig::default()
        };
        config.network.interface = "lo".into();
        config.auth.public_base_url = Some("https://boards.example.com/".into());

        let url = http_boot_url(&config, "asus-1", "kernel.bin").unwrap();
        assert_eq!(
            url.as_str(),
            "https://boards.example.com/boot/sessions/asus-1/kernel.bin"
        );
    }

    #[test]
    fn http_boot_url_defaults_to_resolved_server_ip() {
        let mut config = ServerConfig {
            listen_addr: "0.0.0.0:2999".parse().unwrap(),
            ..ServerConfig::default()
        };
        config.network.interface = "lo".into();
        config.http_boot.public_base_url = None;
        config.auth.public_base_url = Some("http://127.0.0.1:2999".into());

        let url = http_boot_url(&config, "asus-1", "kernel.elf").unwrap();
        assert_eq!(
            url.as_str(),
            "http://127.0.0.1:2999/boot/sessions/asus-1/kernel.elf"
        );
    }

    #[test]
    fn board_config_new_uboot_profile_supports_use_tftp() {
        let board = BoardConfig {
            allow_instant_use: false,
            reset_command: None,
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
            network_identity: None,
            notes: None,
            disabled: false,
        };

        match board.boot {
            BootConfig::Uboot(profile) => assert!(profile.use_tftp),
            BootConfig::Pxe(_) => panic!("expected uboot"),
            BootConfig::UefiHttp(_) => panic!("expected uboot"),
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

    async fn put_board(app: &Router, id: &str, board: &BoardConfig) -> StatusCode {
        app.clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/admin/boards/{id}"))
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(board).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap()
            .status()
    }

    #[tokio::test]
    async fn onboarding_rejects_reserved_ids_on_create_and_update() {
        let app = test_router().await;
        assert_eq!(
            create_board(&app, sample_board("valid")).await,
            StatusCode::CREATED
        );
        for id in ["new", "New", "NEW", ".", "..", " new "] {
            let board = sample_board(id);
            assert_eq!(create_board(&app, &board).await, StatusCode::BAD_REQUEST);
            assert_eq!(
                put_board(&app, "valid", &board).await,
                StatusCode::BAD_REQUEST
            );
        }
    }

    #[tokio::test]
    async fn onboarding_uboot_draft_requires_serial_before_enable() {
        let app = test_router().await;
        let mut board = sample_board("uart-draft");
        let serial = board.serial.take();
        assert_eq!(create_board(&app, &board).await, StatusCode::BAD_REQUEST);
        board.disabled = true;
        assert_eq!(create_board(&app, &board).await, StatusCode::CREATED);
        board.disabled = false;
        assert_eq!(
            put_board(&app, &board.id, &board).await,
            StatusCode::BAD_REQUEST
        );
        board.serial = serial;
        assert_eq!(put_board(&app, &board.id, &board).await, StatusCode::OK);
    }

    #[tokio::test]
    async fn onboarding_relay_ownership_is_unique_on_create_and_update() {
        let app = test_router().await;
        let mut first = sample_board("relay-owner");
        first.power_management =
            PowerManagementConfig::ZhongshengRelay(crate::config::ZhongshengRelayPowerManagement {
                key: SerialPortKey {
                    kind: SerialPortKeyKind::SerialNumber,
                    value: "offline-relay".into(),
                },
            });
        assert_eq!(create_board(&app, &first).await, StatusCode::CREATED);
        let mut second = first.clone();
        second.id = "relay-other".into();
        assert_eq!(create_board(&app, &second).await, StatusCode::CONFLICT);
        assert_eq!(
            create_board(&app, sample_board(&second.id)).await,
            StatusCode::CREATED
        );
        assert_eq!(
            put_board(&app, &second.id, &second).await,
            StatusCode::CONFLICT
        );
        assert_eq!(put_board(&app, &first.id, &first).await, StatusCode::OK);
    }

    #[cfg(unix)]
    #[test]
    fn onboarding_relay_aliases_cannot_bypass_ownership() {
        let root = tempdir().unwrap();
        let port = root.path().join("port");
        let alias = root.path().join("alias");
        std::fs::write(&port, "").unwrap();
        std::os::unix::fs::symlink(&port, &alias).unwrap();
        let mut first = sample_board("owner");
        first.power_management =
            PowerManagementConfig::ZhongshengRelay(crate::config::ZhongshengRelayPowerManagement {
                key: SerialPortKey {
                    kind: SerialPortKeyKind::UsbId,
                    value: port.to_string_lossy().into(),
                },
            });
        let mut other = first.clone();
        other.id = "other".into();
        other.power_management =
            PowerManagementConfig::ZhongshengRelay(crate::config::ZhongshengRelayPowerManagement {
                key: SerialPortKey {
                    kind: SerialPortKeyKind::UsbPath,
                    value: alias.to_string_lossy().into(),
                },
            });
        let boards = BTreeMap::from([(first.id.clone(), first)]);
        assert!(ensure_unique_relay(&boards, &other, None).is_err());
        assert!(ensure_unique_relay(&boards, &other, Some("owner")).is_ok());
    }

    #[tokio::test]
    async fn onboarding_public_httpboot_stays_disabled_until_credentials_are_provisioned() {
        let (_, state) = test_router_and_state_with_config(|_| {}).await;
        state.config.write().await.edition = crate::config::Edition::Public;
        let board = sample_httpboot_board("public-draft");
        let (_, _, Json(mut saved)) = super::create_board(
            State(state.clone()),
            Json(serde_json::from_value(serde_json::to_value(board).unwrap()).unwrap()),
        )
        .await
        .unwrap();
        assert!(saved.disabled);
        assert!(
            state
                .create_session(&saved.board_type, &[], None)
                .await
                .is_err()
        );
        saved.disabled = false;
        let request = || serde_json::from_value(serde_json::to_value(&saved).unwrap()).unwrap();
        assert!(
            super::update_board(
                Path(saved.id.clone()),
                State(state.clone()),
                Json(request())
            )
            .await
            .is_err()
        );
        let key = "AQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQEBAQE=";
        state.config.write().await.security.mfa_encryption_key = Some(key.into());
        let ciphertext = crate::security::crypto::seal(
            key,
            &format!("device:{}", saved.id),
            b"0123456789abcdef0123456789abcdef",
        )
        .unwrap();
        sqlx::query(
            "INSERT INTO device_connections(board_id,endpoint,token_ciphertext) VALUES($1,$2,$3)",
        )
        .bind(&saved.id)
        .bind("https://board.test")
        .bind(ciphertext)
        .execute(state.database.pool())
        .await
        .unwrap();
        let Json(enabled) = super::update_board(
            Path(saved.id.clone()),
            State(state.clone()),
            Json(request()),
        )
        .await
        .unwrap();
        assert!(!enabled.disabled);
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
        let serial = board.serial.unwrap();
        assert_eq!(serial.key.kind, SerialPortKeyKind::UsbPath);
        assert_eq!(
            serial.key.value,
            "/dev/serial/by-path/pci-0000:00:14.0-usb-0:10.4:1.0-port0"
        );
    }

    #[tokio::test]
    async fn create_httpboot_board_persists_canonical_network_identity() {
        let app = test_router().await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "id": "httpboot-create",
                            "board_type": "Asus-nuc15-x86_64-vmx",
                            "tags": ["httpboot"],
                            "serial": null,
                            "power_management": {
                                "kind": "custom",
                                "power_on_cmd": "true",
                                "power_off_cmd": "true"
                            },
                            "boot": {
                                "kind": "httpboot",
                                "serial_parameters": null
                            },
                            "network_identity": {
                                "mac_address": "1C:69:7A:DC:F3:47"
                            },
                            "notes": null,
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
        let BootConfig::UefiHttp(profile) = board.boot else {
            panic!("expected httpboot");
        };
        assert_eq!(profile.serial_parameters, None);
        assert_eq!(
            board.network_identity.unwrap().mac_address.to_string(),
            "1c:69:7a:dc:f3:47"
        );
    }

    #[tokio::test]
    async fn create_board_rejects_duplicate_mac_with_stable_error_code() {
        let app = test_router().await;
        let first = sample_httpboot_board("uefi-http-01");
        assert_eq!(
            create_board(&app, serde_json::to_value(&first).unwrap()).await,
            StatusCode::CREATED
        );
        let mut second = sample_httpboot_board("uefi-http-02");
        second.board_type = "another-pool".into();

        let response = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/boards")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&second).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let error: crate::api::models::ErrorResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(error.code, "mac_already_bound");
    }

    #[tokio::test]
    async fn virtual_device_list_reports_disabled_without_starting_qemu() {
        let app = test_router().await;
        let response = app
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/virtual-devices")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let virtual_devices: crate::api::models::VirtualDevicesResponse =
            serde_json::from_slice(&body).unwrap();
        assert!(!virtual_devices.enabled);
        assert!(virtual_devices.devices.is_empty());
    }

    #[tokio::test]
    async fn qemu_board_binding_can_be_configured_before_device_authentication() {
        let (app, state) = test_router_and_state_with_config(|config| {
            config.virtual_qemu.enabled = true;
            config.virtual_qemu.tap_pool = vec!["test-tap".into()];
        })
        .await;
        let virtual_id = "virtual-discovery-test";
        let mac = "02:00:00:00:00:42".parse().unwrap();
        state
            .virtual_boards
            .insert_test_device(virtual_id, mac)
            .await
            .unwrap();
        let board = json!({
            "id": "qemu-discovery-test",
            "board_type": "qemu-x86_64",
            "tags": [],
            "serial": {
                "key": { "kind": "qemu", "value": virtual_id },
                "baud_rate": 115200
            },
            "power_management": { "kind": "qemu", "virtual_device_id": virtual_id },
            "boot": { "kind": "httpboot" },
            "network_identity": { "mac_address": mac },
            "notes": null,
            "disabled": false
        });

        assert!(state.loader_registry.snapshots().await.is_empty());
        assert_eq!(create_board(&app, &board).await, StatusCode::CREATED);
        assert!(
            state
                .board_store
                .load_all()
                .await
                .unwrap()
                .contains_key("qemu-discovery-test")
        );

        let mut mismatched = board.clone();
        mismatched["id"] = json!("wrong-mac");
        mismatched["network_identity"]["mac_address"] = json!("02:00:00:00:00:43");
        assert_eq!(
            create_board(&app, &mismatched).await,
            StatusCode::BAD_REQUEST
        );
        mismatched["power_management"]["virtual_device_id"] = json!("missing");
        assert_eq!(
            create_board(&app, &mismatched).await,
            StatusCode::BAD_REQUEST
        );

        let response = app
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/boards/qemu-discovery-test")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(serde_json::to_vec(&board).unwrap()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        state.virtual_boards.shutdown().await;
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
        let boards: Vec<crate::api::models::AdminBoardSummary> =
            serde_json::from_slice(&boards_body).unwrap();
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
        assert_eq!(value["message"], "power-on completed");
    }

    #[tokio::test]
    async fn board_runtime_status_reports_idle_board() {
        let app = test_router().await;
        let board = sample_board("runtime-board");
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/boards/runtime-board/runtime-status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        let status: BoardRuntimeStatusResponse = serde_json::from_slice(&body).unwrap();
        assert_eq!(status.lease_state, BoardLeaseState::Idle);
        assert!(status.active_session_id.is_none());
        assert!(status.last_release_error.is_none());
    }

    #[tokio::test]
    async fn delete_session_returns_accepted_and_marks_session_releasing() {
        let app = test_router().await;
        let mut board = sample_board("release-board");
        board.serial = None;
        board.boot = BootConfig::Pxe(Default::default());
        board.power_management = PowerManagementConfig::Custom(CustomPowerManagement {
            power_on_cmd: "printf power-on >/dev/null".into(),
            power_off_cmd: "sh -c 'sleep 1'".into(),
        });
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );

        let session_id = create_session(&app, "rk3568").await;
        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/sessions/{session_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::ACCEPTED);

        let mut state = SessionLifecycleState::Active;
        for _ in 0..10 {
            let session_response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .uri(format!("/api/v1/sessions/{session_id}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(session_response.status(), StatusCode::OK);
            let session_body = to_bytes(session_response.into_body(), usize::MAX)
                .await
                .unwrap();
            let detail: SessionDetailResponse = serde_json::from_slice(&session_body).unwrap();
            state = detail.session.state;
            if state == SessionLifecycleState::Releasing {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(25)).await;
        }
        assert_eq!(state, SessionLifecycleState::Releasing);
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
                key: SerialPortKey {
                    kind: SerialPortKeyKind::UsbPath,
                    value: relay_port.clone(),
                },
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
        assert_eq!(value["message"], "power-off completed");

        let request = tokio::time::timeout(std::time::Duration::from_secs(1), requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(request, (1, 0, false));

        let _ = stop_tx.send(());
        let _ = server.await.unwrap();
    }

    #[tokio::test]
    #[cfg(unix)]
    async fn delete_session_keeps_zhongsheng_session_releasing_before_final_removal() {
        let app = test_router().await;
        let (relay_port, _relay_handle, server, _requests, stop_tx) = spawn_relay_test_server();
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let mut board = sample_board("relay-release-board");
        board.serial = None;
        board.boot = BootConfig::Pxe(Default::default());
        board.power_management =
            PowerManagementConfig::ZhongshengRelay(ZhongshengRelayPowerManagement {
                key: SerialPortKey {
                    kind: SerialPortKeyKind::UsbPath,
                    value: relay_port,
                },
            });
        assert_eq!(
            create_board(&app, serde_json::to_value(&board).unwrap()).await,
            StatusCode::CREATED
        );

        let session_id = create_session(&app, "rk3568").await;

        let delete_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/sessions/{session_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_response.status(), StatusCode::ACCEPTED);

        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let session_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{session_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(session_response.status(), StatusCode::OK);
        let session_body = to_bytes(session_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let detail: SessionDetailResponse = serde_json::from_slice(&session_body).unwrap();
        assert_eq!(detail.session.state, SessionLifecycleState::Releasing);

        let runtime_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/boards/relay-release-board/runtime-status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(runtime_response.status(), StatusCode::OK);
        let runtime_body = to_bytes(runtime_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let runtime: BoardRuntimeStatusResponse = serde_json::from_slice(&runtime_body).unwrap();
        assert_eq!(runtime.lease_state, BoardLeaseState::Releasing);
        assert_eq!(
            runtime.active_session_id.as_deref(),
            Some(session_id.as_str())
        );

        let zhongsheng_release_settle_delay = std::time::Duration::from_secs(10);
        tokio::time::sleep(zhongsheng_release_settle_delay + std::time::Duration::from_millis(300))
            .await;

        let session_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{session_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(session_response.status(), StatusCode::NOT_FOUND);

        let runtime_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/v1/admin/boards/relay-release-board/runtime-status")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(runtime_response.status(), StatusCode::OK);
        let runtime_body = to_bytes(runtime_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let runtime: BoardRuntimeStatusResponse = serde_json::from_slice(&runtime_body).unwrap();
        assert_eq!(runtime.lease_state, BoardLeaseState::Idle);
        assert!(runtime.active_session_id.is_none());

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
    async fn create_board_rejects_invalid_static_uboot_network_config() {
        let app = test_router().await;

        for boot in [
            json!({ "kind": "uboot", "use_tftp": true, "network_mode": "static_ip" }),
            json!({
                "kind": "uboot",
                "use_tftp": true,
                "network_mode": "static_ip",
                "board_ip": "not-an-ip"
            }),
            json!({
                "kind": "uboot",
                "use_tftp": true,
                "network_mode": "static_ip",
                "board_ip": "192.168.10.20",
                "server_ip": "not-an-ip"
            }),
            json!({
                "kind": "uboot",
                "use_tftp": true,
                "network_mode": "static_ip",
                "board_ip": "192.168.10.20",
                "netmask": "not-an-ip"
            }),
            json!({
                "kind": "uboot",
                "use_tftp": true,
                "network_mode": "static_ip",
                "board_ip": "192.168.10.20",
                "gatewayip": "not-an-ip"
            }),
        ] {
            let status = create_board(
                &app,
                json!({
                    "id": null,
                    "board_type": "rk3568",
                    "tags": [],
                    "serial": null,
                    "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
                    "boot": boot,
                    "notes": null,
                    "disabled": false
                }),
            )
            .await;

            assert_eq!(status, StatusCode::BAD_REQUEST);
        }
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
    async fn admin_dtb_endpoints_enforce_upload_limit() {
        let app = test_router().await;
        let oversized = vec![b'x'; mib_to_bytes(DTB_UPLOAD_MAX_MIB) + 1];

        let create_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/dtbs")
                    .header("X-Dtb-Name", "oversized.dtb")
                    .body(Body::from(oversized.clone()))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(create_response.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let create_body = to_bytes(create_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let create_value: serde_json::Value = serde_json::from_slice(&create_body).unwrap();
        assert_eq!(create_value["code"], "payload_too_large");
        assert_eq!(
            create_value["message"],
            format!("DTB upload body exceeds limit of {DTB_UPLOAD_MAX_MIB} MiB")
        );

        assert_eq!(
            upload_dtb(&app, "replace-me.dtb", "dtb-v1").await,
            StatusCode::CREATED
        );
        let replace_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/dtbs/replace-me.dtb")
                    .body(Body::from(oversized))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(replace_response.status(), StatusCode::PAYLOAD_TOO_LARGE);
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
                    "disabled": true
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
            BootConfig::UefiHttp(_) => panic!("expected uboot"),
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
                    "serial": {
                        "key": {
                            "kind": "usb_path",
                            "value": "/dev/serial/by-path/pci-0000:00:14.0-usb-0:10.4:1.0-port0"
                        },
                        "baud_rate": 115200
                    },
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
            "serial": {
                "key": {
                    "kind": "usb_path",
                    "value": "/dev/serial/by-path/pci-0000:00:14.0-usb-0:10.4:1.0-port0"
                },
                "baud_rate": 115200
            },
            "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
            "boot": { "kind": "uboot", "use_tftp": false },
            "notes": null,
            "disabled": false
        });
        let board_b = json!({
            "id": "rk3568-02",
            "board_type": "rk3568",
            "tags": ["lab-b"],
            "serial": {
                "key": {
                    "kind": "serial_number",
                    "value": "BG02M9TR"
                },
                "baud_rate": 115200
            },
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
    async fn http_boot_upload_file_and_public_download_use_session_file_path() {
        let app = test_router().await;
        assert_eq!(
            create_board(
                &app,
                serde_json::to_value(sample_httpboot_board("uefi-http-01")).unwrap()
            )
            .await,
            StatusCode::CREATED
        );
        let session_id = create_session(&app, "x86_64-uefi-http").await;

        let upload = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{session_id}/http-boot/files"))
                    .header(
                        DEVICE_PROTOCOL_VERSION_HEADER,
                        DEVICE_PROTOCOL_VERSION.to_string(),
                    )
                    .header("X-File-Path", "kernel.bin")
                    .body(Body::from("kernel-image"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::CREATED);
        let upload_body = to_bytes(upload.into_body(), usize::MAX).await.unwrap();
        let uploaded: serde_json::Value = serde_json::from_slice(&upload_body).unwrap();
        assert_eq!(uploaded["filename"], "kernel.bin");
        assert_eq!(
            uploaded["relative_path"],
            format!("ostool/sessions/{session_id}/kernel.bin")
        );
        assert_eq!(
            uploaded["http_url"],
            format!("http://127.0.0.1:2999/boot/sessions/{session_id}/kernel.bin")
        );

        let download_kernel = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/boot/sessions/{session_id}/kernel.bin"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(download_kernel.status(), StatusCode::OK);
        assert_eq!(download_kernel.headers()[header::CONTENT_LENGTH], "12");
        let kernel_body = to_bytes(download_kernel.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(kernel_body.as_ref(), b"kernel-image");

        let range_kernel = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/boot/sessions/{session_id}/kernel.bin"))
                    .header(header::RANGE, "bytes=0-5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(range_kernel.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(
            range_kernel.headers()[header::CONTENT_RANGE],
            "bytes 0-5/12"
        );
        let range_body = to_bytes(range_kernel.into_body(), usize::MAX)
            .await
            .unwrap();
        assert_eq!(range_body.as_ref(), b"kernel");

        let delete_session = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/sessions/{session_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(delete_session.status(), StatusCode::ACCEPTED);

        let download_after_release = app
            .oneshot(
                Request::builder()
                    .uri(format!("/boot/sessions/{session_id}/kernel.bin"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        // The asynchronous release may finish while its HTTP audit result is persisted.
        assert!(matches!(
            download_after_release.status(),
            StatusCode::CONFLICT | StatusCode::NOT_FOUND
        ));
    }

    #[tokio::test]
    async fn httpboot_payload_matrix_accepts_opaque_future_arch_bytes() {
        for (with_initramfs, with_cmdline) in
            [(false, false), (false, true), (true, false), (true, true)]
        {
            let (app, state) = test_router_and_state_with_config(|_| {}).await;
            let board_id = format!("payload-{with_initramfs}-{with_cmdline}");
            assert_eq!(
                create_board(
                    &app,
                    serde_json::to_value(sample_httpboot_board(&board_id)).unwrap()
                )
                .await,
                StatusCode::CREATED
            );
            let session_id = create_session(&app, "x86_64-uefi-http").await;
            if with_initramfs {
                let response = app
                    .clone()
                    .oneshot(
                        Request::builder()
                            .method("PUT")
                            .uri(format!("/api/v1/sessions/{session_id}/http-boot/files"))
                            .header(
                                DEVICE_PROTOCOL_VERSION_HEADER,
                                DEVICE_PROTOCOL_VERSION.to_string(),
                            )
                            .header("X-File-Path", "initramfs.cpio")
                            .body(Body::from("future initramfs"))
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                assert_eq!(response.status(), StatusCode::CREATED);
            }
            let mut request = Request::builder()
                .method("PUT")
                .uri(format!("/api/v1/sessions/{session_id}/http-boot/kernel"))
                .header(
                    DEVICE_PROTOCOL_VERSION_HEADER,
                    DEVICE_PROTOCOL_VERSION.to_string(),
                )
                .header("X-HttpBoot-Remote-Name", "future.image");
            if with_initramfs {
                request = request.header("X-HttpBoot-Initramfs-Path", "initramfs.cpio");
            }
            if with_cmdline {
                request = request.header("X-HttpBoot-Cmdline", "console=ttyS0");
            }
            let response = app
                .clone()
                .oneshot(request.body(Body::from("not an ELF; future ISA")).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::CREATED);
            let session = state.session_state(&session_id).await.unwrap();
            let command = session.boot_command().await.unwrap();
            assert_eq!(command.initramfs.is_some(), with_initramfs);
            assert_eq!(
                command.cmdline.as_deref(),
                with_cmdline.then_some("console=ttyS0")
            );
            assert_eq!(
                command.kernel_path,
                format!("/boot/sessions/{session_id}/future.image")
            );
            let stored = state
                .tftp_manager
                .read()
                .await
                .get_session_file(&session_id, "future.image")
                .await
                .unwrap()
                .unwrap();
            assert_eq!(
                tokio::fs::read(stored.disk_path).await.unwrap(),
                b"not an ELF; future ISA"
            );
        }
    }

    #[tokio::test]
    async fn httpboot_version_gate_precedes_body_and_file_side_effects() {
        let (app, state) = test_router_and_state_with_config(|_| {}).await;
        assert_eq!(
            create_board(
                &app,
                serde_json::to_value(sample_httpboot_board("version-gate")).unwrap()
            )
            .await,
            StatusCode::CREATED
        );
        let session_id = create_session(&app, "x86_64-uefi-http").await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{session_id}/http-boot/kernel"))
                    .header("X-HttpBoot-Remote-Name", "kernel.elf")
                    .body(Body::from("must not be read"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&body).unwrap(),
            json!({
                "code": "unsupported_protocol",
                "message": "axloader protocol 7 is required"
            })
        );
        let session = state.session_state(&session_id).await.unwrap();
        assert!(session.boot_command().await.is_none());
        assert!(!session.is_stop_requested());
        assert!(
            state
                .tftp_manager
                .read()
                .await
                .list_session_files(&session_id)
                .await
                .unwrap()
                .is_empty()
        );
    }

    #[tokio::test]
    async fn obsolete_loader_routes_remain_forbidden() {
        let app = test_router().await;
        for (method, uri) in [
            ("POST", "/api/v1/loaders/poll"),
            ("POST", "/api/v1/loaders/status"),
            ("POST", "/api/v1/loaders/ota-status"),
            (
                "GET",
                "/api/v1/loader-updates/01234567-89ab-cdef-0123-456789abcdef/image",
            ),
        ] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method(method)
                        .uri(uri)
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::FORBIDDEN, "{method} {uri}");
        }
    }

    #[tokio::test]
    async fn http_boot_upload_rejects_non_httpboot_board() {
        let app = test_router().await;
        assert_eq!(
            create_board(&app, serde_json::to_value(sample_board("pxe-01")).unwrap()).await,
            StatusCode::CREATED
        );
        let session_id = create_session(&app, "rk3568").await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{session_id}/http-boot/files"))
                    .header(
                        DEVICE_PROTOCOL_VERSION_HEADER,
                        DEVICE_PROTOCOL_VERSION.to_string(),
                    )
                    .header("X-File-Path", "kernel.bin")
                    .body(Body::from("kernel-image"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn local_tftp_uploads_return_download_urls_without_exposing_them_publicly() {
        let (app, state) = test_router_and_state_with_config(|config| {
            if let TftpConfig::Builtin(tftp) = &mut config.tftp {
                tftp.enabled = true;
                tftp.bind_addr = "127.0.0.1:1069".parse().unwrap();
            }
        })
        .await;
        assert_eq!(
            create_board(&app, sample_board("local-tftp")).await,
            StatusCode::CREATED
        );
        let sid = create_session(&app, "rk3568").await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{sid}/files"))
                    .header("x-file-path", "kernel.fit")
                    .body(Body::from("fixture FIT"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let payload: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(
            payload["tftp_url"],
            format!("tftp://127.0.0.1:1069/ostool/sessions/{sid}/kernel.fit")
        );
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{sid}/tftp"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let payload: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(payload["server_ip"], "127.0.0.1");
        let manager = state.tftp_manager.read().await.clone();
        let file = manager
            .get_session_file(&sid, "kernel.fit")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            tokio::fs::read(&file.disk_path).await.unwrap(),
            b"fixture FIT"
        );
        // Even a changed in-memory edition must never disclose TFTP URLs publicly.
        state.config.write().await.edition = crate::config::Edition::Public;
        assert!(
            super::file_response(&state, file)
                .await
                .unwrap()
                .tftp_url
                .is_none()
        );
        state.shutdown_sessions().await.unwrap();
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
        assert!(uploaded["tftp_url"].is_null());

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
    async fn shared_session_http_works_without_tftp_and_expires_with_session() {
        let app = test_router_with_config(|config| {
            let TftpConfig::Builtin(tftp) = &mut config.tftp else {
                panic!("expected builtin TFTP test configuration");
            };
            tftp.enabled = false;
        })
        .await;
        let mut board = sample_board("shared-http");
        let BootConfig::Uboot(profile) = &mut board.boot else {
            panic!("expected U-Boot board");
        };
        profile.use_tftp = false;
        assert_eq!(create_board(&app, board).await, StatusCode::CREATED);
        let session_id = create_session(&app, "rk3568").await;

        let profile_response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{session_id}/boot-profile"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(profile_response.status(), StatusCode::OK);
        let profile_body = to_bytes(profile_response.into_body(), usize::MAX)
            .await
            .unwrap();
        let profile: BootProfileResponse = serde_json::from_slice(&profile_body).unwrap();
        assert_eq!(profile.server_ip, None);
        assert_eq!(
            profile.http_base_url.unwrap().as_str(),
            "http://127.0.0.1:2999/"
        );

        let relative_path = "tools/network/probe script.sh";
        let upload = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{session_id}/files"))
                    .header("X-File-Path", relative_path)
                    .body(Body::from("0123456789"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(upload.status(), StatusCode::CREATED);
        let upload_body = to_bytes(upload.into_body(), usize::MAX).await.unwrap();
        let uploaded: SharedSessionFileResponse = serde_json::from_slice(&upload_body).unwrap();
        assert_eq!(uploaded.filename, "probe script.sh");
        assert_eq!(
            uploaded.relative_path,
            format!("ostool/sessions/{session_id}/{relative_path}")
        );
        assert_eq!(uploaded.tftp_url, None);
        let http_url = uploaded.http_url.unwrap();
        assert_eq!(
            http_url.as_str(),
            format!(
                "http://127.0.0.1:2999/share/sessions/{session_id}/tools/network/probe%20script.sh"
            )
        );

        let download_path = format!("/share/sessions/{session_id}/tools/network/probe%20script.sh");
        let download = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&download_path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(download.status(), StatusCode::OK);
        let body = to_bytes(download.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), b"0123456789");

        let range = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(&download_path)
                    .header(header::RANGE, "bytes=2-5")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(range.status(), StatusCode::PARTIAL_CONTENT);
        assert_eq!(range.headers()[header::CONTENT_RANGE], "bytes 2-5/10");
        let body = to_bytes(range.into_body(), usize::MAX).await.unwrap();
        assert_eq!(body.as_ref(), b"2345");

        let escape = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{session_id}/files"))
                    .header("X-File-Path", "../escape.sh")
                    .body(Body::from("escape"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(escape.status(), StatusCode::BAD_REQUEST);

        let release = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri(format!("/api/v1/sessions/{session_id}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(release.status(), StatusCode::ACCEPTED);

        let expired = app
            .oneshot(
                Request::builder()
                    .uri(download_path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(expired.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn boot_profile_returns_static_uboot_network_with_resolved_fallbacks() {
        let app = test_router().await;
        assert_eq!(
            create_board(
                &app,
                json!({
                    "id": "static-profile",
                    "board_type": "rk3568",
                    "tags": [],
                    "serial": {"key":{"kind":"serial_number","value":"profile-console"},"baud_rate":115200},
                    "power_management": { "kind": "custom", "power_on_cmd": "echo on", "power_off_cmd": "echo off" },
                    "boot": {
                        "kind": "uboot",
                        "use_tftp": true,
                        "network_mode": "static_ip",
                        "board_ip": "192.168.10.20",
                        "server_ip": "192.168.10.2",
                        "netmask": "255.255.255.0",
                        "gatewayip": "192.168.10.1"
                    },
                    "notes": null,
                    "disabled": false
                }),
            )
            .await,
            StatusCode::CREATED
        );
        let session_id = create_session(&app, "rk3568").await;

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri(format!("/api/v1/sessions/{session_id}/boot-profile"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(
            status,
            StatusCode::OK,
            "response body: {}",
            String::from_utf8_lossy(&body)
        );
        let value: serde_json::Value = serde_json::from_slice(&body).unwrap();

        assert_eq!(value["boot"]["network_mode"], "dhcp");
        for field in ["board_ip", "server_ip", "netmask", "gatewayip"] {
            assert!(value["boot"][field].is_null());
        }
        assert!(value["server_ip"].is_null());
        assert!(value["netmask"].is_null());
    }

    #[test]
    fn normalize_board_upsert_request_trims_uboot_load_addresses() {
        let request = AdminBoardUpsertRequest {
            allow_instant_use: false,
            reset_command: None,
            id: Some("demo".into()),
            board_type: "demo".into(),
            tags: vec![],
            network_identity: None,
            notes: None,
            disabled: false,
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "echo on".into(),
                power_off_cmd: "echo off".into(),
            }),
            boot: BootConfig::Uboot(crate::config::UbootProfile {
                kernel_load_addr: Some(" 0x80200000 ".into()),
                fit_load_addr: Some(" ".into()),
                bootm_addr: Some(" 0x82200000 ".into()),
                ..Default::default()
            }),
        };

        let request = super::normalize_board_upsert_request(request).unwrap();
        let BootConfig::Uboot(profile) = request.boot else {
            panic!("expected uboot profile");
        };

        assert_eq!(profile.kernel_load_addr.as_deref(), Some("0x80200000"));
        assert_eq!(profile.fit_load_addr, None);
        assert_eq!(profile.bootm_addr.as_deref(), Some("0x82200000"));
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
    async fn session_file_upload_enforces_dynamic_limit() {
        let app = test_router().await;
        assert_eq!(
            create_board(
                &app,
                serde_json::to_value(sample_board("limited-files")).unwrap()
            )
            .await,
            StatusCode::CREATED
        );
        let session_id = create_session(&app, "rk3568").await;

        let initial_upload = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{session_id}/files"))
                    .header("X-File-Path", "boot/Image")
                    .body(Body::from(vec![b'a'; 1024 * 1024]))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(initial_upload.status(), StatusCode::CREATED);

        let update_limit = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri("/api/v1/admin/server-config")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"network":{"interface":"lo"},"upload_limits":{"session_file_max_mib":1}}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(update_limit.status(), StatusCode::OK);

        let oversized = vec![b'b'; 1024 * 1024 + 1];
        let oversized_upload = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("PUT")
                    .uri(format!("/api/v1/sessions/{session_id}/files"))
                    .header("X-File-Path", "boot/Image")
                    .body(Body::from(oversized))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(oversized_upload.status(), StatusCode::PAYLOAD_TOO_LARGE);
        let oversized_body = to_bytes(oversized_upload.into_body(), usize::MAX)
            .await
            .unwrap();
        let oversized_value: serde_json::Value = serde_json::from_slice(&oversized_body).unwrap();
        assert_eq!(oversized_value["code"], "payload_too_large");
        assert_eq!(
            oversized_value["message"],
            "session file upload body exceeds limit of 1 MiB"
        );
    }

    #[tokio::test]
    async fn nested_upload_method_is_not_supported() {
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
        assert_eq!(response.status(), StatusCode::METHOD_NOT_ALLOWED);
    }

    #[tokio::test]
    async fn create_session_returns_created_when_board_is_available() {
        let app = test_router().await;
        let board = json!({
            "id": "demo-01",
            "board_type": "demo",
            "tags": [],
            "serial": {
                "key": {
                    "kind": "usb_path",
                    "value": "/dev/serial/by-path/pci-0000:00:14.0-usb-0:10.4:1.0-port0"
                },
                "baud_rate": 115200
            },
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
    async fn create_session_allocates_requested_board_id() {
        let app = test_router().await;
        for board_id in ["demo-01", "demo-02"] {
            let mut board = sample_board(board_id);
            board.board_type = "demo".to_string();
            assert_eq!(
                create_board(&app, serde_json::to_value(board).unwrap()).await,
                StatusCode::CREATED
            );
        }

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
                            "board_id": " demo-02 ",
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
        assert_eq!(value["board_id"], "demo-02");
    }

    #[tokio::test]
    async fn create_session_rejects_missing_requested_board_id() {
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
                            "board_type": "demo",
                            "board_id": "missing-demo-01",
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
        assert_eq!(value["message"], "board `missing-demo-01` not found");
    }

    #[tokio::test]
    async fn create_session_returns_conflict_without_waiting_when_pool_is_busy() {
        let app = test_router().await;
        let board = json!({
            "id": "demo-01",
            "board_type": "demo",
            "tags": [],
            "serial": {
                "key": {
                    "kind": "usb_path",
                    "value": "/dev/serial/by-path/pci-0000:00:14.0-usb-0:10.4:1.0-port0"
                },
                "baud_rate": 115200
            },
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

    #[tokio::test]
    async fn loader_image_routes_validate_size_and_delete_unreferenced_image() {
        let (app, _) = test_router_and_state_with_config(|_| {}).await;
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/loader-images")
                    .header(header::CONTENT_LENGTH, 0)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let error: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        assert_eq!(error["code"], "bad_request");
        assert_eq!(error["message"], "empty OTA image");

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/loader-images")
                    .header(header::CONTENT_LENGTH, crate::ota::MAX_IMAGE_BYTES + 1)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::PAYLOAD_TOO_LARGE);

        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("DELETE")
                    .uri("/api/v1/admin/loader-images/not-a-digest")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);

        for (declared_length, version) in [(1, "v1".to_owned()), (4, "x".repeat(97))] {
            let response = app
                .clone()
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/api/v1/admin/loader-images")
                        .header(header::CONTENT_LENGTH, declared_length)
                        .header("X-Image-Version", version)
                        .body(Body::from("data"))
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        }

        let image = b"opaque loader image".to_vec();
        let response = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v1/admin/loader-images")
                    .header(header::CONTENT_LENGTH, image.len())
                    .body(Body::from(image))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::CREATED);
        let uploaded: crate::ota::Image =
            serde_json::from_slice(&to_bytes(response.into_body(), usize::MAX).await.unwrap())
                .unwrap();
        let delete = || {
            Request::builder()
                .method("DELETE")
                .uri(format!("/api/v1/admin/loader-images/{}", uploaded.sha256))
                .body(Body::empty())
                .unwrap()
        };
        assert_eq!(
            app.clone().oneshot(delete()).await.unwrap().status(),
            StatusCode::NO_CONTENT
        );
        assert_eq!(
            app.oneshot(delete()).await.unwrap().status(),
            StatusCode::NOT_FOUND
        );
    }
}
