use std::{
    io::ErrorKind,
    path::Path,
    process::Command,
    sync::{Arc, Mutex},
};

use anyhow::{Context, bail};
use async_trait::async_trait;
use tftpd::{Config, Server};

use crate::{
    config::{BuiltinTftpConfig, SystemTftpdHpaConfig, TftpConfig},
    tftp::{
        files::{
            TftpFileRef, get_session_file, list_session_files, put_session_file,
            remove_session_dir, remove_session_file,
        },
        status::{
            TftpStatus, clear_last_error, current_last_error, run_capture, set_last_error,
            system_service_state, udp_port_69_is_listening,
        },
    },
};

fn ensure_ostool_prefix(root_dir: &Path) -> anyhow::Result<()> {
    let ostool_dir = root_dir.join("ostool");
    std::fs::create_dir_all(&ostool_dir)
        .with_context(|| format!("failed to create {}", ostool_dir.display()))?;
    Ok(())
}

#[async_trait]
pub trait TftpManager: Send + Sync {
    async fn start_if_needed(&self) -> anyhow::Result<()>;
    async fn reconcile(&self) -> anyhow::Result<()>;
    async fn status(&self) -> anyhow::Result<TftpStatus>;
    async fn put_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
        bytes: &[u8],
    ) -> anyhow::Result<TftpFileRef>;
    async fn get_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> anyhow::Result<Option<TftpFileRef>>;
    async fn list_session_files(&self, session_id: &str) -> anyhow::Result<Vec<TftpFileRef>>;
    async fn remove_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> anyhow::Result<()>;
    async fn remove_session_dir(&self, session_id: &str) -> anyhow::Result<()>;
    fn root_dir(&self) -> &Path;
}

pub fn build_tftp_manager(config: &TftpConfig) -> Arc<dyn TftpManager> {
    match config {
        TftpConfig::Builtin(cfg) => Arc::new(BuiltinTftpManager::new(cfg.clone())),
        TftpConfig::SystemTftpdHpa(cfg) => Arc::new(SystemTftpdHpaManager::new(cfg.clone())),
    }
}

pub struct BuiltinTftpManager {
    config: BuiltinTftpConfig,
    start_result: Mutex<Option<Result<(), String>>>,
}

impl BuiltinTftpManager {
    pub fn new(config: BuiltinTftpConfig) -> Self {
        Self {
            config,
            start_result: Mutex::new(None),
        }
    }
}

#[async_trait]
impl TftpManager for BuiltinTftpManager {
    async fn start_if_needed(&self) -> anyhow::Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        let mut start_result = self.start_result.lock().unwrap();
        if matches!(*start_result, Some(Ok(()))) {
            return Ok(());
        }

        let result = (|| -> anyhow::Result<()> {
            std::fs::create_dir_all(&self.config.root_dir)
                .with_context(|| format!("failed to create {}", self.config.root_dir.display()))?;

            let mut config = Config::default();
            config.directory = self.config.root_dir.clone();
            config.send_directory = config.directory.clone();
            config.receive_directory = config.directory.clone();
            config.read_only = true;
            config.port = self.config.bind_addr.port();
            config.ip_address = self.config.bind_addr.ip();

            let mut server = Server::new(&config).map_err(|error| {
                anyhow::anyhow!(
                    "failed to bind builtin TFTP at {}: {error}",
                    self.config.bind_addr
                )
            })?;
            // `Server::listen` has no stop API. An independent thread lets the
            // process exit without Tokio waiting for the blocking listener.
            std::thread::Builder::new()
                .name("builtin-tftp".to_string())
                .spawn(move || server.listen())
                .context("failed to spawn builtin TFTP server thread")?;
            Ok(())
        })();
        *start_result = Some(
            result
                .as_ref()
                .copied()
                .map_err(|error| format!("{error:#}")),
        );
        result
    }

    async fn reconcile(&self) -> anyhow::Result<()> {
        std::fs::create_dir_all(&self.config.root_dir)
            .with_context(|| format!("failed to create {}", self.config.root_dir.display()))?;
        self.start_if_needed().await
    }

    async fn status(&self) -> anyhow::Result<TftpStatus> {
        let writable = ensure_ostool_prefix(&self.config.root_dir).is_ok();
        let start_result = self.start_result.lock().unwrap();
        Ok(TftpStatus {
            provider: "builtin".to_string(),
            enabled: self.config.enabled,
            healthy: matches!(*start_result, Some(Ok(()))),
            writable,
            resolved_server_ip: None,
            resolved_netmask: None,
            root_dir: self.config.root_dir.clone(),
            bind_addr_or_address: Some(self.config.bind_addr.to_string()),
            service_state: None,
            last_error: start_result
                .as_ref()
                .and_then(|result| result.as_ref().err())
                .cloned(),
        })
    }

    async fn put_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
        bytes: &[u8],
    ) -> anyhow::Result<TftpFileRef> {
        put_session_file(&self.config.root_dir, session_id, relative_path, bytes)
    }

    async fn get_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> anyhow::Result<Option<TftpFileRef>> {
        get_session_file(&self.config.root_dir, session_id, relative_path)
    }

    async fn list_session_files(&self, session_id: &str) -> anyhow::Result<Vec<TftpFileRef>> {
        list_session_files(&self.config.root_dir, session_id)
    }

    async fn remove_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> anyhow::Result<()> {
        remove_session_file(&self.config.root_dir, session_id, relative_path)
    }

    async fn remove_session_dir(&self, session_id: &str) -> anyhow::Result<()> {
        remove_session_dir(&self.config.root_dir, session_id)
    }

    fn root_dir(&self) -> &Path {
        &self.config.root_dir
    }
}

pub struct SystemTftpdHpaManager {
    config: SystemTftpdHpaConfig,
    last_error: Mutex<Option<String>>,
}

impl SystemTftpdHpaManager {
    pub fn new(config: SystemTftpdHpaConfig) -> Self {
        Self {
            config,
            last_error: Mutex::new(None),
        }
    }

    fn render_config(&self) -> String {
        let username = self.config.username.as_deref().unwrap_or("tftp");
        format!(
            "TFTP_USERNAME=\"{username}\"\nTFTP_DIRECTORY=\"{}\"\nTFTP_ADDRESS=\"{}\"\nTFTP_OPTIONS=\"{}\"\n",
            self.config.root_dir.display(),
            self.config.address,
            self.config.options
        )
    }

    fn ensure_binary_installed(&self) -> anyhow::Result<()> {
        run_capture("which", &["in.tftpd"]).with_context(|| {
            "tftpd-hpa binary `in.tftpd` not found; install `tftpd-hpa` first".to_string()
        })?;
        Ok(())
    }
}

#[async_trait]
impl TftpManager for SystemTftpdHpaManager {
    async fn start_if_needed(&self) -> anyhow::Result<()> {
        if self.config.root_dir.exists() {
            return Ok(());
        }

        if !self.config.manage_config {
            bail!(
                "TFTP root {} does not exist; create it manually, or enable `tftp.manage_config`",
                self.config.root_dir.display()
            );
        }

        std::fs::create_dir_all(&self.config.root_dir)
            .with_context(|| format!("failed to create {}", self.config.root_dir.display()))?;
        Ok(())
    }

    async fn reconcile(&self) -> anyhow::Result<()> {
        if !cfg!(target_os = "linux") {
            bail!("system_tftpd_hpa provider is only supported on Linux");
        }
        if !self.config.enabled {
            return Ok(());
        }

        self.ensure_binary_installed()?;
        self.start_if_needed().await?;

        if self.config.manage_config {
            if let Some(parent) = self.config.config_path.parent() {
                std::fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            if let Err(err) = std::fs::write(&self.config.config_path, self.render_config()) {
                if err.kind() == ErrorKind::PermissionDenied {
                    let message = format!(
                        "failed to write {}: permission denied; rerun with sudo, or set `tftp.manage_config = false` and manage the file manually",
                        self.config.config_path.display()
                    );
                    set_last_error(&self.last_error, message.clone());
                    bail!(message);
                }
                return Err(err).with_context(|| {
                    format!("failed to write {}", self.config.config_path.display())
                });
            }
            let status = Command::new("systemctl")
                .arg("restart")
                .arg(&self.config.service_name)
                .status()
                .with_context(|| format!("failed to restart {}", self.config.service_name))?;
            if !status.success() {
                let message = format!(
                    "systemctl restart {} exited with status {status}; check `systemctl status {}` and `journalctl -xeu {}`. Current TFTP_DIRECTORY={}",
                    self.config.service_name,
                    self.config.service_name,
                    self.config.service_name,
                    self.config.root_dir.display()
                );
                set_last_error(&self.last_error, message.clone());
                bail!(message);
            }

            if !udp_port_69_is_listening()? {
                let message = format!("{} is not listening on UDP 69", self.config.service_name);
                set_last_error(&self.last_error, message.clone());
                bail!(message);
            }
        }

        clear_last_error(&self.last_error);
        Ok(())
    }

    async fn status(&self) -> anyhow::Result<TftpStatus> {
        let healthy = if self.config.enabled {
            udp_port_69_is_listening().unwrap_or(false)
        } else {
            false
        };
        let writable = match ensure_ostool_prefix(&self.config.root_dir) {
            Ok(()) => true,
            Err(err) => {
                set_last_error(&self.last_error, format!("{err:#}"));
                false
            }
        };
        Ok(TftpStatus {
            provider: "system_tftpd_hpa".to_string(),
            enabled: self.config.enabled,
            healthy,
            writable,
            resolved_server_ip: None,
            resolved_netmask: None,
            root_dir: self.config.root_dir.clone(),
            bind_addr_or_address: Some(self.config.address.clone()),
            service_state: system_service_state(&self.config.service_name),
            last_error: current_last_error(&self.last_error),
        })
    }

    async fn put_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
        bytes: &[u8],
    ) -> anyhow::Result<TftpFileRef> {
        put_session_file(&self.config.root_dir, session_id, relative_path, bytes)
    }

    async fn get_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> anyhow::Result<Option<TftpFileRef>> {
        get_session_file(&self.config.root_dir, session_id, relative_path)
    }

    async fn list_session_files(&self, session_id: &str) -> anyhow::Result<Vec<TftpFileRef>> {
        list_session_files(&self.config.root_dir, session_id)
    }

    async fn remove_session_file(
        &self,
        session_id: &str,
        relative_path: &str,
    ) -> anyhow::Result<()> {
        remove_session_file(&self.config.root_dir, session_id, relative_path)
    }

    async fn remove_session_dir(&self, session_id: &str) -> anyhow::Result<()> {
        remove_session_dir(&self.config.root_dir, session_id)
    }

    fn root_dir(&self) -> &Path {
        &self.config.root_dir
    }
}

/// The database is authoritative. Files are regenerated only for serial transfer.
pub struct DatabaseTftpManager {
    inner: Arc<dyn TftpManager>,
    files: crate::persistence::files::SessionFileStore,
}
impl DatabaseTftpManager {
    pub fn new(
        inner: Arc<dyn TftpManager>,
        files: crate::persistence::files::SessionFileStore,
    ) -> Self {
        Self { inner, files }
    }
}
#[async_trait]
impl TftpManager for DatabaseTftpManager {
    async fn start_if_needed(&self) -> anyhow::Result<()> {
        self.inner.start_if_needed().await
    }
    async fn reconcile(&self) -> anyhow::Result<()> {
        self.inner.reconcile().await
    }
    async fn status(&self) -> anyhow::Result<TftpStatus> {
        self.inner.status().await
    }
    fn root_dir(&self) -> &Path {
        self.inner.root_dir()
    }
    async fn put_session_file(
        &self,
        sid: &str,
        path: &str,
        bytes: &[u8],
    ) -> anyhow::Result<TftpFileRef> {
        let file = self.files.write_bytes(sid, path, bytes).await?;
        self.files.materialize(&file, self.root_dir()).await
    }
    async fn get_session_file(&self, sid: &str, path: &str) -> anyhow::Result<Option<TftpFileRef>> {
        match self.files.get(sid, path).await? {
            Some(file) => Ok(Some(self.files.materialize(&file, self.root_dir()).await?)),
            None => Ok(None),
        }
    }
    async fn list_session_files(&self, sid: &str) -> anyhow::Result<Vec<TftpFileRef>> {
        self.files
            .list(sid)
            .await?
            .iter()
            .map(|file| file.reference(self.root_dir()))
            .collect()
    }
    async fn remove_session_file(&self, sid: &str, path: &str) -> anyhow::Result<()> {
        self.files.delete(sid, path).await?;
        self.inner.remove_session_file(sid, path).await
    }
    async fn remove_session_dir(&self, sid: &str) -> anyhow::Result<()> {
        self.files.delete_session(sid).await?;
        self.inner.remove_session_dir(sid).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::UdpSocket;

    #[tokio::test]
    async fn builtin_reports_bind_failure_and_can_retry() {
        let occupied = UdpSocket::bind("127.0.0.1:0").unwrap();
        let root = tempfile::tempdir().unwrap();
        let manager = BuiltinTftpManager::new(BuiltinTftpConfig {
            enabled: true,
            root_dir: root.path().into(),
            bind_addr: occupied.local_addr().unwrap(),
        });

        assert!(manager.start_if_needed().await.is_err());
        let failed = manager.status().await.unwrap();
        assert!(!failed.healthy);
        assert!(failed.last_error.is_some());

        drop(occupied);
        manager.start_if_needed().await.unwrap();
        let started = manager.status().await.unwrap();
        assert!(started.healthy);
        assert!(started.last_error.is_none());
    }
}
