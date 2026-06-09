use std::{
    io,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{
    build::config::Cargo,
    invocation::Invocation,
    project::variables::{self, VariableScope},
    utils::PathResultExt,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct HttpBootConfig {
    pub board_type: String,
    pub server: Option<String>,
    pub port: Option<u16>,
    pub remote_name: Option<String>,
    #[serde(default = "default_power_cycle")]
    pub power_cycle: bool,
    #[serde(default = "default_open_console")]
    pub open_console: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunHttpBootOptions {
    pub show_output: bool,
}

fn default_power_cycle() -> bool {
    true
}

fn default_open_console() -> bool {
    true
}

impl HttpBootConfig {
    fn replace_strings(&mut self, scope: &VariableScope) -> anyhow::Result<()> {
        self.board_type = variables::expand_variables(&self.board_type, scope)?;
        self.server = self
            .server
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.remote_name = self
            .remote_name
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        Ok(())
    }

    fn normalize(&mut self, config_name: &str) -> anyhow::Result<()> {
        normalize_required_string(&mut self.board_type, "board_type", config_name)?;
        normalize_optional_string(&mut self.server);
        normalize_optional_string(&mut self.remote_name);
        Ok(())
    }
}

pub fn default_config() -> HttpBootConfig {
    HttpBootConfig {
        board_type: "x86_64-uefi-http".to_string(),
        remote_name: Some("kernel.elf".to_string()),
        power_cycle: true,
        open_console: true,
        ..Default::default()
    }
}

pub async fn read_config_from_path_for_cargo(
    invocation: &Invocation,
    cargo: &Cargo,
    path: &Path,
) -> anyhow::Result<HttpBootConfig> {
    let scope = crate::build::cargo_variable_scope(invocation.project_layout(), cargo)?;
    read_httpboot_config_from_path(&scope, path).await
}

pub async fn ensure_config_for_cargo(
    invocation: &Invocation,
    cargo: &Cargo,
) -> anyhow::Result<HttpBootConfig> {
    let workspace_dir = invocation.workspace_dir().to_path_buf();
    ensure_config_in_dir_for_cargo(invocation, cargo, &workspace_dir).await
}

pub async fn ensure_config_in_dir_for_cargo(
    invocation: &Invocation,
    cargo: &Cargo,
    dir: &Path,
) -> anyhow::Result<HttpBootConfig> {
    let scope = crate::build::cargo_variable_scope(invocation.project_layout(), cargo)?;
    ensure_httpboot_config_in_dir(&scope, dir, default_config()).await
}

pub async fn ensure_config_in_dir(
    invocation: &Invocation,
    dir: &Path,
) -> anyhow::Result<HttpBootConfig> {
    let scope = invocation.variable_scope()?;
    ensure_httpboot_config_in_dir(&scope, dir, default_config()).await
}

pub async fn read_config_from_path(
    invocation: &Invocation,
    path: &Path,
) -> anyhow::Result<HttpBootConfig> {
    let scope = invocation.variable_scope()?;
    read_httpboot_config_from_path(&scope, path).await
}

pub async fn run_httpboot(
    _invocation: &mut Invocation,
    _config: &HttpBootConfig,
    _options: RunHttpBootOptions,
) -> anyhow::Result<()> {
    anyhow::bail!(
        "`ostool run httpboot` used the removed manifest-v1 HTTP Boot flow; use the \
         discovery-based AxVisor HTTP Boot publisher instead"
    )
}

pub(crate) async fn read_httpboot_config_from_path(
    scope: &VariableScope,
    path: &Path,
) -> anyhow::Result<HttpBootConfig> {
    let config_path = variables::expand_path_variables(path, scope)?;
    read_httpboot_config_at_path(scope, config_path).await
}

pub(crate) async fn ensure_httpboot_config_in_dir(
    scope: &VariableScope,
    dir: &Path,
    default_config: HttpBootConfig,
) -> anyhow::Result<HttpBootConfig> {
    let dir = variables::expand_path_variables(dir, scope)?;
    ensure_httpboot_config_at_path(scope, dir.join(".httpboot.toml"), default_config).await
}

async fn read_httpboot_config_at_path(
    scope: &VariableScope,
    config_path: PathBuf,
) -> anyhow::Result<HttpBootConfig> {
    let mut config: HttpBootConfig = fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("failed to read HTTP Boot config: {}", config_path.display()))
        .and_then(|content| {
            toml::from_str(&content).with_context(|| {
                format!(
                    "failed to parse HTTP Boot config: {}",
                    config_path.display()
                )
            })
        })?;
    config.replace_strings(scope)?;
    config.normalize(&format!("HTTP Boot config {}", config_path.display()))?;
    Ok(config)
}

async fn ensure_httpboot_config_at_path(
    scope: &VariableScope,
    config_path: PathBuf,
    default_config: HttpBootConfig,
) -> anyhow::Result<HttpBootConfig> {
    let mut config = match fs::read_to_string(&config_path).await {
        Ok(_) => return read_httpboot_config_at_path(scope, config_path).await,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let config = default_config;
            fs::write(&config_path, toml::to_string_pretty(&config)?)
                .await
                .with_path("failed to write file", &config_path)?;
            config
        }
        Err(err) => return Err(err.into()),
    };

    config.replace_strings(scope)?;
    config.normalize(&format!("HTTP Boot config {}", config_path.display()))?;
    Ok(config)
}

fn normalize_required_string(
    value: &mut String,
    field_name: &str,
    config_name: &str,
) -> anyhow::Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("`{field_name}` must not be empty in {config_name}");
    }
    if trimmed.len() != value.len() {
        *value = trimmed.to_string();
    }
    Ok(())
}

fn normalize_optional_string(value: &mut Option<String>) {
    if let Some(raw) = value {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            *value = None;
        } else if trimmed.len() != raw.len() {
            *raw = trimmed.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use super::{HttpBootConfig, ensure_httpboot_config_in_dir, read_httpboot_config_from_path};
    use crate::project::variables::VariableScope;

    #[test]
    fn httpboot_config_normalizes_supported_fields() {
        let mut config = HttpBootConfig {
            board_type: " x86-httpboot ".into(),
            server: Some(" 10.3.10.192 ".into()),
            port: Some(2999),
            remote_name: Some(" kernel.elf ".into()),
            power_cycle: true,
            open_console: true,
        };

        config.normalize("test config").unwrap();

        assert_eq!(config.board_type, "x86-httpboot");
        assert_eq!(config.server.as_deref(), Some("10.3.10.192"));
        assert_eq!(config.remote_name.as_deref(), Some("kernel.elf"));
    }

    #[test]
    fn httpboot_config_rejects_empty_board_type() {
        let mut config = HttpBootConfig {
            board_type: " ".into(),
            ..Default::default()
        };
        let tmp = tempdir().unwrap();
        let scope = test_scope(tmp.path());

        config.replace_strings(&scope).unwrap();
        let err = config.normalize("test config").unwrap_err();

        assert!(err.to_string().contains("board_type"));
    }

    #[tokio::test]
    async fn read_httpboot_config_rejects_removed_manifest_fields() {
        let tmp = tempdir().unwrap();
        let config_path = tmp.path().join(".httpboot.toml");
        std::fs::write(
            &config_path,
            r#"
board_type = "x86-httpboot"
kernel_load_addr = "0x200000"
"#,
        )
        .unwrap();
        let scope = test_scope(tmp.path());

        let err = read_httpboot_config_from_path(&scope, &config_path)
            .await
            .unwrap_err();

        let message = format!("{err:#}");
        assert!(message.contains("unknown field"));
        assert!(message.contains("kernel_load_addr"));
    }

    #[tokio::test]
    async fn ensure_httpboot_config_writes_minimal_default_file() {
        let tmp = tempdir().unwrap();
        let scope = test_scope(tmp.path());
        let default_config = HttpBootConfig {
            board_type: "demo-httpboot".into(),
            remote_name: Some("kernel.elf".into()),
            power_cycle: true,
            open_console: true,
            ..Default::default()
        };

        let config = ensure_httpboot_config_in_dir(&scope, tmp.path(), default_config)
            .await
            .unwrap();
        let content = std::fs::read_to_string(tmp.path().join(".httpboot.toml")).unwrap();

        assert_eq!(config.board_type, "demo-httpboot");
        assert!(content.contains("board_type = \"demo-httpboot\""));
        assert!(!content.contains("kernel_load_addr"));
        assert!(!content.contains("entry_point"));
    }

    fn test_scope(path: &std::path::Path) -> VariableScope {
        VariableScope::new(path.into(), path.into(), std::env::temp_dir())
    }
}
