//! Crate-private runner execution summaries.
//!
//! Runners still expose `anyhow::Result<()>`; this module keeps the internal
//! success/failure mapping reusable for future system-test aggregation.

use std::{process::ExitStatus, time::Duration};

use crate::run::output_matcher::StreamMatch;

#[derive(Debug)]
pub(crate) enum RunnerExitStatus {
    Process(ExitStatus),
    NotAvailable,
}

impl RunnerExitStatus {
    pub(crate) fn process(status: ExitStatus) -> Self {
        Self::Process(status)
    }

    pub(crate) fn not_available() -> Self {
        Self::NotAvailable
    }
}

#[derive(Debug)]
pub(crate) struct RunnerExecutionSummary {
    runner: &'static str,
    exit_status: RunnerExitStatus,
    stream_match: Option<StreamMatch>,
    terminal_error: Option<anyhow::Error>,
    stderr_log: Option<String>,
    elapsed: Duration,
}

impl RunnerExecutionSummary {
    pub(crate) fn new(
        runner: &'static str,
        exit_status: RunnerExitStatus,
        elapsed: Duration,
    ) -> Self {
        Self {
            runner,
            exit_status,
            stream_match: None,
            terminal_error: None,
            stderr_log: None,
            elapsed,
        }
    }

    pub(crate) fn with_stream_match(mut self, stream_match: Option<StreamMatch>) -> Self {
        self.stream_match = stream_match;
        self
    }

    pub(crate) fn with_terminal_error(mut self, terminal_error: Option<anyhow::Error>) -> Self {
        self.terminal_error = terminal_error;
        self
    }

    pub(crate) fn with_stderr_log(mut self, stderr: &[u8]) -> Self {
        self.stderr_log = Some(String::from_utf8_lossy(stderr).into_owned());
        self
    }

    #[cfg(test)]
    fn runner(&self) -> &'static str {
        self.runner
    }

    #[cfg(test)]
    fn elapsed(&self) -> Duration {
        self.elapsed
    }

    pub(crate) fn into_result(self) -> anyhow::Result<()> {
        let Self {
            runner,
            exit_status,
            stream_match,
            terminal_error,
            stderr_log,
            elapsed,
        } = self;
        let _ = (runner, elapsed);

        if let Some(err) = terminal_error {
            return Err(err);
        }

        if let Some(matched) = stream_match {
            matched.kind.into_result(&matched)?;
            return Ok(());
        }

        if let RunnerExitStatus::Process(status) = exit_status
            && !status.success()
        {
            return Err(anyhow::anyhow!("{}", stderr_log.unwrap_or_default()));
        }

        Ok(())
    }
}

pub(crate) fn timeout_duration(timeout: Option<u64>) -> Option<Duration> {
    match timeout {
        Some(0) | None => None,
        Some(secs) => Some(Duration::from_secs(secs)),
    }
}

#[cfg(test)]
mod tests {
    use super::{RunnerExecutionSummary, RunnerExitStatus, timeout_duration};
    use crate::run::output_matcher::{StreamMatch, StreamMatchKind};
    use std::time::{Duration, Instant};

    fn stream_match(kind: StreamMatchKind) -> StreamMatch {
        StreamMatch {
            kind,
            matched_regex: "READY|PANIC".into(),
            matched_text: "kernel READY".into(),
            deadline: Instant::now(),
        }
    }

    #[test]
    fn timeout_zero_and_none_disable_timeout() {
        assert_eq!(timeout_duration(None), None);
        assert_eq!(timeout_duration(Some(0)), None);
        assert_eq!(timeout_duration(Some(5)), Some(Duration::from_secs(5)));
    }

    #[test]
    fn summary_preserves_success_match_as_ok() {
        let summary = RunnerExecutionSummary::new(
            "test runner",
            RunnerExitStatus::not_available(),
            Duration::from_millis(7),
        )
        .with_stream_match(Some(stream_match(StreamMatchKind::Success)));

        assert_eq!(summary.runner(), "test runner");
        assert_eq!(summary.elapsed(), Duration::from_millis(7));
        summary.into_result().unwrap();
    }

    #[test]
    fn summary_preserves_fail_match_error() {
        let err = RunnerExecutionSummary::new(
            "test runner",
            RunnerExitStatus::not_available(),
            Duration::ZERO,
        )
        .with_stream_match(Some(stream_match(StreamMatchKind::Fail)))
        .into_result()
        .unwrap_err();

        assert!(err.to_string().contains("Fail pattern matched"));
    }

    #[test]
    fn summary_returns_terminal_error_before_match_result() {
        let err = RunnerExecutionSummary::new(
            "test runner",
            RunnerExitStatus::not_available(),
            Duration::ZERO,
        )
        .with_terminal_error(Some(anyhow::anyhow!("terminal timed out")))
        .with_stream_match(Some(stream_match(StreamMatchKind::Success)))
        .into_result()
        .unwrap_err();

        assert_eq!(err.to_string(), "terminal timed out");
    }

    #[cfg(unix)]
    #[test]
    fn summary_maps_nonzero_process_status_to_stderr_log() {
        use std::os::unix::process::ExitStatusExt;

        let err = RunnerExecutionSummary::new(
            "test runner",
            RunnerExitStatus::process(ExitStatusExt::from_raw(1 << 8)),
            Duration::ZERO,
        )
        .with_stderr_log(b"boot failed")
        .into_result()
        .unwrap_err();

        assert_eq!(err.to_string(), "boot failed");
    }
}
