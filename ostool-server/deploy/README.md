# PostgreSQL, runtime configuration and public deployment

Build once with `cargo build --release -p ostool-server`. The React bundle is
embedded by the build script; no domain, database URL or password belongs in
`VITE_*` variables. The same binary can run on different domains.

The binary embeds separate website and console entries. Website login is
`/login`; administrator login is `/admin/login`. Both use the same PostgreSQL
accounts with separate browser sessions, so one login/logout does not affect the
other. The proxy must forward `/assets/*` as well as `/admin/*` and website routes;
the Caddy example proxies all of them. Custom prebuilt frontend directories must
contain `index.html`, `admin/index.html` and `assets/`.

Both editions require PostgreSQL and share the same data schema. The default
local edition has no authentication or user/rental management. For public deployment, explicitly set
`OSTOOL_EDITION=public`; the server enforces authentication and requires
PostgreSQL. `OSTOOL_AUTH_ENABLED=false` is rejected in public edition. When
upgrading an earlier authenticated deployment, add this edition variable first.

Copy [the public environment example](public.env.example) to a private file, supply the
database URL, hostname and bootstrap administrator, then run:

```bash
chmod 600 /path/to/ostool.env
./target/release/ostool-server --config /path/to/config.toml --env-file /path/to/ostool.env
```

Without `--env-file`, the server reads `.env` from its working directory if it
exists. Process variables override `.env`, and deployment variables override
TOML. Invalid booleans, ports, pool sizes and public URLs fail startup. The
database URL and bootstrap password are never written back to TOML or returned
to the browser. Bootstrap creates a new admin only; it does not promote an
existing registered account or reset its password. Remove the bootstrap password
after first initialization.

For systemd, the existing service may be configured with a drop-in like this
(the example is documentation; nothing installs or modifies a unit):

```ini
[Service]
EnvironmentFile=/path/to/ostool.env
```

Use plain `KEY=value` assignments as in the example, without shell `export` or
shell substitution. Keep deployment listener, data directories and hardware paths
in TOML/`.env`. Network, upload and TFTP settings are initialized from TOML on a
fresh database and
subsequently managed through the UI/database. `OSTOOL_DATABASE_SCHEMA` optionally
selects a schema (1–63 ASCII letters, digits or underscores); migrations create
it when enabled. Use the same URL/schema when switching edition to retain data.
See [systemd EnvironmentFile](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html#EnvironmentFile=).

Put [Caddy](Caddyfile.example) on ports 80/443 and bind the Rust listener to
`127.0.0.1:2999`. Configure DNS and `OSTOOL_PUBLIC_BASE_URL` for the HTTPS origin.
Caddy handles TLS, WebSocket upgrades and SSE streaming without separate
front-end servers; see the [reverse proxy documentation](https://caddyserver.com/docs/caddyfile/directives/reverse_proxy).
Keep PostgreSQL, TFTP, UDP discovery, boot downloads and the network test listener
on private interfaces. Hardware uses the LAN endpoint while users/CLI use the
public HTTPS origin. The Caddy example excludes hardware endpoints from public
ingress; it does not make hardware protocols into authenticated public APIs.

Registration is email/password account creation, not email verification or an
email delivery workflow. Passwords use Argon2; duplicates return HTTP 409. New
accounts receive the member role and cannot enter the administrator console.
Admins manage users and roles through `/admin/users` and `/admin/users/roles`.
Roles currently grant either board access or full administration, without
per-board or individual-operation permission rules. Disabling/deleting a user
and changing its role/password revoke credentials. Built-in roles cannot be
deleted or have their permission level changed. In-use custom roles must be
unassigned before changing permission level or deleting them.

## Persistence status

Both editions use PostgreSQL for durable business data. Bootstrap deployment
configuration and hardware assets remain external; live hardware state and
lease artifacts have their own short lifecycle. Back up the database and
deployment configuration. Runtime business operations never read old files.

| Data | Current authority | Restart behavior |
| --- | --- | --- |
| Users, roles, OAuth sessions, device codes, PAT hashes (public only) | PostgreSQL | Persisted |
| Scheduled board rentals (public only) | PostgreSQL | Persisted; eligibility recalculated at startup |
| Board inventory | PostgreSQL in both editions | Database only; manual offline import available for existing TOMLs |
| DTB payloads and metadata | PostgreSQL (`dtb_records.content` is BYTEA) | Persisted; source files are not used |
| OTA images, jobs and device observations | PostgreSQL (`ota_images`, `server_records`) | Persisted; old OTA files are not imported |
| Editable TFTP/network/upload settings | PostgreSQL (`server_records`) | Initial defaults from TOML on a fresh DB; UI changes survive restart |
| Bootstrap listener, database URL/schema, edition, secrets, hardware paths | TOML + environment | Applied on startup; never copied into database settings |
| Active leases, serial connections, loader discovery, power action state | Memory | No live hardware sessions restored on restart |
| Session artifacts | Files scoped to leases | Cleaned up with lease lifecycle |

The server has no board/DTB directory configuration, startup file import,
quarantine workflow or file storage fallback. Active sessions are not restored as
hardware connections after restart; this does not provide persistent
historical-lease reporting.

## Existing board migration

After deploying the new binaries, run the separate migration executable manually.
The install/update scripts install it but never execute it. Source checkout users
can build both binaries with `cargo build --release -p ostool-server` and use
`scripts/migrate-boards.sh` as a launcher.

Back up PostgreSQL and the existing board/DTB directories, then stop the old
service before the actual import. Set the **target** `OSTOOL_DATABASE_URL` and,
if needed, `OSTOOL_DATABASE_SCHEMA` in the deployment environment file. The tool
reads only database settings from `--config` / `--env-file` and process variables;
it does not initialize authentication, network, power, serial or hardware services.
Process variables override the environment file.

For a target schema already initialized by the new version, first check the
migration without changing schema or business records:

```bash
ostool-server/scripts/migrate-boards.sh \
  --env-file /path/to/ostool.env \
  --board-dir /path/to/existing/boards \
  --dtb-dir /path/to/existing/dtbs \
  --dry-run
```

Run the import by removing `--dry-run`:

```bash
ostool-server/scripts/migrate-boards.sh \
  --env-file /path/to/ostool.env \
  --board-dir /path/to/existing/boards \
  --dtb-dir /path/to/existing/dtbs
```

Without a checkout, invoke the installed `ostool-server-migrate-boards` with the
same arguments. `--config /path/to/config.toml` is optional for target database
settings. Normal import applies SQLx schema migrations first, so an empty target
database does not require starting the hardware service. `--dry-run` requires an
already initialized schema and disables schema migrations. If no board references
a DTB, omit `--dtb-dir`.

The tool reads direct `<board-id>.toml` files in sorted order, validates the
current board configuration and imports only DTBs referenced by those boards.
It does not convert obsolete board formats or import OTA/settings/account files.
All inputs and ID/MAC/DTB conflicts are checked before insertion. New boards,
referenced DTBs and the import audit event commit in one transaction; any failure
rolls them all back. Identical existing records are skipped, making repeated
runs safe. Existing IDs or DTB names with different content, duplicate MACs,
invalid boards and missing referenced DTBs fail the whole import without
silently overwriting or dropping inventory. Source files are never moved,
rewritten or deleted.

After success, start/restart the new server to load the imported inventory.
Retain the source backup as needed; subsequent file edits have no effect. All
future board/DTB operations use the management UI/API and PostgreSQL.

## Verification

```bash
# Use a disposable database; tests create isolated schemas.
export OSTOOL_TEST_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:5432/ostool_tests
export RUST_TEST_THREADS=4
cargo test -p ostool-server
pnpm --dir ostool-server/webui test
pnpm --dir ostool-server/webui build
```

PostgreSQL integration tests use a disposable database and an isolated schema:

```bash
cargo test -p ostool-server --test auth_postgres --test account_postgres \
  --test unified_persistence --test audit_postgres --test board_migration -- --ignored
```

For the full browser account-management test, start a disposable public environment
using `OSTOOL_EDITION=public scripts/dev-env.sh start`, build the current server, then run:

```bash
cargo build -p ostool-server
OSTOOL_E2E_AUTH_BASE_URL=http://127.0.0.1:2999 \
OSTOOL_E2E_ADMIN_EMAIL=admin@example.com \
OSTOOL_E2E_ADMIN_PASSWORD=admin-password-123 \
OSTOOL_E2E_LOCAL_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:54328/ostool_browser_tests \
  pnpm --dir ostool-server/webui test:e2e
```

The local browser database must be a separate disposable database created for
tests. The fixture uses a fresh schema per run, so test records and existing
boards never collide with a previous run or development inventory. CI provides
its PostgreSQL service automatically. Public account tests create users/roles
in the separately supplied public test environment. Without
`OSTOOL_E2E_AUTH_BASE_URL`, those public-account tests are skipped.

## Scheduled board rentals

`/admin/rentals` assigns an enabled user a specific enabled board for a UTC
interval `[starts_at, ends_at)`. Intervals for the same board cannot overlap;
adjacent intervals are allowed. The server serializes admission and mutations,
and uses a per-board PostgreSQL advisory transaction lock for overlap checks.

Members see only currently allocated board types and can obtain a session only
on an allocated board. An existing short session still needs its usual heartbeat.
The session deadline is capped by the rental end, including serial-generated
heartbeats. At expiry/cancellation, the existing release coordinator closes
serial streams, powers off and cleans artifacts. Actual physical release may
finish later than the scheduled cutoff; the next allocation never bypasses a
board's releasing/error state. Administrators may acquire idle boards for
maintenance without a rental.

New rentals do not power on hardware. Editing a rental preserves its board and
user; cancel and create another to change those fields. Cancelled/expired
records remain visible. Renaming/deleting a board with unfinished rentals is
blocked until those rentals are cancelled. Local edition exposes none of these
APIs and does not change existing `ostool` session behavior.
