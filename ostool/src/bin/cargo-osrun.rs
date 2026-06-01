//! Cargo runner entrypoint that dispatches built ELF artifacts to QEMU or U-Boot.

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
    invocation::{Invocation, InvocationOptions},
    logger,
    run::qemu::RunQemuOptions,
};

#[derive(Debug, Parser, Clone)]
struct RunnerArgs {
    program: PathBuf,

    /// Path to the Cargo-built ELF artifact.
    elf: PathBuf,

    /// Test name
    test_name: Option<String>,

    /// Convert the ELF to a raw binary before running.
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

/// Parses Cargo runner arguments and starts the selected runtime backend.
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
    let parsed_args = format!("{args:#?}");

    let RunnerArgs {
        elf,
        to_bin,
        config,
        no_run,
        debug,
        command,
        dtb_dump,
        build_dir,
        bin_dir,
        ..
    } = args;
    let bin_dir: Option<PathBuf> = bin_dir.map(PathBuf::from);
    let build_dir: Option<PathBuf> = build_dir.map(PathBuf::from);

    let invocation = Invocation::new(InvocationOptions::new(
        Some(manifest),
        build_dir,
        bin_dir,
        debug,
    ))?;
    let log_path = logger::init_file_logger(invocation.workspace_dir())?;
    let _ = LOG_PATH.set(log_path.clone());
    debug!(
        "Logging initialized at {} for manifest {}",
        log_path.display(),
        invocation.manifest_path().display()
    );
    debug!("Parsed arguments: {parsed_args}");

    if no_run {
        exit(0);
    }

    let mut invocation = invocation;

    invocation.prepare_elf_artifact(elf, to_bin).await?;

    match command {
        Some(SubCommands::Uboot(_)) => {
            let config = match config.as_deref() {
                Some(path) => ostool::run::uboot::read_config_from_path(&invocation, path).await?,
                None => {
                    let workspace_dir = invocation.workspace_dir().to_path_buf();
                    ostool::run::uboot::ensure_config_in_dir(&invocation, &workspace_dir).await?
                }
            };
            ostool::run::uboot::run_uboot(&mut invocation, &config).await?;
        }
        None => {
            let config = match config.as_deref() {
                Some(path) => ostool::run::qemu::read_config_from_path(&invocation, path).await?,
                None => {
                    let workspace_dir = invocation.workspace_dir().to_path_buf();
                    ostool::run::qemu::ensure_config_in_dir(&invocation, &workspace_dir).await?
                }
            };
            ostool::run::qemu::run_qemu(&mut invocation, &config, RunQemuOptions { dtb_dump })
                .await?;
        }
    }

    Ok(())
}

/// Reports runner errors to logs, terminal output, and the file-log hint.
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

#[cfg(test)]
mod tests {
    use std::path::Path;

    use clap::Parser;

    use super::{RunnerArgs, SubCommands};

    /// Verifies the default runner path parses QEMU-related flags.
    #[test]
    fn parse_default_qemu_runner_args() {
        let args = RunnerArgs::try_parse_from([
            "cargo-osrun",
            "qemu-system-aarch64",
            "target/kernel.elf",
            "--to-bin",
            "--config",
            "qemu.toml",
            "--show-output",
            "--debug",
            "--dtb-dump",
            "--build-dir",
            "target/custom",
            "--bin-dir",
            "dist",
        ])
        .unwrap();

        assert_eq!(args.program, Path::new("qemu-system-aarch64"));
        assert_eq!(args.elf, Path::new("target/kernel.elf"));
        assert!(args.to_bin);
        assert_eq!(args.config.as_deref(), Some(Path::new("qemu.toml")));
        assert!(args.show_output);
        assert!(args.debug);
        assert!(args.dtb_dump);
        assert_eq!(args.build_dir.as_deref(), Some("target/custom"));
        assert_eq!(args.bin_dir.as_deref(), Some("dist"));
        assert!(args.command.is_none());
    }

    /// Verifies the U-Boot subcommand owns arguments after `--`.
    #[test]
    fn parse_uboot_runner_subcommand() {
        let args = RunnerArgs::try_parse_from([
            "cargo-osrun",
            "qemu-system-aarch64",
            "target/kernel.elf",
            "uboot",
            "--",
            "bootm",
            "${kernel_addr_r}",
        ])
        .unwrap();

        match args.command {
            Some(SubCommands::Uboot(uboot)) => {
                assert_eq!(uboot.runner_args, ["bootm", "${kernel_addr_r}"]);
            }
            other => panic!("unexpected command: {other:?}"),
        }
    }
}
