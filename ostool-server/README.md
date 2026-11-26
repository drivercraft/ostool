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

Serial port selection uses stable device identities. USB ports are exposed through
`/dev/serial/by-id/` when available, so multi-interface adapters such as FT4232H
remain bound to the selected interface even when the kernel renumbers `ttyUSB*`
after reconnecting. Existing serial-number and by-path configurations remain
supported when they identify a unique port, even if USB ID is the preferred
identity. If multiple interfaces share a serial number, serial-number selection
returns an ambiguity error; select the interface-specific `usb_id` or `usb_path`
instead. The board editor preserves saved identities, recognizes by-id and
by-path aliases, and marks shared serial numbers as ambiguous.

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

## Deployment and configuration

Startup configuration comes from the selected `.env` file (with process variables
overriding values when launching the binary directly). Use [.env.deploy](.env.deploy)
for deployment or [.env.dev](.env.dev) for local debugging. Both are checked-in examples
with credential placeholders: copy the appropriate file to the ignored `.env`, fill
its values, and keep real credentials private. Examples are not loaded automatically. No server TOML is generated or loaded. Both React apps read
runtime configuration through the API and need no separate environment file.
Editable inventory, users, rentals and business settings remain in PostgreSQL.

Use the unified deployment script from a repository checkout:

```bash
cp ostool-server/.env.deploy ostool-server/.env
chmod 600 ostool-server/.env
# Edit the single .env before deploying.
ostool-server/scripts/deploy.sh docker check
ostool-server/scripts/deploy.sh docker up
ostool-server/scripts/deploy.sh docker status
ostool-server/scripts/deploy.sh docker down
# Native installation, with existing PostgreSQL/Redis connection URLs:
ostool-server/scripts/deploy.sh systemd check
ostool-server/scripts/deploy.sh systemd up --dry-run
ostool-server/scripts/deploy.sh systemd up
```

`up` also updates an installation. It builds before replacing the executable,
preserves data, and backs up an existing installed `.env`. `--env-file /path/.env`
selects the one source file; deployment ignores stale `OSTOOL_*` variables inherited
from the caller. `systemd up --output-dir /tmp/deployment-preview` only generates
installation artifacts and does not build, modify `/etc`, or manage host services.

Docker requires Docker/Compose and includes PostgreSQL/Redis. It uses the supplied
connection URLs without silently rewriting them. For managed databases, configure
the URLs and ensure the supplied containers' ports do not conflict with your services.
Native builds require Rust, Node.js with `util.parseEnv` support (20.12+), the
project-declared pnpm and the normal crate dependencies.
The systemd installer uses existing database/cache services; it does not install OS
packages or change package sources. Public systemd deployment also requires Caddy.

`deploy/` contains the Dockerfile, Compose definition, the backend systemd unit and
one Caddy site block. There is no application-specific Caddy service or Caddy user.
The native backend runs as `ostool`, with `/etc/ostool-server/.env` and
`/var/lib/ostool-server`. Native power commands must be executable by `ostool`;
serial access uses `dialout` where available. Additional writable hardware paths
need an explicit service override.

Docker serial deployments can add the least-privilege override:

```bash
# Set OSTOOL_SERIAL_DEVICE=/dev/ttyUSB0 and OSTOOL_SERIAL_GID to the device's group ID in .env.
ostool-server/scripts/deploy.sh docker up --compose-file ostool-server/deploy/compose.serial.yaml
# The same override is accepted by the Docker development launcher.
ostool-server/scripts/dev.sh start --compose-file ostool-server/deploy/compose.serial.yaml
```

The override maps only the chosen character device, adds its numeric group and
mounts the udev database read-only. Use actual ttyUSB/ttyACM nodes; keep stable USB
identity in the board configuration. For several ports, extend the standard Compose
override's `devices`/`group_add` lists. It does not require privileged containers.

Public native deployment uses the installed, standard `caddy.service` and
`/etc/caddy/Caddyfile`. It manages only `/etc/caddy/ostool-server.caddy`, appends one
`import ostool-server.caddy` if needed, validates the complete candidate configuration,
and reloads Caddy. Existing sites and global options remain intact. Failed validation
restores the site fragment without replacing the main file or stopping the running
service. Failed reload restores the previous files. Public deployments use the existing Caddy service. Local deployments use direct HTTP without login.

Docker deployment manages only the backend, PostgreSQL and Redis. Connect its
loopback API to your existing Caddy, whether that Caddy uses the official systemd
service or the official container. Copy the site block from `deploy/ostool-server.caddy` into
the existing configuration, substitute your domain/upstream, run `caddy validate`,
and reload the same Caddy instance. The script never stops or replaces a shared proxy.

TOML and dotenv are configuration formats, not application tiers. This project keeps
one small dotenv bootstrap file because Compose and systemd can consume it directly.
Both React apps use runtime API settings. Database connections, listener/data paths,
initial administrator and the root encryption key belong to bootstrap configuration.
Site information, legal text, registration, CAPTCHA/session policies, SMTP and
Turnstile belong to database-backed system settings. SMTP passwords and Turnstile
secrets use AES-256-GCM with field-specific authenticated contexts; the UI writes
replacement values or explicit clears, and receives only configured-state flags.
The root key must stay outside the database it decrypts; replacing it needs a planned
re-encryption of existing settings and MFA seeds, not an ordinary settings edit.

### Local and public editions

Both editions require PostgreSQL. `local` uses anonymous HTTP with exclusive hardware sessions; it has no account, rental or approval workflow and does not require Redis. Set CLI `auth_mode="disabled"`.
The local console retains device management, sessions/history, audit logs, site branding, network/upload settings and TFTP. Account, role, registration, email, CAPTCHA and legal-policy screens are hidden; their APIs and SSE queries are disabled.
`public` requires login, permissions, rentals and Redis, with a loopback backend behind Caddy HTTPS. Bootstrap its first administrator through `.env`. Authentication follows the edition: local=false, public=true; a conflicting `OSTOOL_AUTH_ENABLED` is rejected.

Both editions serve the same Board REST and serial WebSocket routes, request
fields, response structures and success status codes. Local board operations and
device management need no login. Public board requests require Bearer credentials
and retain role, rental and session ownership checks; the management console
requires its separate administrator login. The CLI's manual `auth_mode` selects
whether it sends credentials; it cannot disable authentication on a public server.
See the [deployment/API comparison](docs/api.md#两种部署共用的契约).

The frontend reads the edition at runtime, so the same binary serves either
edition. Configuration precedence is process environment, then the selected
`.env` file, then built-in defaults. Without `--env-file`, the server reads `.env` from its
working directory. See the [deployment example](.env.deploy), [local debugging example](.env.dev) and
[public deployment](#public-deployment).

Board inventory, DTB content, OTA images/jobs/observations and editable
network/TFTP/upload settings and chunked session artifacts are stored in PostgreSQL. Deployment addresses,
credentials and hardware paths stay in the environment configuration; live hardware
connections and discovery stay in memory, and session files expire with their
leases. Restarting does not restore hardware connections. Existing board files
are imported only by the [independent migration tool](#existing-board-migration).

The persistence model separates durable identity and scheduling from live I/O:

| Resource | Stored data |
| --- | --- |
| Users and roles | Identity, password hash, role/status, verification and login/change timestamps; MFA, email proofs, password resets, consent and audits have their own tables |
| Board inventory | Complete typed board profile in JSONB; generated/indexed board type, status, tags, MAC and boot kind support lookup and uniqueness |
| DTBs | Name, size, SHA-256, binary content and update timestamp |
| Browser/CLI sign-ins | Hashed access/refresh credentials, client/scope, rotation family, expiry, revocation and security version in `oauth_sessions` |
| Scheduled rentals | Users, boards and time windows in `board_rentals` and `rental_requests` |
| Live board sessions | In-process leases and serial/socket handles; `session_owners` stores their authorization association, not a recoverable hardware session |

Restarting the same database-backed deployment preserves valid login credentials
and scheduled rentals. Browser refresh distinguishes credential rejection (401)
from temporary network/service errors: outages retain the login and draft, retry
connection with backoff, and reauthorize before receiving protected updates.
A database failure closes protected streams without declaring the credentials
expired. Normal expiry and explicit revocation still require login.

The current board controller is a single-process service. A restart ends its live
hardware sessions; it does not automatically reconnect serial handles or restore
exclusive hardware use. Durable usage history is stored separately in PostgreSQL; multi-controller
scheduling would require distributed ownership and recovery checks. Sharing
PostgreSQL alone does not make active hardware sessions work across replicas.

The network throughput and HTTP Boot listeners are disabled by deployment defaults.
Enable them explicitly with the single environment file when needed on the board network.

Every active board session also exposes uploaded files through
`GET /share/sessions/{session_id}/{relative_path}`. This endpoint supports full
and single-range downloads for every boot mode and remains available when TFTP
is disabled. Upload responses include a board-reachable `http_url`; both the
file and URL expire when the session is released or times out.

For UEFI HTTP Boot boards, configure `boot.kind = "httpboot"` and
`network_identity.mac_address`. `httpboot` is the only current wire kind;
`uefi_http`, `boot_arch`, and the removed loader announcement
`public_base_url` are rejected or require manual deployment cleanup. The server
does not migrate board files.

Both HTTP Boot upload endpoints require
`X-Axloader-Protocol-Version: 7` before reading the body or touching session
files. The kernel is transferred as opaque bytes with length and SHA-256;
ostool-server does not parse ELF, inspect ISA, or select an EFI entry symbol.
axloader validates the real image against its local target when `start`
prepares the handoff. The board editor displays the device-reported `arch`
string as diagnostics only, including names unknown to ostool.

The permanent MAC, UART `serial_id`/binding, and current `boot_epoch` remain the
device identity and association keys. Device rejects with 400/413/422 are read
as top-level `{code,message}`, published to the session loader status and
WebSocket path, and retained as the first failure for that boot ID. An uncertain
start is queried only in the same boot epoch and boot ID. If the result remains
unconfirmed, the serial lease stays under console/timeout/cancel observation and
later announcements in that boot generation do not recreate or re-upload it.

Loader OTA treats images as opaque bytes. `OtaStore` accepts nonempty images up
to 32 MiB, preserves labels, size and SHA-256, and verifies storage integrity
before delivery. axloader owns PE/COFF, `Machine` and platform compatibility
checks. Neither the server nor the Web UI gates OTA by CPU architecture. Offline
jobs may queue; missing `ota` state leaves a visible waiting reason. The current
reverse OTA flow calls device `PUT /api/v1/ota/image` and
`POST /api/v1/ota/confirm` with the v7 header through provisioned HTTPS endpoints;
old poll, download, and `ota-status` routes are removed. Device credentials and
trusted certificates remain required; there is no plaintext fallback.

The board editor places device addresses and credentials side by side, with CA
certificates on a separate row. OTA uses one image selector followed by upload,
delete and deploy buttons. Selecting a file starts its upload and shows byte
progress inside the selector; completion displays the filename. Deployment uses
SSE stage updates with an indeterminate bar, without estimating byte percentages.

A device's `202` means staged or ready for handoff, not completed: success requires the matching new
slot to run and be confirmed with the expected digest. Device `400`, `413` and
`422` responses fail the operation once and preserve the HTTP status and bounded
response body internally; management API/SSE projections use safe messages. `409` refreshes state/boot epoch and delays retry; `503`
waits for state, capability or epoch to change without repeated PUTs. Network
errors and other `5xx` responses use finite retries with backoff; uncertain
results, including the final attempt, require a fresh device-status check before
failure. See [loader upgrades](docs/axloader-network-control.md#22-装载器升级)
for the device protocol and confirmation rules.

## Network throughput tests

`OSTOOL_NETWORK_TEST_ENABLED=false` is the deployment default. The authenticated listener defaults to `127.0.0.1:3000`; `/v1/tests` is forwarded through HTTPS by the supplied Caddy templates. If its port is changed, update the existing gateway route to match. To enable the
hardware-network throughput listener, set it to `true`; its address, active test
limit and duration limit use the `OSTOOL_NETWORK_TEST_*` keys documented in
the optional comments in `.env.deploy` and `.env.dev`. Keep this listener on loopback behind the authenticated HTTPS gateway. See the
[API guide](docs/api.md) for transfer semantics and client commands.

## Local development environment

Use Docker Engine on Linux with the Compose plugin and host networking. The
container build installs the declared Rust,
Node.js 24 and pnpm 10.33.0 tools; no host Rust/Node installation is needed.

```bash
cp ostool-server/.env.dev ostool-server/.env
chmod 600 ostool-server/.env
# Replace both database-password placeholders, generate the root key, and set the admin password.
ostool-server/scripts/dev.sh start
ostool-server/scripts/dev.sh status
ostool-server/scripts/dev.sh build
ostool-server/scripts/dev.sh restart
ostool-server/scripts/dev.sh stop
```

The debugging example defaults to the public edition, `https://localhost:8443`,
a loopback API on port 3999, PostgreSQL on 54339 and Redis on 56390, with an
`ostool_dev` schema/cache namespace. This exercises the real login/registration
flow without a production domain. After first startup, run `scripts/dev.sh cert`
and trust the exported CA on the browser/CLI machine. For anonymous hardware
work, change the edition to `local` and the public origin to `http://127.0.0.1:3999`.
Business settings remain in the console; enable registration verification and the
debug registration-code setting there when needed.

The deployment example uses `/var/lib/ostool-server`, a placeholder public domain,
production bootstrap credentials and the normal loopback service ports. Replace
the domain/email and credential placeholders before use. Use URL-safe passwords
or URL-encode the password in the database URL; it must match the Docker database
initialization password. Preserve the existing password for reused database volumes
and keep the root key stable across deployments. Native installations use their
existing PostgreSQL/Redis connection URLs and Caddy service.

The independent Docker-only launcher runs the same release binary as deployment,
together with PostgreSQL/Redis and a standard Caddy container. It has no development authentication/origin bypass or simulated
runtime. `start` and `restart` build current source, apply Compose configuration and
reload the development gateway configuration;
`build` only builds the image. `stop` removes backend/database/cache containers and retains named data
volumes. It never changes systemd or `/etc/ostool-server`. Certificates are exported to `ostool-server/.ostool-server-dev/root.crt`.
Containers use the Compose
project name `ostool-server`; both feature editions share the same data volumes.

The launcher calls Compose directly and never calls `deploy.sh`, systemd or sudo.
It uses the `gateway` profile only for public edition, deriving gateway addresses
from the canonical origin and allowed origins in the same `.env`. The backend has
no debug flags, alternate authentication paths or simulated runtime. `seed` is an
explicit offline import; it does not change the running release program.
U-Boot examples without a serial console and public HTTPboot examples without UDP
discovery are imported as disabled drafts, since the seed has no real device
endpoints. Existing records remain unchanged. To enable an HTTPboot board, configure
its HTTP/HTTPS endpoint or enable UDP discovery; native axloader needs no token or CA.
Inventory, people, roles, bookings, applications, session history and audits use ordinary business
names and consistent relationships, without UI fixture labels. Imported members
have independent random passwords; set a member password in user management when
login is needed. The public import also creates the administrator configured by
`OSTOOL_ADMIN_USERNAME`, `OSTOOL_ADMIN_EMAIL` and `OSTOOL_ADMIN_PASSWORD` if absent;
existing administrator passwords and records are preserved. `cert`
exports the Docker gateway CA for the client to trust. The Docker project and data
volumes are shared with `deploy.sh docker`; do not manage the same environment
concurrently through both commands.

The launcher reads `ostool-server/.env` through Compose, independent of the working
directory. Deployment ignores caller `OSTOOL_*` variables, so the selected file
is the single source for both Compose and the server.
`OSTOOL_ENV_FILE` can select
another environment file; the same file supplies the container's runtime settings.
There is no shell evaluation of dotenv contents and no `OSTOOL_DEV_*` settings.
The previous host-process data directory is not imported or deleted automatically.

For either edition, set a canonical HTTPS origin and bind the backend to
`127.0.0.1:2999`; add its site to your existing Caddy configuration. Certificate ports,
redirects, trust and other global proxy options remain under Caddy's own configuration.
There are no server-specific proxy global options. For the Docker debug environment,
`dev.sh cert` exports the internal CA; production proxies use their standard certificate
management. `dev.sh logs server` selects one service; `check` validates Compose without
starting containers. Both scripts provide grouped help, TTY-only color, strict argument
validation and examples with `--help`. Their usage, parsing, preparation and execution
sections follow the organisation of the referenced
[tgosimages scripts](https://github.com/rcore-os/tgosimages/tree/main/scripts),
without introducing a shared wrapper framework or host-service handling in `dev.sh`.

## Management console

The Rust API is in `src/api/`, persistence in `src/persistence/`, and SQLx database
migrations in `migrations/`. `webui/` builds two independent React/Vite entries:

| Entry | Private source | Purpose |
| --- | --- | --- |
| `index.html` | `src/apps/web/` | Website, login and member workspace |
| `admin/index.html` | `src/apps/admin/` | Console and administrator login |

Unknown website, account and console pages share a themed 404 view with return
navigation. Direct requests to unknown page paths return HTTP 404 while retaining
the corresponding website or console layout. API and asset errors keep their
existing response formats. Keep the document path matcher in `src/static_assets.rs`
aligned with the React routers when adding pages.

Shared controls are in `src/components/`, API clients/SSE in `src/api/`, contracts
in `src/types/api.ts`, and styles under `src/styles/` with `global.css` as the
entry. API, event-stream, configuration and algorithm tests live beside source;
component and browser test suites are not maintained. Neither app imports the
other's private pages. Website and console use independent browser sessions and theme settings,
sharing PostgreSQL accounts and role checks. `/login` opens `/account/*`, while
`/qcl/login` rejects non-administrators. Logging out of one does not log out of
the other. Local edition has no browser login or account/rental features.

The public website uses a shared header, avatar/theme menus and footer, an initial
viewport-sized hero, and separate feature sections. The personal workspace keeps
overview, profile, security, password changes (`/account/password`), rentals,
applications and CLI tokens on distinct routes. Desktop content uses 80% of the
viewport width with responsive mobile gutters. Website and console controls, status
badges, dialogs and theme colors share the same design tokens.
All paginated lists use the same footer, including session history, audits and
device documents. Tables start with a row-number column that continues across
pages within the current filter; cards retain their record title and show the
same number separately. Record IDs remain unchanged.
Board, user and custom-role lists provide quick enable/disable actions in both
table and card layouts. Updates retain permission checks, activation validation,
self-account protection and role revision checks, and arrive through SSE.
Board and user status updates use `PATCH` on their admin item routes with a
`{"disabled": boolean}` body; role actions reuse the revisioned role `PUT`.
Current sessions show one primary status; terminal connection is secondary text
and automatic serial diagnostics are available on demand. Releasing sessions no
longer display unrelated connection or default device-wait badges.
Normal release waits for cleanup. Administrators can explicitly force-end a
disconnected or stuck session from sessions or device lists; this cancels old
release retries, removes the active session and persistently disables the board.
History records `admin_forced` and preserves cleanup errors without claiming
hardware power-off or recovery succeeded. Force-deleting a board also cancels
unfinished rentals, and requires device, session and rental management permissions.

Operation results and loading errors use the same top-center popup notifications
on both surfaces, including login pages and dialogs. They support manual dismissal;
forms do not repeat these messages inline or in the bottom action bar. Draft state,
field guidance, diagnostic records and notices requiring an action remain contextual.
Field guidance uses a clickable exclamation icon beside each field title, rather
than text below controls or long placeholders. Section descriptions remain visible
below their headings, with no section help icon.
Boolean configuration fields use a title above a Switch in the same control row
height as inputs. List selections, permission sets and explicit confirmations keep
checkboxes. Toggling a configuration field still edits the draft until it is saved. Board editor sections
stack vertically with compact rows on wide screens. Tags and notes share a row;
power selectors share the first row and the two command/test groups share the next.
Controls use the same height. Running an operation
only disables the power buttons; fields remain editable. Results arrive through
SSE and use the shared popup, without a separate result panel or query/clear buttons.
Website navigation starts new pages at the top, preserves document anchor links,
and leaves back/forward scroll restoration to the browser. Live data updates do
not reset the current page's position.

The console groups devices/DTBs/short sessions, rentals, users/roles, audit logs
and bottom system settings. Toolbars, independently scrolling content and footer
status/actions fill the viewport. Lists support table/card layouts with shared
filters, actions and pagination; dashboard task tables have a fixed layout.
The site-information tab contains branding, contact/support, operator/registration
details and the maintenance notice; these groups share one save action. Terms and
privacy policies retain their independent tabs and publication histories.
All updates use SSE, preserving drafts, focus and scroll position. Configuration
conflicts require explicit resolution. See the
[management interface and event protocol](docs/admin-ui.md).

The dashboard provides runtime health (`/qcl/overview`), background tasks
(`/qcl/jobs`) and operational metrics (`/qcl/operations`). Resource and service
snapshots update every five seconds through SSE; charts retain at most five
minutes from this process. Unavailable host metrics stay unavailable. Local
edition hides user/rental metrics. The dashboard footer includes connection
health; the device toolbar opens discovery information.

`/qcl/audits` provides PostgreSQL-backed pagination, filters, details and CSV
export for identity, permissions, settings, board/session/file/power/OTA and
user/role/rental operations. Mutations record an intent before execution; audit
storage failure returns 503 without executing the action. If result recording
fails, the intent remains pending and the response includes
`X-Audit-Status: pending`. Credentials, raw request bodies, file contents and power
commands are excluded. Configure retention under **System settings → Network and uploads →
Audit retention**: `0` retains all records; `1..3650` enables hourly cleanup using
the latest saved policy. Changes require `audits.manage` in public deployments. Pending intents are retained. API/filter details
are in [the API reference](docs/api.md).

System settings manage branding, contacts, filing information, maintenance
notices, versioned user agreements and privacy policies. Public settings update
via SSE. Registration requires explicit consent to current legal versions;
publication preserves immutable history and per-user acceptance records. Database,
SMTP/Turnstile secrets are encrypted in PostgreSQL; Redis connections and the root encryption key remain in the protected bootstrap file,
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
[the management guide](docs/admin-ui.md).

Growing user, rental, application, audit and lease-history lists use database
pagination (default 20, maximum 100 rows), including personal workspace lists.
The admin event stream accepts topic selection and sends resources as they become
ready; list changes carry revision markers rather than complete user/rental histories.
Search is debounced, loading keeps table headers, and updates preserve displayed
rows and editor drafts. Calendars query only the visible device/time range, and user
selection searches at most 50 results. Audit filter options share a 60-second cache.
The initial SQLx baseline includes the pagination indexes.
Bundled Caddy configurations compress hashed static assets with zstd/gzip; HTML
remains uncached and hashed assets keep their long-lived immutable cache policy.

## Public deployment

Use `scripts/deploy.sh docker up` or `scripts/deploy.sh systemd up` with the same
`.env`, selecting `OSTOOL_EDITION=public`. Native deployment uses the existing standard Caddy service for this edition; Docker deployment connects to the existing gateway. The bundled Caddyfile filters loader/update routes from
the public website; session downloads go through the normal authentication middleware. Do not expose the loopback backend to the Internet.

For a direct binary launch:

```bash
cargo build --release -p ostool-server
./target/release/ostool-server --env-file /path/to/.env --check-config
./target/release/ostool-server --env-file /path/to/.env
```

Secrets are runtime settings and must never be `VITE_*` build variables. The same
embedded website/admin assets work for either edition and deployment domain.
HTML is not cached; hashed assets remain immutable. The server never creates a
second configuration file. Data such as editable TFTP/network/upload settings
are initialized once and subsequently managed by the UI/API in PostgreSQL.

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

Reserved usernames and display names are configured in **System settings → Login and security →
Account and registration** (`name_policy.username_keywords` and
`name_policy.display_name_keywords`). Keywords use case-insensitive substring matching;
comparison uses Unicode NFKC and removes separators, whitespace and invisible marks.
Defaults reserve admin/administrator/root/system and official-support identities,
including 管理员、官方、客服 in display names. Lists contain at most 128 plain-text
keywords of 64 characters each; empty lists disable that field's restriction.
Registration, member profile edits and administrator user writes enforce the same
server policy. Existing names remain valid until changed, so policy updates do not
lock out administrators or block unrelated profile edits. Trusted deployment admin
provisioning and offline data imports remain separate from user-facing naming rules.
Policies persist in PostgreSQL, apply on save and survive restarts; local edition
continues to have no identity or account-policy UI. This filtering limits obvious
impersonation; it is not a full Unicode confusable-identifier implementation.

Connection addresses and the root MFA/settings encryption key stay in `.env`. SMTP passwords and Turnstile secrets are editable, encrypted database settings; responses expose only credential-presence flags.
The server rejects incomplete SMTP/Turnstile/MFA combinations. Account-policy, authentication and rental features are public-only.

User agreements and privacy policies have separate tabs with a workspace-filling
plain-text editor, split preview, fullscreen mode and an icon to open version
history. Preview matches the public legal page and does not execute HTML. Loading
a historical version only edits the draft; publishing creates a new immutable
version. Unrelated settings updates do not replace a document draft. Site contacts
also include support hours and a separate privacy email, falling back to the
support email when empty.

### Startup configuration and system settings

`.env` contains only dependencies needed before the server can open its runtime
settings: edition, API listener/data directory, PostgreSQL/Redis connections and
cache namespace, root encryption key, first-admin bootstrap, external origins and
trusted proxies, plus separately bound hardware/test listeners and QEMU process
resources. Container database/serial mount variables also stay in the startup file.
The root key cannot be stored in the database that it decrypts.

System settings own mail, CAPTCHA, registration/recovery/MFA policies, session and
rate limits, administrator network restrictions, local registration previews,
audit retention, network-interface selection, upload budgets and TFTP configuration.
Saving updates PostgreSQL and SSE; business values are not imported from or
overridden by `.env`. Retention workers read the latest policy on each hourly run.
Startup rejects the removed business environment keys by name, without printing
values. Remove those keys from old deployment files and manage them in the console.

SMTP username/API Key, SMTP password/Secret Key and Turnstile Secret Key are encrypted
with separate field contexts and displayed only as **Configured/Not configured**.
Inputs remain empty after save or reload; omitting or leaving a value empty preserves
it, explicit clear flags delete it, and replace+clear conflicts are rejected.
Only the current settings record format is accepted. Startup does not migrate
plaintext credentials, import old records or fill missing credential envelopes.
Old settings must be converted offline before upgrading; malformed records fail
without being rewritten.

### Security configuration

Public browser login, administrator login, registration-email sending and password
recovery share two human-verification providers, selectable under **System settings →
Login and security**:

- **Local image CAPTCHA (custom)**: the server generates randomized PNG images;
  no external verification service is needed. Challenges last 120 seconds.
- **Cloudflare Turnstile**: use a Managed widget, set its public Site Key, encrypted
  Secret Key, and an explicit hostname allowlist covering the website and console.
  The root settings-encryption key must be configured before saving the secret.
  Hostnames accept spaces, newlines and English/Chinese commas; values are lowercased
  and deduplicated. URLs, ports, wildcards and invalid DNS labels are rejected.
  Challenges last 300 seconds; tokens are validated server-side with action,
  hostname and the challenge UUID in `cdata`, following the
  [Cloudflare validation contract](https://developers.cloudflare.com/turnstile/get-started/server-side-validation/).

Save the configuration, then use **Test saved configuration** in the same page to
complete a real challenge. The test works with either provider and requires
`security.manage`; its challenge cannot authorize login or registration. Public
clients cannot request the `configuration_test` action. The UI tests saved values,
keeps configuration drafts intact, and refreshes the test after a settings revision.
Cloudflare script loads have a 15-second timeout and can be retried; expiration,
widget timeout, unsupported browsers and network/configuration errors clear the proof.
CSP allows the official Cloudflare widget on public pages so switching providers
in the console works without reloading the application.

Every challenge is one-shot and bound to purpose, source IP, User-Agent and current
provider/key/hostname configuration. Wrong, empty, oversized or replayed answers
consume the challenge; provider/key/allowlist changes invalidate outstanding proofs.
Issuance defaults to 20 challenges per IP/action/minute and 60 per IP/minute across
all actions; verification has a shared 60/IP/minute budget. Admin test issuance and
verification each allow five/IP/minute. Redis failures deny verification. The local
registration-code preview does not disable human verification. There is no bypass
switch or automatic fallback when Cloudflare is unavailable.

Human verification is configured only through system settings; the startup
environment does not accept provider, site-key, secret-key or hostname overrides. Cloudflare
[dummy keys](https://developers.cloudflare.com/turnstile/troubleshooting/testing/)
do not return action/cdata, so they cannot pass this application's strict binding
checks; test the complete widget with real keys for your allowed hostname, or use
the local image provider for offline development. Email verification is optional
and defaults to `OSTOOL_REGISTRATION_EMAIL_VERIFICATION=false`.

After CLI device authorization succeeds, the right-hand login pane switches to
a success card prompting the user to return to the ostool terminal. Failed
authorization keeps the login form and displays the error.

New/changed passwords require at least 8 characters and at most 1024 bytes;
Argon2 hashing is concurrency-limited. Login is limited by account and source IP,
with six failed attempts causing a 15-minute cooldown by default. Auth entrypoints
also have a per-IP limit; 429 includes `Retry-After`. Redis stores TTL-bound
counters using atomic operations; PostgreSQL remains authoritative for users,
sessions, challenges, email verification/reset proofs and audit. Redis connection
failure prevents public startup; runtime limiter failure rejects verification
with 503. Only public edition connects to Redis. Keep Redis private with
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
device approvals. Built-in role permissions are fixed; editing assigned custom
roles revokes affected credentials. Roles with users cannot
be deleted. PATs last 90 days by
default (configurable 1–365) and grant CLI board access only. Admin-console operations,
including audit export and security management, use the authenticated administrator
session without additional password/MFA confirmation. Login-time MFA and administrator
permissions remain enforced. Website personal-account changes, PAT creation and CLI
authorization require identity verification within the last three minutes.
Account security pages manage current login devices
and revocation; SSE and serial connections recheck credentials after revocation.

To enable optional TOTP MFA, generate a stable encryption key once:

```bash
openssl rand -base64 32
```

Store it as `OSTOOL_MFA_ENCRYPTION_KEY` in the protected environment file and back
it up with the database. Replacing/losing it prevents decryption of existing MFA
secrets. TOTP secrets use AES-256-GCM; eight single-use recovery codes are stored
only as hashes. TOTP uses 30-second steps and rejects replay. Keep clocks in sync.
Enable **Require administrator MFA** in system settings to restrict unenrolled admins to enrollment; ordinary
members use their own MFA preference. Changes revoke other sessions/PATs. Public
deployments must provide and back up the same stable key through their environment.

Configure mail in **System settings → Mail service** in the public edition. Select
custom SMTP, Brevo, Mailjet or Amazon SES, set a verified sender, and save credentials
in the encrypted SMTP password field (requires the stable root encryption key).
Brevo uses its SMTP login + SMTP Key; Mailjet uses API Key + Secret Key; SES uses
regional SMTP credentials, not an AWS Secret Access Key. Managed providers use
STARTTLS on port 587; SES derives its endpoint from `ses_region` (default `us-east-1`).
Custom SMTP uses the configured host, port and TLS mode: `starttls` normally uses
587, `tls` 465, and `loopback_plaintext` is restricted to loopback mail relays/tests.
Each provider retains separate encrypted username/password credentials when switching.
The mail page shows a shared sender section and four provider cards. Each card can test
its current draft using saved credentials for blank inputs; tests do not save settings
or send mail. Tests require `security.manage`, allow five attempts/administrator/minute
and ten/IP/minute, and time out after 15 seconds.
Provider selection affects new submissions only. This integration sends through SMTP;
it does not synchronize provider quotas, bounces or delivery receipts.

Enable **Registration email verification** in the security settings to require an
email code before account creation (disabled by default for existing deployments).
The registration page shows the resend cooldown inside the code input. Code expiry is
enforced internally, with a message only when the code expires. After cooldown,
resending stays on the code step and requires a fresh CAPTCHA.
The registration page has three steps: enter email and complete CAPTCHA to send a
code, validate the code with the server, then set up username/password and accept
the legal documents. The final submission includes `email_challenge_id` and
`email_code`; no second CAPTCHA is needed for email-verified registration.
The precheck does not consume the code or reset failed attempts; final consumption
and account creation share a transaction. Username conflicts or changed legal
versions therefore allow corrections without sending another code. Returning to
previous steps preserves drafts; changing email/code clears verification, and expiry
returns to email verification. Resending requires a fresh CAPTCHA and invalidates
the previous code. The resend countdown survives page reloads in the same tab.
Sites without required email verification use email/account steps and require a
registration CAPTCHA on final submission.

Password recovery is available at `/password-reset` when mail and password recovery are enabled;
it preserves enabled MFA and revokes old sessions/PATs after a successful reset.
Recovery keeps the email-code flow, shows code expiry, permits returning to edit
the email, and blocks duplicate submissions. Resend countdowns survive reloads in
the same tab; HTTP `Retry-After` updates the send/confirmation cooldowns.

For local registration testing, enable **Local debug registration codes** and
**Registration email verification** under **System settings → Login and security**.
The setting defaults to false, is stored in PostgreSQL and takes effect without a
restart, including with the release image used by `scripts/dev.sh`. The API must
use a loopback listener. Keep the public edition (the local edition has no accounts).
Registration requests then simulate delivery without requiring or contacting SMTP.
In **System settings → Mail service → Mail delivery history**, administrators with
`security.read` can see the current **Debug registration code** and its remaining
lifetime; simulated rows are labeled separately from SMTP submissions.

Only registration codes are previewed; password reset and current-email verification
still require SMTP. The one-time code copy lives only in bounded process memory,
never in database rows, logs, registration responses or public projections. The
database still stores only hashes and governs verification, attempts and expiry.
Consumed, replaced, invalidated and expired codes disappear from the current admin
view; restarting loses the preview, so request a new code after the cooldown.
CAPTCHA, IP/email/site limits and all registration steps remain active. Disabling the
setting hides previews and restores normal SMTP registration delivery.
Use the debug policy only in a local test deployment.

`POST /api/v1/auth/email-code` accepts `{email, action, verification}`, where `action`
is `register` or `password_reset` and `verification` is the existing action-bound
human proof. It returns `{challenge_id, expires_in, resend_after, message}`.
`POST /api/v1/auth/registration-email/verify` accepts `{email, challenge_id, code}`
and returns `{verified:true}` without creating an account or consuming the code.
It requires open registration with email verification enabled, shares each code's
failure budget with final registration, and limits each IP to 10 checks per 15 minutes.
Invalid/expired/consumed codes return HTTP 400 `registration_email_invalid`.
`POST /api/v1/auth/password-reset` accepts `{challenge_id, code, new_password}`.
All browser mutations retain Origin/CSRF checks. Requests are accepted before SMTP
submission. Registration requests for an already-used email return HTTP 409
`registration_email_used` after CAPTCHA and IP checks, without issuing a code or
sending mail. Unknown/disabled recovery accounts and SMTP failures retain the same
accepted response. Inspect mail history for submission results;
`sent` means SMTP accepted, not delivery to the inbox. Codes are never returned in public responses or stored in plaintext in PostgreSQL;
the explicit local development preview above is the only display exception. Delivery is attempted once, with a 15-second overall timeout;
restart-interrupted sends are not retried automatically. Request another code after
the cooldown if needed.

Default limits: 30 mail-code requests per IP per 15 minutes before CAPTCHA verification;
60-second shared recipient cooldown; 5 requests per email and 8 per IP per hour across
all mail purposes; and 100 requests per site per hour (`mail_hourly_limit`, adjustable
from 1 to 100000). The shared email/IP limits and window follow `email_policy`;
registration additionally follows `registration_email_policy`, and uses the longer
of its cooldown and the shared cooldown. Unknown recipients and failures count toward
these budgets. Redis enforces distributed counters and fails closed on errors;
HTTP 429 includes `Retry-After`. Codes default to ten minutes and five attempts,
including malformed answers, are single-use and purpose-bound; a resend invalidates
the previous code for that purpose. Reset and signed-in email confirmations are each limited to 10 attempts
per IP per 15 minutes. New account settings default to custom SMTP with the site budget above.

Mail providers, SES regions, sending budgets and all SMTP parameters are configured
only through system settings. SMTP username/API Key, password/Secret Key and the
Turnstile Secret Key use encrypted credential fields. Responses expose presence
flags; editors start empty, blank submissions retain a value, and explicit clear
flags remove it. Switching mail providers clears the previous provider credentials;
enter the new credentials before enabling delivery. Public Turnstile Site Keys remain ordinary editable settings.
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

Management inventory and global SSE use explicit device summaries without power/reset
commands or private boot configuration. An authenticated administrator can still read
full configuration for one selected device through its detail endpoint or the scoped
`board_editor_id`/`board_editor` event subscription. These details are computed per
connection, never put into shared replay history, and discarded by the UI when leaving
the editor. Saves and conflicts continue to synchronize through SSE.
Device registration/binding credentials are excluded from management responses.
Power results never echo commands on any platform. Server failures expose a generic
message and `X-Request-ID`; raw diagnostics remain in server logs, while runtime,
TFTP, OTA and lease-history projections show safe failure summaries. Account secret
handling and public administrator/session authorization remain unchanged.

Both editions share board API paths. Public uploads, downloads and serial
WebSockets check identity, role and rental ownership. Local edition permits anonymous HTTP/WS and shared-file downloads during active sessions. No private-interface anonymous
download listener is opened. User file URLs use the configured HTTPS origin.
A device receives a short-lived capability for one immutable file, with only its hash
stored in PostgreSQL. Capabilities expire after 60 seconds and cannot cross files or outlive a session.

The public user plane and device LAN are separate. Public website, CLI, uploads and
serial WebSockets retain HTTPS, user authentication and rental checks. Device LAN
services are available in either edition; axloader does not need TLS firmware.

- Set `OSTOOL_LOADER_NETWORK_ENABLED=true` to receive v7 UDP discovery on port 2998.
  Only private, link-local or loopback sources are accepted. The server contacts the
  announcement's source IP and advertised port over HTTP, checks MAC/boot epoch,
  and performs boot, serial and OTA using the existing v7 protocol. Old poll APIs
  stay removed. Restrict UDP ingress to the device network interface.
- Alternatively, configure a fixed `http://<device-LAN-IP>:<port>` endpoint in the
  board editor. HTTP endpoints require explicit LAN IPs. Bearer credentials are
  optional; ordinary axloader firmware uses none. HTTPS endpoints and custom CAs
  remain supported for devices that implement them; stored credentials stay encrypted.
- U-Boot retains TFTP and YMODEM, selected by the board's `use_tftp` setting.
  Managed transfers stream a `#` progress bar over the existing binary terminal
  channel: YMODEM includes sent/total bytes; TFTP forwards only progress marks,
  keeping device-LAN addresses and internal file paths private. Progress ends
  with `done` only after the transfer succeeds.
  TFTP-selected boards fail clearly if the service is unavailable rather than
  silently sending large images over the UART. In public edition, enabled TFTP
  must bind an explicit LAN IP (for example `192.168.10.2:69`), not `0.0.0.0` or
  a public IP. TFTP is read-only. The server owns network parameters and commands;
  they need not be exposed to public clients.
- Optional device-side HTTP file service: set `OSTOOL_HTTP_BOOT_LISTEN_ADDR` and
  `OSTOOL_HTTP_BOOT_BASE_URL` to matching LAN addresses, such as `192.168.10.2:3001`
  and `http://192.168.10.2:3001`. This separate listener exposes only
  `/device-files/{file_id}?token=...`, using a 60-second capability for one immutable
  session file, with session/rental and streaming checks. It has no user/admin APIs,
  pages or WebSockets. User-facing URLs keep the public HTTPS origin. v7 kernel,
  initramfs and OTA transfers are server-to-device HTTP PUTs, not URL downloads.

Select the actual device LAN interface in **System settings → Network and uploads** if automatic
selection picks another adapter. Existing installations whose stored TFTP settings
were disabled by an earlier migration must re-enable TFTP in system settings and
choose a LAN bind address. The optional throughput listener remains authenticated
behind Caddy; `OSTOOL_NETWORK_TEST_ENABLED` is independent of device boot protocols.

Uploads stream into private temporary files and commit atomically after checking
the current credential, rental and session. The authoritative commit writes PostgreSQL metadata and 256 KiB content chunks in one transaction; disk copies are disposable UART caches. Release cancels in-flight uploads and
serializes file cleanup with commits. Startup removes interrupted temporary uploads
and directories whose durable session history confirms an ended lease, preserving
unmanaged paths. System settings provide single-file (64 MiB), session (512 MiB/128 files),
account (1024 MiB), global storage (4096 MiB), upload concurrency (4 globally/2 per
account/1 per session), and total timeout (300 s, including validation and database publication) defaults. A 15-second idle timeout
also applies. Downloads stream with bounded buffering, 16 global/2 per-session slots,
and stop on session termination or the configured transfer timeout.

An optional board `reset_command` restores the baseline after power-off and before
reuse, including startup. It must be idempotent, completes within 60 seconds, and
failure keeps the device reserved until recovery succeeds. It runs only when an
administrator configures it; RAM-only boot devices may leave it unset. Persistent
storage/firmware isolation still requires a verified board-specific recovery procedure.
Real DNS/Caddy/TLS and physical firmware need
separate deployment validation; isolated application tests do not prove those
conditions. Authentication rules and routes are in [the API reference](docs/api.md).

## Role permissions and immediate device use

`/qcl/users/roles` provides a unified role list and standalone role editor, with
a grouped capability matrix, group/master selection and automatically included
read dependencies. The server owns the catalog (`GET /api/v1/admin/permissions`);
the same codes authorize management HTTP routes and filter every SSE resource.
Role writes require `code` (3–64 lowercase ASCII letters, digits, `_` or `-`, starting
with a letter) and `enabled`. The code is globally unique and immutable; the internal
`id` keeps existing user associations stable. Existing roles receive codes during
migration and stay enabled. Lists support code searches and status filters.
Disabling an assigned role revokes browser/CLI credentials, refresh tokens and PATs,
blocks new login and role assignment, and terminates its device sessions. Re-enabling
requires fresh credentials. Built-in roles stay enabled, and users cannot change the
status or permissions of their own role.
Role edits include the current revision to reject stale concurrent writes. Custom roles can enter a limited console through `console.access` without receiving
all-administrator privileges. `system.admin` remains the full trusted administrator
capability. Administrators cannot grant capabilities they do not own or edit higher
privilege users/roles. Built-in role capabilities are fixed, and changing your own
active role's permissions is prohibited. Editing an assigned custom role revokes
its old credentials and requires affected users to
log in again. Role metadata changes alone preserve sessions.

The three built-in roles are Administrator (`admin`, `system.admin`), Ordinary user
(`member`, `board.use` and `rental.request`) and Advanced user (`advanced`, the same
permissions as `member` plus `board.instant`). Registration defaults to `member`;
upgrading adds `advanced` without changing existing users or their role assignments.
Neither user role grants console access. Built-in roles cannot be disabled, deleted
or have their permissions and allocation limits changed.

Grant `board.instant` to enable automatic approval of qualifying rental applications.
It includes `rental.request` and `board.use`, without console access. Every member
uses the same application page, selects an available board and a start/end time,
and submits the request. Administrators see tenant names and occupied windows in
the rental calendar; members see only anonymous, merged occupied intervals, with
no tenant details or rental identifiers. Occupied time slots cannot be selected.

Enable **Allow automatic approval** (允许自动审批, `allow_instant_use`) per device.
Automatic approval also checks the role's board-type scope, minimum/maximum duration
and each user’s peak concurrent reservations of the same board type over the requested
window. Defaults are 30–480 minutes and two boards per type; different board types
have independent limits, and manually approved rentals count too. The migration
updates the former default policy while preserving customized limits. As with other
role-policy changes, affected accounts must sign in again; scheduled rentals remain
valid. Eligible members see “（免审批）” beside device names in the catalog, details, application choices and
rental history. Out-of-range durations require confirmation before submitting for
manual review. Nonqualifying requests remain pending for administrator review. Conflicting reservations return 409; exact
adjacent `[start, end)` windows are allowed, so one board can be rented to different
people at different times, including after another person's current rental.

Application submission returns `id`, `status` (`pending` or `auto_approved`) and
`rental_id` plus `decision_note`. Pending requests explain which automatic-approval
condition was not met, both in the response and application history. Automatic approval atomically creates an ordinary scheduled rental with
the exact requested times. It has the same lifecycle as manual approval: ending a
CLI session or restarting the server does not shorten the reservation. CLI requests
only use approved rentals that are currently effective; they never allocate devices
without an application. Public edition enforces identity and rental ownership;
local edition continues to use direct exclusive hardware sessions.
Members can cancel future rentals or end active rentals from **我的租赁** after
confirmation. `DELETE /api/v1/account/rentals/{id}` returns 204 for the owner’s
active/future rental, releases the reservation and capacity, and revokes its sessions.
Other users’ rentals and already ended/cancelled rentals return 404. Application
notes use the same 2000 UTF-16 code-unit limit as browser inputs.

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
settings from `--env-file` and process variables, without starting
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

The baseline includes mandatory, case-insensitive unique usernames. Registrations
and administrator-created accounts require a 3–40 character username (ASCII
letters/digits, underscores or hyphens, starting with a letter/digit); values are
stored in lowercase. Login accepts a username or email. Both identifiers share
the account's failure limits, and changing a username revokes its credentials.
The baseline also includes all three builtin roles, rental and usage history,
session artifacts, device connections/documents, and DTB enable/disable state.

Hardware usage history is stored in `session_history` in both editions. The
administrator's session page links to a paginated history view with device,
identity, date, state and end-reason filters. Device type, username, user ID and
rental ID are captured at allocation time and survive later renaming/deletion.
Release failures retain their diagnostics and remain unfinished until hardware
cleanup succeeds. Normal shutdown requests release; the next controller startup
marks any remaining open records `server_interrupted`, without restoring old
connections. For these records, `ended_at` is detection time, not a claim about
when the process or hardware stopped; `last_heartbeat_at` records last observed
activity. History records allocations after database initialization; earlier usage cannot
be reconstructed. There is no automatic history deletion.

Each inventory/database schema must have one active hardware controller. Running
multiple controllers against the same inventory is unsupported. Browser login
sessions and rentals remain independent of these short-lived hardware leases.

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
  --test board_migration --test dtb_status --test lease_history --test cli_rental_contract --test security_completion -- --ignored
cargo test -p ostool-server --test cli_gateway_contract
pnpm --dir ostool-server/webui test
pnpm --dir ostool-server/webui build
cargo build -p ostool-server
```

The web test command covers API clients, event streams and business algorithms.
UI component and browser tests, their fixtures and their dependencies have been
removed. Verify visual changes manually and keep the production web build passing.
Server tests continue to cover authentication, authorization, persistence, session
lifecycle and hardware transport boundaries with disposable PostgreSQL/Redis.

### Board creation and activation

`POST /api/v1/admin/boards` and `PUT /api/v1/admin/boards/{id}` reject reserved
IDs `new` (case insensitive), `.` and `..` and serial ports already assigned to another enabled board. Disabled configurations do not reserve communication or relay ports; enabling rechecks ownership.
Relay checks compare saved selectors even while offline and, when resolvable,
compare device paths to catch different aliases for the same port. Offline aliases
with different selectors cannot be proven equivalent until the device is present;
use the same stable selector consistently. Communication and relay ports share the same ownership check; each relay currently controls one output.

Enabled U-Boot boards require a configured serial console. Incomplete U-Boot
configurations may be saved with `disabled: true`, but cannot be enabled until
the serial configuration is supplied.

When UDP discovery is disabled, new public HTTPboot boards start with `disabled: true`.
Save the board, configure its fixed device LAN HTTP endpoint (or HTTPS for capable
firmware), then explicitly save `disabled: false`. Credentials are optional; an
existing encrypted credential must remain readable. With UDP discovery enabled,
boards can connect through their configured MAC without a fixed endpoint.
Neither configuration validates live connectivity; check discovery and a boot
operation before offering the board to users. Disable public boards before removing
a fixed endpoint, and configure the new ID before re-enabling a renamed board when
UDP discovery is disabled.

### Website rental applications and device provisioning

Ordinary and approval-free users share the **申请租赁** workflow. The server decides
whether to auto-approve after checking the chosen board, time and role limits.
Administrators retain direct scheduled assignment and application approval.

`PUT /api/v1/admin/boards/{id}/device-connection` accepts a LAN-IP HTTP origin or an
HTTPS origin. Native axloader needs no CA or token. Devices that support authentication
may use an optional token of 32–512 printable characters; leaving it empty preserves
an existing encrypted token. HTTPS devices may supply a custom CA PEM. Changes require
an idle board. Set and back up `OSTOOL_MFA_ENCRYPTION_KEY` before storing credentials;
no key is needed for a credential-free HTTP device connection.


### Public device documentation

`/qcl/device-documents` is a dedicated editor for public descriptions, images, highlights and specifications.
Documents are revisioned PostgreSQL records, preserved on board rename and removed on board deletion.
Readers need `devices.read`; publishing needs `devices.manage` in public edition. Local edition retains anonymous administration.
`/devices/:boardId` presents only this authored content and the public device projection.
Device cards have parallel detail/application actions. The application owns one viewport with device information on the left and the form/calendar on the right, stacking on mobile with internal scrolling.
