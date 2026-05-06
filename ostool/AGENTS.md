# AGENTS.md - ostool crate

## Scope

Applies to `ostool/`, the main CLI/library and `cargo-osrun` binary.

## Local Rules

- Treat command-line behavior, config schemas, generated defaults, and terminal
  interaction as public user-facing contracts.
- Keep `src/lib.rs` exports, `src/main.rs` CLI wiring, and README examples in
  sync when changing public behavior.
- Config changes should flow through the existing typed config structures and
  schema generation paths. Avoid silent fallback defaults for runtime behavior
  unless the current pattern already requires them.
- Serial, QEMU, U-Boot, TFTP, and remote board flows can affect real devices.
  Do not assume hardware was exercised unless it was actually run.

## Validation

- Prefer focused checks such as `cargo test -p ostool` for crate changes.
- When changing CLI parsing or compile-fail expectations, include the relevant
  tests under `ostool/tests/`, including `ostool/tests/ui/` where applicable.
- If a change relies on QEMU, serial hardware, or an `ostool-server` instance,
  state the exact manual or unavailable verification instead of implying it ran.
