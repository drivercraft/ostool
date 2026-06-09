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
const SYSTEM_CONFIG_DIR: &str = "/etc/ostool-server";
const SYSTEM_DATA_DIR: &str = "/var/lib/ostool-server";

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ServerConfig {
    pub listen_addr: SocketAddr,
    pub data_dir: PathBuf,
    pub board_dir: PathBuf,
    pub dtb_dir: PathBuf,
    pub tftp: TftpConfig,
    #[serde(default)]
    pub http_boot: HttpBootConfig,
    pub network: TftpNetworkConfig,
    #[serde(default)]
    pub upload_limits: UploadLimitsConfig,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self::default_for_path(Path::new(".ostool-server.toml"))
    }
}

impl ServerConfig {
    pub fn default_for_path(path: &Path) -> Self {
        let use_system_layout = path.starts_with(SYSTEM_CONFIG_DIR);

        let data_dir = if use_system_layout {
            PathBuf::from(SYSTEM_DATA_DIR)
        } else {
            PathBuf::from(".ostool-server")
        };
        let board_dir = data_dir.join("boards");
        let dtb_dir = data_dir.join("dtbs");
        let http_boot = HttpBootConfig::default_with_root(data_dir.join("http-boot"));

        #[cfg(target_os = "linux")]
        let tftp = TftpConfig::SystemTftpdHpa(SystemTftpdHpaConfig::default());

        #[cfg(not(target_os = "linux"))]
        let tftp = TftpConfig::Builtin(BuiltinTftpConfig::default_with_root(
            data_dir.join("tftp-root"),
        ));

        Self {
            listen_addr: SocketAddr::from(([0, 0, 0, 0], 2999)),
            data_dir,
            board_dir,
            dtb_dir,
            tftp,
            http_boot,
            network: TftpNetworkConfig::default(),
            upload_limits: UploadLimitsConfig::default(),
        }
    }

    pub async fn load_or_create(path: &Path) -> anyhow::Result<Self> {
        match fs::read_to_string(path).await {
            Ok(content) => {
                let mut config: Self = toml::from_str(&content)
                    .with_context(|| format!("failed to parse {}", path.display()))?;
                config.normalize_paths(path)?;
                config.sync_system_tftpd_hpa_config()?;
                config.sync_network_defaults();
                config.validate()?;
                Ok(config)
            }
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                let mut config = Self::default_for_path(path);
                config.normalize_paths(path)?;
                config.sync_system_tftpd_hpa_config()?;
                config.sync_network_defaults();
                config.validate()?;
                config.write_to_path(path).await?;
                Ok(config)
            }
            Err(err) => Err(err.into()),
        }
    }

    pub async fn write_to_path(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .await
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(path, toml::to_string_pretty(self)?)
            .await
            .with_context(|| format!("failed to write {}", path.display()))?;
        Ok(())
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

    fn sync_network_defaults(&mut self) {
        if self.network.interface.trim().is_empty()
            && let Some(interface) = crate::serial::network::default_non_loopback_interface_name()
        {
            self.network.interface = interface;
        }
    }

    pub fn normalize_paths(&mut self, config_path: &Path) -> anyhow::Result<()> {
        let config_dir = config_path
            .parent()
            .filter(|dir| !dir.as_os_str().is_empty())
            .map(PathBuf::from)
            .unwrap_or(current_dir()?);

        self.data_dir = absolutize_path(&config_dir, &self.data_dir);
        self.board_dir = absolutize_path(&config_dir, &self.board_dir);
        self.dtb_dir = absolutize_path(&config_dir, &self.dtb_dir);
        self.http_boot.root_dir = absolutize_path(&config_dir, &self.http_boot.root_dir);

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
        if self.network.interface.trim().is_empty() {
            bail!(
                "network.interface must be configured or auto-detected from a non-loopback interface"
            );
        }
        if self.upload_limits.session_file_max_mib == 0 {
            bail!("upload_limits.session_file_max_mib must be greater than 0");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HttpBootConfig {
    pub enabled: bool,
    pub root_dir: PathBuf,
    pub public_base_url: Option<String>,
    #[serde(default)]
    pub discovery: HttpBootDiscoveryConfig,
}

impl HttpBootConfig {
    pub fn default_with_root(root_dir: PathBuf) -> Self {
        Self {
            enabled: true,
            root_dir,
            public_base_url: None,
            discovery: HttpBootDiscoveryConfig::default(),
        }
    }
}

impl Default for HttpBootConfig {
    fn default() -> Self {
        Self::default_with_root(PathBuf::from(".ostool-server/http-boot"))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct HttpBootDiscoveryConfig {
    pub enabled: bool,
    pub udp_port: u16,
    pub advertise_interval_ms: u64,
}

impl Default for HttpBootDiscoveryConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            udp_port: 2998,
            advertise_interval_ms: 1000,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct UploadLimitsConfig {
    pub session_file_max_mib: u32,
}

impl Default for UploadLimitsConfig {
    fn default() -> Self {
        Self {
            session_file_max_mib: 64,
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

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct TftpNetworkConfig {
    pub interface: String,
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

impl Default for SystemTftpdHpaConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            root_dir: PathBuf::from(DEFAULT_SYSTEM_TFTP_ROOT),
            config_path: PathBuf::from("/etc/default/tftpd-hpa"),
            service_name: "tftpd-hpa".to_string(),
            username: Some("tftp".to_string()),
            address: ":69".to_string(),
            options: "-l -s -c".to_string(),
            manage_config: false,
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
    pub board_type: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub serial: Option<SerialConfig>,
    pub power_management: PowerManagementConfig,
    pub boot: BootConfig,
    pub notes: Option<String>,
    #[serde(default)]
    pub disabled: bool,
}

impl BoardConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        if let BootConfig::Uboot(profile) = &self.boot {
            profile.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct SerialConfig {
    pub key: SerialPortKey,
    pub baud_rate: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_device_path: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resolved_usb_path: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
pub struct SerialPortKey {
    pub kind: SerialPortKeyKind,
    pub value: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SerialPortKeyKind {
    SerialNumber,
    UsbPath,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PowerManagementConfig {
    Custom(CustomPowerManagement),
    ZhongshengRelay(ZhongshengRelayPowerManagement),
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CustomPowerManagement {
    pub power_on_cmd: String,
    pub power_off_cmd: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ZhongshengRelayPowerManagement {
    pub key: SerialPortKey,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum BootConfig {
    Uboot(UbootProfile),
    Pxe(PxeProfile),
    #[serde(rename = "httpboot", alias = "uefi_http")]
    UefiHttp(UefiHttpProfile),
}

impl BootConfig {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::Uboot(_) => "uboot",
            Self::Pxe(_) => "pxe",
            Self::UefiHttp(_) => "httpboot",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
#[serde(deny_unknown_fields)]
pub struct UbootProfile {
    #[serde(default)]
    pub use_tftp: bool,
    pub dtb_name: Option<String>,
    #[serde(default)]
    pub kernel_load_addr: Option<String>,
    #[serde(default)]
    pub fit_load_addr: Option<String>,
    #[serde(default)]
    pub bootm_addr: Option<String>,
    #[serde(default)]
    pub network_mode: UbootNetworkMode,
    #[serde(default)]
    pub board_ip: Option<String>,
    #[serde(default)]
    pub server_ip: Option<String>,
    #[serde(default)]
    pub netmask: Option<String>,
    #[serde(default)]
    pub gatewayip: Option<String>,
}

impl UbootProfile {
    pub fn validate(&self) -> anyhow::Result<()> {
        for (field, value) in [
            ("board_ip", self.board_ip.as_deref()),
            ("server_ip", self.server_ip.as_deref()),
            ("netmask", self.netmask.as_deref()),
            ("gatewayip", self.gatewayip.as_deref()),
        ] {
            if let Some(value) = value {
                ensure_ipv4(field, value)?;
            }
        }

        if self.network_mode == UbootNetworkMode::StaticIp && self.board_ip.is_none() {
            bail!("boot.board_ip must be configured when boot.network_mode is static_ip");
        }

        Ok(())
    }
}

fn ensure_ipv4(field: &str, value: &str) -> anyhow::Result<Ipv4Addr> {
    value
        .parse::<Ipv4Addr>()
        .with_context(|| format!("boot.{field} must be a valid IPv4 address"))
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UbootNetworkMode {
    #[default]
    Dhcp,
    StaticIp,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct PxeProfile {
    pub notes: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UefiHttpStrategy {
    BareBinLoader,
    LoaderDiscovery,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum UefiBootArch {
    X86_64,
    Aarch64,
    Loongarch64,
    Riscv64,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UefiHttpProfile {
    #[serde(default)]
    pub boot_arch: Option<UefiBootArch>,
    #[serde(default)]
    pub strategy: UefiHttpStrategy,
    #[serde(default)]
    pub mac: Option<String>,
}

impl Default for UefiHttpStrategy {
    fn default() -> Self {
        Self::BareBinLoader
    }
}

#[cfg(test)]
mod tests {
    use std::{
        net::SocketAddr,
        path::{Path, PathBuf},
    };

    use serde_json::json;
    use tempfile::tempdir;

    use super::{
        BoardConfig, BootConfig, CustomPowerManagement, PowerManagementConfig, SerialPortKey,
        SerialPortKeyKind, ServerConfig, UbootNetworkMode, UbootProfile, UefiBootArch,
        UefiHttpProfile, UefiHttpStrategy, ZhongshengRelayPowerManagement,
    };

    #[test]
    fn server_config_round_trip_includes_network() {
        let config = ServerConfig::default();
        let encoded = toml::to_string_pretty(&config).unwrap();
        let decoded: ServerConfig = toml::from_str(&encoded).unwrap();
        assert_eq!(decoded.listen_addr, SocketAddr::from(([0, 0, 0, 0], 2999)));
        assert_eq!(decoded.network.interface, "");
        assert_eq!(decoded.upload_limits.session_file_max_mib, 64);
        assert!(decoded.dtb_dir.ends_with("dtbs"));
        assert!(decoded.http_boot.root_dir.ends_with("http-boot"));
    }

    #[test]
    fn server_config_defaults_upload_limits_when_missing() {
        let decoded: ServerConfig = toml::from_str(
            r#"
listen_addr = "0.0.0.0:2999"
data_dir = ".ostool-server"
board_dir = ".ostool-server/boards"
dtb_dir = ".ostool-server/dtbs"

[tftp]
provider = "builtin"
enabled = true
root_dir = ".ostool-server/tftp-root"
bind_addr = "0.0.0.0:69"

[network]
interface = "eth0"
"#,
        )
        .unwrap();

        assert_eq!(decoded.upload_limits.session_file_max_mib, 64);
    }

    #[test]
    fn server_config_defaults_http_boot_discovery_when_missing() {
        let decoded: ServerConfig = toml::from_str(
            r#"
listen_addr = "0.0.0.0:2999"
data_dir = ".ostool-server"
board_dir = ".ostool-server/boards"
dtb_dir = ".ostool-server/dtbs"

[http_boot]
enabled = true
root_dir = ".ostool-server/http-boot"

[tftp]
provider = "builtin"
enabled = true
root_dir = ".ostool-server/tftp-root"
bind_addr = "0.0.0.0:69"

[network]
interface = "eth0"
"#,
        )
        .unwrap();

        assert!(decoded.http_boot.discovery.enabled);
        assert_eq!(decoded.http_boot.discovery.udp_port, 2998);
        assert_eq!(decoded.http_boot.discovery.advertise_interval_ms, 1000);
    }

    #[test]
    fn system_config_defaults_use_fhs_layout() {
        let config = ServerConfig::default_for_path(Path::new("/etc/ostool-server/config.toml"));
        assert_eq!(config.data_dir, PathBuf::from("/var/lib/ostool-server"));
        assert_eq!(
            config.board_dir,
            PathBuf::from("/var/lib/ostool-server/boards")
        );
        assert_eq!(config.dtb_dir, PathBuf::from("/var/lib/ostool-server/dtbs"));
        assert_eq!(
            config.http_boot.root_dir,
            PathBuf::from("/var/lib/ostool-server/http-boot")
        );
    }

    #[tokio::test]
    async fn write_to_path_persists_default_port_2999() {
        let temp = tempdir().unwrap();
        let path = temp.path().join("config.toml");
        let mut config =
            ServerConfig::default_for_path(Path::new("/etc/ostool-server/config.toml"));
        config.network.interface = "eth0".into();

        config.write_to_path(&path).await.unwrap();

        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.contains("listen_addr = \"0.0.0.0:2999\""));
        assert!(content.contains("session_file_max_mib = 64"));
        assert!(content.contains("[http_boot]"));
    }

    #[test]
    fn board_config_rejects_legacy_uboot_net_fields() {
        let config = r#"
id = "demo"
board_type = "demo"
tags = []
disabled = false

[boot]
kind = "uboot"
use_tftp = true

[boot.net]
interface = "eth0"
"#;

        let err = toml::from_str::<BoardConfig>(config).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("unknown field") || message.contains("net"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn board_config_rejects_legacy_power_command_fields() {
        let config = r#"
id = "demo"
board_type = "demo"
tags = []
disabled = false

[boot]
kind = "uboot"
use_tftp = false
board_reset_cmd = "reboot"
board_power_off_cmd = "shutdown"
"#;

        let err = toml::from_str::<BoardConfig>(config).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("unknown field") || message.contains("board_reset_cmd"),
            "unexpected error: {message}"
        );
    }

    #[test]
    fn board_config_defaults_legacy_uboot_network_mode_to_dhcp() {
        let config = r#"
id = "demo"
board_type = "demo"
tags = []
disabled = false

[power_management]
kind = "custom"
power_on_cmd = "echo on"
power_off_cmd = "echo off"

[boot]
kind = "uboot"
use_tftp = true
"#;

        let decoded: BoardConfig = toml::from_str(config).unwrap();
        let BootConfig::Uboot(profile) = decoded.boot else {
            panic!("expected uboot profile");
        };
        assert_eq!(profile.network_mode, UbootNetworkMode::Dhcp);
    }

    #[test]
    fn board_config_rejects_static_uboot_network_without_board_ip() {
        let config = r#"
id = "demo"
board_type = "demo"
tags = []
disabled = false

[power_management]
kind = "custom"
power_on_cmd = "echo on"
power_off_cmd = "echo off"

[boot]
kind = "uboot"
use_tftp = true
network_mode = "static_ip"
"#;

        let decoded: BoardConfig = toml::from_str(config).unwrap();
        let err = decoded.validate().unwrap_err();
        assert!(err.to_string().contains("board_ip"));
    }

    #[test]
    fn board_config_rejects_invalid_uboot_network_ipv4_fields() {
        for field in ["board_ip", "server_ip", "netmask", "gatewayip"] {
            let extra = if field == "board_ip" {
                r#"board_ip = "not-an-ip""#.to_string()
            } else {
                format!("board_ip = \"192.168.10.20\"\n{field} = \"not-an-ip\"")
            };
            let config = format!(
                r#"
id = "demo"
board_type = "demo"
tags = []
disabled = false

[power_management]
kind = "custom"
power_on_cmd = "echo on"
power_off_cmd = "echo off"

[boot]
kind = "uboot"
use_tftp = true
network_mode = "static_ip"
{extra}
"#
            );

            let decoded: BoardConfig = toml::from_str(&config).unwrap();
            let err = decoded.validate().unwrap_err();
            assert!(
                err.to_string().contains(field),
                "expected {field} in error, got {err:#}"
            );
        }
    }

    #[test]
    fn board_config_serialization_omits_removed_fields() {
        let board = BoardConfig {
            id: "demo-1".into(),
            board_type: "demo".into(),
            tags: vec!["lab".into()],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "echo on".into(),
                power_off_cmd: "echo off".into(),
            }),
            boot: BootConfig::Uboot(UbootProfile {
                use_tftp: true,
                dtb_name: Some("board.dtb".into()),
                ..Default::default()
            }),
            notes: None,
            disabled: false,
        };

        let value = serde_json::to_value(&board).unwrap();
        assert_eq!(value["id"], json!("demo-1"));
        assert!(value.get("name").is_none());
        assert!(value["boot"].get("success_regex").is_none());
        assert!(value["boot"].get("fail_regex").is_none());
        assert!(value["boot"].get("uboot_cmd").is_none());
        assert!(value["boot"].get("shell_prefix").is_none());
        assert!(value["boot"].get("shell_init_cmd").is_none());
        assert_eq!(value["boot"]["dtb_name"], json!("board.dtb"));
    }

    #[test]
    fn board_config_uboot_profile_supports_load_addresses() {
        let config = r#"
id = "demo-1"
board_type = "demo"

[power_management]
kind = "custom"
power_on_cmd = "echo on"
power_off_cmd = "echo off"

[boot]
kind = "uboot"
kernel_load_addr = "0x80200000"
fit_load_addr = "0x82200000"
bootm_addr = "0x82200000"
"#;

        let decoded: BoardConfig = toml::from_str(config).unwrap();
        let BootConfig::Uboot(profile) = decoded.boot else {
            panic!("expected uboot profile");
        };
        assert_eq!(profile.kernel_load_addr.as_deref(), Some("0x80200000"));
        assert_eq!(profile.fit_load_addr.as_deref(), Some("0x82200000"));
        assert_eq!(profile.bootm_addr.as_deref(), Some("0x82200000"));
    }

    #[test]
    fn board_config_round_trip_supports_httpboot_boot() {
        let board = BoardConfig {
            id: "uefi-http-01".into(),
            board_type: "x86_64-uefi-http".into(),
            tags: vec!["uefi-http".into()],
            serial: None,
            power_management: PowerManagementConfig::Custom(CustomPowerManagement {
                power_on_cmd: "true".into(),
                power_off_cmd: "true".into(),
            }),
            boot: BootConfig::UefiHttp(UefiHttpProfile {
                boot_arch: Some(UefiBootArch::X86_64),
                strategy: UefiHttpStrategy::LoaderDiscovery,
                mac: Some("1c:69:7a:dc:f3:47".into()),
            }),
            notes: None,
            disabled: false,
        };

        let encoded = toml::to_string_pretty(&board).unwrap();
        assert!(encoded.contains("kind = \"httpboot\""));
        assert!(encoded.contains("strategy = \"loader_discovery\""));
        assert!(encoded.contains("mac = \"1c:69:7a:dc:f3:47\""));

        let decoded: BoardConfig = toml::from_str(&encoded).unwrap();
        let BootConfig::UefiHttp(profile) = decoded.boot else {
            panic!("expected httpboot");
        };
        assert_eq!(profile.boot_arch, Some(UefiBootArch::X86_64));
        assert_eq!(profile.strategy, UefiHttpStrategy::LoaderDiscovery);
        assert_eq!(profile.mac.as_deref(), Some("1c:69:7a:dc:f3:47"));
    }

    #[test]
    fn board_config_round_trip_supports_relay_power_management_key() {
        let board = BoardConfig {
            id: "demo-relay".into(),
            board_type: "demo".into(),
            tags: vec![],
            serial: None,
            power_management: PowerManagementConfig::ZhongshengRelay(
                ZhongshengRelayPowerManagement {
                    key: SerialPortKey {
                        kind: SerialPortKeyKind::SerialNumber,
                        value: "relay-key".into(),
                    },
                },
            ),
            boot: BootConfig::Pxe(Default::default()),
            notes: None,
            disabled: false,
        };

        let encoded = toml::to_string_pretty(&board).unwrap();
        assert!(encoded.contains("kind = \"zhongsheng_relay\""));
        assert!(encoded.contains("[power_management.key]"));
        assert!(encoded.contains("value = \"relay-key\""));

        let decoded: BoardConfig = toml::from_str(&encoded).unwrap();
        let PowerManagementConfig::ZhongshengRelay(relay) = decoded.power_management else {
            panic!("expected relay power management");
        };
        assert_eq!(relay.key.kind, SerialPortKeyKind::SerialNumber);
        assert_eq!(relay.key.value, "relay-key");
    }

    #[test]
    fn board_config_rejects_legacy_relay_serial_port_field() {
        let config = r#"
id = "demo"
board_type = "demo"
tags = []
disabled = false

[power_management]
kind = "zhongsheng_relay"
serial_port = "/dev/ttyUSB0"

[boot]
kind = "pxe"
"#;

        let err = toml::from_str::<BoardConfig>(config).unwrap_err();
        let message = err.to_string();
        assert!(
            message.contains("unknown field") || message.contains("serial_port"),
            "unexpected error: {message}"
        );
    }
}
