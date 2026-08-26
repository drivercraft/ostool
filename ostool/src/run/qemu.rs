//! QEMU emulator runner with UEFI/OVMF support.
//!
//! This module provides functionality for running operating systems in QEMU
//! with support for:
//!
//! - Multiple architectures (x86_64, aarch64, riscv64, etc.)
//! - UEFI boot via OVMF firmware
//! - Debug mode with GDB server
//! - Output pattern matching for test automation
//!
//! # Configuration
//!
//! QEMU configuration is stored in `.qemu.toml` files:
//!
//! ```toml
//! args = ["-nographic", "-cpu", "cortex-a53"]
//! uefi = false
//! # `to_bin` remains supported for explicit legacy configurations, but QEMU
//! # UEFI boot prepares the required BIN artifact automatically.
//! to_bin = true
//! fail_regex = ["PANIC", "FAILED"]
//! ```

use std::{
    ffi::OsString,
    io::{self, ErrorKind},
    path::Path,
    path::PathBuf,
    process::Stdio,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use anyhow::{Context, anyhow};
#[cfg(windows)]
use colored::Colorize;
use object::Architecture;
use schemars::JsonSchema;
use serde::{Deserialize, Deserializer, Serialize};
use tokio::{
    fs,
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command as TokioCommand,
    sync::mpsc,
};

use crate::{
    artifact::state::OutputArtifacts,
    boot::artifacts::{default_qemu_dtb_dump_path, prepare_qemu_dtb_dump},
    build::config::Cargo,
    invocation::Invocation,
    ovmf::{Arch, FileType, Prebuilt, Source, default_cache_dir},
    process::ProcessContext,
    project::variables::{self, VariableScope},
    project::{ProjectLayout, metadata},
    run::{
        execution::{RunnerExecutionSummary, RunnerExitStatus, timeout_duration},
        output_matcher::{FailStreamMatcher, compile_fail_regexes, print_fail_match},
        qemu_plan::{QemuBootSource, QemuCommandPlanInput, build_qemu_command_plan},
        shell_check::{
            ShellCheckDriver, ShellCheckMatcher, ShellCheckStep, normalize_shell_check_steps,
        },
    },
    sterm::{AsyncTerminal, TerminalConfig},
    utils::PathResultExt,
};

enum UefiBootConfig {
    Pflash {
        code: PathBuf,
        vars: PathBuf,
        esp_dir: PathBuf,
    },
}

/// QEMU configuration structure.
///
/// This configuration is typically loaded from a `.qemu.toml` file.
#[derive(Debug, Clone, Serialize, JsonSchema, PartialEq, Eq, Default)]
pub struct QemuConfig {
    /// Additional QEMU command-line arguments.
    pub args: Vec<String>,
    /// Whether to use UEFI boot via OVMF firmware.
    pub uefi: bool,
    /// Legacy explicit request to prepare a raw BIN before loading.
    ///
    /// Runners that require BIN artifacts, such as UEFI boot, prepare them
    /// automatically even when this is unset.
    #[serde(default)]
    pub to_bin: bool,
    /// Regex patterns that indicate failed execution.
    pub fail_regex: Vec<String>,
    /// Ordered shell commands and result checks.
    #[serde(default)]
    pub shell_check_steps: Vec<ShellCheckStep>,
    /// Timeout in seconds. `None` or `0` disables the timeout.
    pub timeout: Option<u64>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QemuConfigWire {
    args: Vec<String>,
    uefi: bool,
    #[serde(default)]
    to_bin: bool,
    fail_regex: Vec<String>,
    #[serde(default)]
    shell_check_steps: Vec<ShellCheckStep>,
    timeout: Option<u64>,
}

impl<'de> Deserialize<'de> for QemuConfig {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let wire = QemuConfigWire::deserialize(deserializer)?;
        Ok(Self {
            args: wire.args,
            uefi: wire.uefi,
            to_bin: wire.to_bin,
            fail_regex: wire.fail_regex,
            shell_check_steps: wire.shell_check_steps,
            timeout: wire.timeout,
        })
    }
}

impl QemuConfig {
    fn replace_strings(&mut self, scope: &VariableScope) -> anyhow::Result<()> {
        self.args = self
            .args
            .iter()
            .map(|arg| variables::expand_variables(arg, scope))
            .collect::<anyhow::Result<Vec<_>>>()?;
        self.fail_regex = self
            .fail_regex
            .iter()
            .map(|arg| variables::expand_variables(arg, scope))
            .collect::<anyhow::Result<Vec<_>>>()?;
        for step in &mut self.shell_check_steps {
            step.replace_strings(scope)?;
        }
        Ok(())
    }

    fn normalize(&mut self, config_name: &str) -> anyhow::Result<()> {
        normalize_shell_check_steps(&mut self.shell_check_steps, config_name).map(drop)
    }

    fn shell_check_matcher(&self) -> anyhow::Result<Option<ShellCheckMatcher>> {
        let mut steps = self.shell_check_steps.clone();
        if steps.is_empty() {
            return Ok(None);
        }
        let resolved = normalize_shell_check_steps(&mut steps, "QEMU runtime config")?;
        Ok(Some(ShellCheckMatcher::from_steps(resolved)?))
    }

    fn requires_bin_artifact(&self) -> bool {
        self.uefi || self.to_bin
    }
}

/// Pure execution options for running an already prepared artifact in QEMU.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunQemuOptions {
    /// Whether to dump the device tree blob.
    pub dtb_dump: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct QemuRunInput {
    process_context: ProcessContext,
    artifacts: OutputArtifacts,
    arch: Option<Architecture>,
    debug: bool,
}

impl QemuRunInput {
    pub(crate) fn new(
        process_context: ProcessContext,
        artifacts: OutputArtifacts,
        arch: Option<Architecture>,
        debug: bool,
    ) -> Self {
        Self {
            process_context,
            artifacts,
            arch,
            debug,
        }
    }
}

pub(crate) fn default_qemu_config(arch: Option<Architecture>) -> QemuConfig {
    build_default_qemu_config(arch)
}

/// Returns the default QEMU runtime configuration for an invocation.
pub fn default_config(invocation: &Invocation) -> QemuConfig {
    default_qemu_config(invocation.runtime_arch())
}

pub(crate) fn default_qemu_config_for_cargo(
    cargo: &Cargo,
    runtime_arch: Option<Architecture>,
) -> QemuConfig {
    build_default_qemu_config(infer_target_arch(&cargo.target).or(runtime_arch))
}

/// Returns the default QEMU runtime configuration for a Cargo build config.
pub fn default_config_for_cargo(invocation: &Invocation, cargo: &Cargo) -> QemuConfig {
    default_qemu_config_for_cargo(cargo, invocation.runtime_arch())
}

/// Reads a QEMU configuration from an explicit path without creating defaults.
pub async fn read_config_from_path(
    invocation: &Invocation,
    path: &Path,
) -> anyhow::Result<QemuConfig> {
    let scope = invocation.variable_scope()?;
    read_qemu_config_from_path(&scope, path).await
}

/// Reads a QEMU configuration using the Cargo package variable scope.
pub async fn read_config_from_path_for_cargo(
    invocation: &Invocation,
    cargo: &Cargo,
    path: &Path,
) -> anyhow::Result<QemuConfig> {
    let scope = crate::build::cargo_variable_scope(invocation.project_layout(), cargo)?;
    read_qemu_config_from_path(&scope, path).await
}

pub(crate) async fn read_qemu_config_from_path(
    variables: &VariableScope,
    path: &Path,
) -> anyhow::Result<QemuConfig> {
    let config_path = variables::expand_path_variables(path, variables)?;
    read_qemu_config_at_path(variables, config_path).await
}

pub(crate) async fn ensure_qemu_config_for_cargo(
    layout: &ProjectLayout,
    variables: &VariableScope,
    cargo: &Cargo,
    runtime_arch: Option<Architecture>,
) -> anyhow::Result<QemuConfig> {
    let package_dir = metadata::package_manifest_dir(layout, &cargo.package)?;
    let arch = infer_target_arch(&cargo.target).or(runtime_arch);
    let config_path = resolve_qemu_config_path_in_dir(&package_dir, arch, None)?;
    let default_config = default_qemu_config_for_cargo(cargo, runtime_arch);
    ensure_qemu_config_at_path(variables, config_path, default_config).await
}

/// Loads or creates a QEMU configuration using the Cargo package directory.
pub async fn ensure_config_for_cargo(
    invocation: &Invocation,
    cargo: &Cargo,
) -> anyhow::Result<QemuConfig> {
    let scope = crate::build::cargo_variable_scope(invocation.project_layout(), cargo)?;
    ensure_qemu_config_for_cargo(
        invocation.project_layout(),
        &scope,
        cargo,
        invocation.runtime_arch(),
    )
    .await
}

pub(crate) async fn ensure_qemu_config_in_dir(
    variables: &VariableScope,
    dir: &Path,
    runtime_arch: Option<Architecture>,
) -> anyhow::Result<QemuConfig> {
    let dir = variables::expand_path_variables(dir, variables)?;
    let config_path = resolve_qemu_config_path_in_dir(&dir, runtime_arch, None)?;
    let default_config = default_qemu_config(runtime_arch);
    ensure_qemu_config_at_path(variables, config_path, default_config).await
}

/// Loads a QEMU configuration from a directory using the default filename search.
pub async fn ensure_config_in_dir(
    invocation: &Invocation,
    dir: &Path,
) -> anyhow::Result<QemuConfig> {
    let scope = invocation.variable_scope()?;
    ensure_qemu_config_in_dir(&scope, dir, invocation.runtime_arch()).await
}

pub(crate) fn prepare_qemu_runtime_config(
    variables: &VariableScope,
    config: &QemuConfig,
) -> anyhow::Result<QemuConfig> {
    let mut config = config.clone();
    config.replace_strings(variables)?;
    config.normalize("QEMU runtime config")?;
    Ok(config)
}

pub(crate) async fn run_qemu_with_config(
    input: QemuRunInput,
    run_args: RunQemuOptions,
    config: QemuConfig,
) -> anyhow::Result<()> {
    if config.requires_bin_artifact() {
        input
            .artifacts
            .require_bin("QEMU runtime requires a prepared BIN artifact")?;
    }

    let mut runner = QemuRunner {
        input,
        config,
        dtbdump: run_args.dtb_dump,
        fail_regex: vec![],
    };
    runner.run().await
}

/// Runs an already prepared artifact in QEMU using a materialized configuration.
pub async fn run_qemu(
    invocation: &mut Invocation,
    config: &QemuConfig,
    options: RunQemuOptions,
) -> anyhow::Result<()> {
    run_qemu_with_debug(invocation, config, options, invocation.options().debug()).await
}

pub(crate) async fn run_qemu_with_debug(
    invocation: &mut Invocation,
    config: &QemuConfig,
    options: RunQemuOptions,
    debug: bool,
) -> anyhow::Result<()> {
    let scope = invocation.variable_scope()?;
    let config = prepare_qemu_runtime_config(&scope, config)?;
    if config.requires_bin_artifact() {
        invocation.ensure_runtime_bin()?;
    }
    let input = QemuRunInput::new(
        invocation.process_context()?,
        invocation.runtime_artifacts().clone(),
        invocation.runtime_arch(),
        debug,
    );
    run_qemu_with_config(input, options, config).await
}

pub(crate) async fn read_qemu_config_at_path(
    variables: &VariableScope,
    config_path: PathBuf,
) -> anyhow::Result<QemuConfig> {
    info!("Using QEMU config file: {}", config_path.display());

    let content = fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("failed to read QEMU config: {}", config_path.display()))?;
    let mut config: QemuConfig = toml::from_str(&content)
        .with_context(|| format!("failed to parse QEMU config: {}", config_path.display()))?;
    config.replace_strings(variables)?;
    config.normalize(&format!("QEMU config {}", config_path.display()))?;
    Ok(config)
}

pub(crate) async fn ensure_qemu_config_at_path(
    variables: &VariableScope,
    config_path: PathBuf,
    default_config: QemuConfig,
) -> anyhow::Result<QemuConfig> {
    info!("Using QEMU config file: {}", config_path.display());

    let config_content = match fs::read_to_string(&config_path).await {
        Ok(_) => return read_qemu_config_at_path(variables, config_path).await,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let mut config = default_config;
            config.normalize(&format!("QEMU config {}", config_path.display()))?;
            fs::write(&config_path, toml::to_string_pretty(&config)?)
                .await
                .with_path("failed to write file", &config_path)?;
            config
        }
        Err(e) => return Err(e.into()),
    };
    Ok(config_content)
}

fn build_default_qemu_config(arch: Option<Architecture>) -> QemuConfig {
    let mut config = QemuConfig {
        to_bin: true,
        ..Default::default()
    };
    config.args.push("-nographic".to_string());
    if let Some(arch) = arch {
        match arch {
            Architecture::Aarch64 => {
                config.args.push("-cpu".to_string());
                config.args.push("cortex-a53".to_string());
            }
            Architecture::Riscv64 => {
                config.args.push("-cpu".to_string());
                config.args.push("rv64".to_string());
            }
            _ => {}
        }
    }
    config
}

pub(crate) fn infer_target_arch(target: &str) -> Option<Architecture> {
    let target = target.trim();
    if target.is_empty() {
        return None;
    }

    let triple = target.split('-').next().unwrap_or(target);
    match triple {
        "aarch64" => Some(Architecture::Aarch64),
        "arm" | "armv7" | "armv7a" | "armv7r" | "thumbv7em" => Some(Architecture::Arm),
        "riscv64" | "riscv64gc" => Some(Architecture::Riscv64),
        "x86_64" => Some(Architecture::X86_64),
        "i386" | "i586" | "i686" => Some(Architecture::I386),
        "loongarch64" => Some(Architecture::LoongArch64),
        _ => None,
    }
}

struct QemuRunner {
    input: QemuRunInput,
    config: QemuConfig,
    dtbdump: bool,
    fail_regex: Vec<regex::Regex>,
}

impl QemuRunner {
    async fn run(&mut self) -> anyhow::Result<()> {
        self.prepare_regex()?;

        let detected_arch = self.input.arch.ok_or_else(|| {
            anyhow!("Please specify `arch` in QEMU config or provide a valid ELF file.")
        })?;
        let arch = format!("{detected_arch:?}").to_lowercase();

        let machine = match detected_arch {
            Architecture::X86_64 | Architecture::I386 => "q35",
            _ => "virt",
        }
        .to_string();

        #[allow(unused_mut)]
        let mut qemu_executable = format!("qemu-system-{arch}");

        #[cfg(windows)]
        {
            println!("{}", "Checking for QEMU executable on Windows...".blue());
            // Windows 特殊处理
            let msys2 =
                PathBuf::from("C:\\msys64\\ucrt64\\bin").join(format!("{qemu_executable}.exe"));

            if msys2.exists() {
                println!("Using QEMU executable from MSYS2: {}", msys2.display());
                qemu_executable = msys2.to_string_lossy().to_string();
            }
        }

        let dtb_dump_path = if self.dtbdump {
            Some(
                prepare_qemu_dtb_dump(default_qemu_dtb_dump_path())
                    .await?
                    .path()
                    .to_path_buf(),
            )
        } else {
            None
        };

        let boot_source = if let Some(uefi) = self.prepare_uefi().await? {
            match uefi {
                UefiBootConfig::Pflash {
                    code,
                    vars,
                    esp_dir,
                } => Some(QemuBootSource::uefi_pflash(code, vars, esp_dir)),
            }
        } else {
            self.input
                .artifacts
                .runtime_image()
                .map(|path| QemuBootSource::direct_kernel_loader(path.to_path_buf()))
        };

        let plan = build_qemu_command_plan(QemuCommandPlanInput {
            executable: qemu_executable,
            config_args: self.config.args.iter().map(OsString::from).collect(),
            default_machine: machine,
            dtb_dump_path,
            debug: self.input.debug,
            boot_source,
        });
        let mut cmd = plan.render(&self.input.process_context);
        cmd.stdin(Stdio::piped());
        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::piped());
        cmd.print_cmd();
        let mut child = TokioCommand::from(cmd.into_std()).spawn()?;
        let started_at = Instant::now();
        let stdin = child.stdin.take().context("failed to capture QEMU stdin")?;
        let stdout = child
            .stdout
            .take()
            .context("failed to capture QEMU stdout")?;
        let stderr = child
            .stderr
            .take()
            .context("failed to capture QEMU stderr")?;

        let (inbound_tx, inbound_rx) = mpsc::unbounded_channel::<Vec<u8>>();
        let (outbound_tx, mut outbound_rx) =
            mpsc::unbounded_channel::<crate::sterm::TerminalInput>();
        let stderr_capture = Arc::new(Mutex::new(Vec::<u8>::new()));

        let stdout_task = tokio::spawn(read_child_stream(stdout, inbound_tx.clone(), None));
        let stderr_task = tokio::spawn(read_child_stream(
            stderr,
            inbound_tx,
            Some(stderr_capture.clone()),
        ));
        let write_task = tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(input) = outbound_rx.recv().await {
                write_qemu_input(&mut stdin, input).await?;
            }
            Ok::<(), anyhow::Error>(())
        });

        let matcher = Arc::new(Mutex::new(FailStreamMatcher::new(self.fail_regex.clone())));
        let shell_check_driver = self
            .config
            .shell_check_matcher()?
            .map(ShellCheckDriver::new);
        let match_result = Arc::new(Mutex::new(None));
        let terminal = AsyncTerminal::new(TerminalConfig {
            intercept_exit_sequence: false,
            timeout: timeout_duration(self.config.timeout),
            timeout_label: "QEMU".to_string(),
        });

        let terminal_result = terminal
            .run_with_write_ack(inbound_rx, outbound_tx, {
                let matcher = matcher.clone();
                let shell_check_driver = shell_check_driver.clone();
                let match_result = match_result.clone();
                move |handle, chunk| {
                    let mut matcher = matcher.lock().unwrap();
                    for byte in chunk {
                        if let Some(matched) = matcher.observe_byte(*byte) {
                            print_fail_match(&matched);
                            let mut result = match_result.lock().unwrap();
                            *result = Some(matched);
                            handle.stop_after(crate::run::output_matcher::MATCH_DRAIN_DURATION);
                        }
                    }

                    if let Some(shell_check_driver) = shell_check_driver.as_ref() {
                        shell_check_driver.observe_chunk(handle, chunk);
                    }

                    if matcher.should_stop() {
                        handle.stop();
                    }
                }
            })
            .await;

        let writer_error = shutdown_qemu_writer_task(write_task).await.err();

        let shell_check_completed = shell_check_driver
            .as_ref()
            .is_some_and(ShellCheckDriver::completed);
        let shell_check_failure = shell_check_driver
            .as_ref()
            .and_then(ShellCheckDriver::completion_error);
        let should_kill = matcher.lock().unwrap().should_stop()
            || shell_check_completed
            || shell_check_failure.is_some()
            || writer_error.is_some()
            || terminal_result.is_err();
        if should_kill
            && child
                .try_wait()
                .context("failed to query QEMU process status")?
                .is_none()
            && let Err(err) = child.kill().await
            && err.kind() != ErrorKind::InvalidInput
        {
            return Err(err.into());
        }

        let status = child.wait().await?;
        let _ = stdout_task.await;
        let _ = stderr_task.await;

        let stderr = stderr_capture.lock().unwrap().clone();
        RunnerExecutionSummary::new(
            "QEMU",
            RunnerExitStatus::process(status),
            started_at.elapsed(),
        )
        .with_terminal_error(terminal_result.err().or(writer_error))
        .with_shell_check_error(shell_check_failure)
        .with_shell_check_completed(shell_check_completed)
        .with_fail_match(match_result.lock().unwrap().take())
        .with_stderr_log(&stderr)
        .into_result()
    }

    async fn prepare_uefi(&self) -> anyhow::Result<Option<UefiBootConfig>> {
        if !self.config.uefi {
            return Ok(None);
        }

        let arch = self
            .input
            .arch
            .ok_or_else(|| anyhow::anyhow!("Cannot determine architecture for OVMF preparation"))?;
        let bios_dir = default_cache_dir();
        fs::create_dir_all(&bios_dir)
            .await
            .with_path("failed to create directory", &bios_dir)?;

        println!("Preparing OVMF firmware for architecture: {arch:?}");
        let prebuilt = Prebuilt::fetch(Source::LATEST, &bios_dir)
            .await
            .with_context(|| format!("failed to prepare OVMF cache: {}", bios_dir.display()))?;
        let arch = match arch {
            Architecture::X86_64 => Arch::X64,
            Architecture::Aarch64 => Arch::Aarch64,
            Architecture::Riscv64 => Arch::Riscv64,
            Architecture::LoongArch64 => Arch::LoongArch64,
            Architecture::I386 => Arch::Ia32,
            o => return Err(anyhow::anyhow!("OVMF is not supported for {o:?} ",)),
        };

        let code = prebuilt.get_file(arch, FileType::Code);
        let vars_template = prebuilt.get_file(arch, FileType::Vars);
        let esp_dir = self.prepare_uefi_esp(arch).await?;
        let vars = self.prepare_uefi_vars(&vars_template).await?;

        Ok(Some(UefiBootConfig::Pflash {
            code,
            vars,
            esp_dir,
        }))
    }

    async fn prepare_uefi_esp(&self, arch: Arch) -> anyhow::Result<PathBuf> {
        let bin_path = self
            .input
            .artifacts
            .require_bin("UEFI boot requires a BIN artifact")?
            .to_path_buf();
        let stem = bin_path
            .file_stem()
            .ok_or_else(|| anyhow!("invalid BIN path: {}", bin_path.display()))?;
        let artifact_dir = self.uefi_artifact_dir(&bin_path)?;
        let esp_dir = artifact_dir.join(format!("{}.esp", stem.to_string_lossy()));
        let boot_dir = esp_dir.join("EFI").join("BOOT");
        fs::create_dir_all(&boot_dir)
            .await
            .with_path("failed to create directory", &boot_dir)?;

        let boot_path = boot_dir.join(Self::default_uefi_boot_filename(arch));
        fs::copy(&bin_path, &boot_path).await.with_context(|| {
            format!(
                "failed to copy EFI image from {} to {}",
                bin_path.display(),
                boot_path.display()
            )
        })?;

        Ok(esp_dir)
    }

    fn uefi_artifact_dir(&self, bin_path: &Path) -> anyhow::Result<PathBuf> {
        if let Some(dir) = self.input.artifacts.runtime_artifact_dir() {
            return Ok(dir.to_path_buf());
        }

        let bin_path = bin_path
            .canonicalize()
            .with_path("failed to canonicalize file", bin_path)?;
        bin_path
            .parent()
            .map(PathBuf::from)
            .ok_or_else(|| anyhow!("invalid BIN path: {}", bin_path.display()))
    }

    async fn prepare_uefi_vars(&self, vars_template: &Path) -> anyhow::Result<PathBuf> {
        let bin_path = self
            .input
            .artifacts
            .require_bin("UEFI boot requires a BIN artifact")?
            .to_path_buf();
        let stem = bin_path
            .file_stem()
            .ok_or_else(|| anyhow!("invalid BIN path: {}", bin_path.display()))?;
        let artifact_dir = self.uefi_artifact_dir(&bin_path)?;
        fs::create_dir_all(&artifact_dir)
            .await
            .with_path("failed to create directory", &artifact_dir)?;

        let vars = artifact_dir.join(format!("{}.vars.fd", stem.to_string_lossy()));
        fs::copy(vars_template, &vars).await.with_context(|| {
            format!(
                "failed to copy OVMF vars from {} to {}",
                vars_template.display(),
                vars.display()
            )
        })?;

        Ok(vars)
    }

    fn default_uefi_boot_filename(arch: Arch) -> &'static str {
        match arch {
            Arch::Aarch64 => "BOOTAA64.EFI",
            Arch::Ia32 => "BOOTIA32.EFI",
            Arch::LoongArch64 => "BOOTLOONGARCH64.EFI",
            Arch::Riscv64 => "BOOTRISCV64.EFI",
            Arch::X64 => "BOOTX64.EFI",
        }
    }

    fn prepare_regex(&mut self) -> anyhow::Result<()> {
        self.fail_regex = compile_fail_regexes(&self.config.fail_regex)?;
        Ok(())
    }
}

async fn write_qemu_input<W>(
    writer: &mut W,
    input: crate::sterm::TerminalInput,
) -> anyhow::Result<()>
where
    W: tokio::io::AsyncWrite + Unpin,
{
    if let Err(error) = writer.write_all(input.bytes()).await {
        input.acknowledge_failed(format!("failed to write QEMU stdin: {error}"));
        return Err(error).context("failed to write QEMU stdin");
    }
    if let Err(error) = writer.flush().await {
        input.acknowledge_failed(format!("failed to flush QEMU stdin: {error}"));
        return Err(error).context("failed to flush QEMU stdin");
    }
    input.acknowledge_flushed();
    Ok(())
}

async fn shutdown_qemu_writer_task(
    mut task: tokio::task::JoinHandle<anyhow::Result<()>>,
) -> anyhow::Result<()> {
    match tokio::time::timeout(Duration::from_secs(1), &mut task).await {
        Ok(Ok(result)) => result,
        Ok(Err(error)) => Err(anyhow!("QEMU stdin writer task join error: {error}")),
        Err(_) => {
            task.abort();
            let _ = task.await;
            Err(anyhow!("QEMU stdin writer task did not stop within 1s"))
        }
    }
}

/// Resolve QEMU configuration file path with architecture-specific priority.
///
/// Configuration search priority:
/// 1. Explicit path (if provided)
/// 2. workspace_dir: qemu-<arch>.toml → .qemu-<arch>.toml → qemu.toml → .qemu.toml
///
/// When architecture is detected, architecture-specific files are checked first.
pub(crate) fn resolve_qemu_config_path_in_dir(
    search_dir: &Path,
    arch: Option<Architecture>,
    explicit_path: Option<PathBuf>,
) -> anyhow::Result<PathBuf> {
    if let Some(path) = explicit_path {
        return Ok(path);
    }

    let arch_str = arch.map(|arch| format!("{arch:?}").to_lowercase());

    // 文件名优先级顺序
    let candidates: Vec<String> = if let Some(ref arch) = arch_str {
        vec![
            format!("qemu-{}.toml", arch),
            format!(".qemu-{}.toml", arch),
            "qemu.toml".to_string(),
            ".qemu.toml".to_string(),
        ]
    } else {
        vec!["qemu.toml".to_string(), ".qemu.toml".to_string()]
    };

    for filename in &candidates {
        let path = search_dir.join(filename);
        if path.exists() {
            return Ok(path);
        }
    }

    let default_filename = if let Some(ref arch) = arch_str {
        format!(".qemu-{arch}.toml")
    } else {
        ".qemu.toml".to_string()
    };

    Ok(search_dir.join(default_filename))
}

async fn read_child_stream<R>(
    mut reader: R,
    tx: mpsc::UnboundedSender<Vec<u8>>,
    capture: Option<Arc<Mutex<Vec<u8>>>>,
) -> anyhow::Result<()>
where
    R: AsyncRead + Unpin,
{
    let mut buffer = [0u8; 1024];
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        if let Some(capture) = capture.as_ref() {
            capture.lock().unwrap().extend_from_slice(&buffer[..read]);
        }
        if tx.send(buffer[..read].to_vec()).is_err() {
            break;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        QemuConfig, QemuRunInput, QemuRunner, RunQemuOptions, build_default_qemu_config,
        default_qemu_config_for_cargo, ensure_config_for_cargo, ensure_qemu_config_at_path,
        infer_target_arch, read_config_from_path, read_qemu_config_at_path,
        resolve_qemu_config_path_in_dir, run_qemu_with_config, shutdown_qemu_writer_task,
        timeout_duration, write_qemu_input,
    };
    use object::Architecture;
    use std::{
        path::{Path, PathBuf},
        pin::Pin,
        sync::{
            Arc,
            atomic::{AtomicBool, Ordering},
        },
        task::{Context, Poll},
        time::Duration,
    };
    use tempfile::TempDir;
    use tokio::io::AsyncWrite;

    use crate::{
        artifact::{
            runtime::{RuntimeArtifactOptions, prepare_runtime_artifacts},
            state::OutputArtifacts,
        },
        build::{
            config::{BuildConfig, BuildSystem, Cargo},
            config_loader,
        },
        invocation::{Invocation, InvocationOptions},
        run::{
            output_matcher::FailStreamMatcher,
            shell_check::{ShellCheckMatcher, ShellCheckStep, normalize_shell_check_steps},
        },
    };
    use std::collections::HashMap;

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
    async fn qemu_writer_acknowledges_only_after_flush() {
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

        write_qemu_input(&mut writer, input).await.unwrap();

        assert_eq!(writer.flushes, 1);
        assert!(callback_ran.load(Ordering::Acquire));
    }

    #[tokio::test]
    async fn qemu_writer_reports_write_failure() {
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

        let error = write_qemu_input(&mut writer, input).await.unwrap_err();

        assert!(error.to_string().contains("failed to write QEMU stdin"));
        assert_eq!(writer.flushes, 0);
        assert_eq!(
            error_seen.lock().unwrap().as_deref(),
            Some("failed to write QEMU stdin: injected write failure")
        );
    }

    #[tokio::test]
    async fn qemu_writer_reports_flush_failure() {
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

        let error = write_qemu_input(&mut writer, input).await.unwrap_err();

        assert!(error.to_string().contains("failed to flush QEMU stdin"));
        assert_eq!(writer.flushes, 0);
        assert_eq!(
            error_seen.lock().unwrap().as_deref(),
            Some("failed to flush QEMU stdin: injected flush failure")
        );
    }

    #[tokio::test]
    async fn qemu_writer_reports_first_chunk_failure_once_for_chunked_operation() {
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
        let error = write_qemu_input(&mut failing_writer, first)
            .await
            .unwrap_err();
        assert!(error.to_string().contains("failed to write QEMU stdin"));

        for _ in 0..2 {
            let input = rx.recv().await.unwrap();
            assert_eq!(input.bytes().len(), 64);
            let mut writer = FlushCheckingWriter {
                callback_ran: Arc::new(AtomicBool::new(false)),
                failure: WriterFailure::None,
                flushes: 0,
            };
            write_qemu_input(&mut writer, input).await.unwrap();
        }
        tokio::task::yield_now().await;

        assert_eq!(
            completions.lock().unwrap().as_slice(),
            &[Some(
                "failed to write QEMU stdin: injected write failure".to_string()
            )]
        );
    }

    #[tokio::test]
    async fn qemu_writer_task_error_is_preserved() {
        let task = tokio::spawn(async { Err(anyhow::anyhow!("injected writer failure")) });

        let error = shutdown_qemu_writer_task(task).await.unwrap_err();

        assert_eq!(error.to_string(), "injected writer failure");
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

    fn make_invocation(dir: &std::path::Path) -> Invocation {
        Invocation::new(InvocationOptions::new(
            Some(dir.to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap()
    }

    fn qemu_input(invocation: &Invocation) -> QemuRunInput {
        QemuRunInput::new(
            invocation.process_context().unwrap(),
            invocation.runtime_artifacts().clone(),
            invocation.runtime_arch(),
            invocation.options().debug(),
        )
    }

    #[test]
    fn default_qemu_config_keeps_existing_defaults_without_overrides() {
        let config = build_default_qemu_config(Some(Architecture::Aarch64));

        assert!(config.to_bin);
        assert_eq!(config.args, vec!["-nographic", "-cpu", "cortex-a53"]);
        assert!(config.fail_regex.is_empty());
        assert_eq!(config.timeout, None);
    }

    #[test]
    fn default_qemu_config_for_other_arch_only_adds_generic_defaults() {
        let config = build_default_qemu_config(Some(Architecture::X86_64));

        assert!(config.to_bin);
        assert_eq!(config.args, vec!["-nographic"]);
        assert_eq!(config.timeout, None);
    }

    #[test]
    fn infer_target_arch_maps_known_target_triples() {
        assert_eq!(
            infer_target_arch("aarch64-unknown-none"),
            Some(Architecture::Aarch64)
        );
        assert_eq!(
            infer_target_arch("riscv64gc-unknown-none-elf"),
            Some(Architecture::Riscv64)
        );
        assert_eq!(
            infer_target_arch("x86_64-unknown-none"),
            Some(Architecture::X86_64)
        );
        assert_eq!(infer_target_arch(""), None);
    }

    #[tokio::test]
    async fn load_existing_qemu_config_preserves_file_contents() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        let config_path = tmp.path().join(".qemu.toml");
        std::fs::write(
            &config_path,
            r#"
args = ["-nographic", "-machine", "virt"]
uefi = false
to_bin = false
fail_regex = ["FAIL"]
shell_check_steps = [
  { shell_prefix = "login:", shell_cmd = "root", success_regex = ["PASS"] },
]
"#,
        )
        .unwrap();

        let invocation = make_invocation(tmp.path());
        let scope = invocation.variable_scope().unwrap();
        let config = read_qemu_config_at_path(&scope, config_path).await.unwrap();

        assert!(!config.to_bin);
        assert_eq!(config.fail_regex, vec!["FAIL"]);
        assert_eq!(
            config.shell_check_steps[0].shell_prefix.as_deref(),
            Some("login:")
        );
        assert_eq!(
            config.shell_check_steps[0].shell_cmd.as_deref(),
            Some("root")
        );
        assert_eq!(
            config.shell_check_steps[0].success_regex.as_deref(),
            Some(&["PASS".to_string()][..])
        );
        assert_eq!(config.args, vec!["-nographic", "-machine", "virt"]);
    }

    #[test]
    fn qemu_config_rejects_legacy_shell_check_fields() {
        toml::from_str::<QemuConfig>(
            r#"
args = ["-nographic"]
uefi = false
fail_regex = []
shell_prefix = "root@starry:"
shell_init_cmd = "echo pass"
success_regex = ["(?m)^pass\\s*$"]
"#,
        )
        .unwrap_err();
    }

    #[test]
    fn qemu_config_rejects_legacy_fields_mixed_with_shell_check_steps() {
        let error = toml::from_str::<QemuConfig>(
            r#"
args = ["-nographic"]
uefi = false
fail_regex = []
shell_prefix = "root@starry:"
shell_check_steps = [
  { shell_prefix = "root@starry:", shell_cmd = "echo pass" },
]
"#,
        )
        .unwrap_err();

        assert!(error.to_string().contains("shell_prefix"));
    }

    #[tokio::test]
    async fn load_missing_qemu_config_uses_default_template() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        let config_path = tmp.path().join(".qemu.toml");

        let invocation = make_invocation(tmp.path());

        let config = ensure_qemu_config_at_path(
            &invocation.variable_scope().unwrap(),
            config_path.clone(),
            build_default_qemu_config(Some(Architecture::Aarch64)),
        )
        .await
        .unwrap();

        assert!(config.to_bin);
        assert_eq!(config.args, vec!["-nographic", "-cpu", "cortex-a53"]);
        assert!(config_path.exists());
    }

    #[tokio::test]
    async fn load_qemu_config_for_cargo_prefers_package_dir() {
        let tmp = TempDir::new().unwrap();
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
            kernel_dir.join(".qemu-aarch64.toml"),
            r#"
args = ["-custom"]
uefi = false
to_bin = true
fail_regex = []
"#,
        )
        .unwrap();

        let invocation =
            Invocation::new(InvocationOptions::new(Some(app_dir), None, None, false)).unwrap();

        let config = ensure_config_for_cargo(
            &invocation,
            &Cargo {
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
            },
        )
        .await
        .unwrap();

        assert_eq!(config.args, vec!["-custom"]);
    }

    #[tokio::test]
    async fn run_qemu_with_config_rejects_missing_required_bin_artifact() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        let source = std::env::current_exe().unwrap();
        let copied = tmp.path().join("sample-elf");
        std::fs::copy(&source, &copied).unwrap();

        let mut invocation = make_invocation(tmp.path());
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
        let input = qemu_input(&invocation);

        assert!(input.artifacts.elf().is_some());
        assert!(input.artifacts.bin().is_none());

        let err = run_qemu_with_config(
            input,
            RunQemuOptions::default(),
            QemuConfig {
                to_bin: true,
                ..Default::default()
            },
        )
        .await
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("QEMU runtime requires a prepared BIN artifact")
        );
    }

    #[test]
    fn qemu_config_marks_bin_required_for_uefi_and_legacy_to_bin() {
        assert!(
            QemuConfig {
                uefi: true,
                to_bin: false,
                ..Default::default()
            }
            .requires_bin_artifact()
        );
        assert!(
            QemuConfig {
                uefi: false,
                to_bin: true,
                ..Default::default()
            }
            .requires_bin_artifact()
        );
        assert!(
            !QemuConfig {
                uefi: false,
                to_bin: false,
                ..Default::default()
            }
            .requires_bin_artifact()
        );
    }

    #[test]
    fn default_qemu_config_for_cargo_uses_target_arch() {
        let config = default_qemu_config_for_cargo(
            &Cargo {
                env: HashMap::new(),
                target: "riscv64gc-unknown-none-elf".into(),
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
            },
            None,
        );

        assert_eq!(config.args, vec!["-nographic", "-cpu", "rv64"]);
    }

    #[test]
    fn qemu_timeout_zero_disables_timeout() {
        assert_eq!(timeout_duration(None), None);
        assert_eq!(timeout_duration(Some(0)), None);
        assert_eq!(timeout_duration(Some(3)), Some(Duration::from_secs(3)));
    }

    #[test]
    fn qemu_config_parses_timeout_from_toml() {
        let config: QemuConfig = toml::from_str(
            r#"
args = ["-nographic"]
uefi = false
to_bin = true
fail_regex = []
timeout = 0
"#,
        )
        .unwrap();

        assert_eq!(config.timeout, Some(0));
    }

    #[test]
    fn qemu_config_defaults_to_bin_to_false_when_field_is_absent() {
        let config: QemuConfig = toml::from_str(
            r#"
args = ["-nographic"]
uefi = false
fail_regex = []
"#,
        )
        .unwrap();

        assert!(!config.to_bin);
    }

    #[test]
    fn qemu_config_normalize_rejects_shell_check_without_prefix() {
        let mut config = QemuConfig {
            shell_check_steps: vec![ShellCheckStep {
                shell_cmd: Some("root".into()),
                ..Default::default()
            }],
            ..Default::default()
        };

        let err = config.normalize("test config").unwrap_err();
        assert!(err.to_string().contains("shell_prefix"));
    }

    #[test]
    fn qemu_config_parses_ordered_shell_check_steps() {
        let mut config: QemuConfig = toml::from_str(
            r#"
args = []
uefi = false
fail_regex = []
shell_check_steps = [
  { shell_prefix = "axvisor:/$", shell_cmd = "vm console 1" },
  { shell_prefix = "root@starry:/root #", shell_cmd = "echo pass", success_regex = ["(?m)^pass\\s*$"], fail_regex = ["(?i)fail"] },
]
"#,
        )
        .unwrap();

        config.normalize("test config").unwrap();

        assert_eq!(config.shell_check_steps.len(), 2);
        assert_eq!(
            config.shell_check_steps[0].shell_cmd.as_deref(),
            Some("vm console 1")
        );
        assert_eq!(
            config.shell_check_steps[1].success_regex.as_deref(),
            Some(&["(?m)^pass\\s*$".to_string()][..])
        );
    }

    #[test]
    fn qemu_config_normalize_trims_prefix_and_preserves_command() {
        let mut config = QemuConfig {
            shell_check_steps: vec![ShellCheckStep {
                shell_prefix: Some(" login: ".into()),
                shell_cmd: Some(" root ".into()),
                ..Default::default()
            }],
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
    fn qemu_shell_check_matcher_can_coexist_with_global_fail_matcher() {
        let mut matcher =
            FailStreamMatcher::new(vec![regex::Regex::new("__never_fail__").unwrap()]);
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("login:".into()),
            shell_cmd: Some("root".into()),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut shell_check = ShellCheckMatcher::from_steps(resolved).unwrap();
        let mut sent = None;

        for byte in b"login: system ready\n" {
            if sent.is_none() {
                sent = shell_check.observe_byte(*byte);
            } else {
                let _ = shell_check.observe_byte(*byte);
            }
            let _ = matcher.observe_byte(*byte);
        }

        assert!(matcher.matched().is_none());
        assert_eq!(sent.as_deref(), Some(&b"root\n"[..]));
    }

    #[test]
    fn uefi_artifact_dir_prefers_runtime_artifact_dir() {
        let runtime_dir = PathBuf::from("/tmp/ostool-runtime");
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        let invocation = make_invocation(tmp.path());
        let mut artifacts = OutputArtifacts::default();
        artifacts.set_runtime_artifact_dir(runtime_dir.clone());
        let input = QemuRunInput::new(
            invocation.process_context().unwrap(),
            artifacts,
            invocation.runtime_arch(),
            invocation.options().debug(),
        );

        let runner = QemuRunner {
            input,
            config: QemuConfig::default(),
            dtbdump: false,
            fail_regex: vec![],
        };

        let resolved = runner
            .uefi_artifact_dir(PathBuf::from("/tmp/ignored/kernel.bin").as_path())
            .unwrap();
        assert_eq!(resolved, runtime_dir);
    }

    // === QEMU 配置路径解析测试 ===

    #[test]
    fn qemu_config_explicit_path_wins() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        let invocation = make_invocation(tmp.path());

        let explicit = tmp.path().join("custom.qemu.toml");
        let result = resolve_qemu_config_path_in_dir(
            invocation.workspace_dir(),
            invocation.runtime_arch(),
            Some(explicit.clone()),
        )
        .unwrap();
        assert_eq!(result, explicit);
    }

    #[test]
    fn qemu_config_workspace_path_used() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        std::fs::write(tmp.path().join("qemu-aarch64.toml"), "").unwrap();

        let invocation = make_invocation(tmp.path());

        let result = resolve_qemu_config_path_in_dir(
            invocation.workspace_dir(),
            Some(Architecture::Aarch64),
            None,
        )
        .unwrap();
        assert_eq!(result, tmp.path().join("qemu-aarch64.toml"));
    }

    #[test]
    fn qemu_config_filename_priority() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        let manifest = tmp.path().to_path_buf();
        let invocation = make_invocation(tmp.path());

        std::fs::write(manifest.join("qemu.toml"), "").unwrap();
        let result = resolve_qemu_config_path_in_dir(
            invocation.workspace_dir(),
            Some(Architecture::Aarch64),
            None,
        )
        .unwrap();
        assert_eq!(result, manifest.join("qemu.toml"));

        std::fs::write(manifest.join("qemu-aarch64.toml"), "").unwrap();
        let result = resolve_qemu_config_path_in_dir(
            invocation.workspace_dir(),
            Some(Architecture::Aarch64),
            None,
        )
        .unwrap();
        assert_eq!(result, manifest.join("qemu-aarch64.toml"));
    }

    #[test]
    fn qemu_config_replaces_string_fields() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
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
            std::env::set_var("OSTOOL_QEMU_TEST_ENV", "env-ok");
        }

        let mut config = QemuConfig {
            args: vec!["${workspace}".into(), "${package}".into()],
            fail_regex: vec!["${workspaceFolder}".into()],
            shell_check_steps: vec![ShellCheckStep {
                shell_prefix: Some("${workspace}".into()),
                shell_cmd: Some("${package}".into()),
                success_regex: Some(vec!["${env:OSTOOL_QEMU_TEST_ENV}".into()]),
                ..Default::default()
            }],
            ..Default::default()
        };

        config
            .replace_strings(&invocation.variable_scope().unwrap())
            .unwrap();

        let expected = tmp.path().display().to_string();
        assert_eq!(config.args, vec![expected.clone(), expected.clone()]);
        assert_eq!(config.fail_regex, vec![expected.clone()]);
        assert_eq!(
            config.shell_check_steps[0].success_regex.as_deref(),
            Some(&["env-ok".to_string()][..])
        );
        assert_eq!(
            config.shell_check_steps[0].shell_prefix.as_deref(),
            Some(expected.as_str())
        );
        assert_eq!(
            config.shell_check_steps[0].shell_cmd.as_deref(),
            Some(expected.as_str())
        );
    }

    #[tokio::test]
    async fn read_qemu_config_from_variable_path_expands_workspace() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        std::fs::write(
            tmp.path().join("qemu.toml"),
            r#"
args = ["-nographic"]
uefi = false
to_bin = false
fail_regex = []
"#,
        )
        .unwrap();
        let invocation = make_invocation(tmp.path());

        let config = read_config_from_path(&invocation, Path::new("${workspace}/qemu.toml"))
            .await
            .unwrap();

        assert_eq!(config.args, vec!["-nographic"]);
    }

    #[test]
    fn qemu_config_default_path_with_search_dir() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        let invocation = make_invocation(tmp.path());

        let result = resolve_qemu_config_path_in_dir(
            invocation.workspace_dir(),
            invocation.runtime_arch(),
            None,
        )
        .unwrap();
        assert_eq!(result, tmp.path().join(".qemu.toml"));
    }

    #[test]
    fn qemu_config_default_path_with_arch() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        let invocation = make_invocation(tmp.path());

        let result = resolve_qemu_config_path_in_dir(
            invocation.workspace_dir(),
            Some(Architecture::Aarch64),
            None,
        )
        .unwrap();
        assert_eq!(result, tmp.path().join(".qemu-aarch64.toml"));
    }

    #[test]
    fn qemu_config_without_arch() {
        let tmp = TempDir::new().unwrap();
        write_single_crate_manifest(tmp.path());
        std::fs::write(tmp.path().join("qemu-aarch64.toml"), "").unwrap();
        std::fs::write(tmp.path().join("qemu.toml"), "").unwrap();

        let invocation = make_invocation(tmp.path());
        let result = resolve_qemu_config_path_in_dir(
            invocation.workspace_dir(),
            invocation.runtime_arch(),
            None,
        )
        .unwrap();
        assert_eq!(result, tmp.path().join("qemu.toml"));
    }

    #[test]
    fn qemu_config_search_dir_prefers_arch_specific_files() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join("qemu-aarch64.toml"), "").unwrap();
        std::fs::write(tmp.path().join("qemu.toml"), "").unwrap();

        let result =
            resolve_qemu_config_path_in_dir(tmp.path(), Some(Architecture::Aarch64), None).unwrap();
        assert_eq!(result, tmp.path().join("qemu-aarch64.toml"));
    }

    #[test]
    fn qemu_config_search_dir_uses_hidden_generic_before_hidden_default_creation() {
        let tmp = TempDir::new().unwrap();
        std::fs::write(tmp.path().join(".qemu.toml"), "").unwrap();

        let result =
            resolve_qemu_config_path_in_dir(tmp.path(), Some(Architecture::Aarch64), None).unwrap();
        assert_eq!(result, tmp.path().join(".qemu.toml"));
    }

    #[test]
    fn qemu_config_search_dir_defaults_to_arch_specific_hidden_file() {
        let tmp = TempDir::new().unwrap();

        let result =
            resolve_qemu_config_path_in_dir(tmp.path(), Some(Architecture::Aarch64), None).unwrap();
        assert_eq!(result, tmp.path().join(".qemu-aarch64.toml"));
    }

    #[test]
    fn qemu_config_search_dir_defaults_without_arch() {
        let tmp = TempDir::new().unwrap();

        let result = resolve_qemu_config_path_in_dir(tmp.path(), None, None).unwrap();
        assert_eq!(result, tmp.path().join(".qemu.toml"));
    }

    #[test]
    fn build_config_explicit_path_wins() {
        let tmp = TempDir::new().unwrap();

        let explicit = tmp.path().join("custom.build.toml");
        let result = config_loader::resolve_build_config_path(tmp.path(), Some(explicit.clone()));
        assert_eq!(result, explicit);
    }

    #[test]
    fn build_config_defaults_to_workspace_root() {
        let tmp = TempDir::new().unwrap();

        let result = config_loader::resolve_build_config_path(tmp.path(), None);
        assert_eq!(result, tmp.path().join(".build.toml"));
    }
}
