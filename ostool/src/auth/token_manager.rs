use std::{env, fs, path::PathBuf, sync::Arc};

use anyhow::{Context as _, bail};
use chrono::{Duration, Utc};
use tokio::sync::Mutex;

use crate::board::global_config::{AuthMode, BoardEndpoint};

use super::{
    client::{AuthClient, complete_device_login},
    credential_store::{CredentialRecord, CredentialStore},
};

#[derive(Clone)]
pub struct TokenManager {
    endpoint: BoardEndpoint,
    store: Arc<CredentialStore>,
    auth: AuthClient,
    cached: Arc<Mutex<Option<CredentialRecord>>>,
}

#[derive(Debug, Clone)]
pub struct AuthStatus {
    pub kind: Option<&'static str>,
    pub expires_at: Option<chrono::DateTime<Utc>>,
    pub scope: Option<String>,
}

impl TokenManager {
    pub fn new(endpoint: BoardEndpoint) -> anyhow::Result<Self> {
        let canonical = endpoint.base_url.as_str().to_string();
        Ok(Self {
            store: Arc::new(CredentialStore::new(&canonical)),
            auth: AuthClient::new(endpoint.clone())?,
            endpoint,
            cached: Arc::new(Mutex::new(None)),
        })
    }

    pub async fn authorization_token(&self) -> anyhow::Result<Option<String>> {
        if self.endpoint.auth_mode == AuthMode::Disabled {
            return Ok(None);
        }
        // Automation credentials are intentionally process-local: do not write
        // them to the credential store or try to refresh an unknown token.
        if let Some(token) = environment_token() {
            return Ok(Some(token));
        }

        let mut cached = self.cached.lock().await;
        if cached.is_none() {
            *cached = self.load_record().await?;
        }
        let Some(record) = cached.clone() else {
            bail!("authentication is required; run `ostool login` or `ostool login --with-token`");
        };

        match record {
            CredentialRecord::PersonalAccessToken {
                token, expires_at, ..
            } => {
                if expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
                    self.delete_record().await?;
                    *cached = None;
                    bail!(
                        "personal access token has expired; create a new token in the web UI and import it with `ostool login --with-token`"
                    );
                }
                Ok(Some(token))
            }
            CredentialRecord::OAuthRefresh {
                access_token,
                access_expires_at,
                ..
            } if access_expires_at > Utc::now() + Duration::seconds(60) => Ok(Some(access_token)),
            CredentialRecord::OAuthRefresh { .. } => {
                // Refresh-token rotation makes concurrent refreshes unsafe. Lock,
                // then reload storage so another process can satisfy this request.
                let _refresh_lock = RefreshLock::acquire(&self.endpoint.base_url).await?;
                let latest = self.load_record().await?;
                let Some(CredentialRecord::OAuthRefresh {
                    refresh_token,
                    access_token,
                    access_expires_at,
                    ..
                }) = latest.as_ref()
                else {
                    *cached = None;
                    bail!(
                        "stored credentials changed; retry the command or run `ostool login` again"
                    );
                };
                if *access_expires_at > Utc::now() + Duration::seconds(60) {
                    *cached = latest.clone();
                    return Ok(Some(access_token.clone()));
                }
                // Even short-lived tokens may have rotated in another process.
                // Always refresh the record read under the cross-process lock.
                let refreshed = self.auth.refresh(refresh_token).await;
                match refreshed {
                    Ok(record) => {
                        let access_token = match &record {
                            CredentialRecord::OAuthRefresh { access_token, .. } => {
                                access_token.clone()
                            }
                            _ => unreachable!(),
                        };
                        self.save_record(record.clone()).await?;
                        *cached = Some(record);
                        Ok(Some(access_token))
                    }
                    Err(error)
                        if error.to_string().contains("invalid_grant")
                            || error.to_string().contains("invalid_token") =>
                    {
                        self.delete_record().await?;
                        *cached = None;
                        Err(error.context("login has expired; run `ostool login` again"))
                    }
                    Err(error) => Err(error),
                }
            }
        }
    }

    pub async fn login_device(&self) -> anyhow::Result<()> {
        if self.endpoint.auth_mode == AuthMode::Disabled {
            bail!("the configured board server has authentication disabled");
        }
        let record = complete_device_login(&self.auth).await?;
        let mut cached = self.cached.lock().await;
        let _refresh_lock = RefreshLock::acquire(&self.endpoint.base_url).await?;
        self.save_record(record.clone()).await?;
        *cached = Some(record);
        Ok(())
    }

    pub async fn import_personal_access_token(&self, token: String) -> anyhow::Result<()> {
        if self.endpoint.auth_mode == AuthMode::Disabled {
            bail!("the configured board server has authentication disabled");
        }
        let token = token.trim().to_string();
        if token.is_empty() {
            bail!("personal access token input is empty");
        }
        let record = CredentialRecord::PersonalAccessToken {
            token,
            expires_at: None,
            scope: None,
        };
        let mut cached = self.cached.lock().await;
        let _refresh_lock = RefreshLock::acquire(&self.endpoint.base_url).await?;
        self.save_record(record.clone()).await?;
        *cached = Some(record);
        Ok(())
    }

    pub async fn status(&self) -> anyhow::Result<AuthStatus> {
        if self.endpoint.auth_mode == AuthMode::Disabled {
            return Ok(AuthStatus {
                kind: None,
                expires_at: None,
                scope: None,
            });
        }
        if environment_token().is_some() {
            return Ok(AuthStatus {
                kind: Some("environment access token"),
                expires_at: None,
                scope: None,
            });
        }
        let record = self.load_record().await?;
        let (kind, expires_at, scope) = match record {
            Some(CredentialRecord::OAuthRefresh {
                access_expires_at,
                scope,
                ..
            }) => (Some("OAuth"), Some(access_expires_at), scope),
            Some(CredentialRecord::PersonalAccessToken {
                expires_at, scope, ..
            }) => (Some("personal access token"), expires_at, scope),
            None => (None, None, None),
        };
        Ok(AuthStatus {
            kind,
            expires_at,
            scope,
        })
    }

    pub async fn logout(&self) -> anyhow::Result<()> {
        let mut cached = self.cached.lock().await;
        let _refresh_lock = RefreshLock::acquire(&self.endpoint.base_url).await?;
        let record = self.load_record().await?;
        if let Some(CredentialRecord::OAuthRefresh { refresh_token, .. }) = record
            && let Err(error) = self.auth.revoke(&refresh_token).await
        {
            eprintln!("warning: failed to revoke OAuth session remotely: {error:#}");
        }
        self.delete_record().await?;
        *cached = None;
        Ok(())
    }

    /// Invalidate the current login without changing the original public API.
    pub async fn invalidate_after_unauthorized(&self) -> anyhow::Result<()> {
        self.invalidate_rejected_token(None).await
    }

    /// A stale HTTP response must not delete credentials saved by another command.
    pub(crate) async fn invalidate_rejected_token(
        &self,
        rejected: Option<&str>,
    ) -> anyhow::Result<()> {
        // Environment credentials never own or invalidate the saved browser login.
        if environment_token().is_some() {
            return Ok(());
        }
        let mut cached = self.cached.lock().await;
        let _refresh_lock = RefreshLock::acquire(&self.endpoint.base_url).await?;
        let latest = self.load_record().await?;
        let current = latest.as_ref().map(|record| match record {
            CredentialRecord::OAuthRefresh { access_token, .. } => access_token.clone(),
            CredentialRecord::PersonalAccessToken { token, .. } => token.clone(),
        });
        if rejected.is_some_and(|rejected| {
            current
                .as_deref()
                .is_some_and(|current| current != rejected)
        }) {
            *cached = latest;
            return Ok(());
        }
        self.delete_record().await?;
        *cached = None;
        Ok(())
    }

    // Linux's synchronous Secret Service implementation may create its own Tokio
    // runtime. Keep all keyring and fallback-file access off async runtime threads.
    async fn load_record(&self) -> anyhow::Result<Option<CredentialRecord>> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.load())
            .await
            .context("credential read task failed")?
    }
    async fn save_record(&self, record: CredentialRecord) -> anyhow::Result<()> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.save(&record))
            .await
            .context("credential write task failed")?
    }
    async fn delete_record(&self) -> anyhow::Result<()> {
        let store = self.store.clone();
        tokio::task::spawn_blocking(move || store.delete())
            .await
            .context("credential removal task failed")?
    }
}

fn environment_token() -> Option<String> {
    env::var_os("OSTOOL_BOARD_ACCESS_TOKEN")
        .map(|value| value.to_string_lossy().trim().to_string())
        .filter(|value| !value.is_empty())
}

struct RefreshLock(fs::File);

impl RefreshLock {
    async fn acquire(base_url: &reqwest::Url) -> anyhow::Result<Self> {
        use sha2::{Digest as _, Sha256};

        let cache_home = env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .context("HOME and XDG_CACHE_HOME are not set")?;
        let digest = Sha256::digest(base_url.as_str().as_bytes());
        let digest = digest
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>();
        let path = cache_home
            .join("ostool")
            .join("auth-locks")
            .join(format!("{digest}.lock"));

        tokio::task::spawn_blocking(move || {
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)
                    .with_context(|| format!("failed to create {}", parent.display()))?;
            }
            let file = fs::OpenOptions::new()
                .create(true)
                .read(true)
                .write(true)
                .truncate(false)
                .open(&path)
                .with_context(|| format!("failed to open {}", path.display()))?;
            fs4::FileExt::lock(&file)
                .with_context(|| format!("failed to lock {}", path.display()))?;
            Ok::<_, anyhow::Error>(Self(file))
        })
        .await
        .context("refresh lock task failed")?
    }
}

impl Drop for RefreshLock {
    fn drop(&mut self) {
        let _ = fs4::FileExt::unlock(&self.0);
    }
}
