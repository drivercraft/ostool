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
//! to_bin = true
//! success_regex = ["All tests passed"]
//! fail_regex = ["PANIC", "FAILED"]
//! ```

use std::{
    ffi::OsString,
    io::{self, BufReader, ErrorKind, Read, Write},
    path::Path,
    path::PathBuf,
    process::{Child, Stdio},
};

use anyhow::{Context, anyhow};
use colored::Colorize;
use crossterm::terminal::disable_raw_mode;
use object::Architecture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{
    ctx::AppContext,
    run::ovmf_prebuilt::{Arch, FileType, Prebuilt, Source},
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
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Default)]
pub struct QemuConfig {
    /// Additional QEMU command-line arguments.
    pub args: Vec<String>,
    /// Whether to use UEFI boot via OVMF firmware.
    pub uefi: bool,
    /// Whether to convert ELF to raw binary before loading.
    pub to_bin: bool,
    /// Regex patterns that indicate successful execution.
    pub success_regex: Vec<String>,
    /// Regex patterns that indicate failed execution.
    pub fail_regex: Vec<String>,
}

/// Arguments for running QEMU.
#[derive(Debug, Clone)]
pub struct RunQemuArgs {
    /// Optional path to QEMU configuration file.
    pub qemu_config: Option<PathBuf>,
    /// Whether to dump the device tree blob.
    pub dtb_dump: bool,
    /// Whether to show QEMU output.
    pub show_output: bool,
}

/// Runs the operating system in QEMU.
///
/// This function configures and launches QEMU with the appropriate settings
/// based on the detected architecture and configuration file.
///
/// # Arguments
///
/// * `ctx` - The application context containing paths and build artifacts.
/// * `args` - QEMU run arguments.
///
/// # Errors
///
/// Returns an error if QEMU fails to start or exits with an error.
pub async fn run_qemu(ctx: AppContext, args: RunQemuArgs) -> anyhow::Result<()> {
    // Build logic will be implemented here
    let config_path = match args.qemu_config.clone() {
        Some(path) => path,
        None => ctx.paths.manifest.join(".qemu.toml"),
    };

    info!("Using QEMU config file: {}", config_path.display());

    let config = if config_path.exists() {
        let config_content = fs::read_to_string(&config_path)
            .await
            .with_path("failed to read file", &config_path)?;
        let config: QemuConfig = toml::from_str(&config_content)
            .with_context(|| format!("failed to parse QEMU config: {}", config_path.display()))?;
        config
    } else {
        let mut config = QemuConfig {
            to_bin: true,
            ..Default::default()
        };
        config.args.push("-nographic".to_string());
        if let Some(arch) = ctx.arch {
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
        fs::write(&config_path, toml::to_string_pretty(&config)?)
            .await
            .with_path("failed to write file", &config_path)?;
        config
    };

    let mut runner = QemuRunner {
        ctx,
        config,
        args: vec![],
        dtbdump: args.dtb_dump,
        success_regex: vec![],
        fail_regex: vec![],
    };
    runner.run().await?;
    Ok(())
}

struct QemuRunner {
    ctx: AppContext,
    config: QemuConfig,
    args: Vec<String>,
    dtbdump: bool,
    success_regex: Vec<regex::Regex>,
    fail_regex: Vec<regex::Regex>,
}

impl QemuRunner {
    async fn run(&mut self) -> anyhow::Result<()> {
        self.preper_regex()?;

        if self.config.to_bin {
            self.ctx.objcopy_output_bin()?;
        }

        let detected_arch = self.ctx.arch.ok_or_else(|| {
            anyhow!("Please specify `arch` in QEMU config or provide a valid ELF file.")
        })?;
        let arch = format!("{detected_arch:?}").to_lowercase();

        let machine = match detected_arch {
            Architecture::X86_64 | Architecture::I386 => "q35",
            _ => "virt",
        }
        .to_string();

        let mut need_machine = true;

        for arg in &self.config.args {
            if arg == "-machine" || arg == "-M" {
                need_machine = false;
            }

            self.args.push(arg.clone());
        }

        #[allow(unused_mut)]
        let mut qemu_executable = format!("qemu-system-{}", arch);

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

        let mut cmd = self.ctx.command(&qemu_executable);

        for arg in &self.config.args {
            cmd.arg(arg);
        }

        if self.dtbdump {
            let dtb_dump_path = PathBuf::from("target/qemu.dtb");
            if let Err(err) = fs::remove_file(&dtb_dump_path).await
                && err.kind() != ErrorKind::NotFound
            {
                return Err(err).with_path("failed to remove file", &dtb_dump_path);
            }
            cmd.arg("-machine")
                .arg(format!("dumpdtb={}", dtb_dump_path.display()));
            // machine = format!("{},dumpdtb=target/qemu.dtb", machine);
        }

        if need_machine {
            cmd.arg("-machine").arg(machine);
        }

        if self.ctx.debug {
            cmd.arg("-s").arg("-S");
        }

        let mut use_kernel_loader = true;
        if let Some(uefi) = self.prepare_uefi().await? {
            match uefi {
                UefiBootConfig::Pflash {
                    code,
                    vars,
                    esp_dir,
                } => {
                    cmd.arg("-drive").arg(format!(
                        "if=pflash,format=raw,unit=0,readonly=on,file={}",
                        code.display()
                    ));
                    cmd.arg("-drive").arg(format!(
                        "if=pflash,format=raw,unit=1,file={}",
                        vars.display()
                    ));
                    cmd.arg("-drive")
                        .arg(format!("format=raw,file=fat:rw:{}", esp_dir.display()));
                    use_kernel_loader = false;
                }
            }
        }

        if use_kernel_loader {
            if let Some(bin_path) = &self.ctx.paths.artifacts.bin {
                cmd.arg("-kernel").arg(bin_path);
            } else if let Some(elf_path) = &self.ctx.paths.artifacts.elf {
                cmd.arg("-kernel").arg(elf_path);
            }
        }
        cmd.stdout(Stdio::piped());
        cmd.print_cmd();
        let mut child = cmd.spawn()?;

        let mut qemu_result: Option<anyhow::Result<()>> = None;

        let stdout = BufReader::new(child.stdout.take().unwrap());
        let mut line_buf = Vec::new();

        for byte in stdout.bytes() {
            let byte = match byte {
                Ok(b) => b,
                Err(e) => {
                    println!("stdout: {:?}", e);
                    continue;
                }
            };
            let _ = std::io::stdout().write_all(&[byte]);
            let _ = std::io::stdout().flush();

            line_buf.push(byte);
            if byte != b'\n' {
                continue;
            }

            let line = String::from_utf8_lossy(&line_buf).to_string();

            self.check_output(&line, &mut child, &mut qemu_result)?;
        }

        let out = child.wait_with_output()?;
        if let Some(res) = qemu_result {
            res?;
        } else if !out.status.success() {
            unsafe {
                return Err(anyhow::anyhow!(
                    "{}",
                    OsString::from_encoded_bytes_unchecked(out.stderr).to_string_lossy()
                ));
            }
        }
        Ok(())
    }

    async fn prepare_uefi(&self) -> anyhow::Result<Option<UefiBootConfig>> {
        if !self.config.uefi {
            return Ok(None);
        }

        let arch =
            self.ctx.arch.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Cannot determine architecture for OVMF preparation")
            })?;
        let tmp = std::env::temp_dir();
        let bios_dir = tmp.join("ostool").join("ovmf");
        fs::create_dir_all(&bios_dir)
            .await
            .with_path("failed to create directory", &bios_dir)?;

        println!("Preparing OVMF firmware for architecture: {:?}", arch);
        let prebuilt = Prebuilt::fetch(Source::LATEST, &bios_dir)
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
            .ctx
            .paths
            .artifacts
            .bin
            .as_ref()
            .ok_or_else(|| anyhow!("UEFI boot requires a BIN artifact"))?;
        let stem = bin_path
            .file_stem()
            .ok_or_else(|| anyhow!("invalid BIN path: {}", bin_path.display()))?;
        let artifact_dir = self.uefi_artifact_dir(bin_path)?;
        let esp_dir = artifact_dir.join(format!("{}.esp", stem.to_string_lossy()));
        let boot_dir = esp_dir.join("EFI").join("BOOT");
        fs::create_dir_all(&boot_dir)
            .await
            .with_path("failed to create directory", &boot_dir)?;

        let boot_path = boot_dir.join(Self::default_uefi_boot_filename(arch));
        fs::copy(bin_path, &boot_path).await.with_context(|| {
            format!(
                "failed to copy EFI image from {} to {}",
                bin_path.display(),
                boot_path.display()
            )
        })?;

        Ok(esp_dir)
    }

    fn uefi_artifact_dir(&self, bin_path: &Path) -> anyhow::Result<PathBuf> {
        let metadata = self.ctx.metadata()?;
        let target_dir = metadata.target_directory.into_std_path_buf();
        let target_dir = target_dir.canonicalize().unwrap_or(target_dir);
        let bin_path = bin_path
            .canonicalize()
            .with_path("failed to canonicalize file", bin_path)?;
        let artifact_dir = match bin_path.strip_prefix(&target_dir) {
            Ok(relative_bin_path) => {
                let artifact_parent = relative_bin_path.parent().ok_or_else(|| {
                    anyhow!(
                        "invalid BIN path under target directory: {}",
                        bin_path.display()
                    )
                })?;
                target_dir.join(artifact_parent)
            }
            Err(_) => bin_path
                .parent()
                .ok_or_else(|| anyhow!("invalid BIN path: {}", bin_path.display()))?
                .to_path_buf(),
        };

        Ok(artifact_dir)
    }

    async fn prepare_uefi_vars(&self, vars_template: &Path) -> anyhow::Result<PathBuf> {
        let bin_path = self
            .ctx
            .paths
            .artifacts
            .bin
            .as_ref()
            .ok_or_else(|| anyhow!("UEFI boot requires a BIN artifact"))?;
        let stem = bin_path
            .file_stem()
            .ok_or_else(|| anyhow!("invalid BIN path: {}", bin_path.display()))?;
        let artifact_dir = self.uefi_artifact_dir(bin_path)?;
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

    fn check_output(
        &self,
        out: &str,
        child: &mut Child,
        res: &mut Option<anyhow::Result<()>>,
    ) -> anyhow::Result<()> {
        // // Process QEMU output line here
        // println!("{}", line);

        for regex in &self.fail_regex {
            if regex.is_match(out) {
                *res = Some(Err(anyhow!(
                    "Detected failure pattern '{}' in QEMU output.",
                    regex.as_str()
                )));

                self.kill_qemu(child)?;
                return Ok(());
            }
        }

        for regex in &self.success_regex {
            if regex.is_match(out) {
                *res = Some(Ok(()));
                println!(
                    "{}",
                    format!(
                        "Detected success pattern '{}' in QEMU output, terminating QEMU.",
                        regex.as_str()
                    )
                    .green()
                );
                self.kill_qemu(child)?;
                return Ok(());
            }
        }

        Ok(())
    }

    fn kill_qemu(&self, child: &mut Child) -> anyhow::Result<()> {
        child.kill()?;

        // 尝试恢复终端状态
        let _ = disable_raw_mode();

        // 使用 stty 命令恢复终端回显 (最可靠的方法)
        let _ = std::process::Command::new("stty")
            .arg("echo")
            .arg("icanon")
            .status();

        // 刷新输出
        let _ = io::stdout().flush();
        println!();

        Ok(())
    }

    fn preper_regex(&mut self) -> anyhow::Result<()> {
        // Prepare regex patterns if needed
        // Compile success regex patterns
        for pattern in self.config.success_regex.iter() {
            // Compile and store the regex
            let regex =
                regex::Regex::new(pattern).map_err(|e| anyhow!("success regex error: {e}"))?;
            self.success_regex.push(regex);
        }

        // Compile fail regex patterns
        for pattern in self.config.fail_regex.iter() {
            // Compile and store the regex
            let regex = regex::Regex::new(pattern).map_err(|e| anyhow!("fail regex error: {e}"))?;
            self.fail_regex.push(regex);
        }

        Ok(())
    }
}
