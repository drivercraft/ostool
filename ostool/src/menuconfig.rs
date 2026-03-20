//! TUI-based menu configuration system.
//!
//! This module provides an interactive terminal user interface for configuring
//! build options, similar to Linux kernel's menuconfig. It supports editing
//! configuration for:
//!
//! - Build settings (`.build.toml`)
//! - QEMU settings (`.qemu.toml`)
//! - U-Boot settings (`.uboot.toml`)

use anyhow::Context;
use anyhow::Result;
use clap::ValueEnum;
use log::info;
use tokio::fs;

use crate::ctx::AppContext;
use crate::run::qemu::QemuConfig;
use crate::run::uboot::UbootConfig;
use crate::utils::PathResultExt;

/// Menu configuration mode selector.
#[derive(ValueEnum, Clone, Debug)]
pub enum MenuConfigMode {
    /// Configure QEMU runner settings.
    Qemu,
    /// Configure U-Boot runner settings.
    Uboot,
}

/// Handler for menu configuration operations.
pub struct MenuConfigHandler;

impl MenuConfigHandler {
    /// Handles the menu configuration command.
    ///
    /// # Arguments
    ///
    /// * `ctx` - The application context.
    /// * `mode` - Optional mode specifying which configuration to edit.
    ///   If `None`, shows the default build configuration menu.
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration cannot be loaded or saved.
    pub async fn handle_menuconfig(
        ctx: &mut AppContext,
        mode: Option<MenuConfigMode>,
    ) -> Result<()> {
        match mode {
            Some(MenuConfigMode::Qemu) => {
                Self::handle_qemu_config(ctx).await?;
            }
            Some(MenuConfigMode::Uboot) => {
                Self::handle_uboot_config(ctx).await?;
            }
            None => {
                // 默认模式：显示当前构建配置
                Self::handle_default_config(ctx).await?;
            }
        }
        Ok(())
    }

    async fn handle_default_config(ctx: &mut AppContext) -> Result<()> {
        ctx.prepare_build_config(None, true).await?;

        Ok(())
    }

    async fn handle_qemu_config(ctx: &mut AppContext) -> Result<()> {
        info!("配置 QEMU 运行参数");

        // 使用来自 qemu 模块的共享解析器
        let config_path = crate::run::qemu::resolve_qemu_config_path(ctx, None)?;

        if config_path.exists() {
            println!("\n当前 QEMU 配置文件: {}", config_path.display());
        } else {
            println!("\n未找到 QEMU 配置文件，将使用默认配置");
        }

        let config = jkconfig::run::<QemuConfig>(config_path.clone(), true, &[])
            .await
            .with_context(|| format!("failed to load QEMU config: {}", config_path.display()))?;

        if let Some(c) = config {
            fs::write(&config_path, toml::to_string_pretty(&c)?)
                .await
                .with_path("failed to write file", &config_path)?;
            println!("\nQEMU 配置已保存到 {}", config_path.display());
        } else {
            println!("\n未更改 QEMU 配置");
        }

        Ok(())
    }

    async fn handle_uboot_config(ctx: &mut AppContext) -> Result<()> {
        info!("配置 U-Boot 运行参数");

        println!("=== U-Boot 配置模式 ===");

        // 检查是否存在 U-Boot 配置文件
        let uboot_config_path = ctx.paths.workspace.join(".uboot.toml");
        if uboot_config_path.exists() {
            println!("\n当前 U-Boot 配置文件: {}", uboot_config_path.display());
            // 这里可以读取并显示当前的 U-Boot 配置
        } else {
            println!("\n未找到 U-Boot 配置文件，将使用默认配置");
        }
        let config = jkconfig::run::<UbootConfig>(uboot_config_path.clone(), true, &[])
            .await
            .with_context(|| {
                format!(
                    "failed to load U-Boot config: {}",
                    uboot_config_path.display()
                )
            })?;
        if let Some(c) = config {
            fs::write(&uboot_config_path, toml::to_string_pretty(&c)?)
                .await
                .with_path("failed to write file", &uboot_config_path)?;
            println!("\nU-Boot 配置已保存到 .uboot.toml");
        } else {
            println!("\n未更改 U-Boot 配置");
        }

        Ok(())
    }
}
