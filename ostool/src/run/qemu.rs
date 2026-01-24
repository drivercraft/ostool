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
    io::{BufReader, Read},
    path::PathBuf,
    process::{Child, Stdio},
};

use anyhow::anyhow;
use colored::Colorize;
use crossterm::terminal::disable_raw_mode;
use object::Architecture;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::io::{self, Write};
use tokio::fs;

use crate::{
    ctx::AppContext,
    run::ovmf_prebuilt::{Arch, FileType, Prebuilt, Source},
};

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
            .map_err(|_| anyhow!("can not open config file: {}", config_path.display()))?;
        let config: QemuConfig = toml::from_str(&config_content)?;
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
        fs::write(&config_path, toml::to_string_pretty(&config)?).await?;
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

        let arch = self.detect_arch()?;

        let machine = "virt".to_string();

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
            let _ = fs::remove_file("target/qemu.dtb").await;
            cmd.arg("-machine").arg("dumpdtb=target/qemu.dtb");
            // machine = format!("{},dumpdtb=target/qemu.dtb", machine);
        }

        if need_machine {
            cmd.arg("-machine").arg(machine);
        }

        if self.ctx.debug {
            cmd.arg("-s").arg("-S");
        }

        if let Some(bios) = self.bios().await? {
            cmd.arg("-bios").arg(bios);
        }

        if let Some(bin_path) = &self.ctx.paths.artifacts.bin {
            cmd.arg("-kernel").arg(bin_path);
        } else if let Some(elf_path) = &self.ctx.paths.artifacts.elf {
            cmd.arg("-kernel").arg(elf_path);
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

    fn detect_arch(&self) -> anyhow::Result<String> {
        if let Some(arch) = &self.ctx.arch {
            return Ok(format!("{:?}", arch).to_lowercase());
        }

        Err(anyhow!(
            "Please specify `arch` in QEMU config or provide a valid ELF file."
        ))
    }

    async fn bios(&self) -> anyhow::Result<Option<PathBuf>> {
        if self.config.uefi {
            Ok(Some(self.preper_ovmf().await?))
        } else {
            Ok(None)
        }
    }

    async fn preper_ovmf(&self) -> anyhow::Result<PathBuf> {
        let arch =
            self.ctx.arch.as_ref().ok_or_else(|| {
                anyhow::anyhow!("Cannot determine architecture for OVMF preparation")
            })?;
        let tmp = std::env::temp_dir();
        let bios_dir = tmp.join("ostool").join("ovmf");
        fs::create_dir_all(&bios_dir).await?;

        println!("Preparing OVMF firmware for architecture: {:?}", arch);
        let prebuilt = Prebuilt::fetch(Source::LATEST, &bios_dir)?;
        let arch = match arch {
            Architecture::X86_64 => Arch::X64,
            Architecture::Aarch64 => Arch::Aarch64,
            Architecture::Riscv64 => Arch::Riscv64,
            Architecture::LoongArch64 => Arch::LoongArch64,
            Architecture::I386 => Arch::Ia32,
            o => return Err(anyhow::anyhow!("OVMF is not supported for {o:?} ",)),
        };

        let bios_path = prebuilt.get_file(arch, FileType::Code);

        Ok(bios_path)
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
