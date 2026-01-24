//! # ostool
//!
//! A comprehensive toolkit for operating system development.
//!
//! `ostool` provides utilities for building, running, and debugging operating systems,
//! with special support for embedded systems and bootloader interaction.
//!
//! ## Features
//!
//! - **Build System**: Cargo-based build configuration with customizable options
//! - **QEMU Integration**: Easy QEMU launching with various architectures support
//! - **U-Boot Support**: Serial communication and file transfer via YMODEM
//! - **TFTP Server**: Built-in TFTP server for network booting
//! - **Menu Configuration**: TUI-based configuration editor (like Linux kernel's menuconfig)
//! - **Serial Terminal**: Interactive serial terminal for device communication
//!
//! ## Modules
//!
//! - [`build`] - Build system configuration and Cargo integration
//! - [`ctx`] - Application context and state management
//! - [`menuconfig`] - TUI-based menu configuration
//! - [`run`] - QEMU, TFTP, and U-Boot runners
//! - [`sterm`] - Serial terminal implementation
//! - [`utils`] - Common utilities and helper functions
//!
//! ## Example
//!
//! ```rust,no_run
//! // ostool is primarily used as a CLI tool
//! // See the binary targets for usage examples
//! ```

#![cfg(not(target_os = "none"))]

/// Build system configuration and Cargo integration.
///
/// Provides functionality for configuring and executing Cargo builds
/// with custom options and target specifications.
pub mod build;

/// Application context and state management.
pub mod ctx;

/// TUI-based menu configuration system.
///
/// Similar to Linux kernel's menuconfig, allows users to configure
/// build options through an interactive terminal interface.
pub mod menuconfig;

/// Runtime execution modules for QEMU, TFTP, and U-Boot.
///
/// Contains implementations for launching QEMU instances,
/// running TFTP servers, and communicating with U-Boot.
pub mod run;

/// Serial terminal implementation.
///
/// Provides an interactive serial terminal for communication
/// with embedded devices and development boards.
pub mod sterm;

/// Common utilities and helper functions.
pub mod utils;

#[macro_use]
extern crate log;
#[macro_use]
extern crate anyhow;

pub use jkconfig::cursive;
