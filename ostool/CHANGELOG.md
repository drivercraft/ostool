# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.23.4](https://github.com/drivercraft/ostool/compare/ostool-v0.23.3...ostool-v0.23.4) - 2026-06-18

### Added

- *(ostool)* expose prepared runtime board APIs ([#143](https://github.com/drivercraft/ostool/pull/143))

## [0.23.3](https://github.com/drivercraft/ostool/compare/ostool-v0.23.2...ostool-v0.23.3) - 2026-06-18

### Added

- *(build)* expose Cargo build artifact output ([#139](https://github.com/drivercraft/ostool/pull/139))

### Fixed

- *(ostool)* use toolchain llvm-objcopy for bin artifacts ([#140](https://github.com/drivercraft/ostool/pull/140))

## [0.23.2](https://github.com/drivercraft/ostool/compare/ostool-v0.23.1...ostool-v0.23.2) - 2026-06-16

### Fixed

- *(ostool)* tolerate closed serial bridge consumer ([#137](https://github.com/drivercraft/ostool/pull/137))

## [0.23.1](https://github.com/drivercraft/ostool/compare/ostool-v0.23.0...ostool-v0.23.1) - 2026-06-16

### Other

- update Cargo.lock dependencies

## [0.23.0](https://github.com/drivercraft/ostool/compare/ostool-v0.22.1...ostool-v0.23.0) - 2026-06-15

### Added

- Add UEFI HTTP boot support ([#121](https://github.com/drivercraft/ostool/pull/121))

### Other

- *(tool)* 抽出 QEMU command 与 runner 执行边界 ([#127](https://github.com/drivercraft/ostool/pull/127))

## [0.22.1](https://github.com/drivercraft/ostool/compare/ostool-v0.22.0...ostool-v0.22.1) - 2026-06-06

### Fixed

- *(ostool)* prepare raw bin for runtime runners ([#125](https://github.com/drivercraft/ostool/pull/125))

## [0.22.0](https://github.com/drivercraft/ostool/compare/ostool-v0.21.0...ostool-v0.22.0) - 2026-06-05

### Fixed

- *(ostool)* add OVMF mirror fallback ([#124](https://github.com/drivercraft/ostool/pull/124))

### Other

- *(tool)* 抽出 U-Boot FIT 与 boot artifact 边界 ([#123](https://github.com/drivercraft/ostool/pull/123))
- *(ostool)* 收敛 artifact 生命周期边界 ([#119](https://github.com/drivercraft/ostool/pull/119))
- *(tool)* 移除旧 Tool 兼容 API ([#118](https://github.com/drivercraft/ostool/pull/118))
- *(tool)* 拆分运行入口与 Tool facade ([#117](https://github.com/drivercraft/ostool/pull/117))
- *(tool)* 接入 invocation 构建状态 ([#115](https://github.com/drivercraft/ostool/pull/115))

## [0.21.0](https://github.com/drivercraft/ostool/compare/ostool-v0.20.0...ostool-v0.21.0) - 2026-05-27

### Added

- *(sterm)* forward mouse events from host terminal to guest serial ([#113](https://github.com/drivercraft/ostool/pull/113))

### Other

- *(tool)* 收敛构建配置与运行产物边界 ([#114](https://github.com/drivercraft/ostool/pull/114))
- *(tool)* 拆分构建与运行产物边界 ([#111](https://github.com/drivercraft/ostool/pull/111))

## [0.20.0](https://github.com/drivercraft/ostool/compare/ostool-v0.19.0...ostool-v0.20.0) - 2026-05-22

### Other

- *(tool)* 拆分调用、项目与进程上下文 ([#108](https://github.com/drivercraft/ostool/pull/108))
- Add board-specific U-Boot load address configuration to ostool-server ([#109](https://github.com/drivercraft/ostool/pull/109))

## [0.19.0](https://github.com/drivercraft/ostool/compare/ostool-v0.18.0...ostool-v0.19.0) - 2026-05-21

### Added

- 添加禁用 someboot 自动注入 Cargo 参数的配置选项 ([#106](https://github.com/drivercraft/ostool/pull/106))

## [0.18.0](https://github.com/drivercraft/ostool/compare/ostool-v0.17.0...ostool-v0.18.0) - 2026-05-21

### Added

- *(ostool)* support board-specific U-Boot load addresses ([#105](https://github.com/drivercraft/ostool/pull/105))

### Other

- 添加 AGENTS 指南 ([#95](https://github.com/drivercraft/ostool/pull/95))

## [0.17.0](https://github.com/drivercraft/ostool/compare/ostool-v0.16.0...ostool-v0.17.0) - 2026-05-14

### Added

- 添加 U-Boot 网络模式配置（DHCP/静态IP） ([#101](https://github.com/drivercraft/ostool/pull/101))

## [0.16.0](https://github.com/drivercraft/ostool/compare/ostool-v0.15.1...ostool-v0.16.0) - 2026-05-12

### Added

- *(ostool)* 添加 Cargo 二进制目标选择 ([#97](https://github.com/drivercraft/ostool/pull/97))
- *(ostool)* 支持显式配置 Cargo 构建 profile ([#96](https://github.com/drivercraft/ostool/pull/96))

## [0.15.1](https://github.com/drivercraft/ostool/compare/ostool-v0.15.0...ostool-v0.15.1) - 2026-04-30

### Fixed

- *(sterm)* remove automatic \n to \r\n conversion in write_output ([#92](https://github.com/drivercraft/ostool/pull/92))

## [0.15.0](https://github.com/drivercraft/ostool/compare/ostool-v0.14.0...ostool-v0.15.0) - 2026-04-15

### Fixed

- *(uboot)* honor board timeout in serial interaction stage ([#88](https://github.com/drivercraft/ostool/pull/88))

## [0.14.0](https://github.com/drivercraft/ostool/compare/ostool-v0.13.0...ostool-v0.14.0) - 2026-04-15

### Other

- *(ostool)* CargoRunnerKind and clean up cargo_run calls ([#87](https://github.com/drivercraft/ostool/pull/87))
- *(ostool)* improve api ([#85](https://github.com/drivercraft/ostool/pull/85))

## [0.13.0](https://github.com/drivercraft/ostool/compare/ostool-v0.12.4...ostool-v0.13.0) - 2026-04-14

### Added

- support forwarding uboot_cmd from board run config ([#83](https://github.com/drivercraft/ostool/pull/83))

## [0.12.4](https://github.com/drivercraft/ostool/compare/ostool-v0.12.3...ostool-v0.12.4) - 2026-04-03

### Added

- enhance UI components and add board statistics in the management view
- add apply_overrides method to BoardRunConfig and refactor board command handling

## [0.12.3](https://github.com/drivercraft/ostool/compare/ostool-v0.12.2...ostool-v0.12.3) - 2026-04-03

### Added

- update schema handling to convert "oneOf" with const variants to Enum and add tests for log field validation
- add validation for required fields before saving configuration

### Other

- improve formatting and readability in various modules
- simplify element handling and improve hook naming consistency
- Add theme support and refactor UI components
- Refactor UI and Web Handlers

## [0.12.2](https://github.com/drivercraft/ostool/compare/ostool-v0.12.1...ostool-v0.12.2) - 2026-04-02

### Added

- add board connect ([#76](https://github.com/drivercraft/ostool/pull/76))

### Added

- add `ostool board connect -b <board-type>` for lightweight interactive board shell access via `ostool-server`

## [0.12.1](https://github.com/drivercraft/ostool/compare/ostool-v0.12.0...ostool-v0.12.1) - 2026-04-02

### Added

- enhance board configuration input handling and improve terminal interaction

### Other

- simplify default implementation for BoardGlobalConfigFile and improve code clarity
- reorganize imports and improve code formatting in multiple files
- Update dependencies and improve hash calculations
- remove logging dependencies and related code from the project
- update configuration handling in install script and remove default config writing from CLI

## [0.12.0](https://github.com/drivercraft/ostool/compare/ostool-v0.11.2...ostool-v0.12.0) - 2026-04-02

### Added

- add remote support ([#67](https://github.com/drivercraft/ostool/pull/67))

## [0.11.2](https://github.com/drivercraft/ostool/compare/ostool-v0.11.1...ostool-v0.11.2) - 2026-03-30

### Fixed

- fix uboot tftp_dir bootfile path ([#64](https://github.com/drivercraft/ostool/pull/64))

## [0.11.1](https://github.com/ZR233/ostool/compare/ostool-v0.11.0...ostool-v0.11.1) - 2026-03-27

### Added

- 增加 U-Boot 运行时错误上下文信息，改进超时处理逻辑

## [0.11.0](https://github.com/drivercraft/ostool/compare/ostool-v0.10.1...ostool-v0.11.0) - 2026-03-26

### Added

- 增加 Riscv64 架构支持 ([#59](https://github.com/drivercraft/ostool/pull/59))
- 增强 QEMU 运行时参数支持，添加覆盖和追加参数结构 ([#60](https://github.com/drivercraft/ostool/pull/60))

## [0.10.1](https://github.com/drivercraft/ostool/compare/ostool-v0.10.0...ostool-v0.10.1) - 2026-03-26

### Added

- 增加对 U-Boot 和 QEMU 配置中字符串替换的支持，改进环境变量处理 ([#57](https://github.com/drivercraft/ostool/pull/57))

## [0.10.0](https://github.com/drivercraft/ostool/compare/ostool-v0.9.0...ostool-v0.10.0) - 2026-03-25

### Added

- 添加超时配置支持到 QEMU 和 U-Boot 运行器，改进串口终端的超时处理 ([#55](https://github.com/drivercraft/ostool/pull/55))

## [0.9.0](https://github.com/drivercraft/ostool/compare/ostool-v0.8.16...ostool-v0.9.0) - 2026-03-20

### Added

- 增强 QEMU 默认覆盖与配置文件解析 ([#53](https://github.com/drivercraft/ostool/pull/53))

## [0.8.16](https://github.com/drivercraft/ostool/compare/ostool-v0.8.15...ostool-v0.8.16) - 2026-03-19

### Added

- 添加字节流匹配器以增强运行时输出检测功能 ([#51](https://github.com/drivercraft/ostool/pull/51))

## [0.8.15](https://github.com/drivercraft/ostool/compare/ostool-v0.8.14...ostool-v0.8.15) - 2026-03-18

### Added

- 实现 QEMU 配置文件查找功能，支持架构优先级 ([#49](https://github.com/drivercraft/ostool/pull/49))

### Other

- *(ostool)* release v0.8.14 ([#47](https://github.com/drivercraft/ostool/pull/47))

## [0.8.14](https://github.com/drivercraft/ostool/compare/ostool-v0.8.13...ostool-v0.8.14) - 2026-03-17

### Other

- 更新版本号至 0.8.14，并增强对 someboot 的构建配置检测支持 ([#46](https://github.com/drivercraft/ostool/pull/46))
- *(ostool)* release v0.8.13 ([#41](https://github.com/drivercraft/ostool/pull/41))

## [0.8.13](https://github.com/drivercraft/ostool/compare/ostool-v0.8.12...ostool-v0.8.13) - 2026-03-16

### Other

- 更新版本号至 0.8.13，并增强 QEMU UEFI 支持 ([#42](https://github.com/drivercraft/ostool/pull/42))
- *(ostool)* release v0.8.12 ([#40](https://github.com/drivercraft/ostool/pull/40))

## [0.8.12](https://github.com/drivercraft/ostool/compare/ostool-v0.8.11...ostool-v0.8.12) - 2026-03-16

### Other

- 优化 ostool run，减少依赖条件 ([#39](https://github.com/drivercraft/ostool/pull/39))

## [0.8.11](https://github.com/drivercraft/ostool/compare/ostool-v0.8.10...ostool-v0.8.11) - 2026-01-29

### Other

- release

## [0.8.10](https://github.com/drivercraft/ostool/compare/ostool-v0.8.9...ostool-v0.8.10) - 2026-01-29

### Other

- 修复 README 中的 CI 徽章链接，更新为新的检查工作流
