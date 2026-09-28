use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::{Context, anyhow};
use clap::{Parser, Subcommand};
use log::info;
use ostool_server::{
    ServerConfig, build_app_state, build_router,
    loader::start_udp_discovery,
    tftp::service::{BuiltinTftpManager, SystemTftpdHpaManager, TftpManager},
    virtual_lab::{VirtualLabAction, run_virtual_lab},
};
use tokio::{sync::watch, task::JoinSet};

#[derive(Parser, Debug)]
#[command(version, about = "ostool board server")]
struct Cli {
    #[arg(short, long, default_value = ".ostool-server.toml")]
    config: PathBuf,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Manage the isolated QEMU network namespace and TAP pool.
    VirtualLab {
        #[command(subcommand)]
        action: VirtualLabCommand,
    },
}

#[derive(Subcommand, Debug)]
enum VirtualLabCommand {
    Up,
    Status,
    Down,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    let config = ServerConfig::load_or_create(&cli.config).await?;
    if let Some(Command::VirtualLab { action }) = cli.command {
        let action = match action {
            VirtualLabCommand::Up => VirtualLabAction::Up,
            VirtualLabCommand::Status => VirtualLabAction::Status,
            VirtualLabCommand::Down => VirtualLabAction::Down,
        };
        return run_virtual_lab(&config.virtual_qemu, action).await;
    }
    let listen_addr = config.listen_addr;
    let listener = tokio::net::TcpListener::bind(listen_addr)
        .await
        .with_context(|| format!("failed to bind management listener at {listen_addr}"))?;
    let network_test_config = config.network_test.clone();
    let network_test_listener = if network_test_config.enabled {
        let addr = network_test_config.listen_addr;
        Some(
            tokio::net::TcpListener::bind(addr)
                .await
                .with_context(|| format!("failed to bind network test listener at {addr}"))?,
        )
    } else {
        None
    };
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
            "failed to power off board `{board_id}` during server startup; marking it disabled for this process: {err}"
        );
    }
    let discovery_task = start_udp_discovery(state.clone()).await?;
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

    #[cfg(target_os = "linux")]
    let _admin_monitors =
        ostool_server::admin_monitor::start(&state).context("failed to start admin OS monitors")?;
    let app = build_router(state.clone());
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let mut servers = JoinSet::new();
    servers.spawn(async move {
        axum::serve(listener, app)
            .with_graceful_shutdown(wait_for_shutdown(shutdown_rx))
            .await
            .context("management listener failed")
    });
    info!("ostoold management API listening on {listen_addr}");
    if let Some(test_listener) = network_test_listener {
        let test_shutdown = shutdown_tx.subscribe();
        let test_addr = network_test_config.listen_addr;
        let test_app = ostool_server::network_test::build_router(network_test_config);
        servers.spawn(async move {
            axum::serve(
                test_listener,
                test_app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(wait_for_shutdown(test_shutdown))
            .await
            .context("network test listener failed")
        });
        info!("ostoold network test API listening on {test_addr}");
    }

    let mut serve_result: anyhow::Result<()> = tokio::select! {
        _ = shutdown_signal() => Ok(()),
        completed = servers.join_next() => match completed {
            Some(Ok(Ok(()))) => Err(anyhow!("server listener stopped unexpectedly")),
            Some(Ok(Err(error))) => Err(error),
            Some(Err(error)) => Err(error.into()),
            None => Err(anyhow!("no server listeners are running")),
        },
    };
    shutdown_tx.send_replace(true);
    while let Some(completed) = servers.join_next().await {
        let result = completed
            .context("server listener task failed")
            .and_then(|result| result);
        if let Err(error) = result {
            if serve_result.is_ok() {
                serve_result = Err(error);
            } else {
                log::warn!("server listener failed while shutting down: {error:#}");
            }
        }
    }
    if let Some(task) = discovery_task {
        task.abort();
        let _ = task.await;
    }
    state.virtual_boards.shutdown().await;
    serve_result
}

async fn wait_for_shutdown(mut shutdown: watch::Receiver<bool>) {
    let _ = shutdown.wait_for(|requested| *requested).await;
}

async fn shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};

        let mut terminate = match signal(SignalKind::terminate()) {
            Ok(signal) => signal,
            Err(error) => {
                log::warn!("failed to install SIGTERM handler: {error}");
                let _ = tokio::signal::ctrl_c().await;
                return;
            }
        };
        tokio::select! {
            result = tokio::signal::ctrl_c() => {
                if let Err(error) = result {
                    log::warn!("failed to wait for Ctrl-C: {error}");
                }
            }
            _ = terminate.recv() => {}
        }
    }

    #[cfg(not(unix))]
    if let Err(error) = tokio::signal::ctrl_c().await {
        log::warn!("failed to wait for Ctrl-C: {error}");
    }
}
