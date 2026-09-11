# ostool-server

`ostool-server` is the board management server for `ostool`.

It provides:

- board allocation and lease management
- remote serial terminal access
- TFTP session file handling
- a systemd-friendly deployment model on Linux

## Serial transport

Serial receive, serial transmit, WebSocket receive, and WebSocket transmit run as
independently polled asynchronous workers. Bounded channels separate the transports;
a blocked network write does not stop serial reads, and a blocked serial write does
not stop output or close handling. All workers belong to the session and are cancelled
before the port is reunited and released.

Each data queue holds at most 64 chunks of 4 KiB (256 KiB). Binary and decoded `tx`
commands must fit within 256 KiB; larger commands must be split by the client. A full
queue ends the session with an error instead of silently dropping bytes. The error
is sent when the WebSocket remains writable; otherwise the connection closes.
Normal serial EOF drains queued output first. Session cancellation discards pending
commands and stops forwarding output. Serial writes do not call blocking `tcdrain`;
close waits up to one second for the driver output queue, then clears buffers even
if the transmitter remains stuck. This close deadline does not extend test timeouts.

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
with `kind = "httpboot"`, `network_identity.mac_address`, and, when needed,
`boot_arch`. The server binds each UDP/HTTP loader registration to the board by
its persisted permanent MAC address. Boot manifests and status reports stay in
the active session; serial is used only for target-system interaction.

## Useful Commands

```bash
systemctl status ostool-server
systemctl restart ostool-server
journalctl -u ostool-server -f
vi /etc/ostool-server/config.toml
```

## Management console

Open `/admin/` for the React + shadcn/ui console. Boards, discovery, virtual
hardware, DTBs, leases, TFTP and server settings update through a single SSE
connection; browser list polling is not used. Draft edits remain local until
saved and concurrent configuration changes are reported without overwriting them.

A new board does not have to be saved before powering it on. Configure Custom,
Zhongsheng relay or an existing QEMU device, then click **上电** or **下电**.
Choose a MAC from live discovery or enter it manually, then save the board.
Power completion means the command completed, not confirmed physical feedback.
Leaving the page does not reverse or cancel a submitted power action.

See [the management protocol and development guide](../docs/admin-ui.md).

Incompatible board TOML files are moved into `board_dir/quarantine/<timestamp>-<UUID>/`
with their original contents and a `reason.json` report. Valid boards still load;
the console shows backup locations. Storage/backup errors remain explicit failures.
