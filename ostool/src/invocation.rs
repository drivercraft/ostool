//! Invocation options, project layout, and runtime state shared by entrypoints.

use std::path::{Path, PathBuf};

use object::Architecture;

use crate::{
    artifact::{runtime::PreparedRuntimeArtifacts, state::OutputArtifacts},
    build::config::{BuildConfig, BuildSystem, Cargo, Custom},
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
    pub fn project_layout(&self) -> &ProjectLayout {
        &self.project_layout
    }

    pub(crate) fn into_parts(self) -> (InvocationOptions, ProjectLayout, InvocationState) {
        (self.options, self.project_layout, self.state)
    }
}

/// Mutable runtime state produced while one invocation is executing.
#[derive(Clone, Debug, Default)]
pub(crate) struct InvocationState {
    arch: Option<Architecture>,
    active_build: Option<ActiveBuildContext>,
    build_config_path: Option<PathBuf>,
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
        self.build_config_path = active_build.config_path().map(PathBuf::from);
        self.active_build = Some(active_build);
    }

    /// Returns the path used to load the active build configuration.
    pub(crate) fn build_config_path(&self) -> Option<&Path> {
        self.build_config_path.as_deref()
    }

    /// Records the path used to load the active build configuration.
    pub(crate) fn set_build_config_path(&mut self, path: Option<PathBuf>) {
        self.build_config_path = path;
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

    /// Returns the activated build configuration.
    pub(crate) fn build_config(&self) -> BuildConfig {
        match self {
            Self::Cargo(active) => BuildConfig {
                system: BuildSystem::Cargo(active.config().clone()),
            },
            Self::Custom(active) => BuildConfig {
                system: BuildSystem::Custom(active.config().clone()),
            },
        }
    }
}

/// Activated Cargo build configuration.
#[derive(Clone, Debug)]
pub(crate) struct ActiveCargoBuild {
    config: Cargo,
    config_path: Option<PathBuf>,
    variable_scope: VariableScope,
}

impl ActiveCargoBuild {
    /// Creates an activated Cargo build context.
    pub(crate) fn new(
        config: Cargo,
        config_path: Option<PathBuf>,
        variable_scope: VariableScope,
    ) -> Self {
        Self {
            config,
            config_path,
            variable_scope,
        }
    }

    /// Returns the final Cargo config after CLI selector overrides.
    pub(crate) fn config(&self) -> &Cargo {
        &self.config
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
    config: Custom,
    config_path: Option<PathBuf>,
    variable_scope: VariableScope,
}

impl ActiveCustomBuild {
    /// Creates an activated custom build context.
    pub(crate) fn new(
        config: Custom,
        config_path: Option<PathBuf>,
        variable_scope: VariableScope,
    ) -> Self {
        Self {
            config,
            config_path,
            variable_scope,
        }
    }

    /// Returns the final custom build config.
    pub(crate) fn config(&self) -> &Custom {
        &self.config
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
        build::config::{Cargo, CargoBuildProfile},
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
        let config = Cargo {
            target: "x86_64-unknown-none".into(),
            package: "kernel".into(),
            profile: Some(CargoBuildProfile::Debug),
            ..Default::default()
        };
        let package_dir = invocation.manifest_dir().to_path_buf();
        let scope = VariableScope::for_package(invocation.project_layout(), package_dir.clone());

        invocation
            .state
            .set_active_build(ActiveBuildContext::Cargo(Box::new(ActiveCargoBuild::new(
                config.clone(),
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
        assert_eq!(active.config(), &config);
        assert_eq!(active.variable_scope().package_dir(), package_dir.as_path());
    }
}
