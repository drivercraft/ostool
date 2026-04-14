# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.3](https://github.com/drivercraft/ostool/compare/jkconfig-v0.2.2...jkconfig-v0.2.3) - 2026-04-14

### Added

- support forwarding uboot_cmd from board run config ([#83](https://github.com/drivercraft/ostool/pull/83))

## [0.2.2](https://github.com/drivercraft/ostool/compare/jkconfig-v0.2.1...jkconfig-v0.2.2) - 2026-04-03

### Added

- update schema handling to convert "oneOf" with const variants to Enum and add tests for log field validation
- enhance validation feedback for required fields in UI
- add validation for required fields before saving configuration
- add is_empty method to ElementPath for better usability

### Other

- improve formatting and readability in various modules
- simplify element handling and improve hook naming consistency
- adjust terminal height and improve layout rendering in TUI
- improve formatting and readability of key bindings and error messages
- Add theme support and refactor UI components
- Refactor UI and Web Handlers

## [0.2.1](https://github.com/drivercraft/ostool/compare/jkconfig-v0.2.0...jkconfig-v0.2.1) - 2026-04-02

### Other

- simplify default implementation for BoardGlobalConfigFile and improve code clarity
- remove logging dependencies and related code from the project

## [0.2.0](https://github.com/drivercraft/ostool/compare/jkconfig-v0.1.8...jkconfig-v0.2.0) - 2026-04-02

### Added

- add remote support ([#67](https://github.com/drivercraft/ostool/pull/67))

## [0.1.8](https://github.com/drivercraft/ostool/compare/jkconfig-v0.1.7...jkconfig-v0.1.8) - 2026-03-25

### Other

- update Cargo.lock dependencies

## [0.1.7](https://github.com/drivercraft/ostool/compare/jkconfig-v0.1.6...jkconfig-v0.1.7) - 2026-03-20

### Other

- update Cargo.lock dependencies

## [0.1.6](https://github.com/drivercraft/ostool/compare/jkconfig-v0.1.5...jkconfig-v0.1.6) - 2026-01-29

### Fixed

- 更新所有 Cargo.toml 文件中的仓库地址
