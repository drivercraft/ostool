//! Application context and state management.
//!
//! This module provides the [`AppContext`] type which holds the global state
//! for the ostool application, including paths, build configuration, and
//! architecture information.

use std::{path::PathBuf, sync::Arc};

use anyhow::anyhow;
use cargo_metadata::Metadata;
use colored::Colorize;
use cursive::Cursive;
use jkconfig::{
    ElemHock,
    data::{app_data::AppData, item::ItemType, types::ElementType},
    ui::components::editors::{show_feature_select, show_list_select},
};

use object::{Architecture, Object};
use tokio::fs;

use crate::build::config::BuildConfig;

/// Configuration for output directories.
///
/// Specifies where build outputs should be placed.
#[derive(Default, Clone)]
pub struct OutputConfig {
    /// Custom build directory (overrides default `target/`).
    pub build_dir: Option<PathBuf>,
    /// Custom binary output directory.
    pub bin_dir: Option<PathBuf>,
}

/// Build artifacts generated during the build process.
#[derive(Default, Clone)]
pub struct OutputArtifacts {
    /// Path to the built ELF file.
    pub elf: Option<PathBuf>,
    /// Path to the converted binary file.
    pub bin: Option<PathBuf>,
}

/// Path configuration grouping all path-related fields.
#[derive(Default, Clone)]
pub struct PathConfig {
    /// Workspace root directory.
    pub workspace: PathBuf,
    /// Cargo manifest directory.
    pub manifest: PathBuf,
    /// Output directory configuration.
    pub config: OutputConfig,
    /// Generated build artifacts.
    pub artifacts: OutputArtifacts,
}

impl PathConfig {
    /// Gets the build directory.
    ///
    /// Returns the configured build directory, or defaults to `manifest/target`.
    pub fn build_dir(&self) -> PathBuf {
        self.config
            .build_dir
            .clone()
            .unwrap_or_else(|| self.manifest.join("target"))
    }

    /// Gets the binary output directory if configured.
    pub fn bin_dir(&self) -> Option<PathBuf> {
        self.config.bin_dir.clone()
    }
}

/// The main application context holding all state.
///
/// `AppContext` is the central state container for ostool operations.
/// It manages paths, build configuration, architecture detection, and
/// provides methods for building and running OS projects.
#[derive(Default, Clone)]
pub struct AppContext {
    /// Path configuration for workspace, manifest, and outputs.
    pub paths: PathConfig,
    /// Whether debug mode is enabled.
    pub debug: bool,
    /// Detected CPU architecture from the ELF file.
    pub arch: Option<Architecture>,
    /// Current build configuration.
    pub build_config: Option<BuildConfig>,
    /// Path to the build configuration file.
    pub build_config_path: Option<PathBuf>,
}

impl AppContext {
    /// Executes a shell command in the current context.
    ///
    /// The command is run in the manifest directory with the `KERNEL_ELF`
    /// environment variable set if an ELF artifact is available.
    ///
    /// # Arguments
    ///
    /// * `cmd` - The shell command to execute.
    ///
    /// # Errors
    ///
    /// Returns an error if the command fails to execute.
    pub fn shell_run_cmd(&self, cmd: &str) -> anyhow::Result<()> {
        let mut command = match std::env::consts::OS {
            "windows" => {
                let mut command = self.command("powershell");
                command.arg("-Command");
                command
            }
            _ => {
                let mut command = self.command("sh");
                command.arg("-c");
                command
            }
        };

        command.arg(cmd);

        if let Some(elf) = &self.paths.artifacts.elf {
            command.env("KERNEL_ELF", elf.display().to_string());
        }

        command.run()?;

        Ok(())
    }

    /// Creates a new command builder for the given program.
    ///
    /// The command is configured to run in the manifest directory with
    /// variable substitution support.
    pub fn command(&self, program: &str) -> crate::utils::Command {
        let this = self.clone();
        crate::utils::Command::new(program, &self.paths.manifest, move |s| {
            this.value_replace_with_var(s)
        })
    }

    /// Gets the Cargo metadata for the current workspace.
    ///
    /// # Errors
    ///
    /// Returns an error if `cargo metadata` fails.
    pub fn metadata(&self) -> anyhow::Result<Metadata> {
        let res = cargo_metadata::MetadataCommand::new()
            .current_dir(&self.paths.manifest)
            .no_deps()
            .exec()?;
        Ok(res)
    }

    /// Sets the ELF file path and detects its architecture.
    ///
    /// This also reads the ELF file to detect the target CPU architecture.
    pub async fn set_elf_path(&mut self, path: PathBuf) {
        self.paths.artifacts.elf = Some(path.clone());
        let binary_data = match fs::read(path).await {
            Ok(data) => data,
            Err(e) => {
                println!("Failed to read ELF file: {e}");
                return;
            }
        };
        let file = match object::File::parse(binary_data.as_slice()) {
            Ok(f) => f,
            Err(e) => {
                println!("Failed to parse ELF file: {e}");
                return;
            }
        };
        self.arch = Some(file.architecture())
    }

    /// Strips debug symbols from the ELF file.
    ///
    /// Creates a new `.elf` file with debug symbols stripped using `rust-objcopy`.
    ///
    /// # Returns
    ///
    /// Returns the path to the stripped ELF file.
    ///
    /// # Errors
    ///
    /// Returns an error if no ELF file is set or `rust-objcopy` fails.
    pub fn objcopy_elf(&mut self) -> anyhow::Result<PathBuf> {
        let elf_path = self
            .paths
            .artifacts
            .elf
            .as_ref()
            .ok_or(anyhow!("elf not exist"))?
            .canonicalize()?;

        let stripped_elf_path = elf_path.with_file_name(
            elf_path
                .file_stem()
                .ok_or(anyhow!("Invalid file path"))?
                .to_string_lossy()
                .to_string()
                + ".elf",
        );
        println!(
            "{}",
            format!(
                "Stripping ELF file...\r\n  original elf: {}\r\n  stripped elf: {}",
                elf_path.display(),
                stripped_elf_path.display()
            )
            .bold()
            .purple()
        );

        let mut objcopy = self.command("rust-objcopy");

        objcopy.arg(format!(
            "--binary-architecture={}",
            format!("{:?}", self.arch.unwrap()).to_lowercase()
        ));
        objcopy.arg(&elf_path);
        objcopy.arg(&stripped_elf_path);

        objcopy.run()?;
        self.paths.artifacts.elf = Some(stripped_elf_path.clone());

        Ok(stripped_elf_path)
    }

    /// Converts the ELF file to raw binary format.
    ///
    /// Uses `rust-objcopy` to convert the ELF file to a flat binary file
    /// suitable for direct loading by bootloaders.
    ///
    /// # Returns
    ///
    /// Returns the path to the generated binary file.
    ///
    /// # Errors
    ///
    /// Returns an error if no ELF file is set or `rust-objcopy` fails.
    pub fn objcopy_output_bin(&mut self) -> anyhow::Result<PathBuf> {
        if self.paths.artifacts.bin.is_some() {
            debug!("BIN file already exists: {:?}", self.paths.artifacts.bin);
            return Ok(self.paths.artifacts.bin.as_ref().unwrap().clone());
        }

        let elf_path = self
            .paths
            .artifacts
            .elf
            .as_ref()
            .ok_or(anyhow!("elf not exist"))?
            .canonicalize()?;

        let bin_name = elf_path
            .file_stem()
            .ok_or(anyhow!("Invalid file path"))?
            .to_string_lossy()
            .to_string()
            + ".bin";

        let bin_path = if let Some(bin_dir) = self.paths.config.bin_dir.clone() {
            bin_dir.join(bin_name)
        } else {
            elf_path.with_file_name(bin_name)
        };

        if let Some(parent) = bin_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        println!(
            "{}",
            format!(
                "Converting ELF to BIN format...\r\n  elf: {}\r\n  bin: {}",
                elf_path.display(),
                bin_path.display()
            )
            .bold()
            .purple()
        );

        let mut objcopy = self.command("rust-objcopy");

        if !self.debug {
            objcopy.arg("--strip-all");
        }

        objcopy
            .arg("-O")
            .arg("binary")
            .arg(&elf_path)
            .arg(&bin_path);

        objcopy.run()?;
        self.paths.artifacts.bin = Some(bin_path.clone());

        Ok(bin_path)
    }

    /// Loads and prepares the build configuration.
    ///
    /// This method loads the build configuration from a TOML file. If `menu` is
    /// true, an interactive TUI is shown for configuration editing.
    ///
    /// # Arguments
    ///
    /// * `config_path` - Optional path to the configuration file. Defaults to
    ///   `.build.toml` in the workspace directory.
    /// * `menu` - If true, shows an interactive configuration menu.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration file cannot be loaded or parsed.
    pub async fn prepare_build_config(
        &mut self,
        config_path: Option<PathBuf>,
        menu: bool,
    ) -> anyhow::Result<BuildConfig> {
        let config_path = match config_path {
            Some(path) => path,
            None => self.paths.workspace.join(".build.toml"),
        };
        self.build_config_path = Some(config_path.clone());

        let Some(c): Option<BuildConfig> = jkconfig::run(
            config_path,
            menu,
            &[self.ui_hock_feature_select(), self.ui_hock_pacage_select()],
        )
        .await?
        else {
            anyhow::bail!("No build configuration obtained");
        };

        self.build_config = Some(c.clone());
        Ok(c)
    }

    /// Replaces variable placeholders in a string.
    ///
    /// Currently supports `${workspaceFolder}` which is replaced with the
    /// workspace directory path.
    pub fn value_replace_with_var<S>(&self, value: S) -> String
    where
        S: AsRef<std::ffi::OsStr>,
    {
        let raw = value.as_ref().to_string_lossy();
        raw.replace(
            "${workspaceFolder}",
            format!("{}", self.paths.workspace.display()).as_ref(),
        )
    }

    /// Returns UI hooks for the configuration editor.
    ///
    /// These hooks provide interactive selection dialogs for features and packages.
    pub fn ui_hocks(&self) -> Vec<ElemHock> {
        vec![self.ui_hock_feature_select(), self.ui_hock_pacage_select()]
    }

    fn ui_hock_feature_select(&self) -> ElemHock {
        let path = "system.features";
        let cargo_toml = self.paths.workspace.join("Cargo.toml");
        ElemHock {
            path: path.to_string(),
            callback: Arc::new(move |siv: &mut Cursive, _path: &str| {
                let mut package = String::new();
                if let Some(app) = siv.user_data::<AppData>()
                    && let Some(pkg) = app.root.get_by_key("system.package")
                    && let ElementType::Item(item) = pkg
                    && let ItemType::String { value: Some(v), .. } = &item.item_type
                {
                    package = v.clone();
                }

                // 调用显示特性选择对话框的函数
                show_feature_select(siv, &package, &cargo_toml, None);
            }),
        }
    }

    fn ui_hock_pacage_select(&self) -> ElemHock {
        let path = "system.package";
        let cargo_toml = self.paths.workspace.join("Cargo.toml");

        ElemHock {
            path: path.to_string(),
            callback: Arc::new(move |siv: &mut Cursive, path: &str| {
                let mut items = Vec::new();
                if let Ok(metadata) = cargo_metadata::MetadataCommand::new()
                    .manifest_path(&cargo_toml)
                    .no_deps()
                    .exec()
                {
                    for pkg in &metadata.packages {
                        items.push(pkg.name.to_string());
                    }
                }

                // 调用显示包选择对话框的函数
                show_list_select(siv, "Pacage", &items, path, on_package_selected);
            }),
        }
    }
}

fn on_package_selected(app: &mut AppData, path: &str, selected: &str) {
    let ElementType::Item(item) = app.root.get_mut_by_key(path).unwrap() else {
        panic!("Not an item element");
    };
    let ItemType::String { value, .. } = &mut item.item_type else {
        panic!("Not a string item");
    };
    *value = Some(selected.to_string());
}
