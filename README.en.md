# ostool

[![Check](https://github.com/drivercraft/ostool/actions/workflows/check.yaml/badge.svg)](https://github.com/drivercraft/ostool/actions/workflows/check.yaml)
[![Crates.io](https://img.shields.io/crates/v/ostool.svg)](https://crates.io/crates/ostool)
[![License](https://img.shields.io/badge/license-MIT%2FApache--2.0-blue.svg)](LICENSE)
[![Rust](https://img.shields.io/badge/rust-1.70+-orange.svg)](https://www.rust-lang.org)

---

## 🌐 Language | 语言

**简体中文** | **[English](README.md)** (当前 | Current)

---

## 📖 Project Overview

**ostool** is a Rust toolset designed specifically for operating system development, aiming to provide OS developers with convenient build, configuration, and startup environments. It's particularly suitable for embedded system development, supporting system testing and debugging through Qemu virtual machines and U-Boot bootloader.

### ✨ Core Features

- 🔧 **Integrated Toolchain** - Complete solution integrating build, configuration, and execution
- 🖥️ **Modern TUI** - Terminal-based user interface providing intuitive configuration editing experience
- ⚙️ **Smart Configuration Management** - JSON Schema-driven configuration validation and editing
- 🚀 **Multiple Boot Methods** - Support for both Qemu virtual machines and U-Boot hardware boot
- 🌐 **Cross-platform Support** - Compatible with Linux, Windows, and other platforms
- 📦 **Modular Architecture** - Extensible component design for customization and integration

## 🏗️ Project Architecture

ostool uses Rust workspace architecture, containing the following core modules:

### Core Components

| Component | Description | Primary Use |
|-----------|-------------|-------------|
| **ostool** | Main toolkit | CLI tools for building and running systems |
| **jkconfig** | Configuration editor | TUI configuration editing interface |
| **fitimage** | FIT image builder | U-Boot compatible boot image generation |
| **uboot-shell** | U-Boot communication | Serial communication and command execution |

### Technology Stack

- **Rust** - Core development language providing memory safety and performance
- **Ratatui** - Modern TUI framework
- **JSON Schema** - Configuration validation and type safety
- **Tokio** - Async runtime
- **Serialport** - Serial communication
- **Clap** - Command-line argument parsing

## 🚀 Quick Start

### Installation

```bash
# Install from crates.io
cargo install ostool

# Or build from source
git clone https://github.com/ZR233/ostool.git
cd ostool
cargo install --path .
```

### Basic Usage

#### 1. View Help

```bash
# View main help
ostool --help

# View build help
ostool build --help

# View run help
ostool run --help

# View configuration help
ostool menuconfig --help
```

#### 2. Configuration Management

```bash
# Use TUI to edit build configuration
ostool menuconfig

# Configure QEMU runtime parameters
ostool menuconfig qemu

# Configure U-Boot runtime parameters
ostool menuconfig uboot
```

#### 3. Build System

```bash
# Build project (using default config file .build.toml)
ostool build

# Build with specific config file
ostool build --config custom-build.toml

# Build in specified working directory
ostool --workdir /path/to/project build
```

#### 4. Run System

```bash
# Run with Qemu
ostool run qemu

# Run with Qemu and enable debugging
ostool run qemu --debug

# Run with Qemu and dump DTB file
ostool run qemu --dtb-dump

# Run with specific Qemu config file
ostool run qemu --qemu-config my-qemu.toml

# Run with U-Boot
ostool run uboot

# Run with specific U-Boot config file
ostool run uboot --uboot-config my-uboot.toml

# Configure the remote board server
ostool board config

# List remote board types
ostool board ls

# Run on a remote board
ostool board run
```

> Exit shortcut: In the serial terminal (e.g., `ostool run uboot`), press `Ctrl+A` then `x` to quit; the tool captures this sequence and exits gracefully instead of sending it to the target device.
> For more keyboard mappings, see `ostool/src/sterm/mod.rs`.

## ⚙️ Configuration Files

ostool uses multiple independent TOML configuration files, each responsible for different functional modules:

### Build Configuration (.build.toml)

The build configuration file defines how to compile your operating system kernel.

#### Cargo Build System Example

```toml
[system]
# Use Cargo build system
system = "Cargo"

[system.Cargo]
# Target triple
target = "aarch64-unknown-none"

# Package name
package = "my-os-kernel"

# Enabled features
features = ["page-alloc-4g"]

# Log level
log = "Info"

# Environment variables
env = { "RUSTFLAGS" = "-C link-arg=-Tlinker.ld" }

# Additional cargo arguments
args = ["--release"]

# Pre-build commands
pre_build_cmds = ["make prepare"]

# Post-build commands
post_build_cmds = ["make post-process"]

# Output as binary file
to_bin = true
```

#### Custom Build System Example

```toml
[system]
# Use custom build system
system = "Custom"

[system.Custom]
# Build command
build_cmd = "make ARCH=aarch64 A=examples/helloworld"

# Generated ELF file path
elf_path = "examples/helloworld/helloworld_aarch64-qemu-virt.elf"

# Output as binary file
to_bin = true
```

### QEMU Configuration (.qemu.toml)

The QEMU configuration file defines virtual machine startup parameters.

```toml
# QEMU startup arguments
args = ["-machine", "virt", "-cpu", "cortex-a57", "-nographic"]

# Enable UEFI boot
uefi = false

# Output as binary file
to_bin = true

# Success regex patterns (for auto-detection)
success_regex = ["Hello from my OS", "Kernel booted successfully"]

# Failure regex patterns (for auto-detection)
fail_regex = ["panic", "error", "failed"]
```

### U-Boot Configuration (.uboot.toml)

The U-Boot configuration file defines hardware startup parameters.

```toml
# Serial device
serial = "/dev/ttyUSB0"

# Baud rate
baud_rate = "115200"

# Device tree file (optional)
dtb_file = "tools/device_tree.dtb"

# Kernel load address (optional)
kernel_load_addr = "0x80080000"

# Network boot configuration (optional)
[net]
interface = "eth0"
board_ip = "192.168.1.100"

# Board reset command (optional)
board_reset_cmd = "reset"

# Board power off command (optional)
board_power_off_cmd = "poweroff"

# Success boot regex patterns
success_regex = ["Starting kernel", "Boot successful"]

# Failure boot regex patterns
fail_regex = ["Boot failed", "Error loading kernel"]
```

### Environment Variable Support

Configuration files support environment variable substitution using `${env:VAR_NAME:-default}` format:

```toml
# .uboot.toml example
serial = "${env:SERIAL_DEVICE:-/dev/ttyUSB0}"
baud_rate = "${env:BAUD_RATE:-115200}"
```

### Board Global Config (`~/.ostool/config.toml`)

`ostool board` commands read a user-level global config. The first board command will create it automatically when missing:

```toml
[board]
server_ip = "localhost"
port = 2999
```

Use the TUI editor to update it:

```bash
ostool board config
```

Project-local `.board.toml` `server` / `port` fields still apply to `ostool board run`, with precedence lower than CLI flags and higher than the global config.

## 🛠️ Subproject Details

### JKConfig - Smart Configuration Editor

**JKConfig** is a TUI configuration editor based on JSON Schema, providing the following features:

#### Main Features

- 🎯 **Smart Interface Generation** - Automatically generate editing interfaces from JSON Schema
- 🔒 **Type Safety** - Support complex data types and validation rules
- 📝 **Multi-format Support** - Read/write TOML, JSON formats
- 💾 **Automatic Backup** - Automatically create backup files when saving
- ⌨️ **Shortcut Key Support** - Vim-style keyboard operations

#### Usage

```bash
# Install
cargo install jkconfig

# Edit configuration
jkconfig -c config.toml -s config-schema.json

# Auto-detect schema
jkconfig -c config.toml
```

#### Keyboard Shortcuts

```text
Navigation:
↑/↓ or j/k       - Move up/down
Enter            - Edit item
Esc              - Return to upper level

Operations:
S                - Save and exit
Q                - Exit without saving
C                - Clear current value
M                - Toggle menu state
Tab              - Switch options
~                - Debug console
```

### FitImage - FIT Image Builder

**FitImage** is a professional tool for creating U-Boot compatible FIT (Flattened Image Tree) images:

#### Main Features

- 🏗️ **Standard FIT Format** - Fully compliant with U-Boot FIT specification
- 📦 **Multi-component Support** - Kernel, device tree, ramdisk, etc.
- 🗜️ **Compression Support** - gzip compression to reduce image size
- 🔐 **Checksum Support** - Multiple checksum algorithms like CRC32, SHA1
- 🎯 **Architecture Compatibility** - ARM, ARM64, and other architectures

#### Usage Example

```rust
use fitimage::{FitImageBuilder, FitImageConfig, ComponentConfig};

// Create FIT image configuration
let config = FitImageConfig::new("My FIT Image")
    .with_kernel(
        ComponentConfig::new("kernel", kernel_data)
            .with_type("kernel")
            .with_arch("arm64")
            .with_load_address(0x80080000)
    )
    .with_fdt(
        ComponentConfig::new("fdt", fdt_data)
            .with_type("flat_dt")
            .with_arch("arm64")
    );

// Build image
let mut builder = FitImageBuilder::new();
let fit_data = builder.build(config)?;

// Save file
std::fs::write("image.fit", fit_data)?;
```

## 🎯 Use Cases

### 1. Local Development Workflow

```bash
# 1. Initialize project
git clone <your-os-project>
cd <your-os-project>

# 2. Use menuconfig to configure build parameters
ostool menuconfig

# 3. Configure QEMU runtime parameters
ostool menuconfig qemu

# 4. Build project
ostool build

# 5. Run with Qemu
ostool run qemu

# 6. Run with debug mode
ostool run qemu --debug
```

### 2. Remote Build and Hardware Testing

```bash
# 1. Use menuconfig to configure custom build
ostool menuconfig

# 2. Configure U-Boot runtime parameters
ostool menuconfig uboot

# 3. Execute build
ostool build

# 4. Boot to hardware via U-Boot
ostool run uboot

# 5. Run with custom U-Boot config
ostool run uboot --uboot-config custom-uboot.toml
```

### 3. Embedded System Development

- 🎯 **Multi-architecture support** - ARM64, RISC-V64, and other architectures
- 🔧 **Device tree management** - Automatic DTB file handling and device tree configuration
- 📡 **Network boot** - Support for TFTP network boot and remote loading
- 🖥️ **Serial debugging** - Real-time serial monitoring and debug output
- 🔐 **FIT images** - Create U-Boot compatible FIT boot images
- ⚡ **Automated builds** - Support for pre/post-build scripts and custom commands

### 4. Advanced Debugging Scenarios

```bash
# Enable verbose logging
RUST_LOG=debug ostool run qemu

# Dump DTB file for debugging
ostool run qemu --dtb-dump

# Work in specified directory
ostool --workdir /path/to/kernel build
ostool --workdir /path/to/kernel run qemu
```

## 🔧 Advanced Configuration

### U-Boot Network Boot Setup

```bash
# TFTP requires root privileges to bind port 69
sudo setcap cap_net_bind_service=+eip $(which ostool)
```

### Debug Configuration

```toml
[qemu]
args = "-s -S"  # Enable GDB debugging

[uboot]
# Enable verbose logging
log_level = "debug"
```

## 🐛 Troubleshooting

### Common Issues

**Q: U-Boot boot failure?**
A: Check the following:
- Serial device path is correct (`/dev/ttyUSB0` or other)
- Serial permissions are sufficient (may need `sudo usermod -a -G dialout $USER`)
- Baud rate settings match hardware
- Device tree file path is correct

**Q: Qemu won't start?**
A: Check the following:
- Built kernel file exists
- Architecture parameters in QEMU configuration are correct
- QEMU for target architecture is installed (e.g., `qemu-system-aarch64`)

**Q: Build failure?**
A: Check the following:
- Build configuration file format is correct
- Custom build commands can execute in terminal
- Cross-compilation toolchain for target architecture is installed

**Q: Configuration file format error?**
A: Check the following:
- TOML syntax is correct (use online TOML validators)
- Configuration file uses correct field names
- Array and string formats conform to specifications

**Q: menuconfig won't start?**
A: Check the following:

- Terminal supports TUI interface
- Necessary dependencies are installed (such as ncurses)
- Configuration file permissions are correct

### Debugging Tips

```bash
# Enable verbose logging
RUST_LOG=debug ostool run qemu

# View complete command-line help
ostool --help
ostool build --help
ostool run --help
ostool run qemu --help
ostool run uboot --help
ostool menuconfig --help

# Check if configuration files are loaded correctly
RUST_LOG=debug ostool build 2>&1 | grep -i config

# Debug in specified working directory
ostool --workdir /path/to/project build
```

### Permission Issues Resolution

```bash
# Add user to dialout group for serial device access
sudo usermod -a -G dialout $USER
# Re-login or restart for permissions to take effect

# Or temporarily use sudo
sudo ostool run uboot
```

## 🤝 Contributing

We welcome community contributions! Please follow these steps:

1. **Fork** this repository
2. **Create** a feature branch (`git checkout -b feature/amazing-feature`)
3. **Commit** your changes (`git commit -m 'Add some amazing feature'`)
4. **Push** to the branch (`git push origin feature/amazing-feature`)
5. **Create** a Pull Request

### Development Environment Setup

```bash
git clone https://github.com/ZR233/ostool.git
cd ostool
cargo build
cargo test
```

## 📄 License

This project is dual-licensed:

- [MIT License](LICENSE)
- [Apache License 2.0](LICENSE)

## 🔗 Related Links

- [GitHub Repository](https://github.com/ZR233/ostool)
- [Crates.io Package](https://crates.io/crates/ostool)
- [Issue Tracker](https://github.com/ZR233/ostool/issues)
- [Documentation Wiki](https://github.com/ZR233/ostool/wiki)

## 🙏 Acknowledgments

Thanks to all developers and users who have contributed to the ostool project!
