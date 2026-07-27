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
- create the board, DTB, and TFTP/session artifact directories
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

HTTP Boot is enabled by default. Uploaded UEFI HTTP Boot artifacts reuse the
existing session file storage and lifecycle, so files are scoped to the active
board session and are cleaned up with that session.

Every active board session also exposes uploaded files through
`GET /share/sessions/{session_id}/{relative_path}`. This endpoint supports full
and single-range downloads for every boot mode and remains available when TFTP
is disabled. Upload responses include a board-reachable `http_url`; both the
file and URL expire when the session is released or times out.

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
