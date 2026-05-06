# AGENTS.md - ostool-server webui

## 适用范围

适用于 `ostool-server/webui/` 中的 Vue/Vite 前端。

## 局部规则

- 使用 `packageManager` 和 `pnpm-lock.yaml` 声明的 pnpm。不要切换到 npm、yarn 或其他
  包管理器。
- Node.js 或 pnpm 不可用时，说明哪些 web UI 检查无法运行，不要切换包管理器，也不要提交
  生成的依赖输出。
- 保持 `src/api/` 和 `src/types/` 中的 API client 类型与
  `ostool-server/src/api/models.rs` 对齐。
- 不要提交 `dist`、`node_modules`、coverage 输出或其他生成的 web 依赖产物。
- 保持现有的运维型 UI 风格：紧凑的 board/session/server 状态视图、明确的错误状态，以及
  直接映射到服务器动作的控件。

## 验证

- UI 逻辑改动优先运行 `pnpm --dir ostool-server/webui test`。
- 修改路由、类型使用、构建配置或嵌入资源时，运行
  `pnpm --dir ostool-server/webui build`。
