# ostool HTTP Boot Loader

This crate contains the UEFI-side loader for `ostool run httpboot`.

Current status:

- The shared no-std core parses `manifest.json`.
- The shared no-std core can extract a URI device path and derive the sibling `manifest.json` URL.
- The `uefi-app` binary builds a minimal UEFI application stub for targets that Rust supports, such as `x86_64-unknown-uefi`.
- The stub opens Loaded Image Protocol, reads its file path URI, and prints the derived manifest URL.
- HTTP download, memory placement, UEFI Boot Services shutdown, cache handling, and the architecture-specific jump backend are intentionally kept as the next implementation boundary.

Build the x86_64 stub after installing the target:

```bash
rustup target add x86_64-unknown-uefi
cargo build -p httpboot-loader --features uefi-app --target x86_64-unknown-uefi
mkdir -p target/httpboot-loader
cp target/x86_64-unknown-uefi/debug/httpboot-loader.efi target/httpboot-loader/BOOTX64.EFI
```

Then set:

```toml
efi_loader_path = "target/httpboot-loader/BOOTX64.EFI"
```
