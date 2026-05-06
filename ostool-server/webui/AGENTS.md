# AGENTS.md - ostool-server webui

## Scope

Applies to the Vue/Vite front end in `ostool-server/webui/`.

## Local Rules

- Use pnpm as declared by `packageManager` and `pnpm-lock.yaml`. Do not switch to
  npm, yarn, or another package manager.
- If Node.js or pnpm are unavailable, report which web UI check could not run
  instead of switching package managers or committing generated dependency
  output.
- Keep API client types in `src/api/` and `src/types/` aligned with
  `ostool-server/src/api/models.rs`.
- Do not commit `dist`, `node_modules`, coverage output, or other generated web
  dependency artifacts.
- Maintain the existing operational UI style: dense board/session/server status
  views, clear error states, and controls that map directly to server actions.

## Validation

- Prefer `pnpm --dir ostool-server/webui test` for UI logic changes.
- Run `pnpm --dir ostool-server/webui build` when changing routes, type usage,
  build configuration, or embedded assets.
