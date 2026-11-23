# AGENTS.md - ostool-server webui

## 适用范围

适用于 `ostool-server/webui/` 中的 React/TypeScript/Vite 前端。

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

- 页面数据统一订阅 `src/api/events.ts`；不要新增轮询或 mutation 后全量 GET。
- 使用官方 shadcn 组件和统一设计 token；推送不得覆盖编辑草稿、焦点或滚动位置。
- 前后台私有代码分别放在 `src/apps/admin/` 和 `src/apps/web/`；共享组件、接口和业务
  定义放在公共目录，两个应用不得相互导入私有页面。全局样式入口为 `src/styles/global.css`。
- 不维护界面组件和浏览器测试；API、事件流、配置转换及业务算法测试与源码同目录，使用 `.test.ts`。

## 验证

- API、事件流或业务算法改动优先运行 `pnpm --dir ostool-server/webui test`。
- 修改路由、类型使用、构建配置或嵌入资源时，运行
  `pnpm --dir ostool-server/webui build`。
