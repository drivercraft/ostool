//! Legacy tool facade for workspace configuration, build, and run workflows.

use std::path::PathBuf;

use anyhow::anyhow;
use cargo_metadata::Metadata;
use jkconfig::data::ElementHook;
use object::Architecture;

use crate::{
    artifact::{
        runtime::{PreparedRuntimeArtifacts, RuntimeArtifactOptions, prepare_runtime_artifacts},
        state::OutputArtifacts,
    },
    build::{
        config::{BuildConfig, BuildSystem, Cargo},
        config_hooks, config_loader,
    },
    ctx::AppContext,
    invocation::Invocation,
    process::ProcessContext,
    project::{ProjectLayout, metadata, resolve_project_layout, variables::VariableScope},
};

/// Static configuration used to initialize a [`Tool`].
#[derive(Default, Clone, Debug)]
pub struct ToolConfig {
    /// Optional manifest path or manifest directory.
    pub manifest: Option<PathBuf>,
    /// Optional custom build output directory.
    pub build_dir: Option<PathBuf>,
    /// Optional custom binary output directory.
    pub bin_dir: Option<PathBuf>,
    /// Whether debug mode is enabled.
    pub debug: bool,
    /// Disable automatic Cargo argument injection from someboot build metadata.
    pub disable_someboot_build_config: bool,
}

/// Main library object orchestrating build and run operations.
#[derive(Clone, Debug)]
pub struct Tool {
    pub(crate) config: ToolConfig,
    pub(crate) manifest_path: PathBuf,
    pub(crate) manifest_dir: PathBuf,
    pub(crate) workspace_dir: PathBuf,
    pub(crate) ctx: AppContext,
}

/// Resolved Cargo manifest and workspace paths derived from `cargo metadata`.
#[derive(Clone, Debug)]
pub struct ManifestContext {
    pub manifest_path: PathBuf,
    pub manifest_dir: PathBuf,
    pub workspace_dir: PathBuf,
}

impl ManifestContext {
    pub fn from_invocation(invocation: &Invocation) -> Self {
        Self {
            manifest_path: invocation.manifest_path().to_path_buf(),
            manifest_dir: invocation.manifest_dir().to_path_buf(),
            workspace_dir: invocation.workspace_dir().to_path_buf(),
        }
    }

    pub(crate) fn from_project_layout(layout: &ProjectLayout) -> Self {
        Self {
            manifest_path: layout.manifest_path().to_path_buf(),
            manifest_dir: layout.manifest_dir().to_path_buf(),
            workspace_dir: layout.workspace_dir().to_path_buf(),
        }
    }
}

impl Tool {
    /// Creates a new tool from the provided configuration.
    pub fn new(config: ToolConfig) -> anyhow::Result<Self> {
        let layout = resolve_project_layout(config.manifest.clone())?;
        Ok(Self::from_project_layout(config, layout))
    }

    /// Creates the legacy tool facade from an already-resolved invocation.
    ///
    /// Invocation options are mapped into `ToolConfig` while the resolved project
    /// layout is reused directly, so manifest/workspace discovery is not repeated.
    /// Someboot build-config injection uses the same default as `Tool::new` and can
    /// be changed afterward with `set_someboot_build_config_enabled`.
    pub fn from_invocation(invocation: Invocation) -> Self {
        let (options, layout) = invocation.into_parts();
        let config = ToolConfig {
            manifest: Some(layout.manifest_path().to_path_buf()),
            build_dir: options.build_dir().map(PathBuf::from),
            bin_dir: options.bin_dir().map(PathBuf::from),
            debug: options.debug(),
            ..Default::default()
        };
        Self::from_project_layout(config, layout)
    }

    pub(crate) fn from_project_layout(config: ToolConfig, layout: ProjectLayout) -> Self {
        Self {
            config,
            manifest_path: layout.manifest_path().to_path_buf(),
            manifest_dir: layout.manifest_dir().to_path_buf(),
            workspace_dir: layout.workspace_dir().to_path_buf(),
            ctx: AppContext::default(),
        }
    }

    pub fn ctx(&self) -> &AppContext {
        &self.ctx
    }

    pub fn ctx_mut(&mut self) -> &mut AppContext {
        &mut self.ctx
    }

    /// Returns the currently prepared runtime artifacts.
    pub(crate) fn runtime_artifacts(&self) -> &OutputArtifacts {
        &self.ctx.artifacts
    }

    /// Returns the architecture detected from the current runtime artifact.
    pub(crate) fn runtime_arch(&self) -> Option<Architecture> {
        self.ctx.arch
    }

    pub fn set_build_config_path(&mut self, path: Option<PathBuf>) {
        self.ctx.build_config_path = path;
    }

    /// Enables or disables automatic Cargo argument injection from someboot build metadata.
    pub fn set_someboot_build_config_enabled(&mut self, enabled: bool) {
        self.config.disable_someboot_build_config = !enabled;
    }

    pub(crate) fn someboot_build_config_enabled(&self, cargo: &Cargo) -> bool {
        !self.config.disable_someboot_build_config && !cargo.disable_someboot_build_config
    }

    pub fn into_context(self) -> AppContext {
        self.ctx
    }

    pub(crate) fn debug_enabled(&self) -> bool {
        self.config.debug
    }

    pub(crate) fn sync_cargo_context(&mut self, cargo: &Cargo) {
        self.ctx.build_config = Some(BuildConfig {
            system: BuildSystem::Cargo(cargo.clone()),
        });
    }

    pub(crate) fn manifest_dir(&self) -> &PathBuf {
        &self.manifest_dir
    }

    pub(crate) fn workspace_dir(&self) -> &PathBuf {
        &self.workspace_dir
    }

    pub(crate) fn build_dir(&self) -> PathBuf {
        self.config
            .build_dir
            .as_ref()
            .map(|dir| self.resolve_dir(dir))
            .unwrap_or_else(|| self.manifest_dir.join("target"))
    }

    pub(crate) fn bin_dir(&self) -> Option<PathBuf> {
        self.config
            .bin_dir
            .as_ref()
            .map(|dir| self.resolve_dir(dir))
    }

    fn resolve_dir(&self, dir: &PathBuf) -> PathBuf {
        if dir.is_relative() {
            self.manifest_dir.join(dir)
        } else {
            dir.clone()
        }
    }

    /// Gets the Cargo metadata for the current manifest.
    pub fn metadata(&self) -> anyhow::Result<Metadata> {
        metadata::cargo_metadata(&self.project_layout())
    }

    pub(crate) fn resolve_package_manifest_dir(&self, package: &str) -> anyhow::Result<PathBuf> {
        metadata::package_manifest_dir(&self.project_layout(), package)
    }

    /// Imports an ELF artifact, strips it to a runtime `.elf`, and optionally
    /// materializes a `.bin` image.
    pub async fn prepare_elf_artifact(
        &mut self,
        path: PathBuf,
        to_bin: bool,
    ) -> anyhow::Result<()> {
        self.prepare_runtime_artifacts_from_elf(path, to_bin).await
    }

    /// Imports an ELF artifact through the shared runtime artifact preparation path.
    ///
    /// This crate-internal implementation backs the public compatibility wrapper
    /// and build orchestration paths without exposing a second public API.
    pub(crate) async fn prepare_runtime_artifacts_from_elf(
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
                debug: self.debug_enabled(),
                cargo_artifact_dir: None,
                strip_elf: true,
                objcopy_program: PathBuf::from("rust-objcopy"),
            },
        )?;
        self.apply_prepared_runtime_artifacts(prepared);
        Ok(())
    }

    /// Ensures a raw BIN artifact exists and returns its path.
    pub(crate) fn ensure_runtime_bin(&mut self) -> anyhow::Result<PathBuf> {
        self.objcopy_output_bin()
    }

    /// Converts the ELF file to raw binary format.
    fn objcopy_output_bin(&mut self) -> anyhow::Result<PathBuf> {
        if let Some(bin) = self.ctx.artifacts.bin() {
            debug!("BIN file already exists: {:?}", bin);
            return Ok(bin.to_path_buf());
        }

        let elf_path = self
            .ctx
            .artifacts
            .elf()
            .ok_or_else(|| anyhow!("elf not exist"))?;
        let process_context = self.process_context()?;
        let prepared = prepare_runtime_artifacts(
            &process_context,
            RuntimeArtifactOptions {
                elf_path: elf_path.to_path_buf(),
                to_bin: true,
                bin_dir: self.bin_dir(),
                debug: self.debug_enabled(),
                cargo_artifact_dir: self.ctx.artifacts.cargo_artifact_dir().map(PathBuf::from),
                strip_elf: false,
                objcopy_program: PathBuf::from("rust-objcopy"),
            },
        )?;
        let bin_path = prepared
            .bin()
            .ok_or_else(|| anyhow!("bin not exist after objcopy"))?
            .to_path_buf();
        self.apply_prepared_runtime_artifacts(prepared);
        Ok(bin_path)
    }

    /// Applies prepared runtime artifacts to legacy context state.
    pub(crate) fn apply_prepared_runtime_artifacts(&mut self, prepared: PreparedRuntimeArtifacts) {
        self.ctx
            .artifacts
            .apply_prepared_runtime_artifacts(&prepared);
        self.ctx.arch = prepared.arch();
    }

    /// Loads and prepares the build configuration.
    pub(crate) async fn prepare_build_config(
        &mut self,
        config_path: Option<PathBuf>,
        menu: bool,
    ) -> anyhow::Result<BuildConfig> {
        let hooks = self.ui_hooks();
        let loaded = config_loader::load_build_config(
            &self.workspace_dir,
            config_path,
            menu,
            &hooks,
            !self.config.disable_someboot_build_config,
        )
        .await?;

        self.ctx.build_config_path = Some(loaded.path().to_path_buf());
        let config = loaded.into_config();
        self.ctx.build_config = Some(config.clone());
        Ok(config)
    }

    fn package_root_for_variables(&self) -> anyhow::Result<PathBuf> {
        if let Some(BuildConfig {
            system: BuildSystem::Cargo(cargo),
        }) = &self.ctx.build_config
        {
            return self.resolve_package_manifest_dir(&cargo.package);
        }

        Ok(self.manifest_dir.clone())
    }

    fn project_layout(&self) -> ProjectLayout {
        ProjectLayout::from_manifest_parts(
            self.manifest_path.clone(),
            self.manifest_dir.clone(),
            self.workspace_dir.clone(),
        )
    }

    pub(crate) fn variable_scope(&self) -> anyhow::Result<VariableScope> {
        let package_dir = self.package_root_for_variables()?;
        Ok(VariableScope::for_package(
            &self.project_layout(),
            package_dir,
        ))
    }

    pub(crate) fn process_context(&self) -> anyhow::Result<ProcessContext> {
        Ok(ProcessContext::new(
            self.manifest_dir.clone(),
            self.workspace_dir.clone(),
            self.variable_scope()?,
            self.ctx.artifacts.elf().map(PathBuf::from),
        ))
    }

    pub(crate) fn ui_hooks(&self) -> Vec<ElementHook> {
        config_hooks::build_config_hooks(&self.workspace_dir)
    }
}

pub fn resolve_manifest_context(input: Option<PathBuf>) -> anyhow::Result<ManifestContext> {
    resolve_project_layout(input).map(|layout| ManifestContext::from_project_layout(&layout))
}

#[cfg(test)]
mod tests {
    use super::{Tool, ToolConfig, resolve_manifest_context};
    use crate::artifact::runtime::{RuntimeArtifactOptions, prepare_runtime_artifacts};
    use crate::build::config::{BuildConfig, BuildSystem, Cargo};
    use crate::run::qemu::resolve_qemu_config_path_in_dir;
    use crate::{process, project::variables};
    use object::Architecture;
    use std::{
        collections::HashMap,
        fs,
        path::{Path, PathBuf},
    };

    #[tokio::test]
    async fn apply_prepared_runtime_artifacts_updates_dirs_and_arch() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/lib.rs"), "").unwrap();

        let source = std::env::current_exe().unwrap();
        let copied = temp.path().join("sample-elf");
        std::fs::copy(&source, &copied).unwrap();

        let mut tool = Tool::new(ToolConfig {
            manifest: Some(temp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();
        let process_context = tool.process_context().unwrap();
        let prepared = prepare_runtime_artifacts(
            &process_context,
            RuntimeArtifactOptions {
                elf_path: copied.clone(),
                to_bin: false,
                bin_dir: None,
                debug: false,
                cargo_artifact_dir: None,
                strip_elf: false,
                objcopy_program: PathBuf::from("rust-objcopy"),
            },
        )
        .unwrap();
        tool.apply_prepared_runtime_artifacts(prepared);

        let expected_elf = copied.canonicalize().unwrap();
        let expected_dir = expected_elf.parent().unwrap().to_path_buf();

        assert_eq!(tool.ctx.artifacts.elf(), Some(expected_elf.as_path()));
        assert_eq!(
            tool.ctx.artifacts.cargo_artifact_dir(),
            Some(expected_dir.as_path())
        );
        assert_eq!(
            tool.ctx.artifacts.runtime_artifact_dir(),
            Some(expected_dir.as_path())
        );
        assert!(tool.ctx.arch.is_some());
        assert!(tool.ctx.artifacts.bin().is_none());
    }

    #[test]
    fn resolve_manifest_context_uses_workspace_root() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\"]\nresolver = \"3\"\n",
        )
        .unwrap();

        let app_dir = temp.path().join("app");
        std::fs::create_dir_all(app_dir.join("src")).unwrap();
        std::fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let manifest = resolve_manifest_context(Some(app_dir.clone())).unwrap();

        assert_eq!(manifest.manifest_path, app_dir.join("Cargo.toml"));
        assert_eq!(manifest.manifest_dir, app_dir);
        assert_eq!(manifest.workspace_dir, temp.path());
    }

    #[test]
    fn resolve_package_manifest_dir_uses_selected_package() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"kernel\"]\nresolver = \"3\"\n",
        )
        .unwrap();

        let app_dir = temp.path().join("app");
        std::fs::create_dir_all(app_dir.join("src")).unwrap();
        std::fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let kernel_dir = temp.path().join("kernel");
        std::fs::create_dir_all(kernel_dir.join("src")).unwrap();
        std::fs::write(
            kernel_dir.join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(kernel_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let tool = Tool::new(ToolConfig {
            manifest: Some(app_dir.clone()),
            ..Default::default()
        })
        .unwrap();

        let resolved = tool.resolve_package_manifest_dir("kernel").unwrap();
        assert_eq!(resolved, kernel_dir);
    }

    #[test]
    fn cargo_qemu_config_resolution_prefers_package_dir_over_workspace_root() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"kernel\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        std::fs::write(temp.path().join("qemu-aarch64.toml"), "").unwrap();

        let app_dir = temp.path().join("app");
        std::fs::create_dir_all(app_dir.join("src")).unwrap();
        std::fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let kernel_dir = temp.path().join("kernel");
        std::fs::create_dir_all(kernel_dir.join("src")).unwrap();
        std::fs::write(
            kernel_dir.join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(kernel_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        std::fs::write(kernel_dir.join(".qemu-aarch64.toml"), "").unwrap();

        let tool = Tool::new(ToolConfig {
            manifest: Some(app_dir),
            ..Default::default()
        })
        .unwrap();

        let package_dir = tool.resolve_package_manifest_dir("kernel").unwrap();
        let resolved =
            resolve_qemu_config_path_in_dir(&package_dir, Some(Architecture::Aarch64), None)
                .unwrap();

        assert_eq!(resolved, kernel_dir.join(".qemu-aarch64.toml"));
    }

    #[tokio::test]
    async fn prepare_build_config_skips_someboot_args_when_cargo_config_disables_them() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"someboot\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::write(
            temp.path().join(".build.toml"),
            r#"
[system.Cargo]
package = "app"
target = "x86_64-unknown-none"
disable_someboot_build_config = true
env = {}
features = []
args = []
pre_build_cmds = []
post_build_cmds = []
to_bin = false
"#,
        )
        .unwrap();
        let app_dir = temp.path().join("app");
        fs::create_dir_all(app_dir.join("src")).unwrap();
        fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nsomeboot = { path = \"../someboot\" }\n",
        )
        .unwrap();
        fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();
        let someboot_dir = temp.path().join("someboot");
        fs::create_dir_all(someboot_dir.join("src")).unwrap();
        fs::write(
            someboot_dir.join("Cargo.toml"),
            "[package]\nname = \"someboot\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(someboot_dir.join("src/lib.rs"), "pub fn marker() {}\n").unwrap();
        fs::write(
            someboot_dir.join("build-info.toml"),
            "[x86_64-unknown-none]\ncargoargs = [\"--someboot-cargoarg\"]\nrustflags = [\"-Cdebuginfo=2\"]\n",
        )
        .unwrap();

        let mut tool = Tool::new(ToolConfig {
            manifest: Some(temp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();
        let config = tool
            .load_build_config_from_path(&temp.path().join(".build.toml"), false)
            .await
            .unwrap();

        let BuildSystem::Cargo(cargo) = config.system else {
            panic!("expected Cargo build config");
        };
        assert!(!cargo.args.iter().any(|arg| arg == "--someboot-cargoarg"));
        assert!(
            !cargo
                .args
                .iter()
                .any(|arg| arg.contains("target.x86_64-unknown-none.rustflags"))
        );
    }

    #[test]
    fn expand_variables_uses_workspace_and_legacy_workspacefolder() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/lib.rs"), "").unwrap();

        let tool = Tool::new(ToolConfig {
            manifest: Some(temp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();

        let scope = tool.variable_scope().unwrap();
        let replaced =
            variables::expand_variables("${workspace}:${workspaceFolder}", &scope).unwrap();
        let expected = temp.path().display().to_string();
        assert_eq!(replaced, format!("{expected}:{expected}"));
    }

    #[test]
    fn expand_variables_uses_cross_platform_tmpdir() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/lib.rs"), "").unwrap();

        let tool = Tool::new(ToolConfig {
            manifest: Some(temp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();

        let scope = tool.variable_scope().unwrap();
        let replaced = variables::expand_variables("${tmpDir}", &scope).unwrap();
        assert_eq!(replaced, std::env::temp_dir().display().to_string());
    }

    /// Verifies that missing environment placeholders expand to an empty string.
    #[test]
    fn expand_variables_uses_empty_string_for_missing_env() {
        let temp = tempfile::tempdir().unwrap();
        write_single_package(temp.path(), "sample");

        let tool = Tool::new(ToolConfig {
            manifest: Some(temp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();

        let missing = format!(
            "__OSTOOL_TEST_ENV_SHOULD_NOT_EXIST_{}__",
            std::process::id()
        );

        let scope = tool.variable_scope().unwrap();
        let replaced =
            variables::expand_variables(&format!("before-${{env:{missing}}}-after"), &scope)
                .unwrap();
        assert_eq!(replaced, "before--after");
    }

    #[test]
    fn expand_variables_uses_package_dir_from_build_config() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"kernel\"]\nresolver = \"3\"\n",
        )
        .unwrap();

        let app_dir = temp.path().join("app");
        std::fs::create_dir_all(app_dir.join("src")).unwrap();
        std::fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let kernel_dir = temp.path().join("kernel");
        std::fs::create_dir_all(kernel_dir.join("src")).unwrap();
        std::fs::write(
            kernel_dir.join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(kernel_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let mut tool = Tool::new(ToolConfig {
            manifest: Some(app_dir),
            ..Default::default()
        })
        .unwrap();
        tool.ctx.build_config = Some(BuildConfig {
            system: BuildSystem::Cargo(Cargo {
                env: HashMap::new(),
                target: "aarch64-unknown-none".into(),
                package: "kernel".into(),
                bin: None,
                features: vec![],
                log: None,
                extra_config: None,
                profile: None,
                disable_someboot_build_config: false,
                args: vec![],
                pre_build_cmds: vec![],
                post_build_cmds: vec![],
                to_bin: false,
            }),
        });

        let scope = tool.variable_scope().unwrap();
        let replaced = variables::expand_variables("${package}", &scope).unwrap();
        assert_eq!(replaced, kernel_dir.display().to_string());
    }

    #[test]
    fn variable_scope_uses_manifest_dir_for_package_without_build_config() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/lib.rs"), "").unwrap();

        let tool = Tool::new(ToolConfig {
            manifest: Some(temp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();

        let scope = tool.variable_scope().unwrap();
        let replaced = variables::expand_variables("${package}", &scope).unwrap();
        assert_eq!(replaced, temp.path().display().to_string());
    }

    #[test]
    fn variable_scope_errors_when_selected_package_is_missing() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\"]\nresolver = \"3\"\n",
        )
        .unwrap();

        let app_dir = temp.path().join("app");
        std::fs::create_dir_all(app_dir.join("src")).unwrap();
        std::fs::write(
            app_dir.join("Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::write(app_dir.join("src/main.rs"), "fn main() {}\n").unwrap();

        let mut tool = Tool::new(ToolConfig {
            manifest: Some(app_dir),
            ..Default::default()
        })
        .unwrap();
        tool.ctx.build_config = Some(BuildConfig {
            system: BuildSystem::Cargo(Cargo {
                package: "missing".into(),
                target: "aarch64-unknown-none".into(),
                ..Default::default()
            }),
        });

        let err = tool.variable_scope().unwrap_err().to_string();
        assert!(err.contains("package 'missing' not found"));
    }

    #[test]
    fn command_replaces_args_and_env() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"sample\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        std::fs::create_dir_all(temp.path().join("src")).unwrap();
        std::fs::write(temp.path().join("src/lib.rs"), "").unwrap();

        let tool = Tool::new(ToolConfig {
            manifest: Some(temp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();

        let process_context = tool.process_context().unwrap();
        let mut cmd = process::command("echo", &process_context);
        cmd.arg("${workspace}");
        cmd.env("PKG_DIR", "${package}");

        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec![temp.path().display().to_string()]);

        let envs: Vec<(String, String)> = cmd
            .get_envs()
            .filter_map(|(k, v)| {
                Some((
                    k.to_string_lossy().into_owned(),
                    v?.to_string_lossy().into_owned(),
                ))
            })
            .collect();
        assert!(
            envs.iter()
                .any(|(k, v)| k == "PKG_DIR" && v == &temp.path().display().to_string())
        );
        assert!(
            envs.iter()
                .any(|(k, v)| k == "WORKSPACE_FOLDER" && v == &temp.path().display().to_string())
        );
    }

    /// Verifies shell hooks receive the runtime kernel ELF path.
    #[cfg(unix)]
    #[tokio::test]
    async fn shell_run_cmd_injects_kernel_elf_when_runtime_elf_exists() {
        let temp = tempfile::tempdir().unwrap();
        write_single_package(temp.path(), "sample");

        let source = std::env::current_exe().unwrap();
        let copied = temp.path().join("sample-elf");
        std::fs::copy(&source, &copied).unwrap();

        let mut tool = Tool::new(ToolConfig {
            manifest: Some(temp.path().to_path_buf()),
            ..Default::default()
        })
        .unwrap();
        let process_context = tool.process_context().unwrap();
        let prepared = prepare_runtime_artifacts(
            &process_context,
            RuntimeArtifactOptions {
                elf_path: copied.clone(),
                to_bin: false,
                bin_dir: None,
                debug: false,
                cargo_artifact_dir: None,
                strip_elf: false,
                objcopy_program: PathBuf::from("rust-objcopy"),
            },
        )
        .unwrap();
        tool.apply_prepared_runtime_artifacts(prepared);

        let output = temp.path().join("kernel-env.txt");
        let process_context = tool.process_context().unwrap();
        process::shell_run_cmd(
            &process_context,
            &format!("printf '%s' \"$KERNEL_ELF\" > {}", output.display()),
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(output).unwrap(),
            copied.canonicalize().unwrap().display().to_string()
        );
    }

    /// Writes a minimal single-package Cargo project for tool tests.
    fn write_single_package(root: &Path, package: &str) {
        fs::write(
            root.join("Cargo.toml"),
            format!("[package]\nname = \"{package}\"\nversion = \"0.1.0\"\nedition = \"2024\"\n"),
        )
        .unwrap();
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(root.join("src/lib.rs"), "").unwrap();
    }
}
