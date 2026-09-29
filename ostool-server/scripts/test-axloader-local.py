#!/usr/bin/env python3
"""Isolated QEMU + real ostool-server v5 integration, without host network privileges."""

import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import socket
import struct
import subprocess
import tempfile
import threading
import time
import urllib.error
import urllib.request

MAC = "02:00:00:00:00:01"


def free_port():
    with socket.socket() as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


def request(url, method="GET", body=None, headers=None, timeout=12):
    if isinstance(body, dict):
        body = json.dumps(body).encode()
        headers = {**(headers or {}), "Content-Type": "application/json"}
    req = urllib.request.Request(url, data=body, headers=headers or {}, method=method)
    with urllib.request.urlopen(req, timeout=timeout) as response:
        data = response.read()
        return json.loads(data) if data else {}


def wait_for(label, probe, seconds=110):
    end = time.monotonic() + seconds
    last = ""
    while time.monotonic() < end:
        try:
            result = probe()
            if result:
                print("local v5:", label)
                return result
        except (OSError, ValueError, KeyError, urllib.error.HTTPError) as error:
            last = str(error)
        time.sleep(0.25)
    raise RuntimeError("timed out waiting for {}: {}".format(label, last))


def run(*args, cwd=None):
    subprocess.run(args, cwd=cwd, check=True, stdout=subprocess.PIPE, stderr=subprocess.PIPE)


def kernel_elf():
    data = bytearray(0x1002)
    data[:4] = b"\x7fELF"
    data[4:7] = b"\x02\x01\x01"
    struct.pack_into("<HHIQQ", data, 16, 2, 62, 1, 0x200000, 64)
    struct.pack_into("<HHH", data, 52, 64, 56, 1)
    struct.pack_into("<IIQQQQQQ", data, 64, 1, 5, 0x1000, 0x200000, 0x200000, 2, 0x1000, 0x1000)
    data[0x1000:] = b"\xeb\xfe"
    return bytes(data)


def relay_beacons(capture, guest_port, stopped):
    capture.settimeout(0.5)
    outbound = socket.socket(socket.AF_INET, socket.SOCK_DGRAM)
    while not stopped.is_set():
        try:
            stream, _ = capture.accept()
        except socket.timeout:
            continue
        except OSError:
            if stopped.is_set():
                break
            raise
        with stream:
            stream.settimeout(0.5)
            frames = bytearray()
            while not stopped.is_set():
                try:
                    chunk = stream.recv(65536)
                except socket.timeout:
                    continue
                if not chunk:
                    break
                frames.extend(chunk)
                while len(frames) >= 4:
                    length = struct.unpack_from(">I", frames)[0]
                    if length < 42 or length > 65535:
                        frames.clear()
                        break
                    if len(frames) < 4 + length:
                        break
                    frame = bytes(frames[4:4 + length])
                    del frames[:4 + length]
                    if frame[12:14] != b"\x08\x00" or frame[23] != 17:
                        continue
                    ip_header = (frame[14] & 15) * 4
                    offset = 14 + ip_header
                    if len(frame) < offset + 8 or struct.unpack_from(">H", frame, offset + 2)[0] != 2998:
                        continue
                    udp_length = struct.unpack_from(">H", frame, offset + 4)[0]
                    if udp_length < 8 or len(frame) < offset + udp_length:
                        continue
                    try:
                        announcement = json.loads(frame[offset + 8:offset + udp_length].decode())
                    except (UnicodeDecodeError, ValueError):
                        continue
                    if announcement.get("protocol_version") != 5 or announcement.get("mac_address") != MAC:
                        continue
                    # Only the test endpoint changes: device status still comes from real QEMU TCP4.
                    announcement["http_port"] = guest_port
                    outbound.sendto(json.dumps(announcement).encode(), ("127.0.0.1", 2998))
    outbound.close()


def build_disk(root, tgos, loader):
    (root / "A.EFI").write_bytes(loader)
    (root / "B.EFI").write_bytes(loader + b"\x01")
    shutil.copy2(tgos / "target/x86_64-unknown-uefi/release/axloader-launcher.efi", root / "BOOTX64.EFI")
    state = subprocess.run(["python3", str(tgos / "bootloader/axloader/scripts/init-ota-state.py"),
                            "--stable", "A.EFI", "--trial", "B.EFI", "--output", "."],
                           cwd=root, check=True, text=True, capture_output=True)
    initial_id = state.stdout.strip().removeprefix("first trial update_id=")
    disk = root / "esp.img"
    run("truncate", "-s", "128M", str(disk))
    run("mkfs.vfat", "-F", "32", "-n", "OSTOOLBOOT", str(disk))
    run("mmd", "-i", str(disk), "::EFI", "::EFI/BOOT", "::EFI/AXLOADER")
    run("mcopy", "-i", str(disk), str(root / "BOOTX64.EFI"), "::EFI/BOOT/BOOTX64.EFI")
    for filename in ("A.EFI", "B.EFI", "STATE0.BIN", "STATE1.BIN"):
        run("mcopy", "-i", str(disk), str(root / filename), "::EFI/AXLOADER/" + filename)
    return disk, initial_id


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tgos", type=Path, required=True)
    parser.add_argument("--server-bin", type=Path, required=True)
    parser.add_argument("--ovmf-code", type=Path, default=Path("/usr/share/OVMF/OVMF_CODE_4M.fd"))
    parser.add_argument("--ovmf-vars", type=Path, default=Path("/usr/share/OVMF/OVMF_VARS_4M.fd"))
    args = parser.parse_args()
    tgos = args.tgos.resolve()
    root = Path(tempfile.mkdtemp(prefix="axloader-v5-local-"))
    print("local v5 artifacts:", root, flush=True)
    loader = (tgos / "target/x86_64-unknown-uefi/release/axloader.efi").read_bytes()
    disk, initial_id = build_disk(root, tgos, loader)
    shutil.copy2(args.ovmf_vars, root / "vars.fd")
    server_port, device_port, capture_port = free_port(), free_port(), free_port()
    data = root / "server-data"
    boards = data / "boards"
    boards.mkdir(parents=True)
    (boards / "qemu-v5.toml").write_text('''id = "qemu-v5"
board_type = "qemu-v5"
disabled = false
[network_identity]
mac_address = "02:00:00:00:00:01"
[serial]
baud_rate = 115200
[serial.key]
kind = "serial_number"
value = "qemu-v5-local"
[power_management]
kind = "custom"
power_on_cmd = "true"
power_off_cmd = "true"
[boot]
kind = "httpboot"
boot_arch = "x86_64"
''')
    config = root / "server.toml"
    config.write_text('''listen_addr = "127.0.0.1:{server_port}"
data_dir = "{data}"
board_dir = "{boards}"
dtb_dir = "{data}/dtbs"
[network_test]
enabled = false
[tftp]
provider = "builtin"
enabled = false
root_dir = "{data}/tftp"
bind_addr = "127.0.0.1:6969"
[network]
interface = "lo"
[http_boot]
enabled = true
root_dir = "{data}/http-boot"
public_base_url = "http://127.0.0.1:{server_port}"
[loader_network]
enabled = true
bind_addr = "127.0.0.1:2998"
'''.format(server_port=server_port, data=data, boards=boards))
    capture = socket.socket()
    capture.bind(("127.0.0.1", capture_port))
    capture.listen()
    stopped = threading.Event()
    relay = threading.Thread(target=relay_beacons, args=(capture, device_port, stopped), daemon=True)
    relay.start()
    children = []
    with (root / "ostool-server.log").open("wb") as server_log:
        server = subprocess.Popen([str(args.server_bin.resolve()), "--config", str(config)],
                                  stdout=server_log, stderr=subprocess.STDOUT)
        children.append(server)
        base = "http://127.0.0.1:{}".format(server_port)
        device = "http://127.0.0.1:{}".format(device_port)
        def start_guest(index):
            out = (root / "qemu-{}.log".format(index)).open("wb")
            guest = subprocess.Popen([
                "qemu-system-x86_64", "-m", "512M", "-smp", "1", "-machine", "q35",
                "-accel", "kvm", "-cpu", "host", "-display", "none", "-monitor", "none",
                "-serial", "stdio", "-netdev", "user,id=user0,hostfwd=tcp:127.0.0.1:{}-:2999".format(device_port),
                "-chardev", "socket,id=discovery_capture,host=127.0.0.1,port={},reconnect-ms=100".format(capture_port),
                "-object", "filter-mirror,id=discovery_mirror,netdev=user0,queue=rx,outdev=discovery_capture",
                "-device", "virtio-net-pci,netdev=user0,mac=" + MAC,
                "-drive", "if=pflash,format=raw,readonly=on,file=" + str(args.ovmf_code),
                "-drive", "if=pflash,format=raw,file=" + str(root / "vars.fd"),
                "-drive", "format=raw,if=ide,file=" + str(disk),
            ], stdin=subprocess.DEVNULL, stdout=out, stderr=subprocess.STDOUT)
            children.append(guest)
            return guest
        def stop_guest(guest):
            guest.terminate()
            try:
                guest.wait(timeout=5)
            except subprocess.TimeoutExpired:
                guest.kill()
                guest.wait()
        try:
            wait_for("local server listening", lambda: request(base + "/api/v1/admin/boards"))
            guest = start_guest(1)
            trial = wait_for("first B trial", lambda: (lambda status: status if status["ota"]["pending_update_id"] == initial_id else None)(request(device + "/api/v1/status")))
            request(device + "/api/v1/ota/confirm", "POST", {"update_id": initial_id}, {"X-Boot-Epoch": trial["boot_epoch"]})
            wait_for("server discovered v5 device", lambda: (lambda body: body if "qemu-v5" in json.dumps(body) and MAC in json.dumps(body) else None)(request(base + "/api/v1/admin/loader-devices")))
            session = request(base + "/api/v1/sessions", "POST", {"board_type": "qemu-v5", "board_id": "qemu-v5", "required_tags": []})
            session_id = session["session_id"]
            image = kernel_elf()
            request(base + "/api/v1/sessions/{}/http-boot/kernel".format(session_id), "PUT", image,
                    {"X-HttpBoot-Arch": "x86_64", "X-HttpBoot-Image-Format": "elf64",
                     "X-HttpBoot-Remote-Name": "kernel.elf"})
            wait_for("server pushed ELF into QEMU and handed off", lambda: (
                b'ready_to_handoff' in (root / "qemu-1.log").read_bytes()
                and b'elf_loaded:' in (root / "qemu-1.log").read_bytes()), 120)
            try:
                request(base + "/api/v1/sessions/" + session_id, "DELETE")
            except urllib.error.HTTPError as error:
                if error.code != 404:
                    raise
            wait_for("board released", lambda: (lambda value: value if value["lease_state"] == "idle" else None)(
                request(base + "/api/v1/admin/boards/qemu-v5/runtime-status")))
            stop_guest(guest)
            guest = start_guest(2)
            wait_for("stable B after boot", lambda: (lambda value: value if value["ota"]["active_sha256"] == hashlib.sha256(loader + b"\x01").hexdigest() else None)(request(device + "/api/v1/status")))
            changed = loader
            image = request(base + "/api/v1/admin/loader-images", "POST", changed,
                            {"Content-Length": str(len(changed)), "X-Image-Version": "local-v5"})
            assignment = request(base + "/api/v1/admin/boards/qemu-v5/loader-updates", "POST",
                                 {"image_sha256": image["sha256"]})
            task_id = assignment["update_id"]
            server.terminate()
            server.wait(timeout=8)
            restarted_log = (root / "ostool-server-restarted.log").open("wb")
            server = subprocess.Popen([str(args.server_bin.resolve()), "--config", str(config)],
                                      stdout=restarted_log, stderr=subprocess.STDOUT)
            children.append(server)
            wait_for("server restarted with persistent OTA task", lambda: request(base + "/api/v1/admin/boards"))
            wait_for("server pushed and confirmed OTA", lambda: (lambda jobs: jobs if task_id in json.dumps(jobs) and 'succeeded' in json.dumps(jobs) else None)(
                request(base + "/api/v1/admin/boards/qemu-v5/loader-updates")), 150)
            status = wait_for("confirmed A slot", lambda: (lambda value: value if value["ota"]["active_sha256"] == hashlib.sha256(loader).hexdigest() and value["ota"]["pending_update_id"] is None else None)(request(device + "/api/v1/status")))
            assert status["ota"]["active_sha256"] == hashlib.sha256(loader).hexdigest()
            stop_guest(guest)
            guest = start_guest(3)
            wait_for("confirmed A survives reboot", lambda: (lambda value: value if value["ota"]["active_sha256"] == hashlib.sha256(loader).hexdigest() and value["ota"]["running_sha256"] == value["ota"]["active_sha256"] else None)(request(device + "/api/v1/status")))
            stop_guest(guest)
            (root / "result.json").write_text(json.dumps({
                "result": "passed",
                "loader_sha256": hashlib.sha256(loader).hexdigest(),
                "server_sha256": hashlib.sha256(args.server_bin.read_bytes()).hexdigest(),
                "task_id": task_id,
                "fat_disk": str(disk),
                "guest_logs": [str(root / "qemu-{}.log".format(n)) for n in (1, 2, 3)],
                "server_logs": [str(root / "ostool-server.log"), str(root / "ostool-server-restarted.log")],
            }, indent=2) + "\n")
            print("local v5: real ostool-server, QEMU boot and OTA confirmed across FAT reboots")
        finally:
            stopped.set()
            capture.close()
            for child in reversed(children):
                if child.poll() is None:
                    child.terminate()
                    try:
                        child.wait(timeout=5)
                    except subprocess.TimeoutExpired:
                        child.kill()
                        child.wait()
            relay.join(timeout=2)


if __name__ == "__main__":
    main()
