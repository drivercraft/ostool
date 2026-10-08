#!/usr/bin/env python3
"""Isolated v6 integration: real OVMF UART, managed PTY, HTTP and WebSocket."""
import argparse
import json
import os
from pathlib import Path
import subprocess
import tempfile


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--tgos", type=Path, required=True)
    parser.add_argument("--kernel", type=Path)
    parser.add_argument("--initramfs", type=Path)
    args = parser.parse_args()
    tgos = args.tgos.resolve()
    workspace = Path(__file__).resolve().parents[2]
    artifacts = Path(tempfile.mkdtemp(prefix="axloader-v6-local-"))
    print("local v6 artifacts:", artifacts, flush=True)
    kernel = (args.kernel or tgos / "target/x86_64-unknown-linux-musl/release/arceos-helloworld").resolve()
    loader = tgos / "target/x86_64-unknown-uefi/release/axloader.efi"
    if not kernel.is_file() or not loader.is_file():
        parser.error("first run cargo xtask axloader test qemu --target x86_64-unknown-uefi in TGOSKits")
    initramfs = args.initramfs.resolve() if args.initramfs else artifacts / "initramfs.cpio"
    if not args.initramfs:
        subprocess.run(["cargo", "xtask", "image", "pack-initramfs", str(tgos / "test-suit/host-initramfs"), str(initramfs)], cwd=tgos, check=True)
    env = {**os.environ, "OSTOOL_QEMU_TGOS": str(tgos), "OSTOOL_QEMU_KERNEL": str(kernel),
           "OSTOOL_QEMU_INITRAMFS": str(initramfs), "OSTOOL_QEMU_ARTIFACTS": str(artifacts)}
    # The Rust harness runs the real server router in its own listener and owns
    # every child, socket and PTY. No system service or physical port is opened.
    subprocess.run(["cargo", "test", "-p", "ostool-server", "--test", "qemu_serial", "--", "--ignored", "--nocapture"], cwd=workspace, env=env, check=True)
    (artifacts / "result.json").write_text(json.dumps({"result": "passed", "serial_config": None,
        "kernel": str(kernel), "loader": str(loader), "initramfs": str(initramfs),
        "kernel_output": str(artifacts / "kernel-serial.log")}, indent=2) + "\n")
    print("local v6: UART identity, automatic parameters, network continue and real kernel output passed")


if __name__ == "__main__":
    main()
