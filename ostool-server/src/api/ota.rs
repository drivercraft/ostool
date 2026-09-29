//! Management and device endpoints for explicit axloader upgrades.

use axum::{
    Json, Router,
    body::{Body, to_bytes},
    extract::{Path, State},
    http::{Request, StatusCode, header},
    routing::{get, post},
};
use httpboot_protocol::{LoaderStatusPhase, LoaderStatusReport, MacAddress, PROTOCOL_VERSION};
use serde::Deserialize;

use crate::{
    api::{error::ApiError, router::board_id_for_mac},
    config::{BootConfig, UefiBootArch},
    ota::{DeleteImageError, Image, Job, MAX_IMAGE_BYTES, Phase},
    state::{AppState, BoardLeaseState},
};

pub fn routes() -> Router<AppState> {
    Router::new()
        .route(
            "/api/v1/admin/loader-images",
            get(list_images).post(upload_image),
        )
        .route(
            "/api/v1/admin/loader-images/{sha256}",
            axum::routing::delete(delete_image),
        )
        .route(
            "/api/v1/admin/boards/{board_id}/loader-updates",
            get(get_job).post(queue_job),
        )
        .route(
            "/api/v1/admin/boards/{board_id}/loader-updates/{update_id}",
            axum::routing::delete(cancel_job),
        )
        .route(
            "/api/v1/loader-updates/{update_id}/image",
            get(download_image),
        )
        .route("/api/v1/loaders/ota-status", post(report_status))
}

async fn list_images(State(state): State<AppState>) -> Result<Json<Vec<Image>>, ApiError> {
    Ok(Json(state.ota.images()?))
}

async fn upload_image(
    State(state): State<AppState>,
    request: Request<Body>,
) -> Result<(StatusCode, Json<Image>), ApiError> {
    let length = request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<usize>().ok())
        .ok_or_else(|| ApiError::bad_request("Content-Length required"))?;
    if length == 0 {
        return Err(ApiError::bad_request("empty EFI image"));
    }
    if length > MAX_IMAGE_BYTES {
        return Err(ApiError::payload_too_large("EFI image exceeds 32 MiB"));
    }
    let version = request
        .headers()
        .get("X-Image-Version")
        .map(|value| value.to_str().map(str::to_owned))
        .transpose()
        .map_err(|_| ApiError::bad_request("invalid image version"))?;
    let body = to_bytes(request.into_body(), MAX_IMAGE_BYTES + 1)
        .await
        .map_err(|_| ApiError::payload_too_large("EFI image exceeds 32 MiB"))?;
    if body.len() != length {
        return Err(ApiError::bad_request("incorrect Content-Length"));
    }
    let image = state
        .ota
        .put_image(&body, version)
        .map_err(|error| ApiError::bad_request(format!("{error:#}")))?;
    state.admin_events.invalidate(&["ota"]);
    Ok((StatusCode::CREATED, Json(image)))
}

async fn delete_image(
    Path(sha256): Path<String>,
    State(state): State<AppState>,
) -> Result<StatusCode, ApiError> {
    state
        .ota
        .delete_image(&sha256)
        .await
        .map_err(|error| match error {
            DeleteImageError::InvalidDigest => ApiError::bad_request(error.to_string()),
            DeleteImageError::NotFound => ApiError::not_found(error.to_string()),
            DeleteImageError::InUse => ApiError::conflict(error.to_string()),
            DeleteImageError::Inconsistent | DeleteImageError::Io(_) => {
                ApiError::internal(error.to_string())
            }
        })?;
    state.admin_events.invalidate(&["ota"]);
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct QueueRequest {
    image_sha256: String,
}

async fn queue_job(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
    Json(request): Json<QueueRequest>,
) -> Result<(StatusCode, Json<Job>), ApiError> {
    let boards = state.boards.read().await;
    let board = boards
        .get(&board_id)
        .ok_or_else(|| ApiError::not_found("board not found"))?;
    if !matches!(
        &board.boot,
        BootConfig::UefiHttp(profile)
            if profile.boot_arch.as_ref() == Some(&UefiBootArch::X86_64)
    ) {
        return Err(ApiError::conflict(
            "loader OTA requires an x86_64 UEFI HTTP board",
        ));
    }
    let mac = board
        .network_identity
        .as_ref()
        .ok_or_else(|| ApiError::conflict("board lacks a configured MAC"))?
        .mac_address;
    let runtime = state
        .board_runtime_status(&board_id)
        .await
        .ok_or_else(|| ApiError::not_found("board runtime not found"))?;
    if runtime.active_session_id.is_some() || runtime.lease_state != BoardLeaseState::Idle {
        return Err(ApiError::conflict("board has an active session"));
    }
    let job = state
        .ota
        .queue(board_id, mac, &request.image_sha256)
        .await
        .map_err(|error| ApiError::conflict(format!("{error:#}")))?;
    drop(boards);
    state.admin_events.invalidate(&["ota"]);
    Ok((StatusCode::CREATED, Json(job)))
}

async fn get_job(
    Path(board_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Json<Option<Job>>, ApiError> {
    if !state.boards.read().await.contains_key(&board_id) {
        return Err(ApiError::not_found("board not found"));
    }
    Ok(Json(state.ota.job(&board_id).await))
}

async fn cancel_job(
    Path((board_id, update_id)): Path<(String, String)>,
    State(state): State<AppState>,
) -> Result<Json<Job>, ApiError> {
    let job = state
        .ota
        .cancel(&board_id, &update_id)
        .await
        .map_err(|error| ApiError::conflict(format!("{error:#}")))?;
    state.admin_events.invalidate(&["ota"]);
    Ok(Json(job))
}

async fn download_image(
    Path(update_id): Path<String>,
    State(state): State<AppState>,
) -> Result<Body, ApiError> {
    let job = state
        .ota
        .jobs()
        .await
        .into_iter()
        .find(|job| job.update_id == update_id && !job.phase.is_terminal())
        .ok_or_else(|| ApiError::not_found("OTA assignment not found"))?;
    let boards = state.boards.read().await;
    if board_id_for_mac(&boards, job.mac_address).as_deref() != Some(job.board_id.as_str()) {
        return Err(ApiError::conflict("board MAC assignment has changed"));
    }
    let bytes = tokio::fs::read(state.ota.image_path(&job.image.sha256)?)
        .await
        .map_err(|error| {
            if error.kind() == std::io::ErrorKind::NotFound {
                ApiError::not_found("OTA image not found")
            } else {
                ApiError::internal(format!("failed to read OTA image: {error}"))
            }
        })?;
    if bytes.len() as u64 != job.image.size {
        return Err(ApiError::internal("stored image length changed"));
    }
    Ok(Body::from(bytes))
}

#[derive(Deserialize)]
struct Report {
    protocol_version: u16,
    registration_id: String,
    mac_address: MacAddress,
    update_id: String,
    phase: Phase,
    error: Option<String>,
    active_sha256: Option<String>,
}

async fn report_status(
    State(state): State<AppState>,
    Json(report): Json<Report>,
) -> Result<StatusCode, ApiError> {
    if report.protocol_version != PROTOCOL_VERSION {
        return Err(ApiError::bad_request("OTA requires v4"));
    }
    state
        .loader_registry
        .accept_status(&LoaderStatusReport {
            protocol_version: report.protocol_version,
            registration_id: report.registration_id,
            mac_address: report.mac_address,
            session_id: String::new(),
            boot_id: String::new(),
            status: LoaderStatusPhase::Accepted,
        })
        .await
        .map_err(|error| ApiError::conflict(format!("invalid loader registration: {error:?}")))?;
    let boards = state.boards.read().await;
    let board_id = board_id_for_mac(&boards, report.mac_address)
        .ok_or_else(|| ApiError::conflict("loader MAC is not bound"))?;
    state
        .ota
        .report(
            &board_id,
            report.mac_address,
            &report.update_id,
            report.phase,
            report.error,
            report.active_sha256.as_deref(),
        )
        .await
        .map_err(|error| ApiError::conflict(format!("{error:#}")))?;
    drop(boards);
    state.admin_events.invalidate(&["ota"]);
    Ok(StatusCode::NO_CONTENT)
}
