# U-Boot Shell

A crate for communicating with u-boot.

`cmd` and `set_env` require a console with input echo enabled. They wait for each
input byte to be echoed before sending the next one, so a successful host-side
write cannot overrun an unacknowledged target receive queue. Missing echo fails
after five seconds; sending a command has a thirty-second deadline. Cursor/color
CSI output is excluded from echo acknowledgements. Commands must be single lines
without control bytes. Retries abort the unfinished line with Ctrl-C and observe a
fresh prompt first. The target firmware's command-line size limit still applies.
Raw `cmd_without_reply` requests and YMODEM transfers retain their own response
protocols.

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
