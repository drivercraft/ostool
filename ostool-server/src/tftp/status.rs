use std::{net::Ipv4Addr, path::PathBuf, process::Command, sync::Mutex};

use anyhow::{Context, bail};
use network_interface::{Addr, NetworkInterface, NetworkInterfaceConfig};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct TftpStatus {
    pub provider: String,
    pub enabled: bool,
    pub healthy: bool,
    pub writable: bool,
    pub resolved_server_ip: Option<String>,
    pub resolved_netmask: Option<String>,
    pub root_dir: PathBuf,
    pub bind_addr_or_address: Option<String>,
    pub service_state: Option<String>,
    pub last_error: Option<String>,
}

pub fn set_last_error(store: &Mutex<Option<String>>, error: impl Into<String>) {
    *store.lock().unwrap() = Some(error.into());
}

pub fn clear_last_error(store: &Mutex<Option<String>>) {
    *store.lock().unwrap() = None;
}

pub fn current_last_error(store: &Mutex<Option<String>>) -> Option<String> {
    store.lock().unwrap().clone()
}

pub fn resolve_interface_ipv4(interface_name: &str) -> anyhow::Result<Option<String>> {
    if interface_name.trim().is_empty() {
        return Ok(None);
    }

    let interfaces = NetworkInterface::show()?;
    for interface in interfaces {
        if interface.name != interface_name {
            continue;
        }

        for addr in interface.addr {
            if let Addr::V4(v4) = addr {
                return Ok(Some(v4.ip.to_string()));
            }
        }
    }

    Ok(None)
}

pub fn run_capture(program: &str, args: &[&str]) -> anyhow::Result<String> {
    let output = Command::new(program)
        .args(args)
        .output()
        .with_context(|| format!("failed to execute `{program}`"))?;
    if !output.status.success() {
        bail!("command `{program}` exited with status {}", output.status);
    }
    Ok(String::from_utf8(output.stdout)
        .with_context(|| format!("failed to decode output from `{program}`"))?
        .trim()
        .to_string())
}

pub fn system_service_state(service_name: &str) -> Option<String> {
    if !cfg!(target_os = "linux") {
        return None;
    }

    run_capture("systemctl", &["is-active", service_name]).ok()
}

pub fn udp_port_69_is_listening() -> anyhow::Result<bool> {
    if !cfg!(target_os = "linux") {
        return Ok(true);
    }

    let output = run_capture("ss", &["-lun"])?;
    Ok(output.lines().any(|line| {
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
    }))
}

pub fn ipv4_unspecified() -> Ipv4Addr {
    Ipv4Addr::UNSPECIFIED
}
