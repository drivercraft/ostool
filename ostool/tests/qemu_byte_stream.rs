use std::{
    io::{ErrorKind, Read},
    net::TcpStream,
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU32, Ordering},
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, bail};
use regex::Regex;

static PORT: AtomicU32 = AtomicU32::new(11000);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MatchKind {
    Success,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MatchOutcome {
    kind: MatchKind,
    matched_regex: String,
    matched_text: String,
    tail_bytes: usize,
}

struct ByteStreamMatcher {
    success_regex: Vec<Regex>,
    fail_regex: Vec<Regex>,
    history: Vec<u8>,
    outcome: Option<MatchOutcome>,
    tail_deadline: Option<Instant>,
}

impl ByteStreamMatcher {
    fn new(success_patterns: &[&str], fail_patterns: &[&str]) -> Result<Self> {
        let mut success_regex = Vec::with_capacity(success_patterns.len());
        for pattern in success_patterns {
            success_regex.push(
                Regex::new(pattern)
                    .with_context(|| format!("failed to compile success regex: {pattern}"))?,
            );
        }

        let mut fail_regex = Vec::with_capacity(fail_patterns.len());
        for pattern in fail_patterns {
            fail_regex.push(
                Regex::new(pattern)
                    .with_context(|| format!("failed to compile fail regex: {pattern}"))?,
            );
        }

        Ok(Self {
            success_regex,
            fail_regex,
            history: Vec::with_capacity(1024),
            outcome: None,
            tail_deadline: None,
        })
    }

    fn is_matched(&self) -> bool {
        self.outcome.is_some()
    }

    fn tail_deadline(&self) -> Option<Instant> {
        self.tail_deadline
    }

    fn outcome(&self) -> Option<&MatchOutcome> {
        self.outcome.as_ref()
    }

    fn feed(&mut self, byte: u8, now: Instant) -> Option<MatchOutcome> {
        if self.outcome.is_some() {
            if byte == b'\n' {
                self.history.clear();
            }
            return None;
        }

        self.history.push(byte);
        let lossy = String::from_utf8_lossy(&self.history);

        if let Some(matched_regex) = self
            .fail_regex
            .iter()
            .find(|regex| regex.is_match(&lossy))
            .map(|regex| regex.as_str().to_string())
        {
            return Some(self.finish(MatchKind::Fail, &matched_regex, lossy.into_owned(), now));
        }

        if let Some(matched_regex) = self
            .success_regex
            .iter()
            .find(|regex| regex.is_match(&lossy))
            .map(|regex| regex.as_str().to_string())
        {
            return Some(self.finish(MatchKind::Success, &matched_regex, lossy.into_owned(), now));
        }

        if byte == b'\n' {
            self.history.clear();
        }

        None
    }

    fn finish(
        &mut self,
        kind: MatchKind,
        matched_regex: &str,
        matched_text: String,
        now: Instant,
    ) -> MatchOutcome {
        let outcome = MatchOutcome {
            kind,
            matched_regex: matched_regex.to_string(),
            matched_text,
            tail_bytes: 0,
        };
        self.tail_deadline = Some(now + Duration::from_millis(500));
        self.outcome = Some(outcome.clone());
        outcome
    }
}

struct QemuGuard(Option<Child>);

impl QemuGuard {
    fn shutdown(mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for QemuGuard {
    fn drop(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

fn qemu_binary() -> &'static str {
    "qemu-system-aarch64"
}

fn uboot_bin() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../assets/u-boot.bin")
}

fn spawn_uboot_qemu() -> Result<(QemuGuard, TcpStream)> {
    let port = PORT.fetch_add(1, Ordering::SeqCst);
    let bin = uboot_bin();

    let child = Command::new(qemu_binary())
        .arg("-serial")
        .arg(format!("tcp::{port},server,nowait"))
        .args([
            "-machine",
            "virt",
            "-cpu",
            "cortex-a57",
            "-nographic",
            "-bios",
            bin.to_str()
                .context("u-boot.bin path contains invalid UTF-8")?,
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|err| {
            if err.kind() == ErrorKind::NotFound {
                anyhow::anyhow!("qemu-system-aarch64 is not installed")
            } else {
                err.into()
            }
        })?;

    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Ok(stream) = TcpStream::connect(("127.0.0.1", port as u16)) {
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .context("failed to set read timeout")?;
            return Ok((QemuGuard(Some(child)), stream));
        }

        if Instant::now() >= deadline {
            return Err(anyhow::anyhow!(
                "timed out waiting for QEMU serial port on {port}"
            ));
        }

        thread::sleep(Duration::from_millis(100));
    }
}

fn run_case(success_patterns: &[&str], fail_patterns: &[&str]) -> Result<Option<MatchOutcome>> {
    let (guard, mut stream) = match spawn_uboot_qemu() {
        Ok(pair) => pair,
        Err(err) if err.to_string().contains("not installed") => {
            eprintln!("skipping qemu-backed test: {err}");
            return Ok(None);
        }
        Err(err) => return Err(err),
    };

    let mut matcher = ByteStreamMatcher::new(success_patterns, fail_patterns)?;
    let mut buffer = [0u8; 512];
    let overall_deadline = Instant::now() + Duration::from_secs(15);
    let mut tail_bytes = 0usize;

    loop {
        if Instant::now() >= overall_deadline {
            bail!("timed out waiting for matcher outcome");
        }

        let timeout = if let Some(deadline) = matcher.tail_deadline() {
            deadline.saturating_duration_since(Instant::now())
        } else {
            Duration::from_millis(100)
        };
        stream
            .set_read_timeout(Some(timeout.max(Duration::from_millis(10))))
            .context("failed to update read timeout")?;

        match stream.read(&mut buffer) {
            Ok(0) => {
                if let Some(outcome) = matcher.outcome().cloned() {
                    let mut outcome = outcome;
                    outcome.tail_bytes = tail_bytes;
                    guard.shutdown();
                    return Ok(Some(outcome));
                }
            }
            Ok(n) => {
                for &byte in &buffer[..n] {
                    let already_matched = matcher.is_matched();
                    let _ = matcher.feed(byte, Instant::now());
                    if already_matched {
                        tail_bytes += 1;
                    }
                }

                if let Some(outcome) = matcher.outcome().cloned()
                    && let Some(deadline) = matcher.tail_deadline()
                    && Instant::now() >= deadline
                {
                    let mut outcome = outcome;
                    outcome.tail_bytes = tail_bytes;
                    guard.shutdown();
                    return Ok(Some(outcome));
                }
            }
            Err(err)
                if err.kind() == ErrorKind::TimedOut || err.kind() == ErrorKind::WouldBlock =>
            {
                if let Some(outcome) = matcher.outcome().cloned()
                    && let Some(deadline) = matcher.tail_deadline()
                    && Instant::now() >= deadline
                {
                    let mut outcome = outcome;
                    outcome.tail_bytes = tail_bytes;
                    guard.shutdown();
                    return Ok(Some(outcome));
                }
            }
            Err(err) => return Err(err.into()),
        }
    }
}

#[test]
fn qemu_byte_stream_success_matches_before_newline() -> Result<()> {
    let Some(outcome) = run_case(
        &[r"Hit any key to stop autoboot:"],
        &[r"__ostool_never_fail__"],
    )?
    else {
        return Ok(());
    };

    assert_eq!(outcome.kind, MatchKind::Success);
    assert_eq!(outcome.matched_regex, r"Hit any key to stop autoboot:");
    assert!(
        outcome
            .matched_text
            .contains("Hit any key to stop autoboot")
    );
    assert!(
        outcome.tail_bytes > 0,
        "expected tail drain bytes after success"
    );
    Ok(())
}

#[test]
fn qemu_byte_stream_fail_matches_before_newline() -> Result<()> {
    let Some(outcome) = run_case(&[r"__ostool_never_success__"], &[r"Net:\s+eth0:"])? else {
        return Ok(());
    };

    assert_eq!(outcome.kind, MatchKind::Fail);
    assert_eq!(outcome.matched_regex, r"Net:\s+eth0:");
    assert!(outcome.matched_text.contains("Net:"));
    assert!(
        outcome.tail_bytes > 0,
        "expected tail drain bytes after fail"
    );
    Ok(())
}

#[test]
fn qemu_byte_stream_fail_wins_when_both_match() -> Result<()> {
    let Some(outcome) = run_case(
        &[r"Hit any key to stop autoboot:"],
        &[r"Hit any key to stop autoboot:"],
    )?
    else {
        return Ok(());
    };

    assert_eq!(outcome.kind, MatchKind::Fail);
    assert_eq!(outcome.matched_regex, r"Hit any key to stop autoboot:");
    assert!(
        outcome
            .matched_text
            .contains("Hit any key to stop autoboot")
    );
    assert!(
        outcome.tail_bytes > 0,
        "expected tail drain bytes after fail"
    );
    Ok(())
}
