//! Main ostool CLI argument parsing and command dispatch.

use std::{path::PathBuf, process::ExitCode};

use anyhow::Result;
use clap::*;
use colored::Colorize as _;
use env_logger::Env;

use log::info;
use ostool::{
    board,
    build::{self, CargoQemuRunnerArgs, CargoRunnerKind, CargoUbootRunnerArgs},
    invocation::{Invocation, InvocationOptions},
    menuconfig::{MenuConfigHandler, MenuConfigMode},
    run::{
        qemu::{QemuConfig, RunQemuOptions},
        uboot::UbootConfig,
    },
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
        #[command(flatten)]
        cargo_selector: CargoSelectorArgs,
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
    Connect(BoardConnectArgs),
    Run(BoardRunArgs),
    Config,
}

#[derive(Args, Debug)]
struct RunQemuCommand {
    /// Path to the build configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,
    #[command(flatten)]
    cargo_selector: CargoSelectorArgs,
    #[command(flatten)]
    qemu: QemuArgs,
}

#[derive(Args, Debug)]
struct RunUbootCommand {
    /// Path to the build configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,
    #[command(flatten)]
    cargo_selector: CargoSelectorArgs,
    #[command(flatten)]
    uboot: UbootArgs,
}

#[derive(Args, Debug, Default, Clone)]
struct CargoSelectorArgs {
    /// Override the Cargo package from the build configuration
    #[arg(long)]
    package: Option<String>,
    /// Select a Cargo binary target within the selected package
    #[arg(long)]
    bin: Option<String>,
}

impl CargoSelectorArgs {
    fn is_empty(&self) -> bool {
        self.package.is_none() && self.bin.is_none()
    }
}

#[derive(Args, Debug)]
struct BoardRunArgs {
    /// Path to the build configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,
    #[command(flatten)]
    cargo_selector: CargoSelectorArgs,
    /// Path to the board runner configuration file, defaults to `pwd/.board.toml`
    #[arg(long = "board-config")]
    board_config: Option<PathBuf>,
    /// Override board type from the board runner configuration
    #[arg(short = 'b', long)]
    board_type: Option<String>,
    #[command(flatten)]
    server: BoardServerArgs,
}

#[derive(Args, Debug)]
struct BoardConnectArgs {
    /// Board type to allocate and connect
    #[arg(short = 'b', long)]
    board_type: String,
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

/// Parses the CLI and dispatches the selected ostool subcommand.
async fn try_main() -> Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info")).init();

    let Cli { manifest, command } = Cli::parse();

    match command {
        SubCommands::Board(args) => match args.command {
            BoardSubCommands::Ls(server) => {
                let global_config = board::load_board_global_config_with_notice()?;
                let (server, port) =
                    global_config.resolve_server(server.server.as_deref(), server.port);
                board::list_boards(&server, port).await?;
            }
            BoardSubCommands::Connect(args) => {
                let global_config = board::load_board_global_config_with_notice()?;
                let (server, port) =
                    global_config.resolve_server(args.server.server.as_deref(), args.server.port);
                board::connect_board(&server, port, &args.board_type).await?;
            }
            BoardSubCommands::Run(args) => {
                let mut invocation = init_invocation(manifest.clone())?;
                let mut loaded_build_config =
                    load_build_config(&invocation, args.config.as_deref()).await?;
                apply_cargo_selector(
                    &mut invocation,
                    &mut loaded_build_config.config,
                    loaded_build_config.path.as_path(),
                    &args.cargo_selector,
                )?;
                let board_config =
                    load_board_config(&mut invocation, args.board_config.as_deref()).await?;
                board::run_board(
                    &mut invocation,
                    &loaded_build_config.config,
                    Some(loaded_build_config.path.as_path()),
                    &board_config,
                    board::RunBoardOptions {
                        board_type: args.board_type,
                        server: args.server.server,
                        port: args.server.port,
                    },
                )
                .await?;
            }
            BoardSubCommands::Config => {
                board::config()?;
            }
        },
        SubCommands::Build {
            config,
            cargo_selector,
        } => {
            let mut invocation = init_invocation(manifest)?;
            let mut loaded_build_config = load_build_config(&invocation, config.as_deref()).await?;
            apply_cargo_selector(
                &mut invocation,
                &mut loaded_build_config.config,
                loaded_build_config.path.as_path(),
                &cargo_selector,
            )?;
            build::build_with_config(
                &mut invocation,
                &loaded_build_config.config,
                Some(loaded_build_config.path.as_path()),
            )
            .await?;
        }
        SubCommands::Run { command } => match command {
            RunSubCommands::Qemu(args) => {
                let RunQemuCommand {
                    config,
                    cargo_selector,
                    qemu,
                } = args;
                let debug = qemu.debug;
                let dtb_dump = qemu.dtb_dump;

                let mut invocation = init_invocation(manifest.clone())?;
                let mut loaded_build_config =
                    load_build_config(&invocation, config.as_deref()).await?;
                apply_cargo_selector(
                    &mut invocation,
                    &mut loaded_build_config.config,
                    loaded_build_config.path.as_path(),
                    &cargo_selector,
                )?;
                match &loaded_build_config.config.system {
                    build::config::BuildSystem::Cargo(config) => {
                        let qemu_config = match qemu.qemu_config.as_deref() {
                            Some(path) => Some(
                                ostool::run::qemu::read_config_from_path_for_cargo(
                                    &invocation,
                                    config,
                                    path,
                                )
                                .await?,
                            ),
                            None => None,
                        };
                        let kind = CargoRunnerKind::new_qemu(CargoQemuRunnerArgs {
                            qemu: qemu_config,
                            debug,
                            dtb_dump,
                        });
                        build::cargo_run(
                            &mut invocation,
                            config,
                            Some(loaded_build_config.path.as_path()),
                            &kind,
                        )
                        .await?;
                    }
                    build::config::BuildSystem::Custom(custom_cfg) => {
                        build::build_with_config(
                            &mut invocation,
                            &loaded_build_config.config,
                            Some(loaded_build_config.path.as_path()),
                        )
                        .await?;
                        invocation
                            .prepare_elf_artifact(
                                custom_cfg.elf_path.clone().into(),
                                custom_cfg.to_bin,
                            )
                            .await?;
                        let qemu_config =
                            load_qemu_config(&mut invocation, qemu.qemu_config.as_deref()).await?;
                        ostool::run::qemu::run_qemu(
                            &mut invocation,
                            &qemu_config,
                            RunQemuOptions { dtb_dump },
                        )
                        .await?;
                    }
                }
            }
            RunSubCommands::Uboot(args) => {
                let RunUbootCommand {
                    config,
                    cargo_selector,
                    uboot,
                } = args;

                let mut invocation = init_invocation(manifest.clone())?;
                let mut loaded_build_config =
                    load_build_config(&invocation, config.as_deref()).await?;
                apply_cargo_selector(
                    &mut invocation,
                    &mut loaded_build_config.config,
                    loaded_build_config.path.as_path(),
                    &cargo_selector,
                )?;
                match &loaded_build_config.config.system {
                    build::config::BuildSystem::Cargo(config) => {
                        let uboot_config = match uboot.uboot_config.as_deref() {
                            Some(path) => Some(
                                ostool::run::uboot::read_config_from_path_for_cargo(
                                    &invocation,
                                    config,
                                    path,
                                )
                                .await?,
                            ),
                            None => None,
                        };
                        let kind = CargoRunnerKind::new_uboot(CargoUbootRunnerArgs {
                            uboot: uboot_config,
                        });
                        build::cargo_run(
                            &mut invocation,
                            config,
                            Some(loaded_build_config.path.as_path()),
                            &kind,
                        )
                        .await?;
                    }
                    build::config::BuildSystem::Custom(custom_cfg) => {
                        build::build_with_config(
                            &mut invocation,
                            &loaded_build_config.config,
                            Some(loaded_build_config.path.as_path()),
                        )
                        .await?;
                        invocation
                            .prepare_elf_artifact(
                                custom_cfg.elf_path.clone().into(),
                                custom_cfg.to_bin,
                            )
                            .await?;
                        let uboot_config =
                            load_uboot_config(&mut invocation, uboot.uboot_config.as_deref())
                                .await?;
                        ostool::run::uboot::run_uboot(&mut invocation, &uboot_config).await?;
                    }
                }
            }
        },
        SubCommands::Menuconfig { mode } => {
            let mut invocation = init_invocation(manifest)?;
            MenuConfigHandler::handle_menuconfig(&mut invocation, mode).await?;
        }
    }

    Ok(())
}

/// Creates the invocation state from an optional manifest argument.
fn init_invocation(manifest_arg: Option<PathBuf>) -> Result<Invocation> {
    let invocation = Invocation::new(InvocationOptions::new(
        manifest_arg.clone(),
        None,
        None,
        false,
    ))?;
    info!("Using manifest {}", invocation.manifest_path().display());
    Ok(invocation)
}

struct LoadedBuildConfig {
    config: build::config::BuildConfig,
    path: PathBuf,
}

/// Loads the build config from an explicit path or workspace default.
async fn load_build_config(
    invocation: &Invocation,
    config_path: Option<&std::path::Path>,
) -> Result<LoadedBuildConfig> {
    let path = match config_path {
        Some(path) => path.to_path_buf(),
        None => invocation.workspace_dir().join(".build.toml"),
    };
    let config = build::load_build_config_from_path(invocation, &path, false).await?;
    Ok(LoadedBuildConfig { config, path })
}

/// Applies `--package` and `--bin` overrides to Cargo build configs.
fn apply_cargo_selector(
    invocation: &mut Invocation,
    build_config: &mut build::config::BuildConfig,
    build_config_path: &std::path::Path,
    selector: &CargoSelectorArgs,
) -> Result<()> {
    if !selector.is_empty() {
        let build::config::BuildSystem::Cargo(cargo_config) = &mut build_config.system else {
            anyhow::bail!("--package/--bin can only be used with system.Cargo build configs");
        };

        if let Some(package) = &selector.package {
            cargo_config.package = package.clone();
        }
        if let Some(bin) = &selector.bin {
            cargo_config.bin = Some(bin.clone());
        }
    }

    build::activate_build_config(invocation, build_config, Some(build_config_path))
}

/// Loads QEMU config from an explicit path or workspace default.
async fn load_qemu_config(
    invocation: &mut Invocation,
    config_path: Option<&std::path::Path>,
) -> Result<QemuConfig> {
    match config_path {
        Some(path) => ostool::run::qemu::read_config_from_path(invocation, path).await,
        None => {
            let workspace_dir = invocation.workspace_dir().to_path_buf();
            ostool::run::qemu::ensure_config_in_dir(invocation, &workspace_dir).await
        }
    }
}

/// Loads U-Boot config from an explicit path or workspace default.
async fn load_uboot_config(
    invocation: &mut Invocation,
    config_path: Option<&std::path::Path>,
) -> Result<UbootConfig> {
    match config_path {
        Some(path) => ostool::run::uboot::read_config_from_path(invocation, path).await,
        None => {
            let workspace_dir = invocation.workspace_dir().to_path_buf();
            ostool::run::uboot::ensure_config_in_dir(invocation, &workspace_dir).await
        }
    }
}

/// Loads board-run config from an explicit path or workspace default.
async fn load_board_config(
    invocation: &mut Invocation,
    config_path: Option<&std::path::Path>,
) -> Result<board::config::BoardRunConfig> {
    match config_path {
        Some(path) => board::read_run_config_from_path(invocation, path).await,
        None => {
            let workspace_dir = invocation.workspace_dir().to_path_buf();
            board::ensure_run_config_in_dir(invocation, &workspace_dir).await
        }
    }
}

/// Prints CLI errors with a structured trace.
fn report_error(err: &anyhow::Error) {
    log::error!("{err:#}");
    log::error!("Trace:\n{err:?}");

    println!("{}", format!("Error: {err:#}").red().bold());
    println!("{}", format!("\nTrace:\n{err:?}").red());
}

#[cfg(test)]
mod tests {
    use std::fs;

    use clap::Parser;
    use ostool::invocation::{Invocation, InvocationOptions};

    use super::{
        BoardArgs, BoardSubCommands, CargoSelectorArgs, Cli, RunSubCommands, SubCommands,
        apply_cargo_selector, build, load_board_config, load_build_config, load_qemu_config,
        load_uboot_config,
    };

    /// Verifies build parsing accepts manifest, config, package, and bin overrides.
    #[test]
    fn parse_build_with_manifest_config_package_and_bin() {
        let cli = Cli::try_parse_from([
            "ostool",
            "--manifest",
            "examples/kernel/Cargo.toml",
            "build",
            "--config",
            "kernel.build.toml",
            "--package",
            "kernel",
            "--bin",
            "kernel-qemu",
        ])
        .unwrap();

        assert_eq!(
            cli.manifest.as_deref(),
            Some(std::path::Path::new("examples/kernel/Cargo.toml"))
        );
        match cli.command {
            SubCommands::Build {
                config,
                cargo_selector,
            } => {
                assert_eq!(
                    config.as_deref(),
                    Some(std::path::Path::new("kernel.build.toml"))
                );
                assert_eq!(cargo_selector.package.as_deref(), Some("kernel"));
                assert_eq!(cargo_selector.bin.as_deref(), Some("kernel-qemu"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    /// Verifies QEMU run parsing accepts build, QEMU, and Cargo selector args.
    #[test]
    fn parse_run_qemu_with_build_qemu_and_cargo_selector_args() {
        let cli = Cli::try_parse_from([
            "ostool",
            "run",
            "qemu",
            "--config",
            "kernel.build.toml",
            "--qemu-config",
            "kernel.qemu.toml",
            "--debug",
            "--dtb-dump",
            "--package",
            "kernel",
            "--bin",
            "kernel-qemu",
        ])
        .unwrap();

        match cli.command {
            SubCommands::Run {
                command: RunSubCommands::Qemu(args),
            } => {
                assert_eq!(
                    args.config.as_deref(),
                    Some(std::path::Path::new("kernel.build.toml"))
                );
                assert_eq!(args.cargo_selector.package.as_deref(), Some("kernel"));
                assert_eq!(args.cargo_selector.bin.as_deref(), Some("kernel-qemu"));
                assert_eq!(
                    args.qemu.qemu_config.as_deref(),
                    Some(std::path::Path::new("kernel.qemu.toml"))
                );
                assert!(args.qemu.debug);
                assert!(args.qemu.dtb_dump);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    /// Verifies U-Boot run parsing accepts build, U-Boot, and Cargo selector args.
    #[test]
    fn parse_run_uboot_with_build_uboot_and_cargo_selector_args() {
        let cli = Cli::try_parse_from([
            "ostool",
            "run",
            "uboot",
            "--config",
            "kernel.build.toml",
            "--uboot-config",
            "kernel.uboot.toml",
            "--package",
            "kernel",
            "--bin",
            "kernel-uboot",
        ])
        .unwrap();

        match cli.command {
            SubCommands::Run {
                command: RunSubCommands::Uboot(args),
            } => {
                assert_eq!(
                    args.config.as_deref(),
                    Some(std::path::Path::new("kernel.build.toml"))
                );
                assert_eq!(args.cargo_selector.package.as_deref(), Some("kernel"));
                assert_eq!(args.cargo_selector.bin.as_deref(), Some("kernel-uboot"));
                assert_eq!(
                    args.uboot.uboot_config.as_deref(),
                    Some(std::path::Path::new("kernel.uboot.toml"))
                );
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

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
    fn parse_board_connect_with_short_board_type() {
        let cli = Cli::try_parse_from(["ostool", "board", "connect", "-b", "rk3568"]).unwrap();

        match cli.command {
            SubCommands::Board(BoardArgs {
                command: BoardSubCommands::Connect(args),
            }) => {
                assert_eq!(args.board_type, "rk3568");
                assert!(args.server.server.is_none());
                assert!(args.server.port.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_board_connect_with_long_args() {
        let cli = Cli::try_parse_from([
            "ostool",
            "board",
            "connect",
            "--board-type",
            "rk3568",
            "--server",
            "10.0.0.2",
            "--port",
            "9000",
        ])
        .unwrap();

        match cli.command {
            SubCommands::Board(BoardArgs {
                command: BoardSubCommands::Connect(args),
            }) => {
                assert_eq!(args.board_type, "rk3568");
                assert_eq!(args.server.server.as_deref(), Some("10.0.0.2"));
                assert_eq!(args.server.port, Some(9000));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn parse_board_connect_requires_board_type() {
        let err = Cli::try_parse_from(["ostool", "board", "connect"]).unwrap_err();
        let rendered = err.to_string();
        assert!(rendered.contains("--board-type"));
    }

    #[test]
    fn parse_board_run_defaults_to_no_overrides() {
        let cli = Cli::try_parse_from(["ostool", "board", "run"]).unwrap();

        match cli.command {
            SubCommands::Board(BoardArgs {
                command: BoardSubCommands::Run(args),
            }) => {
                assert!(args.config.is_none());
                assert!(args.board_config.is_none());
                assert!(args.board_type.is_none());
                assert!(args.server.server.is_none());
                assert!(args.server.port.is_none());
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    /// Verifies board run parsing accepts build and board config overrides.
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
            "--board-type",
            "rk3568",
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
                assert!(args.cargo_selector.package.is_none());
                assert!(args.cargo_selector.bin.is_none());
                assert_eq!(args.board_type.as_deref(), Some("rk3568"));
                assert_eq!(args.server.server.as_deref(), Some("10.0.0.2"));
                assert_eq!(args.server.port, Some(9000));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    /// Verifies board run parsing accepts Cargo package and bin selectors.
    #[test]
    fn parse_board_run_with_cargo_selector_args() {
        let cli = Cli::try_parse_from([
            "ostool",
            "board",
            "run",
            "--package",
            "kernel",
            "--bin",
            "kernel-board",
        ])
        .unwrap();

        match cli.command {
            SubCommands::Board(BoardArgs {
                command: BoardSubCommands::Run(args),
            }) => {
                assert_eq!(args.cargo_selector.package.as_deref(), Some("kernel"));
                assert_eq!(args.cargo_selector.bin.as_deref(), Some("kernel-board"));
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }

    #[test]
    fn apply_cargo_selector_overrides_cargo_build_config() {
        let (_temp, mut invocation) = test_invocation();
        let mut build_config = build::config::BuildConfig {
            system: build::config::BuildSystem::Cargo(build::config::Cargo {
                package: "default-package".into(),
                bin: None,
                ..Default::default()
            }),
        };

        apply_cargo_selector(
            &mut invocation,
            &mut build_config,
            _temp.path().join(".build.toml").as_path(),
            &CargoSelectorArgs {
                package: Some("kernel".into()),
                bin: Some("kernel-qemu".into()),
            },
        )
        .unwrap();

        match &build_config.system {
            build::config::BuildSystem::Cargo(cargo) => {
                assert_eq!(cargo.package, "kernel");
                assert_eq!(cargo.bin.as_deref(), Some("kernel-qemu"));
            }
            other => panic!("unexpected build system: {other:?}"),
        }
    }

    #[test]
    fn apply_cargo_selector_rejects_custom_build_config() {
        let (_temp, mut invocation) = test_invocation();
        let mut build_config = build::config::BuildConfig {
            system: build::config::BuildSystem::Custom(build::config::Custom {
                build_cmd: "make".into(),
                elf_path: "target/kernel.elf".into(),
                to_bin: true,
            }),
        };

        let err = apply_cargo_selector(
            &mut invocation,
            &mut build_config,
            _temp.path().join(".build.toml").as_path(),
            &CargoSelectorArgs {
                package: Some("kernel".into()),
                bin: None,
            },
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--package/--bin can only be used with system.Cargo")
        );
    }

    #[tokio::test]
    async fn cargo_selector_updates_scope_before_board_config_load() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"kernel\"]\nresolver = \"3\"\n",
        )
        .unwrap();

        let app_dir = temp.path().join("app");
        fs::create_dir_all(app_dir.join("src")).unwrap();
        fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let kernel_dir = temp.path().join("kernel");
        fs::create_dir_all(kernel_dir.join("src/bin")).unwrap();
        fs::write(
            kernel_dir.join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(kernel_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(kernel_dir.join("src/bin/kernel-board.rs"), "fn main() {}\n").unwrap();

        fs::write(
            temp.path().join(".board.toml"),
            r#"
board_type = "kernel-board"
dtb_file = "${package}/board.dtb"
"#,
        )
        .unwrap();

        let mut invocation =
            Invocation::new(InvocationOptions::new(Some(app_dir), None, None, false)).unwrap();
        let mut build_config = build::config::BuildConfig {
            system: build::config::BuildSystem::Cargo(build::config::Cargo {
                package: "app".into(),
                target: "aarch64-unknown-none".into(),
                ..Default::default()
            }),
        };

        apply_cargo_selector(
            &mut invocation,
            &mut build_config,
            temp.path().join(".build.toml").as_path(),
            &CargoSelectorArgs {
                package: Some("kernel".into()),
                bin: Some("kernel-board".into()),
            },
        )
        .unwrap();
        let board_config = load_board_config(&mut invocation, None).await.unwrap();

        let expected = kernel_dir.join("board.dtb").display().to_string();
        assert_eq!(board_config.dtb_file.as_deref(), Some(expected.as_str()));
    }

    #[tokio::test]
    async fn cargo_selector_overrides_config_package_before_activation() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"kernel\"]\nresolver = \"3\"\n",
        )
        .unwrap();

        let app_dir = temp.path().join("app");
        fs::create_dir_all(app_dir.join("src")).unwrap();
        fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let kernel_dir = temp.path().join("kernel");
        fs::create_dir_all(kernel_dir.join("src/bin")).unwrap();
        fs::write(
            kernel_dir.join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(kernel_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(kernel_dir.join("src/bin/kernel-board.rs"), "fn main() {}\n").unwrap();

        let build_config_path = temp.path().join(".build.toml");
        fs::write(
            &build_config_path,
            r#"
[system.Cargo]
package = "placeholder"
target = "aarch64-unknown-none"
features = []
log = "Info"
env = {}
args = []
pre_build_cmds = []
post_build_cmds = []
to_bin = false
disable_someboot_build_config = true
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join(".board.toml"),
            r#"
board_type = "kernel-board"
dtb_file = "${package}/board.dtb"
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join(".qemu.toml"),
            r#"
args = ["${package}/kernel"]
uefi = false
to_bin = false
success_regex = []
fail_regex = []
"#,
        )
        .unwrap();
        fs::write(
            temp.path().join(".uboot.toml"),
            r#"
dtb_file = "${package}/board.dtb"
success_regex = []
fail_regex = []
"#,
        )
        .unwrap();

        let mut invocation =
            Invocation::new(InvocationOptions::new(Some(app_dir), None, None, false)).unwrap();
        let mut loaded = load_build_config(&invocation, Some(&build_config_path))
            .await
            .unwrap();

        apply_cargo_selector(
            &mut invocation,
            &mut loaded.config,
            loaded.path.as_path(),
            &CargoSelectorArgs {
                package: Some("kernel".into()),
                bin: Some("kernel-board".into()),
            },
        )
        .unwrap();

        let expected_dtb = kernel_dir.join("board.dtb").display().to_string();

        let board_config = load_board_config(&mut invocation, None).await.unwrap();
        assert_eq!(board_config.board_type, "kernel-board");
        assert_eq!(
            board_config.dtb_file.as_deref(),
            Some(expected_dtb.as_str())
        );

        let qemu_config = load_qemu_config(&mut invocation, None).await.unwrap();
        assert_eq!(
            qemu_config.args,
            vec![kernel_dir.join("kernel").display().to_string()]
        );

        let uboot_config = load_uboot_config(&mut invocation, None).await.unwrap();
        assert_eq!(
            uboot_config.dtb_file.as_deref(),
            Some(expected_dtb.as_str())
        );
    }

    fn test_invocation() -> (tempfile::TempDir, Invocation) {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/lib.rs"), "").unwrap();
        let invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap();
        (temp, invocation)
    }

    #[test]
    fn parse_menuconfig_httpboot_command() {
        let cli = Cli::try_parse_from(["ostool", "menuconfig", "httpboot"]).unwrap();

        match cli.command {
            SubCommands::Menuconfig { mode } => {
                assert!(matches!(mode, Some(super::MenuConfigMode::Httpboot)));
            }
            other => panic!("expected menuconfig command, got {other:?}"),
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
