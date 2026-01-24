# U-Boot Shell

A crate for communicating with u-boot.

## Usage

```rust
use uboot_shell::UbootShell;

let port = "/dev/ttyUSB0";
let baud = 115200;
let rx = serialport::new(port, baud)
    .open()
    .unwrap();
let tx = rx.try_clone().unwrap();
println!("wait for u-boot shell...");
let mut uboot = UbootShell::new(tx, rx).unwrap();
println!("u-boot shell ready");
let res = uboot.cmd("help").unwrap();
println!("{}", res);
```
