# AGENTS.md - ostool-server crate

## Scope

Applies to `ostool-server/`, except `ostool-server/webui/`, which has its own
local instructions.

## Local Rules

- Treat REST/WebSocket models in `src/api/`, session lifecycle, board leasing,
  TFTP file handling, and serial access as cross-component contracts. Update
  server tests and the web UI client/types when those contracts change.
- Do not run `scripts/install.sh`, `scripts/update.sh`, modify systemd units, or
  touch `/etc/ostool-server` on the host unless explicitly requested.
- `build.rs` copies the web UI and runs pnpm to build embedded assets. Keep that
  behavior deterministic and avoid committing `webui/dist`, `node_modules`, or
  other generated dependency output.
- Power-management and serial-discovery changes should be conservative: preserve
  stable device identity handling and make failure modes visible to callers.

## Validation

- Prefer `cargo test -p ostool-server` for server-only changes.
- If API contracts, embedded web assets, or build integration changes, also run
  the relevant `ostool-server/webui` checks from its local instructions when
  Node.js and pnpm are available.
