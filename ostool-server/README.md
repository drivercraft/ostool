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
board inventory can be imported manually with the [offline migration tool](deploy/README.md#existing-board-migration).

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

The default is `OSTOOL_EDITION=local`: no login, registration, user/role or
rental management. Existing `ostool` clients continue to use `auth_mode=disabled`.
PostgreSQL is required in local edition as well; edition only controls account
and rental features.

For public deployments, explicitly set `OSTOOL_EDITION=public`, supply
`OSTOOL_DATABASE_URL` and initialize an administrator. Public edition additionally requires OSTOOL_REDIS_URL for security counters and always
requires authentication for board APIs and administrator credentials for admin
APIs; disabling auth is not supported. Missing PostgreSQL fails startup.
Use [the public environment example](deploy/public.env.example) with Caddy.
Existing authenticated deployments must add `OSTOOL_EDITION=public` when upgrading.
The database connection alone no longer enables account features.

Public login/registration require image CAPTCHA by default. Email verification is
optional and disabled by default. Optional SMTP, TOTP/recovery codes, device/session
revocation, recent identity verification, rate limits and deployment settings are
documented in [security.md](deploy/security.md). Local edition needs none of these settings.

Public edition supports `/register`, `/login`, OAuth Device Authorization and
PATs compatible with `ostool login` / `ostool login --with-token`. Rental
administration assigns a board to a user for a UTC time interval. Members can
only acquire boards allocated to them at the current time. Administrators retain
maintenance access. Expiry/cancellation terminates member sessions through the
existing power-off and file-cleanup coordinator; the next user must wait until
hardware release completes. Ordinary heartbeat TTL cannot extend that interval.

Board inventory, DTB content and OTA data use PostgreSQL from the first start.
To migrate existing board TOMLs after deploying the new version, run
`scripts/migrate-boards.sh` manually; startup and deployment scripts never import
business files. See [migration instructions](deploy/README.md#existing-board-migration)
and the [persistence matrix](deploy/README.md#persistence-status).

The frontend reads runtime edition information from the Rust server rather than
an independent build flag, so one binary supports both editions and UI settings
cannot disable public authentication. `.env` defaults to the process working
directory; `--env-file` selects an explicit file. Process variables win over file
variables. See [.env.example](.env.example) for local defaults.

The sidebar groups boards/DTBs/short sessions under **设备管理**. Public edition
also has **租赁管理 → 租赁列表** and **用户管理 → 用户列表 / 用户角色**. All list
toolbars place actions on the left and search/filter controls on the right.
**系统设置** stays at the bottom, with grouped configuration tabs.

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

Both editions require PostgreSQL. The development script defaults to local
edition and starts a private PostgreSQL 16 container automatically (Docker is
required). Local edition needs no login and no CLI credentials:

```bash
ostool-server/scripts/dev.sh start
ostool-server/scripts/dev.sh status
ostool-server/scripts/dev.sh stop
```

To test authenticated public edition with a private PostgreSQL 16 container:

```bash
OSTOOL_EDITION=public ostool-server/scripts/dev.sh start
OSTOOL_EDITION=public ostool-server/scripts/dev.sh stop
```

Use `restart` or `clean` with the same edition. Data/log/config directories are
`.ostool-server-dev/local` and `.ostool-server-dev/public`; `OSTOOL_DEV_DIR` can
override that location. The default local edition listens on `0.0.0.0:2999`;
open `http://<server-LAN-IP>:2999/` or `/admin/overview` from another LAN device.
The launcher prints the detected IPv4 LAN URLs. PostgreSQL and Redis ports remain
bound to loopback. Public-edition development keeps its `127.0.0.1:2999` listener.

Use `OSTOOL_DEV_HOST` to select a numeric IPv4/IPv6 address. For loopback-only
local testing:

```bash
OSTOOL_DEV_HOST=127.0.0.1 ostool-server/scripts/dev.sh restart
```

Use `restart` to apply a listener change to an already running process, including
an existing config created by an older launcher. `start` preserves an existing
server. Local edition has no authentication, so use its LAN listener only on a
trusted network. Public HTTPS deployment continues to use the deployment settings
and Caddy; this does not relax public-edition origin or demo restrictions.
The local/public database containers default to `ostool-server-dev-local-postgres`
and `ostool-server-dev-public-postgres`, with ports 54328 and 54329 respectively.
The schema and persistence code are identical; deploy either edition against the
same database URL/schema to keep the same data when changing edition.
Only public edition has website `/login` and backend `/admin/login`, with bootstrap defaults `admin@example.com` /
`admin-password-123`. Override `OSTOOL_ADMIN_EMAIL`, `OSTOOL_ADMIN_PASSWORD`,
`OSTOOL_DEV_PG_PORT`, or `OSTOOL_DEV_SERVER_PORT` before starting as needed.
The script does not read the repository `.env`, modify systemd or touch `/etc`.
Where `setsid` is available, it starts the server in an independent session so
closing a command runner does not terminate it; `stop` still uses the recorded
server PID. Other environments retain the `nohup` fallback.

The development launcher enables demo data by default (`OSTOOL_DEV_DEMO=true`).
Restart an existing environment to populate it:

```bash
ostool-server/scripts/dev.sh restart
# Authenticated demo, including users/roles/rentals:
OSTOOL_EDITION=public ostool-server/scripts/dev.sh restart
```

Choose the command for your edition. Both default to the same port; stop the old
edition before switching, or set different `OSTOOL_DEV_SERVER_PORT` values.

Fresh demo environments contain 48 boards (six types, U-Boot/PXE/HTTP Boot,
multiple tags, long/empty notes, enabled/disabled), 24 sample DTBs, 12 live sample
sessions with downloadable text/log files, and 18 discovered devices (online,
offline, conflicting and unbound). Idle, in-use, releasing and error states, plus successful
and failed power notifications, exercise the console's filters and pagination.
Public edition additionally has 36 users (including disabled accounts), six
custom roles, 72 past/current/future/cancelled rentals, and 48 pending/approved/
rejected/withdrawn rental applications. Existing records are
preserved; fixture IDs/names start with `demo` or `[演示]`. These are actual local
storage/database records consumed by the regular API/SSE, not browser mocks.

Public demo accounts: `demo01@example.test` (administrator),
`demo02@example.test` (member), password `demo-user-password-123` for both.
The bootstrap administrator above is also available. Local edition continues
without login or user management. Demo roles follow existing administrator/member
permissions; their names do not introduce additional permission levels.

`--dev-demo` accepts LAN/private or wildcard listeners in local edition; any
configured origin must be loopback or a private IP. Public edition still requires
both a loopback management listener and a loopback authentication origin. It disables UDP discovery, QEMU, the network-test
listener, TFTP service startup and automatic startup power-off. Sample board power
commands are no-ops; discovered IPs use the documentation-only `192.0.2.0/24`
range. Sample DTBs contain a minimal display model, not real hardware definitions.
Demo sessions receive heartbeats while the process runs; manually released
sessions stay released until restart. Runtime snapshots are recreated on restart.
These fixtures are intended for UI inspection, not hardware/boot verification.

To launch without adding/refreshing fixtures, use
`OSTOOL_DEV_DEMO=false ostool-server/scripts/dev.sh restart` (or the same
prefix with `OSTOOL_EDITION=public`). This restores normal hardware startup behavior
and retains already stored demo records. `clean` deletes **all data for the chosen
development edition**; starting again recreates fresh fixtures. Never point the
development launcher at a production database or data directory. Normal direct
server startup and systemd deployments do not enable demo data; direct isolated
development startup can opt in with `ostool-server --config ... --dev-demo`.

## Management console

See [frontend structure](webui/README.md) for the independent `src/apps/admin`
and `src/apps/web` applications, shared components and build entries.

The dashboard groups runtime health (`/admin/overview`), actual background tasks
(`/admin/jobs`) and board/user/rental operations (`/admin/operations`). The server
samples CPU, memory, process RSS, data-disk capacity, configured-interface traffic
and PostgreSQL health every five seconds and pushes updates over the existing SSE
connection. Unsupported metrics remain unavailable. Charts contain up to five
minutes of samples from this process, not historical monitoring data. Local
edition hides account and rental metrics. Connection status appears in the
dashboard status bar; board discovery opens from the board-list toolbar.

`/admin/audits` provides PostgreSQL-backed, server-paginated audit records with
action/resource/result, actor, IP, source and date filters, details and CSV export.
Records include request ID, actor identity, target, outcome, HTTP status and elapsed
time. Login/registration, permission denials, board/session/file/power/OTA actions,
user/role/rental changes, settings changes and audit viewing/export/cleanup are
recorded. Asynchronous power/release results and OTA state changes are separate
events. Passwords, tokens, raw request/response bodies, file contents and shell
commands are excluded. Data starts when this feature is enabled; existing business
records are not converted into invented audit history.

Public edition restricts audit APIs to administrators. Local edition records
anonymous local operators without enabling authentication. Mutating API requests
write an audit intent before executing; unavailable audit storage returns HTTP
503 without executing the operation. If result recording fails afterward, the
intent remains `pending` (outcome unconfirmed) and the response has
`X-Audit-Status: pending`; the runtime dashboard reports audit write failures.
Device polls and session heartbeats are excluded from HTTP audit noise.

Audit records are retained indefinitely by default. Set
`OSTOOL_AUDIT_RETENTION_DAYS=1..3650` before startup to enable hourly retention;
`0` disables automatic cleanup. The cleanup dialog executes a single cleanup and
does not change that environment setting. Unconfirmed records are retained.
`OSTOOL_TRUSTED_PROXY_IPS` is a comma-separated list of trusted reverse-proxy IPs;
only those peers may supply the `X-Forwarded-For` chain. For Caddy on the same host,
set it to `127.0.0.1,::1`. These deployment settings require a server restart and
are not frontend build variables.

Audit API routes: `GET /api/v1/admin/audits`, `GET /api/v1/admin/audits/{id}`,
`GET /api/v1/admin/audits/export`, `POST /api/v1/admin/audits/view` and
`POST /api/v1/admin/audits/cleanup` (`{"keep_days":180}`). List/export filters are
`q`, `action`, `resource`, `result`, `source`, `actor`, `ip`, RFC3339 `from`/`to`,
`page`, and `page_size` (maximum 100). CSV exports contain at most 10,000 records;
larger selections return an error asking for narrower filters. Management SSE
supports `audit_view=1` with `audit_`-prefixed filters; browsers use this subscription
for list updates. `GET /api/v1/admin/dashboard` also exposes the runtime snapshot.

Rental creation and editing use `/admin/rentals/new` and `/admin/rentals/{id}`.
A two-column form at the top selects user/board and start/end times; the calendar
fills the remaining workspace. Year/month/day views support year/month navigation,
dragging consecutive dates (end exclusive at the next midnight), and dragging
half-hour slots in day view. Double-click a date to open day view; keyboard Enter
selects a focused time slot. Expand the day's available windows to inspect bookings
or select a free window. Browsing does not change the draft; conflicting drags are
rejected and Escape cancels a selection. The calendar follows the browser timezone,
handles bookings crossing midnight and excludes cancelled bookings and the rental
being edited. Live conflicts disable save without overwriting the draft;
PostgreSQL rechecks conflicts on save.
The member application page uses the same calendar with anonymized occupancy.
All data lists have a layout icon in the upper-right toolbar for table/card
views. Selection persists per list in the browser; search, filters, pagination
and existing actions work in both views.

Board inventory, DTB bytes/metadata, OTA image bytes/metadata/jobs/observations,
and editable network/TFTP/upload settings are authoritative in PostgreSQL for
both editions. The running server never reads old business files. Existing boards
and their referenced DTBs have a separate, manual migration tool. Deployment variables
(database connection, edition, listen address, credentials and hardware paths)
remain in TOML/`.env`; secrets are excluded from database settings. Live leases,
connections and discovery remain process state, while session artifacts remain
temporary files cleaned up with the lease. See [persistence and deployment](deploy/README.md).

The public website has its own header, footer and authenticated workspace at
`/account/*`, separate from the administrator layout at `/admin/*`. Website login
opens the personal workspace for members and administrators alike. Its avatar
menu links to profile, security, rentals and applications; administrators also
get an explicit management-console link. Light/dark/system theme preferences
are stored independently for the website and console.

The website and console are independent HTML/JavaScript entries, rather than a
single application that chooses its surface after loading. Website pages use
`index.html`; `/admin/*` uses `admin/index.html`. Each has its own initial document
title and login screen. `/login` always enters the website's `/account` workspace;
backend login uses `/admin/login` and `POST /api/v1/admin/auth/login`, which rejects
non-administrators. Browser access/refresh credentials are stored separately;
logging into or out of one surface does not sign into or out of the other.
The account database, role checks and CLI authentication remain shared. Existing
browser credentials must be refreshed by logging in again after this security upgrade.
Cookies are HttpOnly and separate for each surface; no access/refresh tokens remain
in browser localStorage or SSE URLs.
Local edition continues to open the console without authentication.

`pnpm --dir ostool-server/webui build` builds both entries together. A prebuilt
`OSTOOL_SERVER_WEB_DIST_DIR` must contain both `index.html` and `admin/index.html`,
plus `assets/`. Vite development serves the website at `/` and the console at
`/admin/`, matching Rust routing. Deployment proxies must forward `/assets/*`
as well as website, backend, API, OAuth, WebSocket and SSE routes to the server.

Members can edit their name/email (email changes require their password), change
password, manage CLI personal access tokens, view their own rentals and submit
or withdraw rental applications. The public `/devices` page lists all configured
boards as cards in both editions, including disabled boards with an unavailable
state. Visitors can search without logging in; applying requires a public account.
`GET /api/v1/site/devices` and public SSE `device_catalog` expose only the device
ID, board type, tags and disabled flag. Management details remain private.
Administrators review applications under rental management.
Approval rechecks availability and creates the exclusive rental in the same
PostgreSQL transaction; applying alone reserves no time or hardware. All personal
queries and SSE updates are scoped to the authenticated user, excluding management
configuration and other users' data. Password changes invalidate all existing
OAuth sessions, personal access tokens and approved device codes for that user.
These features remain hidden and unavailable in local edition.

After source changes, use `restart` for the running development edition; `start`
keeps an already running process. Restart rebuilds the embedded website and Rust
executable using Cargo. HTML is served with `Cache-Control: no-store`, while
hashed assets are immutable. Prebuilt `OSTOOL_SERVER_WEB_DIST_DIR` changes are
also tracked by the build script. Deployment schema migrations run automatically
on connection to a fresh database/schema. The unreleased development migrations
have been consolidated into `migrations/20261022000000_initial_schema.sql`.
A database that has recorded the earlier development chain cannot run this new
baseline directly; preserve it and use a fresh database/schema, or explicitly
reset disposable development data. See [database initialization](deploy/site-and-data.md#数据库初始迁移).
Future released schema changes must use new incremental migrations.

Open `/admin/` for the React + shadcn/ui console. Boards, discovery, virtual
hardware, DTBs, leases, TFTP and server settings update through a single SSE
connection; browser list polling is not used. Draft edits remain local until
saved and concurrent configuration changes are reported without overwriting them.
The workspace fills the viewport: toolbars and footer status/actions stay fixed
while the center content scrolls. List footers provide local pagination over the
SSE projection; editor and configuration save/cancel controls stay in the footer.
The global titlebar shares its theme with the logo area and provides live
operational notifications, a light/dark/system theme menu, and an account avatar
menu. Theme selection persists in the browser. Local edition shows workspace
actions without authentication entries; public edition exposes account/logout.

A new board does not have to be saved before powering it on. Configure Custom,
Zhongsheng relay or an existing QEMU device, then click **上电** or **下电**.
Choose a MAC from live discovery or enter it manually, then save the board.
Power completion means the command completed, not confirmed physical feedback.
Leaving the page does not reverse or cancel a submitted power action.

See [the management protocol and development guide](../docs/admin-ui.md).

### Site configuration and legal documents

System settings include site branding, contact/operator/filing details, maintenance notices,
versioned user agreements and privacy policies. The website, footer and authentication pages
receive public settings over SSE. Registration requires explicit consent to current legal
versions; immutable revisions and per-user acceptances are persisted in PostgreSQL.
See [site and identity schema](deploy/site-and-data.md). Public dev startup now also starts
private Redis; local dev startup still needs PostgreSQL only.

Member rental requests use a separate list at `/account/requests` and editor at
`/account/requests/new`. The editor shares the administrator rental layout and calendar.
Clicking a cell replaces a previous range; occupied/elapsed cells are gray and disabled.
Portal content scales to 80% of desktop width, with mobile gutters. Icon actions share
unfilled glyphs, a uniform hit area and hover backgrounds.

The website and CLI share the same scheduled allocation: an approved application
or administrator-created rental grants access during `[starts_at, ends_at)`.
Member sessions bind to that rental; heartbeats cannot extend its deadline.
Cancelling/expiring the rental releases the session, and OAuth/PAT revocation
closes active serial connections. Device-side TFTP and trusted-network loader/file
traffic remain separate from user authentication; restrict those listeners to the
board network. See [API authorization boundaries](../docs/api.md).

A cross-package regression test uses the real ostool HTTP/WebSocket clients and a
PTY serial device against an isolated server (PostgreSQL and Redis required):

```bash
cargo test -p ostool-server --test cli_rental_contract -- --ignored
```

The fixture uses loopback HTTP only; production CLI authenticated endpoints still
require HTTPS. This test does not replace physical firmware/TFTP validation.


Public CLI deployment and the security review are documented in
[the CLI authorization review](deploy/cli-auth-review.md). CLI configuration must
explicitly select `auth_mode = "required"`; `login --server` does not change the
server used by later board commands. Device-code approval requires terminal-code
confirmation. Established serial sockets require an unexpired current access
grant in the same login family, so normal refresh preserves the socket while
expiry without refresh closes it. CLI tokens cannot access personal-account APIs,
and session details return a device summary instead of power commands or internal
notes. Fallback credentials are privately and atomically replaced; refresh and 401
cleanup coordinate across processes without deleting a newer saved login.
Rental/authentication features do not require a synchronized CLI upgrade. Client
patches only fix confirmed defects and preserve the existing OAuth/Board wire
protocol. A 401 is returned unchanged and never causes an automatic replay
against an existing gateway. Rental allocation keeps the established
busy/not-found error messages understood by the original CLI waiting loop.
`cargo test -p ostool-server --test cli_gateway_contract` checks the baseline
wire contract against a separate mock gateway.
