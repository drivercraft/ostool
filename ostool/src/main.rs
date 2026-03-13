use std::{env::current_dir, path::PathBuf, process::ExitCode};

use anyhow::{Context, Result};
use clap::*;

use log::info;
use ostool::{
    build::{self, CargoRunnerKind},
    ctx::AppContext,
    menuconfig::{MenuConfigHandler, MenuConfigMode},
    run::{qemu::RunQemuArgs, uboot::RunUbootArgs},
};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    workdir: Option<PathBuf>,
    #[command(subcommand)]
    command: SubCommands,
}

#[derive(Subcommand)]
enum SubCommands {
    Build {
        /// Path to the build configuration file
        #[arg(short, long)]
        config: Option<PathBuf>,
    },
    Run(RunArgs),
    Menuconfig {
        /// Menu configuration mode (qemu or uboot)
        #[arg(value_enum)]
        mode: Option<MenuConfigMode>,
    },
}

#[derive(Args, Debug)]
struct RunArgs {
    /// Path to the build configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: RunSubCommands,
}

#[derive(Subcommand, Debug)]
enum RunSubCommands {
    Qemu(QemuArgs),
    Uboot(UbootArgs),
}

#[derive(Args, Debug, Default)]
pub struct QemuArgs {
    /// Path to the qemu configuration file, default to '.qemu.toml'
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
            eprintln!("Error: {err:#}");
            eprintln!("\nTrace:\n{err:?}");
            ExitCode::FAILURE
        }
    }
}

async fn try_main() -> Result<()> {
    #[cfg(not(feature = "ui-log"))]
    {
        env_logger::builder()
            .filter_level(log::LevelFilter::Info)
            .parse_default_env()
            .init();
    }

    let cli = Cli::parse();

    let pwd = current_dir().context("failed to get current working directory")?;

    let workspace_folder = match cli.workdir {
        Some(dir) => dir,
        None => pwd.clone(),
    };

    let mut ctx = AppContext {
        paths: ostool::ctx::PathConfig {
            workspace: workspace_folder.clone(),
            manifest: workspace_folder.clone(),
            ..Default::default()
        },
        ..Default::default()
    };

    match cli.command {
        SubCommands::Build { config } => {
            ctx.build(config).await?;
        }
        SubCommands::Run(args) => {
            let config = ctx.prepare_build_config(args.config, false).await?;
            match config.system {
                build::config::BuildSystem::Cargo(config) => {
                    let kind = match args.command {
                        RunSubCommands::Qemu(qemu_args) => CargoRunnerKind::Qemu {
                            qemu_config: qemu_args.qemu_config,
                            debug: qemu_args.debug,
                            dtb_dump: qemu_args.dtb_dump,
                        },
                        RunSubCommands::Uboot(uboot_args) => CargoRunnerKind::Uboot {
                            uboot_config: uboot_args.uboot_config,
                        },
                    };
                    ctx.cargo_run(&config, &kind).await?;
                }
                build::config::BuildSystem::Custom(custom_cfg) => {
                    ctx.shell_run_cmd(&custom_cfg.build_cmd)?;
                    ctx.set_elf_path(custom_cfg.elf_path.clone().into()).await?;
                    info!(
                        "ELF {:?}: {}",
                        ctx.arch,
                        ctx.paths.artifacts.elf.as_ref().unwrap().display()
                    );

                    if custom_cfg.to_bin {
                        ctx.objcopy_output_bin()?;
                    }

                    match args.command {
                        RunSubCommands::Qemu(qemu_args) => {
                            ostool::run::qemu::run_qemu(
                                ctx,
                                RunQemuArgs {
                                    qemu_config: qemu_args.qemu_config,
                                    dtb_dump: qemu_args.dtb_dump,
                                    show_output: true,
                                },
                            )
                            .await?;
                        }
                        RunSubCommands::Uboot(uboot_args) => {
                            ostool::run::uboot::run_uboot(
                                ctx,
                                RunUbootArgs {
                                    config: uboot_args.uboot_config,
                                    show_output: true,
                                },
                            )
                            .await?;
                        }
                    }
                }
            }
        }
        SubCommands::Menuconfig { mode } => {
            MenuConfigHandler::handle_menuconfig(&mut ctx, mode).await?;
        }
    }

    Ok(())
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
