# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.1.8](https://github.com/drivercraft/ostool/compare/ostool-server-v0.1.7...ostool-server-v0.1.8) - 2026-04-15

### Other

- *(ostool)* CargoRunnerKind and clean up cargo_run calls ([#87](https://github.com/drivercraft/ostool/pull/87))

## [0.1.7](https://github.com/drivercraft/ostool/compare/ostool-server-v0.1.6...ostool-server-v0.1.7) - 2026-04-09

### Added

- add upload limits configuration for DTB and session files

## [0.1.6](https://github.com/drivercraft/ostool/compare/ostool-server-v0.1.5...ostool-server-v0.1.6) - 2026-04-08

### Added

- add session management features and update UI for session status

### Other

- add session deletion test for Zhongsheng relay boards with settle delay

## [0.1.5](https://github.com/drivercraft/ostool/compare/ostool-server-v0.1.4...ostool-server-v0.1.5) - 2026-04-07

### Other

- Use stable serial keys for relay power management

## [0.1.4](https://github.com/drivercraft/ostool/compare/ostool-server-v0.1.3...ostool-server-v0.1.4) - 2026-04-03

### Added

- enhance UI components and add board statistics in the management view
- add upgrade script for ostool-server installation

### Other

- Refactor serial configuration handling and update UI components

## [0.1.3](https://github.com/drivercraft/ostool/compare/ostool-server-v0.1.2...ostool-server-v0.1.3) - 2026-04-03

### Other

- Add integration tests for WebSocket session lifecycle management
- improve formatting and readability in various modules

## [0.1.2](https://github.com/drivercraft/ostool/compare/ostool-server-v0.1.1...ostool-server-v0.1.2) - 2026-04-02

### Added

- add board connect ([#76](https://github.com/drivercraft/ostool/pull/76))

### Fixed

- improve script robustness and update service unit template handling ([#75](https://github.com/drivercraft/ostool/pull/75))
- improve configuration cleanup process in installation script ([#74](https://github.com/drivercraft/ostool/pull/74))
- update README and service configuration for installation clarity ([#72](https://github.com/drivercraft/ostool/pull/72))

## [0.1.1](https://github.com/drivercraft/ostool/compare/ostool-server-v0.1.0...ostool-server-v0.1.1) - 2026-04-02

### Added

- enhance install script and server configuration management

### Other

- reorganize imports and improve code formatting in multiple files
- Improve ostool-server install flow
- update configuration handling in install script and remove default config writing from CLI
- release ([#68](https://github.com/drivercraft/ostool/pull/68))

## [0.1.0](https://github.com/drivercraft/ostool/releases/tag/ostool-server-v0.1.0) - 2026-04-02

### Added

- add remote support ([#67](https://github.com/drivercraft/ostool/pull/67))
- 添加英文 README 文件，提供项目概述和使用说明

### Fixed

- release frontend build and embedding flow ([#69](https://github.com/drivercraft/ostool/pull/69))

### Other

- 修复 README 中的 CI 徽章链接，更新为新的检查工作流
- add serial exit shortcut notes
- update doc
- 新配置文件，支持串口 ([#4](https://github.com/drivercraft/ostool/pull/4))
- doc
- add remote build doc
- add support for rockchip
- 修正文档
- update
- support arceos
- add arceos support
- Initial commit
