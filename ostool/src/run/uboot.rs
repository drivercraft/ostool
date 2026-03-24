use std::{
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
        output_matcher::{ByteStreamMatcher, MATCH_DRAIN_DURATION, StreamMatchKind},
        tftp,
    },
    sterm::SerialTerm,
    utils::{PathResultExt, replace_env_placeholders},
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
}

impl UbootConfig {
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

#[derive(Debug, Clone)]
pub struct RunUbootArgs {
    pub config: Option<PathBuf>,
    pub show_output: bool,
}

impl Tool {
    pub async fn run_uboot(&mut self, args: RunUbootArgs) -> anyhow::Result<()> {
        let config_path = match args.config.clone() {
            Some(path) => path,
            None => self.workspace_dir().join(".uboot.toml"),
        };

        let config = if config_path.exists() {
            println!("Using U-Boot config: {}", config_path.display());
            let mut config_content = fs::read_to_string(&config_path)
                .await
                .with_path("failed to read file", &config_path)?;

            config_content = replace_env_placeholders(&config_content)?;

            let config: UbootConfig = toml::from_str(&config_content).with_context(|| {
                format!("failed to parse U-Boot config: {}", config_path.display())
            })?;
            config
        } else {
            let config = UbootConfig {
                serial: "/dev/ttyUSB0".to_string(),
                baud_rate: "115200".into(),
                ..Default::default()
            };

            fs::write(&config_path, toml::to_string_pretty(&config)?)
                .await
                .with_path("failed to write file", &config_path)?;
            config
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

impl Runner<'_> {
    /// 生成压缩的 FIT image 包含 kernel 和 FDT
    ///
    /// # 参数
    /// - `kernel_path`: kernel 文件路径
    /// - `dtb_path`: DTB 文件路径（可选）
    /// - `kernel_load_addr`: kernel 加载地址
    ///
    /// # 返回值
    /// 返回生成的 FIT image 文件路径
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
            _ => todo!(),
        };

        // 创建配置，与 test.its 文件中的参数一致
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

            // Can not compress DTB, U-Boot will not accept it
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

        // 使用新的 mkimage API 构建 FIT image
        let mut builder = FitImageBuilder::new();
        let fit_data = builder
            .build(config)
            .with_context(|| errors::FIT_BUILD_ERROR.to_string())?;

        // 保存到文件
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
        self.preper_regex()?;
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
        if is_tftp.is_none() && let Some(ip) = ip_string.as_ref() {
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

        #[cfg(target_os = "linux")]
        let mut linux_system_tftp_active = false;

        #[cfg(target_os = "linux")]
        let fitname = if let Some(system_tftp) = linux_system_tftp.as_ref() {
            let prepared = tftp::stage_linux_fit_image(&fitimage, &system_tftp.directory)?;
            linux_system_tftp_active = true;
            info!(
                "Staged FIT image to: {}",
                prepared.absolute_fit_path.display()
            );
            prepared.relative_filename
        } else if is_tftp.is_some() {
            let tftp_dir = self
                .config
                .net
                .as_ref()
                .and_then(|net| net.tftp_dir.as_ref())
                .unwrap();

            let fitimage = fitimage.file_name().unwrap();
            let tftp_path = PathBuf::from(tftp_dir).join(fitimage);

            info!("Setting TFTP file path: {}", tftp_path.display());
            tftp_path.display().to_string()
        } else {
            let name = fitimage
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(anyhow!("Invalid fitimage filename"))?;

            info!("Using fitimage filename: {}", name);
            name.to_string()
        };

        #[cfg(not(target_os = "linux"))]
        let fitname = if is_tftp.is_some() {
            let tftp_dir = self
                .config
                .net
                .as_ref()
                .and_then(|net| net.tftp_dir.as_ref())
                .unwrap();

            let fitimage = fitimage.file_name().unwrap();
            let tftp_path = PathBuf::from(tftp_dir).join(fitimage);

            info!("Setting TFTP file path: {}", tftp_path.display());
            tftp_path.display().to_string()
        } else {
            let name = fitimage
                .file_name()
                .and_then(|n| n.to_str())
                .ok_or(anyhow!("Invalid fitimage filename"))?;

            info!("Using fitimage filename: {}", name);
            name.to_string()
        };

        #[cfg(target_os = "linux")]
        let network_transfer_ready =
            linux_system_tftp_active || is_tftp.is_some() || builtin_tftp_started;

        #[cfg(not(target_os = "linux"))]
        let network_transfer_ready = is_tftp.is_some() || builtin_tftp_started;

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
        // if self.config.net.is_some() {
        //     info!("TFTP upload FIT image to board...");
        //     let filename = fitimage.file_name().unwrap().to_str().unwrap();

        //     let tftp_cmd = format!("tftp {filename}");
        //     uboot.cmd(&tftp_cmd)?;
        //     uboot.cmd_without_reply("bootm")?;
        // } else {
        //     info!("No TFTP config, using loady to upload FIT image...");
        //     Self::uboot_loady(&mut uboot, fit_loadaddr as usize, fitimage);
        //     uboot.cmd_without_reply("bootm")?;
        // }

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
        let mut shell = SerialTerm::new_with_byte_callback(tx, rx, move |h, byte| {
            let mut matcher = matcher_clone.lock().unwrap();
            if let Some(matched) = matcher.observe_byte(byte) {
                match matched.kind {
                    StreamMatchKind::Success => {
                        println!(
                            "{}",
                            format!(
                                "\r\n=== SUCCESS PATTERN MATCHED: {} ===",
                                matched.matched_regex
                            )
                            .green()
                        );
                        let mut res_lock = res_clone.lock().unwrap();
                        *res_lock = Some(Ok(()));
                    }
                    StreamMatchKind::Fail => {
                        println!(
                            "{}",
                            format!(
                                "\r\n=== FAIL PATTERN MATCHED: {} ===",
                                matched.matched_regex
                            )
                            .red()
                        );
                        let mut res_lock = res_clone.lock().unwrap();
                        *res_lock = Some(Err(anyhow!(
                            "Fail pattern matched '{}': {}",
                            matched.matched_regex,
                            matched.matched_text.trim_end()
                        )));
                    }
                }

                h.stop_after(MATCH_DRAIN_DURATION);
            }

            if matcher.should_stop() {
                h.stop();
            }
        });
        shell.run().await?;
        {
            let mut res_lock = res.lock().unwrap();
            if let Some(result) = res_lock.take() {
                result?;
            }
        }
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
    use super::build_network_boot_request;

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
}
