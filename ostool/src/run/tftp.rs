//! TFTP server helpers for network booting.
//!
//! On Linux, this module prepares a system `tftpd-hpa` installation and stages
//! build artifacts into the configured TFTP root. Other platforms keep using
//! the built-in Rust TFTP server.

use std::{
    env, fs,
    io::{self, IsTerminal, Write},
    net::{IpAddr, Ipv4Addr},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, anyhow, bail};
use colored::Colorize as _;
use tftpd::{Config, Server};

use crate::{artifact::state::OutputArtifacts, utils::PathResultExt};

const TFTP_HPA_CONFIG_PATH: &str = "/etc/default/tftpd-hpa";
const DEFAULT_TFTP_DIRECTORY: &str = "/srv/tftp";
const TFTP_NAMESPACE: &str = "ostool";

static STAGING_SEQUENCE: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LinuxTftpPrepared {
    tftp_root: PathBuf,
    target_dir: PathBuf,
    absolute_fit_path: PathBuf,
    relative_filename: String,
}

impl LinuxTftpPrepared {
    pub fn target_dir(&self) -> &Path {
        &self.target_dir
    }

    pub fn absolute_fit_path(&self) -> &Path {
        &self.absolute_fit_path
    }

    pub fn relative_filename(&self) -> &str {
        &self.relative_filename
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TftpdHpaConfig {
    pub username: Option<String>,
    pub directory: PathBuf,
    pub address: Option<String>,
    pub options: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DistroKind {
    Debian,
    Rhel,
    Arch,
    OpenSuse,
    Alpine,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LeafClaimOutcome {
    Claimed,
    Collision,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommandSpec {
    program: String,
    args: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstallPlan {
    distro: DistroKind,
    commands: Vec<CommandSpec>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct EffectiveUser {
    name: String,
    group: String,
}

#[cfg(target_os = "linux")]
pub fn ensure_linux_tftpd_hpa() -> anyhow::Result<TftpdHpaConfig> {
    let binary = find_tftpd_binary();
    let is_root = is_root_user()?;

    if let Some(path) = binary {
        info!("Using system tftpd-hpa binary: {}", path.display());
    } else {
        let distro = detect_distro_kind()?;
        let install_plan = build_install_plan(distro)?;

        println!("{}", "未检测到 tftpd-hpa (in.tftpd)".yellow());
        println!("发行版: {}", distro.label());
        println!(
            "当前用户是否为 root: {}",
            if is_root { "yes" } else { "no" }
        );

        if install_plan.commands.is_empty() {
            bail!(
                "当前发行版暂不支持自动安装 tftpd-hpa，请手动安装后重试（发行版: {}）",
                distro.label()
            );
        }

        let display = render_command_chain(&install_plan.commands, is_root);
        println!("将执行安装命令:");
        println!("  {display}");

        if !(io::stdin().is_terminal() && io::stdout().is_terminal()) {
            bail!("当前终端不是交互式终端，请手动执行上述命令安装 tftpd-hpa");
        }

        if !prompt_yes_no("是否继续安装 tftpd-hpa? [y/N] ")? {
            bail!("已取消安装 tftpd-hpa");
        }

        for command in &install_plan.commands {
            run_privileged_command(command, is_root)
                .with_context(|| format!("failed to install tftpd-hpa via `{display}`"))?;
        }

        let path = find_tftpd_binary()
            .ok_or_else(|| anyhow!("安装完成后仍未找到 in.tftpd，请确认 tftpd-hpa 是否安装成功"))?;
        info!("Installed system tftpd-hpa binary: {}", path.display());
    }

    let (config, created) = ensure_tftpd_hpa_config(Path::new(TFTP_HPA_CONFIG_PATH), is_root)?;
    if created {
        if command_exists("systemctl") {
            let restart = CommandSpec {
                program: "systemctl".into(),
                args: vec!["restart".into(), "tftpd-hpa".into()],
            };
            run_privileged_command(&restart, is_root)
                .context("failed to restart tftpd-hpa after creating default config")?;
        } else {
            println!(
                "{}",
                "已创建 /etc/default/tftpd-hpa，请手动重启 tftpd-hpa 服务".yellow()
            );
        }
    }

    ensure_tftpd_hpa_service_ready(is_root)?;

    Ok(config)
}

#[cfg(target_os = "linux")]
pub fn stage_linux_fit_image(
    fitimage: &Path,
    tftp_root: &Path,
) -> anyhow::Result<LinuxTftpPrepared> {
    validate_tftp_root(tftp_root)?;
    let file_name = fitimage
        .file_name()
        .ok_or_else(|| anyhow!("invalid FIT image filename: {}", fitimage.display()))?;
    let target_dir = claim_tftp_namespace(tftp_root)?;
    let relative_dir = target_dir
        .strip_prefix(tftp_root)
        .with_context(|| {
            format!(
                "staged TFTP directory {} is outside root {}",
                target_dir.display(),
                tftp_root.display()
            )
        })?
        .to_path_buf();
    let relative_path = relative_dir.join(file_name);
    let prepared = LinuxTftpPrepared {
        tftp_root: tftp_root.to_path_buf(),
        target_dir,
        absolute_fit_path: tftp_root.join(&relative_path),
        relative_filename: relative_path.to_string_lossy().replace('\\', "/"),
    };
    if let Err(err) = fs::copy(fitimage, &prepared.absolute_fit_path) {
        let copy_err = anyhow!(err).context(format!("failed to copy file {}", fitimage.display()));
        if let Err(rollback_err) = cleanup_linux_tftp_staging(&prepared) {
            return Err(copy_err.context(format!(
                "failed to roll back TFTP staging directory {}: {rollback_err:#}",
                prepared.target_dir.display()
            )));
        }
        return Err(copy_err);
    }
    Ok(prepared)
}

pub fn cleanup_linux_tftp_staging(prepared: &LinuxTftpPrepared) -> anyhow::Result<()> {
    validate_linux_tftp_prepared(prepared)?;

    match fs::remove_file(&prepared.absolute_fit_path) {
        Ok(()) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_path(
                "failed to remove staged TFTP file",
                &prepared.absolute_fit_path,
            );
        }
    }

    handle_tftp_staging_directory_removal(
        &prepared.target_dir,
        fs::remove_dir(&prepared.target_dir),
    )
}

fn handle_tftp_staging_directory_removal(
    target_dir: &Path,
    result: io::Result<()>,
) -> anyhow::Result<()> {
    match result {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => Err(err).context(format!(
            "failed to remove TFTP staging directory {}; configure group write access on its parent directory",
            target_dir.display()
        )),
        Err(err) => Err(err).with_path("failed to remove TFTP staging directory", target_dir),
    }
}

/// Starts a built-in TFTP server serving files from the build output directory.
pub fn run_tftp_server(manifest_dir: &Path, artifacts: &OutputArtifacts) -> anyhow::Result<()> {
    let mut file_dir = manifest_dir.to_path_buf();
    if let Some(elf_path) = artifacts.elf() {
        file_dir = elf_path
            .parent()
            .ok_or(anyhow!("{} no parent dir", elf_path.display()))?
            .to_path_buf();
    }

    info!(
        "Starting TFTP server serving files from: {}",
        file_dir.display()
    );

    let mut config = Config::default();
    config.directory = file_dir;
    config.send_directory = config.directory.clone();
    config.port = 69;
    config.ip_address = IpAddr::V4(Ipv4Addr::UNSPECIFIED);

    std::thread::spawn(move || {
        let mut server = Server::new(&config)
            .inspect_err(|e| {
                println!("{e}");
                println!("{}", "TFTP server 启动失败：若权限不足，尝试执行 `sudo setcap cap_net_bind_service=+eip $(which cargo-osrun)&&sudo setcap cap_net_bind_service=+eip $(which ostool)` 并重启终端".red());
                std::process::exit(1);
            })
            .unwrap();
        server.listen();
    });

    Ok(())
}

fn find_tftpd_binary() -> Option<PathBuf> {
    find_command_path("in.tftpd").or_else(|| {
        [
            "/usr/sbin/in.tftpd",
            "/sbin/in.tftpd",
            "/usr/bin/in.tftpd",
            "/usr/sbin/tftpd",
        ]
        .into_iter()
        .map(PathBuf::from)
        .find(|path| path.exists())
    })
}

fn find_command_path(program: &str) -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join(program))
        .find(|candidate| candidate.is_file())
}

fn command_exists(program: &str) -> bool {
    find_command_path(program).is_some()
}

fn prompt_yes_no(prompt: &str) -> anyhow::Result<bool> {
    print!("{prompt}");
    io::stdout().flush().context("failed to flush stdout")?;
    let mut answer = String::new();
    io::stdin()
        .read_line(&mut answer)
        .context("failed to read user input")?;
    let answer = answer.trim().to_ascii_lowercase();
    Ok(matches!(answer.as_str(), "y" | "yes"))
}

fn detect_distro_kind() -> anyhow::Result<DistroKind> {
    let os_release = fs::read_to_string("/etc/os-release")
        .context("failed to read /etc/os-release for distro detection")?;
    Ok(DistroKind::from_os_release(&os_release))
}

fn build_install_plan(distro: DistroKind) -> anyhow::Result<InstallPlan> {
    let commands = match distro {
        DistroKind::Debian => vec![
            CommandSpec {
                program: "apt-get".into(),
                args: vec!["update".into()],
            },
            CommandSpec {
                program: "apt-get".into(),
                args: vec!["install".into(), "-y".into(), "tftpd-hpa".into()],
            },
        ],
        DistroKind::Rhel => {
            let package_manager = if command_exists("dnf") { "dnf" } else { "yum" };
            vec![CommandSpec {
                program: package_manager.into(),
                args: vec!["install".into(), "-y".into(), "tftp-server".into()],
            }]
        }
        DistroKind::Arch => vec![CommandSpec {
            program: "pacman".into(),
            args: vec!["-Sy".into(), "--noconfirm".into(), "tftp-hpa".into()],
        }],
        DistroKind::OpenSuse => vec![CommandSpec {
            program: "zypper".into(),
            args: vec!["install".into(), "-y".into(), "tftp".into()],
        }],
        DistroKind::Alpine => vec![CommandSpec {
            program: "apk".into(),
            args: vec!["add".into(), "tftp-hpa".into()],
        }],
        DistroKind::Unsupported => vec![],
    };

    Ok(InstallPlan { distro, commands })
}

fn render_command_chain(commands: &[CommandSpec], is_root: bool) -> String {
    commands
        .iter()
        .map(|command| render_command(command, is_root))
        .collect::<Vec<_>>()
        .join(" && ")
}

fn render_command(command: &CommandSpec, is_root: bool) -> String {
    let mut parts = Vec::with_capacity(command.args.len() + 2);
    if !is_root {
        parts.push("sudo".to_string());
    }
    parts.push(command.program.clone());
    parts.extend(command.args.clone());
    parts.join(" ")
}

fn run_privileged_command(command: &CommandSpec, is_root: bool) -> anyhow::Result<()> {
    eprintln!("{}", render_command(command, is_root).purple());
    let mut process = if is_root {
        let mut process = Command::new(&command.program);
        process.args(&command.args);
        process
    } else {
        let mut process = Command::new("sudo");
        process.arg(&command.program).args(&command.args);
        process
    };

    let status = process
        .stdin(Stdio::inherit())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .with_context(|| format!("failed to start `{}`", command.program))?;

    if status.success() {
        Ok(())
    } else {
        bail!("command `{}` exited with status {status}", command.program)
    }
}

fn run_capture(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute `{program}`"))?;

    if !output.status.success() {
        bail!("command `{program}` exited with status {}", output.status);
    }

    let text = String::from_utf8(output.stdout)
        .with_context(|| format!("failed to decode output from `{program}`"))?;
    Ok(text.trim().to_string())
}

fn is_root_user() -> anyhow::Result<bool> {
    Ok(run_capture("id", &["-u"])? == "0")
}

fn ensure_tftpd_hpa_service_ready(is_root: bool) -> anyhow::Result<()> {
    if udp_port_69_is_listening()? {
        info!("tftpd-hpa is already listening on UDP port 69");
        return Ok(());
    }

    if command_exists("systemctl") {
        println!(
            "{}",
            "tftpd-hpa 当前未监听 UDP 69，正在尝试启动/重启服务".yellow()
        );
        let restart = CommandSpec {
            program: "systemctl".into(),
            args: vec!["restart".into(), "tftpd-hpa".into()],
        };
        run_privileged_command(&restart, is_root).context("failed to restart tftpd-hpa service")?;

        if udp_port_69_is_listening()? {
            info!("tftpd-hpa is now listening on UDP port 69");
            return Ok(());
        }

        let active = run_capture("systemctl", &["is-active", "tftpd-hpa"])
            .unwrap_or_else(|_| "unknown".to_string());
        bail!("tftpd-hpa 服务重启后仍未监听 UDP 69（systemctl is-active: {active}）");
    }

    bail!("未检测到可用的服务管理器，且 tftpd-hpa 当前未监听 UDP 69，请手动启动服务");
}

fn udp_port_69_is_listening() -> anyhow::Result<bool> {
    let output = run_capture("ss", &["-lun"])?;
    Ok(ss_output_has_udp_port_69(&output))
}

fn ss_output_has_udp_port_69(output: &str) -> bool {
    output.lines().any(|line| {
        let line = line.trim();
        !line.is_empty()
            && !line.starts_with("State")
            && line.split_whitespace().any(|field| {
                field.ends_with(":69")
                    || field.ends_with(":69,")
                    || field.ends_with("]:69")
                    || field == "*:69"
                    || field == "0.0.0.0:69"
                    || field == "[::]:69"
            })
    })
}

fn ensure_tftpd_hpa_config(path: &Path, is_root: bool) -> anyhow::Result<(TftpdHpaConfig, bool)> {
    if path.exists() {
        let content = fs::read_to_string(path).with_path("failed to read file", path)?;
        let config = TftpdHpaConfig::parse(&content)?;
        return Ok((config, false));
    }

    let content = TftpdHpaConfig::render_default();
    write_root_owned_file(path, &content, is_root)?;
    let config = TftpdHpaConfig::parse(&content)?;
    Ok((config, true))
}

fn write_root_owned_file(path: &Path, content: &str, is_root: bool) -> anyhow::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| anyhow!("path {} has no parent directory", path.display()))?;

    if is_root {
        fs::create_dir_all(parent).with_path("failed to create directory", parent)?;
        fs::write(path, content).with_path("failed to write file", path)?;
        return Ok(());
    }

    let temp_path = temp_file_path("ostool-tftpd-hpa");
    fs::write(&temp_path, content).with_path("failed to write temp file", &temp_path)?;

    let mkdir = CommandSpec {
        program: "mkdir".into(),
        args: vec!["-p".into(), parent.display().to_string()],
    };
    let copy = CommandSpec {
        program: "cp".into(),
        args: vec![temp_path.display().to_string(), path.display().to_string()],
    };

    let mkdir_result = run_privileged_command(&mkdir, false);
    let copy_result = run_privileged_command(&copy, false);
    let _ = fs::remove_file(&temp_path);

    mkdir_result?;
    copy_result?;
    Ok(())
}

fn temp_file_path(prefix: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    env::temp_dir().join(format!("{prefix}-{}-{nanos}.tmp", std::process::id()))
}

fn validate_tftp_root(tftp_root: &Path) -> anyhow::Result<()> {
    if !tftp_root.is_absolute() {
        bail!("TFTP root must be absolute: {}", tftp_root.display());
    }
    if tftp_root
        .components()
        .any(|component| component == Component::ParentDir)
    {
        bail!(
            "TFTP root must not contain parent segments: {}",
            tftp_root.display()
        );
    }
    Ok(())
}

fn validate_linux_tftp_prepared(prepared: &LinuxTftpPrepared) -> anyhow::Result<()> {
    validate_tftp_root(&prepared.tftp_root)?;
    let namespace_parent = prepared.tftp_root.join(TFTP_NAMESPACE);
    validate_tftp_namespace_parent(&namespace_parent)?;
    if prepared.target_dir.parent() != Some(namespace_parent.as_path()) {
        bail!(
            "invalid TFTP staging directory outside generated namespace: {}",
            prepared.target_dir.display()
        );
    }
    let leaf = prepared
        .target_dir
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            anyhow!(
                "invalid TFTP staging directory name: {}",
                prepared.target_dir.display()
            )
        })?;
    if !is_generated_namespace_leaf(leaf) {
        bail!("invalid generated TFTP staging directory name: {leaf}");
    }
    if prepared.absolute_fit_path.parent() != Some(prepared.target_dir.as_path()) {
        bail!(
            "staged TFTP file is outside its generated directory: {}",
            prepared.absolute_fit_path.display()
        );
    }
    let expected_relative = PathBuf::from(TFTP_NAMESPACE)
        .join(leaf)
        .join(
            prepared
                .absolute_fit_path
                .file_name()
                .ok_or_else(|| anyhow!("staged TFTP file has no filename"))?,
        )
        .to_string_lossy()
        .replace('\\', "/");
    if prepared.relative_filename != expected_relative {
        bail!(
            "staged TFTP relative filename does not match generated path: {}",
            prepared.relative_filename
        );
    }
    match fs::symlink_metadata(&prepared.target_dir) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!(
            "refusing to clean symlinked TFTP staging directory: {}",
            prepared.target_dir.display()
        ),
        Ok(metadata) if !metadata.is_dir() => bail!(
            "TFTP staging target is not a directory: {}",
            prepared.target_dir.display()
        ),
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => {
            return Err(err).with_path(
                "failed to inspect TFTP staging directory",
                &prepared.target_dir,
            );
        }
    }
    Ok(())
}

fn validate_tftp_namespace_parent(namespace_parent: &Path) -> anyhow::Result<()> {
    match fs::symlink_metadata(namespace_parent) {
        Ok(metadata) if metadata.file_type().is_symlink() => bail!(
            "refusing to use symlinked TFTP namespace parent: {}",
            namespace_parent.display()
        ),
        Ok(metadata) if !metadata.is_dir() => bail!(
            "TFTP namespace parent is not a directory: {}",
            namespace_parent.display()
        ),
        Ok(_) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(err) => Err(err).with_path("failed to inspect TFTP namespace parent", namespace_parent),
    }
}

fn is_generated_namespace_leaf(leaf: &str) -> bool {
    let mut parts = leaf.split('-');
    let Some(pid) = parts.next() else {
        return false;
    };
    let Some(timestamp) = parts.next() else {
        return false;
    };
    let Some(sequence) = parts.next() else {
        return false;
    };
    parts.next().is_none()
        && !pid.is_empty()
        && pid.bytes().all(|byte| byte.is_ascii_digit())
        && !timestamp.is_empty()
        && timestamp.bytes().all(|byte| byte.is_ascii_hexdigit())
        && !sequence.is_empty()
        && sequence.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn staging_namespace_candidate() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let sequence = STAGING_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    format!("{}-{nanos:x}-{sequence:x}", std::process::id())
}

fn claim_tftp_namespace(tftp_root: &Path) -> anyhow::Result<PathBuf> {
    claim_tftp_namespace_with(
        tftp_root,
        std::iter::repeat_with(staging_namespace_candidate),
        |command| run_privileged_command(command, false),
    )
}

fn claim_tftp_namespace_with<I, F>(
    tftp_root: &Path,
    candidates: I,
    mut run_privileged: F,
) -> anyhow::Result<PathBuf>
where
    I: IntoIterator<Item = String>,
    F: FnMut(&CommandSpec) -> anyhow::Result<()>,
{
    validate_tftp_root(tftp_root)?;
    let namespace_parent = tftp_root.join(TFTP_NAMESPACE);
    ensure_tftp_namespace_parent(&namespace_parent, &mut run_privileged)?;

    for candidate in candidates {
        validate_tftp_namespace_parent(&namespace_parent)?;
        let target_dir = namespace_parent.join(candidate);
        let outcome = match fs::create_dir(&target_dir) {
            Ok(()) => LeafClaimOutcome::Claimed,
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => LeafClaimOutcome::Collision,
            Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
                claim_privileged_leaf_with_user(&target_dir, &mut run_privileged, effective_user)?
            }
            Err(err) => {
                return Err(err).with_path("failed to claim TFTP staging directory", &target_dir);
            }
        };
        if outcome == LeafClaimOutcome::Collision {
            continue;
        }
        return Ok(target_dir);
    }

    bail!("unable to claim a unique TFTP staging directory")
}

fn ensure_tftp_namespace_parent<F>(
    namespace_parent: &Path,
    run_privileged: &mut F,
) -> anyhow::Result<()>
where
    F: FnMut(&CommandSpec) -> anyhow::Result<()>,
{
    validate_tftp_namespace_parent(namespace_parent)?;
    match fs::create_dir_all(namespace_parent) {
        Ok(()) => return validate_tftp_namespace_parent(namespace_parent),
        Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {}
        Err(err) => return Err(err).with_path("failed to create directory", namespace_parent),
    }

    let mkdir = CommandSpec {
        program: "mkdir".into(),
        args: vec!["-p".into(), namespace_parent.display().to_string()],
    };
    run_privileged(&mkdir).with_context(|| {
        format!(
            "failed to create TFTP namespace parent {}",
            namespace_parent.display()
        )
    })?;
    validate_tftp_namespace_parent(namespace_parent)
}

fn claim_privileged_leaf_with_user<F, U>(
    target_dir: &Path,
    run_privileged: &mut F,
    effective_user_lookup: U,
) -> anyhow::Result<LeafClaimOutcome>
where
    F: FnMut(&CommandSpec) -> anyhow::Result<()>,
    U: FnOnce() -> anyhow::Result<EffectiveUser>,
{
    let user = effective_user_lookup().with_context(|| {
        format!(
            "failed to resolve effective user before claiming {}",
            target_dir.display()
        )
    })?;
    let outcome = classify_privileged_leaf_claim(target_dir, &mut *run_privileged)?;
    if outcome == LeafClaimOutcome::Collision {
        return Ok(outcome);
    }

    let chown = CommandSpec {
        program: "chown".into(),
        args: vec![
            format!("{}:{}", user.name, user.group),
            target_dir.display().to_string(),
        ],
    };
    if let Err(err) = run_privileged(&chown) {
        let primary = err.context(format!(
            "failed to change ownership for {}",
            target_dir.display()
        ));
        let rmdir = CommandSpec {
            program: "rmdir".into(),
            args: vec![target_dir.display().to_string()],
        };
        return match run_privileged(&rmdir) {
            Ok(()) => Err(primary),
            Err(rollback_err) => Err(primary.context(format!(
                "failed to roll back privileged TFTP directory {}: {rollback_err:#}",
                target_dir.display()
            ))),
        };
    }
    Ok(LeafClaimOutcome::Claimed)
}

fn classify_privileged_leaf_claim<F>(
    target_dir: &Path,
    mut run_privileged: F,
) -> anyhow::Result<LeafClaimOutcome>
where
    F: FnMut(&CommandSpec) -> anyhow::Result<()>,
{
    let mkdir = CommandSpec {
        program: "mkdir".into(),
        args: vec![target_dir.display().to_string()],
    };
    match run_privileged(&mkdir) {
        Ok(()) => Ok(LeafClaimOutcome::Claimed),
        Err(err) => match fs::symlink_metadata(target_dir) {
            Ok(_) => Ok(LeafClaimOutcome::Collision),
            Err(metadata_err) if metadata_err.kind() == io::ErrorKind::NotFound => Err(err),
            Err(metadata_err) => {
                Err(metadata_err).with_path("failed to inspect TFTP staging directory", target_dir)
            }
        },
    }
}

fn effective_user() -> anyhow::Result<EffectiveUser> {
    let name = env::var("SUDO_USER")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .or_else(|| {
            env::var("USER")
                .ok()
                .filter(|value| !value.trim().is_empty())
        })
        .unwrap_or(run_capture("id", &["-un"])?);

    let group = run_capture("id", &["-gn", &name])?;
    Ok(EffectiveUser { name, group })
}

impl TftpdHpaConfig {
    fn parse(content: &str) -> anyhow::Result<Self> {
        let mut username = None;
        let mut directory = None;
        let mut address = None;
        let mut options = None;

        for line in content.lines() {
            let Some((key, value)) = parse_key_value(line) else {
                continue;
            };
            match key {
                "TFTP_USERNAME" => username = Some(value),
                "TFTP_DIRECTORY" => directory = Some(value),
                "TFTP_ADDRESS" => address = Some(value),
                "TFTP_OPTIONS" => options = Some(value),
                _ => {}
            }
        }

        let directory = directory
            .filter(|value| !value.trim().is_empty())
            .ok_or_else(|| anyhow!("tftpd-hpa config is missing TFTP_DIRECTORY"))?;

        Ok(Self {
            username,
            directory: PathBuf::from(directory),
            address,
            options,
        })
    }

    fn render_default() -> String {
        format!(
            "TFTP_USERNAME=\"tftp\"\nTFTP_DIRECTORY=\"{DEFAULT_TFTP_DIRECTORY}\"\nTFTP_ADDRESS=\":69\"\nTFTP_OPTIONS=\"-l -s -c\"\n"
        )
    }
}

fn parse_key_value(line: &str) -> Option<(&str, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let (key, value) = trimmed.split_once('=')?;
    Some((key.trim(), unquote(value.trim())))
}

fn unquote(value: &str) -> String {
    let mut chars = value.chars();
    match (chars.next(), value.chars().last()) {
        (Some('"'), Some('"')) | (Some('\''), Some('\'')) if value.len() >= 2 => {
            value[1..value.len() - 1].to_string()
        }
        _ => value.to_string(),
    }
}

impl DistroKind {
    fn from_os_release(content: &str) -> Self {
        let mut ids = Vec::new();

        for line in content.lines() {
            let Some((key, value)) = parse_key_value(line) else {
                continue;
            };
            match key {
                "ID" => ids.push(value),
                "ID_LIKE" => ids.extend(value.split_whitespace().map(ToOwned::to_owned)),
                _ => {}
            }
        }

        if ids
            .iter()
            .any(|id| matches!(id.as_str(), "debian" | "ubuntu"))
        {
            return Self::Debian;
        }
        if ids.iter().any(|id| {
            matches!(
                id.as_str(),
                "fedora" | "rhel" | "centos" | "rocky" | "almalinux"
            )
        }) {
            return Self::Rhel;
        }
        if ids
            .iter()
            .any(|id| matches!(id.as_str(), "arch" | "archlinux" | "manjaro"))
        {
            return Self::Arch;
        }
        if ids.iter().any(|id| {
            matches!(
                id.as_str(),
                "opensuse" | "opensuse-tumbleweed" | "sles" | "suse"
            )
        }) {
            return Self::OpenSuse;
        }
        if ids.iter().any(|id| id == "alpine") {
            return Self::Alpine;
        }

        Self::Unsupported
    }

    fn label(self) -> &'static str {
        match self {
            Self::Debian => "debian/ubuntu",
            Self::Rhel => "rhel/fedora",
            Self::Arch => "arch",
            Self::OpenSuse => "opensuse/sles",
            Self::Alpine => "alpine",
            Self::Unsupported => "unsupported",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn prepared_record(tftp_root: &Path, leaf: &str) -> LinuxTftpPrepared {
        let target_dir = tftp_root.join(TFTP_NAMESPACE).join(leaf);
        let absolute_fit_path = target_dir.join("image.fit");
        LinuxTftpPrepared {
            tftp_root: tftp_root.to_path_buf(),
            target_dir,
            absolute_fit_path,
            relative_filename: format!("{TFTP_NAMESPACE}/{leaf}/image.fit"),
        }
    }

    #[test]
    fn stage_linux_fit_image_rejects_relative_tftp_root() {
        let err = validate_tftp_root(Path::new("relative-tftp-root")).unwrap_err();

        assert!(err.to_string().contains("TFTP root must be absolute"));
    }

    #[test]
    fn stage_linux_fit_image_rejects_parent_segments_in_tftp_root() {
        let err = validate_tftp_root(Path::new("/tmp/tftp/../escape")).unwrap_err();

        assert!(err.to_string().contains("must not contain parent segments"));
    }

    #[test]
    fn namespace_claim_retries_collision() {
        let temp = tempfile::tempdir().unwrap();
        let namespace_parent = temp.path().join("ostool");
        fs::create_dir_all(namespace_parent.join("123-abc-0")).unwrap();

        let claimed = claim_tftp_namespace_with(
            temp.path(),
            ["123-abc-0".to_string(), "123-abc-1".to_string()],
            |_| Ok(()),
        )
        .unwrap();

        assert_eq!(claimed, namespace_parent.join("123-abc-1"));
        assert!(namespace_parent.join("123-abc-0").is_dir());
    }

    #[test]
    fn namespace_claim_classifies_privileged_collision() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("ostool").join("123-abc-0");
        fs::create_dir_all(target.parent().unwrap()).unwrap();

        let outcome = classify_privileged_leaf_claim(&target, |command| {
            assert_eq!(command.program, "mkdir");
            assert_eq!(command.args, vec![target.display().to_string()]);
            fs::create_dir(&target).unwrap();
            Err(anyhow!("simulated mkdir collision"))
        })
        .unwrap();

        assert_eq!(outcome, LeafClaimOutcome::Collision);
    }

    #[test]
    fn namespace_claim_preserves_privileged_failure_without_leaf() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("ostool").join("123-abc-0");
        fs::create_dir_all(target.parent().unwrap()).unwrap();

        let err =
            classify_privileged_leaf_claim(&target, |_| Err(anyhow!("simulated mkdir failure")))
                .unwrap_err();

        assert!(err.to_string().contains("simulated mkdir failure"));
        assert!(!target.exists());
    }

    #[test]
    fn namespace_claim_user_lookup_failure_happens_before_privileged_mkdir() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("ostool").join("123-abc-0");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        let mut command_calls = 0;

        let err = claim_privileged_leaf_with_user(
            &target,
            &mut |_| {
                command_calls += 1;
                Ok(())
            },
            || -> anyhow::Result<EffectiveUser> { Err(anyhow!("simulated user lookup failure")) },
        )
        .unwrap_err();

        assert!(format!("{err:#}").contains("simulated user lookup failure"));
        assert_eq!(command_calls, 0);
        assert!(!target.exists());
    }

    #[test]
    fn namespace_claim_reports_chown_and_rollback_failures() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("ostool").join("123-abc-0");
        fs::create_dir_all(target.parent().unwrap()).unwrap();

        let err = claim_privileged_leaf_with_user(
            &target,
            &mut |command| match command.program.as_str() {
                "mkdir" => {
                    fs::create_dir(&target).unwrap();
                    Ok(())
                }
                "chown" => Err(anyhow!("simulated chown failure")),
                "rmdir" => Err(anyhow!("simulated rollback failure")),
                other => panic!("unexpected command: {other}"),
            },
            || {
                Ok(EffectiveUser {
                    name: "tester".into(),
                    group: "tester".into(),
                })
            },
        )
        .unwrap_err();
        let message = format!("{err:#}");

        assert!(message.contains("simulated chown failure"));
        assert!(message.contains("simulated rollback failure"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn staging_copy_failure_rolls_back_claimed_leaf() {
        let temp = tempfile::tempdir().unwrap();
        let source_directory = temp.path().join("source.fit");
        fs::create_dir(&source_directory).unwrap();
        let tftp_root = temp.path().join("tftp");

        stage_linux_fit_image(&source_directory, &tftp_root).unwrap_err();

        let namespace_parent = tftp_root.join(TFTP_NAMESPACE);
        assert_eq!(fs::read_dir(namespace_parent).unwrap().count(), 0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn staging_invalid_fit_filename_does_not_claim_leaf() {
        let temp = tempfile::tempdir().unwrap();
        let tftp_root = temp.path().join("tftp");

        stage_linux_fit_image(Path::new("/"), &tftp_root).unwrap_err();

        let namespace_parent = tftp_root.join(TFTP_NAMESPACE);
        assert!(!namespace_parent.exists());
    }

    #[cfg(all(target_os = "linux", unix))]
    #[test]
    fn staging_rejects_symlinked_namespace_parent() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let fitimage = temp.path().join("image.fit");
        fs::write(&fitimage, [1_u8]).unwrap();
        let tftp_root = temp.path().join("tftp");
        let outside = temp.path().join("outside");
        fs::create_dir_all(&tftp_root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        symlink(&outside, tftp_root.join(TFTP_NAMESPACE)).unwrap();

        let err = stage_linux_fit_image(&fitimage, &tftp_root).unwrap_err();

        assert!(err.to_string().contains("symlinked TFTP namespace parent"));
        assert_eq!(fs::read_dir(outside).unwrap().count(), 0);
    }

    #[test]
    fn cleanup_linux_tftp_removes_exact_file_and_leaf() {
        let temp = tempfile::tempdir().unwrap();
        let prepared = prepared_record(temp.path(), "123-abc-0");
        fs::create_dir_all(&prepared.target_dir).unwrap();
        fs::write(&prepared.absolute_fit_path, [1_u8, 2, 3]).unwrap();

        cleanup_linux_tftp_staging(&prepared).unwrap();

        assert!(!prepared.absolute_fit_path.exists());
        assert!(!prepared.target_dir.exists());
        assert!(temp.path().join(TFTP_NAMESPACE).is_dir());
    }

    #[test]
    fn cleanup_linux_tftp_accepts_already_absent_record() {
        let temp = tempfile::tempdir().unwrap();
        let prepared = prepared_record(temp.path(), "123-abc-0");

        cleanup_linux_tftp_staging(&prepared).unwrap();
    }

    #[test]
    fn cleanup_linux_tftp_does_not_remove_unexpected_files() {
        let temp = tempfile::tempdir().unwrap();
        let prepared = prepared_record(temp.path(), "123-abc-0");
        fs::create_dir_all(&prepared.target_dir).unwrap();
        fs::write(&prepared.absolute_fit_path, [1_u8]).unwrap();
        let unexpected = prepared.target_dir.join("keep.txt");
        fs::write(&unexpected, b"keep").unwrap();

        let err = cleanup_linux_tftp_staging(&prepared).unwrap_err();

        assert!(
            err.to_string()
                .contains("failed to remove TFTP staging directory")
        );
        assert!(unexpected.exists());
        assert!(!prepared.absolute_fit_path.exists());
    }

    #[test]
    fn cleanup_linux_tftp_rejects_fabricated_targets() {
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let invalid_leaf = prepared_record(root, "user-data");
        assert!(cleanup_linux_tftp_staging(&invalid_leaf).is_err());

        let shared_parent = LinuxTftpPrepared {
            tftp_root: root.to_path_buf(),
            target_dir: root.join(TFTP_NAMESPACE),
            absolute_fit_path: root.join(TFTP_NAMESPACE).join("image.fit"),
            relative_filename: format!("{TFTP_NAMESPACE}/image.fit"),
        };
        assert!(cleanup_linux_tftp_staging(&shared_parent).is_err());

        let nested = LinuxTftpPrepared {
            tftp_root: root.to_path_buf(),
            target_dir: root.join(TFTP_NAMESPACE).join("123-abc-0").join("nested"),
            absolute_fit_path: root
                .join(TFTP_NAMESPACE)
                .join("123-abc-0")
                .join("nested/image.fit"),
            relative_filename: format!("{TFTP_NAMESPACE}/123-abc-0/nested/image.fit"),
        };
        assert!(cleanup_linux_tftp_staging(&nested).is_err());
    }

    #[test]
    fn cleanup_linux_tftp_rejects_invalid_roots() {
        let relative = prepared_record(Path::new("relative-root"), "123-abc-0");
        assert!(cleanup_linux_tftp_staging(&relative).is_err());

        let parent = prepared_record(Path::new("/tmp/tftp/../escape"), "123-abc-0");
        assert!(cleanup_linux_tftp_staging(&parent).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_linux_tftp_rejects_symlinked_namespace_parent() {
        use std::os::unix::fs::symlink;

        let temp = tempfile::tempdir().unwrap();
        let outside = temp.path().join("outside");
        let outside_leaf = outside.join("123-abc-0");
        fs::create_dir_all(&outside_leaf).unwrap();
        let outside_file = outside_leaf.join("image.fit");
        fs::write(&outside_file, [1_u8]).unwrap();
        symlink(&outside, temp.path().join(TFTP_NAMESPACE)).unwrap();
        let prepared = prepared_record(temp.path(), "123-abc-0");

        let err = cleanup_linux_tftp_staging(&prepared).unwrap_err();

        assert!(err.to_string().contains("symlinked TFTP namespace parent"));
        assert!(outside_file.exists());
    }

    #[test]
    fn cleanup_linux_tftp_permission_denied_requests_group_write_access() {
        let temp = tempfile::tempdir().unwrap();
        let prepared = prepared_record(temp.path(), "123-abc-0");

        let err = handle_tftp_staging_directory_removal(
            &prepared.target_dir,
            Err(io::Error::from(io::ErrorKind::PermissionDenied)),
        )
        .unwrap_err();

        let message = err.to_string();
        assert!(message.contains("failed to remove TFTP staging directory"));
        assert!(message.contains("group write access"));
    }

    #[test]
    fn distro_detection_uses_id_and_id_like() {
        let ubuntu = r#"
ID=ubuntu
ID_LIKE=debian
"#;
        let rocky = r#"
ID=rocky
ID_LIKE="rhel fedora"
"#;
        let arch = "ID=manjaro\nID_LIKE=arch\n";

        assert_eq!(DistroKind::from_os_release(ubuntu), DistroKind::Debian);
        assert_eq!(DistroKind::from_os_release(rocky), DistroKind::Rhel);
        assert_eq!(DistroKind::from_os_release(arch), DistroKind::Arch);
    }

    #[test]
    fn render_command_chain_adds_sudo_for_non_root() {
        let commands = vec![
            CommandSpec {
                program: "apt-get".into(),
                args: vec!["update".into()],
            },
            CommandSpec {
                program: "apt-get".into(),
                args: vec!["install".into(), "-y".into(), "tftpd-hpa".into()],
            },
        ];

        assert_eq!(
            render_command_chain(&commands, false),
            "sudo apt-get update && sudo apt-get install -y tftpd-hpa"
        );
        assert_eq!(
            render_command_chain(&commands, true),
            "apt-get update && apt-get install -y tftpd-hpa"
        );
    }

    #[test]
    fn default_tftpd_hpa_config_matches_plan() {
        assert_eq!(
            TftpdHpaConfig::render_default(),
            "TFTP_USERNAME=\"tftp\"\nTFTP_DIRECTORY=\"/srv/tftp\"\nTFTP_ADDRESS=\":69\"\nTFTP_OPTIONS=\"-l -s -c\"\n"
        );
    }

    #[test]
    fn parse_existing_tftpd_hpa_directory() {
        let config = TftpdHpaConfig::parse(
            r#"
TFTP_USERNAME="tftp"
TFTP_DIRECTORY="/mnt/d/tftpboot/"
TFTP_ADDRESS=":69"
TFTP_OPTIONS="-l -s -c"
"#,
        )
        .unwrap();

        assert_eq!(config.directory, PathBuf::from("/mnt/d/tftpboot/"));
        assert_eq!(config.options.as_deref(), Some("-l -s -c"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn staged_filename_uses_short_unique_namespace() {
        let temp = tempfile::tempdir().unwrap();
        let fitimage = temp.path().join("build/image.fit");
        fs::create_dir_all(fitimage.parent().unwrap()).unwrap();
        fs::write(&fitimage, [1_u8, 2, 3]).unwrap();
        let tftp_root = temp.path().join("tftp");
        let prepared = stage_linux_fit_image(&fitimage, &tftp_root).unwrap();

        assert!(prepared.relative_filename.starts_with("ostool/"));
        assert!(prepared.relative_filename.ends_with("/image.fit"));
        assert!(prepared.relative_filename.len() < 96);
        assert_eq!(
            prepared.absolute_fit_path,
            tftp_root.join(&prepared.relative_filename)
        );
        assert_eq!(
            prepared.target_dir.parent(),
            Some(tftp_root.join("ostool").as_path())
        );
    }

    #[test]
    fn generated_namespace_candidates_are_distinct() {
        assert_ne!(staging_namespace_candidate(), staging_namespace_candidate());
    }

    #[test]
    fn ss_port_detection_matches_udp_69_listener() {
        let output = "\
State  Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\n\
UNCONN 0      0      0.0.0.0:69      0.0.0.0:*\n";

        assert!(ss_output_has_udp_port_69(output));
        assert!(!ss_output_has_udp_port_69(
            "State Recv-Q Send-Q Local Address:Port Peer Address:PortProcess\n"
        ));
    }
}
