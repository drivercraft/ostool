#!/usr/bin/env bash
set -Eeuo pipefail
# Uses the installed, separately built migration executable; no server startup or hardware actions.
if [[ -n "${OSTOOL_MIGRATE_BOARDS_BIN:-}" ]]; then
    exec "$OSTOOL_MIGRATE_BOARDS_BIN" "$@"
fi
if command -v ostool-server-migrate-boards >/dev/null 2>&1; then
    exec ostool-server-migrate-boards "$@"
fi
SCRIPT_DIR=$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
REPO_DIR=$(cd -- "$SCRIPT_DIR/../.." && pwd)
for candidate in "$REPO_DIR/target/release/ostool-server-migrate-boards" "$REPO_DIR/target/debug/ostool-server-migrate-boards"; do
    if [[ -x "$candidate" ]]; then exec "$candidate" "$@"; fi
done
printf '%s\n' 'Migration executable not found. Install ostool-server-migrate-boards or run: cargo build -p ostool-server --bin ostool-server-migrate-boards' >&2
exit 1
