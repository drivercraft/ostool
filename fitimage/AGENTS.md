# AGENTS.md - fitimage crate

## 适用范围

适用于 `fitimage/`，即 U-Boot FIT 镜像构建库。

## 局部规则

- 生成的 FIT 结构、FDT token、load/entry 地址、hash 字段和压缩元数据都属于兼容性敏感
  内容。
- 保持纯库定位。除非用户明确要求调整 package 范围，否则不要在这里新增 CLI 行为。
- 优先扩展现有 builder 和类型化配置结构，不要重复实现字节布局逻辑。
- README 示例和测试应与公开 API 变化保持一致。

## 验证

- crate 改动优先运行 `cargo test -p fitimage`。
- 格式变化应新增或更新 `fitimage/tests/` 下的测试；可行时，在受控环境中用 U-Boot 工具对照
  行为。
