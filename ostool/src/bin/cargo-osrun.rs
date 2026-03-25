use std::{
    env,
    path::PathBuf,
    process::{ExitCode, exit},
    sync::OnceLock,
};

use clap::{Parser, Subcommand};
use colored::Colorize as _;
use log::debug;
use ostool::{
    Tool, ToolConfig, logger, resolve_manifest_context,
    run::{qemu, uboot::RunUbootArgs},
};

#[derive(Debug, Parser, Clone)]
struct RunnerArgs {
    program: PathBuf,

    /// Path to the binary to run on the device
    elf: PathBuf,

    /// Test name
    test_name: Option<String>,

    /// Objcopy elf to binary before running
    #[arg(long("to-bin"))]
    to_bin: bool,

    #[arg(short)]
    /// Enable verbose output
    verbose: bool,

    #[arg(short)]
    /// Enable quiet output (no output except errors)
    quiet: bool,

    /// Path to the runner configuration file
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[arg(long("show-output"))]
    show_output: bool,

    #[arg(long)]
    no_run: bool,

    #[arg(long)]
    debug: bool,

    /// Sub-commands
    #[command(subcommand)]
    command: Option<SubCommands>,

    /// Dump DTB file
    #[arg(long)]
    dtb_dump: bool,

    #[arg(allow_hyphen_values = true)]
    /// Arguments to be run
    runner_args: Vec<String>,

    #[arg(long)]
    build_dir: Option<String>,

    #[arg(long)]
    bin_dir: Option<String>,
}

#[derive(Debug, Subcommand, Clone)]
enum SubCommands {
    Uboot(CliUboot),
}

static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

#[derive(Debug, Parser, Clone)]
struct CliUboot {
    #[arg(allow_hyphen_values = true)]
    runner_args: Vec<String>,
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

async fn try_main() -> anyhow::Result<()> {
    let args = RunnerArgs::parse();
    if env::var("CARGO").is_err() {
        println!(
            "{}",
            "This binary may only be called via `cargo ndk-runner`."
                .red()
                .bold()
        );
        exit(1);
    }

    let manifest_dir: PathBuf = env::var("CARGO_MANIFEST_DIR")?.into();
    let manifest = manifest_dir.join("Cargo.toml");
    let manifest = resolve_manifest_context(Some(manifest))?;
    let log_path = logger::init_file_logger(&manifest.workspace_dir)?;
    let _ = LOG_PATH.set(log_path.clone());
    debug!(
        "Logging initialized at {} for manifest {}",
        log_path.display(),
        manifest.manifest_path.display()
    );
    debug!("Parsed arguments: {:#?}", args);

    if args.no_run {
        exit(0);
    }

    let bin_dir: Option<PathBuf> = args.bin_dir.map(PathBuf::from);
    let build_dir: Option<PathBuf> = args.build_dir.map(PathBuf::from);

    let mut tool = Tool::new(ToolConfig {
        manifest: Some(manifest.manifest_path),
        build_dir,
        bin_dir,
        debug: args.debug,
    })?;

    tool.set_elf_path(args.elf).await?;
    tool.objcopy_elf()?;

    if args.to_bin {
        tool.objcopy_output_bin()?;
    }

    match args.command {
        Some(SubCommands::Uboot(_)) => {
            tool.run_uboot(RunUbootArgs {
                config: args.config,
                show_output: args.show_output,
            })
            .await?;
        }
        None => {
            tool.run_qemu(qemu::RunQemuArgs {
                qemu_config: args.config,
                dtb_dump: args.dtb_dump,
                show_output: args.show_output,
            })
            .await?;
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
