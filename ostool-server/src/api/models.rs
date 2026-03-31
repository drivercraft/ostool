use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::{
    config::{BoardConfig, BootConfig, TftpConfig},
    session::Session,
    tftp::{files::TftpFileRef, status::TftpStatus},
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorResponse {
    pub code: String,
    pub message: String,
    pub details: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardTypeSummary {
    pub board_type: String,
    pub tags: Vec<String>,
    pub total: usize,
    pub available: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateSessionRequest {
    pub board_type: String,
    #[serde(default)]
    pub required_tags: Vec<String>,
    #[serde(default)]
    pub wait: bool,
    pub timeout_ms: Option<u64>,
    pub client_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionCreatedResponse {
    pub session_id: String,
    pub board_id: String,
    pub lease_expires_at: DateTime<Utc>,
    pub serial_available: bool,
    pub boot_mode: String,
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDetailResponse {
    pub session: Session,
    pub board: BoardConfig,
    pub serial_available: bool,
    pub serial_connected: bool,
    pub files: Vec<FileResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SerialStatusResponse {
    pub available: bool,
    pub connected: bool,
    pub port: Option<String>,
    pub baud_rate: Option<u32>,
    pub ws_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileResponse {
    pub slot: String,
    pub filename: String,
    pub relative_path: String,
    pub tftp_url: Option<String>,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

impl FileResponse {
    pub fn from_file(file: TftpFileRef, tftp_url: Option<String>) -> Self {
        Self {
            slot: file.slot.to_string(),
            filename: file.filename,
            relative_path: file.relative_path,
            tftp_url,
            size: file.size,
            uploaded_at: file.uploaded_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TftpSessionResponse {
    pub available: bool,
    pub provider: String,
    pub server_ip: Option<String>,
    pub writable: bool,
    pub files: Vec<FileResponse>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionResponse {
    pub ok: bool,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminTftpConfigResponse {
    pub tftp: TftpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminTftpStatusResponse {
    pub status: TftpStatus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSessionsResponse {
    pub sessions: Vec<Session>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BootProfileResponse {
    pub boot: BootConfig,
}
