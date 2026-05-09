//! Build system configuration and Cargo integration.
//!
//! This module provides functionality for building operating system projects
//! using Cargo or custom build commands. It supports:
//!
//! - Configuring build options via TOML configuration files
//! - Running pre-build and post-build shell commands
//! - Automatic feature detection and configuration
//! - Multiple runner types (QEMU, U-Boot, UEFI HTTP Boot)
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

use anyhow::bail;

use crate::{
    Tool,
    artifact::runtime::{RuntimeArtifactOptions, prepare_runtime_artifacts},
    build::{
        cargo_pipeline::{CargoBuildInput, CargoBuildOutcome, CargoBuildPipeline},
        config::{BuildConfig, BuildSystem, Cargo, Custom},
    },
    invocation::{ActiveBuildContext, ActiveCargoBuild, ActiveCustomBuild},
    project::{ProjectLayout, metadata, variables::VariableScope},
    run::{
        httpboot::{HttpBootConfig, RunHttpBootOptions},
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

/// Parameters for running a built Cargo artifact via UEFI HTTP Boot.
#[derive(Debug, Clone, Default)]
pub struct CargoHttpbootRunnerArgs {
    /// Optional fully prepared HTTP Boot runtime configuration.
    pub httpboot: Option<HttpBootConfig>,
    /// Whether to show HTTP Boot output.
    pub show_output: bool,
}

/// Specifies the type of runner to use after building.
///
/// This enum determines how the built artifact will be executed,
/// through QEMU emulation, U-Boot, or UEFI HTTP Boot on real hardware.
pub enum CargoRunnerKind {
    /// Run the built artifact in QEMU emulator.
    Qemu(Box<CargoQemuRunnerArgs>),
    /// Run the built artifact on real hardware via U-Boot.
    Uboot(Box<CargoUbootRunnerArgs>),
    /// Publish and run the built artifact via UEFI HTTP Boot.
    Httpboot(Box<CargoHttpbootRunnerArgs>),
}

impl CargoRunnerKind {
    pub fn new_qemu(args: CargoQemuRunnerArgs) -> Self {
        Self::Qemu(Box::new(args))
    }

    pub fn new_uboot(args: CargoUbootRunnerArgs) -> Self {
        Self::Uboot(Box::new(args))
    }

    pub fn new_httpboot(args: CargoHttpbootRunnerArgs) -> Self {
        Self::Httpboot(Box::new(args))
    }
}

/// CLI overrides for Cargo package and binary target selection.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct CargoSelector {
    package: Option<String>,
    bin: Option<String>,
}

impl CargoSelector {
    /// Creates a Cargo selector from optional CLI overrides.
    #[cfg(test)]
    pub(crate) fn new(package: Option<String>, bin: Option<String>) -> Self {
        Self { package, bin }
    }

    /// Returns whether no selector override was supplied.
    fn is_empty(&self) -> bool {
        self.package.is_none() && self.bin.is_none()
    }
}

/// Applies Cargo selector overrides to a build configuration.
fn apply_cargo_selector(
    config: &mut config::BuildConfig,
    selector: &CargoSelector,
) -> anyhow::Result<()> {
    if selector.is_empty() {
        return Ok(());
    }

    let config::BuildSystem::Cargo(cargo_config) = &mut config.system else {
        bail!("--package/--bin can only be used with system.Cargo build configs");
    };

    if let Some(package) = &selector.package {
        cargo_config.package = package.clone();
    }
    if let Some(bin) = &selector.bin {
        cargo_config.bin = Some(bin.clone());
    }
    Ok(())
}

pub(crate) fn activate_build_context(
    layout: &ProjectLayout,
    mut config: BuildConfig,
    config_path: Option<PathBuf>,
    selector: &CargoSelector,
) -> anyhow::Result<ActiveBuildContext> {
    apply_cargo_selector(&mut config, selector)?;
    match config.system {
        BuildSystem::Cargo(cargo) => {
            let package_dir = metadata::package_manifest_dir(layout, &cargo.package)?;
            let variable_scope = VariableScope::for_package(layout, package_dir.clone());
            Ok(ActiveBuildContext::Cargo(Box::new(ActiveCargoBuild::new(
                cargo,
                config_path,
                variable_scope,
            ))))
        }
        BuildSystem::Custom(custom) => {
            let variable_scope =
                VariableScope::for_package(layout, layout.manifest_dir().to_path_buf());
            Ok(ActiveBuildContext::Custom(ActiveCustomBuild::new(
                custom,
                config_path,
                variable_scope,
            )))
        }
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
        self.sync_build_context(config)?;
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
        self.sync_cargo_context(config)?;
        let debug = self.debug_enabled();
        let input = self.cargo_build_input(config, debug)?;
        let outcome = CargoBuildPipeline::build(input, config).execute().await?;
        self.apply_cargo_build_outcome(config, &outcome, false, debug)?;
        self.run_cargo_post_build_cmds(config)?;
        Ok(())
    }

    /// Builds or imports the configured artifact and prepares the runtime outputs.
    pub(crate) async fn prepare_runtime_artifacts(
        &mut self,
        config: &config::BuildConfig,
        debug: bool,
    ) -> anyhow::Result<()> {
        self.sync_build_context(config)?;
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
        self.config.debug = debug;
        let input = self.cargo_build_input(config, debug)?;
        let outcome = CargoBuildPipeline::build(input, config)
            .skip_objcopy(true)
            .resolve_artifact_from_json(true)
            .execute()
            .await?;
        self.apply_cargo_build_outcome(config, &outcome, true, debug)?;
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
        self.sync_cargo_context(config)?;

        let debug = matches!(runner, CargoRunnerKind::Qemu(args) if args.debug);
        self.config.debug = debug;

        let input = self.cargo_build_input(config, debug)?;
        let outcome = CargoBuildPipeline::build(input, config)
            .skip_objcopy(true)
            .resolve_artifact_from_json(true)
            .execute()
            .await?;
        self.apply_cargo_build_outcome(config, &outcome, true, debug)?;
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
            CargoRunnerKind::Httpboot(args) => {
                let httpboot = match &args.httpboot {
                    Some(config) => config.clone(),
                    None => self.ensure_httpboot_config_for_cargo(config).await?,
                };
                self.run_httpboot(
                    &httpboot,
                    RunHttpBootOptions {
                        show_output: args.show_output,
                    },
                )
                .await?;
            }
        }

        Ok(())
    }

    fn cargo_build_input(&self, config: &Cargo, debug: bool) -> anyhow::Result<CargoBuildInput> {
        Ok(CargoBuildInput::new(
            self.project_layout(),
            self.process_context()?,
            self.build_dir(),
            self.ctx.build_config_path.clone(),
            debug,
            self.someboot_build_config_enabled(config),
        ))
    }

    fn apply_cargo_build_outcome(
        &mut self,
        config: &Cargo,
        outcome: &CargoBuildOutcome,
        skip_objcopy: bool,
        debug: bool,
    ) -> anyhow::Result<()> {
        let resolved = outcome.resolved_artifact();
        let process_context = self.process_context()?;
        let prepared = prepare_runtime_artifacts(
            &process_context,
            RuntimeArtifactOptions {
                elf_path: resolved.elf_path().to_path_buf(),
                to_bin: config.to_bin && !skip_objcopy,
                bin_dir: self.bin_dir(),
                debug,
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
            config::{BuildConfig, BuildSystem, Cargo, CargoBuildProfile, Custom},
        },
        project::resolve_project_layout,
    };

    use super::{CargoSelector, activate_build_context};

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

        tool.apply_cargo_build_outcome(&config, &outcome, true, false)
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

    #[test]
    fn activate_build_context_applies_cargo_selector_and_scope() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("src/bin")).unwrap();
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        fs::write(temp.path().join("src/bin/kernel-qemu.rs"), "fn main() {}\n").unwrap();
        let layout = resolve_project_layout(Some(temp.path().to_path_buf())).unwrap();
        let config_path = temp.path().join(".build.toml");
        let config = BuildConfig {
            system: BuildSystem::Cargo(Cargo {
                package: "placeholder".into(),
                ..Default::default()
            }),
        };

        let active = activate_build_context(
            &layout,
            config,
            Some(config_path.clone()),
            &CargoSelector::new(Some("kernel".into()), Some("kernel-qemu".into())),
        )
        .unwrap();

        let crate::invocation::ActiveBuildContext::Cargo(active) = active else {
            panic!("expected active Cargo build");
        };
        assert_eq!(active.config().package, "kernel");
        assert_eq!(active.config().bin.as_deref(), Some("kernel-qemu"));
        assert_eq!(active.config_path(), Some(config_path.as_path()));
        assert_eq!(active.variable_scope().package_dir(), temp.path());
    }

    #[test]
    fn activate_build_context_rejects_selector_for_custom_build() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        let layout = resolve_project_layout(Some(temp.path().to_path_buf())).unwrap();
        let config = BuildConfig {
            system: BuildSystem::Custom(Custom {
                build_cmd: "make".into(),
                elf_path: "target/kernel.elf".into(),
                to_bin: true,
            }),
        };

        let err = activate_build_context(
            &layout,
            config,
            None,
            &CargoSelector::new(Some("kernel".into()), None),
        )
        .unwrap_err();

        assert!(
            err.to_string()
                .contains("--package/--bin can only be used with system.Cargo")
        );
    }
}
