# AGENTS.md - ostool-server crate

## 适用范围

适用于 `ostool-server/`。`ostool-server/webui/` 有单独的局部规则。

## 局部规则

- `src/api/` 中的 REST/WebSocket 模型、会话生命周期、开发板租约、TFTP 文件处理和串口
  访问都是跨组件契约。契约变化时，同步更新服务器测试和 web UI client/types。
- 除非用户明确要求，不要在宿主机运行 `scripts/install.sh`、`scripts/update.sh`，不要修改
  systemd unit，也不要触碰 `/etc/ostool-server`。
- `build.rs` 会复制 web UI 并运行 pnpm 构建嵌入资源。保持该行为可复现，不要提交
  `webui/dist`、`node_modules` 或其他生成的依赖输出。
- 电源管理和串口发现改动应保持保守：保留稳定设备身份处理，并让失败模式对调用方可见。

## 验证

- 服务器改动优先运行 `cargo test -p ostool-server`。
- 注意：该 crate 的 `build.rs` 会触发 `pnpm install --frozen-lockfile` 和
  `pnpm run build`，依赖 Node.js 和 pnpm，并会在 Cargo target 目录下生成 web 依赖和
  构建输出。工具不可用时，说明跳过了该检查，不要改用全局安装来掩盖环境差异。
- API 契约、嵌入 web 资源或构建集成变化时，在 Node.js 和 pnpm 可用的环境中同步运行
  `ostool-server/webui` 的局部检查。
