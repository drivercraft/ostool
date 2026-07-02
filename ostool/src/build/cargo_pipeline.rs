//! Cargo build command builder and executor.
//!
//! This module provides the [`CargoBuildPipeline`] type for constructing and executing
//! Cargo build commands with customizable options, environment variables, and
//! pre/post build hooks.

use std::{
    collections::HashMap,
    io::BufReader,
    path::{Path, PathBuf},
    process::Stdio,
};

use anyhow::{Context, anyhow, bail};
use cargo_metadata::{Message, PackageId, TargetKind};
use colored::Colorize;

use crate::{
    build::{
        artifact_selector::{
            CargoExecutableArtifact, ResolvedCargoArtifact, select_executable_artifact,
        },
        config::{Cargo, CargoBuildProfile},
        someboot,
    },
    process::ProcessContext,
    project::{ProjectLayout, metadata},
    utils::{Command, PathResultExt},
};

#[derive(Debug, Clone)]
pub(super) struct CargoBuildInput {
    project_layout: ProjectLayout,
    process_context: ProcessContext,
    build_dir: PathBuf,
    config_path: Option<PathBuf>,
    debug: bool,
    enable_someboot_build_config: bool,
}

impl CargoBuildInput {
    pub(super) fn new(
        project_layout: ProjectLayout,
        process_context: ProcessContext,
        build_dir: PathBuf,
        config_path: Option<PathBuf>,
        debug: bool,
        enable_someboot_build_config: bool,
    ) -> Self {
        Self {
            project_layout,
            process_context,
            build_dir,
            config_path,
            debug,
            enable_someboot_build_config,
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct CargoBuildOutcome {
    resolved_artifact: ResolvedCargoArtifact,
}

impl CargoBuildOutcome {
    pub(crate) fn new(resolved_artifact: ResolvedCargoArtifact) -> Self {
        Self { resolved_artifact }
    }

    pub(crate) fn resolved_artifact(&self) -> &ResolvedCargoArtifact {
        &self.resolved_artifact
    }
}

fn is_executable_target_kind(kinds: &[TargetKind]) -> bool {
    kinds
        .iter()
        .any(|kind| matches!(kind, TargetKind::Bin | TargetKind::Test))
}

#[derive(Debug, Clone)]
struct CargoBuildPlan {
    command: String,
    envs: Vec<(String, String)>,
    extra_envs: Vec<(String, String)>,
    extra_config_path: Option<PathBuf>,
    package: String,
    bin: Option<String>,
    test: Option<String>,
    target: String,
    target_dir: PathBuf,
    features: Vec<String>,
    config_args: Vec<String>,
    someboot_args: Vec<String>,
    release: bool,
    message_format: &'static str,
    extra_args: Vec<String>,
}

impl CargoBuildPlan {
    fn render(&self, cargo_program: &Path, context: &ProcessContext) -> Command {
        let mut cmd = crate::process::command(cargo_program.as_os_str(), context);
        cmd.arg(&self.command);

        for (key, value) in &self.envs {
            println!("{}", format!("{key}={value}").cyan());
            cmd.env(key, value);
        }
        for (key, value) in &self.extra_envs {
            println!("{}", format!("{key}={value}").cyan());
            cmd.env(key, value);
        }

        if let Some(extra_config_path) = &self.extra_config_path {
            cmd.arg("--config");
            cmd.arg(extra_config_path.display().to_string());
        }

        cmd.arg("-p");
        cmd.arg(&self.package);
        if let Some(bin) = &self.bin {
            cmd.arg("--bin");
            cmd.arg(bin);
        }
        if let Some(test) = &self.test {
            cmd.arg("--test");
            cmd.arg(test);
        }
        cmd.arg("--target");
        cmd.arg(&self.target);
        cmd.arg("-Z");
        cmd.arg("unstable-options");
        cmd.arg("--target-dir");
        cmd.arg(self.target_dir.display().to_string());

        if !self.features.is_empty() {
            cmd.arg("--features");
            cmd.arg(self.features.join(","));
        }

        for arg in &self.config_args {
            cmd.arg(arg);
        }
        for arg in &self.someboot_args {
            cmd.arg(arg);
        }

        if self.release {
            cmd.arg("--release");
        }

        cmd.arg("--message-format");
        cmd.arg(self.message_format);

        for arg in &self.extra_args {
            cmd.arg(arg);
        }

        cmd
    }

    #[cfg(test)]
    fn args(&self) -> Vec<String> {
        let mut args = Vec::new();
        args.push(self.command.clone());
        if let Some(extra_config_path) = &self.extra_config_path {
            args.push("--config".into());
            args.push(extra_config_path.display().to_string());
        }
        args.push("-p".into());
        args.push(self.package.clone());
        if let Some(bin) = &self.bin {
            args.push("--bin".into());
            args.push(bin.clone());
        }
        if let Some(test) = &self.test {
            args.push("--test".into());
            args.push(test.clone());
        }
        args.push("--target".into());
        args.push(self.target.clone());
        args.push("-Z".into());
        args.push("unstable-options".into());
        args.push("--target-dir".into());
        args.push(self.target_dir.display().to_string());
        if !self.features.is_empty() {
            args.push("--features".into());
            args.push(self.features.join(","));
        }
        args.extend(self.config_args.clone());
        args.extend(self.someboot_args.clone());
        if self.release {
            args.push("--release".into());
        }
        args.push("--message-format".into());
        args.push(self.message_format.into());
        args.extend(self.extra_args.clone());
        args
    }
}

/// A builder for constructing and executing Cargo commands.
///
/// `CargoBuildPipeline` provides a fluent API for configuring Cargo build or run
/// commands with custom arguments, environment variables, and build hooks.
///
/// This builder is an internal implementation detail used by build orchestration.
pub struct CargoBuildPipeline<'a> {
    input: CargoBuildInput,
    config: &'a Cargo,
    cargo_program: PathBuf,
    command: String,
    extra_args: Vec<String>,
    extra_envs: HashMap<String, String>,
    skip_objcopy: bool,
    resolve_artifact_from_json: bool,
}

impl<'a> CargoBuildPipeline<'a> {
    /// Creates a new `CargoBuildPipeline` for executing `cargo build`.
    ///
    /// # Arguments
    ///
    /// * `input` - Invocation-scoped Cargo build inputs.
    /// * `config` - The Cargo build configuration.
    pub(super) fn build(input: CargoBuildInput, config: &'a Cargo) -> Self {
        Self {
            input,
            config,
            cargo_program: PathBuf::from("cargo"),
            command: "build".to_string(),
            extra_args: Vec::new(),
            extra_envs: HashMap::new(),
            skip_objcopy: false,
            resolve_artifact_from_json: true,
        }
    }

    /// Sets whether to skip the objcopy step after building.
    pub fn skip_objcopy(mut self, skip: bool) -> Self {
        self.skip_objcopy = skip;
        self
    }

    /// Enables artifact path resolution from Cargo JSON messages.
    pub fn resolve_artifact_from_json(mut self, enable: bool) -> Self {
        self.resolve_artifact_from_json = enable;
        self
    }

    #[cfg(test)]
    fn cargo_program(mut self, program: impl Into<PathBuf>) -> Self {
        self.cargo_program = program.into();
        self
    }

    /// Executes the configured Cargo command.
    ///
    /// This runs pre-build commands, executes Cargo, handles output artifacts,
    /// and runs post-build commands.
    ///
    /// # Errors
    ///
    /// Returns an error if any step of the build process fails.
    pub async fn execute(mut self) -> anyhow::Result<CargoBuildOutcome> {
        // 1. Pre-build commands
        self.run_pre_build_cmds()?;

        // 2. Build and run cargo
        let resolved = self.run_cargo().await?;
        Ok(CargoBuildOutcome::new(resolved))
    }

    fn run_pre_build_cmds(&mut self) -> anyhow::Result<()> {
        for cmd in &self.config.pre_build_cmds {
            crate::process::shell_run_cmd(&self.input.process_context, cmd)?;
        }
        Ok(())
    }

    async fn run_cargo(&mut self) -> anyhow::Result<ResolvedCargoArtifact> {
        self.run_cargo_and_resolve_artifact().await
    }

    async fn run_cargo_and_resolve_artifact(&mut self) -> anyhow::Result<ResolvedCargoArtifact> {
        let (target_pkg_id, default_run) = self.target_package_info()?;
        let mut cmd = self.build_cargo_command().await?;

        cmd.stdout(Stdio::piped());
        cmd.stderr(Stdio::inherit());
        cmd.print_cmd();

        let mut child = cmd
            .spawn()
            .context("failed to spawn cargo build command for artifact resolution")?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow!("failed to capture cargo stdout for message parsing"))?;
        let reader = BufReader::new(stdout);

        let mut executable_artifacts: Vec<CargoExecutableArtifact> = Vec::new();
        for message in Message::parse_stream(reader) {
            let message = message.context("failed to parse cargo JSON message stream")?;
            match message {
                Message::CompilerArtifact(artifact) => {
                    if artifact.package_id == target_pkg_id
                        && is_executable_target_kind(&artifact.target.kind)
                        && let Some(executable) = artifact.executable
                    {
                        let elf_path = executable.into_std_path_buf();
                        let cargo_artifact_dir = elf_path
                            .parent()
                            .ok_or_else(|| {
                                anyhow!(
                                    "cargo reported executable without parent directory: {}",
                                    elf_path.display()
                                )
                            })?
                            .to_path_buf();
                        executable_artifacts.push(CargoExecutableArtifact::new(
                            artifact.target.name,
                            ResolvedCargoArtifact::new(elf_path, cargo_artifact_dir),
                        ));
                    }
                }
                Message::CompilerMessage(msg) => {
                    if let Some(rendered) = msg.message.rendered {
                        eprint!("{rendered}");
                    }
                }
                Message::TextLine(line) => {
                    println!("{line}");
                }
                _ => {}
            }
        }

        let status = child
            .wait()
            .context("failed waiting for cargo build process")?;
        if !status.success() {
            bail!("failed with status: {status}");
        }

        let resolved = select_executable_artifact(
            &executable_artifacts,
            self.config.bin.as_deref(),
            self.config.test.as_deref(),
            default_run.as_deref(),
            &self.config.package,
        )?;

        Ok(resolved)
    }

    async fn build_cargo_command(&mut self) -> anyhow::Result<Command> {
        let plan = self.build_cargo_plan().await?;
        Ok(plan.render(&self.cargo_program, &self.input.process_context))
    }

    async fn build_cargo_plan(&mut self) -> anyhow::Result<CargoBuildPlan> {
        if self.config.bin.is_some() && self.config.test.is_some() {
            bail!("system.Cargo.bin and system.Cargo.test are mutually exclusive");
        }
        let features = self.build_features();
        let extra_config_path = self.cargo_extra_config().await?;
        let someboot_args = self.detect_someboot_args(&features)?;
        let mut extra_envs = self
            .extra_envs
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        extra_envs.sort_by(|left, right| left.0.cmp(&right.0));
        let mut envs = self
            .config
            .env
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect::<Vec<_>>();
        envs.sort_by(|left, right| left.0.cmp(&right.0));

        Ok(CargoBuildPlan {
            command: self.command.clone(),
            envs,
            extra_envs,
            extra_config_path,
            package: self.config.package.clone(),
            bin: self.config.bin.clone(),
            test: self.config.test.clone(),
            target: self.config.target.clone(),
            target_dir: self.input.build_dir.clone(),
            features,
            config_args: self.config.args.clone(),
            someboot_args,
            release: self.effective_profile() == CargoBuildProfile::Release,
            message_format: "json-render-diagnostics",
            extra_args: self.extra_args.clone(),
        })
    }

    fn detect_someboot_args(&self, features: &[String]) -> anyhow::Result<Vec<String>> {
        let workspace_manifest = self.input.project_layout.workspace_dir().join("Cargo.toml");
        if self.input.enable_someboot_build_config && workspace_manifest.exists() {
            someboot::detect_build_config_for_package(
                &workspace_manifest,
                &self.config.package,
                features,
                &self.config.target,
            )
            .with_context(|| {
                format!(
                    "failed to detect someboot build config from {}",
                    workspace_manifest.display()
                )
            })
        } else {
            Ok(Vec::new())
        }
    }

    fn target_package_info(&self) -> anyhow::Result<(PackageId, Option<String>)> {
        let metadata = metadata::cargo_metadata(&self.input.project_layout)?;
        let Some(package) = metadata
            .packages
            .iter()
            .find(|pkg| pkg.name == self.config.package)
        else {
            bail!(
                "package '{}' not found in cargo metadata under {}",
                self.config.package,
                self.input.project_layout.manifest_dir().display()
            );
        };
        Ok((package.id.clone(), package.default_run.clone()))
    }

    fn build_features(&self) -> Vec<String> {
        let mut features = self.config.features.clone();
        if let Some(log_level) = self.log_level_feature() {
            features.push(log_level);
        }
        features
    }

    fn effective_profile(&self) -> CargoBuildProfile {
        let default_profile = if self.input.debug {
            CargoBuildProfile::Debug
        } else {
            CargoBuildProfile::Release
        };
        self.config.profile.unwrap_or(default_profile)
    }

    fn log_level_feature(&self) -> Option<String> {
        let level = self.config.log.clone()?;

        let meta = metadata::cargo_metadata(&self.input.project_layout).ok()?;
        let pkg = meta
            .packages
            .iter()
            .find(|p| p.name == self.config.package)?;

        let has_log = pkg.dependencies.iter().any(|dep| dep.name == "log");

        if has_log {
            Some(format!(
                "log/{}max_level_{}",
                if self.effective_profile() == CargoBuildProfile::Debug {
                    ""
                } else {
                    "release_"
                },
                format!("{level:?}").to_lowercase()
            ))
        } else {
            None
        }
    }

    /// Resolves an optional extra Cargo config from a local path or URL.
    async fn cargo_extra_config(&self) -> anyhow::Result<Option<PathBuf>> {
        let s = match self.config.extra_config.as_ref() {
            Some(s) => s,
            None => return Ok(None),
        };

        // Check if it's a URL (starts with http:// or https://)
        if s.starts_with("http://") || s.starts_with("https://") {
            // Convert GitHub URL to raw content URL if needed
            let download_url = Self::convert_to_raw_url(s);

            // Download to temp directory
            match self.download_config_to_temp(&download_url).await {
                Ok(path) => Ok(Some(path)),
                Err(e) => {
                    eprintln!("Failed to download config from {s}: {e}");
                    Err(e)
                }
            }
        } else {
            // It's a local path
            let extra = Path::new(s);

            if extra.is_relative() {
                if let Some(ref config_path) = self.input.config_path {
                    let combined = config_path
                        .parent()
                        .ok_or_else(|| {
                            anyhow::anyhow!(
                                "invalid config path without parent: {}",
                                config_path.display()
                            )
                        })?
                        .join(extra);
                    Ok(Some(combined))
                } else {
                    Ok(Some(extra.to_path_buf()))
                }
            } else {
                Ok(Some(extra.to_path_buf()))
            }
        }
    }

    /// Convert GitHub URL to raw content URL
    /// Supports:
    /// - https://github.com/user/repo/blob/branch/path/file -> https://raw.githubusercontent.com/user/repo/branch/path/file
    /// - https://raw.githubusercontent.com/... (already raw, no change)
    /// - Other URLs: no change
    fn convert_to_raw_url(url: &str) -> String {
        // Already a raw URL
        if url.contains("raw.githubusercontent.com") || url.contains("raw.github.com") {
            return url.to_string();
        }

        // Convert github.com/user/repo/blob/... to raw.githubusercontent.com/user/repo/...
        if url.contains("github.com") && url.contains("/blob/") {
            let converted = url
                .replace("github.com", "raw.githubusercontent.com")
                .replace("/blob/", "/");
            println!("Converting GitHub URL to raw: {url} -> {converted}");
            return converted;
        }

        // Not a GitHub URL or already in correct format
        url.to_string()
    }

    async fn download_config_to_temp(&self, url: &str) -> anyhow::Result<PathBuf> {
        use std::time::SystemTime;

        println!("Downloading cargo config from: {url}");

        // Get system temp directory
        let temp_dir = std::env::temp_dir();

        // Generate filename with timestamp
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Extract filename from URL or use default
        let url_path = url.split('/').next_back().unwrap_or("config.toml");
        let filename = format!("cargo_config_{timestamp}_{url_path}");
        let target_path = temp_dir.join(filename);

        // Create reqwest client
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {e}"))?;

        // Build request with User-Agent for GitHub
        let mut request = client.get(url);

        if url.contains("github.com") || url.contains("githubusercontent.com") {
            // GitHub requires User-Agent
            request = request.header("User-Agent", "ostool-cargo-downloader");
        }

        // Download the file
        let response = request
            .send()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to download from {url}: {e}"))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("HTTP error {}: {}", response.status(), url));
        }

        let content = response
            .bytes()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read response body: {e}"))?;

        // Write to temp file
        tokio::fs::write(&target_path, content)
            .await
            .with_path("failed to write downloaded cargo config", &target_path)
            .with_context(|| format!("while downloading cargo config from {url}"))?;

        println!("Config downloaded to: {}", target_path.display());

        Ok(target_path)
    }
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::CargoBuildPipeline;
    use crate::{
        build::{
            cargo_pipeline::CargoBuildInput,
            config::{Cargo, CargoBuildProfile, LogLevel},
        },
        invocation::{Invocation, InvocationOptions},
        project::metadata,
    };

    fn write_someboot_workspace(root: &Path) {
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"someboot\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("app/src")).unwrap();
        fs::write(
            root.join("app/Cargo.toml"),
            "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nsomeboot = { path = \"../someboot\" }\n",
        )
        .unwrap();
        fs::write(root.join("app/src/main.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(root.join("someboot/src")).unwrap();
        fs::write(
            root.join("someboot/Cargo.toml"),
            "[package]\nname = \"someboot\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::write(root.join("someboot/src/lib.rs"), "pub fn marker() {}\n").unwrap();
        fs::write(
            root.join("someboot/build-info.toml"),
            "[x86_64-unknown-none]\ncargoargs = [\"--someboot-cargoarg\"]\nrustflags = [\"-Cdebuginfo=2\"]\n",
        )
        .unwrap();
    }

    fn write_log_workspace(root: &Path, with_log_dependency: bool) {
        fs::write(
            root.join("Cargo.toml"),
            "[workspace]\nmembers = [\"app\", \"log\"]\nresolver = \"3\"\n",
        )
        .unwrap();
        fs::create_dir_all(root.join("app/src")).unwrap();
        let dependency = if with_log_dependency {
            "\n[dependencies]\nlog = { path = \"../log\" }\n"
        } else {
            ""
        };
        fs::write(
            root.join("app/Cargo.toml"),
            format!(
                "[package]\nname = \"app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n{dependency}"
            ),
        )
        .unwrap();
        fs::write(root.join("app/src/main.rs"), "fn main() {}\n").unwrap();
        fs::create_dir_all(root.join("log/src")).unwrap();
        fs::write(
            root.join("log/Cargo.toml"),
            "[package]\nname = \"log\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[features]\nmax_level_info = []\nrelease_max_level_info = []\n",
        )
        .unwrap();
        fs::write(root.join("log/src/lib.rs"), "pub fn marker() {}\n").unwrap();
    }

    fn cargo_input_for(invocation: &Invocation, config: &Cargo, debug: bool) -> CargoBuildInput {
        CargoBuildInput::new(
            invocation.project_layout().clone(),
            invocation.process_context().unwrap(),
            invocation.build_dir(),
            invocation
                .state()
                .build_config_path()
                .map(std::path::Path::to_path_buf),
            debug,
            !config.disable_someboot_build_config,
        )
    }

    async fn cargo_plan_args_result(
        root: &Path,
        config: &Cargo,
        debug: bool,
    ) -> anyhow::Result<Vec<String>> {
        let invocation = Invocation::new(InvocationOptions::new(
            Some(root.to_path_buf()),
            None,
            None,
            debug,
        ))
        .unwrap();
        let input = cargo_input_for(&invocation, config, debug);
        let mut builder = CargoBuildPipeline::build(input, config).skip_objcopy(true);
        builder.build_cargo_plan().await.map(|plan| plan.args())
    }

    async fn cargo_plan_args(root: &Path, config: &Cargo, debug: bool) -> Vec<String> {
        cargo_plan_args_result(root, config, debug).await.unwrap()
    }

    fn feature_arg(args: &[String]) -> Option<&str> {
        args.windows(2)
            .find(|window| window[0] == "--features")
            .map(|window| window[1].as_str())
    }

    #[tokio::test]
    async fn build_cargo_command_skips_someboot_args_when_cargo_config_disables_them() {
        let temp = tempfile::tempdir().unwrap();
        write_someboot_workspace(temp.path());

        let config = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            disable_someboot_build_config: true,
            profile: Some(CargoBuildProfile::Debug),
            ..Default::default()
        };

        let invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap();
        let input = cargo_input_for(&invocation, &config, false);
        let mut builder = CargoBuildPipeline::build(input, &config).skip_objcopy(true);
        let cmd = builder.build_cargo_command().await.unwrap();
        let args: Vec<String> = cmd
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();

        assert!(!args.iter().any(|arg| arg == "--someboot-cargoarg"));
        assert!(
            !args
                .iter()
                .any(|arg| arg.contains("target.x86_64-unknown-none.rustflags"))
        );
    }

    #[tokio::test]
    async fn build_cargo_plan_injects_someboot_args_once() {
        let temp = tempfile::tempdir().unwrap();
        write_someboot_workspace(temp.path());

        let config = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            profile: Some(CargoBuildProfile::Debug),
            ..Default::default()
        };

        let args = cargo_plan_args(temp.path(), &config, false).await;

        assert_eq!(
            args.iter()
                .filter(|arg| arg.as_str() == "--someboot-cargoarg")
                .count(),
            1
        );
        assert_eq!(
            args.iter()
                .filter(|arg| arg.contains("target.x86_64-unknown-none.rustflags"))
                .count(),
            1
        );
    }

    #[tokio::test]
    async fn build_cargo_plan_uses_debug_flag_as_default_profile() {
        let temp = tempfile::tempdir().unwrap();
        write_log_workspace(temp.path(), false);
        let config = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            ..Default::default()
        };

        let debug_args = cargo_plan_args(temp.path(), &config, true).await;
        let release_args = cargo_plan_args(temp.path(), &config, false).await;

        assert!(!debug_args.iter().any(|arg| arg == "--release"));
        assert!(release_args.iter().any(|arg| arg == "--release"));
    }

    #[tokio::test]
    async fn build_cargo_plan_uses_test_target_selector() {
        let temp = tempfile::tempdir().unwrap();
        write_log_workspace(temp.path(), false);
        let config = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            test: Some("kernel_axtest".into()),
            profile: Some(CargoBuildProfile::Debug),
            ..Default::default()
        };

        let args = cargo_plan_args(temp.path(), &config, false).await;

        assert!(
            args.windows(2)
                .any(|window| window == ["--test", "kernel_axtest"])
        );
        assert!(!args.iter().any(|arg| arg == "--bin"));
    }

    #[tokio::test]
    async fn build_cargo_plan_rejects_bin_and_test_target_together() {
        let temp = tempfile::tempdir().unwrap();
        write_log_workspace(temp.path(), false);
        let config = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            bin: Some("kernel".into()),
            test: Some("kernel_axtest".into()),
            ..Default::default()
        };

        let err = cargo_plan_args_result(temp.path(), &config, false)
            .await
            .unwrap_err();

        assert!(
            err.to_string()
                .contains("system.Cargo.bin and system.Cargo.test")
        );
    }

    #[tokio::test]
    async fn build_cargo_plan_profile_overrides_debug_flag() {
        let temp = tempfile::tempdir().unwrap();
        write_log_workspace(temp.path(), false);

        let debug_profile = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            profile: Some(CargoBuildProfile::Debug),
            ..Default::default()
        };
        let release_profile = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            profile: Some(CargoBuildProfile::Release),
            ..Default::default()
        };

        let debug_args = cargo_plan_args(temp.path(), &debug_profile, false).await;
        let release_args = cargo_plan_args(temp.path(), &release_profile, true).await;

        assert!(!debug_args.iter().any(|arg| arg == "--release"));
        assert!(release_args.iter().any(|arg| arg == "--release"));
    }

    #[tokio::test]
    async fn build_cargo_plan_uses_effective_profile_for_log_feature() {
        let temp = tempfile::tempdir().unwrap();
        write_log_workspace(temp.path(), true);

        let debug_config = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            log: Some(LogLevel::Info),
            profile: Some(CargoBuildProfile::Debug),
            ..Default::default()
        };
        let release_config = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            log: Some(LogLevel::Info),
            profile: Some(CargoBuildProfile::Release),
            ..Default::default()
        };

        let debug_args = cargo_plan_args(temp.path(), &debug_config, false).await;
        let release_args = cargo_plan_args(temp.path(), &release_config, true).await;

        assert_eq!(feature_arg(&debug_args), Some("log/max_level_info"));
        assert_eq!(
            feature_arg(&release_args),
            Some("log/release_max_level_info")
        );
    }

    #[tokio::test]
    async fn build_cargo_plan_skips_log_feature_without_log_dependency() {
        let temp = tempfile::tempdir().unwrap();
        write_log_workspace(temp.path(), false);

        let config = Cargo {
            package: "app".into(),
            target: "x86_64-unknown-none".into(),
            log: Some(LogLevel::Info),
            profile: Some(CargoBuildProfile::Debug),
            ..Default::default()
        };

        let args = cargo_plan_args(temp.path(), &config, false).await;

        assert!(feature_arg(&args).is_none());
    }

    #[tokio::test]
    async fn execute_returns_resolved_cargo_artifact_outcome() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("Cargo.toml"),
            "[package]\nname = \"kernel\"\nversion = \"0.1.0\"\nedition = \"2024\"\n",
        )
        .unwrap();
        fs::create_dir_all(temp.path().join("src")).unwrap();
        fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").unwrap();

        let target_dir = temp.path().join("target");
        fs::create_dir_all(&target_dir).unwrap();
        fs::copy(std::env::current_exe().unwrap(), target_dir.join("kernel")).unwrap();

        let config = Cargo {
            target: "aarch64-unknown-none".into(),
            package: "kernel".into(),
            profile: Some(CargoBuildProfile::Debug),
            ..Default::default()
        };

        let invocation = Invocation::new(InvocationOptions::new(
            Some(temp.path().to_path_buf()),
            None,
            None,
            false,
        ))
        .unwrap();
        let package_id = metadata::cargo_metadata(invocation.project_layout())
            .unwrap()
            .packages
            .iter()
            .find(|package| package.name == "kernel")
            .unwrap()
            .id
            .to_string();

        let cargo_bin = temp.path().join("cargo-bin");
        fs::write(
            &cargo_bin,
            format!(
                "#!/bin/sh\nprintf '%s\\n' '{{\"reason\":\"compiler-artifact\",\"package_id\":\"{package_id}\",\"manifest_path\":\"{root}/Cargo.toml\",\"target\":{{\"kind\":[\"bin\"],\"crate_types\":[\"bin\"],\"name\":\"kernel\",\"src_path\":\"{root}/src/main.rs\",\"edition\":\"2024\",\"doc\":true,\"doctest\":false,\"test\":true}},\"profile\":{{\"opt_level\":\"0\",\"debuginfo\":0,\"debug_assertions\":true,\"overflow_checks\":true,\"test\":false}},\"features\":[],\"filenames\":[],\"executable\":\"{root}/target/kernel\",\"fresh\":false}}'\n",
                root = temp.path().display()
            ),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::metadata(&cargo_bin).unwrap().permissions();
            permissions.set_mode(0o755);
            fs::set_permissions(&cargo_bin, permissions).unwrap();
        }

        let input = cargo_input_for(&invocation, &config, false);
        let outcome = CargoBuildPipeline::build(input, &config)
            .skip_objcopy(true)
            .cargo_program(&cargo_bin)
            .execute()
            .await
            .unwrap();

        assert_eq!(
            outcome.resolved_artifact().elf_path(),
            target_dir.join("kernel")
        );
        assert!(invocation.runtime_artifacts().elf().is_none());
        assert!(invocation.runtime_artifacts().bin().is_none());
        assert!(
            invocation
                .runtime_artifacts()
                .cargo_artifact_dir()
                .is_none()
        );
        assert!(
            invocation
                .runtime_artifacts()
                .runtime_artifact_dir()
                .is_none()
        );
    }
}
