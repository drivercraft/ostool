use std::{collections::BTreeMap, env, fs, io::Write, path::PathBuf};

use anyhow::Context as _;
use chrono::{DateTime, Utc};
use keyring::Entry;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const KEYRING_SERVICE: &str = "ostool";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CredentialRecord {
    OAuthRefresh {
        refresh_token: String,
        access_token: String,
        access_expires_at: DateTime<Utc>,
        #[serde(default)]
        scope: Option<String>,
    },
    PersonalAccessToken {
        token: String,
        #[serde(default)]
        expires_at: Option<DateTime<Utc>>,
        #[serde(default)]
        scope: Option<String>,
    },
}

#[derive(Debug, Clone)]
pub struct CredentialStore {
    server_key: String,
}

impl CredentialStore {
    pub fn new(canonical_server_url: &str) -> Self {
        let digest = Sha256::digest(canonical_server_url.as_bytes());
        let digest = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        Self {
            // URL-derived keys isolate credentials for different gateways without
            // exposing the gateway address through the operating-system keyring UI.
            server_key: format!("board-server:{digest}"),
        }
    }

    pub fn load(&self) -> anyhow::Result<Option<CredentialRecord>> {
        if let Ok(entry) = Entry::new(KEYRING_SERVICE, &self.server_key)
            && let Ok(value) = entry.get_password()
        {
            return serde_json::from_str(&value)
                .map(Some)
                .context("failed to decode keyring credential");
        }
        let path = credential_file_path()?;
        let records = read_file_records(&path)?;
        Ok(records.get(&self.server_key).cloned())
    }

    pub fn save(&self, record: &CredentialRecord) -> anyhow::Result<()> {
        let value = serde_json::to_string(record).context("failed to encode credential")?;
        if let Ok(entry) = Entry::new(KEYRING_SERVICE, &self.server_key)
            && entry.set_password(&value).is_ok()
        {
            return Ok(());
        }

        eprintln!(
            "warning: the system credential store is unavailable; storing ostool credentials in a user-only file"
        );
        let path = credential_file_path()?;
        let _lock = lock_file_records(&path)?;
        let mut records = read_file_records(&path)?;
        records.insert(self.server_key.clone(), record.clone());
        write_file_records(&path, &records)
    }

    pub fn delete(&self) -> anyhow::Result<()> {
        if let Ok(entry) = Entry::new(KEYRING_SERVICE, &self.server_key) {
            let _ = entry.delete_credential();
        }

        let path = credential_file_path()?;
        let _lock = lock_file_records(&path)?;
        let mut records = read_file_records(&path)?;
        if records.remove(&self.server_key).is_some() {
            write_file_records(&path, &records)?;
        }
        Ok(())
    }
}

fn credential_file_path() -> anyhow::Result<PathBuf> {
    let config_home = env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .context("HOME and XDG_CONFIG_HOME are not set")?;
    Ok(config_home.join("ostool").join("hosts.json"))
}

fn read_file_records(path: &PathBuf) -> anyhow::Result<BTreeMap<String, CredentialRecord>> {
    match fs::read_to_string(path) {
        Ok(content) => serde_json::from_str(&content)
            .with_context(|| format!("failed to parse credential fallback {}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
        Err(error) => Err(error).with_context(|| format!("failed to read {}", path.display())),
    }
}

fn write_file_records(
    path: &PathBuf,
    records: &BTreeMap<String, CredentialRecord>,
) -> anyhow::Result<()> {
    let parent = path.parent().expect("credential path has parent");
    ensure_private_directory(parent)?;
    // tempfile creates the file privately before writing any secret. Replacement
    // is atomic: readers never see partial JSON and an existing symlink is not followed.
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(&serde_json::to_vec_pretty(records)?)?;
    file.as_file().sync_all()?;
    file.persist(path)
        .with_context(|| format!("failed to replace {}", path.display()))?;
    Ok(())
}

fn ensure_private_directory(parent: &std::path::Path) -> anyhow::Result<()> {
    let mut builder = fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt as _;
        builder.mode(0o700);
    }
    builder
        .create(parent)
        .with_context(|| format!("failed to create {}", parent.display()))?;
    Ok(())
}

// All fallback mutations share a lock, including changes for different gateways.
// The lock is separate from hosts.json, since that file is atomically replaced.
fn lock_file_records(path: &std::path::Path) -> anyhow::Result<fs::File> {
    ensure_private_directory(path.parent().expect("credential path has parent"))?;
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(0o600);
    }
    let file = options.open(path.with_extension("lock"))?;
    fs4::FileExt::lock(&file)?;
    Ok(file)
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::{PermissionsExt, symlink};

    fn records() -> BTreeMap<String, CredentialRecord> {
        BTreeMap::from([(
            "test-server".into(),
            CredentialRecord::PersonalAccessToken {
                token: "test-private-token".into(),
                expires_at: None,
                scope: None,
            },
        )])
    }

    #[test]
    fn fallback_credentials_are_private_on_creation_and_replacement() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("ostool/hosts.json");
        write_file_records(&path, &records()).unwrap();
        assert_eq!(
            fs::metadata(path.parent().unwrap())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        // Upgrading an older permissive file replaces it with a private inode.
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        write_file_records(&path, &BTreeMap::new()).unwrap();
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert!(read_file_records(&path).unwrap().is_empty());
    }

    #[test]
    fn atomic_replacement_does_not_overwrite_a_symlink_target() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("unrelated-file");
        fs::write(&target, b"leave intact").unwrap();
        let path = root.path().join("hosts.json");
        symlink(&target, &path).unwrap();
        write_file_records(&path, &records()).unwrap();
        assert_eq!(fs::read(&target).unwrap(), b"leave intact");
        assert!(
            !fs::symlink_metadata(&path)
                .unwrap()
                .file_type()
                .is_symlink()
        );
        assert_eq!(read_file_records(&path).unwrap().len(), 1);
    }
}
