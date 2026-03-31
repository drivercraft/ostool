use std::{
    collections::{BTreeMap, BTreeSet},
    path::{Path, PathBuf},
    sync::Arc,
};

use chrono::{Duration, Utc};
use tokio::sync::{RwLock, watch};
use uuid::Uuid;

use crate::{
    board_pool::find_available_board,
    board_store::fs::FileBoardStore,
    config::{BoardConfig, ServerConfig},
    session::Session,
    tftp::service::TftpManager,
};

#[derive(Clone)]
pub struct AppState {
    pub config_path: Arc<PathBuf>,
    pub config: Arc<RwLock<ServerConfig>>,
    pub boards: Arc<RwLock<BTreeMap<String, BoardConfig>>>,
    pub sessions: Arc<RwLock<BTreeMap<String, Session>>>,
    pub active_serial_sessions: Arc<RwLock<BTreeSet<String>>>,
    pub serial_shutdown_signals: Arc<RwLock<BTreeMap<String, watch::Sender<bool>>>>,
    pub board_store: Arc<FileBoardStore>,
    pub tftp_manager: Arc<RwLock<Arc<dyn TftpManager>>>,
}

pub async fn build_app_state(
    config_path: PathBuf,
    config: ServerConfig,
    tftp_manager: Arc<dyn TftpManager>,
) -> anyhow::Result<AppState> {
    let board_store = Arc::new(FileBoardStore::new(config.board_dir.clone()));
    board_store.ensure_dir().await?;
    let boards = board_store.load_all().await?;

    Ok(AppState {
        config_path: Arc::new(config_path),
        config: Arc::new(RwLock::new(config)),
        boards: Arc::new(RwLock::new(boards)),
        sessions: Arc::new(RwLock::new(BTreeMap::new())),
        active_serial_sessions: Arc::new(RwLock::new(BTreeSet::new())),
        serial_shutdown_signals: Arc::new(RwLock::new(BTreeMap::new())),
        board_store,
        tftp_manager: Arc::new(RwLock::new(tftp_manager)),
    })
}

impl AppState {
    pub async fn create_session(
        &self,
        board_type: &str,
        required_tags: &[String],
        client_name: Option<String>,
    ) -> Option<Session> {
        let boards = self.boards.read().await;
        let sessions = self.sessions.read().await;
        let board = find_available_board(&boards, &sessions, board_type, required_tags)?;
        drop(sessions);
        drop(boards);

        let ttl = Duration::seconds(self.config.read().await.lease.default_ttl_secs as i64);
        let now = Utc::now();
        let session = Session {
            id: Uuid::new_v4().to_string(),
            board_id: board.id,
            client_name,
            created_at: now,
            expires_at: now + ttl,
        };

        let mut sessions = self.sessions.write().await;
        sessions.insert(session.id.clone(), session.clone());
        Some(session)
    }

    pub async fn get_session(&self, session_id: &str) -> Option<Session> {
        self.sessions.read().await.get(session_id).cloned()
    }

    pub async fn touch_session(&self, session_id: &str) -> Option<Session> {
        let ttl = Duration::seconds(self.config.read().await.lease.default_ttl_secs as i64);
        let expires_at = Utc::now() + ttl;
        let mut sessions = self.sessions.write().await;
        let session = sessions.get_mut(session_id)?;
        session.touch(expires_at);
        Some(session.clone())
    }

    pub async fn remove_session(&self, session_id: &str) -> anyhow::Result<Option<Session>> {
        if let Some(sender) = self
            .serial_shutdown_signals
            .write()
            .await
            .remove(session_id)
        {
            let _ = sender.send(true);
        }
        self.active_serial_sessions.write().await.remove(session_id);
        self.tftp_manager
            .read()
            .await
            .remove_session_dir(session_id)
            .await?;
        Ok(self.sessions.write().await.remove(session_id))
    }

    pub async fn cleanup_expired_sessions(&self) -> anyhow::Result<Vec<String>> {
        let now = Utc::now();
        let expired = self
            .sessions
            .read()
            .await
            .values()
            .filter(|session| session.expires_at <= now)
            .map(|session| session.id.clone())
            .collect::<Vec<_>>();

        for session_id in &expired {
            let _ = self.remove_session(session_id).await?;
        }

        Ok(expired)
    }

    pub async fn session_board(&self, session_id: &str) -> Option<BoardConfig> {
        let session = self.get_session(session_id).await?;
        self.boards.read().await.get(&session.board_id).cloned()
    }

    pub async fn register_serial_shutdown(&self, session_id: &str) -> watch::Receiver<bool> {
        let (sender, receiver) = watch::channel(false);
        self.serial_shutdown_signals
            .write()
            .await
            .insert(session_id.to_string(), sender);
        receiver
    }

    pub async fn clear_serial_shutdown(&self, session_id: &str) {
        self.serial_shutdown_signals
            .write()
            .await
            .remove(session_id);
    }

    pub fn board_path(&self, board_id: &str) -> std::path::PathBuf {
        self.board_store.path_for_id(board_id)
    }

    pub async fn ensure_data_dirs(&self) -> anyhow::Result<()> {
        let config = self.config.read().await.clone();
        tokio::fs::create_dir_all(&config.data_dir).await?;
        tokio::fs::create_dir_all(&config.board_dir).await?;
        tokio::fs::create_dir_all(config.tftp.root_dir()).await?;
        Ok(())
    }

    pub async fn save_config(&self) -> anyhow::Result<()> {
        let config = self.config.read().await.clone();
        tokio::fs::write(&*self.config_path, toml::to_string_pretty(&config)?).await?;
        Ok(())
    }

    pub fn config_path_default() -> &'static Path {
        Path::new(".ostool-server.toml")
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use tempfile::tempdir;

    use super::build_app_state;
    use crate::{
        ServerConfig,
        session::Session,
        tftp::service::{TftpManager, build_tftp_manager},
    };

    #[tokio::test]
    async fn remove_session_notifies_registered_serial_shutdown() {
        let temp = tempdir().unwrap();
        let root = temp.path().to_path_buf();
        let config_path = root.join(".ostool-server.toml");
        let mut config = ServerConfig::default();
        config.listen_addr = "127.0.0.1:0".parse().unwrap();
        config.data_dir = root.join("data");
        config.board_dir = root.join("boards");
        let manager: Arc<dyn TftpManager> = build_tftp_manager(&config.tftp);
        let state = build_app_state(config_path, config, manager).await.unwrap();

        state.sessions.write().await.insert(
            "session-1".into(),
            Session {
                id: "session-1".into(),
                board_id: "board-1".into(),
                client_name: None,
                created_at: Utc::now(),
                expires_at: Utc::now(),
            },
        );

        let mut shutdown = state.register_serial_shutdown("session-1").await;
        let removed = state.remove_session("session-1").await.unwrap();
        assert!(removed.is_some());
        shutdown.changed().await.unwrap();
        assert!(*shutdown.borrow());
    }
}
