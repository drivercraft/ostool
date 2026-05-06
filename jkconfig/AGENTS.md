# AGENTS.md - jkconfig crate

## Scope

Applies to `jkconfig/`, the Ratatui JSON Schema configuration editor.

## Local Rules

- Keep schema parsing, resolver behavior, and UI editing semantics consistent
  across TOML/JSON inputs.
- Prefer structured `serde_json`, `schemars`, and TOML handling over manual text
  edits for configuration data.
- The optional `web` feature is part of the crate surface. Check feature-gated
  code paths when touching shared data or route handling.
- Update `jkconfig/README.md` when public usage, supported schema behavior, or
  examples change.

## Validation

- Prefer `cargo test -p jkconfig` for crate changes.
- Add focused tests for schema edge cases, resolver behavior, or TUI state
  changes when the modified behavior is not already covered.
