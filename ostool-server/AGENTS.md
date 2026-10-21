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
- 数据库连接和板卡、DTB 仓储位于 `src/persistence/`，本地版和公网版都只使用 PostgreSQL。
  不保留旧模块别名、文件后端或启动时导入。已有板卡的单向迁移只在独立的
  `tools/migrate-boards.rs` 和 `scripts/migrate-boards.sh` 中实现，不得引入运行时旧数据兼容。
- `src/static_assets.rs` 负责嵌入前端资源；SQLx 迁移继续放在 package 根的 `migrations/`。

## 验证

- 服务器测试需要独立的 PostgreSQL，设置 `OSTOOL_TEST_DATABASE_URL` 和
  `RUST_TEST_THREADS=4` 后优先运行 `cargo test -p ostool-server`；禁止使用生产数据库。
- 注意：该 crate 的 `build.rs` 会触发 `pnpm install --frozen-lockfile` 和
  `pnpm run build`，依赖 Node.js 和 pnpm，并会在 Cargo target 目录下生成 web 依赖和
  构建输出。工具不可用时，说明跳过了该检查，不要通过改变工具来源或提交生成产物来
  掩盖环境差异。
- API 契约、嵌入 web 资源或构建集成变化时，在 Node.js 和 pnpm 可用的环境中同步运行
  `ostool-server/webui` 的局部检查。
