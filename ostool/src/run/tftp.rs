//! TFTP server for network booting.
//!
//! This module provides a simple TFTP server for network booting scenarios,
//! typically used with U-Boot to transfer kernel images over the network.
//!
//! # Permissions
//!
//! The TFTP server binds to port 69, which requires elevated privileges.
//! On Linux, you can grant the necessary capabilities with:
//!
//! ```bash
//! sudo setcap cap_net_bind_service=+eip $(which ostool)
//! ```

use std::net::{IpAddr, Ipv4Addr};

use colored::Colorize as _;
use tftpd::{Config, Server};

use crate::ctx::AppContext;

/// Starts a TFTP server serving files from the build output directory.
///
/// The server runs in a background thread and serves files from the directory
/// containing the ELF/binary artifacts.
///
/// # Arguments
///
/// * `app` - The application context containing the file paths.
///
/// # Errors
///
/// Returns an error if the server fails to start (e.g., port already in use
/// or insufficient permissions).
pub fn run_tftp_server(app: &AppContext) -> anyhow::Result<()> {
    // TFTP server implementation goes here
    let mut file_dir = app.paths.manifest.clone();
    if let Some(elf_path) = &app.paths.artifacts.elf {
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
                    println!("{}", e);
                    println!("{}","TFTP server 启动失败：{e:?}。若权限不足，尝试执行 `sudo setcap cap_net_bind_service=+eip $(which cargo-osrun)&&sudo setcap cap_net_bind_service=+eip $(which ostool)` 并重启终端".red());
                    std::process::exit(1);
                }).unwrap();
        server.listen();
    });

    Ok(())
}
