//! Ordinary command submission, echo validation and cancellation ownership.

use std::{
    future::Future,
    io::{Error, ErrorKind, Result},
    time::{Duration, Instant},
};

use futures::{
    AsyncReadExt, AsyncWriteExt,
    future::{Either, select},
    pin_mut,
};
use futures_timer::Delay;

use crate::{CTRL_C, INT, UbootShell};

const ECHO_TIMEOUT: Duration = Duration::from_secs(5);
const SEND_TIMEOUT: Duration = Duration::from_secs(30);
const RECOVERY_TIMEOUT: Duration = Duration::from_secs(5);
const IDLE_TIMEOUT: Duration = Duration::from_millis(300);
const MAX_ATTEMPTS: usize = 3;
const OK: &str = "cmd-ok";
const END: &str = "cmd-end";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum CommandPhase {
    #[default]
    Ready,
    // Set before the first body write, retained if the future is dropped.
    Dirty,
    // Set before any potentially executing write; only a complete result clears it.
    Uncertain,
}

enum SendFailure {
    Echo(Error),
    Transport(Error),
    EarlyExecution(Error),
}

pub(super) async fn bounded<T>(
    limit: Duration,
    stage: &str,
    operation: impl Future<Output = Result<T>>,
) -> Result<T> {
    if limit.is_zero() {
        return Err(Error::new(
            ErrorKind::TimedOut,
            format!("U-Boot {stage} timed out"),
        ));
    }
    let deadline = Delay::new(limit);
    pin_mut!(operation, deadline);
    match select(operation, deadline).await {
        Either::Left((result, _)) => result,
        Either::Right(_) => Err(Error::new(
            ErrorKind::TimedOut,
            format!("U-Boot {stage} timed out"),
        )),
    }
}

fn stage_error(stage: &str, err: Error) -> Error {
    Error::new(
        err.kind(),
        format!("U-Boot {stage}; execution uncertain: {err}"),
    )
}

impl UbootShell {
    pub(super) async fn admit_command(&mut self) -> Result<()> {
        match self.command_phase {
            CommandPhase::Ready => Ok(()),
            CommandPhase::Dirty => self.recover_command_prompt().await,
            CommandPhase::Uncertain => Err(Error::other(
                "U-Boot execution uncertain; establish a new shell session before submitting \
                 commands",
            )),
        }
    }

    /// Writes a command and newline without consuming its response.
    pub async fn cmd_without_reply(&mut self, cmd: &str) -> Result<()> {
        self.admit_command().await?;
        self.write_raw_command(cmd).await?;
        // Successful raw I/O preserves the caller-owned synchronization
        // contract. It need not return to U-Boot (e.g. a kernel boot).
        self.command_phase = CommandPhase::Ready;
        Ok(())
    }

    pub(super) async fn write_raw_command(&mut self, cmd: &str) -> Result<()> {
        // Raw commands may boot a kernel and never return a prompt. Taking the
        // streams still works; ordinary commands must not assume synchronization.
        self.command_phase = CommandPhase::Uncertain;
        self.tx().write_all(cmd.as_bytes()).await?;
        self.tx().write_all(b"\n").await?;
        self.tx().flush().await
    }

    /// Runs a single-line command, retrying only recoverable pre-submission echo failures.
    pub async fn cmd(&mut self, cmd: &str) -> Result<String> {
        if cmd.bytes().any(|byte| byte.is_ascii_control()) {
            return Err(Error::new(
                ErrorKind::InvalidInput,
                "U-Boot command must be a single line without control bytes",
            ));
        }
        self.admit_command().await?;
        self.clear_shell().await?;
        info!("cmd: {cmd}");
        // A separate completion line distinguishes a fully observed failure
        // from missing/corrupt success output. It does not make replay safe.
        let wire_command = format!("{cmd}&& echo {OK}; echo {END}");
        for attempt in 1..=MAX_ATTEMPTS {
            let started = Instant::now();
            self.command_phase = CommandPhase::Dirty;
            match self.send_command_body(&wire_command, started).await {
                Ok(()) => {}
                Err(SendFailure::Echo(err)) => {
                    warn!("U-Boot command echo failed on attempt {attempt}: {err}");
                    self.recover_command_prompt().await.map_err(|recovery| {
                        Error::new(
                            recovery.kind(),
                            format!(
                                "U-Boot pre-submission recovery failed: {recovery}; echo error: \
                                 {err}"
                            ),
                        )
                    })?;
                    if attempt == MAX_ATTEMPTS {
                        return Err(err);
                    }
                    continue;
                }
                Err(SendFailure::Transport(err)) => {
                    self.command_phase = CommandPhase::Uncertain;
                    return Err(stage_error("body transport failed", err));
                }
                Err(SendFailure::EarlyExecution(err)) => {
                    self.command_phase = CommandPhase::Uncertain;
                    return Err(stage_error("early line ending during echo", err));
                }
            }
            // Must precede even polling write_all: a failing/cancelled write may
            // already have delivered the newline to the target.
            self.command_phase = CommandPhase::Uncertain;
            bounded(
                SEND_TIMEOUT.saturating_sub(started.elapsed()),
                "newline submission",
                async {
                    self.tx().write_all(b"\n").await?;
                    self.tx().flush().await
                },
            )
            .await
            .map_err(|err| stage_error("newline submission failed", err))?;
            return self.command_result().await;
        }
        unreachable!("MAX_ATTEMPTS is nonzero")
    }

    async fn send_command_body(
        &mut self,
        command: &str,
        started: Instant,
    ) -> std::result::Result<(), SendFailure> {
        for (offset, expected) in command.bytes().enumerate() {
            bounded(
                SEND_TIMEOUT.saturating_sub(started.elapsed()),
                "body write/flush",
                async {
                    self.tx().write_all(&[expected]).await?;
                    self.tx().flush().await
                },
            )
            .await
            .map_err(SendFailure::Transport)?;
            let limit = ECHO_TIMEOUT.min(SEND_TIMEOUT.saturating_sub(started.elapsed()));
            bounded(limit, "command echo", async {
                Ok(self
                    .expect_echo(offset, expected, Instant::now() + limit)
                    .await)
            })
            .await
            .map_err(SendFailure::Echo)??;
        }
        Ok(())
    }

    async fn expect_echo(
        &mut self,
        offset: usize,
        expected: u8,
        deadline: Instant,
    ) -> std::result::Result<(), SendFailure> {
        // ECMA-48 CSI grammar: parameters, intermediates, then one final byte.
        enum Csi {
            Ordinary,
            Escape,
            Parameters,
            Intermediates,
        }
        let mut state = Csi::Ordinary;
        loop {
            // A continuously ready stream must not starve the timer's poll.
            if Instant::now() >= deadline {
                return Err(SendFailure::Echo(Error::new(
                    ErrorKind::TimedOut,
                    format!(
                        "U-Boot command echo timed out at offset {offset}, expected \
                         0x{expected:02x}"
                    ),
                )));
            }
            let mut byte = [0];
            self.rx().read_exact(&mut byte).await.map_err(|err| {
                if err.kind() == ErrorKind::TimedOut {
                    SendFailure::Echo(err)
                } else {
                    SendFailure::Transport(err)
                }
            })?;
            let actual = byte[0];
            if matches!(actual, b'\r' | b'\n') {
                self.command_phase = CommandPhase::Uncertain;
                return Err(SendFailure::EarlyExecution(echo_mismatch(
                    offset, expected, actual,
                )));
            }
            state = match state {
                Csi::Ordinary if actual == expected => return Ok(()),
                Csi::Ordinary if actual == 0x1b => Csi::Escape,
                Csi::Escape if actual == b'[' => Csi::Parameters,
                Csi::Parameters if (0x30..=0x3f).contains(&actual) => Csi::Parameters,
                Csi::Parameters | Csi::Intermediates if (0x20..=0x2f).contains(&actual) => {
                    Csi::Intermediates
                }
                Csi::Parameters | Csi::Intermediates if (0x40..=0x7e).contains(&actual) => {
                    Csi::Ordinary
                }
                _ => return Err(SendFailure::Echo(echo_mismatch(offset, expected, actual))),
            };
        }
    }

    async fn command_result(&mut self) -> Result<String> {
        let reply = self
            .read_prompt_reply()
            .await
            .map_err(|err| stage_error("command reply failed", err))?;
        let mut lines: Vec<_> = reply
            .lines()
            .map(|line| line.trim_end_matches('\r'))
            .collect();
        if lines.pop() != Some(END) {
            return Err(Error::new(
                ErrorKind::InvalidData,
                "U-Boot command completion marker missing or corrupt; execution uncertain",
            ));
        }
        self.command_phase = CommandPhase::Ready;
        if lines.pop() != Some(OK) {
            return Err(Error::other(
                "U-Boot command failed; complete failure result received (not replayed)",
            ));
        }
        Ok(lines.join("\n").trim().to_string())
    }

    pub(super) async fn read_prompt_reply(&mut self) -> Result<String> {
        let mut reply = Vec::new();
        let prompt = self.perfix.as_bytes().to_vec();
        loop {
            reply.push(self.read_command_byte(ECHO_TIMEOUT).await?);
            if reply.ends_with(&prompt) {
                let start = reply.len() - prompt.len();
                if start == 0 || reply[start - 1] == b'\n' {
                    reply.truncate(start);
                    return String::from_utf8(reply)
                        .map(|reply| reply.trim().to_string())
                        .map_err(|err| Error::new(ErrorKind::InvalidData, err));
                }
            }
        }
    }

    async fn read_command_byte(&mut self, limit: Duration) -> Result<u8> {
        let mut byte = [0];
        bounded(limit, "command receive", self.rx().read_exact(&mut byte)).await?;
        Ok(byte[0])
    }

    async fn recover_command_prompt(&mut self) -> Result<()> {
        self.command_phase = CommandPhase::Dirty;
        let deadline = Instant::now() + RECOVERY_TIMEOUT;
        bounded(
            deadline.saturating_duration_since(Instant::now()),
            "command recovery",
            async {
                let mut repeated_interrupt = false;
                loop {
                    // Never accept a queued old marker/prompt as evidence for
                    // the new interrupt. All rounds share the outer budget.
                    self.drain_recovery_input(deadline).await?;
                    self.tx().write_all(&[CTRL_C]).await?;
                    self.tx().flush().await?;
                    match self.read_interrupt_handshake(deadline).await {
                        Ok(()) => {
                            if repeated_interrupt {
                                // Repeated interrupts may leave extra responses
                                // queued behind the handshake.
                                self.drain_recovery_input(deadline).await?;
                            }
                            return Ok(());
                        }
                        Err(err) if err.kind() == ErrorKind::TimedOut => {
                            repeated_interrupt = true;
                        }
                        Err(err) => return Err(err),
                    }
                }
            },
        )
        .await?;
        self.command_phase = CommandPhase::Ready;
        Ok(())
    }

    async fn drain_recovery_input(&mut self, deadline: Instant) -> Result<()> {
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                return Err(Error::new(
                    ErrorKind::TimedOut,
                    "U-Boot command recovery timed out while draining",
                ));
            }
            match self.read_command_byte(IDLE_TIMEOUT.min(remaining)).await {
                Ok(_) => {}
                Err(err) if err.kind() == ErrorKind::TimedOut && remaining >= IDLE_TIMEOUT => {
                    return Ok(());
                }
                Err(err) => return Err(err),
            }
        }
    }

    async fn read_interrupt_handshake(&mut self, deadline: Instant) -> Result<()> {
        let prefixed = [self.perfix.as_bytes(), INT].concat();
        let mut line = Vec::new();
        let mut overlong = false;
        loop {
            let byte = self
                .read_command_byte(
                    IDLE_TIMEOUT.min(deadline.saturating_duration_since(Instant::now())),
                )
                .await?;
            if byte == b'\n' {
                if line.last() == Some(&b'\r') {
                    line.pop();
                }
                if !overlong && (line == INT || line == prefixed) {
                    break;
                }
                line.clear();
                overlong = false;
            } else if line.len() <= prefixed.len() + 1 {
                line.push(byte);
            } else {
                overlong = true;
            }
        }
        for expected in self.perfix.clone().bytes() {
            if self
                .read_command_byte(
                    IDLE_TIMEOUT.min(deadline.saturating_duration_since(Instant::now())),
                )
                .await?
                != expected
            {
                return Err(Error::new(
                    ErrorKind::InvalidData,
                    "U-Boot recovery prompt must immediately follow the interrupt line",
                ));
            }
        }
        Ok(())
    }
}

fn echo_mismatch(offset: usize, expected: u8, actual: u8) -> Error {
    Error::new(
        ErrorKind::InvalidData,
        format!(
            "U-Boot echo mismatch at offset {offset}: expected 0x{expected:02x}, actual \
             0x{actual:02x}"
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn expired_send_budget_cannot_start_another_write() {
        // Deliberately absent streams: touching either would panic. This models
        // the deadline exhausted by earlier characters in the SAME attempt.
        let mut shell = UbootShell {
            tx: None,
            rx: None,
            perfix: "=> ".into(),
            command_phase: CommandPhase::Dirty,
        };
        match shell
            .send_command_body("remaining suffix", Instant::now() - SEND_TIMEOUT)
            .await
        {
            Err(SendFailure::Transport(error)) => assert_eq!(error.kind(), ErrorKind::TimedOut),
            _ => panic!("expired attempt started sending another character"),
        }
    }
}
