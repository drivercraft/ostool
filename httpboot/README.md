# ostool HTTP Boot

This crate contains the UEFI-side loader for `ostool run httpboot`.

Current status:

- The shared no-std core parses `manifest.json`.
- The shared no-std core can extract a URI device path and derive the sibling `manifest.json` URL.
- The `uefi-app` binary builds a minimal UEFI application stub for targets that Rust supports, such as `x86_64-unknown-uefi`.
- The loader opens Loaded Image Protocol, reads its file path URI, and derives the sibling `manifest.json` URL.
- When the loader is started from a local EFI system partition instead of HTTP Boot, it can use the compile-time `OSTOOL_HTTPBOOT_MANIFEST_URL` fallback.
- The loader uses UEFI HTTP Protocol to download `manifest.json` and the kernel `.bin`.
- The loader places the kernel at `kernel_load_addr`, prepares memory-map and `ExitBootServices` state, and prints the entry plan.
- The final boot jump is behind the default-off `boot-jump` feature.

Build the x86_64 loader after installing the target:

```bash
rustup target add x86_64-unknown-uefi
cargo build -p httpboot --features uefi-app --target x86_64-unknown-uefi
mkdir -p target/httpboot
cp target/x86_64-unknown-uefi/debug/httpboot.efi target/httpboot/BOOTX64.EFI
```

The default build downloads and loads the kernel, prints the jump plan, and stops before `ExitBootServices`.
Build with `boot-jump` only after the x86_64 AxVisor direct-entry ABI is ready:

```bash
cargo build -p httpboot --features boot-jump --target x86_64-unknown-uefi
```

Then set:

```toml
efi_loader_path = "target/httpboot/BOOTX64.EFI"
```

For local ESP or removable-media boot, build the loader with an embedded manifest URL:

```bash
OSTOOL_HTTPBOOT_MANIFEST_URL=http://10.3.10.229:2999/boot/boards/loongchip-httpboot-smoke/current/manifest.json \
  cargo build -p httpboot --features uefi-app --target x86_64-unknown-uefi
```

In that mode the firmware only needs to start the EFI application from disk/USB/ESP.
The loader still uses UEFI HTTP Protocol to download `manifest.json` and `kernel.bin`.

LoongArch64 boards use the native C loader in:

```text
loongarch64-uefi-loader/
```

Build it with:

```bash
make -C loongarch64-uefi-loader
```

The output is:

```text
target/loongarch64-uefi-loader/BOOTLOONGARCH64.EFI
```

If UEFI Shell returns `Command Error Status: Unsupported` before printing
`ostool LoongArch64 UEFI loader`, rebuild this loader and copy the fresh output
to `EFI/BOOT/BOOTLOONGARCH64.EFI`. The native loader keeps `.text` at PE RVA
`0x1000` and emits a valid no-op `.reloc` block because some firmware rejects
EFI images whose first section starts at RVA `0x0` or whose relocation directory
is malformed.

Do not use `BOOTX64.EFI` on a LoongArch board.
