use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, watch};

use crate::config::BoardConfig;

pub const SESSION_TTL: Duration = Duration::seconds(2);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    pub board_id: String,
    pub client_name: Option<String>,
    pub created_at: DateTime<Utc>,
    pub last_heartbeat_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    #[serde(default)]
    pub serial_connected: bool,
}

impl Session {
    pub fn new(board_id: String, client_name: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            board_id,
            client_name,
            created_at: now,
            last_heartbeat_at: now,
            expires_at: now + SESSION_TTL,
            serial_connected: false,
        }
    }

    pub fn touch(&mut self) {
        let now = Utc::now();
        self.last_heartbeat_at = now;
        self.expires_at = now + SESSION_TTL;
    }
}

#[derive(Debug)]
pub struct SessionState {
    info: RwLock<Session>,
    board: BoardConfig,
    shutdown_tx: watch::Sender<bool>,
    release_started: AtomicBool,
    serial_connected: AtomicBool,
}

impl SessionState {
    pub fn new(board: BoardConfig, client_name: Option<String>) -> Arc<Self> {
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        Arc::new(Self {
            info: RwLock::new(Session::new(board.id.clone(), client_name)),
            board,
            shutdown_tx,
            release_started: AtomicBool::new(false),
            serial_connected: AtomicBool::new(false),
        })
    }

    pub fn board(&self) -> &BoardConfig {
        &self.board
    }

    pub async fn snapshot(&self) -> Session {
        let mut info = self.info.read().await.clone();
        info.serial_connected = self.serial_connected.load(Ordering::Acquire);
        info
    }

    pub async fn heartbeat(&self) -> Session {
        let mut info = self.info.write().await;
        info.touch();
        info.serial_connected = self.serial_connected.load(Ordering::Acquire);
        info.clone()
    }

    pub fn begin_release(&self) -> bool {
        self.release_started
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn is_releasing(&self) -> bool {
        self.release_started.load(Ordering::Acquire)
    }

    pub fn subscribe_shutdown(&self) -> watch::Receiver<bool> {
        self.shutdown_tx.subscribe()
    }

    pub fn signal_shutdown(&self) {
        let _ = self.shutdown_tx.send(true);
    }

    pub fn try_set_serial_connected(&self) -> bool {
        self.serial_connected
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_ok()
    }

    pub fn clear_serial_connected(&self) {
        self.serial_connected.store(false, Ordering::Release);
    }

    pub fn set_serial_connected(&self, connected: bool) {
        self.serial_connected.store(connected, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::{SESSION_TTL, Session, SessionState};
    use crate::config::{
        BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig, PxeProfile,
    };

    fn sample_board() -> BoardConfig {
        BoardConfig {
            id: "demo".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "echo on".into(),
                power_off_cmd: "echo off".into(),
            }),
            boot: BootConfig::Pxe(PxeProfile::default()),
            notes: None,
            disabled: false,
        }
    }

    #[test]
    fn session_new_uses_fixed_ttl() {
        let session = Session::new("demo".into(), Some("client".into()));
        assert_eq!(session.expires_at - session.created_at, SESSION_TTL);
        assert_eq!(session.last_heartbeat_at, session.created_at);
    }

    #[tokio::test]
    async fn session_state_heartbeat_updates_expiry() {
        let state = SessionState::new(sample_board(), Some("client".into()));
        let first = state.snapshot().await;
        thread::sleep(std::time::Duration::from_millis(10));
        let updated = state.heartbeat().await;
        assert!(updated.last_heartbeat_at > first.last_heartbeat_at);
        assert!(updated.expires_at > first.expires_at);
    }

    #[test]
    fn session_state_release_is_idempotent() {
        let state = SessionState::new(sample_board(), None);
        assert!(state.begin_release());
        assert!(!state.begin_release());
    }
}
