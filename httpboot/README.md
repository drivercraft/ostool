# ostool HTTP Boot

This crate contains the UEFI-side loader for `ostool run httpboot`.

Current status:

- The shared no-std core parses `manifest.json`.
- The shared no-std core can extract a URI device path and derive the sibling `manifest.json` URL.
- The `uefi-app` binary builds a minimal UEFI application stub for targets that Rust supports, such as `x86_64-unknown-uefi`.
- The loader opens Loaded Image Protocol, reads its file path URI, and derives the sibling `manifest.json` URL.
- The loader uses UEFI HTTP Protocol to download `manifest.json` and the kernel `.bin`.
- The loader places the kernel at `kernel_load_addr`, prepares memory-map and `ExitBootServices` state, and prints the entry plan.
- The final boot jump is enabled by the explicit `boot-jump` feature.

Build the x86_64 loader used to boot AxVisor after installing the target:

```bash
rustup target add x86_64-unknown-uefi
OSTOOL_HTTPBOOT_MANIFEST_URL='http://<server>:2999/boot/boards/<board>/current/manifest.json' \
  cargo build -p httpboot --features boot-jump --target x86_64-unknown-uefi
mkdir -p target/httpboot
cp target/x86_64-unknown-uefi/debug/httpboot.efi target/httpboot/BOOTX64.EFI
```

The `boot-jump` build exits UEFI boot services and jumps to the downloaded kernel entry point.
For loader-only debugging, build without `boot-jump`:

```bash
cargo build -p httpboot --features uefi-app --target x86_64-unknown-uefi
```

That debug build downloads and loads the kernel, prints the jump plan, and reports `error: boot jump disabled` instead of entering AxVisor.

When publishing a loader through ostool config, set:

```toml
efi_loader_path = "target/httpboot/BOOTX64.EFI"
```

The LoongArch loader requires a LoongArch UEFI PE/COFF build path. Do not use
`BOOTX64.EFI` on a LoongArch board.
