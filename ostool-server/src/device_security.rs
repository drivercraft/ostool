//! Provisioned HTTPS device identities. Discovery never chooses the callback origin.
use crate::{AppState, api::error::ApiError, security::crypto};
use anyhow::{Context, ensure};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::StatusCode,
    routing::get,
};
use serde::{Deserialize, Serialize};
use sqlx::Row;

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct ConnectionView {
    pub endpoint: String,
    pub ca_pem: Option<String>,
    pub configured: bool,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionInput {
    pub endpoint: String,
    pub token: Option<String>,
    pub ca_pem: Option<String>,
}
pub struct DeviceConnection {
    pub endpoint: String,
    pub client: reqwest::Client,
}

pub fn routes() -> Router<AppState> {
    Router::new().route(
        "/api/v1/admin/boards/{id}/device-connection",
        get(view).put(save).delete(remove),
    )
}
pub async fn projection(state: &AppState, id: &str) -> anyhow::Result<Option<ConnectionView>> {
    let row = sqlx::query("SELECT endpoint,ca_pem FROM device_connections WHERE board_id=$1")
        .bind(id)
        .fetch_optional(state.database.pool())
        .await?;
    row.map(|row| {
        Ok(ConnectionView {
            endpoint: row.try_get("endpoint")?,
            ca_pem: row.try_get("ca_pem")?,
            configured: true,
        })
    })
    .transpose()
}
async fn view(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Option<ConnectionView>>, ApiError> {
    if !state.boards.read().await.contains_key(&id) {
        return Err(ApiError::not_found("开发板不存在"));
    }
    Ok(Json(projection(&state, &id).await?))
}
fn validate_endpoint(value: &str) -> anyhow::Result<url::Url> {
    let url = url::Url::parse(value)?;
    ensure!(
        url.scheme() == "https"
            && url.host_str().is_some()
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && url.path() == "/",
        "设备地址必须是 HTTPS 来源地址，不能包含路径、凭据或查询参数"
    );
    Ok(url)
}
async fn save(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Json(input): Json<ConnectionInput>,
) -> Result<Json<ConnectionView>, ApiError> {
    let _inventory = state.board_inventory_gate.lock().await;
    let board = state
        .boards
        .read()
        .await
        .get(&id)
        .cloned()
        .ok_or_else(|| ApiError::not_found("开发板不存在"))?;
    if !matches!(board.boot, crate::BootConfig::UefiHttp(_)) {
        return Err(ApiError::bad_request("此设备不使用网络装载器"));
    }
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM device_connections WHERE board_id<>$1")
            .bind(&id)
            .fetch_one(state.database.pool())
            .await?;
    if count >= 4096 {
        return Err(ApiError::conflict("设备认证配置数量已达上限"));
    }
    let endpoint = validate_endpoint(&input.endpoint)
        .map_err(|e| ApiError::bad_request(e.to_string()))?
        .origin()
        .ascii_serialization();
    if input
        .ca_pem
        .as_ref()
        .is_some_and(|value| value.len() > 65536)
    {
        return Err(ApiError::bad_request("设备证书过大"));
    }
    let ca_pem = input.ca_pem.filter(|value| !value.trim().is_empty());
    if let Some(ca) = &ca_pem {
        reqwest::Certificate::from_pem(ca.as_bytes())
            .map_err(|_| ApiError::bad_request("设备 CA 证书无效"))?;
    }
    if state
        .board_runtime_status(&id)
        .await
        .is_some_and(|r| r.lease_state != crate::BoardLeaseState::Idle)
    {
        return Err(ApiError::conflict("请先释放设备再修改认证配置"));
    }
    let ciphertext = if let Some(token) = input.token.filter(|v| !v.is_empty()) {
        if token.len() < 32 || token.len() > 512 || !token.bytes().all(|b| b.is_ascii_graphic()) {
            return Err(ApiError::bad_request(
                "设备凭据须为 32–512 个可打印 ASCII 字符",
            ));
        }
        let config = state.config.read().await;
        let key = config
            .security
            .mfa_encryption_key
            .as_deref()
            .ok_or_else(|| ApiError::bad_request("请先配置部署主密钥 OSTOOL_MFA_ENCRYPTION_KEY"))?;
        crypto::seal(key, &format!("device:{id}"), token.as_bytes())?
    } else {
        sqlx::query_scalar::<_, String>(
            "SELECT token_ciphertext FROM device_connections WHERE board_id=$1",
        )
        .bind(&id)
        .fetch_optional(state.database.pool())
        .await?
        .ok_or_else(|| ApiError::bad_request("首次配置必须填写设备凭据"))?
    };
    sqlx::query("INSERT INTO device_connections(board_id,endpoint,ca_pem,token_ciphertext) VALUES($1,$2,$3,$4) ON CONFLICT(board_id) DO UPDATE SET endpoint=excluded.endpoint,ca_pem=excluded.ca_pem,token_ciphertext=excluded.token_ciphertext,updated_at=now()")
        .bind(&id).bind(&endpoint).bind(&ca_pem).bind(ciphertext).execute(state.database.pool()).await?;
    state.admin_events.invalidate(&["boards"]);
    Ok(Json(ConnectionView {
        endpoint,
        ca_pem,
        configured: true,
    }))
}
async fn remove(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let _inventory = state.board_inventory_gate.lock().await;
    if state
        .board_runtime_status(&id)
        .await
        .is_some_and(|r| r.lease_state != crate::BoardLeaseState::Idle)
    {
        return Err(ApiError::conflict("请先释放设备再移除认证配置"));
    }
    sqlx::query("DELETE FROM device_connections WHERE board_id=$1")
        .bind(id)
        .execute(state.database.pool())
        .await?;
    state.admin_events.invalidate(&["boards"]);
    Ok(StatusCode::NO_CONTENT)
}
pub async fn connection(state: &AppState, id: &str) -> anyhow::Result<DeviceConnection> {
    let row = sqlx::query(
        "SELECT endpoint,ca_pem,token_ciphertext FROM device_connections WHERE board_id=$1",
    )
    .bind(id)
    .fetch_optional(state.database.pool())
    .await?
    .context("device HTTPS authentication is not provisioned")?;
    let endpoint: String = row.try_get("endpoint")?;
    validate_endpoint(&endpoint)?;
    let config = state.config.read().await;
    let key = config
        .security
        .mfa_encryption_key
        .as_deref()
        .context("device credential encryption key is unavailable")?;
    let token = crypto::open(
        key,
        &format!("device:{id}"),
        &row.try_get::<String, _>("token_ciphertext")?,
    )?;
    drop(config);
    let token = std::str::from_utf8(&token)?;
    let mut header = reqwest::header::HeaderValue::from_str(&format!("Bearer {token}"))?;
    header.set_sensitive(true);
    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(reqwest::header::AUTHORIZATION, header);
    let mut client = reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .connect_timeout(std::time::Duration::from_secs(3))
        .read_timeout(std::time::Duration::from_secs(5))
        .timeout(std::time::Duration::from_secs(300))
        .default_headers(headers);
    if let Some(ca) = row.try_get::<Option<String>, _>("ca_pem")? {
        client = client.add_root_certificate(reqwest::Certificate::from_pem(ca.as_bytes())?);
    }
    Ok(DeviceConnection {
        endpoint,
        client: client.build()?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn device_origins_require_tls_and_do_not_accept_inline_credentials_or_paths() {
        for value in [
            "http://127.0.0.1:9000",
            "https://user:secret@board.test",
            "https://board.test/path",
            "https://board.test/?token=secret",
        ] {
            assert!(validate_endpoint(value).is_err(), "{value}");
        }
        assert!(validate_endpoint("https://board.test:9443").is_ok());
    }
}

/// Opaque, short-lived file capabilities for devices that cannot hold a user login.
/// Only hashes are stored; a capability grants one immutable file, never a directory.
pub async fn file_url(state: &AppState, sid: &str, path: &str) -> anyhow::Result<url::Url> {
    let config = state.config.read().await;
    if !config.edition.is_public() {
        let mut url = url::Url::parse(
            config
                .auth
                .public_base_url
                .as_deref()
                .context("service origin is unavailable")?,
        )?;
        {
            let mut segments = url
                .path_segments_mut()
                .map_err(|_| anyhow::anyhow!("invalid service origin"))?;
            segments.pop_if_empty().extend(["share", "sessions", sid]);
            for segment in crate::tftp::files::normalize_relative_path(path)?.split('/') {
                segments.push(segment);
            }
        }
        drop(config);
        ensure!(
            state.session_files.get(sid, path).await?.is_some(),
            "session file missing"
        );
        return Ok(url);
    }
    drop(config);
    let file = state
        .session_files
        .get(sid, path)
        .await?
        .context("session file missing")?;
    let session = state.session_state(sid).await.context("session ended")?;
    ensure!(
        !session.is_stop_requested() && !session.is_releasing(),
        "session ended"
    );
    let token = crypto::random_token();
    sqlx::query("DELETE FROM device_file_grants WHERE expires_at<=now()")
        .execute(state.database.pool())
        .await?;
    sqlx::query("INSERT INTO device_file_grants(token_hash,file_id,expires_at) VALUES($1,$2,now()+interval '60 seconds')")
        .bind(crypto::hash(&token)).bind(file.id).execute(state.database.pool()).await?;
    let mut base = state.config.read().await.auth.public_url()?;
    base.set_path(&format!("/device-files/{}", file.id));
    base.query_pairs_mut().append_pair("token", &token);
    Ok(base)
}
#[derive(Deserialize)]
pub struct FileQuery {
    #[serde(default)]
    token: String,
}
pub async fn download(
    State(state): State<AppState>,
    Path(id): Path<uuid::Uuid>,
    axum::extract::Query(query): axum::extract::Query<FileQuery>,
    headers: axum::http::HeaderMap,
) -> Result<axum::response::Response, ApiError> {
    if query.token.len() > 128 {
        return Err(ApiError::unauthorized("设备文件凭据无效"));
    }
    let row=sqlx::query("SELECT f.session_id FROM device_file_grants g JOIN session_files f ON f.id=g.file_id WHERE g.file_id=$1 AND g.token_hash=$2 AND g.expires_at>now() AND EXISTS(SELECT 1 FROM session_owners o JOIN users u ON u.id=o.user_id JOIN roles r ON r.id=u.role_id LEFT JOIN board_rentals b ON b.id=o.rental_id WHERE o.session_id=f.session_id AND NOT u.disabled AND r.enabled AND (u.is_admin OR (b.cancelled_at IS NULL AND b.starts_at<=now() AND b.ends_at>now() AND 'board.use'=ANY(r.permissions))))")
        .bind(id).bind(crypto::hash(&query.token)).fetch_optional(state.database.pool()).await?
        .ok_or_else(||ApiError::unauthorized("设备文件凭据无效或已过期"))?;
    let sid: String = row.try_get("session_id")?;
    let session = state
        .session_state(&sid)
        .await
        .ok_or_else(|| ApiError::not_found("会话已结束"))?;
    if session.is_stop_requested()
        || session.is_releasing()
        || session.snapshot().await.expires_at <= chrono::Utc::now()
    {
        return Err(ApiError::not_found("会话已结束"));
    }
    // Resolve the exact authorized version. A concurrent replacement at the same
    // path must never upgrade an old capability to the replacement's contents.
    let file = state
        .session_files
        .get_by_id(id)
        .await?
        .ok_or_else(|| ApiError::not_found("设备文件已失效"))?;
    crate::api::router::read_stored_file_response(&state, file, &headers).await
}
