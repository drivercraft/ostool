use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, Ordering},
};

use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc, watch};

use crate::{config::BoardConfig, state::AppState};

pub const SESSION_TTL: Duration = Duration::seconds(2);

const SESSION_STATE_ACTIVE: u8 = 0;
const SESSION_STATE_RELEASING: u8 = 1;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SessionLifecycleState {
    Active,
    Releasing,
}

fn default_session_state() -> SessionLifecycleState {
    SessionLifecycleState::Active
}

impl SessionLifecycleState {
    fn as_u8(self) -> u8 {
        match self {
            Self::Active => SESSION_STATE_ACTIVE,
            Self::Releasing => SESSION_STATE_RELEASING,
        }
    }

    fn from_u8(value: u8) -> Self {
        match value {
            SESSION_STATE_RELEASING => Self::Releasing,
            _ => Self::Active,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionStopReason {
    ApiDelete,
    SerialClosed,
    Expired,
    Dropped,
}

#[derive(Debug)]
enum SessionCommand {
    Stop(SessionStopReason),
}

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
    #[serde(default = "default_session_state")]
    pub state: SessionLifecycleState,
}

impl Session {
    pub fn new(board_id: String, client_name: Option<String>) -> Self {
        Self::new_with_id(uuid::Uuid::new_v4().to_string(), board_id, client_name)
    }

    pub fn new_with_id(id: String, board_id: String, client_name: Option<String>) -> Self {
        let now = Utc::now();
        Self {
            id,
            board_id,
            client_name,
            created_at: now,
            last_heartbeat_at: now,
            expires_at: now + SESSION_TTL,
            serial_connected: false,
            state: SessionLifecycleState::Active,
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
    lifecycle_state: AtomicU8,
    stop_requested: AtomicBool,
    serial_connected: AtomicBool,
    command_tx: Option<mpsc::UnboundedSender<SessionCommand>>,
}

impl SessionState {
    pub fn new(board: BoardConfig, client_name: Option<String>) -> Arc<Self> {
        Self::new_inner(uuid::Uuid::new_v4().to_string(), board, client_name, None)
    }

    pub fn new_with_actor(
        session_id: String,
        board: BoardConfig,
        client_name: Option<String>,
        app_state: AppState,
    ) -> Arc<Self> {
        let (command_tx, command_rx) = mpsc::unbounded_channel();
        let session = Self::new_inner(session_id, board, client_name, Some(command_tx));
        tokio::spawn(run_session_actor(app_state, session.clone(), command_rx));
        session
    }

    fn new_inner(
        session_id: String,
        board: BoardConfig,
        client_name: Option<String>,
        command_tx: Option<mpsc::UnboundedSender<SessionCommand>>,
    ) -> Arc<Self> {
        let (shutdown_tx, _shutdown_rx) = watch::channel(false);
        Arc::new(Self {
            info: RwLock::new(Session::new_with_id(
                session_id,
                board.id.clone(),
                client_name,
            )),
            board,
            shutdown_tx,
            lifecycle_state: AtomicU8::new(SessionLifecycleState::Active.as_u8()),
            stop_requested: AtomicBool::new(false),
            serial_connected: AtomicBool::new(false),
            command_tx,
        })
    }

    pub fn board(&self) -> &BoardConfig {
        &self.board
    }

    pub fn lifecycle_state(&self) -> SessionLifecycleState {
        SessionLifecycleState::from_u8(self.lifecycle_state.load(Ordering::Acquire))
    }

    pub async fn snapshot(&self) -> Session {
        let mut info = self.info.read().await.clone();
        info.serial_connected = self.serial_connected.load(Ordering::Acquire);
        info.state = self.lifecycle_state();
        info
    }

    pub async fn heartbeat(&self) -> Session {
        let mut info = self.info.write().await;
        info.touch();
        info.serial_connected = self.serial_connected.load(Ordering::Acquire);
        info.state = self.lifecycle_state();
        info.clone()
    }

    pub fn begin_release(&self) -> bool {
        self.lifecycle_state
            .compare_exchange(
                SessionLifecycleState::Active.as_u8(),
                SessionLifecycleState::Releasing.as_u8(),
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .is_ok()
    }

    pub fn is_releasing(&self) -> bool {
        self.lifecycle_state() == SessionLifecycleState::Releasing
    }

    pub fn request_stop(&self, reason: SessionStopReason) {
        if self.is_releasing() {
            return;
        }

        if self
            .stop_requested
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return;
        }

        if let Some(command_tx) = &self.command_tx {
            let _ = command_tx.send(SessionCommand::Stop(reason));
        }
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

    pub fn is_serial_connected(&self) -> bool {
        self.serial_connected.load(Ordering::Acquire)
    }
}

impl Drop for SessionState {
    fn drop(&mut self) {
        if self.lifecycle_state() != SessionLifecycleState::Active {
            return;
        }

        if let Some(command_tx) = &self.command_tx {
            let _ = command_tx.send(SessionCommand::Stop(SessionStopReason::Dropped));
        }
    }
}

async fn run_session_actor(
    app_state: AppState,
    session: Arc<SessionState>,
    mut command_rx: mpsc::UnboundedReceiver<SessionCommand>,
) {
    if let Some(SessionCommand::Stop(reason)) = command_rx.recv().await {
        if !session.begin_release() {
            return;
        }

        session.signal_shutdown();
        let snapshot = session.snapshot().await;

        if let Err(err) = app_state
            .transition_board_to_releasing(&snapshot.board_id, &snapshot.id)
            .await
        {
            log::warn!(
                "failed to mark board `{}` releasing for session `{}`: {err}",
                snapshot.board_id,
                snapshot.id
            );
        }

        if let Err(err) = app_state.enqueue_release(session.clone(), reason) {
            log::warn!(
                "failed to enqueue release job for session `{}`: {err}",
                snapshot.id
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use std::thread;

    use super::{SESSION_TTL, Session, SessionLifecycleState, SessionState};
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
        assert_eq!(session.state, SessionLifecycleState::Active);
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
        assert!(state.is_releasing());
    }
}
