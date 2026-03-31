use std::{
    fmt::{Display, Formatter},
    path::{Path, PathBuf},
    str::FromStr,
    time::SystemTime,
};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
#[serde(rename_all = "snake_case")]
pub enum FileSlot {
    Kernel,
    Dtb,
    Initramfs,
    Fit,
    Script,
    Other,
}

impl FileSlot {
    pub const ALL: [FileSlot; 6] = [
        FileSlot::Kernel,
        FileSlot::Dtb,
        FileSlot::Initramfs,
        FileSlot::Fit,
        FileSlot::Script,
        FileSlot::Other,
    ];

    pub fn iter() -> impl Iterator<Item = FileSlot> {
        Self::ALL.into_iter()
    }
}

impl Display for FileSlot {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        let value = match self {
            Self::Kernel => "kernel",
            Self::Dtb => "dtb",
            Self::Initramfs => "initramfs",
            Self::Fit => "fit",
            Self::Script => "script",
            Self::Other => "other",
        };
        write!(f, "{value}")
    }
}

impl FromStr for FileSlot {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "kernel" => Ok(Self::Kernel),
            "dtb" => Ok(Self::Dtb),
            "initramfs" => Ok(Self::Initramfs),
            "fit" => Ok(Self::Fit),
            "script" => Ok(Self::Script),
            "other" => Ok(Self::Other),
            _ => bail!("unsupported slot `{s}`"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TftpFileRef {
    pub slot: FileSlot,
    pub filename: String,
    #[serde(skip_serializing, skip_deserializing)]
    pub disk_path: PathBuf,
    pub relative_path: String,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

pub fn sanitize_filename(name: &str) -> anyhow::Result<String> {
    let normalized = name.replace('\\', "/");
    let candidate = Path::new(&normalized)
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| anyhow::anyhow!("X-File-Name must contain a file name"))?;

    if candidate == "." || candidate == ".." {
        bail!("invalid file name `{candidate}`");
    }

    Ok(candidate.to_string())
}

pub fn relative_path(session_id: &str, slot: FileSlot, filename: &str) -> String {
    format!("ostool/sessions/{session_id}/{slot}/{filename}")
}

pub fn disk_path(root_dir: &Path, session_id: &str, slot: FileSlot, filename: &str) -> PathBuf {
    root_dir
        .join("ostool")
        .join("sessions")
        .join(session_id)
        .join(slot.to_string())
        .join(filename)
}

pub fn put_session_file(
    root_dir: &Path,
    session_id: &str,
    slot: FileSlot,
    filename: &str,
    bytes: &[u8],
) -> anyhow::Result<TftpFileRef> {
    let filename = sanitize_filename(filename)?;
    let slot_dir = root_dir
        .join("ostool")
        .join("sessions")
        .join(session_id)
        .join(slot.to_string());
    std::fs::create_dir_all(&slot_dir)
        .with_context(|| format!("failed to create {}", slot_dir.display()))?;

    if slot_dir.exists() {
        for entry in std::fs::read_dir(&slot_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_file() {
                std::fs::remove_file(&path)
                    .with_context(|| format!("failed to delete {}", path.display()))?;
            }
        }
    }

    let path = slot_dir.join(&filename);
    std::fs::write(&path, bytes).with_context(|| format!("failed to write {}", path.display()))?;

    Ok(TftpFileRef {
        slot,
        filename: filename.clone(),
        disk_path: path,
        relative_path: relative_path(session_id, slot, &filename),
        size: bytes.len() as u64,
        uploaded_at: Utc::now(),
    })
}

pub fn get_session_file(
    root_dir: &Path,
    session_id: &str,
    slot: FileSlot,
) -> anyhow::Result<Option<TftpFileRef>> {
    let slot_dir = root_dir
        .join("ostool")
        .join("sessions")
        .join(session_id)
        .join(slot.to_string());

    if !slot_dir.exists() {
        return Ok(None);
    }

    let maybe_file = std::fs::read_dir(&slot_dir)?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path())
        .find(|path| path.is_file());

    let Some(path) = maybe_file else {
        return Ok(None);
    };

    let metadata = std::fs::metadata(&path)?;
    let filename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| anyhow::anyhow!("invalid file path: {}", path.display()))?
        .to_string();
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let uploaded_at = DateTime::<Utc>::from(modified);

    Ok(Some(TftpFileRef {
        slot,
        filename: filename.clone(),
        disk_path: path,
        relative_path: relative_path(session_id, slot, &filename),
        size: metadata.len(),
        uploaded_at,
    }))
}

pub fn list_session_files(root_dir: &Path, session_id: &str) -> anyhow::Result<Vec<TftpFileRef>> {
    let mut files = Vec::new();
    for slot in FileSlot::iter() {
        if let Some(file) = get_session_file(root_dir, session_id, slot)? {
            files.push(file);
        }
    }
    Ok(files)
}

pub fn remove_session_file(
    root_dir: &Path,
    session_id: &str,
    slot: FileSlot,
) -> anyhow::Result<()> {
    let slot_dir = root_dir
        .join("ostool")
        .join("sessions")
        .join(session_id)
        .join(slot.to_string());
    if slot_dir.exists() {
        std::fs::remove_dir_all(&slot_dir)
            .with_context(|| format!("failed to delete {}", slot_dir.display()))?;
    }
    Ok(())
}

pub fn remove_session_dir(root_dir: &Path, session_id: &str) -> anyhow::Result<()> {
    let session_dir = root_dir.join("ostool").join("sessions").join(session_id);
    if session_dir.exists() {
        std::fs::remove_dir_all(&session_dir)
            .with_context(|| format!("failed to delete {}", session_dir.display()))?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{FileSlot, get_session_file, put_session_file, sanitize_filename};

    #[test]
    fn path_helpers_keep_expected_relative_path() {
        let dir = tempdir().unwrap();
        let saved =
            put_session_file(dir.path(), "abc", FileSlot::Kernel, "Image", b"hello").unwrap();
        assert_eq!(saved.relative_path, "ostool/sessions/abc/kernel/Image");
        assert_eq!(
            saved.disk_path,
            dir.path()
                .join("ostool")
                .join("sessions")
                .join("abc")
                .join("kernel")
                .join("Image")
        );
        let loaded = get_session_file(dir.path(), "abc", FileSlot::Kernel)
            .unwrap()
            .unwrap();
        assert_eq!(loaded.relative_path, "ostool/sessions/abc/kernel/Image");
    }

    #[test]
    fn sanitize_filename_strips_directories() {
        assert_eq!(sanitize_filename("../foo/bar.bin").unwrap(), "bar.bin");
        assert!(sanitize_filename("..").is_err());
    }
}
