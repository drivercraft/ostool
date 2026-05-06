# AGENTS.md - fitimage crate

## Scope

Applies to `fitimage/`, the U-Boot FIT image construction library.

## Local Rules

- Treat generated FIT structure, FDT tokens, load/entry addresses, hash fields,
  and compression metadata as compatibility-sensitive.
- Preserve the library-only surface; do not add CLI behavior here unless the
  user explicitly requests a package scope change.
- Prefer extending existing builders and typed config structs over duplicating
  byte-layout logic.
- Keep README examples and tests aligned with public API changes.

## Validation

- Prefer `cargo test -p fitimage` for crate changes.
- For format changes, add or update tests under `fitimage/tests/` and, when
  practical, compare behavior against U-Boot tooling in an isolated environment.
