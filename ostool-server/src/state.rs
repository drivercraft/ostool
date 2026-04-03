use std::{
    collections::{BTreeMap, BTreeSet},
    future::Future,
    path::PathBuf,
    sync::Arc,
    time::Duration,
};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use tokio::sync::{RwLock, mpsc};

use crate::{
    board_pool::{BoardAllocationStatus, allocate_board},
    board_store::fs::FileBoardStore,
    config::{BoardConfig, PowerManagementConfig, ServerConfig},
    dtb_store::DtbStore,
    power::{PowerAction, PowerActionError, execute_power_action_for_board},
    session::{Session, SessionState, SessionStopReason},
    tftp::service::TftpManager,
};

const RELEASE_RETRY_ATTEMPTS: usize = 3;
const RELEASE_RETRY_DELAY: Duration = Duration::from_millis(200);
const RELEASE_WAIT_TIMEOUT: Duration = Duration::from_secs(2);
const RELEASE_COMPLETION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoardLeaseState {
    Idle,
    Using,
    Releasing,
    Error,
}

#[derive(Debug, Clone)]
pub struct BoardRuntimeState {
    pub lease_state: BoardLeaseState,
    pub active_session_id: Option<String>,
    pub last_release_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

impl Default for BoardRuntimeState {
    fn default() -> Self {
        Self {
            lease_state: BoardLeaseState::Idle,
            active_session_id: None,
            last_release_error: None,
            updated_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct BoardRuntimeStatusSnapshot {
    pub lease_state: BoardLeaseState,
    pub active_session_id: Option<String>,
    pub last_release_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Default)]
pub struct VirtualBoardPowerStatus {
    pub powered: bool,
    pub last_action: Option<PowerAction>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
pub struct BoardPowerStatusSnapshot {
    pub available: bool,
    pub powered: Option<bool>,
    pub last_action: Option<PowerAction>,
    pub updated_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone)]
struct ReleaseJob {
    session: Arc<SessionState>,
    reason: SessionStopReason,
}

#[derive(Clone)]
pub struct AppState {
    pub config_path: Arc<PathBuf>,
    pub config: Arc<RwLock<ServerConfig>>,
    pub boards: Arc<RwLock<BTreeMap<String, BoardConfig>>>,
    pub board_runtimes: Arc<RwLock<BTreeMap<String, BoardRuntimeState>>>,
    pub sessions: Arc<RwLock<BTreeMap<String, Arc<SessionState>>>>,
    pub virtual_power_statuses: Arc<RwLock<BTreeMap<String, VirtualBoardPowerStatus>>>,
    pub board_store: Arc<FileBoardStore>,
    pub dtb_store: Arc<DtbStore>,
    pub tftp_manager: Arc<RwLock<Arc<dyn TftpManager>>>,
    release_tx: mpsc::UnboundedSender<ReleaseJob>,
}

pub async fn build_app_state(
    config_path: PathBuf,
    config: ServerConfig,
    tftp_manager: Arc<dyn TftpManager>,
) -> anyhow::Result<AppState> {
    let board_store = Arc::new(FileBoardStore::new(config.board_dir.clone()));
    board_store.ensure_dir().await?;
    let dtb_store = Arc::new(DtbStore::new(config.dtb_dir.clone()));
    dtb_store.ensure_dir().await?;
    let boards = board_store.load_all().await?;
    let board_runtimes = initial_board_runtimes(&boards);
    let virtual_power_statuses = initial_virtual_power_statuses(&boards);
    let (release_tx, release_rx) = mpsc::unbounded_channel();

    let state = AppState {
        config_path: Arc::new(config_path),
        config: Arc::new(RwLock::new(config)),
        boards: Arc::new(RwLock::new(boards)),
        board_runtimes: Arc::new(RwLock::new(board_runtimes)),
        sessions: Arc::new(RwLock::new(BTreeMap::new())),
        virtual_power_statuses: Arc::new(RwLock::new(virtual_power_statuses)),
        board_store,
        dtb_store,
        tftp_manager: Arc::new(RwLock::new(tftp_manager)),
        release_tx,
    };

    tokio::spawn(run_release_coordinator(state.clone(), release_rx));

    Ok(state)
}

fn initial_board_runtimes(
    boards: &BTreeMap<String, BoardConfig>,
) -> BTreeMap<String, BoardRuntimeState> {
    boards
        .keys()
        .map(|board_id| (board_id.clone(), BoardRuntimeState::default()))
        .collect()
}

fn initial_virtual_power_statuses(
    boards: &BTreeMap<String, BoardConfig>,
) -> BTreeMap<String, VirtualBoardPowerStatus> {
    boards
        .iter()
        .filter(|(_, board)| matches!(board.power_management, PowerManagementConfig::Virtual(_)))
        .map(|(board_id, _)| (board_id.clone(), VirtualBoardPowerStatus::default()))
        .collect()
}

async fn run_release_coordinator(
    state: AppState,
    mut release_rx: mpsc::UnboundedReceiver<ReleaseJob>,
) {
    while let Some(job) = release_rx.recv().await {
        let state = state.clone();
        tokio::spawn(async move {
            state.process_release_job(job).await;
        });
    }
}

impl AppState {
    pub async fn create_session(
        &self,
        board_type: &str,
        required_tags: &[String],
        client_name: Option<String>,
    ) -> Result<Session, BoardAllocationStatus> {
        loop {
            let boards = self.boards.read().await;
            let runtimes = self.board_runtimes.read().await;
            let unavailable_board_ids = runtimes
                .iter()
                .filter(|(_, runtime)| runtime.lease_state != BoardLeaseState::Idle)
                .map(|(board_id, _)| board_id.clone())
                .collect::<BTreeSet<_>>();
            let board = allocate_board(&boards, &unavailable_board_ids, board_type, required_tags)?;
            drop(runtimes);
            drop(boards);

            let session_id = uuid::Uuid::new_v4().to_string();
            if !self.claim_board_for_session(&board.id, &session_id).await {
                continue;
            }

            let session =
                SessionState::new_with_actor(session_id, board, client_name.clone(), self.clone());
            let info = session.snapshot().await;
            self.sessions.write().await.insert(info.id.clone(), session);
            return Ok(info);
        }
    }

    pub async fn get_session(&self, session_id: &str) -> Option<Session> {
        let session = self.sessions.read().await.get(session_id).cloned()?;
        Some(session.snapshot().await)
    }

    pub async fn heartbeat_session(&self, session_id: &str) -> Result<Session, TouchSessionError> {
        let session = self
            .sessions
            .read()
            .await
            .get(session_id)
            .cloned()
            .ok_or(TouchSessionError::NotFound)?;
        if session.is_releasing() {
            return Err(TouchSessionError::Releasing);
        }
        Ok(session.heartbeat().await)
    }

    pub async fn request_session_stop(
        &self,
        session_id: &str,
        reason: SessionStopReason,
    ) -> Option<Session> {
        let session = self.sessions.read().await.get(session_id).cloned()?;
        session.request_stop(reason);
        Some(session.snapshot().await)
    }

    pub async fn remove_session(&self, session_id: &str) -> anyhow::Result<Option<Session>> {
        let snapshot = self
            .request_session_stop(session_id, SessionStopReason::ApiDelete)
            .await;
        if snapshot.is_none() {
            return Ok(None);
        }
        self.wait_for_session_removed(session_id, RELEASE_COMPLETION_TIMEOUT)
            .await?;
        Ok(snapshot)
    }

    pub async fn cleanup_expired_sessions(&self) -> anyhow::Result<Vec<String>> {
        let now = Utc::now();
        let sessions = self
            .sessions
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<_>>();
        let mut expired_ids = Vec::new();

        for session in sessions {
            let snapshot = session.snapshot().await;
            if snapshot.state == crate::session::SessionLifecycleState::Releasing {
                continue;
            }
            if snapshot.expires_at <= now {
                session.request_stop(SessionStopReason::Expired);
                expired_ids.push(snapshot.id);
            }
        }

        Ok(expired_ids)
    }

    pub async fn session_board(&self, session_id: &str) -> Option<BoardConfig> {
        let session = self.sessions.read().await.get(session_id).cloned()?;
        Some(session.board().clone())
    }

    pub async fn session_state(&self, session_id: &str) -> Option<Arc<SessionState>> {
        self.sessions.read().await.get(session_id).cloned()
    }

    pub async fn board_power_status(&self, board_id: &str) -> Option<BoardPowerStatusSnapshot> {
        let board = self.boards.read().await.get(board_id).cloned()?;
        match board.power_management {
            PowerManagementConfig::Virtual(_) => {
                let statuses = self.virtual_power_statuses.read().await;
                let status = statuses.get(board_id).cloned().unwrap_or_default();
                Some(BoardPowerStatusSnapshot {
                    available: true,
                    powered: Some(status.powered),
                    last_action: status.last_action,
                    updated_at: status.updated_at,
                })
            }
            _ => Some(BoardPowerStatusSnapshot {
                available: false,
                powered: None,
                last_action: None,
                updated_at: None,
            }),
        }
    }

    pub async fn board_runtime_status(&self, board_id: &str) -> Option<BoardRuntimeStatusSnapshot> {
        let runtime = self.board_runtimes.read().await.get(board_id).cloned()?;
        Some(BoardRuntimeStatusSnapshot {
            lease_state: runtime.lease_state,
            active_session_id: runtime.active_session_id,
            last_release_error: runtime.last_release_error,
            updated_at: runtime.updated_at,
        })
    }

    pub async fn execute_board_power_action(
        &self,
        board: &BoardConfig,
        action: PowerAction,
    ) -> Result<String, PowerActionError> {
        match board.power_management {
            PowerManagementConfig::Virtual(_) => {
                let mut statuses = self.virtual_power_statuses.write().await;
                let entry = statuses.entry(board.id.clone()).or_default();
                entry.powered = matches!(action, PowerAction::On);
                entry.last_action = Some(action);
                entry.updated_at = Some(Utc::now());
                Ok(format!(
                    "recorded virtual {} for board `{}`",
                    action.label(),
                    board.id
                ))
            }
            _ => execute_power_action_for_board(board, action).await,
        }
    }

    pub fn board_path(&self, board_id: &str) -> std::path::PathBuf {
        self.board_store.path_for_id(board_id)
    }

    pub async fn ensure_data_dirs(&self) -> anyhow::Result<()> {
        let config = self.config.read().await.clone();
        tokio::fs::create_dir_all(&config.data_dir).await?;
        tokio::fs::create_dir_all(&config.board_dir).await?;
        tokio::fs::create_dir_all(&config.dtb_dir).await?;
        tokio::fs::create_dir_all(config.tftp.root_dir()).await?;
        Ok(())
    }

    pub async fn power_off_all_boards_on_startup(&self) -> Vec<(String, String)> {
        let boards = self
            .boards
            .read()
            .await
            .values()
            .cloned()
            .collect::<Vec<BoardConfig>>();
        let mut failures = Vec::new();

        for board in boards {
            if let Err(err) = self
                .execute_board_power_action(&board, PowerAction::Off)
                .await
            {
                failures.push((board.id.clone(), err.to_string()));
                if let Some(stored) = self.boards.write().await.get_mut(&board.id) {
                    stored.disabled = true;
                }
            }
        }

        failures
    }

    pub async fn save_config(&self) -> anyhow::Result<()> {
        let config = self.config.read().await.clone();
        tokio::fs::write(&*self.config_path, toml::to_string_pretty(&config)?).await?;
        Ok(())
    }

    pub async fn sync_board_runtime_states(&self) {
        let boards = self.boards.read().await;
        let mut runtimes = self.board_runtimes.write().await;
        let mut next = BTreeMap::new();
        for board_id in boards.keys() {
            let runtime = runtimes.remove(board_id).unwrap_or_default();
            next.insert(board_id.clone(), runtime);
        }
        *runtimes = next;
    }

    pub async fn sync_virtual_power_statuses(&self) {
        let boards = self.boards.read().await;
        let mut statuses = self.virtual_power_statuses.write().await;
        let mut next = BTreeMap::new();
        for (board_id, board) in boards.iter() {
            if matches!(board.power_management, PowerManagementConfig::Virtual(_)) {
                let status = statuses.remove(board_id).unwrap_or_default();
                next.insert(board_id.clone(), status);
            }
        }
        *statuses = next;
    }

    pub async fn claim_board_for_session(&self, board_id: &str, session_id: &str) -> bool {
        let mut runtimes = self.board_runtimes.write().await;
        let Some(runtime) = runtimes.get_mut(board_id) else {
            return false;
        };
        if runtime.lease_state != BoardLeaseState::Idle {
            return false;
        }

        runtime.lease_state = BoardLeaseState::Using;
        runtime.active_session_id = Some(session_id.to_string());
        runtime.last_release_error = None;
        runtime.updated_at = Utc::now();
        true
    }

    pub async fn transition_board_to_releasing(
        &self,
        board_id: &str,
        session_id: &str,
    ) -> anyhow::Result<()> {
        let mut runtimes = self.board_runtimes.write().await;
        let runtime = runtimes
            .get_mut(board_id)
            .ok_or_else(|| anyhow::anyhow!("board runtime `{board_id}` not found"))?;

        if runtime.lease_state == BoardLeaseState::Releasing
            && runtime.active_session_id.as_deref() == Some(session_id)
        {
            return Ok(());
        }

        if runtime.lease_state != BoardLeaseState::Using
            || runtime.active_session_id.as_deref() != Some(session_id)
        {
            anyhow::bail!(
                "board `{board_id}` is not owned by session `{session_id}` during release transition"
            );
        }

        runtime.lease_state = BoardLeaseState::Releasing;
        runtime.updated_at = Utc::now();
        Ok(())
    }

    pub async fn mark_board_idle(&self, board_id: &str, session_id: &str) -> anyhow::Result<()> {
        let mut runtimes = self.board_runtimes.write().await;
        let runtime = runtimes
            .get_mut(board_id)
            .ok_or_else(|| anyhow::anyhow!("board runtime `{board_id}` not found"))?;

        if runtime.active_session_id.as_deref() != Some(session_id) {
            anyhow::bail!("board `{board_id}` is no longer associated with session `{session_id}`");
        }

        runtime.lease_state = BoardLeaseState::Idle;
        runtime.active_session_id = None;
        runtime.last_release_error = None;
        runtime.updated_at = Utc::now();
        Ok(())
    }

    pub async fn mark_board_error(
        &self,
        board_id: &str,
        session_id: &str,
        error: String,
    ) -> anyhow::Result<()> {
        let mut runtimes = self.board_runtimes.write().await;
        let runtime = runtimes
            .get_mut(board_id)
            .ok_or_else(|| anyhow::anyhow!("board runtime `{board_id}` not found"))?;

        if runtime.active_session_id.as_deref() != Some(session_id) {
            anyhow::bail!("board `{board_id}` is no longer associated with session `{session_id}`");
        }

        runtime.lease_state = BoardLeaseState::Error;
        runtime.active_session_id = None;
        runtime.last_release_error = Some(error);
        runtime.updated_at = Utc::now();
        Ok(())
    }

    pub fn enqueue_release(
        &self,
        session: Arc<SessionState>,
        reason: SessionStopReason,
    ) -> anyhow::Result<()> {
        self.release_tx
            .send(ReleaseJob { session, reason })
            .map_err(|_| anyhow::anyhow!("release coordinator is not running"))
    }

    pub async fn remove_session_runtime(&self, session_id: &str) {
        self.sessions.write().await.remove(session_id);
    }

    async fn wait_for_session_removed(
        &self,
        session_id: &str,
        timeout: Duration,
    ) -> anyhow::Result<()> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if !self.sessions.read().await.contains_key(session_id) {
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                anyhow::bail!("timed out waiting for session `{session_id}` removal");
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }

    async fn process_release_job(&self, job: ReleaseJob) {
        let session = job.session;
        let snapshot = session.snapshot().await;
        log::debug!(
            "processing release for session `{}` on board `{}` due to {:?}",
            snapshot.id,
            snapshot.board_id,
            job.reason
        );

        let mut errors = Vec::new();

        if let Err(err) = self
            .wait_for_session_tasks_to_stop(&session, RELEASE_WAIT_TIMEOUT)
            .await
        {
            errors.push(err);
        }

        if let Err(err) = retry_release_step(RELEASE_RETRY_ATTEMPTS, RELEASE_RETRY_DELAY, || {
            let state = self.clone();
            let board = session.board().clone();
            async move {
                state
                    .execute_board_power_action(&board, PowerAction::Off)
                    .await
                    .map(|_| ())
                    .map_err(|err| err.to_string())
            }
        })
        .await
        {
            errors.push(format!("power-off failed: {err}"));
        }

        if let Err(err) = retry_release_step(RELEASE_RETRY_ATTEMPTS, RELEASE_RETRY_DELAY, || {
            let manager = self.tftp_manager.clone();
            let session_id = snapshot.id.clone();
            async move {
                manager
                    .read()
                    .await
                    .clone()
                    .remove_session_dir(&session_id)
                    .await
                    .map_err(|err| err.to_string())
            }
        })
        .await
        {
            errors.push(format!("tftp cleanup failed: {err}"));
        }

        if errors.is_empty() {
            if let Err(err) = self.mark_board_idle(&snapshot.board_id, &snapshot.id).await {
                log::warn!(
                    "failed to mark board `{}` idle after releasing session `{}`: {err:#}",
                    snapshot.board_id,
                    snapshot.id
                );
            }
        } else {
            let message = errors.join("; ");
            if let Err(err) = self
                .mark_board_error(&snapshot.board_id, &snapshot.id, message.clone())
                .await
            {
                log::warn!(
                    "failed to mark board `{}` error after releasing session `{}`: {err:#}",
                    snapshot.board_id,
                    snapshot.id
                );
            }
            log::warn!(
                "release for session `{}` completed with errors: {}",
                snapshot.id,
                message
            );
        }

        self.remove_session_runtime(&snapshot.id).await;
    }

    async fn wait_for_session_tasks_to_stop(
        &self,
        session: &SessionState,
        timeout: Duration,
    ) -> Result<(), String> {
        let deadline = tokio::time::Instant::now() + timeout;
        loop {
            if !session.is_serial_connected() {
                return Ok(());
            }

            if tokio::time::Instant::now() >= deadline {
                return Err("timed out waiting for session tasks to stop".to_string());
            }

            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

async fn retry_release_step<F, Fut>(
    attempts: usize,
    delay: Duration,
    mut operation: F,
) -> Result<(), String>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), String>>,
{
    let mut last_error = None;
    for attempt in 1..=attempts {
        match operation().await {
            Ok(()) => return Ok(()),
            Err(err) => {
                last_error = Some(err);
                if attempt < attempts {
                    tokio::time::sleep(delay * attempt as u32).await;
                }
            }
        }
    }

    Err(last_error.unwrap_or_else(|| "release step failed".to_string()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TouchSessionError {
    NotFound,
    Releasing,
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path, sync::Arc, time::Duration};

    use async_trait::async_trait;
    use tempfile::tempdir;

    use super::{BoardLeaseState, TouchSessionError, build_app_state};
    use crate::{
        ServerConfig,
        config::{
            BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig, PxeProfile,
            VirtualPowerManagement,
        },
        power::PowerAction,
        session::{SessionState, SessionStopReason},
        tftp::{
            files::TftpFileRef,
            service::{TftpManager, build_tftp_manager},
            status::TftpStatus,
        },
    };

    fn sample_board(board_id: &str) -> BoardConfig {
        BoardConfig {
            id: board_id.into(),
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

    fn sample_virtual_board(board_id: &str) -> BoardConfig {
        BoardConfig {
            id: board_id.into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Virtual(VirtualPowerManagement::default()),
            boot: BootConfig::Pxe(PxeProfile::default()),
            notes: None,
            disabled: false,
        }
    }

    struct FailingRemoveTftpManager {
        root_dir: std::path::PathBuf,
    }

    #[async_trait]
    impl TftpManager for FailingRemoveTftpManager {
        async fn start_if_needed(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn reconcile(&self) -> anyhow::Result<()> {
            Ok(())
        }

        async fn status(&self) -> anyhow::Result<TftpStatus> {
            Ok(TftpStatus {
                provider: "test".into(),
                enabled: true,
                healthy: true,
                writable: true,
                resolved_server_ip: None,
                resolved_netmask: None,
                root_dir: self.root_dir.clone(),
                bind_addr_or_address: None,
                service_state: None,
                last_error: None,
            })
        }

        async fn put_session_file(
            &self,
            _session_id: &str,
            _relative_path: &str,
            _bytes: &[u8],
        ) -> anyhow::Result<TftpFileRef> {
            anyhow::bail!("not implemented in test")
        }

        async fn get_session_file(
            &self,
            _session_id: &str,
            _relative_path: &str,
        ) -> anyhow::Result<Option<TftpFileRef>> {
            Ok(None)
        }

        async fn list_session_files(&self, _session_id: &str) -> anyhow::Result<Vec<TftpFileRef>> {
            Ok(Vec::new())
        }

        async fn remove_session_file(
            &self,
            _session_id: &str,
            _relative_path: &str,
        ) -> anyhow::Result<()> {
            Ok(())
        }

        async fn remove_session_dir(&self, _session_id: &str) -> anyhow::Result<()> {
            anyhow::bail!("simulated TFTP cleanup failure")
        }

        fn root_dir(&self) -> &Path {
            &self.root_dir
        }
    }

    async fn test_state(root: &Path) -> super::AppState {
        let config_path = root.join(".ostool-server.toml");
        let config = ServerConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            data_dir: root.join("data"),
            board_dir: root.join("boards"),
            dtb_dir: root.join("dtbs"),
            network: crate::TftpNetworkConfig {
                interface: "lo".into(),
            },
            ..ServerConfig::default()
        };
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        build_app_state(config_path, config, manager).await.unwrap()
    }

    #[tokio::test]
    async fn create_session_claims_board_runtime() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let state = test_state(&root).await;
        state
            .boards
            .write()
            .await
            .insert("board-1".into(), sample_board("board-1"));
        state.sync_board_runtime_states().await;

        let session = state.create_session("demo", &[], None).await.unwrap();
        let runtime = state.board_runtime_status("board-1").await.unwrap();
        assert_eq!(runtime.lease_state, BoardLeaseState::Using);
        assert_eq!(
            runtime.active_session_id.as_deref(),
            Some(session.id.as_str())
        );
    }

    #[tokio::test]
    async fn heartbeat_rejects_releasing_session() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let state = test_state(&root).await;
        state
            .boards
            .write()
            .await
            .insert("board-1".into(), sample_board("board-1"));
        state.sync_board_runtime_states().await;
        let session = state.create_session("demo", &[], None).await.unwrap();
        let handle = state.session_state(&session.id).await.unwrap();
        handle.set_serial_connected(true);

        state
            .request_session_stop(&session.id, SessionStopReason::ApiDelete)
            .await
            .unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;

        let err = state.heartbeat_session(&session.id).await.unwrap_err();
        assert_eq!(err, TouchSessionError::Releasing);
        handle.clear_serial_connected();
        let removed = state.remove_session(&session.id).await.unwrap();
        assert!(removed.is_some());
    }

    #[tokio::test]
    async fn remove_session_notifies_session_shutdown() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let state = test_state(&root).await;
        let board = sample_board("board-1");
        state
            .boards
            .write()
            .await
            .insert(board.id.clone(), board.clone());
        state.sync_board_runtime_states().await;
        let session =
            SessionState::new_with_actor("session-1".into(), board.clone(), None, state.clone());
        let mut shutdown = session.subscribe_shutdown();
        state.claim_board_for_session(&board.id, "session-1").await;
        state
            .sessions
            .write()
            .await
            .insert("session-1".into(), session);

        let removed = state.remove_session("session-1").await.unwrap();
        assert!(removed.is_some());
        shutdown.changed().await.unwrap();
        assert!(*shutdown.borrow());
    }

    #[tokio::test]
    async fn remove_session_marks_board_error_when_tftp_cleanup_fails() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let power_log = root.join("power.log");
        let config_path = root.join(".ostool-server.toml");
        let config = ServerConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            data_dir: root.join("data"),
            board_dir: root.join("boards"),
            dtb_dir: root.join("dtbs"),
            network: crate::TftpNetworkConfig {
                interface: "lo".into(),
            },
            ..ServerConfig::default()
        };
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();
        *state.tftp_manager.write().await = Arc::new(FailingRemoveTftpManager {
            root_dir: root.join("tftp"),
        });

        let board = BoardConfig {
            id: "board-1".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "printf on >/dev/null".into(),
                power_off_cmd: format!("printf off >> {}", power_log.display()),
            }),
            boot: BootConfig::Pxe(PxeProfile::default()),
            notes: None,
            disabled: false,
        };
        state
            .boards
            .write()
            .await
            .insert(board.id.clone(), board.clone());
        state.sync_board_runtime_states().await;
        let session =
            SessionState::new_with_actor("session-1".into(), board.clone(), None, state.clone());
        state.claim_board_for_session(&board.id, "session-1").await;
        state
            .sessions
            .write()
            .await
            .insert("session-1".into(), session);

        let removed = state.remove_session("session-1").await.unwrap();
        assert!(removed.is_some());
        assert_eq!(fs::read_to_string(power_log).unwrap(), "off");
        assert!(!state.sessions.read().await.contains_key("session-1"));
        let runtime = state.board_runtime_status("board-1").await.unwrap();
        assert_eq!(runtime.lease_state, BoardLeaseState::Error);
        assert!(
            runtime
                .last_release_error
                .unwrap()
                .contains("tftp cleanup failed")
        );
    }

    #[tokio::test]
    async fn duplicate_stop_requests_only_power_off_once() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let power_log = root.join("power.log");
        let state = test_state(&root).await;

        let board = BoardConfig {
            id: "board-1".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "printf on >/dev/null".into(),
                power_off_cmd: format!("printf 'off\\n' >> {}", power_log.display()),
            }),
            boot: BootConfig::Pxe(PxeProfile::default()),
            notes: None,
            disabled: false,
        };
        state
            .boards
            .write()
            .await
            .insert(board.id.clone(), board.clone());
        state.sync_board_runtime_states().await;
        let session =
            SessionState::new_with_actor("session-1".into(), board.clone(), None, state.clone());
        state.claim_board_for_session(&board.id, "session-1").await;
        state
            .sessions
            .write()
            .await
            .insert("session-1".into(), session);

        state
            .request_session_stop("session-1", SessionStopReason::SerialClosed)
            .await;
        let removed = state.remove_session("session-1").await.unwrap();
        assert!(removed.is_some());
        assert_eq!(fs::read_to_string(power_log).unwrap(), "off\n");
    }

    #[tokio::test]
    async fn startup_power_off_sets_virtual_board_status() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let board_dir = root.join("boards");
        std::fs::create_dir_all(&board_dir).unwrap();
        std::fs::write(
            board_dir.join("board-1.toml"),
            r#"
id = "board-1"
board_type = "demo"
tags = []
disabled = false

[power_management]
kind = "virtual"

[boot]
kind = "pxe"
"#,
        )
        .unwrap();

        let config_path = root.join(".ostool-server.toml");
        let config = ServerConfig {
            listen_addr: "127.0.0.1:0".parse().unwrap(),
            data_dir: root.join("data"),
            board_dir,
            dtb_dir: root.join("dtbs"),
            network: crate::TftpNetworkConfig {
                interface: "lo".into(),
            },
            ..ServerConfig::default()
        };
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();

        let failures = state.power_off_all_boards_on_startup().await;
        assert!(failures.is_empty());
        let status = state.board_power_status("board-1").await.unwrap();
        assert_eq!(status.powered, Some(false));
        assert_eq!(status.last_action, Some(PowerAction::Off));
        assert!(status.updated_at.is_some());
    }

    #[tokio::test]
    async fn remove_session_sets_virtual_board_power_status_to_off() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let state = test_state(&root).await;

        let board = sample_virtual_board("board-1");
        state
            .boards
            .write()
            .await
            .insert(board.id.clone(), board.clone());
        state.sync_board_runtime_states().await;
        state.sync_virtual_power_statuses().await;
        let session =
            SessionState::new_with_actor("session-1".into(), board.clone(), None, state.clone());
        state.claim_board_for_session(&board.id, "session-1").await;
        state
            .sessions
            .write()
            .await
            .insert("session-1".into(), session);

        state
            .execute_board_power_action(&board, PowerAction::On)
            .await
            .unwrap();
        let removed = state.remove_session("session-1").await.unwrap();
        assert!(removed.is_some());

        let status = state.board_power_status("board-1").await.unwrap();
        assert_eq!(status.powered, Some(false));
        assert_eq!(status.last_action, Some(PowerAction::Off));
        assert!(status.updated_at.is_some());
        let runtime = state.board_runtime_status("board-1").await.unwrap();
        assert_eq!(runtime.lease_state, BoardLeaseState::Idle);
    }
}
