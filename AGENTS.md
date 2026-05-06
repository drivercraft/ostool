# AGENTS.md - ostool

## 适用范围

本文件适用于整个仓库。子目录中的 `AGENTS.md` 只对对应目录生效，并覆盖或补充本文件规则。

## 项目结构

- `ostool/`: 主 CLI/库，覆盖 OS 构建、`menuconfig`、QEMU、U-Boot、TFTP、串口、
  board-client 和 `cargo-osrun` 流程。
- `ostool-server/`: 开发板管理服务器，包含 API、串口会话、TFTP 文件、电源管理和
  面向 systemd 的部署脚本。
- `ostool-server/webui/`: `ostool-server` 嵌入的 Vue/Vite/pnpm 前端。
- `jkconfig/`: 基于 Ratatui 的 JSON Schema 配置编辑器库，并提供可选 web 功能。
- `fitimage/`: 用于构建 U-Boot 兼容 FIT 镜像的库。
- `uboot-shell/`: 异步 U-Boot shell 与 YMODEM 通信库。
- `.github/workflows/check.yaml`: 当前格式化、clippy、构建和测试命令的 CI 来源。

## 依赖与工具

- 复现检查时，优先使用本仓库或 CI 声明的工具链和依赖版本。
- 除非任务明确要求维护开发环境，不要新增、删除或替换 Dockerfile、Dev Container、
  安装脚本、包管理器等开发环境入口。
- 当前环境缺少必要工具时，说明缺失工具和未能运行的检查，不要改用未纳入仓库管理的
  宿主机配置来替代。

## Git 与提交

- 除非用户明确要求留在当前分支，否则仓库改动应在功能分支上完成。
- 遵循近期提交风格，使用 Conventional Commits，例如 `fix(ostool): ...`、
  `chore(ostool-server): ...`、`docs: ...` 或 `refactor(jkconfig): ...`。
- 不要把无关改动混入同一个提交。只暂存属于当前任务的文件。

## 验证

- 仓库级 Rust 改动应在可行时复现当前 CI target matrix 中的格式化、clippy、构建和
  测试命令。当前 `.github/workflows/check.yaml` 使用
  `x86_64-unknown-linux-gnu`。
- 局部改动优先运行覆盖被修改区域的最小 package 或 web UI 检查，并明确说明实际运行
  或未运行的命令。
- CI 会安装 QEMU、U-Boot tools、libudev、Node.js 24 和 pnpm 10.33.0。把这些视为
  CI 环境证据，不要把它写成所有本地环境都必须安装同一批工具。

## 文档

- 用户可见的 CLI、服务器 API、配置格式、安装路径或工作流发生变化时，同步更新相关
  README 或局部文档。
- 根 README 同时有中文和英文版本。修改共享的用户文档时，保持两个版本一致。
- changelog 按 package 拆分。只有发布或用户明确要求时才更新 changelog。

## Rust 约定

- 保持各 package 已采用的 edition 和公开 API 风格。
- 配置数据优先使用已有 `serde`、`schemars`、TOML 和 JSON 类型进行结构化解析或序列化，
  不要手写脆弱的字符串处理。
- 串口、TFTP、QEMU、U-Boot 和开发板电源相关改动属于操作敏感路径。副作用必须明确，
  并通过测试或记录过的手动验证覆盖。
