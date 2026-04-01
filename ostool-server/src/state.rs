use std::{
    collections::{BTreeMap, BTreeSet},
    path::PathBuf,
    sync::Arc,
};

use chrono::Utc;
use futures_util::future::join_all;
use tokio::sync::RwLock;

use crate::{
    board_pool::{BoardAllocationStatus, allocate_board},
    board_store::fs::FileBoardStore,
    config::{BoardConfig, ServerConfig},
    dtb_store::DtbStore,
    power::{PowerAction, execute_power_action_for_board},
    session::{Session, SessionState},
    tftp::service::TftpManager,
};

#[derive(Clone)]
pub struct AppState {
    pub config_path: Arc<PathBuf>,
    pub config: Arc<RwLock<ServerConfig>>,
    pub boards: Arc<RwLock<BTreeMap<String, BoardConfig>>>,
    pub sessions: Arc<RwLock<BTreeMap<String, Arc<SessionState>>>>,
    pub board_store: Arc<FileBoardStore>,
    pub dtb_store: Arc<DtbStore>,
    pub tftp_manager: Arc<RwLock<Arc<dyn TftpManager>>>,
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

    Ok(AppState {
        config_path: Arc::new(config_path),
        config: Arc::new(RwLock::new(config)),
        boards: Arc::new(RwLock::new(boards)),
        sessions: Arc::new(RwLock::new(BTreeMap::new())),
        board_store,
        dtb_store,
        tftp_manager: Arc::new(RwLock::new(tftp_manager)),
    })
}

impl AppState {
    pub async fn create_session(
        &self,
        board_type: &str,
        required_tags: &[String],
        client_name: Option<String>,
    ) -> Result<Session, BoardAllocationStatus> {
        let boards = self.boards.read().await;
        let sessions = self.sessions.read().await;
        let leased_board_ids = join_all(
            sessions
                .values()
                .map(|session| async move { session.snapshot().await.board_id }),
        )
        .await
        .into_iter()
        .collect::<BTreeSet<_>>();
        let board = allocate_board(&boards, &leased_board_ids, board_type, required_tags)?;
        drop(sessions);
        drop(boards);

        let session = SessionState::new(board, client_name);
        let info = session.snapshot().await;
        self.sessions.write().await.insert(info.id.clone(), session);
        Ok(info)
    }

    pub async fn get_session(&self, session_id: &str) -> Option<Session> {
        let session = self.sessions.read().await.get(session_id).cloned()?;
        Some(session.snapshot().await)
    }

    pub async fn touch_session(&self, session_id: &str) -> Option<Session> {
        let session = self.sessions.read().await.get(session_id).cloned()?;
        if session.is_releasing() {
            return None;
        }
        Some(session.heartbeat().await)
    }

    pub async fn remove_session(&self, session_id: &str) -> anyhow::Result<Option<Session>> {
        let session = self.sessions.read().await.get(session_id).cloned();
        let Some(session) = session else {
            return Ok(None);
        };
        if !session.begin_release() {
            return Ok(Some(session.snapshot().await));
        }
        let session = self
            .sessions
            .write()
            .await
            .remove(session_id)
            .unwrap_or(session);

        session.signal_shutdown();
        session.clear_serial_connected();
        self.tftp_manager
            .read()
            .await
            .remove_session_dir(session_id)
            .await?;
        if let Err(err) = execute_power_action_for_board(session.board(), PowerAction::Off).await {
            log::warn!(
                "failed to power off board `{}` while releasing session `{session_id}`: {err}",
                session.board().id
            );
        }
        Ok(Some(session.snapshot().await))
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
            if snapshot.expires_at <= now {
                expired_ids.push(snapshot.id);
            }
        }

        for session_id in &expired_ids {
            let _ = self.remove_session(session_id).await?;
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
            if let Err(err) = execute_power_action_for_board(&board, PowerAction::Off).await {
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
}

#[cfg(test)]
mod tests {
    use std::{fs, sync::Arc};

    use tempfile::tempdir;

    use super::build_app_state;
    use crate::{
        ServerConfig,
        config::{
            BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig, PxeProfile,
        },
        session::SessionState,
        tftp::service::{TftpManager, build_tftp_manager},
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

    #[tokio::test]
    async fn remove_session_notifies_session_shutdown() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let config_path = root.join(".ostool-server.toml");
        let mut config = ServerConfig::default();
        config.listen_addr = "127.0.0.1:0".parse().unwrap();
        config.data_dir = root.join("data");
        config.board_dir = root.join("boards");
        config.dtb_dir = root.join("dtbs");
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();

        let session = SessionState::new(sample_board("board-1"), None);
        let session_id = session.snapshot().await.id;
        let mut shutdown = session.subscribe_shutdown();
        state
            .sessions
            .write()
            .await
            .insert(session_id.clone(), session);

        let removed = state.remove_session(&session_id).await.unwrap();
        assert!(removed.is_some());
        shutdown.changed().await.unwrap();
        assert!(*shutdown.borrow());
    }

    #[tokio::test]
    async fn startup_power_off_runs_for_all_loaded_boards() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let board_dir = root.join("boards");
        let power_log = root.join("power.log");
        std::fs::create_dir_all(&board_dir).unwrap();
        std::fs::write(
            board_dir.join("board-1.toml"),
            format!(
                r#"
id = "board-1"
board_type = "demo"
tags = []
disabled = false

[power_management]
kind = "custom"
power_on_cmd = "printf on >> /dev/null"
power_off_cmd = "printf board-1 >> {}"

[boot]
kind = "pxe"
"#,
                power_log.display()
            ),
        )
        .unwrap();
        std::fs::write(
            board_dir.join("board-2.toml"),
            format!(
                r#"
id = "board-2"
board_type = "demo"
tags = []
disabled = false

[power_management]
kind = "custom"
power_on_cmd = "printf on >> /dev/null"
power_off_cmd = "printf board-2 >> {}"

[boot]
kind = "pxe"
"#,
                power_log.display()
            ),
        )
        .unwrap();

        let config_path = root.join(".ostool-server.toml");
        let mut config = ServerConfig::default();
        config.listen_addr = "127.0.0.1:0".parse().unwrap();
        config.data_dir = root.join("data");
        config.board_dir = board_dir;
        config.dtb_dir = root.join("dtbs");
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();

        let failures = state.power_off_all_boards_on_startup().await;
        assert!(failures.is_empty());

        let content = fs::read_to_string(power_log).unwrap();
        assert!(content.contains("board-1"));
        assert!(content.contains("board-2"));
    }

    #[tokio::test]
    async fn startup_power_off_failures_disable_boards_and_do_not_abort() {
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
kind = "custom"
power_on_cmd = "printf on >> /dev/null"
power_off_cmd = ""

[boot]
kind = "pxe"
"#,
        )
        .unwrap();

        let config_path = root.join(".ostool-server.toml");
        let mut config = ServerConfig::default();
        config.listen_addr = "127.0.0.1:0".parse().unwrap();
        config.data_dir = root.join("data");
        config.board_dir = board_dir;
        config.dtb_dir = root.join("dtbs");
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();

        let failures = state.power_off_all_boards_on_startup().await;
        assert_eq!(failures.len(), 1);
        assert_eq!(failures[0].0, "board-1");
        assert!(state.boards.read().await.get("board-1").unwrap().disabled);
    }
}
