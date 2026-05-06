# AGENTS.md - ostool crate

## 适用范围

适用于 `ostool/`，也就是主 CLI/库和 `cargo-osrun` 二进制。

## 局部规则

- 命令行行为、配置 schema、生成的默认值和终端交互都是面向用户的公开契约。
- 修改公开行为时，同步维护 `src/lib.rs` 导出、`src/main.rs` CLI 接线和 README 示例。
- 配置改动应经过现有类型化配置结构和 schema 生成路径。除非当前模式已经要求，否则不要
  为运行时行为加入静默 fallback 默认值。
- 串口、QEMU、U-Boot、TFTP 和远程开发板流程可能影响真实设备。未实际运行硬件或对应
  fixture 时，不要声称已经验证。

## 验证

- crate 内改动优先运行 `cargo test -p ostool` 等聚焦检查。
- 修改 CLI 解析或 compile-fail 预期时，同步维护 `ostool/tests/` 下的相关测试，必要时
  包括 `ostool/tests/ui/`。
- 如果改动依赖 QEMU、串口硬件或 `ostool-server` 实例，说明具体的手动验证结果或无法
  验证的原因，不要暗示已经运行。
