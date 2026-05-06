# AGENTS.md - ostool

## Scope

This file applies to the whole repository. More specific `AGENTS.md` files in
subdirectories override or extend these rules for their own tree.

## Project Map

- `ostool/`: main CLI/library for OS build, menuconfig, QEMU, U-Boot, TFTP, serial,
  board-client, and `cargo-osrun` flows.
- `ostool-server/`: board management server, API, serial sessions, TFTP files,
  power management, and systemd-oriented deployment scripts.
- `ostool-server/webui/`: Vue/Vite/pnpm front end embedded by `ostool-server`.
- `jkconfig/`: Ratatui JSON Schema configuration editor library and optional web
  feature.
- `fitimage/`: library for U-Boot-compatible FIT image construction.
- `uboot-shell/`: async U-Boot shell and YMODEM communication library.
- `.github/workflows/check.yaml`: current CI source of truth for formatting,
  clippy, build, and tests.

## Dependency And Tooling Policy

- Third-party development dependencies and toolchains default to Docker, Dev
  Container, CI, or another project-local isolated environment, not direct
  installation on the macOS host.
- Do not run host-level install commands by default, including `brew install`,
  `curl | sh`, `npm -g`, `rustup`, `pyenv`, or language runtime installers.
  These are allowed only when the user explicitly requests that host install in
  the current conversation.
- When a dependency is needed, first look for project-native Docker, Dev
  Container, docker-compose, CI, Makefile, or script entrypoints and run inside
  that environment.
- If this repository lacks a containerized entrypoint for the needed task,
  prefer proposing or adding a project-local Docker/Dev Container path instead
  of extending the host runtime.
- Small operating-system utilities may be exceptions, but explain why they must
  be installed on the host and wait for user confirmation unless the user
  already requested the install.

## Git And Commits

- Work on a feature branch for repository changes unless the user explicitly
  asks to stay on the current branch.
- Follow the repository's recent commit style: Conventional Commits such as
  `fix(ostool): ...`, `chore(ostool-server): ...`, `docs: ...`, or
  `refactor(jkconfig): ...`.
- Keep unrelated edits out of a commit. Stage only the files that belong to the
  requested change.

## Validation

- For repo-wide Rust changes, mirror CI when practical:
  `cargo fmt --all -- --check`, `cargo clippy --target x86_64-unknown-linux-gnu --all-features`,
  `cargo build --target x86_64-unknown-linux-gnu --all-features`, and
  `cargo test --target x86_64-unknown-linux-gnu -- --nocapture`.
- For focused changes, run the narrowest package or web UI checks that cover the
  touched area, and state exactly what was or was not run.
- The CI installs QEMU, U-Boot tools, libudev, Node.js 24, and pnpm 10.33.0.
  Do not install these on the host just to reproduce CI.

## Documentation

- If a user-facing CLI, server API, config format, install path, or workflow
  changes, update the relevant README or local documentation in the same change.
- The root README exists in Chinese and English. Keep both versions aligned when
  editing shared user-facing content.
- Changelogs are package-local. Update them only for release or explicitly
  requested changelog work.

## Rust Conventions

- Preserve the crate edition and public API style already used in each package.
- Prefer structured parsing/serialization through existing `serde`, `schemars`,
  TOML, and JSON types instead of ad hoc string manipulation for config data.
- Treat serial, TFTP, QEMU, U-Boot, and board power changes as operationally
  sensitive. Keep side effects explicit and covered by tests or documented
  manual verification.
