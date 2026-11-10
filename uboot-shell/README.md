# U-Boot Shell

A crate for communicating with u-boot.

## 1. Command transport

`UbootShell::cmd` and `set_env` require a console with input echo enabled. They
keep at most one unacknowledged input byte in flight. A host write or flush is
not an acknowledgement from the target UART. Commands must be single lines
without control bytes; the firmware's command-line size limit still applies.

### 1.1 Echo validation and recovery

The command sender checks the next echoed character rather than searching ahead
for a matching byte. Complete, syntactically valid terminal CSI sequences are
formatting, not character acknowledgements. An unexpected echo stops the attempt
immediately and reports its byte offset, expected byte and observed byte.
Missing echo retains the five-second deadline, and one send attempt retains its
thirty-second deadline.

CSI recognition requires `ESC [`, parameter bytes followed by any intermediate
bytes, and a final byte, in that order. A CR or LF inside or outside such a
sequence is evidence of possible early execution, not ignorable formatting.
An incomplete sequence shares the current character's deadline.

Before the sender attempts the terminating newline, an echo mismatch or missing
echo can be recovered by interrupting the unfinished line with Ctrl-C and
observing a fresh prompt. The next attempt sends the whole command again, never
just the remaining suffix. There are at most three send attempts. Failed prompt
recovery stops the operation; exhausting the attempts must not leave an
unaccounted partial line for a later command. A disconnect, unexpected line
ending or other evidence that execution may already have started is not a
recoverable echo failure.

Recovery has one five-second deadline covering input draining, Ctrl-C write,
flush and receive. It first drains queued input until idle, then sends Ctrl-C.
It must observe a complete `<INTERRUPT>` line, optionally prefixed by the
detected prompt, followed by that prompt at the start of the next console line.
A bare prompt, a prompt embedded in output, or an interrupt marker consumed
before the new Ctrl-C cannot complete recovery. Consoles without this supported
interrupt response fail recovery rather than falling back to a weaker test.
This establishes the supported handshake, not immunity to arbitrary delayed or
corrupted control traffic.

### 1.2 Execution boundary and cancellation

`UbootShell` owns the command phase across asynchronous suspension and
cancellation. It records possible execution **before attempting to write the
newline**, not after a successful flush. A newline write error, reply timeout,
invalid completion reply or failed command never causes automatic re-execution.
Such errors identify the stage and whether execution is uncertain.

A cancelled send can leave a partial input line, so the next command must first
recover the prompt. Cancellation or failure after possible submission cannot be
turned into a new attempt by calling `cmd` again: an unresolved session refuses
further implicit command submission. The caller must establish a new shell
session and decide how to handle the possibly executed operation.

Receiving a fully identified failed command result and losing the result are
different outcomes; neither permits automatic replay. In particular, a missing
or corrupt completion marker is not proof that the command was never executed.
Recovery cancellation keeps the session dirty, and a final pre-submission
failure either completes recovery or leaves that requirement recorded.

### 1.3 Raw I/O and guarantee limits

`cmd_without_reply` and `loady` retain their raw-command and YMODEM protocols;
they do not inherit ordinary-command retries. Their entry and completion paths
must respect unresolved ordinary-command state. A successful `loady` returns to
the prompt before normal commands resume. A caller using the public `tx`, `rx`
or raw I/O interfaces directly owns protocol synchronization.

`loady` checks admission before entering its own retry loop, so an unresolved
ordinary command cannot be reclassified as a recoverable YMODEM error. A raw
boot command does not need to return to U-Boot; the caller may take the streams
and continue interacting with the booted system.

This is not an end-to-end checksummed console protocol. Echo checking detects
observable input corruption; it cannot prove arbitrary output is lossless or
that electrical noise did not turn a byte into an execution control character.
The library fails conservatively when it observes evidence of possible early
execution. It does not promise exactly-once execution over an arbitrarily
corrupting serial line, change baud rate or parity, or repair the physical link.

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
