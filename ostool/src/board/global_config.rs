use std::{
    env, fs,
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use reqwest::Url;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_BOARD_SERVER: &str = "http://localhost";

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
    pub fn new(server: &str, port: Option<u16>, auth_mode: AuthMode) -> anyhow::Result<Self> {
        let mut base_url =
            Url::parse(server).with_context(|| format!("invalid board server URL `{server}`"))?;
        if !matches!(base_url.scheme(), "http" | "https") {
            bail!("board server URL must use http or https");
        }
        if base_url.host().is_none() {
            bail!("board server URL must include a host");
        }
        if auth_mode == AuthMode::Required && base_url.scheme() != "https" {
            bail!("authenticated board server URL must use https");
        }
        if let Some(port) = port {
            if port == 0 {
                bail!("board server port must be in 1..=65535");
            }
            base_url
                .set_port(Some(port))
                .map_err(|_| anyhow::anyhow!("invalid board server port `{port}`"))?;
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
#[serde(deny_unknown_fields)]
pub struct BoardGlobalConfig {
    /// Complete board service URL, including its scheme and optional base path.
    #[serde(default = "default_server")]
    pub server: String,
    /// Optional port override for `server`.
    #[serde(default)]
    pub port: Option<u16>,
    #[serde(default)]
    pub auth_mode: AuthMode,
}

impl Default for BoardGlobalConfig {
    fn default() -> Self {
        Self {
            server: default_server(),
            port: None,
            auth_mode: AuthMode::Disabled,
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

    pub fn resolve_endpoint(
        &self,
        cli_server: Option<&str>,
        cli_port: Option<u16>,
    ) -> anyhow::Result<BoardEndpoint> {
        self.board.resolve_endpoint(cli_server, cli_port)
    }
}

impl BoardGlobalConfig {
    pub fn validate(&self, path: &Path) -> anyhow::Result<()> {
        if self.server.trim().is_empty() {
            bail!("`board.server` must not be empty in {}", path.display());
        }
        BoardEndpoint::new(&self.server, self.port, self.auth_mode)?;
        Ok(())
    }

    pub fn resolve_endpoint(
        &self,
        cli_server: Option<&str>,
        cli_port: Option<u16>,
    ) -> anyhow::Result<BoardEndpoint> {
        let server = cli_server.unwrap_or(&self.server);
        let port = cli_port.or(self.port);
        BoardEndpoint::new(server, port, self.auth_mode)
    }
}

fn default_server() -> String {
    DEFAULT_BOARD_SERVER.to_string()
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

    use super::{AuthMode, BoardGlobalConfig, BoardGlobalConfigFile, LoadedBoardGlobalConfig};

    #[test]
    fn load_or_create_creates_url_based_default_config_when_missing() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".ostool/config.toml");

        let loaded = LoadedBoardGlobalConfig::load_or_create_at(&path).unwrap();

        assert!(loaded.created);
        assert_eq!(loaded.board.server, "http://localhost");
        assert_eq!(loaded.board.port, None);
        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("server = \"http://localhost\""));
        assert!(!content.contains("server_ip"));
    }

    #[test]
    fn resolve_endpoint_uses_url_port_or_scheme_default() {
        let config = BoardGlobalConfig {
            server: "https://board.example.com:9443/base".into(),
            port: None,
            auth_mode: AuthMode::Required,
        };

        assert_eq!(
            config
                .resolve_endpoint(None, None)
                .unwrap()
                .base_url
                .as_str(),
            "https://board.example.com:9443/base/"
        );
        assert_eq!(
            config
                .resolve_endpoint(None, Some(8443))
                .unwrap()
                .base_url
                .as_str(),
            "https://board.example.com:8443/base/"
        );
    }

    #[test]
    fn required_authentication_requires_https() {
        let config = BoardGlobalConfig {
            server: "http://203.0.113.10:8443".into(),
            port: None,
            auth_mode: AuthMode::Required,
        };
        assert!(config.resolve_endpoint(None, None).is_err());
    }

    #[test]
    fn save_persists_url_and_optional_port() {
        let temp = tempdir().unwrap();
        let path = temp.path().join(".ostool/config.toml");
        let mut loaded = LoadedBoardGlobalConfig::load_or_create_at(&path).unwrap();

        loaded.board = BoardGlobalConfig {
            server: "http://10.0.0.2".into(),
            port: Some(9000),
            auth_mode: AuthMode::Disabled,
        };
        loaded.save().unwrap();

        let reloaded = LoadedBoardGlobalConfig::load_or_create_at(&path).unwrap();
        assert!(!reloaded.created);
        assert_eq!(reloaded.board.server, "http://10.0.0.2");
        assert_eq!(reloaded.board.port, Some(9000));
    }

    #[test]
    fn legacy_config_fields_are_rejected() {
        let err = toml::from_str::<BoardGlobalConfigFile>(
            r#"
                [board]
                server_ip = "10.0.0.2"
                port = 9000
            "#,
        )
        .unwrap_err();
        assert!(err.to_string().contains("server_ip"));
    }
}
