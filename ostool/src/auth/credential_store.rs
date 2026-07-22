use std::{collections::BTreeMap, env, fs, path::PathBuf};

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
        let mut records = read_file_records(&path)?;
        records.insert(self.server_key.clone(), record.clone());
        write_file_records(&path, &records)
    }

    pub fn delete(&self) -> anyhow::Result<()> {
        if let Ok(entry) = Entry::new(KEYRING_SERVICE, &self.server_key) {
            let _ = entry.delete_credential();
        }

        let path = credential_file_path()?;
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
    // This is an explicit fallback for hosts without a usable system credential store.
    // Keep the file private even though the OS cannot provide keyring protection here.
    fs::create_dir_all(parent).with_context(|| format!("failed to create {}", parent.display()))?;
    fs::write(path, serde_json::to_vec_pretty(records)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))
            .with_context(|| format!("failed to protect {}", path.display()))?;
    }
    Ok(())
}
