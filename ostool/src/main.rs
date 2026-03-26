use std::{path::PathBuf, process::ExitCode, sync::OnceLock};

use anyhow::Result;
use clap::*;
use colored::Colorize as _;

use log::info;
use ostool::{
    Tool, ToolConfig,
    build::{self, CargoQemuAppendArgs, CargoQemuOverrideArgs, CargoRunnerKind},
    logger,
    menuconfig::{MenuConfigHandler, MenuConfigMode},
    resolve_manifest_context,
    run::{qemu::RunQemuArgs, uboot::RunUbootArgs},
};

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Cli {
    #[arg(short, long)]
    manifest: Option<PathBuf>,
    #[command(subcommand)]
    command: SubCommands,
}

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

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
    let cli = Cli::parse();
    let manifest = resolve_manifest_context(cli.manifest.clone())?;
    let log_path = logger::init_file_logger(&manifest.workspace_dir)?;
    let _ = LOG_PATH.set(log_path.clone());
    info!(
        "Logging initialized at {} for manifest {}",
        log_path.display(),
        manifest.manifest_path.display()
    );

    let mut tool = Tool::new(ToolConfig {
        manifest: cli.manifest,
        ..Default::default()
    })?;

    match cli.command {
        SubCommands::Build { config } => {
            tool.build(config).await?;
        }
        SubCommands::Run(args) => {
            let config = tool.prepare_build_config(args.config, false).await?;
            match config.system {
                build::config::BuildSystem::Cargo(config) => {
                    let kind = match args.command {
                        RunSubCommands::Qemu(qemu_args) => CargoRunnerKind::Qemu {
                            qemu_config: qemu_args.qemu_config,
                            debug: qemu_args.debug,
                            dtb_dump: qemu_args.dtb_dump,
                            default_args: CargoQemuOverrideArgs::default(),
                            append_args: CargoQemuAppendArgs::default(),
                            override_args: CargoQemuOverrideArgs::default(),
                        },
                        RunSubCommands::Uboot(uboot_args) => CargoRunnerKind::Uboot {
                            uboot_config: uboot_args.uboot_config,
                        },
                    };
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

                    match args.command {
                        RunSubCommands::Qemu(qemu_args) => {
                            tool.run_qemu(RunQemuArgs {
                                qemu_config: qemu_args.qemu_config,
                                dtb_dump: qemu_args.dtb_dump,
                                show_output: true,
                            })
                            .await?;
                        }
                        RunSubCommands::Uboot(uboot_args) => {
                            tool.run_uboot(RunUbootArgs {
                                config: uboot_args.uboot_config,
                                show_output: true,
                            })
                            .await?;
                        }
                    }
                }
            }
        }
        SubCommands::Menuconfig { mode } => {
            MenuConfigHandler::handle_menuconfig(&mut tool, mode).await?;
        }
    }

    Ok(())
}

fn report_error(err: &anyhow::Error) {
    log::error!("{err:#}");
    log::error!("Trace:\n{err:?}");

    println!("{}", format!("Error: {err:#}").red().bold());
    println!("{}", format!("\nTrace:\n{err:?}").red());

    if let Some(log_path) = LOG_PATH.get() {
        println!(
            "{}",
            format!("Log file: {}", log_path.display()).yellow().bold()
        );
    }
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
