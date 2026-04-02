use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use clap::Parser;
use log::info;
use ostool_server::{
    ServerConfig, build_app_state, build_router,
    tftp::service::{BuiltinTftpManager, SystemTftpdHpaManager, TftpManager},
};

#[derive(Parser, Debug)]
#[command(version, about = "ostool board server")]
struct Cli {
    #[arg(short, long, default_value = ".ostool-server.toml")]
    config: PathBuf,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    let config = ServerConfig::load_or_create(&cli.config).await?;
    let tftp_manager: Arc<dyn TftpManager> = match &config.tftp {
        ostool_server::TftpConfig::Builtin(cfg) => Arc::new(BuiltinTftpManager::new(cfg.clone())),
        ostool_server::TftpConfig::SystemTftpdHpa(cfg) => {
            Arc::new(SystemTftpdHpaManager::new(cfg.clone()))
        }
    };

    let state = build_app_state(cli.config.clone(), config, tftp_manager.clone()).await?;
    state.ensure_data_dirs().await?;
    for (board_id, err) in state.power_off_all_boards_on_startup().await {
        log::warn!(
            "failed to power off board `{}` during server startup; marking it disabled for this process: {}",
            board_id,
            err
        );
    }
    tftp_manager.start_if_needed().await?;
    if let ostool_server::TftpConfig::SystemTftpdHpa(cfg) = &state.config.read().await.tftp
        && cfg.reconcile_on_start
    {
        tftp_manager.reconcile().await?;
    }

    let gc_state = state.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if let Err(err) = gc_state.cleanup_expired_sessions().await {
                log::warn!("failed to cleanup expired sessions: {err:#}");
            }
        }
    });

    let app = build_router(state.clone());
    let listen_addr = state.config.read().await.listen_addr;
    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .with_context(|| format!("failed to bind {listen_addr}"))?;
    info!("ostoold listening on {}", listen_addr);
    axum::serve(listener, app).await?;
    Ok(())
}
