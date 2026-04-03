use std::{env::current_dir, path::PathBuf};

use anyhow::Context as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    Tool, board::global_config::BoardGlobalConfig, run::shell_init::normalize_shell_init_config,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
pub struct BoardRunConfig {
    pub board_type: String,
    pub dtb_file: Option<String>,
    #[serde(default)]
    pub success_regex: Vec<String>,
    #[serde(default)]
    pub fail_regex: Vec<String>,
    pub shell_prefix: Option<String>,
    pub shell_init_cmd: Option<String>,
    pub server: Option<String>,
    pub port: Option<u16>,
}

impl BoardRunConfig {
    pub fn default_path(explicit_path: Option<PathBuf>) -> anyhow::Result<PathBuf> {
        match explicit_path {
            Some(path) => Ok(path),
            None => Ok(current_dir()?.join(".board.toml")),
        }
    }

    pub async fn load_or_create(
        tool: &Tool,
        explicit_path: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        let config_path = Self::default_path(explicit_path)?;
        let mut config = jkconfig::run::<Self>(config_path.clone(), false, &[])
            .await
            .with_context(|| format!("failed to load board config: {}", config_path.display()))?
            .ok_or_else(|| anyhow!("No board configuration obtained"))?;
        config.replace_strings(tool)?;
        config.normalize(&format!("board config {}", config_path.display()))?;
        Ok(config)
    }

    pub fn resolve_server(
        &self,
        cli_server: Option<&str>,
        cli_port: Option<u16>,
        global_config: &BoardGlobalConfig,
    ) -> (String, u16) {
        let server = cli_server
            .map(str::to_string)
            .or_else(|| self.server.clone())
            .unwrap_or_else(|| global_config.server_ip.clone());
        let port = cli_port.or(self.port).unwrap_or(global_config.port);
        (server, port)
    }

    pub fn apply_overrides(
        &mut self,
        tool: &Tool,
        board_type: Option<&str>,
        server: Option<&str>,
        port: Option<u16>,
    ) -> anyhow::Result<()> {
        if let Some(board_type) = board_type {
            self.board_type = tool.replace_string(board_type)?;
        }

        if let Some(server) = server {
            let server = tool.replace_string(server)?;
            let server = server.trim().to_string();
            if server.is_empty() {
                anyhow::bail!("board server override must not be empty");
            }
            self.server = Some(server);
        }

        if let Some(port) = port {
            if port == 0 {
                anyhow::bail!("board port override must be in 1..=65535");
            }
            self.port = Some(port);
        }

        self.normalize("board run arguments")
    }

    fn replace_strings(&mut self, tool: &Tool) -> anyhow::Result<()> {
        self.board_type = tool.replace_string(&self.board_type)?;
        self.dtb_file = self
            .dtb_file
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.success_regex = self
            .success_regex
            .iter()
            .map(|value| tool.replace_string(value))
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.fail_regex = self
            .fail_regex
            .iter()
            .map(|value| tool.replace_string(value))
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.shell_prefix = self
            .shell_prefix
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.shell_init_cmd = self
            .shell_init_cmd
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.server = self
            .server
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        Ok(())
    }

    fn normalize(&mut self, config_name: &str) -> anyhow::Result<()> {
        self.board_type = self.board_type.trim().to_string();
        if let Some(dtb_file) = self.dtb_file.as_mut() {
            let trimmed = dtb_file.trim();
            if trimmed.is_empty() {
                self.dtb_file = None;
            } else if trimmed.len() != dtb_file.len() {
                *dtb_file = trimmed.to_string();
            }
        }
        if self.board_type.is_empty() {
            anyhow::bail!("`board_type` must not be empty in {config_name}");
        }
        normalize_shell_init_config(
            &mut self.shell_prefix,
            &mut self.shell_init_cmd,
            config_name,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::BoardRunConfig;
    use crate::{Tool, board::global_config::BoardGlobalConfig};

    #[test]
    fn board_run_config_parses_and_normalizes_shell_fields() {
        let mut config: BoardRunConfig = toml::from_str(
            r#"
board_type = " orangepi5plus "
dtb_file = " ${workspace}/board.dtb "
success_regex = ["ok"]
fail_regex = ["panic"]
shell_prefix = " login: "
shell_init_cmd = " root "
server = "10.0.0.2"
port = 9000
"#,
        )
        .unwrap();

        config.normalize("test board config").unwrap();

        assert_eq!(config.board_type, "orangepi5plus");
        assert_eq!(config.dtb_file.as_deref(), Some("${workspace}/board.dtb"));
        assert_eq!(config.shell_prefix.as_deref(), Some("login:"));
        assert_eq!(config.shell_init_cmd.as_deref(), Some("root"));
        assert_eq!(
            config.resolve_server(
                Some("127.0.0.1"),
                None,
                &BoardGlobalConfig {
                    server_ip: "localhost".into(),
                    port: 2999,
                }
            ),
            ("127.0.0.1".to_string(), 9000)
        );
    }

    #[test]
    fn board_run_config_default_path_uses_current_dir() {
        let path = BoardRunConfig::default_path(None).unwrap();
        assert_eq!(
            path.file_name().and_then(|name| name.to_str()),
            Some(".board.toml")
        );
    }

    #[test]
    fn board_run_config_apply_overrides_replaces_board_type_and_server() {
        let mut config: BoardRunConfig = toml::from_str(
            r#"
board_type = "orangepi5plus"
server = "10.0.0.2"
port = 9000
"#,
        )
        .unwrap();
        let tool = Tool::new(Default::default()).unwrap();

        config
            .apply_overrides(&tool, Some(" rk3568 "), Some(" 127.0.0.1 "), Some(7000))
            .unwrap();

        assert_eq!(config.board_type, "rk3568");
        assert_eq!(config.server.as_deref(), Some("127.0.0.1"));
        assert_eq!(config.port, Some(7000));
    }
}
