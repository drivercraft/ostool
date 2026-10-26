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

## Deployment and configuration

The only runtime configuration source is `.env` (with process variables overriding
values when launching the binary directly). Copy [.env.example](.env.example) once
to `.env`, set its values, and keep it private. `.env.example` is a template, never
loaded by the server. No server TOML is generated or loaded. Both React apps read
runtime configuration through the API and need no separate environment file.
Editable inventory, users, rentals and business settings remain in PostgreSQL.

Use the unified deployment script from a repository checkout:

```bash
cp ostool-server/.env.example ostool-server/.env
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
service. Failed reload restores the previous files. Local deployment does not install
or start Caddy. Switching an installed public site to local requires removing its
public site block and reloading Caddy before exposing an anonymous backend.

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

Both editions require PostgreSQL and use the same database schema. The default
`OSTOOL_EDITION=local` has no authentication, account or rental management;
`ostool` continues to connect with `auth_mode=disabled`. Public deployment must
explicitly select `OSTOOL_EDITION=public` and configure PostgreSQL, Redis and an
initial administrator. Public edition always enforces authentication; database
connectivity alone does not enable account features.

The frontend reads the edition at runtime, so the same binary serves either
edition. Configuration precedence is process environment, then the selected
`.env` file, then built-in defaults. Without `--env-file`, the server reads `.env` from its
working directory. See [single environment template](.env.example) and
[public deployment](#public-deployment).

Board inventory, DTB content, OTA images/jobs/observations and editable
network/TFTP/upload settings are stored in PostgreSQL. Deployment addresses,
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

For boards using the UEFI HTTP Boot loader, configure the board boot profile
with `kind = "httpboot"`, `network_identity.mac_address`, and, when needed,
`boot_arch`. The server binds each UDP/HTTP loader registration to the board by
its persisted permanent MAC address. Boot manifests and status reports stay in
the active session; serial is used only for target-system interaction.

## Network throughput tests

`OSTOOL_NETWORK_TEST_ENABLED=false` is the deployment default. To enable the
hardware-network throughput listener, set it to `true`; its address, active test
limit and duration limit use the `OSTOOL_NETWORK_TEST_*` keys documented in
`.env.example`. Keep this listener restricted to the hardware network. See the
[API guide](../docs/api.md) for transfer semantics and client commands.

## Local development environment

Use Docker Engine on Linux with the Compose plugin and host networking. The
container build installs the declared Rust,
Node.js 24 and pnpm 10.33.0 tools; no host Rust/Node installation is needed.

```bash
cp ostool-server/.env.example ostool-server/.env
# Edit .env for your addresses, passwords and feature edition first.
ostool-server/scripts/dev.sh start
ostool-server/scripts/dev.sh status
ostool-server/scripts/dev.sh build
ostool-server/scripts/dev.sh restart
ostool-server/scripts/dev.sh stop
```

The independent Docker-only launcher runs the same release binary as deployment,
together with PostgreSQL/Redis and, for public edition, a standard Caddy container. It has no development authentication/origin bypass or simulated
runtime. `start` and `restart` build current source, apply Compose configuration and
reload the development gateway configuration;
`build` only builds the image. `stop` removes backend/database/cache containers and retains named data
volumes. It never changes systemd or `/etc/ostool-server`. Containers use the Compose
project name `ostool-server`; both feature editions share the same data volumes.

The launcher calls Compose directly and never calls `deploy.sh`, systemd or sudo.
It uses the `gateway` profile only for public edition, deriving gateway addresses
from the canonical origin and allowed origins in the same `.env`. The backend has
no debug flags, alternate authentication paths or simulated runtime. `seed` is an
explicit offline import; it does not change the running release program. Inventory,
people, roles, bookings, applications, lease history and audits use ordinary business
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

Local edition uses direct HTTP on port 2999, including LAN addresses when listening
on `0.0.0.0`. For public edition, set a canonical HTTPS origin and bind the backend to
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
[the management guide](../docs/admin-ui.md).

Growing user, rental, application, audit and lease-history lists use database
pagination (default 20, maximum 100 rows), including personal workspace lists.
The admin event stream accepts topic selection and sends resources as they become
ready; list changes carry revision markers rather than complete user/rental histories.
Search is debounced, loading keeps table headers, and updates preserve displayed
rows and editor drafts. Calendars query only the visible device/time range, and user
selection searches at most 50 results. Audit filter options share a 60-second cache.
SQLx migrations add pagination indexes without changing existing migration checksums.
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
Turnstile settings in the management console; hostname and action are checked.
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
`OSTOOL_REQUIRE_ADMIN_MFA=true` restricts unenrolled admins to enrollment; ordinary
members use their own MFA preference. Changes revoke other sessions/PATs. Public
deployments must provide and back up the same stable key through their environment.

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

Device-side loader/TFTP/boot traffic has a separate network trust boundary.
Use VLAN/firewall isolation between tenants, boards and server infrastructure.
Local and public editions share the same user API paths. Local edition does not
require credentials. Public user APIs remain on loopback behind HTTPS Caddy;
uploads, downloads and serial WebSockets all check the user and active rental.
Returned file URLs use the public HTTPS origin, never the device-network origin.
The device-only download listener binds the selected private interface and the
backend port, and exposes only boot/shared file reads, without management, user,
loader or session-control routes. It is used by server-expanded guest commands;
its address is not returned by the user API. No separate device-listener setting
is required. Source ranges default to that interface subnet and loopback;
`OSTOOL_HARDWARE_ALLOWED_NETWORKS` can restrict them explicitly. Actual TCP peers
are checked, ignoring forwarding headers. Keep this listener and device ports
behind the device-network firewall, with no Internet port forwarding.
Interface changes require a backend restart.
UDP discovery and the separate public-edition throughput listener enforce the same
source networks. Device callbacks reject redirects, bypass proxies, bound status bodies
and concurrency, and unverified announcements cannot cancel an in-progress operation.

New clients use `server_side_operations` in the boot profile to select server-side
U-Boot orchestration. They request preparation, open the authenticated serial
WebSocket, receive RAM load addresses, generate/upload a FIT image and request boot.
The server configures the device network and performs TFTP or serial transfer.
Shared-file shell templates are registered as opaque command IDs and expanded on
the server when sent over the existing serial connection. Public boot/serial/TFTP
responses omit device-network addresses, UART paths and binding IDs; public power
responses never expose control commands. Device v6 is actively controlled by the server;
legacy loader polling is unavailable through public user routes. New CLI clients
still use the existing client orchestration when connected to a server that does
not advertise this capability. The protocol extension is required to move the
original U-Boot orchestration off the client, rather than to implement login.

The built-in TFTP server rejects WRQ and binds the private device interface in public
edition. For an existing tftpd-hpa service, enable the installed
`--map-file /etc/ostool-server-tftp.rules` in its actual service configuration;
the bundled `Pa .*` rule rejects all PUTs ([tftpd-hpa remapping](https://manpages.debian.org/bookworm/tftpd-hpa/in.tftpd.8.en.html#FILENAME_REMAPPING)).
Installing the rule does not change another service's running configuration.
Keep UDP TFTP ports confined to the device network with the host/network firewall.

Uploads stream into private temporary files and commit atomically after checking
the current credential, rental and session. Release cancels in-flight uploads and
serializes file cleanup with commits. Startup removes interrupted temporary uploads
and directories whose durable session history confirms an ended lease, preserving
unmanaged paths. System settings provide single-file (64 MiB), session (512 MiB/128 files),
account (1024 MiB), global storage (4096 MiB), upload concurrency (4 globally/2 per
account/1 per session), and total timeout (300 s) defaults. A 15-second idle timeout
also applies. Downloads stream with bounded buffering, 16 global/2 per-session slots,
and stop on session termination or the configured transfer timeout.

An optional board `reset_command` restores the baseline after power-off and before
reuse, including startup. It must be idempotent, completes within 60 seconds, and
failure keeps the device reserved until recovery succeeds. It runs only when an
administrator configures it; RAM-only boot devices may leave it unset. Persistent
storage/firmware isolation still requires a verified board-specific recovery procedure.
Real public DNS/Caddy/TLS and physical firmware/TFTP need
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

The additive `20261022010000_global_usernames.sql` upgrade introduces mandatory,
case-insensitive unique usernames without resetting accounts or credentials. It
uses existing email prefixes where valid and unique; otherwise it derives a unique
name from the account ID. Administrators can inspect and update these names in
user management. New registrations and administrator-created accounts require a
3–40 character username (ASCII letters/digits, underscores or hyphens, starting
with a letter/digit); values are stored in lowercase. Login accepts either username
or email. Both identifiers share the account's failure limits, and username changes
revoke the account's credentials. No product/tenant account model is introduced.

Hardware usage history is stored in `session_history` in both editions. The
administrator's session page links to a paginated history view with device,
identity, date, state and end-reason filters. Device type, username, user ID and
rental ID are captured at allocation time and survive later renaming/deletion.
Release failures retain their diagnostics and remain unfinished until hardware
cleanup succeeds. Normal shutdown requests release; the next controller startup
marks any remaining open records `server_interrupted`, without restoring old
connections. For these records, `ended_at` is detection time, not a claim about
when the process or hardware stopped; `last_heartbeat_at` records last observed
activity. History starts with this upgrade; earlier completed usage cannot be
reconstructed. There is no automatic history deletion.

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
  --test board_migration --test username_migration --test lease_history --test cli_rental_contract -- --ignored
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
