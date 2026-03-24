use std::{env::current_dir, ffi::OsStr, path::PathBuf, sync::Arc};

use anyhow::{Context, anyhow, bail};
use cargo_metadata::Metadata;
use colored::Colorize;
use cursive::Cursive;
use jkconfig::{
    ElemHock,
    data::{app_data::AppData, item::ItemType, types::ElementType},
    ui::components::editors::{show_feature_select, show_list_select},
};
use object::Object;
use tokio::fs;

use crate::{
    build::{
        config::{BuildConfig, BuildSystem, Cargo},
        someboot,
    },
    ctx::AppContext,
    utils::PathResultExt,
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

impl Tool {
    /// Creates a new tool from the provided configuration.
    pub fn new(config: ToolConfig) -> anyhow::Result<Self> {
        let manifest_path = resolve_manifest_path(config.manifest.clone())?;
        let manifest_dir = manifest_path
            .parent()
            .ok_or_else(|| anyhow!("manifest has no parent: {}", manifest_path.display()))?
            .to_path_buf();

        let metadata = cargo_metadata::MetadataCommand::new()
            .manifest_path(&manifest_path)
            .no_deps()
            .exec()
            .with_context(|| {
                format!(
                    "failed to load cargo metadata from {}",
                    manifest_path.display()
                )
            })?;

        Ok(Self {
            config,
            manifest_path,
            manifest_dir,
            workspace_dir: PathBuf::from(metadata.workspace_root.as_std_path()),
            ctx: AppContext::default(),
        })
    }

    pub fn ctx(&self) -> &AppContext {
        &self.ctx
    }

    pub fn ctx_mut(&mut self) -> &mut AppContext {
        &mut self.ctx
    }

    pub fn into_context(self) -> AppContext {
        self.ctx
    }

    pub(crate) fn debug_enabled(&self) -> bool {
        self.config.debug
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

    /// Executes a shell command in the current context.
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

        if let Some(elf) = &self.ctx.artifacts.elf {
            command.env("KERNEL_ELF", elf.display().to_string());
        }

        command.run()?;
        Ok(())
    }

    /// Creates a new command builder for the given program.
    pub fn command(&self, program: &str) -> crate::utils::Command {
        let workspace_dir = self.workspace_dir.clone();
        let mut command = crate::utils::Command::new(program, &self.manifest_dir, move |s| {
            let raw = s.to_string_lossy();
            raw.replace(
                "${workspaceFolder}",
                format!("{}", workspace_dir.display()).as_ref(),
            )
        });
        command.env("WORKSPACE_FOLDER", self.workspace_dir.display().to_string());
        command
    }

    /// Gets the Cargo metadata for the current manifest.
    pub fn metadata(&self) -> anyhow::Result<Metadata> {
        cargo_metadata::MetadataCommand::new()
            .manifest_path(&self.manifest_path)
            .no_deps()
            .exec()
            .with_context(|| {
                format!(
                    "failed to load cargo metadata from {}",
                    self.manifest_path.display()
                )
            })
    }

    /// Sets the ELF artifact path and synchronizes derived runtime metadata.
    pub async fn set_elf_artifact_path(&mut self, path: PathBuf) -> anyhow::Result<()> {
        let path = path
            .canonicalize()
            .with_path("failed to canonicalize file", &path)?;
        let artifact_dir = path
            .parent()
            .ok_or_else(|| anyhow!("invalid ELF file path: {}", path.display()))?
            .to_path_buf();

        self.ctx.artifacts.elf = Some(path.clone());
        self.ctx.artifacts.bin = None;
        self.ctx.artifacts.cargo_artifact_dir = Some(artifact_dir.clone());
        self.ctx.artifacts.runtime_artifact_dir = Some(artifact_dir);

        let binary_data = fs::read(&path)
            .await
            .with_path("failed to read ELF file", &path)?;
        let file = object::File::parse(binary_data.as_slice())
            .with_context(|| format!("failed to parse ELF file: {}", path.display()))?;
        self.ctx.arch = Some(file.architecture());
        Ok(())
    }

    /// Sets the ELF file path and detects its architecture.
    pub async fn set_elf_path(&mut self, path: PathBuf) -> anyhow::Result<()> {
        self.set_elf_artifact_path(path).await
    }

    /// Strips debug symbols from the ELF file.
    pub fn objcopy_elf(&mut self) -> anyhow::Result<PathBuf> {
        let elf_path = self
            .ctx
            .artifacts
            .elf
            .as_ref()
            .ok_or_else(|| anyhow!("elf not exist"))?;
        let elf_path = elf_path
            .canonicalize()
            .with_path("failed to canonicalize file", elf_path)?;

        let stripped_elf_path = elf_path.with_file_name(
            elf_path
                .file_stem()
                .ok_or_else(|| anyhow!("invalid ELF file path: {}", elf_path.display()))?
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
            format!(
                "{:?}",
                self.ctx
                    .arch
                    .ok_or_else(|| anyhow!("architecture not detected"))?
            )
            .to_lowercase()
        ));
        objcopy.arg(&elf_path);
        objcopy.arg(&stripped_elf_path);
        objcopy.run()?;

        self.ctx.artifacts.elf = Some(stripped_elf_path.clone());
        self.ctx.artifacts.bin = None;
        self.ctx.artifacts.cargo_artifact_dir = stripped_elf_path.parent().map(PathBuf::from);
        self.ctx.artifacts.runtime_artifact_dir = stripped_elf_path.parent().map(PathBuf::from);

        Ok(stripped_elf_path)
    }

    /// Converts the ELF file to raw binary format.
    pub fn objcopy_output_bin(&mut self) -> anyhow::Result<PathBuf> {
        if let Some(bin) = &self.ctx.artifacts.bin {
            debug!("BIN file already exists: {:?}", bin);
            return Ok(bin.clone());
        }

        let elf_path = self
            .ctx
            .artifacts
            .elf
            .as_ref()
            .ok_or_else(|| anyhow!("elf not exist"))?;
        let elf_path = elf_path
            .canonicalize()
            .with_path("failed to canonicalize file", elf_path)?;

        let bin_name = elf_path
            .file_stem()
            .ok_or_else(|| anyhow!("invalid ELF file path: {}", elf_path.display()))?
            .to_string_lossy()
            .to_string()
            + ".bin";

        let bin_path = if let Some(bin_dir) = self.bin_dir() {
            bin_dir.join(bin_name)
        } else {
            elf_path.with_file_name(bin_name)
        };

        if let Some(parent) = bin_path.parent() {
            std::fs::create_dir_all(parent).with_path("failed to create directory", parent)?;
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

        if !self.debug_enabled() {
            objcopy.arg("--strip-all");
        }

        objcopy
            .arg("-O")
            .arg("binary")
            .arg(&elf_path)
            .arg(&bin_path);
        objcopy.run()?;

        self.ctx.artifacts.bin = Some(bin_path.clone());
        self.ctx.artifacts.runtime_artifact_dir = bin_path.parent().map(PathBuf::from);
        Ok(bin_path)
    }

    pub(crate) fn resolve_build_config_path(&self, explicit_path: Option<PathBuf>) -> PathBuf {
        explicit_path.unwrap_or_else(|| self.workspace_dir.join(".build.toml"))
    }

    /// Loads and prepares the build configuration.
    pub async fn prepare_build_config(
        &mut self,
        config_path: Option<PathBuf>,
        menu: bool,
    ) -> anyhow::Result<BuildConfig> {
        let config_path = self.resolve_build_config_path(config_path);
        self.ctx.build_config_path = Some(config_path.clone());

        let Some(mut c): Option<BuildConfig> = jkconfig::run(
            config_path.clone(),
            menu,
            &[self.ui_hock_feature_select(), self.ui_hock_pacage_select()],
        )
        .await
        .with_context(|| format!("failed to load build config: {}", config_path.display()))?
        else {
            bail!("No build configuration obtained");
        };

        if let BuildSystem::Cargo(cargo) = &mut c.system {
            let iter = self.someboot_cargo_args(cargo)?.into_iter();
            cargo.args.extend(iter);
        }

        self.ctx.build_config = Some(c.clone());
        Ok(c)
    }

    fn someboot_cargo_args(&self, cargo: &Cargo) -> anyhow::Result<Vec<String>> {
        let manifest_path = self.workspace_dir.join("Cargo.toml");
        someboot::detect_build_config_for_package(
            &manifest_path,
            &cargo.package,
            &cargo.features,
            &cargo.target,
        )
    }

    pub fn value_replace_with_var<S>(&self, value: S) -> String
    where
        S: AsRef<OsStr>,
    {
        let raw = value.as_ref().to_string_lossy();
        raw.replace(
            "${workspaceFolder}",
            format!("{}", self.workspace_dir.display()).as_ref(),
        )
    }

    pub fn ui_hocks(&self) -> Vec<ElemHock> {
        vec![self.ui_hock_feature_select(), self.ui_hock_pacage_select()]
    }

    fn ui_hock_feature_select(&self) -> ElemHock {
        let path = "system.features";
        let cargo_toml = self.workspace_dir.join("Cargo.toml");
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

                show_feature_select(siv, &package, &cargo_toml, None);
            }),
        }
    }

    fn ui_hock_pacage_select(&self) -> ElemHock {
        let path = "system.package";
        let cargo_toml = self.workspace_dir.join("Cargo.toml");

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

fn resolve_manifest_path(input: Option<PathBuf>) -> anyhow::Result<PathBuf> {
    let path = match input {
        Some(path) => path,
        None => current_dir().context("failed to get current working directory")?,
    };

    let manifest_path = if path.is_dir() {
        path.join("Cargo.toml")
    } else {
        path
    };

    if manifest_path.file_name().and_then(|name| name.to_str()) != Some("Cargo.toml") {
        bail!(
            "manifest must be a Cargo.toml file or a directory containing Cargo.toml: {}",
            manifest_path.display()
        );
    }

    if !manifest_path.exists() {
        bail!("Cargo.toml not found: {}", manifest_path.display());
    }

    manifest_path
        .canonicalize()
        .with_path("failed to canonicalize manifest path", &manifest_path)
}

#[cfg(test)]
mod tests {
    use super::{Tool, ToolConfig};

    #[tokio::test]
    async fn set_elf_artifact_path_updates_dirs_and_arch() {
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
        tool.set_elf_artifact_path(copied.clone()).await.unwrap();

        let expected_elf = copied.canonicalize().unwrap();
        let expected_dir = expected_elf.parent().unwrap().to_path_buf();

        assert_eq!(tool.ctx.artifacts.elf.as_ref(), Some(&expected_elf));
        assert_eq!(
            tool.ctx.artifacts.cargo_artifact_dir.as_ref(),
            Some(&expected_dir)
        );
        assert_eq!(
            tool.ctx.artifacts.runtime_artifact_dir.as_ref(),
            Some(&expected_dir)
        );
        assert!(tool.ctx.arch.is_some());
        assert!(tool.ctx.artifacts.bin.is_none());
    }
}
