# AGENTS.md - ostool-server webui

## Scope

Applies to the Vue/Vite front end in `ostool-server/webui/`.

## Local Rules

- Use pnpm as declared by `packageManager` and `pnpm-lock.yaml`. Do not switch to
  npm, yarn, or another package manager.
- Do not install Node.js or pnpm globally on the host. Use the project's
  container/CI environment, or an already available local toolchain if the user
  has provided one.
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
