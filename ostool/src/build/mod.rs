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

use std::path::{Path, PathBuf};

use crate::{
    Tool,
    artifact::runtime::{RuntimeArtifactOptions, prepare_runtime_artifacts},
    build::{
        cargo_pipeline::{CargoBuildOutcome, CargoBuildPipeline},
        config::{Cargo, Custom},
    },
    run::{
        qemu::{QemuConfig, RunQemuOptions},
        uboot::{RunUbootOptions, UbootConfig},
    },
};

mod artifact_selector;
pub(crate) mod config_hooks;
pub(crate) mod config_loader;

/// Cargo pipeline implementation for building projects.
mod cargo_pipeline;

/// Build configuration types and structures.
pub mod config;

pub mod someboot;

/// Parameters for running a built Cargo artifact in QEMU.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CargoQemuRunnerArgs {
    /// Optional fully prepared QEMU runtime configuration.
    pub qemu: Option<QemuConfig>,
    /// Whether to enable debug mode (GDB server).
    pub debug: bool,
    /// Whether to dump the device tree blob.
    pub dtb_dump: bool,
    /// Whether to show QEMU output.
    pub show_output: bool,
}

/// Parameters for running a built Cargo artifact on real hardware via U-Boot.
#[derive(Debug, Clone, Default)]
pub struct CargoUbootRunnerArgs {
    /// Optional fully prepared U-Boot runtime configuration.
    pub uboot: Option<UbootConfig>,
    /// Whether to show U-Boot output.
    pub show_output: bool,
}

/// Specifies the type of runner to use after building.
///
/// This enum determines how the built artifact will be executed,
/// either through QEMU emulation or via U-Boot on real hardware.
pub enum CargoRunnerKind {
    /// Run the built artifact in QEMU emulator.
    Qemu(Box<CargoQemuRunnerArgs>),
    /// Run the built artifact on real hardware via U-Boot.
    Uboot(Box<CargoUbootRunnerArgs>),
}

impl CargoRunnerKind {
    pub fn new_qemu(args: CargoQemuRunnerArgs) -> Self {
        Self::Qemu(Box::new(args))
    }

    pub fn new_uboot(args: CargoUbootRunnerArgs) -> Self {
        Self::Uboot(Box::new(args))
    }
}

impl Tool {
    /// Returns the default build configuration template.
    pub fn default_build_config(&self) -> config::BuildConfig {
        config::BuildConfig::default()
    }

    /// Loads a build configuration from a workspace-like directory.
    pub async fn load_build_config_from_dir(
        &mut self,
        dir: &Path,
        menu: bool,
    ) -> anyhow::Result<config::BuildConfig> {
        self.prepare_build_config(Some(dir.join(".build.toml")), menu)
            .await
    }

    /// Loads a build configuration from an explicit file path.
    pub async fn load_build_config_from_path(
        &mut self,
        path: &Path,
        menu: bool,
    ) -> anyhow::Result<config::BuildConfig> {
        self.prepare_build_config(Some(path.to_path_buf()), menu)
            .await
    }

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

    /// Runs the custom build command from a build configuration.
    ///
    /// Custom builds use the same artifact preparation path as Cargo builds so
    /// runners consume a single ELF/BIN artifact state model.
    pub(crate) fn build_custom(&mut self, config: &Custom) -> anyhow::Result<()> {
        let process_context = self.process_context()?;
        crate::process::shell_run_cmd(&process_context, &config.build_cmd)?;
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
        self.sync_cargo_context(config);
        let outcome = cargo_pipeline::CargoBuildPipeline::build_auto(self, config)
            .execute()
            .await?;
        self.apply_cargo_build_outcome(config, &outcome, false)?;
        self.run_cargo_post_build_cmds(config)?;
        Ok(())
    }

    /// Builds or imports the configured artifact and prepares the runtime outputs.
    pub(crate) async fn prepare_runtime_artifacts(
        &mut self,
        config: &config::BuildConfig,
        debug: bool,
    ) -> anyhow::Result<()> {
        match &config.system {
            config::BuildSystem::Custom(custom) => {
                self.prepare_custom_runtime_artifacts(custom).await
            }
            config::BuildSystem::Cargo(cargo) => {
                self.prepare_cargo_runtime_artifacts(cargo, debug).await
            }
        }
    }

    async fn prepare_custom_runtime_artifacts(&mut self, config: &Custom) -> anyhow::Result<()> {
        self.build_custom(config)?;
        self.prepare_runtime_artifacts_from_elf(config.elf_path.clone().into(), config.to_bin)
            .await
    }

    async fn prepare_cargo_runtime_artifacts(
        &mut self,
        config: &Cargo,
        debug: bool,
    ) -> anyhow::Result<()> {
        let build_config_path = self.ctx.build_config_path.clone();
        let outcome = CargoBuildPipeline::build(self, config, build_config_path)
            .debug(debug)
            .skip_objcopy(true)
            .resolve_artifact_from_json(true)
            .execute()
            .await?;
        self.apply_cargo_build_outcome(config, &outcome, true)?;
        self.run_cargo_post_build_cmds(config)?;
        Ok(())
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
        self.sync_cargo_context(config);
        let build_config_path = self.ctx.build_config_path.clone();

        let debug = matches!(runner, CargoRunnerKind::Qemu(args) if args.debug);

        let outcome = CargoBuildPipeline::build(self, config, build_config_path)
            .debug(debug)
            .skip_objcopy(true)
            .resolve_artifact_from_json(true)
            .execute()
            .await?;
        self.apply_cargo_build_outcome(config, &outcome, true)?;
        self.run_cargo_post_build_cmds(config)?;

        match runner {
            CargoRunnerKind::Qemu(args) => {
                let qemu = match &args.qemu {
                    Some(config) => config.clone(),
                    None => self.ensure_qemu_config_for_cargo(config).await?,
                };
                self.run_qemu(
                    &qemu,
                    RunQemuOptions {
                        dtb_dump: args.dtb_dump,
                        show_output: args.show_output,
                    },
                )
                .await?;
            }
            CargoRunnerKind::Uboot(args) => {
                let uboot = match &args.uboot {
                    Some(config) => config.clone(),
                    None => self.ensure_uboot_config_for_cargo(config).await?,
                };
                self.run_uboot(
                    &uboot,
                    RunUbootOptions {
                        show_output: args.show_output,
                    },
                )
                .await?;
            }
        }

        Ok(())
    }

    fn apply_cargo_build_outcome(
        &mut self,
        config: &Cargo,
        outcome: &CargoBuildOutcome,
        skip_objcopy: bool,
    ) -> anyhow::Result<()> {
        let resolved = outcome.resolved_artifact();
        let process_context = self.process_context()?;
        let prepared = prepare_runtime_artifacts(
            &process_context,
            RuntimeArtifactOptions {
                elf_path: resolved.elf_path().to_path_buf(),
                to_bin: config.to_bin && !skip_objcopy,
                bin_dir: self.bin_dir(),
                debug: self.debug_enabled(),
                cargo_artifact_dir: Some(resolved.cargo_artifact_dir().to_path_buf()),
                strip_elf: false,
                objcopy_program: PathBuf::from("rust-objcopy"),
            },
        )?;
        self.apply_prepared_runtime_artifacts(prepared);
        Ok(())
    }

    fn run_cargo_post_build_cmds(&mut self, config: &Cargo) -> anyhow::Result<()> {
        let process_context = self.process_context()?;
        for cmd in &config.post_build_cmds {
            crate::process::shell_run_cmd(&process_context, cmd)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{
        Tool, ToolConfig,
        build::{
            artifact_selector::ResolvedCargoArtifact,
            cargo_pipeline::CargoBuildOutcome,
            config::{Cargo, CargoBuildProfile},
        },
    };

    #[test]
    fn apply_cargo_build_outcome_records_runtime_artifact_state() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let cargo_artifact_dir = temp.path().join("target/aarch64/debug");
        fs::create_dir_all(&cargo_artifact_dir).unwrap();
        let elf_path = cargo_artifact_dir.join("kernel");
        fs::copy(std::env::current_exe().unwrap(), &elf_path).unwrap();

        let config = Cargo {
            target: "aarch64-unknown-none".into(),
            package: "kernel".into(),
            profile: Some(CargoBuildProfile::Debug),
            to_bin: true,
            ..Default::default()
        };
        let mut tool = Tool::new(ToolConfig {
            manifest: Some(temp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();
        let outcome = CargoBuildOutcome::new(ResolvedCargoArtifact::new(
            elf_path.clone(),
            cargo_artifact_dir.clone(),
        ));

        tool.apply_cargo_build_outcome(&config, &outcome, true)
            .unwrap();

        let expected_elf = elf_path.canonicalize().unwrap();
        assert_eq!(tool.ctx.artifacts.elf(), Some(expected_elf.as_path()));
        assert!(tool.ctx.artifacts.bin().is_none());
        assert_eq!(
            tool.ctx.artifacts.cargo_artifact_dir(),
            Some(cargo_artifact_dir.as_path())
        );
        assert_eq!(
            tool.ctx.artifacts.runtime_artifact_dir(),
            Some(cargo_artifact_dir.as_path())
        );
        assert!(tool.ctx.arch.is_some());
    }
}
