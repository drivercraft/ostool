use std::{
    fs,
    path::{Component, Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, bail};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TftpFileRef {
    pub filename: String,
    #[serde(skip_serializing, skip_deserializing)]
    pub disk_path: PathBuf,
    pub relative_path: String,
    pub size: u64,
    pub uploaded_at: DateTime<Utc>,
}

pub fn normalize_relative_path(path: &str) -> anyhow::Result<String> {
    let normalized = path.trim().replace('\\', "/");
    if normalized.is_empty() {
        bail!("X-File-Path must contain a file path");
    }
    if normalized.ends_with('/') {
        bail!("file path must not end with `/`");
    }

    let source = Path::new(&normalized);
    let mut parts = Vec::new();
    for component in source.components() {
        match component {
            Component::Normal(segment) => {
                let segment = segment
                    .to_str()
                    .map(str::trim)
                    .filter(|segment| !segment.is_empty())
                    .ok_or_else(|| anyhow::anyhow!("file path contains an invalid segment"))?;
                parts.push(segment.to_string());
            }
            Component::CurDir => bail!("file path must not contain `.` segments"),
            Component::ParentDir => bail!("file path must not contain `..` segments"),
            Component::RootDir | Component::Prefix(_) => {
                bail!("file path must be relative to the session root")
            }
        }
    }

    if parts.is_empty() {
        bail!("X-File-Path must contain a file path");
    }

    Ok(parts.join("/"))
}

pub fn session_relative_path(session_id: &str, relative_path: &str) -> anyhow::Result<String> {
    let normalized_relative_path = normalize_relative_path(relative_path)?;
    Ok(format!(
        "ostool/sessions/{session_id}/{normalized_relative_path}"
    ))
}

pub fn disk_path(
    root_dir: &Path,
    session_id: &str,
    relative_path: &str,
) -> anyhow::Result<PathBuf> {
    let relative_path = normalize_relative_path(relative_path)?;
    Ok(session_root(root_dir, session_id).join(relative_path))
}

pub fn put_session_file(
    root_dir: &Path,
    session_id: &str,
    relative_path: &str,
    bytes: &[u8],
) -> anyhow::Result<TftpFileRef> {
    let normalized_relative_path = normalize_relative_path(relative_path)?;
    let path = session_root(root_dir, session_id).join(&normalized_relative_path);
    let parent = path
        .parent()
        .ok_or_else(|| anyhow::anyhow!("invalid file path: {}", path.display()))?;
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    fs::write(&path, bytes).with_context(|| format!("failed to write {}", path.display()))?;

    Ok(TftpFileRef {
        filename: filename_from_relative_path(&normalized_relative_path)?,
        disk_path: path,
        relative_path: session_relative_path(session_id, &normalized_relative_path)?,
        size: bytes.len() as u64,
        uploaded_at: Utc::now(),
    })
}

pub fn get_session_file(
    root_dir: &Path,
    session_id: &str,
    relative_path: &str,
) -> anyhow::Result<Option<TftpFileRef>> {
    let relative_path = normalize_relative_path(relative_path)?;
    let path = session_root(root_dir, session_id).join(&relative_path);
    file_ref_from_disk(root_dir, session_id, path)
}

pub fn list_session_files(root_dir: &Path, session_id: &str) -> anyhow::Result<Vec<TftpFileRef>> {
    let session_dir = session_root(root_dir, session_id);
    if !session_dir.exists() {
        return Ok(Vec::new());
    }

    let mut stack = vec![session_dir.clone()];
    let mut files = Vec::new();

    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file()
                && let Some(file) = file_ref_from_disk(root_dir, session_id, path)?
            {
                files.push(file);
            }
        }
    }

    files.sort_by(|left, right| left.relative_path.cmp(&right.relative_path));
    Ok(files)
}

pub fn remove_session_file(
    root_dir: &Path,
    session_id: &str,
    relative_path: &str,
) -> anyhow::Result<()> {
    let relative_path = normalize_relative_path(relative_path)?;
    let path = session_root(root_dir, session_id).join(&relative_path);
    if !path.is_file() {
        return Ok(());
    }

    fs::remove_file(&path).with_context(|| format!("failed to delete {}", path.display()))?;
    cleanup_empty_parent_dirs(root_dir, session_id, &path)?;
    Ok(())
}

pub fn remove_session_dir(root_dir: &Path, session_id: &str) -> anyhow::Result<()> {
    let session_dir = session_root(root_dir, session_id);
    if session_dir.exists() {
        fs::remove_dir_all(&session_dir)
            .with_context(|| format!("failed to delete {}", session_dir.display()))?;
    }
    Ok(())
}

fn session_root(root_dir: &Path, session_id: &str) -> PathBuf {
    root_dir.join("ostool").join("sessions").join(session_id)
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
    session_id: &str,
    path: PathBuf,
) -> anyhow::Result<Option<TftpFileRef>> {
    if !path.is_file() {
        return Ok(None);
    }

    let metadata = fs::metadata(&path)?;
    let relative_disk_path = path
        .strip_prefix(session_root(root_dir, session_id))
        .with_context(|| format!("failed to strip session root from {}", path.display()))?;
    let relative_disk_path = relative_disk_path.to_string_lossy().replace('\\', "/");
    let relative_disk_path = normalize_relative_path(&relative_disk_path)?;
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);

    Ok(Some(TftpFileRef {
        filename: filename_from_relative_path(&relative_disk_path)?,
        disk_path: path,
        relative_path: session_relative_path(session_id, &relative_disk_path)?,
        size: metadata.len(),
        uploaded_at: DateTime::<Utc>::from(modified),
    }))
}

fn cleanup_empty_parent_dirs(
    root_dir: &Path,
    session_id: &str,
    file_path: &Path,
) -> anyhow::Result<()> {
    let session_dir = session_root(root_dir, session_id);
    let mut current = file_path.parent();

    while let Some(dir) = current {
        if dir == session_dir {
            break;
        }
        if !dir.starts_with(&session_dir) {
            break;
        }
        if fs::read_dir(dir)?.next().is_some() {
            break;
        }
        fs::remove_dir(dir).with_context(|| format!("failed to delete {}", dir.display()))?;
        current = dir.parent();
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::{fs, thread, time::Duration};

    use tempfile::tempdir;

    use super::{
        disk_path, get_session_file, list_session_files, normalize_relative_path, put_session_file,
        remove_session_file,
    };

    #[test]
    fn path_helpers_keep_expected_relative_path() {
        let dir = tempdir().unwrap();
        let saved = put_session_file(dir.path(), "abc", "boot/Image", b"hello").unwrap();
        assert_eq!(saved.relative_path, "ostool/sessions/abc/boot/Image");
        assert_eq!(
            saved.disk_path,
            dir.path()
                .join("ostool")
                .join("sessions")
                .join("abc")
                .join("boot")
                .join("Image")
        );

        let loaded = get_session_file(dir.path(), "abc", "boot/Image")
            .unwrap()
            .unwrap();
        assert_eq!(loaded.relative_path, "ostool/sessions/abc/boot/Image");
        assert_eq!(
            disk_path(dir.path(), "abc", "boot/Image").unwrap(),
            saved.disk_path
        );
    }

    #[test]
    fn normalize_relative_path_accepts_nested_directories() {
        assert_eq!(
            normalize_relative_path(r"pxe\extlinux/extlinux.conf").unwrap(),
            "pxe/extlinux/extlinux.conf"
        );
        assert_eq!(
            normalize_relative_path(" boot/Image ").unwrap(),
            "boot/Image"
        );
    }

    #[test]
    fn normalize_relative_path_rejects_invalid_inputs() {
        for path in [
            "",
            "   ",
            "/boot/Image",
            "../Image",
            "boot/../Image",
            "./Image",
            "boot/",
        ] {
            assert!(normalize_relative_path(path).is_err(), "{path}");
        }
    }

    #[test]
    fn put_session_file_overwrites_same_path() {
        let dir = tempdir().unwrap();
        put_session_file(dir.path(), "abc", "boot/Image", b"one").unwrap();
        thread::sleep(Duration::from_millis(5));
        let saved = put_session_file(dir.path(), "abc", "boot/Image", b"two-two").unwrap();

        assert_eq!(saved.size, 7);
        assert_eq!(
            fs::read(dir.path().join("ostool/sessions/abc/boot/Image")).unwrap(),
            b"two-two"
        );
        let files = list_session_files(dir.path(), "abc").unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].relative_path, "ostool/sessions/abc/boot/Image");
    }

    #[test]
    fn list_session_files_keeps_multiple_paths_and_sorts() {
        let dir = tempdir().unwrap();
        put_session_file(dir.path(), "abc", "boot/zImage", b"kernel").unwrap();
        put_session_file(dir.path(), "abc", "boot/dtb/board.dtb", b"dtb").unwrap();
        put_session_file(dir.path(), "abc", "rootfs/initrd.img", b"initrd").unwrap();

        let files = list_session_files(dir.path(), "abc").unwrap();
        let relative_paths = files
            .iter()
            .map(|file| file.relative_path.as_str())
            .collect::<Vec<_>>();
        assert_eq!(
            relative_paths,
            vec![
                "ostool/sessions/abc/boot/dtb/board.dtb",
                "ostool/sessions/abc/boot/zImage",
                "ostool/sessions/abc/rootfs/initrd.img",
            ]
        );
    }

    #[test]
    fn remove_session_file_prunes_empty_parent_dirs_only() {
        let dir = tempdir().unwrap();
        put_session_file(dir.path(), "abc", "boot/Image", b"kernel").unwrap();
        put_session_file(dir.path(), "abc", "boot/dtb/board.dtb", b"dtb").unwrap();

        remove_session_file(dir.path(), "abc", "boot/Image").unwrap();

        assert!(!dir.path().join("ostool/sessions/abc/boot/Image").exists());
        assert!(
            dir.path()
                .join("ostool/sessions/abc/boot/dtb/board.dtb")
                .exists()
        );

        remove_session_file(dir.path(), "abc", "boot/dtb/board.dtb").unwrap();
        assert!(!dir.path().join("ostool/sessions/abc/boot/dtb").exists());
        assert!(!dir.path().join("ostool/sessions/abc/boot").exists());
        assert!(dir.path().join("ostool/sessions/abc").exists());
    }
}
