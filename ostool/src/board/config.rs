use std::{env::current_dir, path::PathBuf};

use anyhow::Context as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{
    board::global_config::{AuthMode, BoardEndpoint, BoardGlobalConfig},
    project::variables::{self, VariableScope},
    run::shell_init::normalize_shell_init_config,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoardSessionProgram {
    /// Program path relative to the board configuration's session root.
    pub path: PathBuf,
    /// Literal argv entries. Project and board-session placeholders are expanded
    /// before each entry is POSIX-shell quoted.
    #[serde(default)]
    pub args: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct BoardRunConfig {
    pub board_type: String,
    /// Files shared with the board for the duration of one session.
    ///
    /// Paths are relative to the board configuration file and keep the same
    /// relative path on the session HTTP endpoint.
    #[serde(default)]
    pub session_files: Vec<PathBuf>,
    /// Program uploaded, downloaded, and executed for this board session.
    #[serde(default)]
    pub session_program: Option<BoardSessionProgram>,
    pub dtb_file: Option<String>,
    pub kernel_load_addr: Option<String>,
    pub fit_load_addr: Option<String>,
    pub bootm_addr: Option<String>,
    #[serde(default)]
    pub success_regex: Vec<String>,
    #[serde(default)]
    pub fail_regex: Vec<String>,
    #[serde(default)]
    pub uboot_cmd: Option<Vec<String>>,
    pub shell_prefix: Option<String>,
    pub shell_init_cmd: Option<String>,
    pub timeout: Option<u64>,
    pub auth_mode: Option<AuthMode>,
    /// Complete board service URL. `port` optionally overrides its port.
    pub server: Option<String>,
    pub port: Option<u16>,
}

impl BoardRunConfig {
    pub(crate) fn default_path(explicit_path: Option<PathBuf>) -> anyhow::Result<PathBuf> {
        match explicit_path {
            Some(path) => Ok(path),
            None => Ok(current_dir()?.join(".board.toml")),
        }
    }

    pub(crate) async fn load_or_create(
        scope: &VariableScope,
        explicit_path: Option<PathBuf>,
    ) -> anyhow::Result<Self> {
        let config_path = Self::default_path(explicit_path)?;
        let mut config = jkconfig::run::<Self>(config_path.clone(), false, &[])
            .await
            .with_context(|| format!("failed to load board config: {}", config_path.display()))?
            .ok_or_else(|| anyhow!("No board configuration obtained"))?;
        config.replace_strings(scope)?;
        config.normalize(&format!("board config {}", config_path.display()))?;
        Ok(config)
    }

    pub(crate) fn read_from_path(scope: &VariableScope, path: PathBuf) -> anyhow::Result<Self> {
        let mut config: Self = toml::from_str(
            &std::fs::read_to_string(&path)
                .with_context(|| format!("failed to read board config: {}", path.display()))?,
        )
        .with_context(|| format!("failed to parse board config: {}", path.display()))?;
        config.replace_strings(scope)?;
        config.normalize(&format!("board config {}", path.display()))?;
        Ok(config)
    }

    pub(crate) fn resolve_endpoint(
        &self,
        cli_server: Option<&str>,
        cli_port: Option<u16>,
        global_config: &BoardGlobalConfig,
    ) -> anyhow::Result<BoardEndpoint> {
        let auth_mode = self.auth_mode.unwrap_or(global_config.auth_mode);
        let server = cli_server
            .or(self.server.as_deref())
            .unwrap_or(&global_config.server);
        let port = cli_port.or(self.port).or(global_config.port);
        BoardEndpoint::new(server, port, auth_mode)
    }

    pub(crate) fn apply_overrides(
        &mut self,
        scope: &VariableScope,
        board_type: Option<&str>,
        server: Option<&str>,
        port: Option<u16>,
    ) -> anyhow::Result<()> {
        if let Some(board_type) = board_type {
            self.board_type = variables::expand_variables(board_type, scope)?;
        }

        if let Some(server) = server {
            let server = variables::expand_variables(server, scope)?;
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

    fn replace_strings(&mut self, scope: &VariableScope) -> anyhow::Result<()> {
        self.board_type = variables::expand_variables(&self.board_type, scope)?;
        self.dtb_file = self
            .dtb_file
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.kernel_load_addr = self
            .kernel_load_addr
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.fit_load_addr = self
            .fit_load_addr
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.bootm_addr = self
            .bootm_addr
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.success_regex = self
            .success_regex
            .iter()
            .map(|value| variables::expand_variables(value, scope))
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.fail_regex = self
            .fail_regex
            .iter()
            .map(|value| variables::expand_variables(value, scope))
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.uboot_cmd = self
            .uboot_cmd
            .as_ref()
            .map(|values| {
                values
                    .iter()
                    .map(|value| variables::expand_variables(value, scope))
                    .collect::<anyhow::Result<Vec<_>>>()
            })
            .transpose()?;
        self.shell_prefix = self
            .shell_prefix
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.shell_init_cmd = self
            .shell_init_cmd
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        if let Some(program) = self.session_program.as_mut() {
            program.args = program
                .args
                .iter()
                .map(|value| variables::expand_variables(value, scope))
                .collect::<anyhow::Result<Vec<_>>>()?;
        }
        self.server = self
            .server
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
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
        normalize_optional_string(&mut self.kernel_load_addr);
        normalize_optional_string(&mut self.fit_load_addr);
        normalize_optional_string(&mut self.bootm_addr);
        if let Some(commands) = self.uboot_cmd.as_mut() {
            commands.retain_mut(|command| {
                let trimmed = command.trim();
                if trimmed.is_empty() {
                    false
                } else {
                    if trimmed.len() != command.len() {
                        *command = trimmed.to_string();
                    }
                    true
                }
            });
            if commands.is_empty() {
                self.uboot_cmd = None;
            }
        }
        if self.board_type.is_empty() {
            anyhow::bail!("`board_type` must not be empty in {config_name}");
        }
        normalize_shell_init_config(
            &mut self.shell_prefix,
            &mut self.shell_init_cmd,
            config_name,
        )?;
        if let Some(program) = self.session_program.as_ref() {
            if self.shell_init_cmd.is_some() {
                anyhow::bail!(
                    "`session_program` and `shell_init_cmd` are mutually exclusive in {config_name}"
                );
            }
            if self.shell_prefix.is_none() {
                anyhow::bail!("`session_program` requires `shell_prefix` in {config_name}");
            }
            if program.path.as_os_str().is_empty() {
                anyhow::bail!("`session_program.path` must not be empty in {config_name}");
            }
            if program.path.to_string_lossy().contains("${") {
                anyhow::bail!("`session_program.path` must not contain variables in {config_name}");
            }
            if let Some(argument) = program
                .args
                .iter()
                .find(|argument| argument.contains(['\0', '\r', '\n']))
            {
                anyhow::bail!(
                    "`session_program.args` must not contain NUL or newline characters in \
                     {config_name}: {argument:?}"
                );
            }
        }
        Ok(())
    }
}

fn normalize_optional_string(value: &mut Option<String>) {
    if let Some(inner) = value.as_mut() {
        let trimmed = inner.trim();
        if trimmed.is_empty() {
            *value = None;
        } else if trimmed.len() != inner.len() {
            *inner = trimmed.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{BoardRunConfig, BoardSessionProgram};
    use crate::{
        board::global_config::BoardGlobalConfig,
        board::{ensure_run_config_in_dir, read_run_config_from_path},
        build::config::{BuildConfig, BuildSystem, Cargo},
        invocation::{Invocation, InvocationOptions},
    };
    use std::collections::HashMap;
    use std::path::PathBuf;

    #[derive(serde::Serialize)]
    struct LegacyBoardRunConfigFixture {
        board_type: String,
    }

    fn make_invocation(dir: &std::path::Path) -> Invocation {
        Invocation::new(InvocationOptions::new(
            Some(dir.to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap()
    }

    #[test]
    fn board_run_config_parses_and_normalizes_shell_fields() {
        let mut config: BoardRunConfig = toml::from_str(
            r#"
board_type = " orangepi5plus "
dtb_file = " ${workspace}/board.dtb "
kernel_load_addr = " 0x80200000 "
fit_load_addr = " 0x82200000 "
bootm_addr = " 0x82200000 "
success_regex = ["ok"]
fail_regex = ["panic"]
uboot_cmd = [" run bootcmd "]
shell_prefix = " login: "
shell_init_cmd = " root "
timeout = 15
server = "http://10.0.0.2"
port = 9000
"#,
        )
        .unwrap();

        config.normalize("test board config").unwrap();

        assert_eq!(config.board_type, "orangepi5plus");
        assert_eq!(config.dtb_file.as_deref(), Some("${workspace}/board.dtb"));
        assert_eq!(config.kernel_load_addr.as_deref(), Some("0x80200000"));
        assert_eq!(config.fit_load_addr.as_deref(), Some("0x82200000"));
        assert_eq!(config.bootm_addr.as_deref(), Some("0x82200000"));
        assert_eq!(config.uboot_cmd, Some(vec!["run bootcmd".to_string()]));
        assert_eq!(config.shell_prefix.as_deref(), Some("login:"));
        assert_eq!(config.shell_init_cmd.as_deref(), Some("root"));
        assert_eq!(config.timeout, Some(15));
        assert_eq!(
            config
                .resolve_endpoint(
                    Some("http://127.0.0.1"),
                    None,
                    &BoardGlobalConfig::default(),
                )
                .unwrap()
                .base_url
                .as_str(),
            "http://127.0.0.1:9000/"
        );
    }

    #[test]
    fn board_run_config_session_files_toml_round_trip() {
        let config = BoardRunConfig {
            board_type: "orangepi-5-plus".to_string(),
            session_files: vec![
                PathBuf::from("iperf-smoke.sh"),
                PathBuf::from("tools/network/probe.sh"),
            ],
            ..Default::default()
        };

        let encoded = toml::to_string(&config).unwrap();
        let decoded: BoardRunConfig = toml::from_str(&encoded).unwrap();

        assert_eq!(decoded, config);
    }

    #[test]
    fn board_run_config_session_program_toml_round_trip() {
        let config = BoardRunConfig {
            board_type: "aka-00-sg2002".to_string(),
            shell_prefix: Some("root@starry:".to_string()),
            session_program: Some(BoardSessionProgram {
                path: PathBuf::from("bin/sg2002-libuvc-init"),
                args: vec![
                    "--server".to_string(),
                    "${boardServerIp}".to_string(),
                    "argument with spaces".to_string(),
                ],
            }),
            ..Default::default()
        };

        let encoded = toml::to_string(&config).unwrap();
        let decoded: BoardRunConfig = toml::from_str(&encoded).unwrap();

        assert_eq!(decoded, config);
    }

    #[test]
    fn legacy_board_run_config_defaults_to_no_session_files() {
        let fixture = LegacyBoardRunConfigFixture {
            board_type: "orangepi-5-plus".to_string(),
        };
        let encoded = toml::to_string(&fixture).unwrap();

        let decoded: BoardRunConfig = toml::from_str(&encoded).unwrap();

        assert!(decoded.session_files.is_empty());
        assert!(decoded.session_program.is_none());
    }

    #[test]
    fn session_program_rejects_shell_init_command_and_missing_prefix() {
        let mut conflicting = BoardRunConfig {
            board_type: "aka-00-sg2002".to_string(),
            shell_prefix: Some("root@starry:".to_string()),
            shell_init_cmd: Some("echo legacy".to_string()),
            session_program: Some(BoardSessionProgram {
                path: PathBuf::from("bin/probe"),
                args: Vec::new(),
            }),
            ..Default::default()
        };
        assert!(
            conflicting
                .normalize("test board config")
                .unwrap_err()
                .to_string()
                .contains("mutually exclusive")
        );

        let mut missing_prefix = BoardRunConfig {
            board_type: "aka-00-sg2002".to_string(),
            session_program: Some(BoardSessionProgram {
                path: PathBuf::from("bin/probe"),
                args: Vec::new(),
            }),
            ..Default::default()
        };
        assert!(
            missing_prefix
                .normalize("test board config")
                .unwrap_err()
                .to_string()
                .contains("shell_prefix")
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
server = "http://10.0.0.2"
port = 9000
"#,
        )
        .unwrap();
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/lib.rs"), "").unwrap();
        let invocation = make_invocation(tmp.path());

        config
            .apply_overrides(
                &invocation.variable_scope().unwrap(),
                Some(" rk3568 "),
                Some(" http://127.0.0.1 "),
                Some(7000),
            )
            .unwrap();

        assert_eq!(config.board_type, "rk3568");
        assert_eq!(config.server.as_deref(), Some("http://127.0.0.1"));
        assert_eq!(config.port, Some(7000));
    }

    #[tokio::test]
    async fn read_board_run_config_from_path_normalizes_loaded_values() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/lib.rs"), "").unwrap();
        let config_path = tmp.path().join("custom.board.toml");
        std::fs::write(
            &config_path,
            r#"
board_type = " rk3568 "
shell_prefix = " login: "
shell_init_cmd = " root "
timeout = 8
"#,
        )
        .unwrap();

        let invocation = make_invocation(tmp.path());

        let config = read_run_config_from_path(&invocation, &config_path)
            .await
            .unwrap();
        assert_eq!(config.board_type, "rk3568");
        assert_eq!(config.shell_prefix.as_deref(), Some("login:"));
        assert_eq!(config.shell_init_cmd.as_deref(), Some("root"));
        assert_eq!(config.timeout, Some(8));
    }

    #[tokio::test]
    async fn ensure_board_run_config_in_dir_replaces_package_variables() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"kernel\"]\nresolver = \"3\"\n",
        )
        .unwrap();

        let app_dir = tmp.path().join("app");
        std::fs::create_dir_all(app_dir.join("src")).unwrap();
        std::fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let kernel_dir = tmp.path().join("kernel");
        std::fs::create_dir_all(kernel_dir.join("src")).unwrap();
        std::fs::write(
            kernel_dir.join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(kernel_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        std::fs::write(
            tmp.path().join(".board.toml"),
            r#"
board_type = "kernel-board"
dtb_file = "${package}/board.dtb"
"#,
        )
        .unwrap();

        let mut invocation = make_invocation(&app_dir);
        crate::build::activate_build_config(
            &mut invocation,
            &BuildConfig {
                system: BuildSystem::Cargo(Box::new(Cargo {
                    env: HashMap::new(),
                    target: "aarch64-unknown-none".into(),
                    package: "kernel".into(),
                    bin: None,
                    test: None,
                    features: vec![],
                    log: None,
                    extra_config: None,
                    profile: None,
                    disable_someboot_build_config: false,
                    args: vec![],
                    pre_build_cmds: vec![],
                    post_build_cmds: vec![],
                    to_bin: false,
                })),
            },
            None,
        )
        .unwrap();

        let config = ensure_run_config_in_dir(&invocation, tmp.path())
            .await
            .unwrap();
        let expected = kernel_dir.join("board.dtb").display().to_string();
        assert_eq!(config.dtb_file.as_deref(), Some(expected.as_str()));
    }
}
