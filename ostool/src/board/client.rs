use std::fmt;

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use httpboot_protocol::{
    DEVICE_PROTOCOL_VERSION_HEADER, KernelPublishResponse, LoaderStatusResponse,
    decode_device_operation_error, device_protocol_version_header_value,
};
use reqwest::{Method, StatusCode};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use url::Url;

use crate::{
    auth::token_manager::TokenManager,
    board::global_config::{AuthMode, BoardEndpoint},
    http_io::{MAX_ERROR_BODY_BYTES, read_limited_body},
};

#[derive(Clone)]
pub struct BoardServerClient {
    client: reqwest::Client,
    base_url: Url,
    ws_base_url: Url,
    endpoint: BoardEndpoint,
    token_manager: TokenManager,
}

impl fmt::Debug for BoardServerClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("BoardServerClient")
            .field("base_url", &self.base_url)
            .field("auth_mode", &self.endpoint.auth_mode)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardTypeSummary {
    pub board_type: String,
    pub tags: Vec<String>,
    pub total: usize,
    pub available: usize,
    pub leases: Option<Vec<BoardLease>>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BoardLease {
    pub board_id: String,
    pub date_begin: String,
    pub date_end: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateSessionRequest {
    pub board_type: String,
    pub required_tags: Vec<String>,
    pub client_name: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateSessionRequestWithBoardId {
    pub board_type: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board_id: Option<String>,
    pub required_tags: Vec<String>,
    pub client_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionCreatedResponse {
    pub session_id: String,
    pub board_id: String,
    pub lease_expires_at: DateTime<Utc>,
    pub serial_available: bool,
    pub boot_mode: String,
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct HeartbeatResponse {
    pub session_id: String,
    pub lease_expires_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BootConfig {
    Uboot(UbootProfile),
    Pxe(PxeProfile),
    #[serde(rename = "httpboot")]
    UefiHttp(UefiHttpProfile),
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct UbootProfile {
    #[serde(default)]
    pub use_tftp: bool,
    pub dtb_name: Option<String>,
    #[serde(default)]
    pub kernel_load_addr: Option<String>,
    #[serde(default)]
    pub fit_load_addr: Option<String>,
    #[serde(default)]
    pub bootm_addr: Option<String>,
    #[serde(default)]
    pub network_mode: UbootNetworkMode,
    #[serde(default)]
    pub board_ip: Option<String>,
    #[serde(default)]
    pub server_ip: Option<String>,
    #[serde(default)]
    pub netmask: Option<String>,
    #[serde(default)]
    pub gatewayip: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UbootNetworkMode {
    #[default]
    Dhcp,
    StaticIp,
}

#[derive(Debug, Clone, Deserialize, Default)]
pub struct PxeProfile {
    pub notes: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HttpBootKernelUpload {
    pub remote_name: String,
    pub bytes: Vec<u8>,
    pub initramfs: Option<Vec<u8>>,
    pub cmdline: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UefiHttpProfile {}

#[derive(Debug, Clone, Deserialize)]
pub struct BootProfileResponse {
    pub boot: BootConfig,
    pub server_ip: Option<String>,
    pub netmask: Option<String>,
    pub interface: Option<String>,
    pub http_base_url: Option<Url>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SerialStatusResponse {
    pub available: bool,
    pub connected: bool,
    pub port: Option<String>,
    pub baud_rate: Option<u32>,
    pub ws_url: Option<String>,
    #[serde(default)]
    pub runtime: Option<SerialRuntimeStatus>,
    #[serde(default)]
    pub manager: ostool_serial::ManagerSnapshot,
}
#[derive(Debug, Clone, Deserialize)]
pub struct SerialRuntimeStatus {
    pub phase: String,
    pub port: Option<String>,
    pub parameters: Option<httpboot_protocol::SerialParameters>,
    pub boot_epoch: Option<String>,
    pub binding_id: Option<String>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SharedSessionFileResponse {
    pub filename: String,
    pub relative_path: String,
    pub tftp_url: Option<Url>,
    pub http_url: Option<Url>,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

#[deprecated(note = "use SharedSessionFileResponse")]
pub type FileResponse = SharedSessionFileResponse;

#[derive(Debug, Clone, Deserialize)]
pub struct HttpBootFileResponse {
    pub filename: String,
    pub relative_path: String,
    pub http_url: Url,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TftpSessionResponse {
    pub available: bool,
    pub provider: String,
    pub server_ip: Option<String>,
    pub netmask: Option<String>,
    pub writable: bool,
    pub files: Vec<SharedSessionFileResponse>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SessionDtbResponse {
    pub dtb_name: Option<String>,
    pub relative_path: Option<String>,
    pub session_file_path: Option<String>,
    pub tftp_url: Option<Url>,
}

#[derive(Debug, Clone)]
pub struct BoardServerClientError {
    pub status: StatusCode,
    pub code: Option<String>,
    pub message: String,
}

impl fmt::Display for BoardServerClientError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.code {
            Some(code) => write!(
                formatter,
                "{} (HTTP {}, code {code})",
                self.message, self.status
            ),
            None => write!(formatter, "{} (HTTP {})", self.message, self.status),
        }
    }
}

impl std::error::Error for BoardServerClientError {}

impl BoardServerClientError {
    pub fn is_no_available_board_for(&self, board_type: &str) -> bool {
        self.status == StatusCode::CONFLICT
            && self.code.as_deref() == Some("conflict")
            && self.message == format!("no available board for type `{board_type}`")
    }

    pub fn is_no_available_board_id(&self, board_id: &str) -> bool {
        let board_id = board_id.trim();
        self.status == StatusCode::CONFLICT
            && self.code.as_deref() == Some("conflict")
            && self.message == format!("board `{board_id}` is not available")
    }

    pub fn is_board_type_not_found_for(&self, board_type: &str) -> bool {
        self.status == StatusCode::NOT_FOUND
            && self.code.as_deref() == Some("not_found")
            && self.message == format!("board type `{board_type}` not found")
    }
}

impl BoardServerClient {
    pub fn new(server: &str, port: u16) -> anyhow::Result<Self> {
        Self::new_with_endpoint(BoardEndpoint::new(
            &format!("http://{server}"),
            Some(port),
            AuthMode::Disabled,
        )?)
    }

    pub fn new_with_endpoint(endpoint: BoardEndpoint) -> anyhow::Result<Self> {
        Ok(Self {
            client: reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .context("failed to build HTTP client")?,
            base_url: endpoint.base_url.clone(),
            ws_base_url: endpoint.websocket_base_url()?,
            token_manager: TokenManager::new(endpoint.clone())?,
            endpoint,
        })
    }

    pub async fn list_board_types(&self) -> Result<Vec<BoardTypeSummary>, BoardServerClientError> {
        let response = self
            .request(Method::GET, self.endpoint("/api/v1/board-types"))
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub async fn create_session(
        &self,
        board_type: &str,
    ) -> Result<SessionCreatedResponse, BoardServerClientError> {
        self.create_session_inner(board_type, None).await
    }

    pub async fn create_session_with_board_id(
        &self,
        board_type: &str,
        board_id: &str,
    ) -> Result<SessionCreatedResponse, BoardServerClientError> {
        let board_id = board_id.trim();
        if board_id.is_empty() {
            return Err(BoardServerClientError {
                status: StatusCode::BAD_REQUEST,
                code: Some("bad_request".to_string()),
                message: "board_id must not be empty when provided".to_string(),
            });
        }
        self.create_session_inner(board_type, Some(board_id)).await
    }

    async fn create_session_inner(
        &self,
        board_type: &str,
        board_id: Option<&str>,
    ) -> Result<SessionCreatedResponse, BoardServerClientError> {
        let response = self
            .request(Method::POST, self.endpoint("/api/v1/sessions"))
            .await?
            .json(&CreateSessionRequestWithBoardId {
                board_type: board_type.to_string(),
                board_id: board_id.map(ToOwned::to_owned),
                required_tags: vec![],
                client_name: Some("ostool".to_string()),
            })
            .send()
            .await
            .map_err(Self::request_error)?;
        let session: SessionCreatedResponse = self.decode_json(response).await?;
        if let Some(requested_board_id) = board_id
            && session.board_id != requested_board_id
        {
            if let Err(error) = self.delete_session(&session.session_id).await {
                log::warn!(
                    "failed to release mismatched board session `{}` after requesting board `{}`: {error}",
                    session.session_id,
                    requested_board_id
                );
            }
            return Err(BoardServerClientError {
                status: StatusCode::CONFLICT,
                code: Some("conflict".to_string()),
                message: format!(
                    "server allocated board `{}` while board `{}` was requested",
                    session.board_id, requested_board_id
                ),
            });
        }
        Ok(session)
    }

    pub async fn heartbeat(
        &self,
        session_id: &str,
    ) -> Result<HeartbeatResponse, BoardServerClientError> {
        let response = self
            .request(
                Method::POST,
                self.endpoint_segments(&["api", "v1", "sessions", session_id, "heartbeat"]),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub async fn delete_session(&self, session_id: &str) -> Result<(), BoardServerClientError> {
        let response = self
            .request(
                Method::DELETE,
                self.endpoint_segments(&["api", "v1", "sessions", session_id]),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        if response.status() == StatusCode::NOT_FOUND {
            return Ok(());
        }
        self.decode_empty(response).await
    }

    pub async fn get_boot_profile(
        &self,
        session_id: &str,
    ) -> Result<BootProfileResponse, BoardServerClientError> {
        let response = self
            .request(
                Method::GET,
                self.endpoint_segments(&["api", "v1", "sessions", session_id, "boot-profile"]),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub async fn get_serial_status(
        &self,
        session_id: &str,
    ) -> Result<SerialStatusResponse, BoardServerClientError> {
        let response = self
            .request(
                Method::GET,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/serial")),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub async fn get_loader_status(
        &self,
        session_id: &str,
    ) -> Result<LoaderStatusResponse, BoardServerClientError> {
        let response = self
            .request(
                Method::GET,
                self.endpoint_segments(&["api", "v1", "sessions", session_id, "loader-status"]),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub async fn get_tftp_status(
        &self,
        session_id: &str,
    ) -> Result<TftpSessionResponse, BoardServerClientError> {
        let response = self
            .request(
                Method::GET,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/tftp")),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub async fn get_session_dtb(
        &self,
        session_id: &str,
    ) -> Result<SessionDtbResponse, BoardServerClientError> {
        let response = self
            .request(
                Method::GET,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/dtb")),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub async fn download_session_dtb(
        &self,
        session_id: &str,
    ) -> Result<Vec<u8>, BoardServerClientError> {
        let response = self
            .request(
                Method::GET,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/dtb/download")),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_bytes(response).await
    }

    pub async fn power_on_board(&self, session_id: &str) -> Result<(), BoardServerClientError> {
        let response = self
            .request(
                Method::POST,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/board/power-on")),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_empty(response).await
    }

    pub async fn power_off_board(&self, session_id: &str) -> Result<(), BoardServerClientError> {
        let response = self
            .request(
                Method::POST,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/board/power-off")),
            )
            .await?
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_empty(response).await
    }

    pub async fn upload_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
        bytes: Vec<u8>,
    ) -> Result<SharedSessionFileResponse, BoardServerClientError> {
        let response = self
            .request(
                Method::PUT,
                self.endpoint_segments(&["api", "v1", "sessions", session_id, "files"]),
            )
            .await?
            .header("X-File-Path", relative_path)
            .body(bytes)
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub async fn upload_http_boot_file(
        &self,
        session_id: &str,
        relative_path: &str,
        bytes: Vec<u8>,
    ) -> Result<HttpBootFileResponse, BoardServerClientError> {
        let response = self
            .request(
                Method::PUT,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/http-boot/files")),
            )
            .await?
            .header(
                DEVICE_PROTOCOL_VERSION_HEADER,
                device_protocol_version_header_value(),
            )
            .header("X-File-Path", relative_path)
            .body(bytes)
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub async fn upload_http_boot_kernel(
        &self,
        session_id: &str,
        upload: HttpBootKernelUpload,
    ) -> Result<KernelPublishResponse, BoardServerClientError> {
        let has_initramfs = upload.initramfs.is_some();
        if let Some(archive) = upload.initramfs {
            self.upload_http_boot_file(session_id, "initramfs.cpio", archive)
                .await?;
        }
        let mut request = self
            .request(
                Method::PUT,
                self.endpoint(&format!("/api/v1/sessions/{session_id}/http-boot/kernel")),
            )
            .await?
            .header(
                DEVICE_PROTOCOL_VERSION_HEADER,
                device_protocol_version_header_value(),
            )
            .header("X-HttpBoot-Remote-Name", upload.remote_name);
        if has_initramfs {
            request = request.header("X-HttpBoot-Initramfs-Path", "initramfs.cpio");
        }
        if let Some(cmdline) = upload.cmdline {
            request = request.header("X-HttpBoot-Cmdline", cmdline);
        }
        let response = request
            .body(upload.bytes)
            .send()
            .await
            .map_err(Self::request_error)?;
        self.decode_json(response).await
    }

    pub fn resolve_ws_url(&self, ws_url: &str) -> anyhow::Result<Url> {
        if ws_url.starts_with("ws://") || ws_url.starts_with("wss://") {
            let url =
                Url::parse(ws_url).with_context(|| format!("invalid websocket URL `{ws_url}`"))?;
            // A server-provided absolute URL must not redirect a Bearer token
            // to another origin. Relative URLs are resolved against ws_base_url below.
            if self.endpoint.auth_mode == AuthMode::Required
                && (url.scheme() != self.ws_base_url.scheme()
                    || url.host() != self.ws_base_url.host()
                    || url.port_or_known_default() != self.ws_base_url.port_or_known_default())
            {
                anyhow::bail!(
                    "refusing to send authentication credentials to cross-origin websocket URL `{url}`"
                );
            }
            return Ok(url);
        }

        self.ws_base_url
            .join(ws_url)
            .with_context(|| format!("failed to resolve websocket URL `{ws_url}`"))
    }

    fn endpoint(&self, path: &str) -> Url {
        self.base_url
            .join(path.trim_start_matches('/'))
            .expect("static API path should be valid")
    }

    fn endpoint_segments(&self, segments: &[&str]) -> Url {
        let mut url = self.base_url.clone();
        let mut path = url
            .path_segments_mut()
            .expect("HTTP board server URL must support path segments");
        path.pop_if_empty();
        for segment in segments {
            path.push(segment);
        }
        drop(path);
        url
    }

    pub async fn websocket_authorization(&self) -> anyhow::Result<Option<String>> {
        self.token_manager.authorization_token().await
    }

    async fn request(
        &self,
        method: Method,
        url: Url,
    ) -> Result<reqwest::RequestBuilder, BoardServerClientError> {
        let request = self.client.request(method, url);
        match self.token_manager.authorization_token().await {
            // TokenManager returns None only for the explicitly anonymous mode.
            Ok(Some(token)) => Ok(request.bearer_auth(token)),
            Ok(None) => Ok(request),
            Err(error) => Err(BoardServerClientError {
                status: StatusCode::UNAUTHORIZED,
                code: Some("authentication_required".to_string()),
                message: error.to_string(),
            }),
        }
    }

    async fn decode_json<T: DeserializeOwned>(
        &self,
        response: reqwest::Response,
    ) -> Result<T, BoardServerClientError> {
        if response.status().is_success() {
            response.json::<T>().await.map_err(Self::request_error)
        } else {
            self.clear_invalid_credential_if_unauthorized(&response)
                .await;
            Err(Self::api_error(response).await)
        }
    }

    async fn decode_empty(
        &self,
        response: reqwest::Response,
    ) -> Result<(), BoardServerClientError> {
        if response.status().is_success() {
            Ok(())
        } else {
            self.clear_invalid_credential_if_unauthorized(&response)
                .await;
            Err(Self::api_error(response).await)
        }
    }

    async fn decode_bytes(
        &self,
        response: reqwest::Response,
    ) -> Result<Vec<u8>, BoardServerClientError> {
        if response.status().is_success() {
            response
                .bytes()
                .await
                .map(|bytes| bytes.to_vec())
                .map_err(Self::request_error)
        } else {
            self.clear_invalid_credential_if_unauthorized(&response)
                .await;
            Err(Self::api_error(response).await)
        }
    }

    async fn clear_invalid_credential_if_unauthorized(&self, response: &reqwest::Response) {
        // Do not keep a credential the gateway has explicitly rejected. The next
        // command must authenticate again instead of silently falling back to LAN mode.
        if response.status() == StatusCode::UNAUTHORIZED
            && let Err(error) = self.token_manager.invalidate_after_unauthorized().await
        {
            log::warn!("failed to remove rejected board credentials: {error:#}");
        }
    }

    async fn api_error(response: reqwest::Response) -> BoardServerClientError {
        let status = response.status();
        let body = match read_limited_body(response, MAX_ERROR_BODY_BYTES).await {
            Ok(body) => body,
            Err(error) => {
                return BoardServerClientError {
                    status,
                    code: Some("invalid_device_response".to_string()),
                    message: format!("invalid_device_response (HTTP {status}): {error}"),
                };
            }
        };
        parse_error_body(status, &String::from_utf8_lossy(&body))
    }

    fn request_error(err: reqwest::Error) -> BoardServerClientError {
        BoardServerClientError {
            status: StatusCode::SERVICE_UNAVAILABLE,
            code: Some("request_failed".to_string()),
            message: err.to_string(),
        }
    }
}

fn parse_error_body(status: StatusCode, body: &str) -> BoardServerClientError {
    match decode_device_operation_error(body.as_bytes()) {
        Ok(error) => BoardServerClientError {
            status,
            code: Some(error.code),
            message: error.message,
        },
        Err(_) if !body.trim().is_empty() => BoardServerClientError {
            status,
            code: Some("invalid_device_response".to_string()),
            message: format!("invalid_device_response (HTTP {status}): {}", body.trim()),
        },
        Err(_) => BoardServerClientError {
            status,
            code: None,
            message: format!("request failed with status {status}"),
        },
    }
}

impl fmt::Display for BoardTypeSummary {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({}/{})", self.board_type, self.available, self.total)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    use chrono::{DateTime, Utc};
    use reqwest::StatusCode;
    use serde::Serialize;
    use tokio::{
        io::{AsyncReadExt as _, AsyncWriteExt as _},
        net::TcpStream,
    };
    use url::Url;

    use super::{
        BoardServerClient, BoardTypeSummary, BootConfig, HttpBootKernelUpload, parse_error_body,
    };
    use crate::board::global_config::{AuthMode, BoardEndpoint};

    #[derive(Serialize)]
    struct FutureSharedSessionFileResponse {
        filename: String,
        relative_path: String,
        tftp_url: Option<Url>,
        http_url: Option<Url>,
        size: u64,
        uploaded_at: DateTime<Utc>,
        checksum: String,
    }

    #[test]
    fn board_type_summary_accepts_response_without_leases() {
        let summaries: Vec<BoardTypeSummary> = serde_json::from_str(
            r#"[
                {
                    "board_type": "rk3568",
                    "tags": [],
                    "total": 2,
                    "available": 1
                }
            ]"#,
        )
        .unwrap();

        assert!(summaries[0].leases.is_none());
    }

    #[test]
    fn board_type_summary_parses_optional_leases() {
        let summaries: Vec<BoardTypeSummary> = serde_json::from_str(
            r#"[
                {
                    "board_type": "Rock-4D",
                    "tags": [],
                    "total": 1,
                    "available": 1,
                    "leases": [
                        {
                            "board_id": "Rock-4D-1",
                            "date_begin": "2026-07-31 15:40:00",
                            "date_end": "2026-08-03 15:40:00"
                        }
                    ]
                }
            ]"#,
        )
        .unwrap();

        let leases = summaries[0].leases.as_ref().unwrap();
        assert_eq!(leases.len(), 1);
        assert_eq!(leases[0].board_id, "Rock-4D-1");
        assert_eq!(leases[0].date_begin, "2026-07-31 15:40:00");
        assert_eq!(leases[0].date_end, "2026-08-03 15:40:00");
    }

    #[test]
    fn resolve_relative_ws_url_uses_server_defaults() {
        let client = BoardServerClient::new("127.0.0.1", 8080).unwrap();
        let url = client
            .resolve_ws_url("/api/v1/sessions/demo/serial/ws")
            .unwrap();
        assert_eq!(
            url.as_str(),
            "ws://127.0.0.1:8080/api/v1/sessions/demo/serial/ws"
        );
    }

    #[test]
    fn shared_session_file_response_round_trips_urls_and_ignores_future_fields() {
        let fixture = FutureSharedSessionFileResponse {
            filename: "probe script.sh".to_string(),
            relative_path: "ostool/sessions/demo/tools/probe script.sh".to_string(),
            tftp_url: None,
            http_url: Some(
                Url::parse("http://192.168.1.2:2999/share/sessions/demo/tools/probe%20script.sh")
                    .unwrap(),
            ),
            size: 42,
            uploaded_at: "2026-07-27T00:00:00Z".parse().unwrap(),
            checksum: "future-field".to_string(),
        };

        let encoded = serde_json::to_vec(&fixture).unwrap();
        let decoded: super::SharedSessionFileResponse = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded.filename, fixture.filename);
        assert_eq!(decoded.relative_path, fixture.relative_path);
        assert_eq!(decoded.http_url, fixture.http_url);
        assert_eq!(decoded.size, fixture.size);
    }

    #[test]
    fn resolve_absolute_ws_url_keeps_original_value() {
        let client = BoardServerClient::new("127.0.0.1", 8080).unwrap();
        let url = client
            .resolve_ws_url("ws://10.0.0.2:9000/api/v1/sessions/demo/serial/ws")
            .unwrap();
        assert_eq!(
            url.as_str(),
            "ws://10.0.0.2:9000/api/v1/sessions/demo/serial/ws"
        );
    }

    #[test]
    fn authenticated_client_rejects_cross_origin_websocket_url() {
        let client = BoardServerClient::new_with_endpoint(
            BoardEndpoint::new("https://203.0.113.10:8443", None, AuthMode::Required).unwrap(),
        )
        .unwrap();
        assert!(
            client
                .resolve_ws_url("wss://203.0.113.11:8443/api/v1/sessions/demo/serial/ws")
                .is_err()
        );
    }

    #[test]
    fn parse_error_body_prefers_structured_api_errors() {
        let error = parse_error_body(
            StatusCode::UNPROCESSABLE_ENTITY,
            r#"{"code":"future_device_error","message":"future device detail"}"#,
        );
        assert_eq!(error.status, StatusCode::UNPROCESSABLE_ENTITY);
        assert_eq!(error.code.as_deref(), Some("future_device_error"));
        assert_eq!(error.message, "future device detail");
    }

    #[tokio::test]
    async fn chunked_error_body_is_rejected_at_the_shared_limit() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = listener.accept().await.unwrap();
            let _ = read_http_request_bytes(&mut socket).await;
            let chunk = "x".repeat(33 * 1024);
            let response = format!(
                "HTTP/1.1 500 Internal Server Error\r\ntransfer-encoding: chunked\r\nconnection: close\r\n\r\n{:x}\r\n{chunk}\r\n{:x}\r\n{chunk}\r\n0\r\n\r\n",
                chunk.len(),
                chunk.len()
            );
            socket.write_all(response.as_bytes()).await.unwrap();
        });

        let endpoint =
            BoardEndpoint::new(&format!("http://{address}"), None, AuthMode::Disabled).unwrap();
        let client = BoardServerClient::new_with_endpoint(endpoint).unwrap();
        let error = client.list_board_types().await.unwrap_err();
        assert_eq!(error.status, StatusCode::INTERNAL_SERVER_ERROR);
        assert_eq!(error.code.as_deref(), Some("invalid_device_response"));
        assert!(
            error.message.contains("body exceeds 65536 bytes"),
            "{error}"
        );
        server.await.unwrap();
    }

    #[tokio::test]
    async fn httpboot_uploads_send_v7_on_files_and_kernel_without_arch_headers() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let kernel = b"future-isa-kernel".to_vec();
        let expected_kernel = kernel.clone();
        let server = tokio::spawn(async move {
            let (mut files, _) = listener.accept().await.unwrap();
            let request = read_http_request_bytes(&mut files).await;
            let header_end = request
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .unwrap();
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            assert!(headers.starts_with("put /api/v1/sessions/demo/http-boot/files "));
            assert!(headers.contains("x-axloader-protocol-version: 7\r\n"));
            assert_eq!(&request[header_end + 4..], b"initramfs");
            write_http_response(
                &mut files,
                "201 Created",
                &serde_json::json!({
                    "filename": "initramfs.cpio",
                    "relative_path": "ostool/sessions/demo/initramfs.cpio",
                    "http_url": format!("http://{address}/boot/initramfs.cpio"),
                    "size": 9,
                    "uploaded_at": "2026-11-10T00:00:00Z"
                })
                .to_string(),
            )
            .await;

            let (mut kernel_socket, _) = listener.accept().await.unwrap();
            let request = read_http_request_bytes(&mut kernel_socket).await;
            let header_end = request
                .windows(4)
                .position(|part| part == b"\r\n\r\n")
                .unwrap();
            let headers = String::from_utf8_lossy(&request[..header_end]).to_ascii_lowercase();
            assert!(headers.starts_with("put /api/v1/sessions/demo/http-boot/kernel "));
            assert!(headers.contains("x-axloader-protocol-version: 7\r\n"));
            assert!(!headers.contains("x-httpboot-arch:"));
            assert!(!headers.contains("x-httpboot-image-format:"));
            assert!(!headers.contains("x-httpboot-entry-symbol:"));
            assert_eq!(&request[header_end + 4..], b"future-isa-kernel");
            write_http_response(
                &mut kernel_socket,
                "201 Created",
                &serde_json::json!({
                    "boot_id": "boot-1",
                    "kernel_url": format!("http://{address}/boot/kernel.elf"),
                    "kernel_size": 17,
                    "kernel_sha256": null
                })
                .to_string(),
            )
            .await;
        });

        let endpoint =
            BoardEndpoint::new(&format!("http://{address}"), None, AuthMode::Disabled).unwrap();
        let client = BoardServerClient::new_with_endpoint(endpoint).unwrap();
        client
            .upload_http_boot_kernel(
                "demo",
                HttpBootKernelUpload {
                    remote_name: "kernel.elf".into(),
                    bytes: kernel,
                    initramfs: Some(b"initramfs".to_vec()),
                    cmdline: Some("console=ttyS0".into()),
                },
            )
            .await
            .unwrap();
        server.await.unwrap();
        assert_eq!(expected_kernel, b"future-isa-kernel".to_vec());
    }

    #[test]
    fn not_found_error_is_classified_as_missing_board_type() {
        let error = parse_error_body(
            StatusCode::NOT_FOUND,
            r#"{"code":"not_found","message":"board type `rk3568` not found"}"#,
        );
        assert!(error.is_board_type_not_found_for("rk3568"));
        assert!(!error.is_no_available_board_for("rk3568"));
    }

    #[tokio::test]
    async fn create_session_releases_mismatched_requested_board_id() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let saw_delete = Arc::new(AtomicBool::new(false));
        let saw_delete_server = saw_delete.clone();

        let server = tokio::spawn(async move {
            let (mut post_socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut post_socket).await;
            assert!(request.starts_with("POST /api/v1/sessions "));
            assert!(request.contains(r#""board_id":"demo-02""#));
            write_http_response(
                &mut post_socket,
                "201 Created",
                r#"{"session_id":"session-1","board_id":"demo-01","lease_expires_at":"2026-09-03T00:00:00Z","serial_available":false,"boot_mode":"uboot","ws_url":null}"#,
            )
            .await;

            let (mut delete_socket, _) = listener.accept().await.unwrap();
            let request = read_http_request(&mut delete_socket).await;
            assert!(request.starts_with("DELETE /api/v1/sessions/session-1 "));
            saw_delete_server.store(true, Ordering::SeqCst);
            write_http_response(&mut delete_socket, "202 Accepted", "").await;
        });

        let endpoint =
            BoardEndpoint::new(&format!("http://{address}"), None, AuthMode::Disabled).unwrap();
        let client = BoardServerClient::new_with_endpoint(endpoint).unwrap();
        let error = client
            .create_session_with_board_id("demo", " demo-02 ")
            .await
            .unwrap_err();

        server.await.unwrap();
        assert!(saw_delete.load(Ordering::SeqCst));
        assert_eq!(error.status, StatusCode::CONFLICT);
        assert_eq!(error.code.as_deref(), Some("conflict"));
        assert_eq!(
            error.message,
            "server allocated board `demo-01` while board `demo-02` was requested"
        );
    }

    async fn read_http_request(socket: &mut TcpStream) -> String {
        String::from_utf8(read_http_request_bytes(socket).await).unwrap()
    }

    pub(crate) async fn read_http_request_bytes(socket: &mut TcpStream) -> Vec<u8> {
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 1024];
        loop {
            let read = socket.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "client closed connection before request completed");
            bytes.extend_from_slice(&buffer[..read]);
            if request_complete(&bytes) {
                break;
            }
        }
        bytes
    }

    fn request_complete(bytes: &[u8]) -> bool {
        let Some(header_end) = bytes.windows(4).position(|window| window == b"\r\n\r\n") else {
            return false;
        };
        let headers = String::from_utf8_lossy(&bytes[..header_end]);
        let content_length = headers
            .lines()
            .find_map(|line| {
                let (name, value) = line.split_once(':')?;
                name.eq_ignore_ascii_case("content-length")
                    .then_some(value.trim())
            })
            .and_then(|value| value.parse::<usize>().ok())
            .unwrap_or(0);
        bytes.len() >= header_end + 4 + content_length
    }

    pub(crate) async fn write_http_response(socket: &mut TcpStream, status: &str, body: &str) {
        let response = format!(
            "HTTP/1.1 {status}\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
            body.len()
        );
        socket.write_all(response.as_bytes()).await.unwrap();
    }

    #[test]
    fn parse_uboot_boot_profile() {
        let response: super::BootProfileResponse = serde_json::from_str(
            r#"{
                "boot": {
                    "kind": "uboot",
                    "use_tftp": true,
                    "kernel_load_addr": "0x80200000",
                    "fit_load_addr": "0x82200000",
                    "bootm_addr": "0x82200000"
                },
                "server_ip": "10.0.0.2",
                "netmask": "255.255.255.0",
                "interface": "eth0"
            }"#,
        )
        .unwrap();

        assert_eq!(response.server_ip.as_deref(), Some("10.0.0.2"));
        match response.boot {
            BootConfig::Uboot(profile) => {
                assert!(profile.use_tftp);
                assert_eq!(profile.kernel_load_addr.as_deref(), Some("0x80200000"));
                assert_eq!(profile.fit_load_addr.as_deref(), Some("0x82200000"));
                assert_eq!(profile.bootm_addr.as_deref(), Some("0x82200000"));
            }
            BootConfig::Pxe(_) | BootConfig::UefiHttp(_) => panic!("expected uboot profile"),
        }
    }

    #[test]
    fn parse_static_uboot_boot_profile() {
        let response: super::BootProfileResponse = serde_json::from_str(
            r#"{
                "boot": {
                    "kind": "uboot",
                    "use_tftp": true,
                    "network_mode": "static_ip",
                    "board_ip": "192.168.10.20",
                    "server_ip": "192.168.10.2",
                    "netmask": "255.255.255.0",
                    "gatewayip": "192.168.10.1"
                },
                "server_ip": "192.168.10.2",
                "netmask": "255.255.255.0",
                "interface": "eth0"
            }"#,
        )
        .unwrap();

        match response.boot {
            BootConfig::Uboot(profile) => {
                assert!(profile.use_tftp);
                assert_eq!(profile.network_mode, super::UbootNetworkMode::StaticIp);
                assert_eq!(profile.board_ip.as_deref(), Some("192.168.10.20"));
                assert_eq!(profile.server_ip.as_deref(), Some("192.168.10.2"));
                assert_eq!(profile.netmask.as_deref(), Some("255.255.255.0"));
                assert_eq!(profile.gatewayip.as_deref(), Some("192.168.10.1"));
            }
            BootConfig::Pxe(_) => panic!("expected uboot profile"),
            BootConfig::UefiHttp(_) => panic!("expected uboot profile"),
        }
    }

    #[test]
    fn parse_httpboot_boot_profile_only_accepts_the_current_kind() {
        let response: super::BootProfileResponse =
            serde_json::from_value(serde_json::json!({"boot": {"kind": "httpboot"}})).unwrap();
        assert!(matches!(response.boot, BootConfig::UefiHttp(_)));

        for kind in ["uefi_http", "httpboot"] {
            let value = serde_json::json!({"boot": {"kind": kind, "boot_arch": "future_arch"}});
            assert!(serde_json::from_value::<super::BootProfileResponse>(value).is_err());
        }
    }

    #[test]
    fn parse_tftp_session_file_response() {
        let response: super::TftpSessionResponse = serde_json::from_str(
            r#"{
                "available": true,
                "provider": "builtin",
                "server_ip": "10.0.0.2",
                "netmask": "255.255.255.0",
                "writable": true,
                "files": [
                    {
                        "filename": "image.fit",
                        "relative_path": "ostool/sessions/demo/boot/image.fit",
                        "tftp_url": "tftp://10.0.0.2/ostool/sessions/demo/boot/image.fit",
                        "size": 1234,
                        "uploaded_at": "2026-04-01T00:00:00Z"
                    }
                ]
            }"#,
        )
        .unwrap();

        assert!(response.available);
        assert_eq!(response.files.len(), 1);
        assert_eq!(response.files[0].filename, "image.fit");
    }

    #[test]
    fn parse_session_dtb_response() {
        let response: super::SessionDtbResponse = serde_json::from_str(
            r#"{
                "dtb_name": "board.dtb",
                "relative_path": "ostool/sessions/demo/boot/dtb/board.dtb",
                "session_file_path": "boot/dtb/board.dtb",
                "tftp_url": "tftp://10.0.0.2/ostool/sessions/demo/boot/dtb/board.dtb"
            }"#,
        )
        .unwrap();

        assert_eq!(response.dtb_name.as_deref(), Some("board.dtb"));
        assert_eq!(
            response.session_file_path.as_deref(),
            Some("boot/dtb/board.dtb")
        );
    }
}
