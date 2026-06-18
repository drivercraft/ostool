//! Invocation options, project layout, and runtime state shared by entrypoints.

use std::path::{Path, PathBuf};

use anyhow::anyhow;
use object::Architecture;

use crate::{
    artifact::{
        runtime::{PreparedRuntimeArtifacts, RuntimeArtifactOptions, prepare_runtime_artifacts},
        state::OutputArtifacts,
    },
    process::ProcessContext,
    project::{ProjectLayout, resolve_project_layout, variables::VariableScope},
};

/// Static inputs for one CLI or library invocation.
#[derive(Clone, Debug, Default)]
pub struct InvocationOptions {
    manifest: Option<PathBuf>,
    build_dir: Option<PathBuf>,
    bin_dir: Option<PathBuf>,
    debug: bool,
}

impl InvocationOptions {
    /// Creates immutable invocation options from CLI or library inputs.
    pub fn new(
        manifest: Option<PathBuf>,
        build_dir: Option<PathBuf>,
        bin_dir: Option<PathBuf>,
        debug: bool,
    ) -> Self {
        Self {
            manifest,
            build_dir,
            bin_dir,
            debug,
        }
    }

    /// Returns the optional Cargo manifest path supplied by the caller.
    pub fn manifest(&self) -> Option<&Path> {
        self.manifest.as_deref()
    }

    /// Returns the optional build output directory supplied by the caller.
    pub fn build_dir(&self) -> Option<&Path> {
        self.build_dir.as_deref()
    }

    /// Returns the optional BIN output directory supplied by the caller.
    pub fn bin_dir(&self) -> Option<&Path> {
        self.bin_dir.as_deref()
    }

    /// Returns whether debug-mode runtime artifacts should be preserved.
    pub fn debug(&self) -> bool {
        self.debug
    }
}

/// Top-level immutable inputs plus resolved project layout.
#[derive(Clone, Debug)]
pub struct Invocation {
    options: InvocationOptions,
    project_layout: ProjectLayout,
    state: InvocationState,
}

impl Invocation {
    /// Resolves the project layout for this invocation.
    pub fn new(options: InvocationOptions) -> anyhow::Result<Self> {
        let project_layout = resolve_project_layout(options.manifest().map(PathBuf::from))?;
        Ok(Self {
            options,
            project_layout,
            state: InvocationState::default(),
        })
    }

    /// Returns immutable options for this invocation.
    pub fn options(&self) -> &InvocationOptions {
        &self.options
    }

    /// Returns the canonical Cargo manifest path used by this invocation.
    pub fn manifest_path(&self) -> &Path {
        self.project_layout.manifest_path()
    }

    /// Returns the package directory containing the selected manifest.
    pub fn manifest_dir(&self) -> &Path {
        self.project_layout.manifest_dir()
    }

    /// Returns the Cargo workspace root from metadata.
    pub fn workspace_dir(&self) -> &Path {
        self.project_layout.workspace_dir()
    }

    /// Returns the resolved project layout for this invocation.
    pub(crate) fn project_layout(&self) -> &ProjectLayout {
        &self.project_layout
    }

    pub(crate) fn state(&self) -> &InvocationState {
        &self.state
    }

    pub(crate) fn set_active_build(&mut self, active_build: ActiveBuildContext) {
        self.state.set_active_build(active_build);
    }

    pub(crate) fn runtime_artifacts(&self) -> &OutputArtifacts {
        self.state.artifacts()
    }

    pub(crate) fn runtime_arch(&self) -> Option<Architecture> {
        self.state.arch()
    }

    pub(crate) fn build_dir(&self) -> PathBuf {
        self.options
            .build_dir()
            .map(|dir| self.resolve_dir(dir))
            .unwrap_or_else(|| self.manifest_dir().join("target"))
    }

    pub(crate) fn bin_dir(&self) -> Option<PathBuf> {
        self.options.bin_dir().map(|dir| self.resolve_dir(dir))
    }

    fn resolve_dir(&self, dir: &Path) -> PathBuf {
        if dir.is_relative() {
            self.manifest_dir().join(dir)
        } else {
            dir.to_path_buf()
        }
    }

    pub(crate) fn variable_scope(&self) -> anyhow::Result<VariableScope> {
        let package_dir = self
            .state
            .active_build()
            .map(|active| active.variable_scope().package_dir().to_path_buf())
            .unwrap_or_else(|| self.manifest_dir().to_path_buf());
        Ok(VariableScope::for_package(
            self.project_layout(),
            package_dir,
        ))
    }

    pub(crate) fn process_context(&self) -> anyhow::Result<ProcessContext> {
        Ok(ProcessContext::new(
            self.manifest_dir().to_path_buf(),
            self.workspace_dir().to_path_buf(),
            self.variable_scope()?,
            self.runtime_artifacts().elf().map(PathBuf::from),
        ))
    }

    pub(crate) fn apply_prepared_runtime_artifacts(&mut self, prepared: PreparedRuntimeArtifacts) {
        self.state.apply_prepared_runtime_artifacts(&prepared);
    }

    pub(crate) fn ensure_runtime_bin(&mut self) -> anyhow::Result<PathBuf> {
        if let Some(bin) = self.runtime_artifacts().bin() {
            debug!("BIN file already exists: {bin:?}");
            return Ok(bin.to_path_buf());
        }

        let elf_path = self
            .runtime_artifacts()
            .elf()
            .ok_or_else(|| anyhow!("elf not exist"))?
            .to_path_buf();
        let process_context = self.process_context()?;
        let prepared = prepare_runtime_artifacts(
            &process_context,
            RuntimeArtifactOptions {
                elf_path,
                to_bin: true,
                bin_dir: self.bin_dir(),
                debug: self.options.debug(),
                cargo_artifact_dir: self
                    .runtime_artifacts()
                    .cargo_source_artifact_dir()
                    .map(PathBuf::from),
                strip_elf: false,
            },
        )?;
        let bin_path = prepared
            .bin()
            .ok_or_else(|| anyhow!("bin not exist after conversion"))?
            .to_path_buf();
        self.apply_prepared_runtime_artifacts(prepared);
        Ok(bin_path)
    }

    /// Imports an ELF artifact, strips it to a runtime `.elf`, and optionally
    /// materializes a `.bin` image.
    pub async fn prepare_elf_artifact(
        &mut self,
        path: PathBuf,
        to_bin: bool,
    ) -> anyhow::Result<()> {
        let process_context = self.process_context()?;
        let prepared = prepare_runtime_artifacts(
            &process_context,
            RuntimeArtifactOptions {
                elf_path: path,
                to_bin,
                bin_dir: self.bin_dir(),
                debug: self.options.debug(),
                cargo_artifact_dir: None,
                strip_elf: true,
            },
        )?;
        self.apply_prepared_runtime_artifacts(prepared);
        Ok(())
    }
}

/// Mutable runtime state produced while one invocation is executing.
#[derive(Clone, Debug, Default)]
pub(crate) struct InvocationState {
    arch: Option<Architecture>,
    active_build: Option<ActiveBuildContext>,
    artifacts: OutputArtifacts,
}

impl InvocationState {
    /// Returns the detected runtime artifact architecture.
    pub(crate) fn arch(&self) -> Option<Architecture> {
        self.arch
    }

    /// Returns the activated build context, if a build config has been loaded.
    pub(crate) fn active_build(&self) -> Option<&ActiveBuildContext> {
        self.active_build.as_ref()
    }

    /// Replaces the currently active build context.
    pub(crate) fn set_active_build(&mut self, active_build: ActiveBuildContext) {
        self.active_build = Some(active_build);
    }

    /// Returns the path used to load the active build configuration.
    pub(crate) fn build_config_path(&self) -> Option<&Path> {
        self.active_build
            .as_ref()
            .and_then(ActiveBuildContext::config_path)
    }

    /// Returns prepared runtime artifacts.
    pub(crate) fn artifacts(&self) -> &OutputArtifacts {
        &self.artifacts
    }

    /// Records prepared runtime artifacts and their detected architecture.
    pub(crate) fn apply_prepared_runtime_artifacts(&mut self, prepared: &PreparedRuntimeArtifacts) {
        self.artifacts.apply_prepared_runtime_artifacts(prepared);
        self.arch = prepared.arch();
    }
}

/// Build configuration after CLI overrides and package scope resolution.
#[derive(Clone, Debug)]
pub(crate) enum ActiveBuildContext {
    /// Cargo build configuration selected for this invocation.
    Cargo(Box<ActiveCargoBuild>),
    /// Custom shell build configuration selected for this invocation.
    Custom(ActiveCustomBuild),
}

impl ActiveBuildContext {
    /// Returns the path used to load this build context.
    pub(crate) fn config_path(&self) -> Option<&Path> {
        match self {
            Self::Cargo(active) => active.config_path(),
            Self::Custom(active) => active.config_path(),
        }
    }

    /// Returns the variable scope used for this active build.
    pub(crate) fn variable_scope(&self) -> &VariableScope {
        match self {
            Self::Cargo(active) => active.variable_scope(),
            Self::Custom(active) => active.variable_scope(),
        }
    }
}

/// Activated Cargo build configuration.
#[derive(Clone, Debug)]
pub(crate) struct ActiveCargoBuild {
    config_path: Option<PathBuf>,
    variable_scope: VariableScope,
}

impl ActiveCargoBuild {
    /// Creates an activated Cargo build context.
    pub(crate) fn new(config_path: Option<PathBuf>, variable_scope: VariableScope) -> Self {
        Self {
            config_path,
            variable_scope,
        }
    }

    /// Returns the build config path, if known.
    pub(crate) fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// Returns the variable scope derived from the selected package.
    pub(crate) fn variable_scope(&self) -> &VariableScope {
        &self.variable_scope
    }
}

/// Activated custom build configuration.
#[derive(Clone, Debug)]
pub(crate) struct ActiveCustomBuild {
    config_path: Option<PathBuf>,
    variable_scope: VariableScope,
}

impl ActiveCustomBuild {
    /// Creates an activated custom build context.
    pub(crate) fn new(config_path: Option<PathBuf>, variable_scope: VariableScope) -> Self {
        Self {
            config_path,
            variable_scope,
        }
    }

    /// Returns the build config path, if known.
    pub(crate) fn config_path(&self) -> Option<&Path> {
        self.config_path.as_deref()
    }

    /// Returns the variable scope used by this custom build.
    pub(crate) fn variable_scope(&self) -> &VariableScope {
        &self.variable_scope
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use crate::{
        invocation::{ActiveBuildContext, ActiveCargoBuild, Invocation, InvocationOptions},
        project::variables::VariableScope,
    };

    fn write_package(root: &std::path::Path) {
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    }

    #[test]
    fn invocation_starts_with_empty_runtime_state() {
        let temp = tempfile::tempdir().unwrap();
        write_package(temp.path());

        let invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap();

        assert!(invocation.state.active_build().is_none());
        assert!(invocation.state.build_config_path().is_none());
        assert!(invocation.state.artifacts().elf().is_none());
        assert!(invocation.state.arch().is_none());
    }

    #[test]
    fn invocation_state_records_active_cargo_build() {
        let temp = tempfile::tempdir().unwrap();
        write_package(temp.path());
        let mut invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap();
        let config_path = temp.path().join(".build.toml");
        let package_dir = invocation.manifest_dir().to_path_buf();
        let scope = VariableScope::for_package(invocation.project_layout(), package_dir.clone());

        invocation
            .state
            .set_active_build(ActiveBuildContext::Cargo(Box::new(ActiveCargoBuild::new(
                Some(config_path.clone()),
                scope,
            ))));

        assert_eq!(
            invocation.state.build_config_path(),
            Some(config_path.as_path())
        );
        let Some(ActiveBuildContext::Cargo(active)) = invocation.state.active_build() else {
            panic!("active Cargo build missing");
        };
        assert_eq!(active.variable_scope().package_dir(), package_dir.as_path());
    }
}
