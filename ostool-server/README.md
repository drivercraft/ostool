# ostool-server

`ostool-server` is the board management server for `ostool`.

It provides:

- board allocation and lease management
- remote serial terminal access
- TFTP session file handling
- HTTP network throughput tests on a dedicated port
- a systemd-friendly deployment model on Linux

## Source layout

```text
ostool-server/
├── src/
│   ├── api/                  # HTTP API, request/response contracts
│   ├── persistence/
│   │   ├── postgres.rs       # PostgreSQL pool, migrations and account storage
│   │   ├── boards.rs         # Development-board repository
│   │   ├── dtbs.rs           # DTB metadata and binary-content repository
│   ├── static_assets.rs      # Embedded website/admin document and asset delivery
│   └── ...                   # Hardware services and application state
├── migrations/               # SQLx schema migrations, relative to Cargo.toml
├── tools/migrate-boards.rs    # Standalone offline board importer
├── scripts/migrate-boards.sh  # Manual migration launcher
├── tests/                    # Rust integration tests with disposable PostgreSQL
└── webui/                    # React/Vite website and management console
```

Both local and public editions use only PostgreSQL for durable business data.
`src/persistence/` contains the database repositories; `migrations/` contains
SQLx schema history and stays relative to `Cargo.toml`. The service never reads
old board/DTB/OTA files and has no file backend or old module aliases. Existing
board inventory can be imported manually with the [offline migration tool](#existing-board-migration).

## Serial transport

On Unix, a dedicated thread drains physical serial input independently of the
HTTP/WebSocket executor. A 256 KiB byte buffer transfers ownership to the async
session; its short mutex protects only memory copies, never serial or network I/O.
This also covers executor stalls caused by synchronous management operations.
The remaining serial/WebSocket directions use independently polled async workers
and bounded channels. QEMU serial streams continue using their async transport.

Closing or cancelling a session signals and joins the physical reader before the
port and board lease can be reused. The reader never waits for buffer capacity;
its only blocking wait is a serial read with a 20 ms timeout. Overflow is reported
after the already buffered bytes, instead of silently losing data.

Each data queue holds at most 64 chunks of 4 KiB (256 KiB). Binary and decoded `tx`
commands must fit within 256 KiB; larger commands must be split by the client. A full
queue ends the session with an error instead of silently dropping bytes. The error
is sent when the WebSocket remains writable; otherwise the connection closes.
Normal serial EOF drains queued output first. Session cancellation discards pending
commands and stops forwarding output. Serial writes do not call blocking `tcdrain`;
close waits up to one second for the driver output queue, then clears buffers even
if the transmitter remains stuck. This close deadline does not extend test timeouts.

`run_serial_ws()` requests session release on every terminal path, including
failure to open the serial device. The server sends an `error` control message
with the open failure before closing the WebSocket when the connection is writable.
The lease then follows the normal power-off and file cleanup flow; client
heartbeats cannot keep a failed serial session allocated.

## Install

Before installing `ostool-server`, make sure `Node.js` and `pnpm` are available in your environment.
The crate build process compiles the bundled web UI, so `cargo install` will fail if either tool is missing.

You can download and install Node.js from:

```text
https://nodejs.org/en/download
```

After Node.js is installed, install `pnpm` with:

```bash
npm install -g pnpm
```

### Install directly with curl

The install script can be executed directly from GitHub:

```bash
curl -fsSL https://raw.githubusercontent.com/drivercraft/ostool/main/ostool-server/scripts/install.sh | bash
```

The script will:

- check that Node.js 18+ and pnpm are available for the embedded web UI build
- install `ostool-server` with `cargo install`
- install `ostool-server` and `ostool-server-migrate-boards` to `/usr/local/bin`
- stop an existing `ostool-server` systemd service if present
- recreate `/etc/ostool-server`
- create configuration, runtime data and TFTP/session artifact directories
- install `/etc/systemd/system/ostool-server.service`
- start the service if you confirm it

If the script is executed remotely and the local `ostool-server.service` template is unavailable, it will automatically download the matching service template from:

```text
https://raw.githubusercontent.com/drivercraft/ostool/main/ostool-server/scripts/ostool-server.service
```

### Install from local source

If you already have the repository locally:

```bash
bash ostool-server/scripts/install.sh --local ./ostool-server
```

## Upgrade

To upgrade an existing `ostool-server` installation while preserving the current config and data:

```bash
bash ostool-server/scripts/update.sh
```

You can also run the upgrade script directly from GitHub:

```bash
curl -fsSL https://raw.githubusercontent.com/drivercraft/ostool/main/ostool-server/scripts/update.sh | bash
```

To upgrade from a local checkout instead of crates.io:

```bash
bash ostool-server/scripts/update.sh --local ./ostool-server
```

## Configuration

The default config path is:

```text
/etc/ostool-server/config.toml
```

If the config file does not exist, `ostool-server` will create it automatically on first start and write the generated defaults back to disk.

The default listen address is:

```text
0.0.0.0:2999
```

### Local and public editions

Both editions require PostgreSQL and use the same database schema. The default
`OSTOOL_EDITION=local` has no authentication, account or rental management;
`ostool` continues to connect with `auth_mode=disabled`. Public deployment must
explicitly select `OSTOOL_EDITION=public` and configure PostgreSQL, Redis and an
initial administrator. Public edition always enforces authentication; database
connectivity alone does not enable account features.

The frontend reads the edition at runtime, so the same binary serves either
edition. Configuration precedence is process environment, then the selected
`.env` file, then TOML. Without `--env-file`, the server reads `.env` from its
working directory. See [local defaults](.env.example), the
[public environment example](deploy/public.env.example) and
[public deployment](#public-deployment).

Board inventory, DTB content, OTA images/jobs/observations and editable
network/TFTP/upload settings are stored in PostgreSQL. Deployment addresses,
credentials and hardware paths stay in TOML/environment variables; live hardware
connections and discovery stay in memory, and session files expire with their
leases. Restarting does not restore hardware connections. Existing board files
are imported only by the [independent migration tool](#existing-board-migration).

The network throughput test listener is enabled by default on `0.0.0.0:3000`.
It has a separate router from the management listener, so board management,
session file uploads, and `/admin/` stay on port 2999. The test listener has no
authentication and should only be reachable from a trusted LAN. Its address,
concurrency limit, and maximum test duration can be changed in the server TOML
configuration; see [the network test API](../docs/api.md#网络吞吐测试-api).

HTTP Boot is enabled by default. Uploaded UEFI HTTP Boot artifacts reuse the
existing session file storage and lifecycle, so files are scoped to the active
board session and are cleaned up with that session.

Every active board session also exposes uploaded files through
`GET /share/sessions/{session_id}/{relative_path}`. This endpoint supports full
and single-range downloads for every boot mode and remains available when TFTP
is disabled. Upload responses include a board-reachable `http_url`; both the
file and URL expire when the session is released or times out.

For boards using the UEFI HTTP Boot loader, configure the board boot profile
with `kind = "httpboot"`, `network_identity.mac_address`, and, when needed,
`boot_arch`. The server binds each UDP/HTTP loader registration to the board by
its persisted permanent MAC address. Boot manifests and status reports stay in
the active session; serial is used only for target-system interaction.

## Network throughput tests

The dedicated HTTP listener measures raw upload and download streams without
installing a client. It does not implement the iperf2 protocol. A test ID holds
separate upload and download results, and both directions may run concurrently
under the same ID. Upload bytes are counted and discarded without being stored;
download data is generated while the connection is writable. Neither direction
has a fixed byte-count limit. The default limit is 64 test IDs with active
transfers and one hour per transfer; creating an ID does not use a transfer
slot. Pending IDs expire after 10 minutes; finished results remain
queryable for one hour, up to 4096 records.

Existing config files can omit the `[network_test]` section and use these
defaults. To override them, add the section to `/etc/ostool-server/config.toml`
and restart the service:

```toml
[network_test]
enabled = true
listen_addr = "0.0.0.0:3000"
max_active_tests = 64
max_duration_secs = 3600
```

Create a test, then use its returned `test_id` in these commands. The examples
transfer 1 GiB per direction and discard the downloaded data locally:

```bash
base=http://10.3.10.194:3000
curl -fsS -X POST "$base/v1/tests"
test_id='UUID_FROM_POST_RESPONSE'
dd if=/dev/zero bs=1M count=1024 status=none | curl -fsS -T - "$base/v1/tests/$test_id/upload"
curl -fsS "$base/v1/tests/$test_id/download?bytes=1073741824" -o /dev/null
curl -fsS "$base/v1/tests/$test_id"
```

For a simultaneous two-way test, start upload and download on the same ID and
wait for both requests before reading the final result. Create a fresh test ID
and replace the placeholder before running these commands:

```bash
curl -fsS -X POST "$base/v1/tests"
test_id='UUID_FROM_NEW_POST_RESPONSE'
dd if=/dev/zero bs=1M count=1024 status=none | curl -fsS -T - "$base/v1/tests/$test_id/upload" &
upload_pid=$!
curl -fsS "$base/v1/tests/$test_id/download?duration_secs=30" -o /dev/null &
download_pid=$!
wait "$upload_pid"
wait "$download_pid"
curl -fsS "$base/v1/tests/$test_id"
```

An upload or download can be started only once for each test ID. A canceled
connection, network error, or time limit leaves a queryable terminal result.
An upload that reaches the time limit returns HTTP 408 when the connection is
still open. A byte-count download interrupted by the time limit ends its body;
query the test ID for `timed_out` and compare bytes received by the client.
Use `GET /healthz` on port 3000 to check the test listener. These endpoints are
independent of board session file uploads, which remain on port 2999.

## Useful Commands

```bash
systemctl status ostool-server
systemctl restart ostool-server
journalctl -u ostool-server -f
vi /etc/ostool-server/config.toml
```

## Local development environment

Run from the repository root with Rust, Node.js, pnpm and Docker available:

```bash
ostool-server/scripts/dev.sh start
ostool-server/scripts/dev.sh status
ostool-server/scripts/dev.sh build
ostool-server/scripts/dev.sh restart
ostool-server/scripts/dev.sh stop
```

`start` starts the private PostgreSQL container and server, building when needed;
starting an already running edition keeps that process. `build` only builds the
website, console and Rust executable. Use `restart` after source or listener
changes to rebuild embedded assets and replace the process. `stop` stops the
selected edition's server and backing containers. `clean` additionally deletes
**all data belonging to that development edition**; use it only for disposable
data. The launcher does not change systemd or `/etc/ostool-server`.

The launcher reads `ostool-server/.env` regardless of the working directory;
copy [.env.example](.env.example) as needed. Process variables take precedence.
Only `OSTOOL_DEV_*`, `OSTOOL_EDITION`, `OSTOOL_ADMIN_EMAIL` and
`OSTOOL_ADMIN_PASSWORD` are imported; deployment database URLs, listener and
security settings do not redirect the launcher's private services.
`OSTOOL_DEV_ENV_FILE` selects another file. Node's dotenv parser treats values
as data without shell execution or expansion.

| Setting | Default |
| --- | --- |
| `OSTOOL_EDITION` | `local` |
| `OSTOOL_DEV_HOST` | `0.0.0.0` in either development edition |
| `OSTOOL_DEV_SERVER_PORT` | `2999` |
| `OSTOOL_DEV_DIR` | `.ostool-server-dev/<edition>` |
| `OSTOOL_DEV_PG_PORT` | `54328` locally; `54329` for public testing |
| `OSTOOL_DEV_REDIS_PORT` | `56380`, public testing only |
| `OSTOOL_DEV_DEMO` | `true` |

Open `http://127.0.0.1:2999/` or `/admin/overview`; local development also accepts
`http://<server-LAN-IP>:2999/`. Set `OSTOOL_DEV_HOST=127.0.0.1` for loopback-only
access. `OSTOOL_LISTEN_ADDR` controls direct Rust startup; the launcher generates
its listener from `OSTOOL_DEV_HOST` and `OSTOOL_DEV_SERVER_PORT`.
The local anonymous listener belongs on a trusted network.

For the authenticated development edition:

```bash
OSTOOL_EDITION=public ostool-server/scripts/dev.sh start
```

Its bootstrap administrator defaults to `admin@example.com` /
`admin-password-123`; override `OSTOOL_ADMIN_EMAIL` and `OSTOOL_ADMIN_PASSWORD`
before initialization. PostgreSQL data is separate per development edition;
both defaults use HTTP port 2999, so stop the old edition before switching or
assign distinct `OSTOOL_DEV_SERVER_PORT` values. Occupied listeners are rejected
before backing services start. The launcher always passes `--dev`: local and public feature editions both support
loopback/private LAN debugging, including HTTP. Public authentication, CAPTCHA,
roles and rentals stay enabled. Origins must match the request host; an unrelated
website cannot use the development exception. The flag cannot be enabled through
TOML or environment variables and is absent from installed systemd services.
Development mode rejects public-IP origins/listeners; it is not a deployment tool.

For optional LAN HTTPS debugging on Linux Docker:

```dotenv
OSTOOL_EDITION=public
OSTOOL_DEV_HOST=0.0.0.0
OSTOOL_DEV_HTTPS_HOST=192.168.20.10
OSTOOL_DEV_HTTPS_PORT=3443
OSTOOL_DEV_DEMO=true
```

Replace the HTTPS host with this machine's actual LAN IP, then run `dev.sh restart`.
The launcher runs the official `caddy:2-alpine` image using the host network, binds
HTTPS only to that IP, automatically keeps the Rust backend on loopback and blocks
hardware routes. Local edition ignores the public HTTPS options and keeps its
normal HTTP listener, so switching edition does not require removing them.
It exports its development root CA to `.ostool-server-dev/public/caddy/root.crt`;
import and trust that certificate on each accessing device. It never modifies the
host trust store automatically. Keep the Caddy data directory to retain the CA
across restarts. `stop` shuts down Caddy, Rust, PostgreSQL and Redis; `clean` also
removes their development data and CA. Without `OSTOOL_DEV_HTTPS_HOST`, either
edition supports HTTP debugging on `OSTOOL_DEV_HOST`.


Demo data includes 48 boards, 24 DTBs, 12 live sessions and 18 discovered devices.
Public testing also adds 36 users, six roles, 72 rentals and 48 applications.
Records use `demo` / `[演示]` names; existing edits are preserved. Public demo
accounts `demo01@example.test` (administrator) and `demo02@example.test` (member)
both use `demo-user-password-123`. These credentials are for development only.

Demo mode disables actual discovery, QEMU, network testing, TFTP startup and
startup power-off; sample power commands are no-ops and sample DTBs are display
fixtures. Runtime fixtures are recreated at startup; released sessions stay
released until restart. `OSTOOL_DEV_DEMO=false` with `restart` disables fixture
creation and restores normal hardware startup, while retaining persisted data.
Normal direct/systemd startup does not enable demo mode. Do not point development
or test services at production data.

## Management console

The Rust API is in `src/api/`, persistence in `src/persistence/`, and SQLx database
migrations in `migrations/`. `webui/` builds two independent React/Vite entries:

| Entry | Private source | Purpose |
| --- | --- | --- |
| `index.html` | `src/apps/web/` | Website, login and member workspace |
| `admin/index.html` | `src/apps/admin/` | Console and administrator login |

Shared controls are in `src/components/`, API clients/SSE in `src/api/`, contracts
in `src/types/api.ts`, and styles under `src/styles/` with `global.css` as the
entry. Unit tests live beside source; Playwright tests and private server fixtures
live in `tests/e2e/` and `tests/fixtures/`. Neither app imports the other's private
pages. Website and console use independent browser sessions and theme settings,
sharing PostgreSQL accounts and role checks. `/login` opens `/account/*`, while
`/admin/login` rejects non-administrators. Logging out of one does not log out of
the other. Local edition opens the console without login and hides account and
rental features.

The console groups devices/DTBs/short sessions, rentals, users/roles, audit logs
and bottom system settings. Toolbars, independently scrolling content and footer
status/actions fill the viewport. Lists support table/card layouts with shared
filters, actions and pagination; dashboard task tables have a fixed layout.
All updates use SSE, preserving drafts, focus and scroll position. Configuration
conflicts require explicit resolution. See the
[management interface and event protocol](../docs/admin-ui.md).

The dashboard provides runtime health (`/admin/overview`), background tasks
(`/admin/jobs`) and operational metrics (`/admin/operations`). Resource and service
snapshots update every five seconds through SSE; charts retain at most five
minutes from this process. Unavailable host metrics stay unavailable. Local
edition hides user/rental metrics. The dashboard footer includes connection
health; the device toolbar opens discovery information.

`/admin/audits` provides PostgreSQL-backed pagination, filters, details and CSV
export for identity, permissions, settings, board/session/file/power/OTA and
user/role/rental operations. Mutations record an intent before execution; audit
storage failure returns 503 without executing the action. If result recording
fails, the intent remains pending and the response includes
`X-Audit-Status: pending`. Credentials, raw request bodies, file contents and power
commands are excluded. `OSTOOL_AUDIT_RETENTION_DAYS=0` retains all records;
`1..3650` enables hourly cleanup. Pending intents are retained. API/filter details
are in [the API reference](../docs/api.md).

System settings manage branding, contacts, filing information, maintenance
notices, versioned user agreements and privacy policies. Public settings update
via SSE. Registration requires explicit consent to current legal versions;
publication preserves immutable history and per-user acceptance records. Database,
SMTP passwords, Redis connections and encryption credentials remain in deployment environment files,
not browser responses or editable site settings. Logo/favicon settings accept
same-origin static paths; Caddy can serve those assets. Legal text is escaped
plain text, not executable HTML.

Members manage their profile, security, personal access tokens, rentals and
applications in `/account/*`. The public device directory exposes only ID, type,
tags and disabled state. Applications do not reserve time or grant access until
approved. Approval and an administrator-created rental grant the same exclusive
UTC interval `[starts_at, ends_at)`; database locking prevents overlap. Member
sessions are bound to that rental and cannot extend its deadline through
heartbeats. Expiry/cancellation uses normal serial shutdown, power-off and file
cleanup; the next user must wait for physical release. Administrators retain
maintenance access. Frontend workflows and calendar behavior are documented in
[the management guide](../docs/admin-ui.md).

## Public deployment

Use `scripts/install.sh` for an installed service and Caddy for certificates and
HTTPS. Do not use `scripts/dev.sh`, `--dev` or `--dev-demo` in public deployment.
Installed services run the normal binary without development exceptions or demo
fixtures; public edition requires HTTPS and a configured canonical origin.

Build with `cargo build --release -p ostool-server`. The build embeds both React
entries; domain names, database URLs and passwords must not be `VITE_*` variables.
The same executable supports different deployment domains. A custom
`OSTOOL_SERVER_WEB_DIST_DIR` must contain `index.html`, `admin/index.html` and
`assets/`. HTML uses `Cache-Control: no-store`; hashed assets are immutable.

Copy [deploy/public.env.example](deploy/public.env.example) to a private file and
replace its database, Redis, public URL and initial administrator values:

```bash
chmod 600 /path/to/ostool.env
./target/release/ostool-server --config /path/to/config.toml --env-file /path/to/ostool.env
```

Use literal `KEY=value` assignments, without shell `export` or command
substitution. Invalid deployment settings fail startup. Initialization creates a
new administrator only; it does not promote an existing registered account or
reset its password. Remove the bootstrap password after initialization.

An existing systemd service can load that same file through a drop-in:

```ini
[Service]
EnvironmentFile=/path/to/ostool.env
```

Use [deploy/Caddyfile.example](deploy/Caddyfile.example) for HTTPS on ports 80/443
and keep Rust on `127.0.0.1:2999`. Set DNS and `OSTOOL_PUBLIC_BASE_URL` to the actual
HTTPS origin. Forward website, `/admin/*`, `/assets/*`, API, OAuth, WebSocket and
SSE paths. Caddy supports connection upgrades and streaming without a separate
frontend service. Same-host Caddy uses `OSTOOL_TRUSTED_PROXY_IPS=127.0.0.1,::1`;
only trusted peers can supply `X-Forwarded-For`. Missing/invalid forwarding chains
from trusted proxies are treated as unknown sources, not private clients.

Keep PostgreSQL, Redis, TFTP, UDP discovery, hardware downloads and the network
test listener private. `OSTOOL_HARDWARE_ALLOWED_NETWORKS` should contain actual
board subnets; the public Caddy example blocks hardware routes. A loopback Rust
listener requires a separate device-network ingress for board HTTP Boot, shared
files and OTA downloads. `OSTOOL_ADMIN_ALLOWED_NETWORKS` can restrict console
sources using comma-separated CIDRs. Do not expose a local anonymous edition as
a public service.

### Editable account and mail policies

Public system settings include registration/password-recovery switches, CAPTCHA
provider and public Turnstile settings, administrator MFA requirements, login
failure thresholds/cooldowns, per-user/client session limits, token lifetimes,
SMTP host/port/TLS/account/sender, and email-code expiry, attempts, resend cooldowns
and per-email/IP rolling-window limits. These settings initialize from deployment
configuration once, then persist in PostgreSQL `server_records` and apply when
saved. Version checks reject concurrent edits, and SSE updates both the console
and public registration/recovery availability. New token/code lifetimes apply to
new issuance; existing absolute expiries are preserved. Session limits apply at
new login; reduced email attempt limits invalidate exhausted challenges.

Connection addresses, trusted networks, SMTP passwords, Turnstile secrets and the
MFA encryption key stay in `.env`; the UI exposes only credential-presence flags.
The server rejects incomplete SMTP/Turnstile/MFA combinations. Local edition has
no account-policy API or authentication/rental UI.

User agreements and privacy policies have separate tabs with a workspace-filling
plain-text editor, split preview, fullscreen mode and an icon to open version
history. Preview matches the public legal page and does not execute HTML. Loading
a historical version only edits the draft; publishing creates a new immutable
version. Unrelated settings updates do not replace a document draft. Site contacts
also include support hours and a separate privacy email, falling back to the
support email when empty.

### Security configuration

Public browser login/registration use one-shot image CAPTCHA by default, bound
to purpose, source and browser identity, with a 120-second lifetime. Every attempt
consumes the challenge. `OSTOOL_CAPTCHA_PROVIDER=turnstile` requires all three
Turnstile settings from the environment example; hostname and action are checked.
There is no CAPTCHA bypass switch. Email verification is optional and defaults
to `OSTOOL_REGISTRATION_EMAIL_VERIFICATION=false`.

New/changed passwords require at least 15 characters and at most 1024 bytes;
Argon2 hashing is concurrency-limited. Login is limited by account and source IP,
with six failed attempts causing a 15-minute cooldown by default. Auth entrypoints
also have a per-IP limit; 429 includes `Retry-After`. Redis stores TTL-bound
counters using atomic operations; PostgreSQL remains authoritative for users,
sessions, challenges, email verification/reset proofs and audit. Redis connection
failure prevents public startup; runtime limiter failure rejects verification
with 503. Local edition does not connect to Redis. Keep Redis private with
credentials/ACL, use TLS (`rediss://`) remotely, configure `noeviction`, and enable
AOF if lockout windows must survive restart. Clearing Redis resets counters.

Browser credentials use separate HttpOnly, SameSite=Strict, host-only cookies;
HTTPS adds Secure and `__Host-`. Access/refresh tokens are absent from JavaScript
storage, login JSON and SSE URLs. Cookie writes require the expected Origin.
Responses include CSP, frame/MIME restrictions and private-response cache
controls; HTML receives a per-response nonce. Access tokens default to 15 minutes,
refresh chains to at most 30 days with a one-day idle limit and ten active chains
per user/client. Refresh rotates credentials; replay revokes the whole chain and
never extends its absolute or sensitive-operation deadline.

Password/role changes and disabled/deleted users revoke sessions, PATs and pending
device approvals. Built-in role permissions are fixed; assigned custom roles must
be unassigned before changing permission level or deletion. PATs last 90 days by
default (configurable 1–365) and grant CLI board access only. Sensitive changes,
audit export, PAT creation and CLI authorization require identity verification
within the last three minutes. Account security pages manage current login devices
and revocation; SSE and serial connections recheck credentials after revocation.

To enable optional TOTP MFA, generate a stable encryption key once:

```bash
openssl rand -base64 32
```

Store it as `OSTOOL_MFA_ENCRYPTION_KEY` in the protected environment file and back
it up with the database. Replacing/losing it prevents decryption of existing MFA
secrets. TOTP secrets use AES-256-GCM; eight single-use recovery codes are stored
only as hashes. TOTP uses 30-second steps and rejects replay. Keep clocks in sync.
`OSTOOL_REQUIRE_ADMIN_MFA=true` restricts unenrolled admins to enrollment; ordinary
members use their own MFA preference. Changes revoke other sessions/PATs. Public
development retains its generated key under `.ostool-server-dev/public/mfa.key`;
production must manage its own secret.

Optional SMTP enables verification of the current email and password recovery.
Configure `OSTOOL_SMTP_HOST`, `OSTOOL_SMTP_FROM` and the port/TLS/credential settings
in the public example. `starttls` normally uses port 587, `tls` port 465;
`loopback_plaintext` is limited to local testing/mail relays. Registration requires
email codes only with `OSTOOL_REGISTRATION_EMAIL_VERIFICATION=true`. Codes expire
after ten minutes or five attempts, are consumed once, and have per-email/IP
resend limits. Password recovery preserves enabled MFA and revokes old credentials.
Email verification, password reset, password changes and legal consent have
separate PostgreSQL records; the [initial migration](migrations/20261022000000_initial_schema.sql)
is the source of truth for fields, indexes and lifecycle constraints.

For public CLI access, configure `board.server` to the HTTPS site and
`board.auth_mode="required"`; `login --server` affects that login only.
Existing OAuth Device Authorization, Bearer/PAT, Board REST and WebSocket protocols
are preserved, without adding CAPTCHA to CLI calls. CLI credentials have
`board:operate` scope and cannot access personal-account or administrator APIs.
Rental eligibility and session ownership are enforced on subsequent operations;
serial sockets require an unexpired access grant in the login family or PAT.
Normal refresh preserves the socket, while revocation or expiry closes it.
The CLI returns 401 unchanged, without automatically replaying mutations.

Device-side loader/TFTP/boot traffic has a separate network trust boundary.
Use VLAN/firewall isolation between tenants, boards and server infrastructure.
Lease release does not erase board storage or restore firmware; provide a verified
reset procedure before reuse. Uploads have single-file limits but no aggregate
per-user/session quota. Real public DNS/Caddy/TLS and physical firmware/TFTP need
separate deployment validation; isolated application tests do not prove those
conditions. Authentication rules and routes are in [the API reference](../docs/api.md).

## Existing board migration

The service only uses PostgreSQL; it has no file backend, startup import or runtime
legacy-data compatibility. After deploying the new binaries, back up PostgreSQL
and the old board/DTB directories, stop the old service, then import manually:

```bash
cargo build --release -p ostool-server
ostool-server/scripts/migrate-boards.sh \
  --env-file /path/to/ostool.env \
  --board-dir /path/to/existing/boards \
  --dtb-dir /path/to/existing/dtbs
```

An installed `ostool-server-migrate-boards` accepts the same arguments. Install
and update scripts install the tool but never run it. It reads only target database
settings from `--config`, `--env-file` and process variables, without starting
authentication, network, serial or power services. Process values take precedence.
Normal import initializes an empty target through SQLx migrations. `--dry-run`
requires an already initialized schema and changes neither schema nor records.
Omit `--dtb-dir` if the boards reference no DTBs.

The tool reads direct `<board-id>.toml` files, validates current board formats and
imports only referenced DTBs. ID/MAC/DTB conflicts and missing files fail the whole
operation. Boards, DTBs and the import audit event commit in one transaction;
identical records are skipped, different existing content is never overwritten.
Source files are not moved, changed or deleted. OTA/settings/accounts and obsolete
board formats are not imported. Start/restart the new service after success;
subsequent board/DTB operations use the UI/API and PostgreSQL.

### Database initialization

`migrations/20261022000000_initial_schema.sql` is the single initial migration for
this unreleased branch. SQLx applies it to an empty database/schema once and checks
its history on subsequent starts. Future changes after release require incremental
migrations. A development database with the earlier migration chain cannot use
this baseline directly. Preserve it and use a new database or
`OSTOOL_DATABASE_SCHEMA` (1–63 ASCII letters, digits or underscores); reset an old
environment only if all of that edition's data is explicitly disposable. Do not
delete migration history or mark an unapplied migration complete to bypass checks.
The service never resets data automatically. Existing board-file import is
independent of old SQLx development history.

## Verification

Use disposable loopback PostgreSQL and Redis instances, never development or
production inventory. Tests create isolated database schemas:

```bash
export OSTOOL_TEST_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:5432/ostool_tests
export OSTOOL_TEST_REDIS_URL=redis://127.0.0.1:6379/0
export RUST_TEST_THREADS=4
cargo test -p ostool-server
cargo test -p ostool-server --test auth_postgres --test account_postgres \
  --test audit_postgres --test security_postgres --test unified_persistence \
  --test board_migration --test cli_rental_contract -- --ignored
cargo test -p ostool-server --test cli_gateway_contract
pnpm --dir ostool-server/webui test
pnpm --dir ostool-server/webui build
cargo build -p ostool-server
OSTOOL_E2E_PUBLIC=true \
OSTOOL_E2E_REDIS_URL=redis://127.0.0.1:6379/0 \
OSTOOL_E2E_LOCAL_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:5432/ostool_tests \
  pnpm --dir ostool-server/webui test:e2e
```

Browser tests start separate local/public servers, create fresh `ostool_e2e_...`
schemas and stop the fixtures automatically; ports 4174, 4175, 4191 and UDP 2998
must be available. Public tests set known CAPTCHA digests only in that isolated
database; the application has no test bypass. Without `OSTOOL_E2E_PUBLIC=true`
or explicitly configured public fixtures, public tests are skipped. An external
fixture must provide matching `OSTOOL_E2E_AUTH_BASE_URL`,
`OSTOOL_E2E_AUTH_DATABASE_URL`, `OSTOOL_E2E_AUTH_DATABASE_SCHEMA`,
`OSTOOL_E2E_ADMIN_EMAIL` and `OSTOOL_E2E_ADMIN_PASSWORD`, with loopback hosts and an
`ostool_e2e_` schema. See [browser test details](../docs/admin-ui.md#开发与验证).
