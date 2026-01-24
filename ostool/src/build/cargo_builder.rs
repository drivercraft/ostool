//! Cargo build command builder and executor.
//!
//! This module provides the [`CargoBuilder`] type for constructing and executing
//! Cargo build commands with customizable options, environment variables, and
//! pre/post build hooks.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
};

use colored::Colorize;

use crate::{build::config::Cargo, ctx::AppContext, utils::Command};

/// A builder for constructing and executing Cargo commands.
///
/// `CargoBuilder` provides a fluent API for configuring Cargo build or run
/// commands with custom arguments, environment variables, and build hooks.
///
/// # Example
///
/// ```rust,no_run
/// use ostool::build::cargo_builder::CargoBuilder;
/// use ostool::build::config::Cargo;
/// use ostool::ctx::AppContext;
///
/// // CargoBuilder is typically used internally by AppContext
/// // See AppContext::cargo_build() and AppContext::cargo_run()
/// ```
pub struct CargoBuilder<'a> {
    ctx: &'a mut AppContext,
    config: &'a Cargo,
    command: String,
    extra_args: Vec<String>,
    extra_envs: HashMap<String, String>,
    skip_objcopy: bool,
    config_path: Option<PathBuf>,
}

impl<'a> CargoBuilder<'a> {
    /// Creates a new `CargoBuilder` for executing `cargo build`.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The application context.
    /// * `config` - The Cargo build configuration.
    /// * `config_path` - Optional path to the configuration file.
    pub fn build(ctx: &'a mut AppContext, config: &'a Cargo, config_path: Option<PathBuf>) -> Self {
        Self {
            ctx,
            config,
            command: "build".to_string(),
            extra_args: Vec::new(),
            extra_envs: HashMap::new(),
            skip_objcopy: false,
            config_path,
        }
    }

    /// Creates a new `CargoBuilder` for executing `cargo run`.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The application context.
    /// * `config` - The Cargo build configuration.
    /// * `config_path` - Optional path to the configuration file.
    pub fn run(ctx: &'a mut AppContext, config: &'a Cargo, config_path: Option<PathBuf>) -> Self {
        Self {
            ctx,
            config,
            command: "run".to_string(),
            extra_args: Vec::new(),
            extra_envs: HashMap::new(),
            skip_objcopy: true,
            config_path,
        }
    }

    /// Returns `true` if this builder is configured for `cargo run`.
    pub fn is_run(&self) -> bool {
        self.command == "run"
    }

    /// Sets the debug mode for the build.
    ///
    /// When enabled, builds in debug mode and enables GDB server for QEMU.
    pub fn debug(self, debug: bool) -> Self {
        self.ctx.debug = debug;
        self
    }

    /// Creates a build command using the context's stored config path.
    pub fn build_auto(ctx: &'a mut AppContext, config: &'a Cargo) -> Self {
        let config_path = ctx.build_config_path.clone();
        Self::build(ctx, config, config_path)
    }

    /// Creates a run command using the context's stored config path.
    pub fn run_auto(ctx: &'a mut AppContext, config: &'a Cargo) -> Self {
        let config_path = ctx.build_config_path.clone();
        Self::run(ctx, config, config_path)
    }

    /// Adds a single argument to the Cargo command.
    pub fn arg(mut self, arg: impl Into<String>) -> Self {
        self.extra_args.push(arg.into());
        self
    }

    /// Adds multiple arguments to the Cargo command.
    pub fn args<I, S>(mut self, args: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.extra_args.extend(args.into_iter().map(|s| s.into()));
        self
    }

    /// Sets an environment variable for the Cargo command.
    pub fn env(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.extra_envs.insert(key.into(), value.into());
        self
    }

    /// Sets whether to skip the objcopy step after building.
    pub fn skip_objcopy(mut self, skip: bool) -> Self {
        self.skip_objcopy = skip;
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
    pub async fn execute(mut self) -> anyhow::Result<()> {
        // 1. Pre-build commands
        self.run_pre_build_cmds()?;

        // 2. Build and run cargo
        self.run_cargo().await?;

        // 3. Handle output
        self.handle_output().await?;

        // 4. Post-build commands
        self.run_post_build_cmds()?;

        Ok(())
    }

    fn run_pre_build_cmds(&mut self) -> anyhow::Result<()> {
        for cmd in &self.config.pre_build_cmds {
            self.ctx.shell_run_cmd(cmd)?;
        }
        Ok(())
    }

    async fn run_cargo(&mut self) -> anyhow::Result<()> {
        let mut cmd = self.build_cargo_command().await?;
        cmd.run()?;
        Ok(())
    }

    async fn build_cargo_command(&mut self) -> anyhow::Result<Command> {
        let mut cmd = self.ctx.command("cargo");

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
        cmd.arg("--target");
        cmd.arg(&self.config.target);
        cmd.arg("-Z");
        cmd.arg("unstable-options");

        if let Some(build_dir) = &self.ctx.paths.config.build_dir {
            cmd.arg("--target-dir");
            cmd.arg(build_dir.display().to_string());
        }

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

        // Release mode
        if !self.ctx.debug {
            cmd.arg("--release");
        }

        // Extra args
        for arg in &self.extra_args {
            cmd.arg(arg);
        }

        if self.is_run() && self.ctx.debug {
            cmd.arg("--debug");
        }

        Ok(cmd)
    }

    async fn handle_output(&mut self) -> anyhow::Result<()> {
        let target_dir = self.ctx.paths.build_dir();

        let elf_path = target_dir
            .join(&self.config.target)
            .join(if self.ctx.debug { "debug" } else { "release" })
            .join(&self.config.package);

        self.ctx.set_elf_path(elf_path).await;

        if self.config.to_bin && !self.skip_objcopy {
            self.ctx.objcopy_output_bin()?;
        }

        Ok(())
    }

    fn run_post_build_cmds(&mut self) -> anyhow::Result<()> {
        for cmd in &self.config.post_build_cmds {
            self.ctx.shell_run_cmd(cmd)?;
        }
        Ok(())
    }

    fn build_features(&self) -> Vec<String> {
        let mut features = self.config.features.clone();
        if let Some(log_level) = self.log_level_feature() {
            features.push(log_level);
        }
        features
    }

    fn log_level_feature(&self) -> Option<String> {
        let level = self.config.log.clone()?;

        let meta = self.ctx.metadata().ok()?;
        let pkg = meta
            .packages
            .iter()
            .find(|p| p.name == self.config.package)?;

        let has_log = pkg.dependencies.iter().any(|dep| dep.name == "log");

        if has_log {
            Some(format!(
                "log/{}max_level_{}",
                if self.ctx.debug { "" } else { "release_" },
                format!("{:?}", level).to_lowercase()
            ))
        } else {
            None
        }
    }

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
                if let Some(ref config_path) = self.config_path {
                    let combined = config_path
                        .parent()
                        .ok_or_else(|| anyhow::anyhow!("Invalid config path"))?
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
            .map_err(|e| anyhow::anyhow!("Failed to write to temp file: {}", e))?;

        println!("Config downloaded to: {}", target_path.display());

        Ok(target_path)
    }
}
