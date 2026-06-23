use std::{path::PathBuf, sync::Arc, time::Duration};

use anyhow::Context;
use clap::{Parser, Subcommand};
use log::info;
use ostool_server::{
    ServerConfig,
    auth::AuthService,
    build_app_state, build_router,
    storage::{DynStorage, UserProfile, mysql::MysqlStorage, sqlite::SqliteStorage},
    tftp::service::{BuiltinTftpManager, SystemTftpdHpaManager, TftpManager},
};

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
    Admin {
        #[command(subcommand)]
        command: AdminCommand,
    },
}

#[derive(Subcommand, Debug)]
enum AdminCommand {
    Init {
        #[arg(long)]
        username: String,
        #[arg(long)]
        password: Option<String>,
        #[arg(long)]
        display_name: Option<String>,
        #[arg(long)]
        email: Option<String>,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    env_logger::init();

    let cli = Cli::parse();
    let config = ServerConfig::load_or_create(&cli.config).await?;
    if let Some(command) = cli.command {
        return run_command(config, command).await;
    }
    serve(cli.config, config).await
}

async fn run_command(config: ServerConfig, command: Command) -> anyhow::Result<()> {
    match command {
        Command::Admin {
            command:
                AdminCommand::Init {
                    username,
                    password,
                    display_name,
                    email,
                },
        } => {
            let storage: DynStorage = match &config.database {
                ostool_server::DatabaseConfig::Mysql(mysql) => {
                    Arc::new(MysqlStorage::connect(&mysql.url).await?)
                }
                ostool_server::DatabaseConfig::Sqlite(sqlite) => {
                    Arc::new(SqliteStorage::connect(&sqlite.url).await?)
                }
            };
            let auth = AuthService::new(storage.clone());
            if storage.find_user_by_username(&username).await?.is_some() {
                anyhow::bail!("user `{username}` already exists");
            }
            let password = password.unwrap_or_else(|| "admin".to_string());
            let user = auth
                .create_user(
                    username.clone(),
                    display_name.unwrap_or_else(|| username.clone()),
                    email.unwrap_or_else(|| format!("{username}@ostool.local")),
                    password,
                    UserProfile::default(),
                    vec!["admin".to_string()],
                )
                .await?;
            println!("created admin user `{}` ({})", user.username, user.id);
            Ok(())
        }
    }
}

async fn serve(config_path: PathBuf, config: ServerConfig) -> anyhow::Result<()> {
    let tftp_manager: Arc<dyn TftpManager> = match &config.tftp {
        ostool_server::TftpConfig::Builtin(cfg) => Arc::new(BuiltinTftpManager::new(cfg.clone())),
        ostool_server::TftpConfig::SystemTftpdHpa(cfg) => {
            Arc::new(SystemTftpdHpaManager::new(cfg.clone()))
        }
    };

    let state = build_app_state(config_path, config, tftp_manager.clone()).await?;
    state.ensure_data_dirs().await?;
    for (board_id, err) in state.power_off_all_boards_on_startup().await {
        log::warn!(
            "failed to power off board `{board_id}` during server startup; marking it disabled for this process: {err}"
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
    info!("ostoold listening on {listen_addr}");
    axum::serve(listener, app).await?;
    Ok(())
}
