use std::{
    io,
    path::{Path, PathBuf},
};

use anyhow::Context as _;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use tokio::fs;

use crate::{
    Tool,
    board::{
        acquire_board_session,
        client::{
            BoardServerClient, BootConfig as RemoteBootConfig, HttpBootManifest,
            SessionCreatedResponse, UefiBootArch,
        },
        finalize_session, load_board_global_config_with_notice, print_allocated_board_session,
        session::BoardSession,
        terminal,
    },
    utils::PathResultExt,
};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema, Default)]
pub struct HttpBootConfig {
    pub board_type: String,
    pub server: Option<String>,
    pub port: Option<u16>,
    pub remote_name: Option<String>,
    pub kernel_load_addr: String,
    pub entry_point: String,
    #[serde(default = "default_power_cycle")]
    pub power_cycle: bool,
    #[serde(default = "default_open_console")]
    pub open_console: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RunHttpBootOptions {
    pub show_output: bool,
}

fn default_power_cycle() -> bool {
    true
}

fn default_open_console() -> bool {
    true
}

impl HttpBootConfig {
    fn replace_strings(&mut self, tool: &Tool) -> anyhow::Result<()> {
        self.board_type = tool.replace_string(&self.board_type)?;
        self.server = self
            .server
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.remote_name = self
            .remote_name
            .as_deref()
            .map(|value| tool.replace_string(value))
            .transpose()?;
        self.kernel_load_addr = tool.replace_string(&self.kernel_load_addr)?;
        self.entry_point = tool.replace_string(&self.entry_point)?;
        Ok(())
    }

    fn normalize(&mut self, config_name: &str) -> anyhow::Result<()> {
        normalize_required_string(&mut self.board_type, "board_type", config_name)?;
        normalize_required_string(&mut self.kernel_load_addr, "kernel_load_addr", config_name)?;
        normalize_required_string(&mut self.entry_point, "entry_point", config_name)?;
        normalize_optional_string(&mut self.server);
        normalize_optional_string(&mut self.remote_name);
        Ok(())
    }
}

impl Tool {
    pub fn default_httpboot_config(&self) -> HttpBootConfig {
        HttpBootConfig {
            board_type: "x86_64-uefi-http".to_string(),
            remote_name: Some("kernel.bin".to_string()),
            kernel_load_addr: "0x200000".to_string(),
            entry_point: "0x200000".to_string(),
            power_cycle: true,
            open_console: true,
            ..Default::default()
        }
    }

    pub async fn read_httpboot_config_from_path_for_cargo(
        &mut self,
        cargo: &crate::build::config::Cargo,
        path: &Path,
    ) -> anyhow::Result<HttpBootConfig> {
        self.sync_cargo_context(cargo);
        let config_path = self.replace_path_variables(path.to_path_buf())?;
        read_httpboot_config_at_path(self, config_path).await
    }

    pub async fn ensure_httpboot_config_for_cargo(
        &mut self,
        cargo: &crate::build::config::Cargo,
    ) -> anyhow::Result<HttpBootConfig> {
        self.sync_cargo_context(cargo);
        let workspace_dir = self.workspace_dir().clone();
        self.ensure_httpboot_config_in_dir_for_cargo(cargo, &workspace_dir)
            .await
    }

    pub async fn ensure_httpboot_config_in_dir_for_cargo(
        &mut self,
        cargo: &crate::build::config::Cargo,
        dir: &Path,
    ) -> anyhow::Result<HttpBootConfig> {
        self.sync_cargo_context(cargo);
        let dir = self.replace_path_variables(dir.to_path_buf())?;
        ensure_httpboot_config_at_path(
            self,
            dir.join(".httpboot.toml"),
            self.default_httpboot_config(),
        )
        .await
    }

    pub async fn ensure_httpboot_config_in_dir(
        &mut self,
        dir: &Path,
    ) -> anyhow::Result<HttpBootConfig> {
        let dir = self.replace_path_variables(dir.to_path_buf())?;
        ensure_httpboot_config_at_path(
            self,
            dir.join(".httpboot.toml"),
            self.default_httpboot_config(),
        )
        .await
    }

    pub async fn read_httpboot_config_from_path(
        &mut self,
        path: &Path,
    ) -> anyhow::Result<HttpBootConfig> {
        let config_path = self.replace_path_variables(path.to_path_buf())?;
        read_httpboot_config_at_path(self, config_path).await
    }

    pub async fn run_httpboot(
        &mut self,
        config: &HttpBootConfig,
        options: RunHttpBootOptions,
    ) -> anyhow::Result<()> {
        let _ = options.show_output;
        let mut config = config.clone();
        config.replace_strings(self)?;
        config.normalize("HTTP Boot runtime config")?;

        let kernel_bin = self.objcopy_output_bin()?;
        let global_config = load_board_global_config_with_notice()?;
        let (server, port) = global_config.resolve_server(config.server.as_deref(), config.port);
        let (client, session) = acquire_board_session(&server, port, &config.board_type).await?;
        print_allocated_board_session(&session, &config.board_type);

        let run_result = run_httpboot_session(&client, session.info(), &config, &kernel_bin).await;
        let run_result = finish_httpboot_session(&client, &session, &config, run_result).await;
        finalize_session(session, run_result).await
    }
}

async fn read_httpboot_config_at_path(
    tool: &Tool,
    config_path: PathBuf,
) -> anyhow::Result<HttpBootConfig> {
    let mut config: HttpBootConfig = fs::read_to_string(&config_path)
        .await
        .with_context(|| format!("failed to read HTTP Boot config: {}", config_path.display()))
        .and_then(|content| {
            toml::from_str(&content).with_context(|| {
                format!(
                    "failed to parse HTTP Boot config: {}",
                    config_path.display()
                )
            })
        })?;
    config.replace_strings(tool)?;
    config.normalize(&format!("HTTP Boot config {}", config_path.display()))?;
    Ok(config)
}

async fn ensure_httpboot_config_at_path(
    tool: &Tool,
    config_path: PathBuf,
    default_config: HttpBootConfig,
) -> anyhow::Result<HttpBootConfig> {
    let mut config = match fs::read_to_string(&config_path).await {
        Ok(_) => return read_httpboot_config_at_path(tool, config_path).await,
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            let config = default_config;
            fs::write(&config_path, toml::to_string_pretty(&config)?)
                .await
                .with_path("failed to write file", &config_path)?;
            config
        }
        Err(err) => return Err(err.into()),
    };

    config.replace_strings(tool)?;
    config.normalize(&format!("HTTP Boot config {}", config_path.display()))?;
    Ok(config)
}

struct HttpBootPublishedUrls {
    loader_url: String,
    manifest_url: String,
    kernel_url: String,
}

async fn finish_httpboot_session(
    client: &BoardServerClient,
    session: &BoardSession,
    config: &HttpBootConfig,
    run_result: anyhow::Result<HttpBootPublishedUrls>,
) -> anyhow::Result<()> {
    let urls = run_result?;
    println!("HTTP Boot artifacts published:");
    println!("  loader_url: {}", urls.loader_url);
    println!("  manifest_url: {}", urls.manifest_url);
    println!("  kernel_url: {}", urls.kernel_url);

    if config.power_cycle {
        client
            .power_off_board(&session.info().session_id)
            .await
            .context("failed to power off board")?;
        client
            .power_on_board(&session.info().session_id)
            .await
            .context("failed to power on board")?;
    }

    if !config.open_console {
        return Ok(());
    }

    if session.info().serial_available {
        let ws_path = session
            .info()
            .ws_url
            .as_deref()
            .ok_or_else(|| anyhow::anyhow!("server did not return a serial websocket URL"))?;
        let ws_url = client.resolve_ws_url(ws_path)?;
        terminal::run_serial_terminal(ws_url).await
    } else {
        println!("Board has no serial configuration; HTTP Boot artifacts are ready.");
        Ok(())
    }
}

async fn run_httpboot_session(
    client: &BoardServerClient,
    session: &SessionCreatedResponse,
    config: &HttpBootConfig,
    kernel_bin: &Path,
) -> anyhow::Result<HttpBootPublishedUrls> {
    if session.boot_mode != "uefi_http" {
        anyhow::bail!(
            "unsupported remote boot mode `{}`; only `uefi_http` is supported",
            session.boot_mode
        );
    }

    let boot_profile = client
        .get_boot_profile(&session.session_id)
        .await
        .context("failed to get HTTP Boot profile")?;
    let RemoteBootConfig::UefiHttp(profile) = boot_profile.boot else {
        anyhow::bail!("server returned a non-uefi_http boot profile");
    };

    let remote_name = config
        .remote_name
        .clone()
        .or(profile.kernel_file.clone())
        .or_else(|| {
            kernel_bin
                .file_name()
                .and_then(|name| name.to_str())
                .map(str::to_string)
        })
        .ok_or_else(|| anyhow::anyhow!("failed to determine remote kernel filename"))?;

    let kernel_bytes = std::fs::read(kernel_bin)
        .with_context(|| format!("failed to read {}", kernel_bin.display()))?;
    let kernel_size = kernel_bytes.len() as u64;
    let uploaded = client
        .upload_http_boot_file(&session.session_id, &remote_name, kernel_bytes)
        .await
        .with_context(|| format!("failed to upload HTTP Boot file `{remote_name}`"))?;

    let arch = profile
        .boot_arch
        .as_ref()
        .map(uefi_boot_arch_name)
        .unwrap_or("other")
        .to_string();
    let manifest = HttpBootManifest {
        kernel_url: uploaded.http_url.clone(),
        kernel_size,
        kernel_load_addr: config.kernel_load_addr.clone(),
        entry_point: config.entry_point.clone(),
        arch,
    };
    let manifest_file = client
        .upload_http_boot_manifest(&session.session_id, &manifest)
        .await
        .context("failed to upload HTTP Boot manifest")?;

    let loader_file = profile
        .loader_file
        .as_deref()
        .ok_or_else(|| anyhow::anyhow!("uefi_http boot profile is missing `loader_file`"))?;
    let loader_url = manifest_file
        .http_url
        .trim_end_matches("manifest.json")
        .to_string()
        + loader_file;

    Ok(HttpBootPublishedUrls {
        loader_url,
        manifest_url: manifest_file.http_url,
        kernel_url: uploaded.http_url,
    })
}

fn uefi_boot_arch_name(arch: &UefiBootArch) -> &'static str {
    match arch {
        UefiBootArch::X86_64 => "x86_64",
        UefiBootArch::Aarch64 => "aarch64",
        UefiBootArch::Loongarch64 => "loongarch64",
        UefiBootArch::Riscv64 => "riscv64",
        UefiBootArch::Other => "other",
    }
}

fn normalize_required_string(
    value: &mut String,
    field_name: &str,
    config_name: &str,
) -> anyhow::Result<()> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("`{field_name}` must not be empty in {config_name}");
    }
    if trimmed.len() != value.len() {
        *value = trimmed.to_string();
    }
    Ok(())
}

fn normalize_optional_string(value: &mut Option<String>) {
    if let Some(raw) = value {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            *value = None;
        } else if trimmed.len() != raw.len() {
            *raw = trimmed.to_string();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HttpBootConfig;

    #[test]
    fn httpboot_config_normalizes_required_fields() {
        let mut config = HttpBootConfig {
            board_type: " x86_64-uefi-http ".into(),
            kernel_load_addr: " 0x200000 ".into(),
            entry_point: " 0x200000 ".into(),
            remote_name: Some(" kernel.bin ".into()),
            ..Default::default()
        };

        config.normalize("test").unwrap();

        assert_eq!(config.board_type, "x86_64-uefi-http");
        assert_eq!(config.kernel_load_addr, "0x200000");
        assert_eq!(config.entry_point, "0x200000");
        assert_eq!(config.remote_name.as_deref(), Some("kernel.bin"));
    }
}
