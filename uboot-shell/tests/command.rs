//! Deterministic byte-stream contract tests through the real public shell API.
use std::{
    collections::VecDeque,
    future::Future,
    io::{Error, ErrorKind, Result},
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll},
    time::{Duration, Instant},
};

use futures::{
    future::poll_fn,
    io::{AsyncRead, AsyncWrite},
};
use uboot_shell::UbootShell;
const CTRL_C: u8 = 3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Gate {
    BodyWrite,
    BodyFlush,
    Echo,
    InterruptWrite,
    InterruptFlush,
    InterruptReply,
    NewlineWrite,
    NewlineFlush,
    Reply,
}

#[derive(Default)]
struct EchoConsole {
    reply: VecDeque<(u8, bool)>,
    pending_echo: usize,
    dropped: usize,
    line: Vec<u8>,
    executed: Vec<Vec<u8>>,
    reader: Option<std::task::Waker>,
    fail_next_echo: bool,
    aborted_lines: usize,
    wrong_echo: bool,
    fail_submit_flush: bool,
    corrupt_result: bool,
    failed_result: bool,
    echo_prefix: Option<&'static [u8]>,
    repeat_prefix: bool,
    interrupt_reply: Option<&'static [u8]>,
    interrupt_replies: VecDeque<&'static [u8]>,
    interrupt_byte_interval: Option<Duration>,
    interrupt_byte_delay: Option<futures_timer::Delay>,
    continuous_interrupt_noise: bool,
    eof_after_interrupt: bool,
    block: Option<Gate>,
    blocked: bool,
    fail_write: Option<Gate>,
    receive_error: Option<ErrorKind>,
    silent_echo: bool,
    last_write: Option<u8>,
    writes: Vec<u8>,
}

impl EchoConsole {
    fn at_gate(&mut self, gate: Gate, cx: &Context<'_>) -> bool {
        if self.block == Some(gate) {
            self.blocked = true;
            self.reader = Some(cx.waker().clone());
            return true;
        }
        false
    }

    fn receive(&mut self, bytes: &[u8]) {
        self.writes.extend_from_slice(bytes);
        for &byte in bytes {
            self.last_write = Some(byte);
            if byte == CTRL_C {
                self.line.clear();
                self.aborted_lines += 1;
                self.reply.extend(
                    self.interrupt_replies
                        .pop_front()
                        .or(self.interrupt_reply)
                        .unwrap_or(b"<INTERRUPT>\r\n=> ")
                        .iter()
                        .map(|&byte| (byte, false)),
                );
                continue;
            }
            if self.pending_echo == 1 {
                self.dropped += 1;
                continue;
            }
            if byte == b'\n' {
                let line = std::mem::take(&mut self.line);
                let completed = line.ends_with(b"&& echo cmd-ok; echo cmd-end");
                // Model both wire versions so the SAME regression tests can
                // run against the baseline without failing on marker spelling.
                let acknowledged = completed || line.ends_with(b"&& echo cmd-ok");
                self.executed.push(line);
                let reply: &[u8] = if self.failed_result {
                    b"\r\nfailure\r\ncmd-end\r\n=> "
                } else if acknowledged && !self.corrupt_result && completed {
                    b"\r\ndevice-result\r\ncmd-ok\r\ncmd-end\r\n=> "
                } else if acknowledged && !self.corrupt_result {
                    b"\r\ndevice-result\r\ncmd-ok\r\n=> "
                } else {
                    b"\r\ninvalid command\r\n=> "
                };
                self.reply.extend(reply.iter().map(|&byte| (byte, false)));
            } else {
                self.line.push(byte);
                if std::mem::take(&mut self.silent_echo) {
                    continue;
                }
                self.pending_echo += 1;
                let prefix = self.echo_prefix.unwrap_or(b"\x1b[0m");
                self.reply.extend(prefix.iter().map(|&byte| (byte, false)));
                if !self.repeat_prefix {
                    self.echo_prefix = None;
                }
                if std::mem::take(&mut self.wrong_echo) {
                    self.reply.push_back((b'!', false));
                }
                self.reply.push_back((byte, true));
            }
        }
        if let Some(reader) = self.reader.take() {
            reader.wake();
        }
    }
}

struct EchoTx(Arc<Mutex<EchoConsole>>);
struct EchoRx(Arc<Mutex<EchoConsole>>);

impl AsyncWrite for EchoTx {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, bytes: &[u8]) -> Poll<Result<usize>> {
        // Like a host TTY, accepting a write does not prove the remote
        // console's finite receive capacity could retain every byte.
        let mut console = self.0.lock().unwrap();
        let gate = match bytes {
            [CTRL_C] => Gate::InterruptWrite,
            [b'\n'] => Gate::NewlineWrite,
            _ => Gate::BodyWrite,
        };
        if console.at_gate(gate, cx) {
            return Poll::Pending;
        }
        if console.fail_write == Some(gate) {
            return Poll::Ready(Err(Error::other("injected write failure")));
        }
        console.receive(bytes);
        Poll::Ready(Ok(bytes.len()))
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Result<()>> {
        let mut console = self.0.lock().unwrap();
        let gate = match console.last_write {
            Some(CTRL_C) => Gate::InterruptFlush,
            Some(b'\n') => Gate::NewlineFlush,
            _ => Gate::BodyFlush,
        };
        if console.at_gate(gate, cx) {
            return Poll::Pending;
        }
        if !console.executed.is_empty() && std::mem::take(&mut console.fail_submit_flush) {
            return Poll::Ready(Err(Error::other("injected submit flush failure")));
        }
        Poll::Ready(Ok(()))
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Result<()>> {
        Poll::Ready(Ok(()))
    }
}

async fn echo_shell(console: &Arc<Mutex<EchoConsole>>) -> UbootShell {
    let shell = UbootShell::new(EchoTx(console.clone()), EchoRx(console.clone()))
        .await
        .unwrap();
    let mut fixture = console.lock().unwrap();
    fixture.aborted_lines = 0;
    fixture.writes.clear();
    shell
}

#[tokio::test]
async fn strict_echo_stops_before_matching_later_byte() {
    let console = Arc::new(Mutex::new(EchoConsole {
        wrong_echo: true,
        ..Default::default()
    }));
    let mut shell = echo_shell(&console).await;
    assert_eq!(shell.cmd("echo safe").await.unwrap(), "device-result");
    let console = console.lock().unwrap();
    assert_eq!(console.aborted_lines, 1, "wrong echo was ignored");
    assert_eq!(console.executed.len(), 1);
    assert_eq!(console.dropped, 0);
}

#[tokio::test]
async fn submitted_command_is_never_replayed_after_error() {
    for flush_failure in [true, false] {
        let console = Arc::new(Mutex::new(EchoConsole {
            fail_submit_flush: flush_failure,
            corrupt_result: !flush_failure,
            ..Default::default()
        }));
        let mut shell = echo_shell(&console).await;
        let error = shell.cmd("echo side-effect").await.unwrap_err();
        assert!(error.to_string().contains("execution uncertain"), "{error}");
        assert_eq!(
            console.lock().unwrap().executed.len(),
            1,
            "submitted command was replayed"
        );
        assert_unresolved_entries_rejected(&mut shell, &console).await;
    }
}

impl AsyncRead for EchoRx {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bytes: &mut [u8],
    ) -> Poll<Result<usize>> {
        if bytes.is_empty() {
            return Poll::Ready(Ok(0));
        }
        let mut console = self.0.lock().unwrap();
        let gate = if console.pending_echo > 0 {
            Some(Gate::Echo)
        } else if console.last_write == Some(CTRL_C) && console.aborted_lines > 0 {
            Some(Gate::InterruptReply)
        } else if !console.executed.is_empty() {
            Some(Gate::Reply)
        } else {
            None
        };
        if gate.is_some_and(|gate| console.at_gate(gate, cx)) {
            return Poll::Pending;
        }
        if console.pending_echo > 0
            && let Some(kind) = console.receive_error.take()
        {
            return Poll::Ready(Err(Error::new(kind, "injected receive error")));
        }
        if console.fail_next_echo && console.pending_echo > 0 {
            console.fail_next_echo = false;
            return Poll::Ready(Err(Error::new(ErrorKind::TimedOut, "injected echo loss")));
        }
        if console.reply.is_empty() {
            if console.continuous_interrupt_noise && console.aborted_lines > 0 {
                bytes[0] = 0x83;
                return Poll::Ready(Ok(1));
            }
            if console.eof_after_interrupt && console.aborted_lines > 0 {
                return Poll::Ready(Ok(0));
            }
            console.reader = Some(cx.waker().clone());
            return Poll::Pending;
        }
        if console.last_write == Some(CTRL_C)
            && console.aborted_lines > 0
            && let Some(interval) = console.interrupt_byte_interval
        {
            let delay = console
                .interrupt_byte_delay
                .get_or_insert_with(|| futures_timer::Delay::new(interval));
            if Pin::new(delay).poll(cx).is_pending() {
                return Poll::Pending;
            }
            console.interrupt_byte_delay = None;
        }
        let size = bytes.len().min(console.reply.len());
        for output in &mut bytes[..size] {
            let (byte, echo) = console.reply.pop_front().unwrap();
            *output = byte;
            if echo {
                console.pending_echo -= 1;
            }
        }
        Poll::Ready(Ok(size))
    }
}

#[tokio::test]
async fn long_command_waits_for_remote_echo_before_advancing() {
    for fail_next_echo in [false, true] {
        let console = Arc::new(Mutex::new(EchoConsole {
            fail_next_echo,
            ..Default::default()
        }));
        let mut shell = echo_shell(&console).await;
        let command = format!("setenv bootargs '{}'", "console=ttyS0 ".repeat(24));
        let result = tokio::time::timeout(Duration::from_secs(2), shell.cmd(&command)).await;
        assert_eq!(
            result
                .expect("command lost bytes before reaching the device")
                .unwrap(),
            "device-result"
        );
        let console = console.lock().unwrap();
        assert_eq!(console.dropped, 0, "command overflowed the remote console");
        assert_eq!(console.aborted_lines, usize::from(fail_next_echo));
        assert_eq!(
            console.executed,
            [format!("{command}&& echo cmd-ok; echo cmd-end").into_bytes()]
        );
    }
}

// Every check below calls the real public entry point; the fixture only provides
// UART observations and side effects. In particular Ctrl-C never purges RX.
async fn assert_unresolved_entries_rejected(
    shell: &mut UbootShell,
    console: &Arc<Mutex<EchoConsole>>,
) {
    let before = console.lock().unwrap().writes.clone();
    for error in [
        shell.cmd("echo forbidden").await.unwrap_err(),
        shell.cmd_without_reply("booti").await.unwrap_err(),
        shell
            .loady(0x8000, "not-opened", |_, _| {
                panic!("loady admitted an unresolved command")
            })
            .await
            .unwrap_err(),
    ] {
        assert!(error.to_string().contains("execution uncertain"), "{error}");
    }
    assert_eq!(console.lock().unwrap().writes, before);
}

#[tokio::test]
async fn echo_grammar_and_early_execution_are_enforced() {
    // Valid formatting is skipped as a unit. Invalid grammar cannot swallow a
    // later matching byte; CR/LF cannot be hidden inside even an incomplete CSI.
    for (prefix, recoveries, uncertain) in [
        (&b"\x1b[0m"[..], 0, false),
        (&b"\x1b[1;2 m"[..], 0, false),
        (&b"\x1bX"[..], 1, false),
        (&b"\x1b[ 0m"[..], 1, false),
        (&b"\x1b[0\x00m"[..], 1, false),
        (&b"\r"[..], 0, true),
        (&b"\n"[..], 0, true),
        (&b"\x1b[0\rm"[..], 0, true),
        (&b"\x1b[\nm"[..], 0, true),
    ] {
        let console = Arc::new(Mutex::new(EchoConsole::default()));
        let mut shell = echo_shell(&console).await;
        console.lock().unwrap().echo_prefix = Some(prefix);
        let result = shell.cmd("echo safe").await;
        if uncertain {
            let error = result.unwrap_err().to_string();
            assert!(error.contains("offset 0"), "{error}");
            assert!(error.contains("expected 0x65"), "{error}");
            assert!(error.contains("actual 0x0"), "{error}");
            assert!(console.lock().unwrap().executed.is_empty());
            assert_unresolved_entries_rejected(&mut shell, &console).await;
        } else {
            assert_eq!(result.unwrap(), "device-result", "{prefix:?}");
            assert_eq!(console.lock().unwrap().executed.len(), 1);
        }
        assert_eq!(
            console.lock().unwrap().aborted_lines,
            recoveries,
            "{prefix:?}"
        );
    }
}

#[tokio::test]
async fn recovery_requires_a_fresh_complete_interrupt_handshake() {
    for (response, accepted) in [
        (&b"=> "[..], false),
        (&b""[..], false),
        (&b"noise<INTERRUPT>\r\n=> "[..], false),
        (&b"<INTERRUPT>\r\nnoise=> "[..], false),
        (&b"<INTERRUPT>\r\n\r\n=> "[..], false),
        (&b"<INTERRUPT>\r\n=> "[..], true),
        (&b"=> <INTERRUPT>\r\n=> "[..], true),
    ] {
        let console = Arc::new(Mutex::new(EchoConsole::default()));
        let mut shell = echo_shell(&console).await;
        {
            let mut fixture = console.lock().unwrap();
            // An entire OLD marker + prompt is queued behind the bad echo.
            // The new interrupt response is appended, never substituted for RX.
            fixture.echo_prefix = Some(b"!\r\n<INTERRUPT>\r\n=> ");
            fixture.interrupt_reply = Some(response);
            fixture.eof_after_interrupt = true;
        }
        let result = shell.cmd("echo fresh").await;
        assert_eq!(result.is_ok(), accepted, "{response:?}: {result:?}");
        assert_eq!(
            console.lock().unwrap().executed.len(),
            usize::from(accepted)
        );
        assert_eq!(console.lock().unwrap().aborted_lines, 1);
    }

    for first_response in [&b"\x83"[..], &b""[..]] {
        let console = Arc::new(Mutex::new(EchoConsole::default()));
        let mut shell = echo_shell(&console).await;
        {
            let mut fixture = console.lock().unwrap();
            fixture.wrong_echo = true;
            // Leave stale input in the SAME RX queue after the fresh handshake.
            // Without the final drain it will corrupt the next body's echo.
            fixture.interrupt_replies = [
                first_response,
                b"<INTERRUPT>\r\n=> \x83\r\n=> <INTERRUPT>\r\n=> ",
            ]
            .into();
        }
        let result = shell.cmd("echo fresh").await;
        assert_eq!(
            result.unwrap_or_else(|err| panic!("{first_response:?}: {err}")),
            "device-result"
        );
        let fixture = console.lock().unwrap();
        assert_eq!(
            fixture.aborted_lines, 2,
            "did not synchronize exactly twice"
        );
        assert_eq!(
            fixture.executed,
            [b"echo fresh&& echo cmd-ok; echo cmd-end".to_vec()],
            "command was replayed or corrupted"
        );
        assert_eq!(fixture.dropped, 0);
    }

    // The complete response takes longer than one idle window, but each byte
    // arrives before idle. A whole-handshake 300ms limit would reject it.
    let console = Arc::new(Mutex::new(EchoConsole::default()));
    let mut shell = echo_shell(&console).await;
    {
        let mut fixture = console.lock().unwrap();
        fixture.wrong_echo = true;
        fixture.interrupt_byte_interval = Some(Duration::from_millis(40));
    }
    assert_eq!(shell.cmd("echo slow").await.unwrap(), "device-result");
    let fixture = console.lock().unwrap();
    assert_eq!(fixture.aborted_lines, 1);
    assert_eq!(
        fixture.executed,
        [b"echo slow&& echo cmd-ok; echo cmd-end".to_vec()]
    );
}

#[tokio::test]
async fn exhausting_echo_attempts_cleans_the_last_partial_line() {
    let console = Arc::new(Mutex::new(EchoConsole::default()));
    let mut shell = echo_shell(&console).await;
    {
        let mut fixture = console.lock().unwrap();
        fixture.echo_prefix = Some(b"!");
        fixture.repeat_prefix = true;
    }
    let error = shell.cmd("echo first").await.unwrap_err().to_string();
    assert!(
        error.contains("offset 0: expected 0x65, actual 0x21"),
        "{error}"
    );
    {
        let mut fixture = console.lock().unwrap();
        assert_eq!(fixture.aborted_lines, 3);
        assert!(fixture.executed.is_empty());
        assert!(fixture.line.is_empty());
        fixture.echo_prefix = None;
        fixture.repeat_prefix = false;
    }
    assert_eq!(shell.cmd("echo next").await.unwrap(), "device-result");
    assert_eq!(console.lock().unwrap().executed.len(), 1);
}

#[tokio::test]
async fn transport_errors_and_missing_results_are_not_retryable() {
    for gate in [Gate::BodyWrite, Gate::NewlineWrite] {
        let console = Arc::new(Mutex::new(EchoConsole::default()));
        let mut shell = echo_shell(&console).await;
        console.lock().unwrap().fail_write = Some(gate);
        let error = shell.cmd("echo unsafe").await.unwrap_err().to_string();
        assert!(error.contains("execution uncertain"), "{error}");
        assert_eq!(console.lock().unwrap().aborted_lines, 0);
        assert_unresolved_entries_rejected(&mut shell, &console).await;
    }
    for kind in [
        ErrorKind::BrokenPipe,
        ErrorKind::UnexpectedEof,
        ErrorKind::InvalidData,
    ] {
        let console = Arc::new(Mutex::new(EchoConsole::default()));
        let mut shell = echo_shell(&console).await;
        console.lock().unwrap().receive_error = Some(kind);
        assert!(shell.cmd("echo unsafe").await.is_err());
        assert_eq!(console.lock().unwrap().aborted_lines, 0);
        assert!(console.lock().unwrap().executed.is_empty());
        assert_unresolved_entries_rejected(&mut shell, &console).await;
    }
    let console = Arc::new(Mutex::new(EchoConsole::default()));
    let mut shell = echo_shell(&console).await;
    console.lock().unwrap().corrupt_result = true;
    let error = shell.cmd("echo once").await.unwrap_err().to_string();
    assert!(error.contains("completion marker"), "{error}");
    assert_unresolved_entries_rejected(&mut shell, &console).await;
    assert_eq!(console.lock().unwrap().executed.len(), 1);
}

#[tokio::test]
async fn identified_failure_allows_next_command_but_never_replays() {
    let console = Arc::new(Mutex::new(EchoConsole::default()));
    let mut shell = echo_shell(&console).await;
    console.lock().unwrap().failed_result = true;
    let error = shell.cmd("false").await.unwrap_err().to_string();
    assert!(error.contains("complete failure result"), "{error}");
    assert_eq!(console.lock().unwrap().executed.len(), 1);
    console.lock().unwrap().failed_result = false;
    assert_eq!(shell.cmd("echo next").await.unwrap(), "device-result");
    assert_eq!(console.lock().unwrap().executed.len(), 2);
}

async fn cancel_at_gate(shell: &mut UbootShell, console: &Arc<Mutex<EchoConsole>>) {
    let mut command = Box::pin(shell.cmd("echo interrupted"));
    // No arbitrary sleep: the fixture explicitly reports reaching the exact
    // Pending boundary, including writes that have not accepted any bytes yet.
    tokio::time::timeout(
        Duration::from_secs(3),
        poll_fn(|cx| {
            assert!(
                command.as_mut().poll(cx).is_pending(),
                "gate was not reached"
            );
            if console.lock().unwrap().blocked {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }),
    )
    .await
    .expect("operation did not reach its explicit Pending gate");
    drop(command);
    let mut fixture = console.lock().unwrap();
    fixture.block = None;
    fixture.blocked = false;
}

#[tokio::test]
async fn cancellation_owns_body_recovery_and_submission_across_calls() {
    for gate in [
        Gate::BodyWrite,
        Gate::BodyFlush,
        Gate::Echo,
        Gate::InterruptWrite,
        Gate::InterruptFlush,
        Gate::InterruptReply,
        Gate::NewlineWrite,
        Gate::NewlineFlush,
        Gate::Reply,
    ] {
        let console = Arc::new(Mutex::new(EchoConsole::default()));
        let mut shell = echo_shell(&console).await;
        {
            let mut fixture = console.lock().unwrap();
            fixture.block = Some(gate);
            fixture.wrong_echo = matches!(
                gate,
                Gate::InterruptWrite | Gate::InterruptFlush | Gate::InterruptReply
            );
        }
        cancel_at_gate(&mut shell, &console).await;
        if matches!(gate, Gate::NewlineWrite | Gate::NewlineFlush | Gate::Reply) {
            assert_unresolved_entries_rejected(&mut shell, &console).await;
            assert_eq!(
                console.lock().unwrap().executed.len(),
                usize::from(gate != Gate::NewlineWrite)
            );
        } else {
            assert_eq!(
                shell.cmd("echo next").await.unwrap(),
                "device-result",
                "{gate:?}"
            );
            let fixture = console.lock().unwrap();
            assert!(fixture.aborted_lines >= 1, "{gate:?}");
            assert_eq!(
                fixture.executed,
                [b"echo next&& echo cmd-ok; echo cmd-end".to_vec()]
            );
            assert_eq!(fixture.dropped, 0, "{gate:?}");
        }
    }
}

#[tokio::test]
async fn echo_and_recovery_deadlines_cover_pending_io() {
    // A genuinely missing echo spends the real five-second character budget,
    // recovers and resends the entire line once. No test clock or short limit.
    let console = Arc::new(Mutex::new(EchoConsole::default()));
    let mut shell = echo_shell(&console).await;
    console.lock().unwrap().silent_echo = true;
    let started = Instant::now();
    assert_eq!(shell.cmd("echo delayed").await.unwrap(), "device-result");
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(console.lock().unwrap().aborted_lines, 1);
    assert_eq!(console.lock().unwrap().executed.len(), 1);

    // An incomplete CSI shares the SAME character deadline; receiving its
    // parameter byte is not an acknowledgement for that command character.
    let console = Arc::new(Mutex::new(EchoConsole::default()));
    let mut shell = echo_shell(&console).await;
    console.lock().unwrap().echo_prefix = Some(b"\x1b[");
    let started = Instant::now();
    assert_eq!(shell.cmd("0").await.unwrap(), "device-result");
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert_eq!(console.lock().unwrap().aborted_lines, 1);
    assert_eq!(console.lock().unwrap().executed.len(), 1);

    // The recovery deadline includes drain and a blocked interrupt flush, not
    // only receive. Cancellation by that deadline must retain the dirty line.
    let console = Arc::new(Mutex::new(EchoConsole::default()));
    let mut shell = echo_shell(&console).await;
    {
        let mut fixture = console.lock().unwrap();
        fixture.wrong_echo = true;
        fixture.block = Some(Gate::InterruptFlush);
    }
    let started = Instant::now();
    let error = tokio::time::timeout(Duration::from_secs(7), shell.cmd("echo delayed"))
        .await
        .expect("recovery exceeded its total budget")
        .unwrap_err();
    assert!(
        error.to_string().contains("command recovery timed out"),
        "{error}"
    );
    assert!(started.elapsed() >= Duration::from_secs(5));
    assert!(console.lock().unwrap().blocked);
    assert!(console.lock().unwrap().executed.is_empty());
    console.lock().unwrap().block = None;
    assert_eq!(shell.cmd("echo recovered").await.unwrap(), "device-result");
    assert_eq!(console.lock().unwrap().aborted_lines, 2);

    // Repeated idle timeouts cannot restart the total budget; an always-ready
    // noisy stream must not starve it either.
    for continuous_interrupt_noise in [false, true] {
        let console = Arc::new(Mutex::new(EchoConsole::default()));
        let mut shell = echo_shell(&console).await;
        {
            let mut fixture = console.lock().unwrap();
            fixture.wrong_echo = true;
            fixture.interrupt_reply = Some(b"\x83");
            fixture.continuous_interrupt_noise = continuous_interrupt_noise;
        }
        let started = Instant::now();
        let error = tokio::time::timeout(Duration::from_secs(7), shell.cmd("echo noise"))
            .await
            .expect("interrupt receive exceeded the shared recovery budget")
            .unwrap_err();
        assert_eq!(error.kind(), ErrorKind::TimedOut);
        assert!(started.elapsed() >= Duration::from_secs(5));
        assert!(started.elapsed() < Duration::from_secs(7));
        {
            let mut fixture = console.lock().unwrap();
            assert!(fixture.executed.is_empty());
            if !continuous_interrupt_noise {
                assert!(fixture.aborted_lines > 1);
            }
            fixture.interrupt_reply = None;
            fixture.continuous_interrupt_noise = false;
        }
        assert_eq!(shell.cmd("echo recovered").await.unwrap(), "device-result");
        assert_eq!(console.lock().unwrap().executed.len(), 1);
    }
}

#[tokio::test]
async fn raw_boot_preserves_unacknowledged_protocol_and_stream_handoff() {
    let console = Arc::new(Mutex::new(EchoConsole::default()));
    let mut shell = echo_shell(&console).await;
    shell
        .cmd_without_reply("booti 0x8000 - 0x9000")
        .await
        .unwrap();
    assert_eq!(console.lock().unwrap().writes, b"booti 0x8000 - 0x9000\n");
    assert_eq!(console.lock().unwrap().aborted_lines, 0);
    assert!(shell.tx.take().is_some());
    assert!(shell.rx.take().is_some());

    let console = Arc::new(Mutex::new(EchoConsole::default()));
    let mut shell = echo_shell(&console).await;
    console.lock().unwrap().block = Some(Gate::BodyWrite);
    cancel_at_gate(&mut shell, &console).await;
    shell.cmd_without_reply("booti").await.unwrap();
    assert_eq!(console.lock().unwrap().writes, b"\x03booti\n");
    assert_eq!(console.lock().unwrap().aborted_lines, 1);
}
