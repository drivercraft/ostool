# ostool-server

`ostool-server` is the board management server for `ostool`.

It provides:

- board allocation and lease management
- remote serial terminal access
- TFTP session file handling
- a systemd-friendly deployment model on Linux

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
- install the binary to `/usr/local/bin/ostool-server`
- stop an existing `ostool-server` systemd service if present
- recreate `/etc/ostool-server`
- create the DTB and TFTP/session artifact directories
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

`ostool-server` stores platform users, RBAC roles and permissions, web login
sessions, development board configuration, user-facing leases, DTB metadata,
and audit logs in the configured database. MySQL and SQLite are supported;
generated configs use MySQL by default for production-like deployments.

The schema keeps one clear owner for each concept:

- `users` stores account identity, display profile, contact fields, disabled
  state, password hash, last login, and timestamps. `display_name` is the
  canonical real/display name; `nickname` is optional and intentionally separate.
- `roles`, `permissions`, `user_roles`, and `role_permissions` store RBAC
  configuration without duplicating role names on user rows.
- `auth_sessions` stores web login sessions and session audit context such as
  token hash, IP address, user agent, last seen time, revocation time, and expiry.
- `board_configs` stores the authoritative development board configuration JSON.
  Query-facing inventory pages derive board metadata from this single source to
  avoid duplicate board fields drifting apart.
- `site_settings` stores website-level runtime settings such as site name,
  branding URLs, announcements, maintenance mode, self-service rental policy,
  support contacts, and lease duration defaults. Bootstrap settings such as
  database URL, listen address, and data directories remain in the TOML config.
- `leases` stores user-to-board allocations, state transitions, expiry, release
  time, and failure details.
- `dtb_files` stores DTB metadata only; `audit_logs` stores management actions
  and request context.

For MySQL, create the database and user before first start, then configure:

```bash
mysql -u root -p -e "
CREATE DATABASE IF NOT EXISTS ostool CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;
CREATE USER IF NOT EXISTS 'ostool'@'127.0.0.1' IDENTIFIED BY 'ostool';
GRANT ALL PRIVILEGES ON ostool.* TO 'ostool'@'127.0.0.1';
FLUSH PRIVILEGES;
"
```

```toml
[database]
provider = "mysql"
url = "mysql://ostool:ostool@127.0.0.1:3306/ostool"
```

For local development, the generated config uses the same local MySQL URL. MySQL
databases are not created automatically by the server.

For SQLite, no external database service is needed. Point the database URL at a
local file; parent directories and the database file are created automatically:

```toml
[database]
provider = "sqlite"
url = "sqlite:.ostool-server/ostool.db"
```

DTB metadata is stored in the database. DTB binary files, TFTP files, and UEFI
HTTP Boot artifacts are still stored on the file system because they are binary
or session artifacts. DTB metadata records the filename, storage path, size,
SHA-256 hash, uploader, and timestamps.

Fresh databases are seeded with sample board resources and ordinary user
accounts so the resource, dashboard, and admin pages have useful data
immediately. The sample users are `alice`, `bob`, `carol`, and `dave`; their
demo password is `ostool123`. Existing databases with sample data enabled will
also backfill any missing sample users on startup. Disable this behavior when
you need an empty inventory:

```toml
[sample_data]
enabled = false
```

The development config can also seed a default administrator when the account
does not exist yet:

```toml
[sample_data.admin]
enabled = true
username = "admin"
password = "admin"
display_name = "平台管理员"
email = "admin@ostool.local"
```

For production deployments, change the password or disable this seed and use the
CLI bootstrap command below.

To reset local development data, stop the server and reset the selected
database. For MySQL, recreate the database:

```bash
mysql -u root -p -e 'DROP DATABASE IF EXISTS ostool; CREATE DATABASE ostool CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;'
```

For SQLite, remove the database file:

```bash
rm -f .ostool-server/ostool.db
```

## Admin bootstrap

After installing or before first login, create an administrator account:

```bash
ostool-server --config /etc/ostool-server/config.toml admin init \
  --username admin \
  --password 'change-me' \
  --display-name 'Platform Admin' \
  --email admin@ostool.local
```

Then open the platform UI:

```text
http://<server-ip>:2999/
```

The public pages are available without login. User dashboard pages require a
normal user account, and `/admin/*` requires an administrator account.

HTTP Boot is enabled by default. Uploaded UEFI HTTP Boot artifacts reuse the
existing session file storage and lifecycle, so files are scoped to the active
board session and are cleaned up with that session.

For boards using the UEFI HTTP Boot loader, configure the board boot profile
with `kind = "httpboot"` and, when needed, `boot_arch`. The server uses the
allocated board session and that board's serial configuration to send the boot
offer to axloader; the board NIC MAC address is not part of the current control
flow.

## Useful Commands

```bash
systemctl status ostool-server
systemctl restart ostool-server
journalctl -u ostool-server -f
vi /etc/ostool-server/config.toml
```
