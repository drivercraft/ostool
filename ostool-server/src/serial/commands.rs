//! Server-side expansion of board shell commands; callers receive opaque IDs only.
use crate::{AppState, api::error::ApiError, session::SessionState};
use axum::{
    Json, Router,
    extract::{Path, State},
    routing::post,
};
use uuid::Uuid;

pub fn routes() -> Router<AppState> {
    Router::new().route("/api/v1/sessions/{session_id}/commands", post(register))
}
async fn register(
    Path(id): Path<String>,
    State(state): State<AppState>,
    Json(templates): Json<Vec<String>>,
) -> Result<Json<Vec<Uuid>>, ApiError> {
    if templates.len() > 64
        || templates.iter().any(|s| s.len() > 8192 || s.contains('\0'))
        || templates.iter().map(String::len).sum::<usize>() > 65536
    {
        return Err(ApiError::bad_request("命令数量或长度超限"));
    }
    let session = state
        .session_state(&id)
        .await
        .ok_or_else(|| ApiError::not_found("session not found"))?;
    if session.is_stop_requested() || session.snapshot().await.expires_at <= chrono::Utc::now() {
        return Err(ApiError::conflict("会话已结束"));
    }
    for command in &templates {
        expand(&state, &session, command)
            .await
            .map_err(|_| ApiError::bad_request("共享文件或命令变量无效"))?;
    }
    let mut commands = session.shell_commands.write().await;
    if commands.len() + templates.len() > 64
        || commands.values().map(String::len).sum::<usize>()
            + templates.iter().map(String::len).sum::<usize>()
            > 65536
    {
        return Err(ApiError::conflict("会话命令存储已满"));
    }
    let mut ids = Vec::with_capacity(templates.len());
    for command in templates {
        let id = Uuid::new_v4();
        commands.insert(id, command);
        ids.push(id);
    }
    Ok(Json(ids))
}
pub(super) async fn resolve(
    state: &AppState,
    session: &SessionState,
    id: Uuid,
) -> anyhow::Result<Vec<u8>> {
    anyhow::ensure!(
        !session.is_stop_requested() && session.snapshot().await.expires_at > chrono::Utc::now(),
        "session ended"
    );
    let command = session
        .shell_commands
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("unknown session command"))?;
    Ok(expand(state, session, &command).await?.into_bytes())
}
pub(super) async fn expand(
    state: &AppState,
    session: &SessionState,
    command: &str,
) -> anyhow::Result<String> {
    let mut result = String::new();
    let mut remaining = command;
    while let Some((head, tail)) = remaining.split_once("${") {
        result.push_str(head);
        let Some((name, rest)) = tail.split_once('}') else {
            result.push_str("${");
            result.push_str(tail);
            return Ok(result);
        };
        if name == "boardServerIp"
            || name == "boardServerHttpBaseUrl"
            || name.starts_with("sessionFile:")
        {
            let config = state.config.read().await.clone();
            let base = crate::hardware::download_base_url(&config)?;
            if name == "boardServerIp" {
                result.push_str(
                    base.host_str()
                        .ok_or_else(|| anyhow::anyhow!("missing device network"))?,
                );
            } else if name == "boardServerHttpBaseUrl" {
                result.push_str(base.as_str());
            } else {
                let path = crate::tftp::files::normalize_relative_path(
                    name.trim_start_matches("sessionFile:"),
                )?;
                let id = session.snapshot().await.id;
                anyhow::ensure!(
                    state
                        .tftp_manager
                        .read()
                        .await
                        .get_session_file(&id, &path)
                        .await?
                        .is_some(),
                    "session file missing"
                );
                let mut url = base;
                url.path_segments_mut()
                    .map_err(|_| anyhow::anyhow!("invalid device URL"))?
                    .pop_if_empty()
                    .extend(["share", "sessions", id.as_str()])
                    .extend(path.split('/'));
                result.push_str(url.as_str());
            }
        } else {
            anyhow::ensure!(
                !name.starts_with("boardServer") && !name.starts_with("sessionFile"),
                "unknown session placeholder"
            );
            result.push_str("${");
            result.push_str(name);
            result.push('}');
        }
        remaining = rest;
    }
    result.push_str(remaining);
    Ok(result)
}
