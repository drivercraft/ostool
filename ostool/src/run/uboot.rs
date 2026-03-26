use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use anyhow::Context;
use byte_unit::Byte;
use colored::Colorize;
use fitimage::{ComponentConfig, FitImageBuilder, FitImageConfig};
use indicatif::{ProgressBar, ProgressState, ProgressStyle};
use log::{info, warn};
use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::fs;
use uboot_shell::UbootShell;

use crate::{
    Tool,
    run::{
        output_matcher::{
            ByteStreamMatcher, MATCH_DRAIN_DURATION, compile_regexes, print_match_event,
        },
        shell_init::{ShellAutoInitMatcher, normalize_shell_init_config, spawn_delayed_send},
        tftp,
    },
    sterm::SerialTerm,
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
    /// Serial console device
    /// e.g., /dev/ttyUSB0 on linux, COM3 on Windows
    pub serial: String,
    pub baud_rate: String,
    pub dtb_file: Option<String>,
    /// Kernel load address
    /// if not specified, use U-Boot env variable 'loadaddr'
    pub kernel_load_addr: Option<String>,
    /// Fit Image load address
    /// if not specified, use automatically calculated address
    pub fit_load_addr: Option<String>,
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
    /// Timeout in seconds after entering kernel output. `None` or `0` disables the timeout.
    pub timeout: Option<u64>,
}

impl UbootConfig {
    fn replace_strings(&mut self, tool: &Tool) -> anyhow::Result<()> {
        self.serial = tool.replace_string(&self.serial)?;
        self.baud_rate = tool.replace_string(&self.baud_rate)?;
        self.dtb_file = self
            .dtb_file
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.kernel_load_addr = self
            .kernel_load_addr
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.fit_load_addr = self
            .fit_load_addr
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.board_reset_cmd = self
            .board_reset_cmd
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.board_power_off_cmd = self
            .board_power_off_cmd
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
        self.uboot_cmd = self
            .uboot_cmd
            .as_ref()
            .map(|values| {
                values
                    .iter()
                    .map(|value| tool.replace_string(value))
                    .collect::<anyhow::Result<Vec<_>>>()
            })
            .transpose()?;
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
        if let Some(net) = &mut self.net {
            net.replace_strings(tool)?;
        }
        Ok(())
    }

    pub fn kernel_load_addr_int(&self) -> Option<u64> {
        self.addr_int(self.kernel_load_addr.as_ref())
    }

    pub fn fit_load_addr_int(&self) -> Option<u64> {
        self.addr_int(self.fit_load_addr.as_ref())
    }

    fn addr_int(&self, addr_str: Option<&String>) -> Option<u64> {
        addr_str.as_ref().and_then(|addr_str| {
            if addr_str.starts_with("0x") || addr_str.starts_with("0X") {
                u64::from_str_radix(&addr_str[2..], 16).ok()
            } else {
                addr_str.parse::<u64>().ok()
            }
        })
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
    fn replace_strings(&mut self, tool: &Tool) -> anyhow::Result<()> {
        self.interface = tool.replace_string(&self.interface)?;
        self.board_ip = self
            .board_ip
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.gatewayip = self
            .gatewayip
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.netmask = self
            .netmask
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.tftp_dir = self
            .tftp_dir
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct RunUbootArgs {
    pub config: Option<PathBuf>,
    pub show_output: bool,
}

impl Tool {
    pub async fn run_uboot(&mut self, args: RunUbootArgs) -> anyhow::Result<()> {
        let config_path = match args.config.clone() {
            Some(path) => self.replace_path_variables(path)?,
            None => self.workspace_dir().join(".uboot.toml"),
        };

        let config = match fs::read_to_string(&config_path).await {
            Ok(content) => {
                println!("Using U-Boot config: {}", config_path.display());
                let mut config: UbootConfig = toml::from_str(&content).with_context(|| {
                    format!("failed to parse U-Boot config: {}", config_path.display())
                })?;
                config.replace_strings(self)?;
                config.normalize(&format!("U-Boot config {}", config_path.display()))?;
                config
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {
                let mut config = UbootConfig {
                    serial: "/dev/ttyUSB0".to_string(),
                    baud_rate: "115200".into(),
                    ..Default::default()
                };
                config.normalize(&format!("U-Boot config {}", config_path.display()))?;
                fs::write(&config_path, toml::to_string_pretty(&config)?)
                    .await
                    .with_path("failed to write file", &config_path)?;
                config
            }
            Err(e) => return Err(e.into()),
        };

        let baud_rate = config.baud_rate.parse::<u32>().with_context(|| {
            format!(
                "baud_rate is not a valid integer in {}",
                config_path.display()
            )
        })?;

        let mut runner = Runner {
            tool: self,
            config,
            baud_rate,
            success_regex: vec![],
            fail_regex: vec![],
        };
        runner.run().await?;
        Ok(())
    }
}

struct Runner<'a> {
    tool: &'a mut Tool,
    config: UbootConfig,
    success_regex: Vec<regex::Regex>,
    fail_regex: Vec<regex::Regex>,
    baud_rate: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct NetworkBootRequest {
    bootfile: String,
    bootcmd: String,
    ipaddr: Option<String>,
}

struct SharedWrite {
    inner: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl SharedWrite {
    fn new(inner: Arc<Mutex<Box<dyn Write + Send>>>) -> Self {
        Self { inner }
    }
}

impl Write for SharedWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.inner.lock().unwrap().write(buf)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.lock().unwrap().flush()
    }
}

impl Runner<'_> {
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

        let arch = match self.tool.ctx.arch.as_ref().unwrap() {
            object::Architecture::Aarch64 => "arm64",
            object::Architecture::Arm => "arm",
            object::Architecture::LoongArch64 => "loongarch64",
            object::Architecture::Riscv64 => "riscv",
            _ => todo!(),
        };

        let mut config = FitImageConfig::new("Various kernels, ramdisks and FDT blobs")
            .with_kernel(
                ComponentConfig::new("kernel", kernel_data)
                    .with_description("This kernel")
                    .with_type("kernel")
                    .with_arch(arch)
                    .with_os("linux")
                    .with_compression(true)
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
        let res = self._run().await;
        if let Some(ref cmd) = self.config.board_power_off_cmd
            && !cmd.trim().is_empty()
        {
            let _ = self.tool.shell_run_cmd(cmd);
            info!("Board powered off");
        }
        res
    }

    async fn _run(&mut self) -> anyhow::Result<()> {
        self.prepare_regex()?;
        self.tool.objcopy_output_bin()?;

        let kernel = self
            .tool
            .ctx
            .artifacts
            .bin
            .as_ref()
            .ok_or(anyhow!("bin not exist"))?;

        info!("Starting U-Boot runner...");

        info!("kernel from: {}", kernel.display());

        let ip_string = self.detect_tftp_ip();

        let is_tftp = self
            .config
            .net
            .as_ref()
            .and_then(|net| net.tftp_dir.as_deref())
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .map(PathBuf::from);

        #[cfg(target_os = "linux")]
        let linux_system_tftp = if let Some(directory) = is_tftp.clone() {
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
        } else if self.config.net.is_some() && ip_string.is_some() {
            Some(tftp::ensure_linux_tftpd_hpa()?)
        } else {
            None
        };

        let mut builtin_tftp_started = false;

        #[cfg(not(target_os = "linux"))]
        if is_tftp.is_none()
            && let Some(ip) = ip_string.as_ref()
        {
            info!("TFTP server IP: {}", ip);
            tftp::run_tftp_server(self.tool)?;
            builtin_tftp_started = true;
        }

        #[cfg(target_os = "linux")]
        if linux_system_tftp.is_none()
            && is_tftp.is_none()
            && let Some(ip) = ip_string.as_ref()
        {
            info!("TFTP server IP: {}", ip);
            tftp::run_tftp_server(self.tool)?;
            builtin_tftp_started = true;
        }

        info!(
            "Opening serial port: {} @ {}",
            self.config.serial, self.baud_rate
        );

        let rx = serialport::new(&self.config.serial, self.baud_rate as _)
            .timeout(Duration::from_millis(200))
            .open()
            .with_context(|| format!("failed to open serial port {}", self.config.serial))?;
        let tx = rx
            .try_clone()
            .with_context(|| format!("failed to clone serial port {}", self.config.serial))?;

        println!("Waiting for board on power or reset...");
        let handle: thread::JoinHandle<anyhow::Result<UbootShell>> = thread::spawn(move || {
            let uboot = UbootShell::new(tx, rx)?;
            Ok(uboot)
        });

        if let Some(cmd) = self.config.board_reset_cmd.clone()
            && !cmd.trim().is_empty()
        {
            self.tool.shell_run_cmd(&cmd)?;
        }

        let mut net_ok = false;

        let mut uboot = handle.join().unwrap()?;
        uboot.set_env("autoload", "yes")?;

        if let Some(ref cmds) = self.config.uboot_cmd {
            for cmd in cmds.iter() {
                info!("Running U-Boot command: {}", cmd);
                uboot.cmd(cmd)?;
            }
        }

        if let Some(ref net) = self.config.net {
            if let Some(ref gatewayip) = net.gatewayip {
                uboot.set_env("gatewayip", gatewayip)?;
            }

            if let Some(ref netmask) = net.netmask {
                uboot.set_env("netmask", netmask)?;
            }
        }

        if let Some(ref ip) = ip_string
            && let Ok(output) = uboot.cmd("net list")
        {
            let device_list = output.strip_prefix("net list").unwrap_or(&output).trim();

            if device_list.is_empty() {
                let _ = uboot.cmd("bootdev hunt ethernet");
            }

            info!("Board network ok");

            uboot.set_env("serverip", ip.clone())?;
            net_ok = true;
        }

        let mut fdt_load_addr = None;
        let mut ramfs_load_addr = None;

        if let Ok(addr) = uboot.env_int("fdt_addr_r") {
            fdt_load_addr = Some(addr as u64);
        }

        if let Ok(addr) = uboot.env_int("ramdisk_addr_r") {
            ramfs_load_addr = Some(addr as u64);
        }

        let kernel_entry = if let Some(entry) = self.config.kernel_load_addr_int() {
            info!("Using configured kernel load address: {entry:#x}");
            entry
        } else if let Ok(entry) = uboot.env_int("kernel_addr_r") {
            info!("Using $kernel_addr_r as kernel entry: {entry:#x}");
            entry as u64
        } else if let Ok(entry) = uboot.env_int("loadaddr") {
            info!("Using $loadaddr as kernel entry: {entry:#x}");
            entry as u64
        } else {
            return Err(anyhow!("Cannot determine kernel entry address"));
        };

        let mut fit_loadaddr = if let Ok(addr) = uboot.env_int("kernel_comp_addr_r") {
            info!("image load to kernel_comp_addr_r: {addr:#x}");
            addr as u64
        } else if let Ok(addr) = uboot.env_int("kernel_addr_c") {
            info!("image load to kernel_addr_c: {addr:#x}");
            addr as u64
        } else {
            let addr = (kernel_entry + 0x02000000) & 0xffff_ffff_ff00_0000;
            info!("No kernel_comp_addr_r or kernel_addr_c, use calculated address: {addr:#x}");
            addr
        };

        if let Some(fit_load_addr_int) = self.config.fit_load_addr_int() {
            fit_loadaddr = fit_load_addr_int;
        }

        uboot.set_env("loadaddr", format!("{:#x}", fit_loadaddr))?;

        info!("fitimage loadaddr: {fit_loadaddr:#x}");
        info!("kernel entry: {kernel_entry:#x}");
        let dtb = self.config.dtb_file.clone();
        if let Some(ref dtb_file) = dtb {
            info!("Using DTB from: {}", dtb_file);
        }

        let dtb_path = dtb.as_ref().map(Path::new);
        let fitimage = self
            .generate_fit_image(
                kernel,
                dtb_path,
                kernel_entry,
                kernel_entry,
                fdt_load_addr,
                ramfs_load_addr,
            )
            .await?;

        let (fitname, linux_tftp_active) = if cfg!(target_os = "linux") {
            if let Some(system_tftp) = linux_system_tftp.as_ref() {
                let prepared = tftp::stage_linux_fit_image(&fitimage, &system_tftp.directory)?;
                info!(
                    "Staged FIT image to: {}",
                    prepared.absolute_fit_path.display()
                );
                (prepared.relative_filename, true)
            } else if let Some(tftp_dir) = is_tftp.as_deref() {
                let fitimage = fitimage.file_name().unwrap();
                let tftp_path = PathBuf::from(tftp_dir).join(fitimage);
                info!("Setting TFTP file path: {}", tftp_path.display());
                (tftp_path.display().to_string(), false)
            } else {
                let name = fitimage
                    .file_name()
                    .and_then(|n| n.to_str())
                    .ok_or(anyhow!("Invalid fitimage filename"))?;
                info!("Using fitimage filename: {}", name);
                (name.to_string(), false)
            }
        } else if let Some(tftp_dir) = is_tftp.as_deref() {
            let fitimage = fitimage.file_name().unwrap();
            let tftp_path = PathBuf::from(tftp_dir).join(fitimage);
            info!("Setting TFTP file path: {}", tftp_path.display());
            (tftp_path.display().to_string(), false)
        } else {
            let name = fitimage
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(anyhow!("Invalid fitimage filename"))?;
            info!("Using fitimage filename: {}", name);
            (name.to_string(), false)
        };

        let network_transfer_ready = linux_tftp_active || is_tftp.is_some() || builtin_tftp_started;

        let bootcmd = if let Some(request) = build_network_boot_request(
            self.config
                .net
                .as_ref()
                .and_then(|net| net.board_ip.as_deref()),
            net_ok,
            network_transfer_ready,
            &fitname,
        ) {
            if let Some(ref board_ip) = request.ipaddr {
                uboot.set_env("ipaddr", board_ip)?;
            }
            uboot.set_env("bootfile", &request.bootfile)?;
            request.bootcmd
        } else {
            info!("No TFTP config, using loady to upload FIT image...");
            Self::uboot_loady(&mut uboot, fit_loadaddr as usize, fitimage);
            "bootm".to_string()
        };

        info!("Booting kernel with command: {}", bootcmd);
        uboot.cmd_without_reply(&bootcmd)?;

        let tx = uboot.tx.take().unwrap();
        let rx = uboot.rx.take().unwrap();

        drop(uboot);

        println!("{}", "Interacting with U-Boot shell...".green());

        let matcher = Arc::new(Mutex::new(ByteStreamMatcher::new(
            self.success_regex.clone(),
            self.fail_regex.clone(),
        )));

        let res = Arc::new(Mutex::<Option<anyhow::Result<()>>>::new(None));
        let res_clone = res.clone();
        let matcher_clone = matcher.clone();
        let shared_tx = Arc::new(Mutex::new(tx));
        let shell_init = Arc::new(Mutex::new(self.config.shell_auto_init()));
        let shell_init_clone = shell_init.clone();
        let shared_tx_clone = shared_tx.clone();
        let mut shell = SerialTerm::new_with_byte_callback(
            Box::new(SharedWrite::new(shared_tx)),
            rx,
            move |h, byte| {
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
                    spawn_delayed_send(shared_tx_clone.clone(), command);
                }

                if matcher.should_stop() {
                    h.stop();
                }
            },
        );
        if let Some(timeout) = timeout_duration(self.config.timeout) {
            shell = shell.with_timeout(timeout, "kernel boot");
        }
        shell.run().await?;
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

    fn detect_tftp_ip(&self) -> Option<String> {
        let net = self.config.net.as_ref()?;

        let mut ip_string = String::new();

        let interfaces = NetworkInterface::show().unwrap();
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

    fn uboot_loady(uboot: &mut UbootShell, addr: usize, file: impl Into<PathBuf>) {
        println!("{}", "\r\nsend file".green());

        let pb = ProgressBar::new(100);
        pb.set_style(ProgressStyle::with_template("{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {bytes}/{total_bytes} ({eta})")
        .unwrap()
        .with_key("eta", |state: &ProgressState, w: &mut dyn core::fmt::Write| write!(w, "{:.1}s", state.eta().as_secs_f64()).unwrap())
        .progress_chars("#>-"));

        let res = uboot
            .loady(addr, file, |x, a| {
                pb.set_length(a as _);
                pb.set_position(x as _);
            })
            .unwrap();

        pb.finish_with_message("upload done");

        println!("{}", res);
        println!("send ok");
    }
}

fn timeout_duration(timeout: Option<u64>) -> Option<Duration> {
    match timeout {
        Some(0) | None => None,
        Some(secs) => Some(Duration::from_secs(secs)),
    }
}

fn build_network_boot_request(
    board_ip: Option<&str>,
    net_ok: bool,
    network_transfer_ready: bool,
    fitname: &str,
) -> Option<NetworkBootRequest> {
    if !network_transfer_ready {
        return None;
    }

    if let Some(board_ip) = board_ip {
        return Some(NetworkBootRequest {
            bootfile: fitname.to_string(),
            bootcmd: format!("tftp {fitname} && bootm"),
            ipaddr: Some(board_ip.to_string()),
        });
    }

    if net_ok {
        return Some(NetworkBootRequest {
            bootfile: fitname.to_string(),
            bootcmd: format!("dhcp {fitname} && bootm"),
            ipaddr: None,
        });
    }

    None
}

#[cfg(test)]
mod tests {
    use super::{Net, UbootConfig, build_network_boot_request, timeout_duration};
    use crate::{
        Tool, ToolConfig,
        build::config::{BuildConfig, BuildSystem, Cargo},
    };
    use std::collections::HashMap;
    use std::time::Duration;

    #[test]
    fn network_boot_request_uses_same_filename_for_bootfile() {
        let request = build_network_boot_request(
            Some("192.168.1.10"),
            false,
            true,
            "ostool/home/user/workspace/target/image.fit",
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
    fn network_boot_request_requires_ready_transport() {
        assert!(
            build_network_boot_request(Some("192.168.1.10"), false, false, "image.fit").is_none()
        );
        assert!(build_network_boot_request(None, false, true, "image.fit").is_none());
        assert_eq!(
            build_network_boot_request(None, true, true, "image.fit")
                .unwrap()
                .bootcmd,
            "dhcp image.fit && bootm"
        );
    }

    #[test]
    fn uboot_config_normalize_rejects_shell_init_without_prefix() {
        let mut config = UbootConfig {
            serial: "/dev/null".into(),
            baud_rate: "115200".into(),
            shell_init_cmd: Some("root".into()),
            ..Default::default()
        };

        let err = config.normalize("test config").unwrap_err();
        assert!(err.to_string().contains("shell_prefix"));
    }

    #[test]
    fn uboot_config_normalize_trims_shell_fields() {
        let mut config = UbootConfig {
            serial: "/dev/null".into(),
            baud_rate: "115200".into(),
            shell_prefix: Some(" login: ".into()),
            shell_init_cmd: Some(" root ".into()),
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

        let mut tool = Tool::new(ToolConfig {
            manifest: Some(tmp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();
        tool.ctx.build_config = Some(BuildConfig {
            system: BuildSystem::Cargo(Cargo {
                env: HashMap::new(),
                target: "aarch64-unknown-none".into(),
                package: "sample".into(),
                features: vec![],
                log: None,
                extra_config: None,
                args: vec![],
                pre_build_cmds: vec![],
                post_build_cmds: vec![],
                to_bin: false,
            }),
        });
        unsafe {
            std::env::set_var("OSTOOL_UBOOT_TEST_ENV", "env-ok");
        }

        let mut config = UbootConfig {
            serial: "${workspace}/tty".into(),
            baud_rate: "${env:OSTOOL_UBOOT_TEST_ENV}".into(),
            dtb_file: Some("${package}/board.dtb".into()),
            kernel_load_addr: Some("${workspaceFolder}".into()),
            fit_load_addr: Some("${package}".into()),
            board_reset_cmd: Some("${workspace}".into()),
            board_power_off_cmd: Some("${package}".into()),
            success_regex: vec!["${workspace}".into()],
            fail_regex: vec!["${package}".into()],
            uboot_cmd: Some(vec!["setenv boot ${workspace}".into()]),
            shell_prefix: Some("${workspace}".into()),
            shell_init_cmd: Some("${package}".into()),
            net: Some(Net {
                interface: "${env:OSTOOL_UBOOT_TEST_ENV}".into(),
                board_ip: Some("${workspace}".into()),
                gatewayip: Some("${package}".into()),
                netmask: Some("${workspaceFolder}".into()),
                tftp_dir: Some("${package}/tftp".into()),
            }),
            ..Default::default()
        };

        config.replace_strings(&tool).unwrap();

        let expected = tmp.path().display().to_string();
        assert_eq!(config.serial, format!("{expected}/tty"));
        assert_eq!(config.baud_rate, "env-ok");
        assert_eq!(
            config.dtb_file.as_deref(),
            Some(format!("{expected}/board.dtb").as_str())
        );
        assert_eq!(config.kernel_load_addr.as_deref(), Some(expected.as_str()));
        assert_eq!(config.fit_load_addr.as_deref(), Some(expected.as_str()));
        assert_eq!(config.board_reset_cmd.as_deref(), Some(expected.as_str()));
        assert_eq!(
            config.board_power_off_cmd.as_deref(),
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
        let net = config.net.unwrap();
        assert_eq!(net.interface, "env-ok");
        assert_eq!(net.board_ip.as_deref(), Some(expected.as_str()));
        assert_eq!(net.gatewayip.as_deref(), Some(expected.as_str()));
        assert_eq!(net.netmask.as_deref(), Some(expected.as_str()));
        assert_eq!(
            net.tftp_dir.as_deref(),
            Some(format!("{expected}/tftp").as_str())
        );
    }
}
