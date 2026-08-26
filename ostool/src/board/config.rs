use std::{env::current_dir, path::PathBuf};

use anyhow::Context as _;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};

use crate::{
    board::global_config::{AuthMode, BoardEndpoint, BoardGlobalConfig},
    project::variables::{self, VariableScope},
    run::{
        shell_check::{ShellCheckStep, normalize_shell_check_steps},
        uboot::UbootPromptConfig,
    },
};

#[derive(Debug, Clone, Serialize, JsonSchema, Default, PartialEq, Eq)]
#[schemars(deny_unknown_fields)]
pub struct BoardRunConfig {
    pub board_type: String,
    /// Files shared with the board for the duration of one session.
    ///
    /// Paths are relative to the board configuration file and keep the same
    /// relative path on the session HTTP endpoint.
    #[serde(default)]
    pub session_files: Vec<PathBuf>,
    pub dtb_file: Option<String>,
    pub kernel_load_addr: Option<String>,
    pub fit_load_addr: Option<String>,
    pub bootm_addr: Option<String>,
    #[serde(default)]
    pub fail_regex: Vec<String>,
    #[serde(default)]
    pub uboot_cmd: Option<Vec<String>>,
    #[serde(default)]
    pub prompt: UbootPromptConfig,
    /// Ordered shell commands and result checks.
    #[serde(default)]
    pub shell_check_steps: Vec<ShellCheckStep>,
    pub timeout: Option<u64>,
    pub auth_mode: Option<AuthMode>,
    /// Complete board service URL. `port` optionally overrides its port.
    pub server: Option<String>,
    pub port: Option<u16>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct BoardRunConfigWire {
    board_type: String,
    #[serde(default)]
    session_files: Vec<PathBuf>,
    dtb_file: Option<String>,
    kernel_load_addr: Option<String>,
    fit_load_addr: Option<String>,
    bootm_addr: Option<String>,
    #[serde(default)]
    fail_regex: Vec<String>,
    #[serde(default)]
    uboot_cmd: Option<Vec<String>>,
    #[serde(default)]
    prompt: UbootPromptConfig,
    #[serde(default)]
    shell_check_steps: Vec<ShellCheckStep>,
    timeout: Option<u64>,
    auth_mode: Option<AuthMode>,
    server: Option<String>,
    port: Option<u16>,
}

impl<'de> Deserialize<'de> for BoardRunConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = BoardRunConfigWire::deserialize(deserializer)?;
        Ok(Self {
            board_type: wire.board_type,
            session_files: wire.session_files,
            dtb_file: wire.dtb_file,
            kernel_load_addr: wire.kernel_load_addr,
            fit_load_addr: wire.fit_load_addr,
            bootm_addr: wire.bootm_addr,
            fail_regex: wire.fail_regex,
            uboot_cmd: wire.uboot_cmd,
            prompt: wire.prompt,
            shell_check_steps: wire.shell_check_steps,
            timeout: wire.timeout,
            auth_mode: wire.auth_mode,
            server: wire.server,
            port: wire.port,
        })
    }
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
        self.prompt.replace_strings(scope)?;
        for step in &mut self.shell_check_steps {
            step.replace_strings(scope)?;
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
        self.prompt.normalize(config_name)?;
        normalize_shell_check_steps(&mut self.shell_check_steps, config_name).map(drop)
    }

    pub(crate) fn effective_shell_check_steps(&self) -> Vec<ShellCheckStep> {
        self.shell_check_steps.clone()
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
    use super::BoardRunConfig;
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
fail_regex = ["panic"]
uboot_cmd = [" run bootcmd "]
shell_check_steps = [
  { shell_prefix = " login: ", shell_cmd = " root ", success_regex = ["ok"] },
]
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
        assert_eq!(
            config.shell_check_steps[0].shell_prefix.as_deref(),
            Some("login:")
        );
        assert_eq!(
            config.shell_check_steps[0].shell_cmd.as_deref(),
            Some(" root ")
        );
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
    fn board_run_config_accepts_inherited_prefix_in_ordered_steps() {
        let mut config: BoardRunConfig = toml::from_str(
            r#"
board_type = "orangepi-5-plus"
shell_check_steps = [
  { shell_prefix = "axvisor:/$", shell_cmd = "help" },
  { shell_cmd = "vm list" },
]
"#,
        )
        .unwrap();

        config.normalize("test board config").unwrap();

        assert_eq!(
            config.shell_check_steps[0].shell_prefix.as_deref(),
            Some("axvisor:/$")
        );
        assert_eq!(config.shell_check_steps[1].shell_prefix, None);
    }

    #[test]
    fn board_run_config_rejects_legacy_shell_check_fields() {
        toml::from_str::<BoardRunConfig>(
            r#"
            board_type = "visionfive2"
            shell_prefix = "root@starry:"
            shell_init_cmd = "echo pass"
            success_regex = ["(?m)^pass\\s*$"]
            "#,
        )
        .unwrap_err();
    }

    #[test]
    fn board_run_config_rejects_legacy_fields_mixed_with_shell_check_steps() {
        let error = toml::from_str::<BoardRunConfig>(
            r#"
            board_type = "visionfive2"
            shell_prefix = "root@starry:"
            shell_check_steps = [
            { shell_prefix = "root@starry:", shell_cmd = "echo pass" },
            ]
            "#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("shell_prefix"));
    }

    #[test]
    fn legacy_board_run_config_defaults_to_no_session_files() {
        let fixture = LegacyBoardRunConfigFixture {
            board_type: "orangepi-5-plus".to_string(),
        };
        let encoded = toml::to_string(&fixture).unwrap();

        let decoded: BoardRunConfig = toml::from_str(&encoded).unwrap();

        assert!(decoded.session_files.is_empty());
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
    fn board_run_config_schema_rejects_unknown_top_level_fields() {
        let schema = schemars::schema_for!(BoardRunConfig);
        let schema = serde_json::to_value(schema).unwrap();

        assert_eq!(schema["additionalProperties"], false);
        assert!(schema["properties"].get("shell_check_steps").is_some());
        assert!(schema["properties"].get("success_regex").is_none());
        assert!(schema["properties"].get("shell_prefix").is_none());
        assert!(schema["properties"].get("shell_init_cmd").is_none());
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
shell_check_steps = [
  { shell_prefix = " login: ", shell_cmd = " root " },
]
timeout = 8
"#,
        )
        .unwrap();

        let invocation = make_invocation(tmp.path());

        let config = read_run_config_from_path(&invocation, &config_path)
            .await
            .unwrap();
        assert_eq!(config.board_type, "rk3568");
        assert_eq!(
            config.shell_check_steps[0].shell_prefix.as_deref(),
            Some("login:")
        );
        assert_eq!(
            config.shell_check_steps[0].shell_cmd.as_deref(),
            Some(" root ")
        );
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

    #[tokio::test]
    async fn read_board_run_config_expands_every_shell_check_step_string() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/lib.rs"), "").unwrap();
        let config_path = tmp.path().join("variables.board.toml");
        std::fs::write(
            &config_path,
            r#"
board_type = "sample"
shell_check_steps = [
  {
    shell_prefix = "${package}:/$",
    shell_cmd = "run ${package}",
    success_regex = ["${package} passed"],
    fail_regex = ["${package} failed"],
  },
]
"#,
        )
        .unwrap();

        let config = read_run_config_from_path(&make_invocation(tmp.path()), &config_path)
            .await
            .unwrap();
        let step = &config.shell_check_steps[0];
        let package_path = tmp.path().display().to_string();

        assert_eq!(step.shell_prefix, Some(format!("{package_path}:/$")));
        assert_eq!(
            step.shell_cmd.as_deref(),
            Some(format!("run {package_path}").as_str())
        );
        assert_eq!(
            step.success_regex,
            Some(vec![format!("{package_path} passed")])
        );
        assert_eq!(
            step.fail_regex,
            Some(vec![format!("{package_path} failed")])
        );
    }
}
