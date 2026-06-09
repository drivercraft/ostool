//! Build system configuration and Cargo integration.
//!
//! This module provides functionality for building operating system projects
//! using Cargo or custom build commands. It supports:
//!
//! - Configuring build options via TOML configuration files
//! - Running pre-build and post-build shell commands
//! - Automatic feature detection and configuration
//! - Multiple runner types (QEMU and U-Boot)
//!
//! # Example
//!
//! ```rust,no_run
//! use ostool::build::config::{BuildConfig, BuildSystem, Cargo};
//!
//! // Build configurations are typically loaded from TOML files
//! // See .build.toml for example configuration format
//! ```

use std::path::{Path, PathBuf};

use anyhow::bail;

use crate::{
    artifact::object_tools::ObjectTools,
    artifact::runtime::{
        RuntimeArtifactOptions, prepare_runtime_artifacts as prepare_runtime_artifact_outputs,
    },
    build::{
        cargo_pipeline::{CargoBuildInput, CargoBuildOutcome, CargoBuildPipeline},
        config::{BuildConfig, BuildSystem, Cargo, Custom},
    },
    invocation::{ActiveBuildContext, ActiveCargoBuild, ActiveCustomBuild, Invocation},
    project::{ProjectLayout, metadata, variables::VariableScope},
    run::{
        qemu::{QemuConfig, RunQemuOptions},
        uboot::UbootConfig,
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
}

/// Parameters for running a built Cargo artifact on real hardware via U-Boot.
#[derive(Debug, Clone, Default)]
pub struct CargoUbootRunnerArgs {
    /// Optional fully prepared U-Boot runtime configuration.
    pub uboot: Option<UbootConfig>,
}

/// Specifies the type of runner to use after building.
///
/// This enum determines how the built artifact will be executed,
/// through QEMU emulation or U-Boot on real hardware.
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
            let variable_scope = cargo_variable_scope(layout, &cargo)?;
            Ok(ActiveBuildContext::Cargo(Box::new(ActiveCargoBuild::new(
                config_path,
                variable_scope,
            ))))
        }
        BuildSystem::Custom(_) => {
            let variable_scope =
                VariableScope::for_package(layout, layout.manifest_dir().to_path_buf());
            Ok(ActiveBuildContext::Custom(ActiveCustomBuild::new(
                config_path,
                variable_scope,
            )))
        }
    }
}

pub(crate) fn cargo_variable_scope(
    layout: &ProjectLayout,
    cargo: &Cargo,
) -> anyhow::Result<VariableScope> {
    let package_dir = metadata::package_manifest_dir(layout, &cargo.package)?;
    Ok(VariableScope::for_package(layout, package_dir))
}

/// Returns the default build configuration template.
pub fn default_build_config() -> config::BuildConfig {
    config::BuildConfig::default()
}

/// Loads a build configuration from a workspace-like directory.
///
/// This only parses the config file. Apply caller overrides first, then call
/// [`activate_build_config`] or a build/run helper with the final config.
pub async fn load_build_config_from_dir(
    invocation: &Invocation,
    dir: &Path,
    menu: bool,
) -> anyhow::Result<config::BuildConfig> {
    prepare_build_config(invocation, Some(dir.join(".build.toml")), menu).await
}

/// Loads a build configuration from an explicit file path.
///
/// This only parses the config file. Apply caller overrides first, then call
/// [`activate_build_config`] or a build/run helper with the final config.
pub async fn load_build_config_from_path(
    invocation: &Invocation,
    path: &Path,
    menu: bool,
) -> anyhow::Result<config::BuildConfig> {
    prepare_build_config(invocation, Some(path.to_path_buf()), menu).await
}

/// Records the selected build configuration as active for variable expansion.
///
/// Pass `Some(path)` when `config` was loaded from a build config file so
/// relative Cargo extra config paths resolve against that file. Pass `None`
/// for in-memory build configs.
pub fn activate_build_config(
    invocation: &mut Invocation,
    config: &config::BuildConfig,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    let active = activate_build_context(
        invocation.project_layout(),
        config.clone(),
        config_path.map(Path::to_path_buf),
        &CargoSelector::default(),
    )?;
    invocation.set_active_build(active);
    Ok(())
}

async fn prepare_build_config(
    invocation: &Invocation,
    config_path: Option<PathBuf>,
    menu: bool,
) -> anyhow::Result<BuildConfig> {
    let hooks = config_hooks::build_config_hooks(invocation.workspace_dir());
    config_loader::load_build_config(invocation.workspace_dir(), config_path, menu, &hooks).await
}

/// Builds the project using the specified build configuration.
///
/// `config_path` is the optional source path for `config`.
pub async fn build_with_config(
    invocation: &mut Invocation,
    config: &config::BuildConfig,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    activate_build_config(invocation, config, config_path)?;
    match &config.system {
        config::BuildSystem::Custom(custom) => build_custom(invocation, custom)?,
        config::BuildSystem::Cargo(cargo) => {
            cargo_build(invocation, cargo, config_path).await?;
        }
    }
    Ok(())
}

/// Runs the custom build command from a build configuration.
pub(crate) fn build_custom(invocation: &mut Invocation, config: &Custom) -> anyhow::Result<()> {
    let process_context = invocation.process_context()?;
    crate::process::shell_run_cmd(&process_context, &config.build_cmd)?;
    Ok(())
}

/// Builds the project using Cargo.
///
/// `config_path` is the optional `.build.toml` source path for `config`.
pub async fn cargo_build(
    invocation: &mut Invocation,
    config: &Cargo,
    config_path: Option<&Path>,
) -> anyhow::Result<()> {
    activate_build_config(
        invocation,
        &BuildConfig {
            system: BuildSystem::Cargo(config.clone()),
        },
        config_path,
    )?;
    let debug = invocation.options().debug();
    let input = cargo_build_input(invocation, config, debug)?;
    let outcome = CargoBuildPipeline::build(input, config).execute().await?;
    apply_cargo_build_outcome(invocation, config, &outcome, false, debug)?;
    run_cargo_post_build_cmds(invocation, config)?;
    Ok(())
}

/// Builds or imports the configured artifact and prepares the runtime outputs.
pub(crate) async fn prepare_runtime_artifacts(
    invocation: &mut Invocation,
    config: &config::BuildConfig,
    config_path: Option<&Path>,
    debug: bool,
) -> anyhow::Result<()> {
    activate_build_config(invocation, config, config_path)?;
    match &config.system {
        config::BuildSystem::Custom(custom) => {
            prepare_custom_runtime_artifacts(invocation, custom).await
        }
        config::BuildSystem::Cargo(cargo) => {
            prepare_cargo_runtime_artifacts(invocation, cargo, debug).await
        }
    }
}

async fn prepare_custom_runtime_artifacts(
    invocation: &mut Invocation,
    config: &Custom,
) -> anyhow::Result<()> {
    build_custom(invocation, config)?;
    invocation
        .prepare_elf_artifact(config.elf_path.clone().into(), config.to_bin)
        .await
}

async fn prepare_cargo_runtime_artifacts(
    invocation: &mut Invocation,
    config: &Cargo,
    debug: bool,
) -> anyhow::Result<()> {
    let input = cargo_build_input(invocation, config, debug)?;
    let outcome = CargoBuildPipeline::build(input, config)
        .skip_objcopy(true)
        .resolve_artifact_from_json(true)
        .execute()
        .await?;
    apply_cargo_build_outcome(invocation, config, &outcome, true, debug)?;
    run_cargo_post_build_cmds(invocation, config)?;
    Ok(())
}

/// Builds and runs the project using Cargo with the specified runner.
///
/// `config_path` is the optional `.build.toml` source path for `config`.
pub async fn cargo_run(
    invocation: &mut Invocation,
    config: &Cargo,
    config_path: Option<&Path>,
    runner: &CargoRunnerKind,
) -> anyhow::Result<()> {
    activate_build_config(
        invocation,
        &BuildConfig {
            system: BuildSystem::Cargo(config.clone()),
        },
        config_path,
    )?;

    let debug = matches!(runner, CargoRunnerKind::Qemu(args) if args.debug);
    let input = cargo_build_input(invocation, config, debug)?;
    let outcome = CargoBuildPipeline::build(input, config)
        .skip_objcopy(true)
        .resolve_artifact_from_json(true)
        .execute()
        .await?;
    apply_cargo_build_outcome(invocation, config, &outcome, true, debug)?;
    run_cargo_post_build_cmds(invocation, config)?;

    match runner {
        CargoRunnerKind::Qemu(args) => {
            let qemu = match &args.qemu {
                Some(config) => config.clone(),
                None => crate::run::qemu::ensure_config_for_cargo(invocation, config).await?,
            };
            crate::run::qemu::run_qemu_with_debug(
                invocation,
                &qemu,
                RunQemuOptions {
                    dtb_dump: args.dtb_dump,
                },
                debug,
            )
            .await?;
        }
        CargoRunnerKind::Uboot(args) => {
            let uboot = match &args.uboot {
                Some(config) => config.clone(),
                None => crate::run::uboot::ensure_config_for_cargo(invocation, config).await?,
            };
            crate::run::uboot::run_uboot(invocation, &uboot).await?;
        }
    }

    Ok(())
}

fn cargo_build_input(
    invocation: &Invocation,
    config: &Cargo,
    debug: bool,
) -> anyhow::Result<CargoBuildInput> {
    Ok(CargoBuildInput::new(
        invocation.project_layout().clone(),
        invocation.process_context()?,
        invocation.build_dir(),
        invocation
            .state()
            .build_config_path()
            .map(std::path::Path::to_path_buf),
        debug,
        !config.disable_someboot_build_config,
    ))
}

fn apply_cargo_build_outcome(
    invocation: &mut Invocation,
    config: &Cargo,
    outcome: &CargoBuildOutcome,
    skip_objcopy: bool,
    debug: bool,
) -> anyhow::Result<()> {
    let resolved = outcome.resolved_artifact();
    let process_context = invocation.process_context()?;
    let prepared = prepare_runtime_artifact_outputs(
        &process_context,
        RuntimeArtifactOptions {
            elf_path: resolved.elf_path().to_path_buf(),
            to_bin: config.to_bin && !skip_objcopy,
            bin_dir: invocation.bin_dir(),
            debug,
            cargo_artifact_dir: Some(resolved.cargo_artifact_dir().to_path_buf()),
            strip_elf: false,
            objcopy_program: ObjectTools.objcopy(),
        },
    )?;
    invocation.apply_prepared_runtime_artifacts(prepared);
    Ok(())
}

fn run_cargo_post_build_cmds(invocation: &mut Invocation, config: &Cargo) -> anyhow::Result<()> {
    let process_context = invocation.process_context()?;
    for cmd in &config.post_build_cmds {
        crate::process::shell_run_cmd(&process_context, cmd)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{
        build::{
            artifact_selector::ResolvedCargoArtifact,
            cargo_pipeline::CargoBuildOutcome,
            config::{BuildConfig, BuildSystem, Cargo, CargoBuildProfile, Custom},
        },
        invocation::{Invocation, InvocationOptions},
        project::resolve_project_layout,
    };

    use super::{
        CargoSelector, activate_build_config, activate_build_context, apply_cargo_build_outcome,
        build_with_config,
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
        let mut invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap();
        let outcome = CargoBuildOutcome::new(ResolvedCargoArtifact::new(
            elf_path.clone(),
            cargo_artifact_dir.clone(),
        ));

        apply_cargo_build_outcome(&mut invocation, &config, &outcome, true, false).unwrap();

        let expected_elf = elf_path.canonicalize().unwrap();
        assert_eq!(
            invocation.runtime_artifacts().elf(),
            Some(expected_elf.as_path())
        );
        assert!(invocation.runtime_artifacts().bin().is_none());
        assert_eq!(
            invocation.runtime_artifacts().cargo_artifact_dir(),
            Some(cargo_artifact_dir.as_path())
        );
        assert_eq!(
            invocation.runtime_artifacts().cargo_source_artifact_dir(),
            Some(cargo_artifact_dir.as_path())
        );
        assert_eq!(
            invocation.runtime_artifacts().cargo_source_elf(),
            Some(expected_elf.as_path())
        );
        assert_eq!(
            invocation.runtime_artifacts().runtime_artifact_dir(),
            Some(cargo_artifact_dir.as_path())
        );
        assert!(invocation.runtime_artifacts().debug_artifacts().is_empty());
        assert!(invocation.runtime_arch().is_some());
    }

    #[tokio::test]
    async fn custom_build_only_does_not_prepare_runtime_artifacts() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        let marker = temp.path().join("custom-build-ran");
        let mut invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap();
        let config = BuildConfig {
            system: BuildSystem::Custom(Custom {
                build_cmd: format!("printf built > {}", marker.display()),
                elf_path: "target/kernel.elf".into(),
                to_bin: true,
            }),
        };

        build_with_config(&mut invocation, &config, None)
            .await
            .unwrap();

        assert_eq!(fs::read_to_string(marker).unwrap(), "built");
        assert!(invocation.runtime_artifacts().elf().is_none());
        assert!(invocation.runtime_artifacts().bin().is_none());
        assert!(invocation.runtime_arch().is_none());
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
        assert_eq!(active.config_path(), Some(config_path.as_path()));
        assert_eq!(active.variable_scope().package_dir(), temp.path());
    }

    #[test]
    fn activate_build_config_uses_explicit_path_and_clears_absent_path() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();
        let mut invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap();
        let config = BuildConfig {
            system: BuildSystem::Cargo(Cargo {
                package: "kernel".into(),
                ..Default::default()
            }),
        };
        let config_path = temp.path().join(".build.toml");

        activate_build_config(&mut invocation, &config, Some(&config_path)).unwrap();
        assert_eq!(
            invocation.state().build_config_path(),
            Some(config_path.as_path())
        );

        activate_build_config(&mut invocation, &config, None).unwrap();
        assert!(invocation.state().build_config_path().is_none());
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
