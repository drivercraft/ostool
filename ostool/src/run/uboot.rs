use std::{
    io,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::Context;
use async_trait::async_trait;
use byte_unit::Byte;
use colored::Colorize;
use fitimage::{ComponentConfig, FitImageBuilder, FitImageConfig};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use log::{info, warn};
use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
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
    invocation::Invocation,
    process::ProcessContext,
    project::variables::{self, VariableScope},
    run::{
        output_matcher::{
            ByteStreamMatcher, MATCH_DRAIN_DURATION, compile_regexes, print_match_event,
        },
        shell_init::{SHELL_INIT_DELAY, ShellAutoInitMatcher, normalize_shell_init_config},
        tftp,
    },
    sterm::{AsyncTerminal, TerminalConfig},
    utils::PathResultExt,
};

/// FIT image 生成相关的错误消息常量
mod errors {
    pub const KERNEL_READ_ERROR: &str = "读取 kernel 文件失败";
    pub const DTB_READ_ERROR: &str = "读取 DTB 文件失败";
    pub const FIT_BUILD_ERROR: &str = "构建 FIT image 失败";
    pub const FIT_SAVE_ERROR: &str = "保存 FIT image 失败";
    pub const DIR_ERROR: &str = "无法获取 kernel 文件目录";
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
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
    /// TFTP boot configuration
    pub net: Option<Net>,
    /// Board reset command
    /// shell command to reset the board
    pub board_reset_cmd: Option<String>,
    /// Board power off command
    /// shell command to power off the board
    pub board_power_off_cmd: Option<String>,
    pub success_regex: Vec<String>,
    pub fail_regex: Vec<String>,
    pub uboot_cmd: Option<Vec<String>>,
    /// String prefix that indicates the target shell is ready after boot.
    pub shell_prefix: Option<String>,
    /// Command sent once after `shell_prefix` is detected.
    pub shell_init_cmd: Option<String>,
    /// Timeout in seconds after entering the serial terminal interaction stage. `None` or `0`
    /// disables the timeout.
    pub timeout: Option<u64>,
    #[serde(flatten)]
    pub local: LocalUbootConfig,
}

#[derive(Default, Serialize, Deserialize, JsonSchema, Debug, Clone)]
pub struct LocalUbootConfig {
    /// Serial console device
    /// e.g., /dev/ttyUSB0 on linux, COM3 on Windows
    pub serial: Option<String>,
    pub baud_rate: Option<String>,
    /// TFTP boot configuration
    pub net: Option<Net>,
    /// Board reset command
    /// shell command to reset the board
    pub board_reset_cmd: Option<String>,
    /// Board power off command
    /// shell command to power off the board
    pub board_power_off_cmd: Option<String>,
}

impl UbootConfig {
    pub fn from_board_run_config(config: &BoardRunConfig) -> Self {
        Self {
            dtb_file: config.dtb_file.clone(),
            kernel_load_addr: config.kernel_load_addr.clone(),
            fit_load_addr: config.fit_load_addr.clone(),
            bootm_addr: config.bootm_addr.clone(),
            success_regex: config.success_regex.clone(),
            fail_regex: config.fail_regex.clone(),
            uboot_cmd: config.uboot_cmd.clone(),
            shell_prefix: config.shell_prefix.clone(),
            shell_init_cmd: config.shell_init_cmd.clone(),
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
        normalize_shell_init_config(
            &mut self.shell_prefix,
            &mut self.shell_init_cmd,
            config_name,
        )
    }

    fn shell_auto_init(&self) -> Option<ShellAutoInitMatcher> {
        ShellAutoInitMatcher::new(self.shell_prefix.clone(), self.shell_init_cmd.clone())
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
    let backend = LocalBackend::new(config.local.clone());
    let mut runner = Runner::new(input, config, backend);
    runner.run().await
}

/// Runs an already prepared artifact via U-Boot.
pub async fn run_uboot(invocation: &mut Invocation, config: &UbootConfig) -> anyhow::Result<()> {
    let scope = invocation.variable_scope()?;
    let config = prepare_uboot_runtime_config(&scope, config)?;
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
    success_regex: Vec<regex::Regex>,
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

struct PreparedBootArtifact {
    bootfile: Option<String>,
    network_transfer_ready: bool,
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
        fitimage: &Path,
        runtime: &ResolvedRuntime,
    ) -> anyhow::Result<PreparedBootArtifact>;
    async fn finish_console(&mut self) -> anyhow::Result<()>;
    async fn after_run(&mut self, context: &ProcessContext) -> anyhow::Result<()>;
}

struct LocalBackend {
    config: LocalUbootConfig,
    baud_rate: Option<u32>,
    linux_system_tftp: Option<tftp::TftpdHpaConfig>,
    existing_tftp_dir: Option<PathBuf>,
    builtin_tftp_started: bool,
}

impl LocalBackend {
    fn new(config: LocalUbootConfig) -> Self {
        Self {
            config,
            baud_rate: None,
            linux_system_tftp: None,
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
                info!("TFTP server IP: {}", ip);
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
                info!("TFTP server IP: {}", ip);
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

        info!("Opening serial port: {} @ {}", serial, baud_rate);
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
        if let Some(cmd) = self.config.board_reset_cmd.as_deref()
            && !cmd.trim().is_empty()
        {
            crate::process::shell_run_cmd(context, cmd)?;
        }
        Ok(())
    }

    async fn stage_fit_image(
        &mut self,
        fitimage: &Path,
        _runtime: &ResolvedRuntime,
    ) -> anyhow::Result<PreparedBootArtifact> {
        let Some(file_name) = fitimage.file_name().and_then(|name| name.to_str()) else {
            return Err(anyhow!("Invalid fitimage filename"));
        };

        #[cfg(target_os = "linux")]
        {
            if let Some(system_tftp) = self.linux_system_tftp.as_ref() {
                let prepared = tftp::stage_linux_fit_image(fitimage, &system_tftp.directory)?;
                info!(
                    "Staged FIT image to: {}",
                    prepared.absolute_fit_path.display()
                );
                return Ok(PreparedBootArtifact {
                    bootfile: Some(prepared.relative_filename),
                    network_transfer_ready: true,
                });
            }
        }

        if let Some(tftp_dir) = self.existing_tftp_dir.as_deref() {
            let tftp_path = PathBuf::from(tftp_dir).join(file_name);
            info!("Setting TFTP file path: {}", tftp_path.display());
            return Ok(PreparedBootArtifact {
                bootfile: Some(tftp_path.display().to_string()),
                network_transfer_ready: true,
            });
        }

        if self.builtin_tftp_started {
            info!("Using fitimage filename: {}", file_name);
            return Ok(PreparedBootArtifact {
                bootfile: Some(file_name.to_string()),
                network_transfer_ready: true,
            });
        }

        Ok(PreparedBootArtifact {
            bootfile: None,
            network_transfer_ready: false,
        })
    }

    async fn finish_console(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn after_run(&mut self, context: &ProcessContext) -> anyhow::Result<()> {
        if let Some(cmd) = self.config.board_power_off_cmd.as_deref()
            && !cmd.trim().is_empty()
            && let Err(err) = crate::process::shell_run_cmd(context, cmd)
        {
            log::warn!("board power-off command failed: {err:#}");
        }
        Ok(())
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
                format!(
                    "failed to get boot profile for session `{}`",
                    self.session.session_id
                )
            })?;
        let serial_status = self
            .client
            .get_serial_status(&self.session.session_id)
            .await
            .with_context(|| {
                format!(
                    "failed to get serial status for session `{}`",
                    self.session.session_id
                )
            })?;
        let tftp_status = self
            .client
            .get_tftp_status(&self.session.session_id)
            .await
            .with_context(|| {
                format!(
                    "failed to get tftp status for session `{}`",
                    self.session.session_id
                )
            })?;

        let profile = match &boot_profile.boot {
            RemoteBootConfig::Uboot(profile) => profile.clone(),
            other => {
                return Err(anyhow!(
                    "unsupported remote boot mode `{:?}`; only `uboot` is supported",
                    other
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
        let (tx, rx, tasks) = connect_serial_stream(ws_url).await?;
        self.console_tasks = Some(tasks);
        Ok(ConsoleTransport { tx, rx })
    }

    async fn after_console_open(&mut self, _context: &ProcessContext) -> anyhow::Result<()> {
        println!("Waiting for remote board to power on through ostool-server...");
        Ok(())
    }

    async fn stage_fit_image(
        &mut self,
        fitimage: &Path,
        runtime: &ResolvedRuntime,
    ) -> anyhow::Result<PreparedBootArtifact> {
        let tftp_status = self
            .tftp_status
            .as_ref()
            .ok_or_else(|| anyhow!("remote runtime not initialized"))?;
        if !runtime.use_tftp || !tftp_status.available {
            return Ok(PreparedBootArtifact {
                bootfile: None,
                network_transfer_ready: false,
            });
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

        Ok(PreparedBootArtifact {
            bootfile: Some(uploaded.relative_path),
            network_transfer_ready: true,
        })
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

impl<B> Runner<B>
where
    B: RunnerBackend,
{
    fn new(input: UbootRunInput, config: UbootConfig, backend: B) -> Self {
        Self {
            input,
            config,
            success_regex: vec![],
            fail_regex: vec![],
            backend,
        }
    }

    /// 生成包含 kernel 和 FDT 的压缩 FIT image。
    async fn generate_fit_image(
        &self,
        kernel_path: &Path,
        dtb_path: Option<&Path>,
        kernel_load_addr: u64,
        kernel_entry_addr: u64,
        fdt_load_addr: Option<u64>,
        _ramfs_load_addr: Option<u64>,
    ) -> anyhow::Result<PathBuf> {
        info!("Making FIT image...");
        // 生成压缩的 FIT image
        let output_dir = kernel_path
            .parent()
            .and_then(|p| p.to_str())
            .ok_or_else(|| anyhow!("{}: {}", errors::DIR_ERROR, kernel_path.display()))?;

        // 读取 kernel 数据
        let kernel_data = fs::read(kernel_path)
            .await
            .with_path(errors::KERNEL_READ_ERROR, kernel_path)?;

        info!(
            "kernel: {} (size: {:.2})",
            kernel_path.display(),
            Byte::from(kernel_data.len())
        );

        let arch = self
            .input
            .arch
            .ok_or_else(|| anyhow!("Cannot determine architecture for FIT image generation"))?;
        let arch = match arch {
            object::Architecture::Aarch64 => "arm64",
            object::Architecture::Arm => "arm",
            object::Architecture::LoongArch64 => "loongarch64",
            object::Architecture::Riscv64 => "riscv",
            other => anyhow::bail!("unsupported architecture for FIT image generation: {other:?}"),
        };

        let mut config = FitImageConfig::new("Various kernels, ramdisks and FDT blobs")
            .with_kernel(
                ComponentConfig::new("kernel", kernel_data)
                    .with_description("This kernel")
                    .with_type("kernel")
                    .with_arch(arch)
                    .with_os("linux")
                    .with_compression(false)
                    .with_load_address(kernel_load_addr)
                    .with_entry_point(kernel_entry_addr),
            );
        let mut fdt_name = None;

        // 处理 DTB 文件
        if let Some(dtb_path) = dtb_path {
            let data = fs::read(dtb_path)
                .await
                .with_path(errors::DTB_READ_ERROR, dtb_path)?;
            info!(
                "已读取 DTB 文件: {} (大小: {:.2})",
                dtb_path.display(),
                Byte::from(data.len())
            );
            fdt_name = Some("fdt");

            // U-Boot 不接受压缩的 DTB
            let mut fdt_config = ComponentConfig::new("fdt", data.clone())
                .with_description("This fdt")
                .with_type("flat_dt")
                .with_arch(arch);

            if let Some(addr) = fdt_load_addr {
                fdt_config = fdt_config.with_load_address(addr);
            }

            config = config.with_fdt(fdt_config);
        } else {
            warn!("未指定 DTB 文件，将生成仅包含 kernel 的 FIT image");
        }

        config = config
            .with_default_config("config-ostool")
            .with_configuration(
                "config-ostool",
                "ostool configuration",
                Some("kernel"),
                fdt_name,
                None::<String>,
            );

        let mut builder = FitImageBuilder::new();
        let fit_data = builder
            .build(config)
            .with_context(|| errors::FIT_BUILD_ERROR.to_string())?;
        let output_path = Path::new(output_dir).join("image.fit");
        fs::write(&output_path, fit_data)
            .await
            .with_path(errors::FIT_SAVE_ERROR, &output_path)?;

        info!("FIT image ok: {}", output_path.display());
        Ok(output_path)
    }

    async fn run(&mut self) -> anyhow::Result<()> {
        let run_result = self._run().await;

        if let Err(err) = self.backend.finish_console().await {
            if run_result.is_ok() {
                return Err(err);
            }
            log::warn!("backend console cleanup failed: {err:#}");
        }

        if let Err(err) = self.backend.after_run(&self.input.process_context).await {
            if run_result.is_ok() {
                return Err(err);
            }
            log::warn!("backend post-run cleanup failed: {err:#}");
        }

        run_result
    }

    async fn _run(&mut self) -> anyhow::Result<()> {
        self.prepare_regex()?;

        let kernel = self
            .input
            .artifacts
            .runtime_image()
            .ok_or_else(|| anyhow!("U-Boot runner requires a prepared runtime image"))?
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
        let mut uboot = UbootShell::new(tx, rx).await?;
        uboot.set_env("autoload", "yes").await?;

        if let Some(ref cmds) = self.config.uboot_cmd {
            for cmd in cmds.iter() {
                info!("Running U-Boot command: {}", cmd);
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
        let mut ramfs_load_addr = None;

        if let Ok(addr) = uboot.env_int("fdt_addr_r").await {
            fdt_load_addr = Some(addr as u64);
        }

        if let Ok(addr) = uboot.env_int("ramdisk_addr_r").await {
            ramfs_load_addr = Some(addr as u64);
        }

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
            .set_env("loadaddr", format!("{:#x}", fit_loadaddr))
            .await?;

        info!("fitimage loadaddr: {fit_loadaddr:#x}");
        info!("kernel entry: {kernel_entry:#x}");
        if let Some(ref dtb_path) = prepared_dtb.fit_source {
            info!("Using DTB from: {}", dtb_path.display());
        }
        let fitimage = self
            .generate_fit_image(
                &kernel,
                prepared_dtb.fit_source.as_deref(),
                kernel_entry,
                kernel_entry,
                fdt_load_addr,
                ramfs_load_addr,
            )
            .await?;

        let prepared = self.backend.stage_fit_image(&fitimage, &runtime).await?;

        let bootm_arg = self.resolved_bootm_arg(fit_loadaddr, &runtime);
        let bootcmd = if let Some(fitname) = prepared.bootfile.as_deref() {
            if let Some(request) = build_network_boot_request(
                runtime.static_ip,
                net_ok,
                prepared.network_transfer_ready,
                fitname,
                bootm_arg,
            ) {
                uboot.set_env("bootfile", &request.bootfile).await?;
                request.bootcmd
            } else {
                info!("No network boot request available, using loady to upload FIT image...");
                Self::uboot_loady(&mut uboot, fit_loadaddr as usize, fitimage).await?;
                self.serial_bootm_command(bootm_arg)
            }
        } else {
            info!("No TFTP config, using loady to upload FIT image...");
            Self::uboot_loady(&mut uboot, fit_loadaddr as usize, fitimage).await?;
            self.serial_bootm_command(bootm_arg)
        };

        info!("Booting kernel with command: {}", bootcmd);
        uboot.cmd_without_reply(&bootcmd).await?;

        println!("{}", "Interacting with U-Boot shell...".green());

        let matcher = Arc::new(Mutex::new(ByteStreamMatcher::new(
            self.success_regex.clone(),
            self.fail_regex.clone(),
        )));

        let res = Arc::new(Mutex::<Option<anyhow::Result<()>>>::new(None));
        let res_clone = res.clone();
        let matcher_clone = matcher.clone();
        let shell_init = Arc::new(Mutex::new(self.config.shell_auto_init()));
        let shell_init_clone = shell_init.clone();
        let mut serial_rx = uboot.rx.take().unwrap().compat();
        let mut serial_tx = uboot.tx.take().unwrap().compat_write();
        drop(uboot);
        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (outbound_tx, mut outbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();

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
            while let Some(bytes) = outbound_rx.recv().await {
                serial_tx
                    .write_all(&bytes)
                    .await
                    .context("failed to write serial input")?;
                serial_tx
                    .flush()
                    .await
                    .context("failed to flush serial input")?;
            }
            Ok::<(), anyhow::Error>(())
        });

        let terminal = AsyncTerminal::new(TerminalConfig {
            intercept_exit_sequence: true,
            timeout: timeout_duration(self.config.timeout),
            timeout_label: "kernel boot".to_string(),
        });
        terminal
            .run(inbound_rx, outbound_tx, move |h, byte| {
                let mut matcher = matcher_clone.lock().unwrap();
                if let Some(matched) = matcher.observe_byte(byte) {
                    print_match_event(&matched);
                    let mut res_lock = res_clone.lock().unwrap();
                    *res_lock = Some(matched.kind.into_result(&matched));
                    h.stop_after(MATCH_DRAIN_DURATION);
                }

                let mut shell_init = shell_init_clone.lock().unwrap();
                if let Some(shell_init) = shell_init.as_mut()
                    && let Some(command) = shell_init.observe_byte(byte)
                {
                    h.send_after(SHELL_INIT_DELAY, command);
                }

                if matcher.should_stop() {
                    h.stop();
                }
            })
            .await?;
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
            if let Some(result) = res_lock.take() {
                result?;
            }
        }
        Ok(())
    }

    fn prepare_regex(&mut self) -> anyhow::Result<()> {
        let (success, fail) = compile_regexes(&self.config.success_regex, &self.config.fail_regex)?;
        self.success_regex = success;
        self.fail_regex = fail;
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

        println!("{}", res);
        println!("send ok");
        Ok(())
    }
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

    info!("TFTP : {}", ip_string);
    Some(ip_string)
}

fn timeout_duration(timeout: Option<u64>) -> Option<Duration> {
    match timeout {
        Some(0) | None => None,
        Some(secs) => Some(Duration::from_secs(secs)),
    }
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
    use std::{collections::HashMap, time::Duration};

    use super::{
        LocalUbootConfig, Net, UbootConfig, build_network_boot_request, ensure_config_in_dir,
        timeout_duration,
    };
    use crate::{
        board::config::BoardRunConfig,
        build::config::{BuildConfig, BuildSystem, Cargo},
        invocation::{Invocation, InvocationOptions},
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
    fn uboot_config_normalize_rejects_shell_init_without_prefix() {
        let mut config = UbootConfig {
            shell_init_cmd: Some("root".into()),
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
    fn uboot_config_normalize_trims_shell_fields() {
        let mut config = UbootConfig {
            shell_prefix: Some(" login: ".into()),
            shell_init_cmd: Some(" root ".into()),
            local: LocalUbootConfig {
                serial: Some("/dev/null".into()),
                baud_rate: Some("115200".into()),
                ..Default::default()
            },
            ..Default::default()
        };

        config.normalize("test config").unwrap();

        assert_eq!(config.shell_prefix.as_deref(), Some("login:"));
        assert_eq!(config.shell_init_cmd.as_deref(), Some("root"));
    }

    #[test]
    fn uboot_timeout_zero_disables_timeout() {
        assert_eq!(timeout_duration(None), None);
        assert_eq!(timeout_duration(Some(0)), None);
        assert_eq!(timeout_duration(Some(5)), Some(Duration::from_secs(5)));
    }

    #[test]
    fn uboot_config_parses_timeout_from_toml() {
        let config: UbootConfig = toml::from_str(
            r#"
serial = "/dev/null"
baud_rate = "115200"
success_regex = []
fail_regex = []
timeout = 0
"#,
        )
        .unwrap();

        assert_eq!(config.timeout, Some(0));
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
                system: BuildSystem::Cargo(Cargo {
                    env: HashMap::new(),
                    target: "aarch64-unknown-none".into(),
                    package: "sample".into(),
                    bin: None,
                    features: vec![],
                    log: None,
                    extra_config: None,
                    profile: None,
                    disable_someboot_build_config: false,
                    args: vec![],
                    pre_build_cmds: vec![],
                    post_build_cmds: vec![],
                    to_bin: false,
                }),
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
            success_regex: vec!["${workspace}".into()],
            fail_regex: vec!["${package}".into()],
            uboot_cmd: Some(vec!["setenv boot ${workspace}".into()]),
            shell_prefix: Some("${workspace}".into()),
            shell_init_cmd: Some("${package}".into()),
            local: LocalUbootConfig {
                serial: Some("${workspace}/tty".into()),
                baud_rate: Some("${env:OSTOOL_UBOOT_TEST_ENV}".into()),
                board_reset_cmd: Some("${workspace}".into()),
                board_power_off_cmd: Some("${package}".into()),
                net: Some(Net {
                    interface: "${env:OSTOOL_UBOOT_TEST_ENV}".into(),
                    board_ip: Some("${workspace}".into()),
                    gatewayip: Some("${package}".into()),
                    netmask: Some("${workspaceFolder}".into()),
                    tftp_dir: Some("${package}/tftp".into()),
                }),
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
        assert_eq!(
            config.local.board_reset_cmd.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            config.local.board_power_off_cmd.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(config.success_regex, vec![expected.clone()]);
        assert_eq!(config.fail_regex, vec![expected.clone()]);
        assert_eq!(
            config.uboot_cmd,
            Some(vec![format!("setenv boot {expected}")])
        );
        assert_eq!(config.shell_prefix.as_deref(), Some(expected.as_str()));
        assert_eq!(config.shell_init_cmd.as_deref(), Some(expected.as_str()));
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
            dtb_file: Some("/tmp/board.dtb".into()),
            kernel_load_addr: Some("0x80200000".into()),
            fit_load_addr: Some("0x82200000".into()),
            bootm_addr: Some("0x82200000".into()),
            success_regex: vec!["ok".into()],
            fail_regex: vec!["fail".into()],
            uboot_cmd: Some(vec!["run ab_select_cmd".into(), "run avb_boot".into()]),
            shell_prefix: Some("login:".into()),
            shell_init_cmd: Some("root".into()),
            timeout: Some(12),
            server: None,
            port: None,
        });

        assert_eq!(config.dtb_file.as_deref(), Some("/tmp/board.dtb"));
        assert_eq!(config.kernel_load_addr.as_deref(), Some("0x80200000"));
        assert_eq!(config.fit_load_addr.as_deref(), Some("0x82200000"));
        assert_eq!(config.bootm_addr.as_deref(), Some("0x82200000"));
        assert_eq!(config.success_regex, vec!["ok"]);
        assert_eq!(config.timeout, Some(12));
        assert_eq!(
            config.uboot_cmd,
            Some(vec![
                "run ab_select_cmd".to_string(),
                "run avb_boot".to_string()
            ])
        );
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
success_regex = []
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
                system: BuildSystem::Cargo(Cargo {
                    env: HashMap::new(),
                    target: "aarch64-unknown-none".into(),
                    package: "kernel".into(),
                    bin: None,
                    features: vec![],
                    log: None,
                    extra_config: None,
                    profile: None,
                    disable_someboot_build_config: false,
                    args: vec![],
                    pre_build_cmds: vec![],
                    post_build_cmds: vec![],
                    to_bin: false,
                }),
            },
            None,
        )
        .unwrap();

        let config = ensure_config_in_dir(&invocation, tmp.path()).await.unwrap();
        let expected = kernel_dir.join("board.dtb").display().to_string();
        assert_eq!(config.dtb_file.as_deref(), Some(expected.as_str()));
    }
}
