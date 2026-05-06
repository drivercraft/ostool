# AGENTS.md - jkconfig crate

## 适用范围

适用于 `jkconfig/`，即 Ratatui JSON Schema 配置编辑器。

## 局部规则

- 保持 schema 解析、resolver 行为和 UI 编辑语义在 TOML/JSON 输入之间一致。
- 配置数据优先使用 `serde_json`、`schemars` 和 TOML 的结构化处理，不要手写文本替换。
- 可选 `web` feature 属于 crate 对外表面。修改共享数据或路由处理时，检查 feature-gated
  路径。
- 公开用法、支持的 schema 行为或示例变化时，同步更新 `jkconfig/README.md`。

## 验证

- crate 改动优先运行 `cargo test -p jkconfig`。
- 修改行为尚未被覆盖时，为 schema 边界、resolver 行为或 TUI 状态变化添加聚焦测试。
