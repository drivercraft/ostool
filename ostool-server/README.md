# ostool-server

`ostool-server` is the board management server for `ostool`.

It provides:

- board allocation and lease management
- remote serial terminal access
- TFTP session file handling
- HTTP network throughput tests on a dedicated port
- a systemd-friendly deployment model on Linux

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
