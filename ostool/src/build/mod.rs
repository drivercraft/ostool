//! Build system configuration and Cargo integration.
//!
//! This module provides functionality for building operating system projects
//! using Cargo or custom build commands. It supports:
//!
//! - Configuring build options via TOML configuration files
//! - Running pre-build and post-build shell commands
//! - Automatic feature detection and configuration
//! - Multiple runner types (QEMU, U-Boot)
//!
//! # Example
//!
//! ```rust,no_run
//! use ostool::build::config::{BuildConfig, BuildSystem, Cargo};
//! use ostool::Tool;
//!
//! // Build configurations are typically loaded from TOML files
//! // See .build.toml for example configuration format
//! ```

use std::path::PathBuf;

use crate::{
    Tool,
    build::{
        cargo_builder::CargoBuilder,
        config::{Cargo, Custom},
    },
    run::{
        qemu::{RunQemuArgs, resolve_qemu_config_path_in_dir},
        uboot::RunUbootArgs,
    },
};

/// Cargo builder implementation for building projects.
pub mod cargo_builder;

/// Build configuration types and structures.
pub mod config;

pub mod someboot;

/// Specifies the type of runner to use after building.
///
/// This enum determines how the built artifact will be executed,
/// either through QEMU emulation or via U-Boot on real hardware.
pub enum CargoRunnerKind {
    /// Run the built artifact in QEMU emulator.
    Qemu {
        /// Optional path to QEMU configuration file.
        qemu_config: Option<PathBuf>,
        /// Whether to enable debug mode (GDB server).
        debug: bool,
        /// Whether to dump the device tree blob.
        dtb_dump: bool,
        /// Optional override for the generated QEMU config `to_bin` default.
        to_bin: Option<bool>,
        /// Extra default QEMU command-line arguments.
        args: Vec<String>,
        /// Regex patterns that indicate successful execution.
        success_regex: Vec<String>,
        /// Regex patterns that indicate failed execution.
        fail_regex: Vec<String>,
    },
    /// Run the built artifact on real hardware via U-Boot.
    Uboot {
        /// Optional path to U-Boot configuration file.
        uboot_config: Option<PathBuf>,
    },
}

impl Tool {
    /// Builds the project using the specified build configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - The build configuration specifying how to build the project.
    ///
    /// # Errors
    ///
    /// Returns an error if the build process fails.
    pub async fn build_with_config(&mut self, config: &config::BuildConfig) -> anyhow::Result<()> {
        match &config.system {
            config::BuildSystem::Custom(custom) => self.build_custom(custom)?,
            config::BuildSystem::Cargo(cargo) => {
                self.cargo_build(cargo).await?;
            }
        }
        Ok(())
    }

    /// Builds the project from the specified configuration file path.
    ///
    /// This is the main entry point for building projects. It loads the
    /// configuration from the specified path (or default `.build.toml`)
    /// and executes the build.
    ///
    /// # Arguments
    ///
    /// * `config_path` - Optional path to the build configuration file.
    ///   Defaults to `.build.toml` in the workspace directory.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be loaded or the build fails.
    pub async fn build(&mut self, config_path: Option<PathBuf>) -> anyhow::Result<()> {
        let build_config = self.prepare_build_config(config_path, false).await?;
        println!("Build configuration: {:?}", build_config);
        self.build_with_config(&build_config).await
    }

    /// Executes a custom build using shell commands.
    ///
    /// # Arguments
    ///
    /// * `config` - Custom build configuration containing the shell command.
    ///
    /// # Errors
    ///
    /// Returns an error if the shell command fails.
    pub fn build_custom(&mut self, config: &Custom) -> anyhow::Result<()> {
        self.shell_run_cmd(&config.build_cmd)?;
        Ok(())
    }

    /// Builds the project using Cargo.
    ///
    /// # Arguments
    ///
    /// * `config` - Cargo build configuration.
    ///
    /// # Errors
    ///
    /// Returns an error if the Cargo build fails.
    pub async fn cargo_build(&mut self, config: &Cargo) -> anyhow::Result<()> {
        cargo_builder::CargoBuilder::build_auto(self, config)
            .execute()
            .await
    }

    /// Builds and runs the project using Cargo with the specified runner.
    ///
    /// # Arguments
    ///
    /// * `config` - Cargo build configuration.
    /// * `runner` - The type of runner to use (QEMU or U-Boot).
    ///
    /// # Errors
    ///
    /// Returns an error if the build or run fails.
    pub async fn cargo_run(
        &mut self,
        config: &Cargo,
        runner: &CargoRunnerKind,
    ) -> anyhow::Result<()> {
        let build_config_path = self.ctx.build_config_path.clone();

        let debug = matches!(runner, CargoRunnerKind::Qemu { debug: true, .. });

        CargoBuilder::build(self, config, build_config_path)
            .debug(debug)
            .skip_objcopy(true)
            .resolve_artifact_from_json(true)
            .execute()
            .await?;

        match runner {
            CargoRunnerKind::Qemu {
                qemu_config,
                dtb_dump,
                to_bin,
                args,
                success_regex,
                fail_regex,
                ..
            } => {
                let package_dir = self.resolve_package_manifest_dir(&config.package)?;
                let resolved_qemu_config = resolve_qemu_config_path_in_dir(
                    &package_dir,
                    self.ctx.arch,
                    qemu_config.clone(),
                )?;

                self.run_qemu_with_more_default_args(
                    RunQemuArgs {
                        qemu_config: Some(resolved_qemu_config),
                        dtb_dump: *dtb_dump,
                        show_output: true,
                    },
                    *to_bin,
                    args.clone(),
                    success_regex.clone(),
                    fail_regex.clone(),
                )
                .await?;
            }
            CargoRunnerKind::Uboot { uboot_config } => {
                self.run_uboot(RunUbootArgs {
                    config: uboot_config.clone(),
                    show_output: true,
                })
                .await?;
            }
        }

        Ok(())
    }
}
