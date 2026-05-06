# AGENTS.md - uboot-shell crate

## Scope

Applies to `uboot-shell/`, the async U-Boot shell and YMODEM library.

## Local Rules

- Treat prompt detection, interrupt handling, timeouts, byte streams, and YMODEM
  transfer behavior as protocol-sensitive.
- Keep the crate runtime-neutral around `futures::io` unless a broader design
  change is requested.
- Do not claim hardware U-Boot behavior was verified unless a real target or
  explicit serial fixture was actually exercised.
- Keep logging useful for protocol diagnosis without flooding normal output.

## Validation

- Prefer `cargo test -p uboot-shell` for crate changes.
- Add focused byte-stream or protocol tests when changing timeout, prompt, CRC,
  or YMODEM behavior.
