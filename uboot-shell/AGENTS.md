# AGENTS.md - uboot-shell crate

## 适用范围

适用于 `uboot-shell/`，即异步 U-Boot shell 与 YMODEM 库。

## 局部规则

- prompt 检测、中断处理、超时、字节流和 YMODEM 传输行为都属于协议敏感内容。
- 除非要求更大范围的设计调整，否则围绕 `futures::io` 保持 runtime-neutral。
- 未实际运行真实目标或明确的串口 fixture 时，不要声称已经验证硬件 U-Boot 行为。
- 日志应有助于协议诊断，但不要淹没正常输出。

## 验证

- crate 改动优先运行 `cargo test -p uboot-shell`。
- 修改超时、prompt、CRC 或 YMODEM 行为时，添加聚焦的字节流或协议测试。
