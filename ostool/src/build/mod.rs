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
//! use ostool::ctx::AppContext;
//!
//! // Build configurations are typically loaded from TOML files
//! // See .build.toml for example configuration format
//! ```

use std::path::PathBuf;

use anyhow::Context;

use crate::{
    build::{
        cargo_builder::CargoBuilder,
        config::{Cargo, Custom},
    },
    ctx::AppContext,
    run::{qemu::RunQemuArgs, uboot::RunUbootArgs},
    utils::PathResultExt,
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
    },
    /// Run the built artifact on real hardware via U-Boot.
    Uboot {
        /// Optional path to U-Boot configuration file.
        uboot_config: Option<PathBuf>,
    },
}

impl AppContext {
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
        let build_config_path = self.build_config_path.clone();

        let normalize = |dir: &PathBuf| -> anyhow::Result<PathBuf> {
            let bin_path = if dir.is_relative() {
                self.paths.manifest.join(dir)
            } else {
                dir.clone()
            };

            match bin_path.canonicalize() {
                Ok(path) => Ok(path),
                Err(file_err) => {
                    let Some(parent) = bin_path.parent() else {
                        return Err(file_err).with_path("failed to canonicalize path", &bin_path);
                    };
                    let Some(file_name) = bin_path.file_name() else {
                        return Err(file_err).with_path("failed to canonicalize path", &bin_path);
                    };

                    parent
                        .canonicalize()
                        .map(|parent_dir| parent_dir.join(file_name))
                        .with_path("failed to canonicalize parent path", parent)
                        .with_context(|| {
                            format!("failed to normalize path: {}", bin_path.display())
                        })
                }
            }
        };

        let build_dir = self
            .paths
            .config
            .build_dir
            .as_ref()
            .map(&normalize)
            .transpose()?;

        let bin_dir = self
            .paths
            .config
            .bin_dir
            .as_ref()
            .map(normalize)
            .transpose()?;

        self.paths.config.build_dir = build_dir;
        self.paths.config.bin_dir = bin_dir;

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
                ..
            } => {
                crate::run::qemu::run_qemu(
                    self.clone(),
                    RunQemuArgs {
                        qemu_config: qemu_config.clone(),
                        dtb_dump: *dtb_dump,
                        show_output: true,
                    },
                )
                .await?;
            }
            CargoRunnerKind::Uboot { uboot_config } => {
                crate::run::uboot::run_uboot(
                    self.clone(),
                    RunUbootArgs {
                        config: uboot_config.clone(),
                        show_output: true,
                    },
                )
                .await?;
            }
        }

        Ok(())
    }
}
