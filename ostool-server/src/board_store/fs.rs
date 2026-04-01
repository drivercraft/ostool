use std::{collections::BTreeMap, ffi::OsStr, path::PathBuf};

use anyhow::{Context, bail};
use tokio::fs;

use crate::config::BoardConfig;

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
        let mut dir = fs::read_dir(&self.board_dir).await?;

        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            if path.extension() != Some(OsStr::new("toml")) {
                continue;
            }

            let content = fs::read_to_string(&path)
                .await
                .with_context(|| format!("failed to read {}", path.display()))?;
            let board: BoardConfig = toml::from_str(&content)
                .with_context(|| format!("failed to parse {}", path.display()))?;

            let stem = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .ok_or_else(|| {
                    anyhow::anyhow!("invalid board config file name: {}", path.display())
                })?;

            if board.id != stem {
                bail!(
                    "board id mismatch in {}: file stem is `{stem}` but content id is `{}`",
                    path.display(),
                    board.id
                );
            }

            if boards.insert(board.id.clone(), board).is_some() {
                bail!("duplicate board id `{stem}` in {}", path.display());
            }
        }

        Ok(boards)
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
            notes: None,
            disabled: false,
        };

        store.write_board(&board).await.unwrap();
        let loaded = store.load_all().await.unwrap();
        assert_eq!(loaded.get("rk3568-01").unwrap().id, "rk3568-01");
        assert!(dir.path().join("rk3568-01.toml").exists());
    }
}
