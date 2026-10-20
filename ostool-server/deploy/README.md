# PostgreSQL, runtime configuration and public deployment

Build once with `cargo build --release -p ostool-server`. The React bundle is
embedded by the build script; no domain, database URL or password belongs in
`VITE_*` variables. The same binary can run on different domains.

Copy [the environment example](../.env.example) to a private file, supply the
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
shell substitution. Keep service listener, data directories, board LAN interface
and TFTP configuration in the TOML file. See [systemd EnvironmentFile](https://www.freedesktop.org/software/systemd/man/latest/systemd.exec.html#EnvironmentFile=).

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

Not all server data is in PostgreSQL. Back up the database **and** the configured
file directories. The system settings storage tab shows this same distinction.

| Data | Current authority | Restart behavior |
| --- | --- | --- |
| Users, roles, OAuth sessions, device codes, PAT hashes | PostgreSQL | Persisted |
| Board inventory | PostgreSQL when configured; TOML in standalone mode | One-time legacy import tracked in `storage_imports`; old files never overwrite DB after import |
| DTB payloads | Files; metadata mirrored in PostgreSQL | Files required; DB metadata is not a full migration |
| OTA images, jobs and device observations | Files | Existing file persistence retained |
| Service/TFTP/network configuration | TOML + runtime environment | Runtime overrides applied on startup |
| Active leases, serial connections, loader discovery, power action state | Memory | No live hardware sessions restored on restart |
| Session artifacts | Files scoped to leases | Cleaned up with lease lifecycle |

Deletion of an imported board cannot resurrect it from the retained TOML backup.
To import additional inventory after initialization, use the management API.
This deployment work does not claim a completed migration of OTA/configuration
data or persistent historical leases.

## Verification

```bash
cargo test -p ostool-server
pnpm --dir ostool-server/webui test
pnpm --dir ostool-server/webui build
```

PostgreSQL integration tests use a disposable database and an isolated schema:

```bash
OSTOOL_TEST_DATABASE_URL=postgres://ostool:ostool@127.0.0.1:54329/ostool \
  cargo test -p ostool-server --test auth_postgres -- --ignored
```

For the full browser account-management test, start a disposable environment
using `scripts/dev-env.sh start`, build the current server, then run:

```bash
cargo build -p ostool-server
OSTOOL_E2E_AUTH_BASE_URL=http://127.0.0.1:2999 \
OSTOOL_E2E_ADMIN_EMAIL=admin@example.com \
OSTOOL_E2E_ADMIN_PASSWORD=admin-password-123 \
  pnpm --dir ostool-server/webui test:e2e
```

The browser suite creates test users/roles in that environment; the ordinary
board-management fixture is separate. Without `OSTOOL_E2E_AUTH_BASE_URL`, only
the PostgreSQL account-management browser test is skipped.
