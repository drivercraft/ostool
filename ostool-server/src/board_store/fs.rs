use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsStr,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use tokio::fs;

use crate::config::BoardConfig;

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct QuarantinedBoard {
    pub original_path: PathBuf,
    pub backup_path: PathBuf,
    pub reason: String,
    pub quarantined_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug)]
pub struct FileBoardStore {
    board_dir: PathBuf,
}

impl FileBoardStore {
    pub fn new(board_dir: PathBuf) -> Self {
        Self { board_dir }
    }

    pub fn path_for_id(&self, board_id: &str) -> PathBuf {
        self.board_dir.join(format!("{board_id}.toml"))
    }

    pub async fn ensure_dir(&self) -> anyhow::Result<()> {
        fs::create_dir_all(&self.board_dir).await?;
        Ok(())
    }

    pub async fn load_all(&self) -> anyhow::Result<BTreeMap<String, BoardConfig>> {
        let mut boards = BTreeMap::new();
        let mut network_identities = BTreeSet::new();
        let mut dir = fs::read_dir(&self.board_dir).await?;
        let mut paths = Vec::new();
        while let Some(entry) = dir.next_entry().await? {
            if entry.path().extension() == Some(OsStr::new("toml")) {
                paths.push(entry.path());
            }
        }
        // A duplicate MAC has a deterministic winner; directory enumeration order
        // must not decide which board remains available after a restart.
        paths.sort();
        for path in paths {
            let bytes = fs::read(&path)
                .await
                .with_context(|| format!("failed to read {}", path.display()))?;
            let parsed = (|| -> anyhow::Result<BoardConfig> {
                let content = std::str::from_utf8(&bytes).context("board config is not UTF-8")?;
                let board: BoardConfig =
                    toml::from_str(content).context("incompatible board TOML")?;
                board.validate().context("invalid board config")?;
                let stem = path
                    .file_stem()
                    .and_then(OsStr::to_str)
                    .context("invalid board file name")?;
                if board.id != stem {
                    bail!(
                        "board id mismatch: file stem `{stem}`, content id `{}`",
                        board.id
                    );
                }
                if boards.contains_key(&board.id) {
                    bail!("duplicate board id `{}`", board.id);
                }
                if let Some(identity) = &board.network_identity
                    && network_identities.contains(&identity.mac_address)
                {
                    bail!("duplicate network identity MAC `{}`", identity.mac_address);
                }
                Ok(board)
            })();
            match parsed {
                Ok(board) => {
                    if let Some(identity) = &board.network_identity {
                        network_identities.insert(identity.mac_address);
                    }
                    boards.insert(board.id.clone(), board);
                }
                Err(error) => {
                    self.quarantine(&path, format!("{error:#}")).await?;
                }
            }
        }
        Ok(boards)
    }

    async fn quarantine(&self, path: &Path, reason: String) -> anyhow::Result<()> {
        let root = self.board_dir.join("quarantine");
        fs::create_dir_all(&root)
            .await
            .context("failed to create board quarantine directory; original config retained")?;
        let quarantined_at = chrono::Utc::now();
        let destination = root.join(format!(
            "{}-{}",
            quarantined_at.format("%Y%m%dT%H%M%SZ"),
            uuid::Uuid::new_v4()
        ));
        fs::create_dir(&destination).await?;
        let backup_path = destination.join(path.file_name().context("board file has no name")?);
        let report = QuarantinedBoard {
            original_path: path.to_path_buf(),
            backup_path: backup_path.clone(),
            reason,
            quarantined_at,
        };
        // Write diagnostics first. If any IO fails before rename, leave the source
        // untouched. A fresh directory prevents overwriting any previous backup.
        fs::write(
            destination.join("reason.json"),
            serde_json::to_vec_pretty(&report)?,
        )
        .await?;
        fs::rename(path, &backup_path).await.with_context(|| {
            format!(
                "failed to quarantine {}; original config retained",
                path.display()
            )
        })?;
        log::warn!(
            "quarantined incompatible board config {} -> {}: {}",
            path.display(),
            backup_path.display(),
            report.reason
        );
        Ok(())
    }

    pub async fn quarantined(&self) -> anyhow::Result<Vec<QuarantinedBoard>> {
        let root = self.board_dir.join("quarantine");
        let mut dir = match fs::read_dir(root).await {
            Ok(dir) => dir,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        let mut reports = Vec::new();
        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path().join("reason.json");
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            match fs::read(&path).await {
                Ok(bytes) => match serde_json::from_slice::<QuarantinedBoard>(&bytes) {
                    Ok(report) if fs::try_exists(&report.backup_path).await? => {
                        reports.push(report)
                    }
                    Ok(_) => {}
                    Err(error) => {
                        log::warn!("invalid quarantine metadata {}: {error}", path.display())
                    }
                },
                Err(error) => {
                    log::warn!("unreadable quarantine metadata {}: {error}", path.display())
                }
            }
        }
        reports.sort_by_key(|r| std::cmp::Reverse(r.quarantined_at));
        Ok(reports)
    }

    pub async fn write_board(&self, board: &BoardConfig) -> anyhow::Result<()> {
        self.ensure_dir().await?;
        let path = self.path_for_id(&board.id);
        let temp_path = path.with_extension("toml.tmp");
        let content = toml::to_string_pretty(board)?;
        fs::write(&temp_path, content).await?;
        fs::rename(&temp_path, &path).await?;
        Ok(())
    }

    pub async fn delete_board(&self, board_id: &str) -> anyhow::Result<()> {
        let path = self.path_for_id(board_id);
        match fs::remove_file(&path).await {
            Ok(()) => Ok(()),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(err) => Err(err.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::FileBoardStore;
    use crate::config::{
        BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig, PxeProfile,
    };

    #[tokio::test]
    async fn incompatible_board_is_moved_without_preventing_valid_boards_loading() {
        let dir = tempdir().unwrap();
        let store = FileBoardStore::new(dir.path().to_path_buf());
        let invalid = b"id = 'old'\nboard_type = 'legacy'\n[boot]\nkind = 'removed-boot-kind'\n";
        tokio::fs::write(dir.path().join("old.toml"), invalid)
            .await
            .unwrap();
        let loaded = store
            .load_all()
            .await
            .expect("incompatible config must not abort startup");
        assert!(loaded.is_empty());
        assert!(!dir.path().join("old.toml").exists());
        let mut quarantine = tokio::fs::read_dir(dir.path().join("quarantine"))
            .await
            .unwrap();
        let entry = quarantine.next_entry().await.unwrap().unwrap();
        assert_eq!(
            tokio::fs::read(entry.path().join("old.toml"))
                .await
                .unwrap(),
            invalid
        );
        assert!(entry.path().join("reason.json").exists());
        assert!(store.load_all().await.unwrap().is_empty());
    }

    fn valid_board(id: &str) -> BoardConfig {
        BoardConfig {
            id: id.into(),
            board_type: "test".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "true".into(),
                power_off_cmd: "true".into(),
            }),
            boot: BootConfig::Pxe(PxeProfile::default()),
            network_identity: None,
            notes: None,
            disabled: false,
        }
    }

    #[tokio::test]
    async fn valid_boards_survive_validation_errors_and_duplicate_macs() {
        let dir = tempdir().unwrap();
        let store = FileBoardStore::new(dir.path().to_path_buf());
        store.write_board(&valid_board("valid")).await.unwrap();
        let mut missing_mac = valid_board("old-http");
        missing_mac.boot = BootConfig::UefiHttp(crate::config::UefiHttpProfile {
            boot_arch: None,
            serial_parameters: None,
        });
        store.write_board(&missing_mac).await.unwrap();
        let mut first = valid_board("a");
        first.network_identity = Some(crate::config::BoardNetworkIdentity {
            mac_address: "02:00:00:00:00:01".parse().unwrap(),
        });
        let mut second = first.clone();
        second.id = "b".into();
        store.write_board(&second).await.unwrap();
        store.write_board(&first).await.unwrap();
        let loaded = store.load_all().await.unwrap();
        assert_eq!(
            loaded.keys().map(String::as_str).collect::<Vec<_>>(),
            ["a", "valid"]
        );
        let reports = store.quarantined().await.unwrap();
        assert_eq!(reports.len(), 2);
        assert!(
            reports
                .iter()
                .any(|r| r.reason.contains("duplicate network identity"))
        );
        assert!(
            reports
                .iter()
                .any(|r| r.reason.contains("network_identity"))
        );
        assert_eq!(store.load_all().await.unwrap().len(), 2);
    }

    #[tokio::test]
    async fn backups_do_not_overwrite_and_backup_failure_retains_source() {
        let dir = tempdir().unwrap();
        let store = FileBoardStore::new(dir.path().to_path_buf());
        let path = dir.path().join("old.toml");
        for bytes in [b"invalid-one".as_slice(), b"\xff\xfe".as_slice()] {
            tokio::fs::write(&path, bytes).await.unwrap();
            store.load_all().await.unwrap();
        }
        let reports = store.quarantined().await.unwrap();
        assert_eq!(reports.len(), 2);
        assert_ne!(reports[0].backup_path, reports[1].backup_path);
        assert_eq!(
            tokio::fs::read(&reports[1].backup_path).await.unwrap(),
            b"invalid-one"
        );
        let blocked = tempdir().unwrap();
        tokio::fs::write(blocked.path().join("old.toml"), b"invalid")
            .await
            .unwrap();
        tokio::fs::write(blocked.path().join("quarantine"), b"must not replace")
            .await
            .unwrap();
        assert!(
            FileBoardStore::new(blocked.path().to_path_buf())
                .load_all()
                .await
                .is_err()
        );
        assert_eq!(
            tokio::fs::read(blocked.path().join("old.toml"))
                .await
                .unwrap(),
            b"invalid"
        );
        assert_eq!(
            tokio::fs::read(blocked.path().join("quarantine"))
                .await
                .unwrap(),
            b"must not replace"
        );
    }

    #[tokio::test]
    async fn board_store_round_trip_per_file() {
        let dir = tempdir().unwrap();
        let store = FileBoardStore::new(dir.path().to_path_buf());
        let board = BoardConfig {
            id: "rk3568-01".into(),
            board_type: "rk3568".into(),
            tags: vec!["usb".into()],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "echo on".into(),
                power_off_cmd: "echo off".into(),
            }),
            boot: BootConfig::Pxe(PxeProfile::default()),
            network_identity: None,
            notes: None,
            disabled: false,
        };

        store.write_board(&board).await.unwrap();
        let loaded = store.load_all().await.unwrap();
        assert_eq!(loaded.get("rk3568-01").unwrap().id, "rk3568-01");
        assert!(dir.path().join("rk3568-01.toml").exists());
    }
}
