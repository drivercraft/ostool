use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use reqwest::Url;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_BOARD_SERVER_IP: &str = "localhost";
pub const DEFAULT_BOARD_SERVER_PORT: u16 = 2999;

#[derive(Debug, Clone, Copy, Serialize, Deserialize, JsonSchema, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    #[default]
    Disabled,
    Required,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoardEndpoint {
    pub base_url: Url,
    pub auth_mode: AuthMode,
}

impl BoardEndpoint {
    pub fn new(server_url: &str, auth_mode: AuthMode) -> anyhow::Result<Self> {
        let mut base_url = Url::parse(server_url)
            .with_context(|| format!("invalid board server URL `{server_url}`"))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            bail!("board server URL must use http or https");
        }
        if base_url.host().is_none() {
            bail!("board server URL must include a host");
        }
        if auth_mode == AuthMode::Required && base_url.scheme() != "https" {
            bail!("authenticated board server URL must use https");
        }
        // Url::join treats a base without a trailing slash as a file path and
        // would discard its final path component when resolving API endpoints.
        if !base_url.path().ends_with('/') {
            let path = format!("{}/", base_url.path());
            base_url.set_path(&path);
        }
        Ok(Self {
            base_url,
            auth_mode,
        })
    }

    pub fn websocket_base_url(&self) -> anyhow::Result<Url> {
        let mut url = self.base_url.clone();
        match url.scheme() {
            "http" => url
                .set_scheme("ws")
                .map_err(|_| anyhow::anyhow!("failed to build websocket URL"))?,
            "https" => url
                .set_scheme("wss")
                .map_err(|_| anyhow::anyhow!("failed to build websocket URL"))?,
            _ => unreachable!("BoardEndpoint validates schemes"),
        }
        Ok(url)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct BoardGlobalConfigFile {
    #[serde(default)]
    pub board: BoardGlobalConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BoardGlobalConfig {
    /// Complete board server URL. When set, it takes precedence over the legacy host and port.
    #[serde(default)]
    pub server_url: Option<String>,
    #[serde(default)]
    pub auth_mode: AuthMode,
    #[serde(default = "default_server_ip")]
    pub server_ip: String,
    #[serde(default = "default_server_port")]
    pub port: u16,
}

impl Default for BoardGlobalConfig {
    fn default() -> Self {
        Self {
            server_url: None,
            auth_mode: AuthMode::Disabled,
            server_ip: DEFAULT_BOARD_SERVER_IP.to_string(),
            port: DEFAULT_BOARD_SERVER_PORT,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LoadedBoardGlobalConfig {
    pub path: PathBuf,
    pub board: BoardGlobalConfig,
    pub created: bool,
}

impl LoadedBoardGlobalConfig {
    pub fn load_or_create() -> anyhow::Result<Self> {
        let path = default_config_path()?;
        Self::load_or_create_at(&path)
    }

    pub fn load_or_create_at(path: &Path) -> anyhow::Result<Self> {
        match fs::read_to_string(path) {
            Ok(content) => {
                let file: BoardGlobalConfigFile = toml::from_str(&content)
                    .with_context(|| format!("failed to parse {}", path.display()))?;
                file.board.validate(path)?;
                Ok(Self {
                    path: path.to_path_buf(),
                    board: file.board,
                    created: false,
                })
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let file = BoardGlobalConfigFile::default();
                write_config_file(path, &file)?;
                Ok(Self {
                    path: path.to_path_buf(),
                    board: file.board,
                    created: true,
                })
            }
            Err(err) => Err(err).with_context(|| format!("failed to read {}", path.display())),
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        write_config_file(
            &self.path,
            &BoardGlobalConfigFile {
                board: self.board.clone(),
            },
        )
    }

    pub fn resolve_server(&self, cli_server: Option<&str>, cli_port: Option<u16>) -> (String, u16) {
        self.board.resolve_server(cli_server, cli_port)
    }

    pub fn resolve_endpoint(
        &self,
        cli_server_url: Option<&str>,
        cli_server: Option<&str>,
        cli_port: Option<u16>,
    ) -> anyhow::Result<BoardEndpoint> {
        self.board
            .resolve_endpoint(cli_server_url, cli_server, cli_port)
    }
}

impl BoardGlobalConfig {
    pub fn resolve_server(&self, cli_server: Option<&str>, cli_port: Option<u16>) -> (String, u16) {
        let server_ip = cli_server
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(str::to_string)
            .unwrap_or_else(|| self.server_ip.clone());
        let port = cli_port.unwrap_or(self.port);
        (server_ip, port)
    }

    pub fn validate(&self, path: &Path) -> anyhow::Result<()> {
        if self.server_ip.trim().is_empty() {
            bail!("`board.server_ip` must not be empty in {}", path.display());
        }
        if self.port == 0 {
            bail!("`board.port` must be in 1..=65535 in {}", path.display());
        }
        if let Some(server_url) = self.server_url.as_deref() {
            if self.server_ip != DEFAULT_BOARD_SERVER_IP || self.port != DEFAULT_BOARD_SERVER_PORT {
                bail!(
                    "`board.server_url` cannot be combined with non-default `board.server_ip` or `board.port` in {}",
                    path.display()
                );
            }
            BoardEndpoint::new(server_url, self.auth_mode)?;
        }
        Ok(())
    }

    pub fn resolve_endpoint(
        &self,
        cli_server_url: Option<&str>,
        cli_server: Option<&str>,
        cli_port: Option<u16>,
    ) -> anyhow::Result<BoardEndpoint> {
        if cli_server_url.is_some() && (cli_server.is_some() || cli_port.is_some()) {
            bail!("--server-url cannot be used with --server or --port");
        }

        if let Some(server_url) = cli_server_url {
            return BoardEndpoint::new(server_url, self.auth_mode);
        }
        if cli_server.is_none()
            && cli_port.is_none()
            && let Some(server_url) = self.server_url.as_deref()
        {
            return BoardEndpoint::new(server_url, self.auth_mode);
        }

        let (server, port) = self.resolve_server(cli_server, cli_port);
        BoardEndpoint::new(&format!("http://{server}:{port}"), AuthMode::Disabled)
    }
}

fn default_server_ip() -> String {
    DEFAULT_BOARD_SERVER_IP.to_string()
}

const fn default_server_port() -> u16 {
    DEFAULT_BOARD_SERVER_PORT
}

fn default_config_path() -> anyhow::Result<PathBuf> {
    let home = env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".ostool").join("config.toml"))
}

fn write_config_file(path: &Path, file: &BoardGlobalConfigFile) -> anyhow::Result<()> {
    file.board.validate(path)?;
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("failed to create {}", parent.display()))?;
    }
    fs::write(path, toml::to_string_pretty(file)?)
        .with_context(|| format!("failed to write {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{AuthMode, BoardGlobalConfig, LoadedBoardGlobalConfig};

    #[test]
    fn load_or_create_creates_default_config_when_missing() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".ostool/config.toml");

        let loaded = LoadedBoardGlobalConfig::load_or_create_at(&path).unwrap();

        assert!(loaded.created);
        assert_eq!(loaded.board.server_ip, "localhost");
        assert_eq!(loaded.board.port, 2999);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("[board]"));
        assert!(content.contains("server_ip = \"localhost\""));
        assert!(content.contains("port = 2999"));
    }

    #[test]
    fn resolve_server_prefers_cli_over_global_defaults() {
        let config = BoardGlobalConfig {
            server_ip: "10.0.0.2".into(),
            port: 8000,
            ..BoardGlobalConfig::default()
        };

        assert_eq!(
            config.resolve_server(Some("192.168.1.2"), Some(9000)),
            ("192.168.1.2".to_string(), 9000)
        );
        assert_eq!(
            config.resolve_server(None, None),
            ("10.0.0.2".to_string(), 8000)
        );
    }

    #[test]
    fn required_authentication_requires_https() {
        let config = BoardGlobalConfig {
            server_url: Some("http://203.0.113.10:8443".into()),
            auth_mode: AuthMode::Required,
            ..BoardGlobalConfig::default()
        };
        assert!(config.resolve_endpoint(None, None, None).is_err());
    }

    #[test]
    fn https_server_url_is_used_for_authenticated_endpoint() {
        let config = BoardGlobalConfig {
            server_url: Some("https://203.0.113.10:8443".into()),
            auth_mode: AuthMode::Required,
            ..BoardGlobalConfig::default()
        };
        let endpoint = config.resolve_endpoint(None, None, None).unwrap();
        assert_eq!(endpoint.base_url.as_str(), "https://203.0.113.10:8443/");
        assert_eq!(endpoint.auth_mode, AuthMode::Required);
    }

    #[test]
    fn save_persists_updated_values() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".ostool/config.toml");
        let mut loaded = LoadedBoardGlobalConfig::load_or_create_at(&path).unwrap();

        loaded.board.server_ip = "10.0.0.2".into();
        loaded.board.port = 9000;
        loaded.save().unwrap();

        let reloaded = LoadedBoardGlobalConfig::load_or_create_at(&path).unwrap();
        assert!(!reloaded.created);
        assert_eq!(reloaded.board.server_ip, "10.0.0.2");
        assert_eq!(reloaded.board.port, 9000);
    }
}
