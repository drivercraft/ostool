# Frontend structure

This package builds two React applications with Vite's multiple HTML entries:
the public website at `/` and the management console at `/admin/`. Rust embeds
both outputs and serves their documents and assets. Each application owns its
routes, login screen, browser session and theme preference.

```text
webui/
├── index.html                   # Website HTML entry
├── admin/index.html             # Management HTML entry
├── src/
│   ├── apps/
│   │   ├── admin/
│   │   │   ├── main.tsx         # Management bootstrap
│   │   │   ├── App.tsx          # Management routes and shell
│   │   │   ├── pages/           # Boards, rentals, users, audit, dashboard, login
│   │   │   ├── components/      # Management-only components
│   │   │   ├── board-form.ts    # Board editor model
│   │   │   └── login.css
│   │   └── web/
│   │       ├── main.tsx         # Website bootstrap
│   │       ├── App.tsx          # Website routes, landing pages and login
│   │       ├── session.tsx      # Website session provider
│   │       └── pages/           # Member workspace
│   ├── components/              # Shared controls, layouts and shadcn/ui
│   ├── api/                     # Shared HTTP client and SSE transport
│   ├── domain/                  # Shared business labels and rules
│   ├── types/                   # API contracts
│   ├── styles/                  # Shared theme, typography and scoped styles
│   ├── utils/                   # Shared pure helpers
│   ├── lib/                     # UI utilities
│   ├── auth-session.ts          # Separate website/admin credential namespaces
│   └── edition.tsx              # Runtime local/public edition
└── e2e/                         # Browser integration tests
```

Pages and components private to one application stay in its `apps/` directory.
An application must not import the other application's pages or private
components. Move genuinely shared UI to `components/`, business labels/rules to
`domain/`, and transport/types to `api/` and `types/`. Tests live beside the code
they cover. Shared styles use application classes where their rules differ.

The table/card switch presents the same records with the same typography:
values and names use `1rem` (normally 16px), labels and secondary text use
`0.875rem` (normally 14px), and ordinary content uses regular weight. The shared
card renderer preserves cell classes such as monospace identifiers.

Run from the repository root:

```bash
pnpm --dir ostool-server/webui test
pnpm --dir ostool-server/webui build
cargo build -p ostool-server
OSTOOL_EDITION=public ostool-server/scripts/dev-env.sh restart
```

`pnpm --dir ostool-server/webui dev` serves the website at `/` and management at
`/admin/`; its API proxy expects a development backend on port 4175. See the
[server README](../README.md#local-development-environment) for the one-command
environment and [deployment guide](../deploy/README.md#verification) for browser
test database configuration. Building produces both HTML entries and hashed
assets in one output directory. No database credentials or deployment hostname
are baked into either application.
