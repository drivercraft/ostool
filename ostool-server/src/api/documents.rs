//! Public device documents have their own administrator editing workflow.
use crate::{
    AppState,
    api::error::ApiError,
    persistence::documents::{DeviceDocument, DocumentView},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::get,
};
use serde::Deserialize;

pub fn routes() -> Router<AppState> {
    Router::new()
        .route("/api/v1/admin/device-documents/{id}", get(view).put(save))
        .route("/api/v1/site/device-documents/{id}", get(view))
}
pub async fn projection(state: &AppState, id: &str) -> Result<DocumentView, ApiError> {
    if !state.boards.read().await.contains_key(id) {
        return Err(ApiError::not_found("开发板不存在或已删除"));
    }
    Ok(state.database.device_document(id).await?)
}
async fn view(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<DocumentView>, ApiError> {
    Ok(Json(projection(&state, &id).await?))
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Save {
    revision: i64,
    document: DeviceDocument,
}
async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<Save>,
) -> Result<Json<DocumentView>, ApiError> {
    let _inventory = state.board_inventory_gate.lock().await;
    if !state.boards.read().await.contains_key(&id) {
        return Err(ApiError::not_found("开发板不存在或已删除"));
    }
    input
        .document
        .validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    if input.revision < 0
        || !state
            .database
            .save_device_document(&id, input.revision, &input.document)
            .await?
    {
        return Err(ApiError::conflict("文档已更新，请保留草稿后载入最新版本"));
    }
    state.admin_events.invalidate(&["device_documents"]);
    Ok(Json(projection(&state, &id).await?))
}
