use std::{
    env::current_dir,
    net::{Ipv4Addr, SocketAddr},
    path::{Path, PathBuf},
};

use anyhow::{Context, bail};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::fs;

const DEFAULT_SYSTEM_TFTP_ROOT: &str = "/srv/tftp";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServerConfig {
    pub listen_addr: SocketAddr,
    pub data_dir: PathBuf,
    pub board_dir: PathBuf,
    pub lease: LeaseConfig,
    pub tftp: TftpConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        let data_dir = PathBuf::from(".ostool-server");
        let board_dir = data_dir.join("boards");

        #[cfg(target_os = "linux")]
        let tftp = TftpConfig::SystemTftpdHpa(SystemTftpdHpaConfig::default());

        #[cfg(not(target_os = "linux"))]
        let tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(
            data_dir.join("tftp-root"),
        ));

        Self {
            listen_addr: SocketAddr::from(([127, 0, 0, 1], 8080)),
            data_dir,
            board_dir,
            lease: LeaseConfig::default(),
            tftp,
        }
    }
}

impl ServerConfig {
    pub async fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        match fs::read_to_string(path).await {
            Ok(content) => {
                let mut config: Self = toml::from_str(&content)
                    .with_context(|| format!("failed to parse {}", path.display()))?;
                config.normalize_paths(path)?;
                config.sync_system_tftpd_hpa_config()?;
                config.validate()?;
                Ok(config)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let mut config = Self::default();
                config.normalize_paths(path)?;
                config.sync_system_tftpd_hpa_config()?;
                config.validate()?;
                if let Some(parent) = path.parent() {
                    fs::create_dir_all(parent).await?;
                }
                fs::write(path, toml::to_string_pretty(&config)?).await?;
                Ok(config)
            }
            Err(err) => Err(err.into()),
        }
    }

    fn sync_system_tftpd_hpa_config(&mut self) -> anyhow::Result<()> {
        let TftpConfig::SystemTftpdHpa(cfg) = &mut self.tftp else {
            return Ok(());
        };

        match parse_tftpd_hpa_file(&cfg.config_path) {
            Ok(Some(existing)) => {
                let existing_dir = if existing.directory.is_absolute() {
                    existing.directory
                } else {
                    PathBuf::from(DEFAULT_SYSTEM_TFTP_ROOT)
                };
                cfg.root_dir = existing_dir;
                if let Some(username) = existing.username {
                    cfg.username = Some(username);
                }
                if let Some(address) = existing.address {
                    cfg.address = address;
                }
                if let Some(options) = existing.options {
                    cfg.options = options;
                }
            }
            Ok(None) => {
                cfg.root_dir = PathBuf::from(DEFAULT_SYSTEM_TFTP_ROOT);
            }
            Err(err) => return Err(err),
        }

        Ok(())
    }

    pub fn normalize_paths(&mut self, config_path: &Path) -> anyhow::Result<()> {
        let config_dir = config_path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or(current_dir()?);

        self.data_dir = absolutize_path(&config_dir, &self.data_dir);
        self.board_dir = absolutize_path(&config_dir, &self.board_dir);

        match &mut self.tftp {
            TftpConfig::Builtin(cfg) => {
                cfg.root_dir = absolutize_path(&config_dir, &cfg.root_dir);
            }
            TftpConfig::SystemTftpdHpa(cfg) => {
                cfg.root_dir = absolutize_path(&config_dir, &cfg.root_dir);
                cfg.config_path = absolutize_path(&config_dir, &cfg.config_path);
            }
        }

        Ok(())
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        if self.lease.default_ttl_secs == 0 {
            bail!("lease.default_ttl_secs must be > 0");
        }
        if self.lease.max_ttl_secs < self.lease.default_ttl_secs {
            bail!("lease.max_ttl_secs must be >= lease.default_ttl_secs");
        }
        if self.lease.gc_interval_secs == 0 {
            bail!("lease.gc_interval_secs must be > 0");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct LeaseConfig {
    pub default_ttl_secs: u64,
    pub max_ttl_secs: u64,
    pub gc_interval_secs: u64,
}

impl Default for LeaseConfig {
    fn default() -> Self {
        Self {
            default_ttl_secs: 900,
            max_ttl_secs: 3600,
            gc_interval_secs: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "provider", rename_all = "snake_case")]
pub enum TftpConfig {
    Builtin(BuiltinTftpConfig),
    SystemTftpdHpa(SystemTftpdHpaConfig),
}

impl TftpConfig {
    pub fn enabled(&self) -> bool {
        match self {
            Self::Builtin(cfg) => cfg.enabled,
            Self::SystemTftpdHpa(cfg) => cfg.enabled,
        }
    }

    pub fn root_dir(&self) -> &Path {
        match self {
            Self::Builtin(cfg) => &cfg.root_dir,
            Self::SystemTftpdHpa(cfg) => &cfg.root_dir,
        }
    }

    pub fn provider_name(&self) -> &'static str {
        match self {
            Self::Builtin(_) => "builtin",
            Self::SystemTftpdHpa(_) => "system_tftpd_hpa",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BuiltinTftpConfig {
    pub enabled: bool,
    pub root_dir: PathBuf,
    pub bind_addr: SocketAddr,
}

impl BuiltinTftpConfig {
    pub fn default_with_root(root_dir: PathBuf) -> Self {
        Self {
            enabled: true,
            root_dir,
            bind_addr: SocketAddr::from((Ipv4Addr::UNSPECIFIED, 69)),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SystemTftpdHpaConfig {
    pub enabled: bool,
    pub root_dir: PathBuf,
    pub config_path: PathBuf,
    pub service_name: String,
    pub username: Option<String>,
    pub address: String,
    pub options: String,
    pub manage_config: bool,
    pub reconcile_on_start: bool,
}

impl SystemTftpdHpaConfig {
    pub fn default() -> Self {
        Self {
            enabled: true,
            root_dir: PathBuf::from(DEFAULT_SYSTEM_TFTP_ROOT),
            config_path: PathBuf::from("/etc/default/tftpd-hpa"),
            service_name: "tftpd-hpa".to_string(),
            username: Some("tftp".to_string()),
            address: ":69".to_string(),
            options: "-l -s -c".to_string(),
            manage_config: true,
            reconcile_on_start: true,
        }
    }
}

#[derive(Debug)]
struct ParsedTftpdHpaConfig {
    username: Option<String>,
    directory: PathBuf,
    address: Option<String>,
    options: Option<String>,
}

fn parse_tftpd_hpa_file(path: &Path) -> anyhow::Result<Option<ParsedTftpdHpaConfig>> {
    if !path.exists() {
        return Ok(None);
    }

    let content = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read {}", path.display()))?;
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
            "TFTP_DIRECTORY" => directory = Some(PathBuf::from(value)),
            "TFTP_ADDRESS" => address = Some(value),
            "TFTP_OPTIONS" => options = Some(value),
            _ => {}
        }
    }

    let directory = directory.unwrap_or_else(|| PathBuf::from(DEFAULT_SYSTEM_TFTP_ROOT));
    Ok(Some(ParsedTftpdHpaConfig {
        username,
        directory,
        address,
        options,
    }))
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

fn absolutize_path(base_dir: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        base_dir.join(path)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BoardConfig {
    pub id: String,
    pub name: String,
    pub board_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub serial: Option<SerialConfig>,
    pub boot: BootConfig,
    pub notes: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SerialConfig {
    pub port: String,
    pub baud_rate: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BootConfig {
    Uboot(UbootProfile),
    Pxe(PxeProfile),
}

impl BootConfig {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Uboot(_) => "uboot",
            Self::Pxe(_) => "pxe",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct UbootProfile {
    pub kernel_load_addr: Option<String>,
    pub fit_load_addr: Option<String>,
    pub net: Option<UbootNetConfig>,
    pub board_reset_cmd: Option<String>,
    pub board_power_off_cmd: Option<String>,
    #[serde(default)]
    pub success_regex: Vec<String>,
    #[serde(default)]
    pub fail_regex: Vec<String>,
    pub uboot_cmd: Option<Vec<String>>,
    pub shell_prefix: Option<String>,
    pub shell_init_cmd: Option<String>,
    pub timeout: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct UbootNetConfig {
    pub interface: String,
    pub board_ip: Option<String>,
    pub gatewayip: Option<String>,
    pub netmask: Option<String>,
    pub server_ip_override: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct PxeProfile {
    pub notes: Option<String>,
}
