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
use cargo_metadata::{Message, PackageId};
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
                        && artifact.target.is_bin()
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
            default_run.as_deref(),
            &self.config.package,
        )?;

        Ok(resolved)
    }

    async fn build_cargo_command(&mut self) -> anyhow::Result<Command> {
        let mut cmd =
            crate::process::command(self.cargo_program.as_os_str(), &self.input.process_context);

        cmd.arg(&self.command);

        for (k, v) in &self.config.env {
            println!("{}", format!("{k}={v}").cyan());
            cmd.env(k, v);
        }
        for (k, v) in &self.extra_envs {
            println!("{}", format!("{k}={v}").cyan());
            cmd.env(k, v);
        }

        // Extra config
        if let Some(extra_config_path) = self.cargo_extra_config().await? {
            cmd.arg("--config");
            cmd.arg(extra_config_path.display().to_string());
        }

        // Package and target
        cmd.arg("-p");
        cmd.arg(&self.config.package);
        if let Some(bin) = &self.config.bin {
            cmd.arg("--bin");
            cmd.arg(bin);
        }
        cmd.arg("--target");
        cmd.arg(&self.config.target);
        cmd.arg("-Z");
        cmd.arg("unstable-options");

        cmd.arg("--target-dir");
        cmd.arg(self.input.build_dir.display().to_string());

        // Features
        let features = self.build_features();
        if !features.is_empty() {
            cmd.arg("--features");
            cmd.arg(features.join(","));
        }

        // Config args
        for arg in &self.config.args {
            cmd.arg(arg);
        }

        // Auto-detected args from someboot/build-info.toml
        let workspace_manifest = self.input.project_layout.workspace_dir().join("Cargo.toml");
        if self.input.enable_someboot_build_config && workspace_manifest.exists() {
            let detected_args = someboot::detect_build_config_for_package(
                &workspace_manifest,
                &self.config.package,
                &features,
                &self.config.target,
            )
            .with_context(|| {
                format!(
                    "failed to detect someboot build config from {}",
                    workspace_manifest.display()
                )
            })?;
            for arg in detected_args {
                cmd.arg(arg);
            }
        }

        // Release mode
        if self.effective_profile() == CargoBuildProfile::Release {
            cmd.arg("--release");
        }

        cmd.arg("--message-format");
        cmd.arg("json-render-diagnostics");

        // Extra args
        for arg in &self.extra_args {
            cmd.arg(arg);
        }

        Ok(cmd)
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
                format!("{:?}", level).to_lowercase()
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
                    eprintln!("Failed to download config from {}: {}", s, e);
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
            println!("Converting GitHub URL to raw: {} -> {}", url, converted);
            return converted;
        }

        // Not a GitHub URL or already in correct format
        url.to_string()
    }

    async fn download_config_to_temp(&self, url: &str) -> anyhow::Result<PathBuf> {
        use std::time::SystemTime;

        println!("Downloading cargo config from: {}", url);

        // Get system temp directory
        let temp_dir = std::env::temp_dir();

        // Generate filename with timestamp
        let timestamp = SystemTime::now()
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap()
            .as_secs();

        // Extract filename from URL or use default
        let url_path = url.split('/').next_back().unwrap_or("config.toml");
        let filename = format!("cargo_config_{}_{}", timestamp, url_path);
        let target_path = temp_dir.join(filename);

        // Create reqwest client
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .map_err(|e| anyhow::anyhow!("Failed to create HTTP client: {}", e))?;

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
            .map_err(|e| anyhow::anyhow!("Failed to download from {}: {}", url, e))?;

        if !response.status().is_success() {
            return Err(anyhow::anyhow!("HTTP error {}: {}", response.status(), url));
        }

        let content = response
            .bytes()
            .await
            .map_err(|e| anyhow::anyhow!("Failed to read response body: {}", e))?;

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
            config::{Cargo, CargoBuildProfile},
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
        let input = CargoBuildInput::new(
            invocation.project_layout().clone(),
            invocation.process_context().unwrap(),
            invocation.build_dir(),
            invocation
                .state()
                .build_config_path()
                .map(std::path::Path::to_path_buf),
            false,
            !config.disable_someboot_build_config,
        );
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

        let input = CargoBuildInput::new(
            invocation.project_layout().clone(),
            invocation.process_context().unwrap(),
            invocation.build_dir(),
            invocation
                .state()
                .build_config_path()
                .map(std::path::Path::to_path_buf),
            false,
            !config.disable_someboot_build_config,
        );
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
