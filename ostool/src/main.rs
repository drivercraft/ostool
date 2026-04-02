use std::{path::PathBuf, process::ExitCode};

use anyhow::Result;
use clap::*;
use colored::Colorize as _;
use env_logger::Env;

use log::info;
use ostool::{
    Tool, ToolConfig, board,
    board::{
        client::BoardServerClient, config::BoardRunConfig, config_tui::run_board_config_tui,
        global_config::LoadedBoardGlobalConfig, session::BoardSession,
    },
    build::{
        self, CargoQemuAppendArgs, CargoQemuOverrideArgs, CargoQemuRunnerArgs, CargoRunnerKind,
        CargoUbootRunnerArgs, cargo_builder::CargoBuilder,
    },
    menuconfig::{MenuConfigHandler, MenuConfigMode},
    resolve_manifest_context,
    run::{qemu::RunQemuArgs, uboot::RunUbootArgs},
};

#[derive(Parser, Debug)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    manifest: Option<PathBuf>,
    #[command(subcommand)]
    command: SubCommands,
}

#[derive(Subcommand, Debug)]
enum SubCommands {
    Build {
        /// Path to the build configuration file
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    Run {
        #[command(subcommand)]
        command: RunSubCommands,
    },
    Board(BoardArgs),
    Menuconfig {
        /// Menu configuration mode (qemu or uboot)
        #[arg(value_enum)]
        mode: Option<MenuConfigMode>,
    },
}

#[derive(Args, Debug, Default, Clone)]
struct BoardServerArgs {
    /// ostool-server host
    #[arg(long)]
    server: Option<String>,
    /// ostool-server port
    #[arg(long)]
    port: Option<u16>,
}

#[derive(Args, Debug)]
struct BoardArgs {
    #[command(subcommand)]
    command: BoardSubCommands,
}

#[derive(Subcommand, Debug)]
enum BoardSubCommands {
    Ls(BoardServerArgs),
    Run(BoardRunArgs),
    Config,
}

#[derive(Args, Debug)]
struct RunQemuCommand {
    /// Path to the build configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,
    #[command(flatten)]
    qemu: QemuArgs,
}

#[derive(Args, Debug)]
struct RunUbootCommand {
    /// Path to the build configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,
    #[command(flatten)]
    uboot: UbootArgs,
}

#[derive(Args, Debug)]
struct BoardRunArgs {
    /// Path to the build configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,
    /// Path to the board runner configuration file, defaults to `pwd/.board.toml`
    #[arg(long = "board-config")]
    board_config: Option<PathBuf>,
    #[command(flatten)]
    server: BoardServerArgs,
}

#[derive(Subcommand, Debug)]
enum RunSubCommands {
    Qemu(RunQemuCommand),
    Uboot(RunUbootCommand),
}

#[derive(Args, Debug, Default)]
pub struct QemuArgs {
    /// Path to the qemu configuration file
    ///
    /// Default behavior when not specified:
    /// - Cargo build system: use the target package directory
    /// - Custom build system: use the workspace directory
    /// - With architecture detected: .qemu-{arch}.toml (e.g., .qemu-aarch64.toml)
    /// - Without architecture: .qemu.toml
    #[arg(short, long)]
    qemu_config: Option<PathBuf>,
    #[arg(short, long)]
    debug: bool,
    /// Dump DTB file
    #[arg(long)]
    dtb_dump: bool,
}

#[derive(Args, Debug)]
pub struct UbootArgs {
    /// Path to the uboot configuration file, default to '.uboot.toml'
    #[arg(short, long)]
    uboot_config: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    match try_main().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            report_error(&err);
            ExitCode::FAILURE
        }
    }
}

async fn try_main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let Cli { manifest, command } = Cli::parse();

    match command {
        SubCommands::Board(args) => match args.command {
            BoardSubCommands::Ls(server) => {
                let global_config = load_board_global_config_with_notice()?;
                let (server, port) =
                    global_config.resolve_server(server.server.as_deref(), server.port);
                board::list_boards(&server, port).await?;
            }
            BoardSubCommands::Run(args) => {
                let global_config = load_board_global_config_with_notice()?;
                let mut tool = init_tool(manifest.clone())?;
                let board_config =
                    BoardRunConfig::load_or_create(&tool, args.board_config.clone()).await?;
                prepare_uboot_artifacts(&mut tool, args.config.clone()).await?;

                let (server, port) = board_config.resolve_server(
                    args.server.server.as_deref(),
                    args.server.port,
                    &global_config.board,
                );
                let client = BoardServerClient::new(&server, port)?;
                let session =
                    BoardSession::acquire(client.clone(), &board_config.board_type).await?;

                println!("Allocated board session:");
                println!("  board_type: {}", board_config.board_type);
                println!("  board_id: {}", session.info().board_id);
                println!("  session_id: {}", session.info().session_id);
                println!("  lease_expires_at: {}", session.info().lease_expires_at);
                println!("  boot_mode: {}", session.info().boot_mode);

                let run_result = match session.info().boot_mode.as_str() {
                    "uboot" => {
                        tool.run_uboot_remote(&board_config, client, session.info().clone())
                            .await
                    }
                    other => Err(anyhow::anyhow!(
                        "unsupported board boot mode `{other}`; only `uboot` is supported"
                    )),
                };

                let release_result = session.release().await;
                match (run_result, release_result) {
                    (Ok(()), Ok(())) => {}
                    (Err(err), Ok(())) => return Err(err),
                    (Ok(()), Err(err)) => return Err(err),
                    (Err(run_err), Err(release_err)) => {
                        return Err(run_err.context(format!(
                            "additionally failed to release board session: {release_err:#}"
                        )));
                    }
                }
            }
            BoardSubCommands::Config => {
                run_board_config_tui()?;
            }
        },
        SubCommands::Build { config } => {
            let mut tool = init_tool(manifest)?;
            tool.build(config).await?;
        }
        SubCommands::Run { command } => match command {
            RunSubCommands::Qemu(args) => {
                let RunQemuCommand { config, qemu } = args;
                let qemu_config = qemu.qemu_config.clone();
                let debug = qemu.debug;
                let dtb_dump = qemu.dtb_dump;

                let mut tool = init_tool(manifest.clone())?;
                let config = tool.prepare_build_config(config, false).await?;
                match config.system {
                    build::config::BuildSystem::Cargo(config) => {
                        let kind = CargoRunnerKind::Qemu(Box::new(CargoQemuRunnerArgs {
                            qemu_config,
                            debug,
                            dtb_dump,
                            default_args: CargoQemuOverrideArgs::default(),
                            append_args: CargoQemuAppendArgs::default(),
                            override_args: CargoQemuOverrideArgs::default(),
                        }));
                        tool.cargo_run(&config, &kind).await?;
                    }
                    build::config::BuildSystem::Custom(custom_cfg) => {
                        tool.shell_run_cmd(&custom_cfg.build_cmd)?;
                        tool.set_elf_path(custom_cfg.elf_path.clone().into())
                            .await?;
                        info!(
                            "ELF {:?}: {}",
                            tool.ctx().arch,
                            tool.ctx().artifacts.elf.as_ref().unwrap().display()
                        );

                        if custom_cfg.to_bin {
                            tool.objcopy_output_bin()?;
                        }

                        tool.run_qemu(RunQemuArgs {
                            qemu_config,
                            dtb_dump,
                            show_output: true,
                        })
                        .await?;
                    }
                }
            }
            RunSubCommands::Uboot(args) => {
                let RunUbootCommand { config, uboot } = args;
                let uboot_config = uboot.uboot_config;

                let mut tool = init_tool(manifest.clone())?;
                let config = tool.prepare_build_config(config, false).await?;
                match config.system {
                    build::config::BuildSystem::Cargo(config) => {
                        let kind = CargoRunnerKind::Uboot(CargoUbootRunnerArgs {
                            uboot_config: uboot_config.clone(),
                        });
                        tool.cargo_run(&config, &kind).await?;
                    }
                    build::config::BuildSystem::Custom(custom_cfg) => {
                        tool.shell_run_cmd(&custom_cfg.build_cmd)?;
                        tool.set_elf_path(custom_cfg.elf_path.clone().into())
                            .await?;
                        info!(
                            "ELF {:?}: {}",
                            tool.ctx().arch,
                            tool.ctx().artifacts.elf.as_ref().unwrap().display()
                        );

                        if custom_cfg.to_bin {
                            tool.objcopy_output_bin()?;
                        }

                        tool.run_uboot(RunUbootArgs {
                            config: uboot_config,
                            show_output: true,
                        })
                        .await?;
                    }
                }
            }
        },
        SubCommands::Menuconfig { mode } => {
            let mut tool = init_tool(manifest)?;
            MenuConfigHandler::handle_menuconfig(&mut tool, mode).await?;
        }
    }

    Ok(())
}

async fn prepare_uboot_artifacts(
    tool: &mut Tool,
    config_path: Option<PathBuf>,
) -> Result<ostool::build::config::BuildConfig> {
    let config = tool.prepare_build_config(config_path, false).await?;
    match &config.system {
        build::config::BuildSystem::Cargo(cargo) => {
            let build_config_path = tool.ctx().build_config_path.clone();
            CargoBuilder::build(tool, cargo, build_config_path)
                .skip_objcopy(true)
                .resolve_artifact_from_json(true)
                .execute()
                .await?;
        }
        build::config::BuildSystem::Custom(custom_cfg) => {
            tool.shell_run_cmd(&custom_cfg.build_cmd)?;
            tool.set_elf_path(custom_cfg.elf_path.clone().into())
                .await?;
            info!(
                "ELF {:?}: {}",
                tool.ctx().arch,
                tool.ctx().artifacts.elf.as_ref().unwrap().display()
            );

            if custom_cfg.to_bin {
                tool.objcopy_output_bin()?;
            }
        }
    }
    Ok(config)
}

fn init_tool(manifest_arg: Option<PathBuf>) -> Result<Tool> {
    let manifest = resolve_manifest_context(manifest_arg.clone())?;
    info!("Using manifest {}", manifest.manifest_path.display());

    Tool::new(ToolConfig {
        manifest: manifest_arg,
        ..Default::default()
    })
}

fn report_error(err: &anyhow::Error) {
    log::error!("{err:#}");
    log::error!("Trace:\n{err:?}");

    println!("{}", format!("Error: {err:#}").red().bold());
    println!("{}", format!("\nTrace:\n{err:?}").red());
}

fn load_board_global_config_with_notice() -> Result<LoadedBoardGlobalConfig> {
    let loaded = LoadedBoardGlobalConfig::load_or_create()?;
    if loaded.created {
        println!("Created default board config: {}", loaded.path.display());
    }
    Ok(loaded)
}

impl From<QemuArgs> for RunQemuArgs {
    fn from(value: QemuArgs) -> Self {
        RunQemuArgs {
            qemu_config: value.qemu_config,
            dtb_dump: value.dtb_dump,
            show_output: true,
        }
    }
}

impl From<UbootArgs> for RunUbootArgs {
    fn from(value: UbootArgs) -> Self {
        RunUbootArgs {
            config: value.uboot_config,
            show_output: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use super::{BoardArgs, BoardSubCommands, Cli, SubCommands};

    #[test]
    fn parse_board_ls_with_server_args() {
        let cli = Cli::try_parse_from([
            "ostool", "board", "ls", "--server", "10.0.0.2", "--port", "9000",
        ])
        .unwrap();

        match cli.command {
            SubCommands::Board(BoardArgs {
                command: BoardSubCommands::Ls(server),
            }) => {
                assert_eq!(server.server.as_deref(), Some("10.0.0.2"));
                assert_eq!(server.port, Some(9000));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_board_run_with_board_type() {
        let cli = Cli::try_parse_from(["ostool", "board", "run"]).unwrap();

        match cli.command {
            SubCommands::Board(BoardArgs {
                command: BoardSubCommands::Run(args),
            }) => {
                assert!(args.config.is_none());
                assert!(args.board_config.is_none());
                assert!(args.server.server.is_none());
                assert!(args.server.port.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_board_run_with_build_and_board_config() {
        let cli = Cli::try_parse_from([
            "ostool",
            "board",
            "run",
            "--config",
            "board.build.toml",
            "--board-config",
            "remote.board.toml",
            "--server",
            "10.0.0.2",
            "--port",
            "9000",
        ])
        .unwrap();

        match cli.command {
            SubCommands::Board(BoardArgs {
                command: BoardSubCommands::Run(args),
            }) => {
                assert_eq!(
                    args.config.as_deref(),
                    Some(std::path::Path::new("board.build.toml"))
                );
                assert_eq!(
                    args.board_config.as_deref(),
                    Some(std::path::Path::new("remote.board.toml"))
                );
                assert_eq!(args.server.server.as_deref(), Some("10.0.0.2"));
                assert_eq!(args.server.port, Some(9000));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_board_config_command() {
        let cli = Cli::try_parse_from(["ostool", "board", "config"]).unwrap();

        match cli.command {
            SubCommands::Board(BoardArgs {
                command: BoardSubCommands::Config,
            }) => {}
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_run_board_is_rejected() {
        let err = Cli::try_parse_from(["ostool", "run", "board"]).unwrap_err();
        let rendered = err.to_string();
        assert!(rendered.contains("unrecognized subcommand"));
        assert!(rendered.contains("board"));
    }
}
