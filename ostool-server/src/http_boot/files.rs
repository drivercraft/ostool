use std::{
    fs,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::Context;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::tftp::files::normalize_relative_path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpBootFileRef {
    pub filename: String,
    #[serde(skip_serializing, skip_deserializing)]
    pub disk_path: PathBuf,
    pub relative_path: String,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

pub fn board_current_relative_path(board_id: &str, relative_path: &str) -> anyhow::Result<String> {
    let board_id = normalize_board_id(board_id)?;
    let normalized_relative_path = normalize_relative_path(relative_path)?;
    Ok(format!(
        "boards/{board_id}/current/{normalized_relative_path}"
    ))
}

pub fn board_current_disk_path(
    root_dir: &Path,
    board_id: &str,
    relative_path: &str,
) -> anyhow::Result<PathBuf> {
    let board_id = normalize_board_id(board_id)?;
    let normalized_relative_path = normalize_relative_path(relative_path)?;
    Ok(board_current_root(root_dir, board_id).join(normalized_relative_path))
}

pub fn put_board_current_file(
    root_dir: &Path,
    board_id: &str,
    relative_path: &str,
    bytes: &[u8],
) -> anyhow::Result<HttpBootFileRef> {
    let board_id = normalize_board_id(board_id)?;
    let normalized_relative_path = normalize_relative_path(relative_path)?;
    let path = board_current_root(root_dir, board_id).join(&normalized_relative_path);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid file path: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;

    let temp_path = temp_path_for(&path);
    fs::write(&temp_path, bytes)
        .with_context(|| format!("failed to write {}", temp_path.display()))?;
    fs::rename(&temp_path, &path).with_context(|| {
        format!(
            "failed to rename {} to {}",
            temp_path.display(),
            path.display()
        )
    })?;

    Ok(HttpBootFileRef {
        filename: filename_from_relative_path(&normalized_relative_path)?,
        disk_path: path,
        relative_path: board_current_relative_path(board_id, &normalized_relative_path)?,
        size: bytes.len() as u64,
        uploaded_at: Utc::now(),
    })
}

pub fn get_board_current_file(
    root_dir: &Path,
    board_id: &str,
    relative_path: &str,
) -> anyhow::Result<Option<HttpBootFileRef>> {
    let board_id = normalize_board_id(board_id)?;
    let relative_path = normalize_relative_path(relative_path)?;
    let path = board_current_root(root_dir, board_id).join(&relative_path);
    file_ref_from_disk(root_dir, board_id, path)
}

fn normalize_board_id(board_id: &str) -> anyhow::Result<&str> {
    let board_id = board_id.trim();
    if board_id.is_empty() {
        anyhow::bail!("board id must not be empty");
    }
    if board_id == "." || board_id == ".." || board_id.contains('/') || board_id.contains('\\') {
        anyhow::bail!("board id must not contain path separators or dot segments");
    }
    Ok(board_id)
}

fn board_current_root(root_dir: &Path, board_id: &str) -> PathBuf {
    root_dir.join("boards").join(board_id).join("current")
}

fn filename_from_relative_path(relative_path: &str) -> anyhow::Result<String> {
    Path::new(relative_path)
        .file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_string())
        .ok_or_else(|| anyhow::anyhow!("invalid file path `{relative_path}`"))
}

fn file_ref_from_disk(
    root_dir: &Path,
    board_id: &str,
    path: PathBuf,
) -> anyhow::Result<Option<HttpBootFileRef>> {
    if !path.is_file() {
        return Ok(None);
    }

    let metadata = fs::metadata(&path)?;
    let relative_disk_path = path
        .strip_prefix(board_current_root(root_dir, board_id))
        .with_context(|| format!("failed to strip board current root from {}", path.display()))?;
    let relative_disk_path = relative_disk_path.to_string_lossy().replace('\\', "/");
    let relative_disk_path = normalize_relative_path(&relative_disk_path)?;
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

    Ok(Some(HttpBootFileRef {
        filename: filename_from_relative_path(&relative_disk_path)?,
        disk_path: path,
        relative_path: board_current_relative_path(board_id, &relative_disk_path)?,
        size: metadata.len(),
        uploaded_at: DateTime::<Utc>::from(modified),
    }))
}

fn temp_path_for(path: &Path) -> PathBuf {
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("upload");
    path.with_file_name(format!("{filename}.tmp"))
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::tempdir;

    use super::{
        board_current_disk_path, board_current_relative_path, get_board_current_file,
        put_board_current_file,
    };

    #[test]
    fn put_board_current_file_writes_stable_board_path() {
        let dir = tempdir().unwrap();
        let saved = put_board_current_file(dir.path(), "demo-01", "kernel.bin", b"kernel").unwrap();

        assert_eq!(saved.filename, "kernel.bin");
        assert_eq!(saved.relative_path, "boards/demo-01/current/kernel.bin");
        assert_eq!(
            fs::read(dir.path().join("boards/demo-01/current/kernel.bin")).unwrap(),
            b"kernel"
        );

        let loaded = get_board_current_file(dir.path(), "demo-01", "kernel.bin")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.relative_path, "boards/demo-01/current/kernel.bin");
        assert_eq!(
            board_current_disk_path(dir.path(), "demo-01", "kernel.bin").unwrap(),
            saved.disk_path
        );
    }

    #[test]
    fn board_current_path_reuses_relative_path_validation() {
        assert_eq!(
            board_current_relative_path("demo-01", r"boot\loader.efi").unwrap(),
            "boards/demo-01/current/boot/loader.efi"
        );

        for path in ["", "/kernel.bin", "../kernel.bin", "boot/"] {
            assert!(board_current_relative_path("demo-01", path).is_err());
        }
        for board_id in ["", ".", "..", "nested/board", r"nested\board"] {
            assert!(board_current_relative_path(board_id, "kernel.bin").is_err());
        }
    }
}
