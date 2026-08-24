use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow};
use async_trait::async_trait;
use colored::Colorize;
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use log::info;
use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use tokio::{
    fs,
    io::{AsyncReadExt, AsyncWriteExt},
    sync::mpsc,
};
use tokio_serial::SerialPortBuilderExt;
use tokio_util::compat::{
    FuturesAsyncReadCompatExt, FuturesAsyncWriteCompatExt, TokioAsyncReadCompatExt,
    TokioAsyncWriteCompatExt,
};
use uboot_shell::UbootShell;

use crate::{
    artifact::state::OutputArtifacts,
    board::{
        client::{
            BoardServerClient, BootConfig as RemoteBootConfig, BootProfileResponse,
            SerialStatusResponse, SessionCreatedResponse, SessionDtbResponse, TftpSessionResponse,
            UbootNetworkMode as RemoteUbootNetworkMode,
        },
        config::BoardRunConfig,
        serial_stream::{
            BoxedAsyncRead, BoxedAsyncWrite, SerialStreamTasks, connect_serial_stream,
        },
    },
    boot::{
        artifacts::{BootArtifact, BootArtifactKind, StagedBootArtifact},
        fit::{self, FitInput},
    },
    invocation::Invocation,
    process::ProcessContext,
    project::variables::{self, VariableScope},
    run::{
        execution::{RunnerExecutionSummary, RunnerExitStatus, timeout_duration},
        output_matcher::{
            FailStreamMatcher, MATCH_DRAIN_DURATION, compile_fail_regexes, print_fail_match,
        },
        shell_check::{
            ShellCheckDriver, ShellCheckMatcher, ShellCheckStep, normalize_shell_check_steps,
        },
        tftp,
    },
    sterm::{AsyncTerminal, TerminalConfig},
    utils::PathResultExt,
};

/// Keep a dead serial console from holding a board runner forever when no
/// positive U-Boot timeout is configured.
const DEFAULT_UBOOT_SHELL_TIMEOUT: Duration = Duration::from_secs(300);

async fn new_uboot_shell<Tx, Rx>(tx: Tx, rx: Rx, timeout: Duration) -> anyhow::Result<UbootShell>
where
    Tx: futures::io::AsyncWrite + Send + Unpin + 'static,
    Rx: futures::io::AsyncRead + Send + Unpin + 'static,
{
    tokio::time::timeout(timeout, UbootShell::new(tx, rx))
        .await
        .with_context(|| format!("timed out waiting for U-Boot shell after {timeout:?}"))?
        .context("failed to initialize U-Boot shell")
}

#[derive(Debug, Clone, Serialize, JsonSchema, Default)]
pub struct UbootConfig {
    pub dtb_file: Option<String>,
    /// Kernel load address
    /// if not specified, use U-Boot env variable 'loadaddr'
    pub kernel_load_addr: Option<String>,
    /// Fit Image load address
    /// if not specified, use automatically calculated address
    pub fit_load_addr: Option<String>,
    /// Address passed to `bootm` after serial FIT upload.
    /// if not specified, use the FIT load address when configured.
    pub bootm_addr: Option<String>,
    /// Board reset command
    /// shell command to reset the board
    pub board_reset_cmd: Option<String>,
    /// Board power off command
    /// shell command to power off the board
    pub board_power_off_cmd: Option<String>,
    pub fail_regex: Vec<String>,
    pub uboot_cmd: Option<Vec<String>>,
    /// Ordered shell commands and result checks.
    #[serde(default)]
    pub shell_check_steps: Vec<ShellCheckStep>,
    /// Timeout in seconds after entering the serial terminal interaction stage. `None` or `0`
    /// disables the terminal timeout; U-Boot shell initialization still uses a bounded default.
    pub timeout: Option<u64>,
    #[serde(flatten)]
    pub local: LocalUbootConfig,
}

#[derive(Deserialize)]
struct UbootConfigWire {
    dtb_file: Option<String>,
    kernel_load_addr: Option<String>,
    fit_load_addr: Option<String>,
    bootm_addr: Option<String>,
    board_reset_cmd: Option<String>,
    board_power_off_cmd: Option<String>,
    success_regex: Option<serde::de::IgnoredAny>,
    fail_regex: Vec<String>,
    uboot_cmd: Option<Vec<String>>,
    #[serde(default)]
    shell_check_steps: Vec<ShellCheckStep>,
    timeout: Option<u64>,
    shell_prefix: Option<serde::de::IgnoredAny>,
    shell_init_cmd: Option<serde::de::IgnoredAny>,
    shell_init_steps: Option<serde::de::IgnoredAny>,
    #[serde(flatten)]
    local: LocalUbootConfig,
}

impl<'de> Deserialize<'de> for UbootConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = UbootConfigWire::deserialize(deserializer)?;
        reject_removed_uboot_key(&wire)?;
        Ok(Self {
            dtb_file: wire.dtb_file,
            kernel_load_addr: wire.kernel_load_addr,
            fit_load_addr: wire.fit_load_addr,
            bootm_addr: wire.bootm_addr,
            board_reset_cmd: wire.board_reset_cmd,
            board_power_off_cmd: wire.board_power_off_cmd,
            fail_regex: wire.fail_regex,
            uboot_cmd: wire.uboot_cmd,
            shell_check_steps: wire.shell_check_steps,
            timeout: wire.timeout,
            local: wire.local,
        })
    }
}

fn reject_removed_uboot_key<E>(wire: &UbootConfigWire) -> Result<(), E>
where
    E: serde::de::Error,
{
    for (key, present) in [
        ("shell_prefix", wire.shell_prefix.is_some()),
        ("shell_init_cmd", wire.shell_init_cmd.is_some()),
        ("shell_init_steps", wire.shell_init_steps.is_some()),
        ("success_regex", wire.success_regex.is_some()),
    ] {
        if present {
            return Err(E::custom(format!(
                "removed U-Boot config key `{key}`; use `shell_check_steps`"
            )));
        }
    }
    Ok(())
}

#[derive(Default, Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct LocalUbootConfig {
    /// Serial console device
    /// e.g., /dev/ttyUSB0 on linux, COM3 on Windows
    pub serial: Option<String>,
    pub baud_rate: Option<String>,
    /// TFTP boot configuration
    pub net: Option<Net>,
    /// Legacy Rust API compatibility field. Use `UbootConfig::board_reset_cmd`.
    #[serde(skip)]
    #[schemars(skip)]
    pub board_reset_cmd: Option<String>,
    /// Legacy Rust API compatibility field. Use `UbootConfig::board_power_off_cmd`.
    #[serde(skip)]
    #[schemars(skip)]
    pub board_power_off_cmd: Option<String>,
}

impl UbootConfig {
    pub fn from_board_run_config(config: &BoardRunConfig) -> Self {
        Self {
            dtb_file: config.dtb_file.clone(),
            kernel_load_addr: config.kernel_load_addr.clone(),
            fit_load_addr: config.fit_load_addr.clone(),
            bootm_addr: config.bootm_addr.clone(),
            fail_regex: config.fail_regex.clone(),
            uboot_cmd: config.uboot_cmd.clone(),
            shell_check_steps: config.shell_check_steps.clone(),
            timeout: config.timeout,
            ..Default::default()
        }
    }

    fn replace_strings(&mut self, scope: &VariableScope) -> anyhow::Result<()> {
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
        self.board_reset_cmd = self
            .board_reset_cmd
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.board_power_off_cmd = self
            .board_power_off_cmd
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
        for step in &mut self.shell_check_steps {
            step.replace_strings(scope)?;
        }
        self.local.replace_strings(scope)?;
        Ok(())
    }

    pub fn kernel_load_addr_int(&self) -> Option<u64> {
        self.addr_int(self.kernel_load_addr.as_ref())
    }

    pub fn fit_load_addr_int(&self) -> Option<u64> {
        self.addr_int(self.fit_load_addr.as_ref())
    }

    pub fn bootm_addr_int(&self) -> Option<u64> {
        self.addr_int(self.bootm_addr.as_ref())
    }

    fn addr_int(&self, addr_str: Option<&String>) -> Option<u64> {
        parse_addr_int(addr_str)
    }

    fn normalize(&mut self, config_name: &str) -> anyhow::Result<()> {
        normalize_shell_check_steps(&mut self.shell_check_steps, config_name).map(drop)
    }

    fn shell_check_matcher(&self) -> anyhow::Result<Option<ShellCheckMatcher>> {
        if self.shell_check_steps.is_empty() {
            return Ok(None);
        }
        let mut steps = self.shell_check_steps.clone();
        let resolved = normalize_shell_check_steps(&mut steps, "U-Boot runtime config")?;
        Ok(Some(ShellCheckMatcher::from_steps(resolved)?))
    }
}

fn parse_addr_int(addr_str: Option<&String>) -> Option<u64> {
    addr_str.as_ref().and_then(|addr_str| {
        if addr_str.starts_with("0x") || addr_str.starts_with("0X") {
            u64::from_str_radix(&addr_str[2..], 16).ok()
        } else {
            addr_str.parse::<u64>().ok()
        }
    })
}

impl LocalUbootConfig {
    fn replace_strings(&mut self, scope: &VariableScope) -> anyhow::Result<()> {
        self.serial = self
            .serial
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.baud_rate = self
            .baud_rate
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.board_reset_cmd = self
            .board_reset_cmd
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.board_power_off_cmd = self
            .board_power_off_cmd
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        if let Some(net) = &mut self.net {
            net.replace_strings(scope)?;
        }
        Ok(())
    }
}

#[derive(Default, Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct Net {
    pub interface: String,
    pub board_ip: Option<String>,
    pub gatewayip: Option<String>,
    pub netmask: Option<String>,
    /// Use an existing TFTP root directory directly. On Linux this skips all
    /// tftpd-hpa detection, installation, config, and service checks.
    pub tftp_dir: Option<String>,
}

impl Net {
    fn replace_strings(&mut self, scope: &VariableScope) -> anyhow::Result<()> {
        self.interface = variables::expand_variables(&self.interface, scope)?;
        self.board_ip = self
            .board_ip
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.gatewayip = self
            .gatewayip
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.netmask = self
            .netmask
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.tftp_dir = self
            .tftp_dir
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub(crate) struct UbootRunInput {
    process_context: ProcessContext,
    artifacts: OutputArtifacts,
    arch: Option<object::Architecture>,
}

impl UbootRunInput {
    pub(crate) fn new(
        process_context: ProcessContext,
        artifacts: OutputArtifacts,
        arch: Option<object::Architecture>,
    ) -> Self {
        Self {
            process_context,
            artifacts,
            arch,
        }
    }

    pub(crate) fn artifacts(&self) -> &OutputArtifacts {
        &self.artifacts
    }
}

pub(crate) fn default_uboot_config() -> UbootConfig {
    UbootConfig {
        local: LocalUbootConfig {
            serial: Some("/dev/ttyUSB0".to_string()),
            baud_rate: Some("115200".to_string()),
            ..Default::default()
        },
        ..Default::default()
    }
}

/// Returns the default U-Boot runtime configuration.
pub fn default_config() -> UbootConfig {
    default_uboot_config()
}

/// Reads a U-Boot configuration from an explicit path without creating defaults.
pub async fn read_config_from_path(
    invocation: &Invocation,
    path: &Path,
) -> anyhow::Result<UbootConfig> {
    let scope = invocation.variable_scope()?;
    read_uboot_config_from_path(&scope, path).await
}

/// Reads a U-Boot configuration using the Cargo package variable scope.
pub async fn read_config_from_path_for_cargo(
    invocation: &Invocation,
    cargo: &crate::build::config::Cargo,
    path: &Path,
) -> anyhow::Result<UbootConfig> {
    let scope = crate::build::cargo_variable_scope(invocation.project_layout(), cargo)?;
    read_uboot_config_from_path(&scope, path).await
}

pub(crate) async fn read_uboot_config_from_path(
    variables: &VariableScope,
    path: &Path,
) -> anyhow::Result<UbootConfig> {
    let config_path = variables::expand_path_variables(path, variables)?;
    read_uboot_config_at_path(variables, config_path).await
}

pub(crate) async fn ensure_uboot_config_in_dir(
    variables: &VariableScope,
    dir: &Path,
) -> anyhow::Result<UbootConfig> {
    let dir = variables::expand_path_variables(dir, variables)?;
    ensure_uboot_config_at_path(variables, dir.join(".uboot.toml"), default_uboot_config()).await
}

/// Loads or creates a U-Boot configuration from a directory.
pub async fn ensure_config_in_dir(
    invocation: &Invocation,
    dir: &Path,
) -> anyhow::Result<UbootConfig> {
    let scope = invocation.variable_scope()?;
    ensure_uboot_config_in_dir(&scope, dir).await
}

/// Loads or creates a U-Boot configuration using the workspace directory.
pub async fn ensure_config_for_cargo(
    invocation: &Invocation,
    cargo: &crate::build::config::Cargo,
) -> anyhow::Result<UbootConfig> {
    let scope = crate::build::cargo_variable_scope(invocation.project_layout(), cargo)?;
    ensure_uboot_config_in_dir(&scope, invocation.workspace_dir()).await
}

pub(crate) fn prepare_uboot_runtime_config(
    variables: &VariableScope,
    config: &UbootConfig,
) -> anyhow::Result<UbootConfig> {
    let mut config = config.clone();
    config.replace_strings(variables)?;
    config.normalize("U-Boot runtime config")?;
    Ok(config)
}

pub(crate) async fn run_uboot_with_config(
    input: UbootRunInput,
    config: UbootConfig,
) -> anyhow::Result<()> {
    let backend = LocalBackend::new(
        config.local.clone(),
        config.board_reset_cmd.clone(),
        config.board_power_off_cmd.clone(),
    );
    let mut runner = Runner::new(input, config, backend);
    runner.run().await
}

/// Runs an already prepared artifact via U-Boot.
pub async fn run_uboot(invocation: &mut Invocation, config: &UbootConfig) -> anyhow::Result<()> {
    let scope = invocation.variable_scope()?;
    let config = prepare_uboot_runtime_config(&scope, config)?;
    invocation.ensure_runtime_bin()?;
    let input = uboot_run_input(invocation)?;
    run_uboot_with_config(input, config).await
}

pub(crate) fn uboot_run_input(invocation: &Invocation) -> anyhow::Result<UbootRunInput> {
    Ok(UbootRunInput::new(
        invocation.process_context()?,
        invocation.runtime_artifacts().clone(),
        invocation.runtime_arch(),
    ))
}

pub(crate) async fn run_uboot_remote(
    input: UbootRunInput,
    board_config: &BoardRunConfig,
    client: BoardServerClient,
    session: SessionCreatedResponse,
) -> anyhow::Result<()> {
    let config = UbootConfig::from_board_run_config(board_config);
    let backend = RemoteBackend::new(client, session);
    let mut runner = Runner::new(input, config, backend);
    runner.run().await
}

pub(crate) async fn read_uboot_config_at_path(
    variables: &VariableScope,
    config_path: PathBuf,
) -> anyhow::Result<UbootConfig> {
    let mut config: UbootConfig = fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("failed to read U-Boot config: {}", config_path.display()))
        .and_then(|content| {
            toml::from_str(&content).with_context(|| {
                format!("failed to parse U-Boot config: {}", config_path.display())
            })
        })?;
    config.replace_strings(variables)?;
    config.normalize(&format!("U-Boot config {}", config_path.display()))?;
    Ok(config)
}

pub(crate) async fn ensure_uboot_config_at_path(
    variables: &VariableScope,
    config_path: PathBuf,
    default_config: UbootConfig,
) -> anyhow::Result<UbootConfig> {
    let mut config = match fs::read_to_string(&config_path).await {
        Ok(_) => return read_uboot_config_at_path(variables, config_path).await,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let config = default_config;
            fs::write(&config_path, toml::to_string_pretty(&config)?)
                .await
                .with_path("failed to write file", &config_path)?;
            config
        }
        Err(err) => return Err(err.into()),
    };

    config.replace_strings(variables)?;
    config.normalize(&format!("U-Boot config {}", config_path.display()))?;
    Ok(config)
}

struct Runner<B> {
    input: UbootRunInput,
    config: UbootConfig,
    fail_regex: Vec<regex::Regex>,
    backend: B,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkBootRequest {
    bootfile: String,
    bootcmd: String,
}

struct ConsoleTransport {
    tx: BoxedAsyncWrite,
    rx: BoxedAsyncRead,
}

struct SecondaryRunFailure {
    phase: &'static str,
    error: anyhow::Error,
}

#[derive(Debug, Clone, Default)]
struct ResolvedRuntime {
    server_ip: Option<String>,
    netmask: Option<String>,
    interface: Option<String>,
    gateway_ip: Option<String>,
    board_ip: Option<String>,
    static_ip: bool,
    kernel_load_addr: Option<u64>,
    fit_load_addr: Option<u64>,
    bootm_addr: Option<u64>,
    use_tftp: bool,
}

#[derive(Debug, Clone, Default)]
struct PreparedDtb {
    fit_source: Option<PathBuf>,
}

#[async_trait]
trait RunnerBackend {
    async fn resolve_runtime(
        &mut self,
        input: &UbootRunInput,
        config: &UbootConfig,
    ) -> anyhow::Result<ResolvedRuntime>;
    async fn prepare_dtb(
        &mut self,
        input: &UbootRunInput,
        config: &UbootConfig,
    ) -> anyhow::Result<PreparedDtb>;
    async fn open_console(&mut self) -> anyhow::Result<ConsoleTransport>;
    async fn after_console_open(&mut self, context: &ProcessContext) -> anyhow::Result<()>;
    async fn stage_fit_image(
        &mut self,
        fit_artifact: &BootArtifact,
        runtime: &ResolvedRuntime,
    ) -> anyhow::Result<StagedBootArtifact>;
    async fn finish_console(&mut self) -> anyhow::Result<()>;
    async fn after_run(&mut self, context: &ProcessContext) -> anyhow::Result<()>;
}

struct LocalBackend {
    config: LocalUbootConfig,
    /// Host-side reset command, taken from `UbootConfig::board_reset_cmd`.
    reset_cmd: Option<String>,
    /// Host-side power-off command, taken from `UbootConfig::board_power_off_cmd`.
    power_off_cmd: Option<String>,
    baud_rate: Option<u32>,
    linux_system_tftp: Option<tftp::TftpdHpaConfig>,
    linux_tftp_staging: Vec<tftp::LinuxTftpPrepared>,
    existing_tftp_dir: Option<PathBuf>,
    builtin_tftp_started: bool,
}

impl LocalBackend {
    fn new(
        config: LocalUbootConfig,
        reset_cmd: Option<String>,
        power_off_cmd: Option<String>,
    ) -> Self {
        let reset_cmd = reset_cmd.or_else(|| config.board_reset_cmd.clone());
        let power_off_cmd = power_off_cmd.or_else(|| config.board_power_off_cmd.clone());
        Self {
            config,
            reset_cmd,
            power_off_cmd,
            baud_rate: None,
            linux_system_tftp: None,
            linux_tftp_staging: Vec::new(),
            existing_tftp_dir: None,
            builtin_tftp_started: false,
        }
    }
}

#[async_trait]
impl RunnerBackend for LocalBackend {
    async fn resolve_runtime(
        &mut self,
        input: &UbootRunInput,
        _config: &UbootConfig,
    ) -> anyhow::Result<ResolvedRuntime> {
        let baud_rate = self
            .config
            .baud_rate
            .as_deref()
            .ok_or_else(|| anyhow!("local U-Boot backend requires `baud_rate`"))?
            .parse::<u32>()
            .context("`baud_rate` is not a valid integer")?;
        self.baud_rate = Some(baud_rate);

        let server_ip = detect_tftp_ip(self.config.net.as_ref());
        let existing_tftp_dir = self
            .config
            .net
            .as_ref()
            .and_then(|net| net.tftp_dir.as_deref())
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);
        self.existing_tftp_dir = existing_tftp_dir.clone();

        #[cfg(target_os = "linux")]
        {
            self.linux_system_tftp = if let Some(directory) = existing_tftp_dir.clone() {
                info!(
                    "Linux detected: using net.tftp_dir={} and skipping all tftpd-hpa checks",
                    directory.display()
                );
                Some(tftp::TftpdHpaConfig {
                    username: None,
                    directory,
                    address: None,
                    options: None,
                })
            } else if self.config.net.is_some() && server_ip.is_some() {
                Some(tftp::ensure_linux_tftpd_hpa()?)
            } else {
                None
            };
        }

        #[cfg(not(target_os = "linux"))]
        {
            if existing_tftp_dir.is_none()
                && let Some(ip) = server_ip.as_ref()
            {
                info!("TFTP server IP: {ip}");
                tftp::run_tftp_server(input.process_context.workdir(), &input.artifacts)?;
                self.builtin_tftp_started = true;
            }
        }

        #[cfg(target_os = "linux")]
        {
            if self.linux_system_tftp.is_none()
                && existing_tftp_dir.is_none()
                && let Some(ip) = server_ip.as_ref()
            {
                info!("TFTP server IP: {ip}");
                tftp::run_tftp_server(input.process_context.workdir(), &input.artifacts)?;
                self.builtin_tftp_started = true;
            }
        }

        Ok(ResolvedRuntime {
            server_ip,
            netmask: self.config.net.as_ref().and_then(|net| net.netmask.clone()),
            interface: self
                .config
                .net
                .as_ref()
                .map(|net| net.interface.clone())
                .filter(|value| !value.trim().is_empty()),
            gateway_ip: self
                .config
                .net
                .as_ref()
                .and_then(|net| net.gatewayip.clone()),
            board_ip: self
                .config
                .net
                .as_ref()
                .and_then(|net| net.board_ip.clone()),
            static_ip: self
                .config
                .net
                .as_ref()
                .and_then(|net| net.board_ip.as_ref())
                .is_some(),
            use_tftp: self.config.net.is_some(),
            ..Default::default()
        })
    }

    async fn prepare_dtb(
        &mut self,
        _input: &UbootRunInput,
        config: &UbootConfig,
    ) -> anyhow::Result<PreparedDtb> {
        Ok(PreparedDtb {
            fit_source: config.dtb_file.as_ref().map(PathBuf::from),
        })
    }

    async fn open_console(&mut self) -> anyhow::Result<ConsoleTransport> {
        let serial = self
            .config
            .serial
            .as_deref()
            .ok_or_else(|| anyhow!("local U-Boot backend requires `serial`"))?;
        let baud_rate = self
            .baud_rate
            .ok_or_else(|| anyhow!("local U-Boot backend missing parsed baud rate"))?;

        info!("Opening serial port: {serial} @ {baud_rate}");
        let serial = tokio_serial::new(serial, baud_rate)
            .timeout(Duration::from_millis(200))
            .open_native_async()
            .with_context(|| format!("failed to open serial port {serial}"))?;
        let (rx, tx) = tokio::io::split(serial);

        Ok(ConsoleTransport {
            tx: Box::new(tx.compat_write()),
            rx: Box::new(rx.compat()),
        })
    }

    async fn after_console_open(&mut self, context: &ProcessContext) -> anyhow::Result<()> {
        println!("Waiting for board on power or reset...");
        if let Some(cmd) = self.reset_cmd.as_deref()
            && !cmd.trim().is_empty()
        {
            crate::process::shell_run_cmd(context, cmd)?;
        }
        Ok(())
    }

    async fn stage_fit_image(
        &mut self,
        fit_artifact: &BootArtifact,
        _runtime: &ResolvedRuntime,
    ) -> anyhow::Result<StagedBootArtifact> {
        let fitimage = fit_artifact_path(fit_artifact)?;
        let Some(file_name) = fitimage.file_name().and_then(|name| name.to_str()) else {
            return Err(anyhow!("Invalid fitimage filename"));
        };

        #[cfg(target_os = "linux")]
        {
            if let Some(system_tftp) = self.linux_system_tftp.as_ref() {
                let prepared = tftp::stage_linux_fit_image(fitimage, &system_tftp.directory)?;
                let relative_filename = prepared.relative_filename().to_string();
                info!(
                    "Staged FIT image to: {}",
                    prepared.absolute_fit_path().display()
                );
                self.linux_tftp_staging.push(prepared);
                return Ok(StagedBootArtifact::network(relative_filename));
            }
        }

        if let Some(tftp_dir) = self.existing_tftp_dir.as_deref() {
            let tftp_path = PathBuf::from(tftp_dir).join(file_name);
            info!("Setting TFTP file path: {}", tftp_path.display());
            return Ok(StagedBootArtifact::network(tftp_path.display().to_string()));
        }

        if self.builtin_tftp_started {
            info!("Using fitimage filename: {file_name}");
            return Ok(StagedBootArtifact::network(file_name));
        }

        Ok(StagedBootArtifact::no_network())
    }

    async fn finish_console(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn after_run(&mut self, context: &ProcessContext) -> anyhow::Result<()> {
        let mut cleanup_failures = Vec::new();
        for prepared in self.linux_tftp_staging.drain(..) {
            let target = prepared.target_dir().display().to_string();
            if let Err(err) = tftp::cleanup_linux_tftp_staging(&prepared) {
                cleanup_failures.push(format!("{target}: {err:#}"));
            }
        }

        if let Some(cmd) = self.power_off_cmd.as_deref()
            && !cmd.trim().is_empty()
            && let Err(err) = crate::process::shell_run_cmd(context, cmd)
        {
            log::warn!("board power-off command failed: {err:#}");
        }

        if cleanup_failures.is_empty() {
            Ok(())
        } else {
            Err(anyhow!(
                "failed to clean local TFTP staging:\n{}",
                cleanup_failures.join("\n")
            ))
        }
    }
}

struct RemoteBackend {
    client: BoardServerClient,
    session: SessionCreatedResponse,
    boot_profile: Option<BootProfileResponse>,
    serial_status: Option<SerialStatusResponse>,
    tftp_status: Option<TftpSessionResponse>,
    session_dtb: Option<SessionDtbResponse>,
    console_tasks: Option<SerialStreamTasks>,
}

impl RemoteBackend {
    fn new(client: BoardServerClient, session: SessionCreatedResponse) -> Self {
        Self {
            client,
            session,
            boot_profile: None,
            serial_status: None,
            tftp_status: None,
            session_dtb: None,
            console_tasks: None,
        }
    }
}

#[async_trait]
impl RunnerBackend for RemoteBackend {
    async fn resolve_runtime(
        &mut self,
        _input: &UbootRunInput,
        _config: &UbootConfig,
    ) -> anyhow::Result<ResolvedRuntime> {
        let boot_profile = self
            .client
            .get_boot_profile(&self.session.session_id)
            .await
            .with_context(|| {
                let session_id = &self.session.session_id;
                format!("failed to get boot profile for session `{session_id}`")
            })?;
        let serial_status = self
            .client
            .get_serial_status(&self.session.session_id)
            .await
            .with_context(|| {
                let session_id = &self.session.session_id;
                format!("failed to get serial status for session `{session_id}`")
            })?;
        let tftp_status = self
            .client
            .get_tftp_status(&self.session.session_id)
            .await
            .with_context(|| {
                let session_id = &self.session.session_id;
                format!("failed to get tftp status for session `{session_id}`")
            })?;

        let profile = match &boot_profile.boot {
            RemoteBootConfig::Uboot(profile) => profile.clone(),
            other => {
                return Err(anyhow!(
                    "unsupported remote boot mode `{other:?}`; only `uboot` is supported"
                ));
            }
        };

        if !serial_status.available {
            return Err(anyhow!(
                "session `{}` has no serial console available",
                self.session.session_id
            ));
        }
        if serial_status.ws_url.is_none() && self.session.ws_url.is_none() {
            return Err(anyhow!(
                "session `{}` did not return a serial websocket URL",
                self.session.session_id
            ));
        }

        let static_ip = profile.network_mode == RemoteUbootNetworkMode::StaticIp;
        let server_ip = profile.server_ip.clone().or_else(|| {
            tftp_status
                .server_ip
                .clone()
                .or_else(|| boot_profile.server_ip.clone())
        });
        let netmask = profile.netmask.clone().or_else(|| {
            tftp_status
                .netmask
                .clone()
                .or_else(|| boot_profile.netmask.clone())
        });

        self.boot_profile = Some(boot_profile.clone());
        self.serial_status = Some(serial_status);
        self.tftp_status = Some(tftp_status);

        Ok(ResolvedRuntime {
            server_ip,
            netmask,
            interface: boot_profile.interface.clone(),
            gateway_ip: profile.gatewayip.clone(),
            board_ip: profile.board_ip.clone(),
            static_ip,
            kernel_load_addr: parse_addr_int(profile.kernel_load_addr.as_ref()),
            fit_load_addr: parse_addr_int(profile.fit_load_addr.as_ref()),
            bootm_addr: parse_addr_int(profile.bootm_addr.as_ref()),
            use_tftp: profile.use_tftp,
        })
    }

    async fn prepare_dtb(
        &mut self,
        input: &UbootRunInput,
        config: &UbootConfig,
    ) -> anyhow::Result<PreparedDtb> {
        let session_dtb = self
            .client
            .get_session_dtb(&self.session.session_id)
            .await
            .with_context(|| {
                format!(
                    "failed to get session DTB metadata for session `{}`",
                    self.session.session_id
                )
            })?;
        self.session_dtb = Some(session_dtb.clone());

        if let Some(local_dtb) = config.dtb_file.as_ref().map(PathBuf::from) {
            let upload_path = if let Some(session_file_path) = session_dtb.session_file_path.clone()
            {
                session_file_path
            } else {
                let file_name = local_dtb
                    .file_name()
                    .and_then(|name| name.to_str())
                    .ok_or_else(|| anyhow!("invalid DTB filename: {}", local_dtb.display()))?;
                format!("boot/dtb/{file_name}")
            };
            let payload = fs::read(&local_dtb)
                .await
                .with_path("failed to read DTB file", &local_dtb)?;
            self.client
                .upload_session_file(&self.session.session_id, &upload_path, payload)
                .await
                .with_context(|| {
                    format!(
                        "failed to upload DTB override for session `{}`",
                        self.session.session_id
                    )
                })?;
            return Ok(PreparedDtb {
                fit_source: Some(local_dtb),
            });
        }

        let Some(dtb_name) = session_dtb.dtb_name.as_deref() else {
            return Ok(PreparedDtb::default());
        };
        let bytes = self
            .client
            .download_session_dtb(&self.session.session_id)
            .await
            .with_context(|| {
                format!(
                    "failed to download preset DTB for session `{}`",
                    self.session.session_id
                )
            })?;
        let output_dir = input
            .artifacts
            .runtime_artifact_dir()
            .map(PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        fs::create_dir_all(&output_dir)
            .await
            .with_context(|| format!("failed to create {}", output_dir.display()))?;
        let target_path = output_dir.join(format!("ostool-{}-{dtb_name}", self.session.session_id));
        fs::write(&target_path, bytes)
            .await
            .with_path("failed to write preset DTB", &target_path)?;

        Ok(PreparedDtb {
            fit_source: Some(target_path),
        })
    }

    async fn open_console(&mut self) -> anyhow::Result<ConsoleTransport> {
        let serial_status = self
            .serial_status
            .as_ref()
            .ok_or_else(|| anyhow!("remote runtime not initialized"))?;
        let ws_url = serial_status
            .ws_url
            .as_deref()
            .or(self.session.ws_url.as_deref())
            .ok_or_else(|| anyhow!("server did not return a serial websocket URL"))?;
        let ws_url = self.client.resolve_ws_url(ws_url)?;
        let (tx, rx, tasks) =
            connect_serial_stream(ws_url, self.client.websocket_authorization().await?).await?;
        self.console_tasks = Some(tasks);
        Ok(ConsoleTransport { tx, rx })
    }

    async fn after_console_open(&mut self, _context: &ProcessContext) -> anyhow::Result<()> {
        println!("Waiting for remote board to power on through ostool-server...");
        Ok(())
    }

    async fn stage_fit_image(
        &mut self,
        fit_artifact: &BootArtifact,
        runtime: &ResolvedRuntime,
    ) -> anyhow::Result<StagedBootArtifact> {
        let fitimage = fit_artifact_path(fit_artifact)?;
        let tftp_status = self
            .tftp_status
            .as_ref()
            .ok_or_else(|| anyhow!("remote runtime not initialized"))?;
        if !runtime.use_tftp || !tftp_status.available {
            return Ok(StagedBootArtifact::no_network());
        }

        let fit_name = fitimage
            .file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| anyhow!("Invalid fitimage filename"))?;
        let upload_path = format!("boot/{fit_name}");
        let payload = fs::read(fitimage)
            .await
            .with_path("failed to read file", fitimage)?;
        let uploaded = self
            .client
            .upload_session_file(&self.session.session_id, &upload_path, payload)
            .await
            .with_context(|| {
                format!(
                    "failed to upload FIT image for session `{}`",
                    self.session.session_id
                )
            })?;

        Ok(StagedBootArtifact::network(uploaded.relative_path))
    }

    async fn finish_console(&mut self) -> anyhow::Result<()> {
        if let Some(tasks) = self.console_tasks.take() {
            tasks.shutdown_with_timeout(Duration::from_secs(2)).await?;
        }
        Ok(())
    }

    async fn after_run(&mut self, _context: &ProcessContext) -> anyhow::Result<()> {
        Ok(())
    }
}

async fn finalize_backend_run<B: RunnerBackend>(
    backend: &mut B,
    context: &ProcessContext,
    run_result: anyhow::Result<()>,
) -> anyhow::Result<()> {
    let console_cleanup = backend.finish_console().await;
    let post_run_cleanup = backend.after_run(context).await;
    let (result, secondary_failures) =
        select_runner_result(run_result, console_cleanup, post_run_cleanup);

    for failure in secondary_failures {
        log::warn!("backend {} failed: {:#}", failure.phase, failure.error);
    }
    result
}

fn select_runner_result(
    run_result: anyhow::Result<()>,
    console_cleanup: anyhow::Result<()>,
    post_run_cleanup: anyhow::Result<()>,
) -> (anyhow::Result<()>, Vec<SecondaryRunFailure>) {
    match run_result {
        Err(primary) => {
            let mut secondary = Vec::new();
            if let Err(error) = console_cleanup {
                secondary.push(SecondaryRunFailure {
                    phase: "console cleanup",
                    error,
                });
            }
            if let Err(error) = post_run_cleanup {
                secondary.push(SecondaryRunFailure {
                    phase: "post-run cleanup",
                    error,
                });
            }
            (Err(primary), secondary)
        }
        Ok(()) => match console_cleanup {
            Err(primary) => {
                let secondary = post_run_cleanup
                    .err()
                    .map(|error| SecondaryRunFailure {
                        phase: "post-run cleanup",
                        error,
                    })
                    .into_iter()
                    .collect();
                (Err(primary), secondary)
            }
            Ok(()) => (post_run_cleanup, Vec::new()),
        },
    }
}

impl<B> Runner<B>
where
    B: RunnerBackend,
{
    fn new(input: UbootRunInput, config: UbootConfig, backend: B) -> Self {
        Self {
            input,
            config,
            fail_regex: vec![],
            backend,
        }
    }

    async fn run(&mut self) -> anyhow::Result<()> {
        let run_result = self._run().await;
        finalize_backend_run(&mut self.backend, &self.input.process_context, run_result).await
    }

    async fn _run(&mut self) -> anyhow::Result<()> {
        self.prepare_regex()?;

        let kernel = self
            .input
            .artifacts
            .require_bin("U-Boot runner requires a prepared BIN artifact")?
            .to_path_buf();

        info!("Starting U-Boot runner...");

        info!("kernel from: {}", kernel.display());

        let runtime = self
            .backend
            .resolve_runtime(&self.input, &self.config)
            .await?;
        let prepared_dtb = self.backend.prepare_dtb(&self.input, &self.config).await?;
        if let Some(interface) = runtime.interface.as_deref() {
            info!("Using network interface hint: {interface}");
        }
        let ConsoleTransport { tx, rx } = self.backend.open_console().await?;
        self.backend
            .after_console_open(&self.input.process_context)
            .await?;

        let mut net_ok = false;
        let shell_timeout =
            timeout_duration(self.config.timeout).unwrap_or(DEFAULT_UBOOT_SHELL_TIMEOUT);
        info!("Waiting for U-Boot shell response (timeout: {shell_timeout:?})...");
        let mut uboot = new_uboot_shell(tx, rx, shell_timeout).await?;
        uboot.set_env("autoload", "yes").await?;

        if let Some(ref cmds) = self.config.uboot_cmd {
            for cmd in cmds.iter() {
                info!("Running U-Boot command: {cmd}");
                uboot.cmd(cmd).await?;
            }
        }

        if let Some(ref gatewayip) = runtime.gateway_ip {
            uboot.set_env("gatewayip", gatewayip).await?;
        }

        if let Some(ref netmask) = runtime.netmask {
            uboot.set_env("netmask", netmask).await?;
        }

        if runtime.static_ip
            && let Some(ref board_ip) = runtime.board_ip
        {
            uboot.set_env("ipaddr", board_ip).await?;
        }

        if let Some(ref ip) = runtime.server_ip
            && let Ok(output) = uboot.cmd("net list").await
        {
            let device_list = output.strip_prefix("net list").unwrap_or(&output).trim();

            if device_list.is_empty() {
                let _ = uboot.cmd("bootdev hunt ethernet").await;
            }

            info!("Board network ok");

            uboot.set_env("serverip", ip.clone()).await?;
            net_ok = true;
        }

        let mut fdt_load_addr = None;
        if let Ok(addr) = uboot.env_int("fdt_addr_r").await {
            fdt_load_addr = Some(addr as u64);
        }

        let _ramfs_load_addr = uboot.env_int("ramdisk_addr_r").await.ok();

        let kernel_entry = if let Some(entry) = self
            .config
            .kernel_load_addr_int()
            .or(runtime.kernel_load_addr)
        {
            info!("Using configured kernel load address: {entry:#x}");
            entry
        } else if let Ok(entry) = uboot.env_int("kernel_addr_r").await {
            info!("Using $kernel_addr_r as kernel entry: {entry:#x}");
            entry as u64
        } else if let Ok(entry) = uboot.env_int("loadaddr").await {
            info!("Using $loadaddr as kernel entry: {entry:#x}");
            entry as u64
        } else {
            return Err(anyhow!("Cannot determine kernel entry address"));
        };

        let mut fit_loadaddr = if let Ok(addr) = uboot.env_int("kernel_comp_addr_r").await {
            info!("image load to kernel_comp_addr_r: {addr:#x}");
            addr as u64
        } else if let Ok(addr) = uboot.env_int("kernel_addr_c").await {
            info!("image load to kernel_addr_c: {addr:#x}");
            addr as u64
        } else {
            let addr = (kernel_entry + 0x02000000) & 0xffff_ffff_ff00_0000;
            info!("No kernel_comp_addr_r or kernel_addr_c, use calculated address: {addr:#x}");
            addr
        };

        if let Some(fit_load_addr_int) = self.config.fit_load_addr_int().or(runtime.fit_load_addr) {
            fit_loadaddr = fit_load_addr_int;
        }

        uboot
            .set_env("loadaddr", format!("{fit_loadaddr:#x}"))
            .await?;

        info!("fitimage loadaddr: {fit_loadaddr:#x}");
        info!("kernel entry: {kernel_entry:#x}");
        if let Some(ref dtb_path) = prepared_dtb.fit_source {
            info!("Using DTB from: {}", dtb_path.display());
        }
        let arch = self
            .input
            .arch
            .ok_or_else(|| anyhow!("Cannot determine architecture for FIT image generation"))?;
        let generated_fit = fit::generate_fit_image(FitInput {
            kernel_path: kernel.clone(),
            dtb_path: prepared_dtb.fit_source.clone(),
            arch,
            kernel_load_addr: kernel_entry,
            kernel_entry_addr: kernel_entry,
            fdt_load_addr,
            output_path: None,
        })
        .await?;
        let fit_artifact = BootArtifact::fit_image(generated_fit.path());

        let prepared = self
            .backend
            .stage_fit_image(&fit_artifact, &runtime)
            .await?;

        let bootm_arg = self.resolved_bootm_arg(fit_loadaddr, &runtime);
        let bootcmd = if let Some(fitname) = prepared.bootfile() {
            if let Some(request) = build_network_boot_request(
                runtime.static_ip,
                net_ok,
                prepared.network_transfer_ready(),
                fitname,
                bootm_arg,
            ) {
                uboot.set_env("bootfile", &request.bootfile).await?;
                request.bootcmd
            } else {
                info!("No network boot request available, using loady to upload FIT image...");
                Self::uboot_loady(&mut uboot, fit_loadaddr as usize, fit_artifact.path()).await?;
                self.serial_bootm_command(bootm_arg)
            }
        } else {
            info!("No TFTP config, using loady to upload FIT image...");
            Self::uboot_loady(&mut uboot, fit_loadaddr as usize, fit_artifact.path()).await?;
            self.serial_bootm_command(bootm_arg)
        };

        info!("Booting kernel with command: {bootcmd}");
        uboot.cmd_without_reply(&bootcmd).await?;

        println!("{}", "Interacting with U-Boot shell...".green());

        let matcher = Arc::new(Mutex::new(FailStreamMatcher::new(self.fail_regex.clone())));

        let res = Arc::new(Mutex::new(None));
        let res_clone = res.clone();
        let matcher_clone = matcher.clone();
        let shell_check_driver = self
            .config
            .shell_check_matcher()?
            .map(ShellCheckDriver::new);
        let shell_check_driver_clone = shell_check_driver.clone();
        let mut serial_rx = uboot.rx.take().unwrap().compat();
        let mut serial_tx = uboot.tx.take().unwrap().compat_write();
        drop(uboot);
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (outbound_tx, mut outbound_rx) =
            mpsc::unbounded_channel::<crate::sterm::TerminalInput>();

        let read_task = tokio::spawn(async move {
            let mut buffer = [0u8; 1024];
            loop {
                let read = serial_rx
                    .read(&mut buffer)
                    .await
                    .context("failed to read serial output")?;
                if read == 0 {
                    break;
                }
                if inbound_tx.send(buffer[..read].to_vec()).is_err() {
                    break;
                }
            }
            Ok::<(), anyhow::Error>(())
        });

        let write_task = tokio::spawn(async move {
            while let Some(input) = outbound_rx.recv().await {
                write_uboot_input(&mut serial_tx, input).await?;
            }
            Ok::<(), anyhow::Error>(())
        });

        let terminal = AsyncTerminal::new(TerminalConfig {
            intercept_exit_sequence: true,
            timeout: timeout_duration(self.config.timeout),
            timeout_label: "kernel boot".to_string(),
        });
        let started_at = Instant::now();
        let terminal_result = terminal
            .run_with_write_ack(inbound_rx, outbound_tx, move |h, chunk| {
                let mut matcher = matcher_clone.lock().unwrap();
                for byte in chunk {
                    if let Some(matched) = matcher.observe_byte(*byte) {
                        print_fail_match(&matched);
                        let mut res_lock = res_clone.lock().unwrap();
                        *res_lock = Some(matched);
                        h.stop_after(MATCH_DRAIN_DURATION);
                    }
                }

                if let Some(shell_check_driver) = shell_check_driver_clone.as_ref() {
                    shell_check_driver.observe_chunk(h, chunk);
                }

                if matcher.should_stop() {
                    h.stop();
                }
            })
            .await;
        let mut write_task = write_task;
        let write_join = tokio::time::timeout(Duration::from_secs(1), &mut write_task).await;
        match write_join {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(err))) => return Err(err),
            Ok(Err(err)) if !err.is_cancelled() => {
                return Err(anyhow!("serial writer task join error: {err}"));
            }
            Ok(Err(_)) => {}
            Err(_) => {
                write_task.abort();
                let _ = write_task.await;
            }
        }

        let mut read_task = read_task;
        let read_join = tokio::time::timeout(Duration::from_millis(300), &mut read_task).await;
        match read_join {
            Ok(Ok(Ok(()))) => {}
            Ok(Ok(Err(err))) => return Err(err),
            Ok(Err(err)) if !err.is_cancelled() => {
                return Err(anyhow!("serial reader task join error: {err}"));
            }
            Ok(Err(_)) => {}
            Err(_) => {
                read_task.abort();
                let _ = read_task.await;
            }
        }

        {
            let mut res_lock = res.lock().unwrap();
            let shell_check_completed = shell_check_driver
                .as_ref()
                .is_some_and(ShellCheckDriver::completed);
            let shell_check_failure = shell_check_driver
                .as_ref()
                .and_then(ShellCheckDriver::completion_error);
            RunnerExecutionSummary::new(
                "kernel boot",
                RunnerExitStatus::not_available(),
                started_at.elapsed(),
            )
            .with_terminal_error(terminal_result.err())
            .with_shell_check_error(shell_check_failure)
            .with_shell_check_completed(shell_check_completed)
            .with_fail_match(res_lock.take())
            .into_result()?;
        }
        Ok(())
    }

    fn prepare_regex(&mut self) -> anyhow::Result<()> {
        self.fail_regex = compile_fail_regexes(&self.config.fail_regex)?;
        Ok(())
    }

    fn resolved_bootm_arg(&self, fit_loadaddr: u64, runtime: &ResolvedRuntime) -> Option<u64> {
        self.config
            .bootm_addr_int()
            .or(runtime.bootm_addr)
            .or_else(|| {
                self.config
                    .fit_load_addr_int()
                    .or(runtime.fit_load_addr)
                    .map(|_| fit_loadaddr)
            })
    }

    fn serial_bootm_command(&self, bootm_arg: Option<u64>) -> String {
        bootm_command(bootm_arg)
    }

    async fn uboot_loady(
        uboot: &mut UbootShell,
        addr: usize,
        file: impl Into<PathBuf>,
    ) -> anyhow::Result<()> {
        println!("{}", "\r\nsend file".green());

        let pb = ProgressBar::new(100);
        pb.set_style(
            ProgressStyle::with_template(
                "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] \
                 {bytes}/{total_bytes} ({eta})",
            )
            .unwrap()
            .with_key(
                "eta",
                |state: &ProgressState, w: &mut dyn core::fmt::Write| {
                    write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap()
                },
            )
            .progress_chars("#>-"),
        );

        let res = uboot
            .loady(addr, file, |x, a| {
                pb.set_length(a as _);
                pb.set_position(x as _);
            })
            .await?;

        pb.finish_with_message("upload done");

        println!("{res}");
        println!("send ok");
        Ok(())
    }
}

async fn write_uboot_input<W>(
    writer: &mut W,
    input: crate::sterm::TerminalInput,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    if let Err(error) = writer.write_all(input.bytes()).await {
        input.acknowledge_failed(format!("failed to write U-Boot serial input: {error}"));
        return Err(error).context("failed to write U-Boot serial input");
    }
    if let Err(error) = writer.flush().await {
        input.acknowledge_failed(format!("failed to flush U-Boot serial input: {error}"));
        return Err(error).context("failed to flush U-Boot serial input");
    }
    input.acknowledge_flushed();
    Ok(())
}

fn detect_tftp_ip(net: Option<&Net>) -> Option<String> {
    let net = net?;

    let mut ip_string = String::new();

    let interfaces = NetworkInterface::show().ok()?;
    for interface in interfaces.iter() {
        debug!("net Interface: {}", interface.name);
        if interface.name == net.interface {
            let addr_list: Vec<Addr> = interface.addr.to_vec();
            for one in addr_list {
                if let Addr::V4(v4_if_addr) = one {
                    ip_string = v4_if_addr.ip.to_string();
                }
            }
        }
    }

    if ip_string.trim().is_empty() {
        return None;
    }

    info!("TFTP : {ip_string}");
    Some(ip_string)
}

fn fit_artifact_path(fit_artifact: &BootArtifact) -> anyhow::Result<&Path> {
    if fit_artifact.kind() != BootArtifactKind::FitImage {
        return Err(anyhow!(
            "expected FIT image boot artifact, got {:?}",
            fit_artifact.kind()
        ));
    }

    Ok(fit_artifact.path())
}

fn build_network_boot_request(
    static_ip: bool,
    net_ok: bool,
    network_transfer_ready: bool,
    fitname: &str,
    bootm_arg: Option<u64>,
) -> Option<NetworkBootRequest> {
    if !network_transfer_ready {
        return None;
    }

    if static_ip {
        return Some(NetworkBootRequest {
            bootfile: fitname.to_string(),
            bootcmd: format!("tftp {fitname} && {}", bootm_command(bootm_arg)),
        });
    }

    if net_ok {
        return Some(NetworkBootRequest {
            bootfile: fitname.to_string(),
            bootcmd: format!("dhcp {fitname} && {}", bootm_command(bootm_arg)),
        });
    }

    None
}

fn bootm_command(bootm_arg: Option<u64>) -> String {
    if let Some(addr) = bootm_arg {
        format!("bootm {addr:#x}")
    } else {
        "bootm".to_string()
    }
}

#[cfg(test)]
mod tests {
    use std::{
        collections::HashMap,
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll},
        time::Duration,
    };

    use async_trait::async_trait;
    use tokio::io::AsyncWrite;
    use tokio_util::compat::{TokioAsyncReadCompatExt, TokioAsyncWriteCompatExt};

    use super::{
        LocalBackend, LocalUbootConfig, Net, RemoteBackend, ResolvedRuntime, RunnerBackend,
        UbootConfig, build_network_boot_request, ensure_config_in_dir, fit_artifact_path,
        timeout_duration, write_uboot_input,
    };
    use crate::{
        artifact::runtime::{RuntimeArtifactOptions, prepare_runtime_artifacts},
        board::{
            client::{BoardServerClient, SessionCreatedResponse, TftpSessionResponse},
            config::BoardRunConfig,
        },
        boot::artifacts::BootArtifact,
        build::config::{BuildConfig, BuildSystem, Cargo},
        invocation::{Invocation, InvocationOptions},
        run::{ShellCheckStep, tftp},
    };

    fn make_invocation(dir: &std::path::Path) -> Invocation {
        Invocation::new(InvocationOptions::new(
            Some(dir.to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap()
    }

    fn write_single_crate_manifest(dir: &std::path::Path) {
        std::fs::write(
            dir.join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[workspace]\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/lib.rs"), "").unwrap();
    }

    fn local_backend() -> LocalBackend {
        LocalBackend {
            config: LocalUbootConfig::default(),
            reset_cmd: None,
            power_off_cmd: None,
            baud_rate: None,
            linux_system_tftp: None,
            linux_tftp_staging: Vec::new(),
            existing_tftp_dir: None,
            builtin_tftp_started: false,
        }
    }

    #[derive(Clone, Copy)]
    enum WriterFailure {
        None,
        Write,
        Flush,
    }

    struct FlushCheckingWriter {
        callback_ran: Arc<AtomicBool>,
        failure: WriterFailure,
        flushes: usize,
    }

    impl AsyncWrite for FlushCheckingWriter {
        fn poll_write(
            self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
            bytes: &[u8],
        ) -> Poll<std::io::Result<usize>> {
            if matches!(self.failure, WriterFailure::Write) {
                Poll::Ready(Err(std::io::Error::other("injected write failure")))
            } else {
                Poll::Ready(Ok(bytes.len()))
            }
        }

        fn poll_flush(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<std::io::Result<()>> {
            assert!(!self.callback_ran.load(Ordering::Acquire));
            if matches!(self.failure, WriterFailure::Flush) {
                return Poll::Ready(Err(std::io::Error::other("injected flush failure")));
            }
            self.flushes += 1;
            Poll::Ready(Ok(()))
        }

        fn poll_shutdown(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
            Poll::Ready(Ok(()))
        }
    }

    #[tokio::test]
    async fn uboot_writer_acknowledges_only_after_flush() {
        let callback_ran = Arc::new(AtomicBool::new(false));
        let callback_ran_clone = callback_ran.clone();
        let input = crate::sterm::TerminalInput::for_test(b"command\n".to_vec(), move |result| {
            result.unwrap();
            callback_ran_clone.store(true, Ordering::Release);
        });
        let mut writer = FlushCheckingWriter {
            callback_ran: callback_ran.clone(),
            failure: WriterFailure::None,
            flushes: 0,
        };

        write_uboot_input(&mut writer, input).await.unwrap();

        assert_eq!(writer.flushes, 1);
        assert!(callback_ran.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn uboot_writer_reports_write_failure() {
        let error_seen = Arc::new(std::sync::Mutex::new(None));
        let error_seen_clone = error_seen.clone();
        let input = crate::sterm::TerminalInput::for_test(b"command\n".to_vec(), move |result| {
            *error_seen_clone.lock().unwrap() = result.err().map(|error| error.to_string());
        });
        let mut writer = FlushCheckingWriter {
            callback_ran: Arc::new(AtomicBool::new(false)),
            failure: WriterFailure::Write,
            flushes: 0,
        };

        let error = write_uboot_input(&mut writer, input).await.unwrap_err();

        assert!(
            error
                .to_string()
                .contains("failed to write U-Boot serial input")
        );
        assert_eq!(writer.flushes, 0);
        assert_eq!(
            error_seen.lock().unwrap().as_deref(),
            Some("failed to write U-Boot serial input: injected write failure")
        );
    }

    #[tokio::test]
    async fn uboot_writer_reports_flush_failure() {
        let error_seen = Arc::new(std::sync::Mutex::new(None));
        let error_seen_clone = error_seen.clone();
        let input = crate::sterm::TerminalInput::for_test(b"command\n".to_vec(), move |result| {
            *error_seen_clone.lock().unwrap() = result.err().map(|error| error.to_string());
        });
        let mut writer = FlushCheckingWriter {
            callback_ran: Arc::new(AtomicBool::new(false)),
            failure: WriterFailure::Flush,
            flushes: 0,
        };

        let error = write_uboot_input(&mut writer, input).await.unwrap_err();

        assert!(
            error
                .to_string()
                .contains("failed to flush U-Boot serial input")
        );
        assert_eq!(writer.flushes, 0);
        assert_eq!(
            error_seen.lock().unwrap().as_deref(),
            Some("failed to flush U-Boot serial input: injected flush failure")
        );
    }

    #[tokio::test]
    async fn uboot_writer_reports_first_chunk_failure_once_for_chunked_operation() {
        let (handle, mut rx) = crate::sterm::TerminalHandle::acknowledged_for_test();
        let completions = Arc::new(std::sync::Mutex::new(Vec::new()));
        let completions_clone = completions.clone();
        handle.send_after_chunks_then(
            Duration::ZERO,
            vec![b'x'; 192],
            64,
            Duration::ZERO,
            move |_, result| {
                completions_clone
                    .lock()
                    .unwrap()
                    .push(result.err().map(|error| error.to_string()));
            },
        );

        let first = rx.recv().await.unwrap();
        assert_eq!(first.bytes().len(), 64);
        let mut failing_writer = FlushCheckingWriter {
            callback_ran: Arc::new(AtomicBool::new(false)),
            failure: WriterFailure::Write,
            flushes: 0,
        };
        let error = write_uboot_input(&mut failing_writer, first)
            .await
            .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("failed to write U-Boot serial input")
        );

        for _ in 0..2 {
            let input = rx.recv().await.unwrap();
            assert_eq!(input.bytes().len(), 64);
            let mut writer = FlushCheckingWriter {
                callback_ran: Arc::new(AtomicBool::new(false)),
                failure: WriterFailure::None,
                flushes: 0,
            };
            write_uboot_input(&mut writer, input).await.unwrap();
        }
        tokio::task::yield_now().await;

        assert_eq!(
            completions.lock().unwrap().as_slice(),
            &[Some(
                "failed to write U-Boot serial input: injected write failure".to_string()
            )]
        );
    }

    fn write_fit_image(root: &std::path::Path) -> std::path::PathBuf {
        let fit_path = root.join("target").join("image.fit");
        std::fs::create_dir_all(fit_path.parent().unwrap()).unwrap();
        std::fs::write(&fit_path, [1_u8, 2, 3, 4]).unwrap();
        fit_path
    }

    struct CleanupTrackingBackend {
        finish_calls: usize,
        after_calls: usize,
        finish_error: Option<&'static str>,
        after_error: Option<&'static str>,
    }

    #[async_trait]
    impl RunnerBackend for CleanupTrackingBackend {
        async fn resolve_runtime(
            &mut self,
            _input: &super::UbootRunInput,
            _config: &UbootConfig,
        ) -> anyhow::Result<ResolvedRuntime> {
            unreachable!()
        }

        async fn prepare_dtb(
            &mut self,
            _input: &super::UbootRunInput,
            _config: &UbootConfig,
        ) -> anyhow::Result<super::PreparedDtb> {
            unreachable!()
        }

        async fn open_console(&mut self) -> anyhow::Result<super::ConsoleTransport> {
            unreachable!()
        }

        async fn after_console_open(
            &mut self,
            _context: &crate::process::ProcessContext,
        ) -> anyhow::Result<()> {
            unreachable!()
        }

        async fn stage_fit_image(
            &mut self,
            _fit_artifact: &BootArtifact,
            _runtime: &ResolvedRuntime,
        ) -> anyhow::Result<crate::boot::artifacts::StagedBootArtifact> {
            unreachable!()
        }

        async fn finish_console(&mut self) -> anyhow::Result<()> {
            self.finish_calls += 1;
            match self.finish_error {
                Some(message) => Err(anyhow!(message)),
                None => Ok(()),
            }
        }

        async fn after_run(
            &mut self,
            _context: &crate::process::ProcessContext,
        ) -> anyhow::Result<()> {
            self.after_calls += 1;
            match self.after_error {
                Some(message) => Err(anyhow!(message)),
                None => Ok(()),
            }
        }
    }

    #[tokio::test]
    async fn runner_cleanup_invokes_after_run_when_console_cleanup_fails() {
        let temp = tempfile::tempdir().unwrap();
        write_single_crate_manifest(temp.path());
        let context = make_invocation(temp.path()).process_context().unwrap();
        let mut backend = CleanupTrackingBackend {
            finish_calls: 0,
            after_calls: 0,
            finish_error: Some("console cleanup failed"),
            after_error: Some("post-run cleanup failed"),
        };

        let err = super::finalize_backend_run(&mut backend, &context, Ok(()))
            .await
            .unwrap_err();

        assert_eq!(backend.finish_calls, 1);
        assert_eq!(backend.after_calls, 1);
        assert!(err.to_string().contains("console cleanup failed"));
    }

    #[test]
    fn runner_cleanup_error_precedence_covers_all_combinations() {
        for mask in 0_u8..8 {
            let run_fails = mask & 0b100 != 0;
            let finish_fails = mask & 0b010 != 0;
            let after_fails = mask & 0b001 != 0;
            let result = |fails: bool, message: &'static str| {
                if fails { Err(anyhow!(message)) } else { Ok(()) }
            };

            let (primary, secondary) = super::select_runner_result(
                result(run_fails, "run failed"),
                result(finish_fails, "console cleanup failed"),
                result(after_fails, "post-run cleanup failed"),
            );

            let expected_primary = if run_fails {
                Some("run failed")
            } else if finish_fails {
                Some("console cleanup failed")
            } else if after_fails {
                Some("post-run cleanup failed")
            } else {
                None
            };
            assert_eq!(
                primary.as_ref().err().map(ToString::to_string).as_deref(),
                expected_primary,
                "mask {mask:03b}"
            );

            let phases = secondary
                .iter()
                .map(|failure| failure.phase)
                .collect::<Vec<_>>();
            let expected_phases = if run_fails {
                [
                    finish_fails.then_some("console cleanup"),
                    after_fails.then_some("post-run cleanup"),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
            } else if finish_fails && after_fails {
                vec!["post-run cleanup"]
            } else {
                Vec::new()
            };
            assert_eq!(phases, expected_phases, "mask {mask:03b}");
        }
    }

    fn prepare_elf_only_invocation(dir: &std::path::Path) -> Invocation {
        let source = std::env::current_exe().unwrap();
        let copied = dir.join("sample-elf");
        std::fs::copy(source, &copied).unwrap();

        let mut invocation = make_invocation(dir);
        let prepared = prepare_runtime_artifacts(
            &invocation.process_context().unwrap(),
            RuntimeArtifactOptions {
                elf_path: copied,
                to_bin: false,
                bin_dir: None,
                debug: false,
                cargo_artifact_dir: None,
                strip_elf: false,
            },
        )
        .unwrap();
        invocation.apply_prepared_runtime_artifacts(prepared);
        assert!(invocation.runtime_artifacts().elf().is_some());
        assert!(invocation.runtime_artifacts().bin().is_none());
        invocation
    }

    #[test]
    fn network_boot_request_uses_same_filename_for_bootfile() {
        let request = build_network_boot_request(
            true,
            false,
            true,
            "ostool/home/user/workspace/target/image.fit",
            None,
        )
        .unwrap();

        assert_eq!(
            request.bootfile,
            "ostool/home/user/workspace/target/image.fit"
        );
        assert_eq!(
            request.bootcmd,
            "tftp ostool/home/user/workspace/target/image.fit && bootm"
        );
    }

    #[test]
    fn network_boot_request_uses_tftp_for_static_ip_mode() {
        let request = build_network_boot_request(true, false, true, "image.fit", None).unwrap();

        assert_eq!(request.bootcmd, "tftp image.fit && bootm");
        assert_eq!(request.bootfile, "image.fit");
    }

    #[test]
    fn network_boot_request_requires_ready_transport() {
        assert!(build_network_boot_request(true, false, false, "image.fit", None).is_none());
        assert!(build_network_boot_request(false, false, true, "image.fit", None).is_none());
        assert_eq!(
            build_network_boot_request(false, true, true, "image.fit", None)
                .unwrap()
                .bootcmd,
            "dhcp image.fit && bootm"
        );
    }

    #[test]
    fn network_boot_request_passes_configured_bootm_addr() {
        let request =
            build_network_boot_request(true, false, true, "image.fit", Some(0x82200000)).unwrap();

        assert_eq!(request.bootcmd, "tftp image.fit && bootm 0x82200000");
    }

    #[test]
    fn fit_artifact_path_rejects_non_fit_artifacts() {
        let artifact = BootArtifact::qemu_dtb_dump("target/qemu.dtb");
        let err = fit_artifact_path(&artifact).unwrap_err();

        assert!(
            err.to_string()
                .contains("expected FIT image boot artifact, got QemuDtbDump")
        );
    }

    #[tokio::test]
    async fn run_uboot_prepares_bin_for_elf_only_invocation() {
        let tmp = tempfile::tempdir().unwrap();
        write_single_crate_manifest(tmp.path());
        let mut invocation = prepare_elf_only_invocation(tmp.path());

        let err = super::run_uboot(&mut invocation, &UbootConfig::default())
            .await
            .unwrap_err();

        assert!(invocation.runtime_artifacts().bin().is_some());
        assert!(err.to_string().contains("local U-Boot backend requires"));
    }

    #[tokio::test]
    async fn run_uboot_with_config_rejects_elf_only_input() {
        let tmp = tempfile::tempdir().unwrap();
        write_single_crate_manifest(tmp.path());
        let invocation = prepare_elf_only_invocation(tmp.path());
        let input = super::uboot_run_input(&invocation).unwrap();

        let err = super::run_uboot_with_config(input, UbootConfig::default())
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("U-Boot runner requires a prepared BIN artifact")
        );
    }

    #[tokio::test]
    async fn local_stage_fit_image_without_network_disables_transfer() {
        let temp = tempfile::tempdir().unwrap();
        let fit_path = write_fit_image(temp.path());
        let mut backend = local_backend();

        let staged = backend
            .stage_fit_image(
                &BootArtifact::fit_image(&fit_path),
                &ResolvedRuntime::default(),
            )
            .await
            .unwrap();

        assert_eq!(staged.bootfile(), None);
        assert!(!staged.network_transfer_ready());
    }

    #[tokio::test]
    async fn local_stage_fit_image_uses_existing_tftp_dir_display_path() {
        let temp = tempfile::tempdir().unwrap();
        let fit_path = write_fit_image(temp.path());
        let tftp_dir = temp.path().join("tftp-root");
        let mut backend = local_backend();
        backend.existing_tftp_dir = Some(tftp_dir.clone());

        let staged = backend
            .stage_fit_image(
                &BootArtifact::fit_image(&fit_path),
                &ResolvedRuntime::default(),
            )
            .await
            .unwrap();

        let expected = tftp_dir.join("image.fit").display().to_string();
        assert_eq!(staged.bootfile(), Some(expected.as_str()));
        assert!(staged.network_transfer_ready());
    }

    #[tokio::test]
    async fn local_stage_fit_image_uses_builtin_tftp_filename() {
        let temp = tempfile::tempdir().unwrap();
        let fit_path = write_fit_image(temp.path());
        let mut backend = local_backend();
        backend.builtin_tftp_started = true;

        let staged = backend
            .stage_fit_image(
                &BootArtifact::fit_image(&fit_path),
                &ResolvedRuntime::default(),
            )
            .await
            .unwrap();

        assert_eq!(staged.bootfile(), Some("image.fit"));
        assert!(staged.network_transfer_ready());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn local_stage_fit_image_copies_to_linux_tftp_root() {
        let temp = tempfile::tempdir().unwrap();
        let fit_path = write_fit_image(temp.path());
        let tftp_root = temp.path().join("srv-tftp");
        let mut backend = local_backend();
        backend.linux_system_tftp = Some(tftp::TftpdHpaConfig {
            username: None,
            directory: tftp_root.clone(),
            address: None,
            options: None,
        });

        let staged = backend
            .stage_fit_image(
                &BootArtifact::fit_image(&fit_path),
                &ResolvedRuntime::default(),
            )
            .await
            .unwrap();

        let relative_filename = staged.bootfile().unwrap();
        assert!(relative_filename.starts_with("ostool/"));
        assert!(relative_filename.ends_with("/image.fit"));
        assert!(staged.network_transfer_ready());
        assert_eq!(
            std::fs::read(tftp_root.join(relative_filename)).unwrap(),
            [1_u8, 2, 3, 4]
        );
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn local_backend_cleans_staged_linux_tftp_image() {
        let temp = tempfile::tempdir().unwrap();
        write_single_crate_manifest(temp.path());
        let fit_path = write_fit_image(temp.path());
        let tftp_root = temp.path().join("tftp-root");
        let mut backend = local_backend();
        backend.linux_system_tftp = Some(tftp::TftpdHpaConfig {
            username: None,
            directory: tftp_root,
            address: None,
            options: None,
        });

        backend
            .stage_fit_image(
                &BootArtifact::fit_image(&fit_path),
                &ResolvedRuntime::default(),
            )
            .await
            .unwrap();

        assert_eq!(backend.linux_tftp_staging.len(), 1);
        let staged_file = backend.linux_tftp_staging[0]
            .absolute_fit_path()
            .to_path_buf();
        let staged_dir = backend.linux_tftp_staging[0].target_dir().to_path_buf();
        let context = make_invocation(temp.path()).process_context().unwrap();
        backend.after_run(&context).await.unwrap();

        assert!(!staged_file.exists());
        assert!(!staged_dir.exists());
        assert!(backend.linux_tftp_staging.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn local_backend_cleanup_continues_after_error() {
        let temp = tempfile::tempdir().unwrap();
        write_single_crate_manifest(temp.path());
        let tftp_root = temp.path().join("tftp-root");
        let first_source = write_fit_image(temp.path());
        let second_source = temp.path().join("other.fit");
        std::fs::write(&second_source, [5_u8, 6]).unwrap();
        let first = tftp::stage_linux_fit_image(&first_source, &tftp_root).unwrap();
        let second = tftp::stage_linux_fit_image(&second_source, &tftp_root).unwrap();
        let unexpected = first.target_dir().join("keep.txt");
        std::fs::write(&unexpected, b"keep").unwrap();
        let second_file = second.absolute_fit_path().to_path_buf();
        let second_dir = second.target_dir().to_path_buf();
        let mut backend = local_backend();
        backend.linux_tftp_staging = vec![first, second];

        let context = make_invocation(temp.path()).process_context().unwrap();
        let err = backend.after_run(&context).await.unwrap_err();

        assert!(
            err.to_string()
                .contains("failed to clean local TFTP staging")
        );
        assert!(unexpected.exists());
        assert!(!second_file.exists());
        assert!(!second_dir.exists());
        assert!(backend.linux_tftp_staging.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn local_backend_cleanup_reports_every_failure() {
        let temp = tempfile::tempdir().unwrap();
        write_single_crate_manifest(temp.path());
        let tftp_root = temp.path().join("tftp-root");
        let first_source = write_fit_image(temp.path());
        let second_source = temp.path().join("other.fit");
        std::fs::write(&second_source, [5_u8, 6]).unwrap();
        let first = tftp::stage_linux_fit_image(&first_source, &tftp_root).unwrap();
        let second = tftp::stage_linux_fit_image(&second_source, &tftp_root).unwrap();
        std::fs::write(first.target_dir().join("keep.txt"), b"keep").unwrap();
        std::fs::remove_file(second.absolute_fit_path()).unwrap();
        std::fs::create_dir(second.absolute_fit_path()).unwrap();
        let first_dir = first.target_dir().display().to_string();
        let second_file = second.absolute_fit_path().display().to_string();
        let mut backend = local_backend();
        backend.linux_tftp_staging = vec![first, second];

        let context = make_invocation(temp.path()).process_context().unwrap();
        let err = backend.after_run(&context).await.unwrap_err();
        let message = format!("{err:#}");

        assert!(message.contains(&first_dir));
        assert!(message.contains(&second_file));
        assert!(message.contains("failed to remove TFTP staging directory"));
        assert!(message.contains("failed to remove staged TFTP file"));
        assert!(backend.linux_tftp_staging.is_empty());
    }

    #[cfg(target_os = "linux")]
    #[tokio::test]
    async fn local_backend_power_off_runs_after_cleanup_failure() {
        let temp = tempfile::tempdir().unwrap();
        write_single_crate_manifest(temp.path());
        let tftp_root = temp.path().join("tftp-root");
        let fit_path = write_fit_image(temp.path());
        let prepared = tftp::stage_linux_fit_image(&fit_path, &tftp_root).unwrap();
        std::fs::write(prepared.target_dir().join("keep.txt"), b"keep").unwrap();
        let marker = temp.path().join("powered-off");
        let mut backend = local_backend();
        backend.power_off_cmd = Some(format!("touch {}", marker.display()));
        backend.linux_tftp_staging.push(prepared);

        let context = make_invocation(temp.path()).process_context().unwrap();
        backend.after_run(&context).await.unwrap_err();

        assert!(marker.exists());
        assert!(backend.linux_tftp_staging.is_empty());
    }

    #[tokio::test]
    async fn remote_stage_fit_image_without_tftp_disables_transfer() {
        let temp = tempfile::tempdir().unwrap();
        let fit_path = write_fit_image(temp.path());
        let mut backend = RemoteBackend {
            client: BoardServerClient::new("127.0.0.1", 8080).unwrap(),
            session: SessionCreatedResponse {
                session_id: "demo".into(),
                board_id: "board-1".into(),
                lease_expires_at: chrono::Utc::now(),
                serial_available: true,
                boot_mode: "uboot".into(),
                ws_url: None,
            },
            boot_profile: None,
            serial_status: None,
            tftp_status: Some(TftpSessionResponse {
                available: false,
                provider: "none".into(),
                server_ip: None,
                netmask: None,
                writable: false,
                files: vec![],
            }),
            session_dtb: None,
            console_tasks: None,
        };
        let runtime = ResolvedRuntime {
            use_tftp: true,
            ..Default::default()
        };

        let staged = backend
            .stage_fit_image(&BootArtifact::fit_image(&fit_path), &runtime)
            .await
            .unwrap();

        assert_eq!(staged.bootfile(), None);
        assert!(!staged.network_transfer_ready());
    }

    #[test]
    fn uboot_config_normalize_rejects_shell_check_without_prefix() {
        let mut config = UbootConfig {
            shell_check_steps: vec![ShellCheckStep {
                shell_cmd: Some("root".into()),
                ..Default::default()
            }],
            local: LocalUbootConfig {
                serial: Some("/dev/null".into()),
                baud_rate: Some("115200".into()),
                ..Default::default()
            },
            ..Default::default()
        };

        let err = config.normalize("test config").unwrap_err();
        assert!(err.to_string().contains("shell_prefix"));
    }

    #[test]
    fn uboot_config_normalize_trims_prefix_and_preserves_command() {
        let mut config = UbootConfig {
            shell_check_steps: vec![ShellCheckStep {
                shell_prefix: Some(" login: ".into()),
                shell_cmd: Some(" root ".into()),
                ..Default::default()
            }],
            local: LocalUbootConfig {
                serial: Some("/dev/null".into()),
                baud_rate: Some("115200".into()),
                ..Default::default()
            },
            ..Default::default()
        };

        config.normalize("test config").unwrap();

        assert_eq!(
            config.shell_check_steps[0].shell_prefix.as_deref(),
            Some("login:")
        );
        assert_eq!(
            config.shell_check_steps[0].shell_cmd.as_deref(),
            Some(" root ")
        );
    }

    #[test]
    fn uboot_timeout_zero_disables_timeout() {
        assert_eq!(timeout_duration(None), None);
        assert_eq!(timeout_duration(Some(0)), None);
        assert_eq!(timeout_duration(Some(5)), Some(Duration::from_secs(5)));
    }

    #[tokio::test]
    async fn uboot_shell_initialization_times_out_without_serial_response() {
        let (host, _peer) = tokio::io::duplex(64);
        let (rx, tx) = tokio::io::split(host);
        let result = tokio::time::timeout(
            Duration::from_millis(50),
            super::new_uboot_shell(tx.compat_write(), rx.compat(), Duration::from_millis(10)),
        )
        .await;

        let error = match result {
            Ok(Err(error)) => error,
            Ok(Ok(_)) => panic!("U-Boot shell unexpectedly initialized"),
            Err(_) => panic!("U-Boot shell initialization did not return a bounded error"),
        };
        assert!(
            error
                .to_string()
                .contains("timed out waiting for U-Boot shell")
        );
    }

    #[test]
    fn uboot_config_parses_timeout_from_toml() {
        let config: UbootConfig = toml::from_str(
            r#"
serial = "/dev/null"
baud_rate = "115200"
fail_regex = []
timeout = 0
"#,
        )
        .unwrap();

        assert_eq!(config.timeout, Some(0));
    }

    #[test]
    fn uboot_config_parses_network_into_local_backend() {
        let config: UbootConfig = toml::from_str(
            r#"
serial = "/dev/null"
baud_rate = "115200"
fail_regex = []

[net]
interface = "eth0"
"#,
        )
        .unwrap();

        let net = config.local.net.unwrap();
        assert_eq!(net.interface, "eth0");
    }

    #[test]
    fn uboot_config_parses_board_commands_at_top_level() {
        let config: UbootConfig = toml::from_str(
            r#"
serial = "/dev/null"
baud_rate = "115200"
fail_regex = []
board_reset_cmd = "reset-board"
board_power_off_cmd = "power-off-board"
"#,
        )
        .unwrap();

        assert_eq!(config.board_reset_cmd.as_deref(), Some("reset-board"));
        assert_eq!(
            config.board_power_off_cmd.as_deref(),
            Some("power-off-board")
        );
        assert_eq!(config.local.board_reset_cmd, None);
        assert_eq!(config.local.board_power_off_cmd, None);

        let serialized = toml::to_string(&config).unwrap();
        assert_eq!(serialized.matches("board_reset_cmd").count(), 1);
        assert_eq!(serialized.matches("board_power_off_cmd").count(), 1);

        let backend = LocalBackend::new(
            config.local,
            config.board_reset_cmd,
            config.board_power_off_cmd,
        );
        assert_eq!(backend.reset_cmd.as_deref(), Some("reset-board"));
        assert_eq!(backend.power_off_cmd.as_deref(), Some("power-off-board"));
    }

    #[test]
    fn local_backend_keeps_legacy_command_fields_usable() {
        let backend = LocalBackend::new(
            LocalUbootConfig {
                board_reset_cmd: Some("legacy-reset".into()),
                board_power_off_cmd: Some("legacy-power-off".into()),
                ..Default::default()
            },
            None,
            None,
        );

        assert_eq!(backend.reset_cmd.as_deref(), Some("legacy-reset"));
        assert_eq!(backend.power_off_cmd.as_deref(), Some("legacy-power-off"));
    }

    #[test]
    fn uboot_config_replaces_string_fields() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/lib.rs"), "").unwrap();

        let mut invocation = make_invocation(tmp.path());
        crate::build::activate_build_config(
            &mut invocation,
            &BuildConfig {
                system: BuildSystem::Cargo(Box::new(Cargo {
                    env: HashMap::new(),
                    target: "aarch64-unknown-none".into(),
                    package: "sample".into(),
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
        unsafe {
            std::env::set_var("OSTOOL_UBOOT_TEST_ENV", "env-ok");
        }

        let mut config = UbootConfig {
            dtb_file: Some("${package}/board.dtb".into()),
            kernel_load_addr: Some("${workspaceFolder}".into()),
            fit_load_addr: Some("${package}".into()),
            bootm_addr: Some("${workspace}".into()),
            board_reset_cmd: Some("${workspace}".into()),
            board_power_off_cmd: Some("${package}".into()),
            fail_regex: vec!["${package}".into()],
            uboot_cmd: Some(vec!["setenv boot ${workspace}".into()]),
            shell_check_steps: vec![ShellCheckStep {
                shell_prefix: Some("${workspace}".into()),
                shell_cmd: Some("${package}".into()),
                success_regex: Some(vec!["${workspace}".into()]),
                ..Default::default()
            }],
            local: LocalUbootConfig {
                serial: Some("${workspace}/tty".into()),
                baud_rate: Some("${env:OSTOOL_UBOOT_TEST_ENV}".into()),
                net: Some(Net {
                    interface: "${env:OSTOOL_UBOOT_TEST_ENV}".into(),
                    board_ip: Some("${workspace}".into()),
                    gatewayip: Some("${package}".into()),
                    netmask: Some("${workspaceFolder}".into()),
                    tftp_dir: Some("${package}/tftp".into()),
                }),
                ..Default::default()
            },
            ..Default::default()
        };

        config
            .replace_strings(&invocation.variable_scope().unwrap())
            .unwrap();

        let expected = tmp.path().display().to_string();
        assert_eq!(
            config.local.serial.as_deref(),
            Some(format!("{expected}/tty").as_str())
        );
        assert_eq!(config.local.baud_rate.as_deref(), Some("env-ok"));
        assert_eq!(
            config.dtb_file.as_deref(),
            Some(format!("{expected}/board.dtb").as_str())
        );
        assert_eq!(config.kernel_load_addr.as_deref(), Some(expected.as_str()));
        assert_eq!(config.fit_load_addr.as_deref(), Some(expected.as_str()));
        assert_eq!(config.bootm_addr.as_deref(), Some(expected.as_str()));
        assert_eq!(config.board_reset_cmd.as_deref(), Some(expected.as_str()));
        assert_eq!(
            config.board_power_off_cmd.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(config.fail_regex, vec![expected.clone()]);
        assert_eq!(
            config.shell_check_steps[0].success_regex.as_deref(),
            Some(&[expected.clone()][..])
        );
        assert_eq!(
            config.uboot_cmd,
            Some(vec![format!("setenv boot {expected}")])
        );
        assert_eq!(
            config.shell_check_steps[0].shell_prefix.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            config.shell_check_steps[0].shell_cmd.as_deref(),
            Some(expected.as_str())
        );
        let net = config.local.net.unwrap();
        assert_eq!(net.interface, "env-ok");
        assert_eq!(net.board_ip.as_deref(), Some(expected.as_str()));
        assert_eq!(net.gatewayip.as_deref(), Some(expected.as_str()));
        assert_eq!(net.netmask.as_deref(), Some(expected.as_str()));
        assert_eq!(
            net.tftp_dir.as_deref(),
            Some(format!("{expected}/tftp").as_str())
        );
    }

    #[test]
    fn uboot_config_from_board_run_config_keeps_dtb_file() {
        let config = UbootConfig::from_board_run_config(&BoardRunConfig {
            board_type: "rk3568".into(),
            session_files: Vec::new(),
            dtb_file: Some("/tmp/board.dtb".into()),
            kernel_load_addr: Some("0x80200000".into()),
            fit_load_addr: Some("0x82200000".into()),
            bootm_addr: Some("0x82200000".into()),
            fail_regex: vec!["fail".into()],
            uboot_cmd: Some(vec!["run ab_select_cmd".into(), "run avb_boot".into()]),
            shell_check_steps: Vec::new(),
            timeout: Some(12),
            auth_mode: None,
            server: None,
            port: None,
        });

        assert_eq!(config.dtb_file.as_deref(), Some("/tmp/board.dtb"));
        assert_eq!(config.kernel_load_addr.as_deref(), Some("0x80200000"));
        assert_eq!(config.fit_load_addr.as_deref(), Some("0x82200000"));
        assert_eq!(config.bootm_addr.as_deref(), Some("0x82200000"));
        assert_eq!(config.timeout, Some(12));
        assert_eq!(
            config.uboot_cmd,
            Some(vec![
                "run ab_select_cmd".to_string(),
                "run avb_boot".to_string()
            ])
        );
    }

    #[test]
    fn uboot_config_from_board_run_config_keeps_shell_check_steps() {
        let board_config = BoardRunConfig {
            shell_check_steps: vec![crate::run::ShellCheckStep {
                shell_prefix: Some("axvisor:/$".into()),
                shell_cmd: Some("vm console 1".into()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let config = UbootConfig::from_board_run_config(&board_config);

        assert_eq!(config.shell_check_steps, board_config.shell_check_steps);
    }

    #[tokio::test]
    async fn ensure_uboot_config_in_dir_creates_default_file() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(tmp.path().join("src")).unwrap();
        std::fs::write(tmp.path().join("src/lib.rs"), "").unwrap();

        let invocation = make_invocation(tmp.path());

        let config = ensure_config_in_dir(&invocation, tmp.path()).await.unwrap();

        assert_eq!(config.local.serial.as_deref(), Some("/dev/ttyUSB0"));
        assert_eq!(config.local.baud_rate.as_deref(), Some("115200"));
        assert!(tmp.path().join(".uboot.toml").exists());
    }

    #[tokio::test]
    async fn ensure_uboot_config_in_dir_replaces_package_variables() {
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
            tmp.path().join(".uboot.toml"),
            r#"
dtb_file = "${package}/board.dtb"
fail_regex = []
serial = "/dev/null"
baud_rate = "115200"
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

        let config = ensure_config_in_dir(&invocation, tmp.path()).await.unwrap();
        let expected = kernel_dir.join("board.dtb").display().to_string();
        assert_eq!(config.dtb_file.as_deref(), Some(expected.as_str()));
    }
}
