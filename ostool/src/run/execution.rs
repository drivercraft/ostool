//! Crate-private runner execution summaries.
//!
//! Runners still expose `anyhow::Result<()>`; this module keeps the internal
//! success/failure mapping reusable for future system-test aggregation.

use std::{process::ExitStatus, time::Duration};

use crate::run::output_matcher::FailMatch;

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
    fail_match: Option<FailMatch>,
    terminal_error: Option<anyhow::Error>,
    shell_check_error: Option<anyhow::Error>,
    shell_check_completed: bool,
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
            fail_match: None,
            terminal_error: None,
            shell_check_error: None,
            shell_check_completed: false,
            stderr_log: None,
            elapsed,
        }
    }

    pub(crate) fn with_fail_match(mut self, fail_match: Option<FailMatch>) -> Self {
        self.fail_match = fail_match;
        self
    }

    pub(crate) fn with_terminal_error(mut self, terminal_error: Option<anyhow::Error>) -> Self {
        self.terminal_error = terminal_error;
        self
    }

    pub(crate) fn with_shell_check_completed(mut self, completed: bool) -> Self {
        self.shell_check_completed = completed;
        self
    }

    pub(crate) fn with_shell_check_error(mut self, error: Option<anyhow::Error>) -> Self {
        self.shell_check_error = error;
        self
    }

    pub(crate) fn with_stderr_log(mut self, stderr: &[u8]) -> Self {
        self.stderr_log = Some(String::from_utf8_lossy(stderr).into_owned());
        self
    }

    pub(crate) fn into_result(self) -> anyhow::Result<()> {
        let Self {
            runner,
            exit_status,
            fail_match,
            terminal_error,
            shell_check_error,
            shell_check_completed,
            stderr_log,
            elapsed,
        } = self;
        let _ = (runner, elapsed);

        if let Some(err) = terminal_error {
            return Err(err);
        }

        if let Some(matched) = fail_match {
            return Err(matched.into_error());
        }

        if let Some(error) = shell_check_error {
            return Err(error);
        }

        if shell_check_completed {
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
    use crate::run::output_matcher::FailMatch;
    use std::time::{Duration, Instant};

    fn fail_match() -> FailMatch {
        FailMatch {
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
    fn summary_preserves_fail_match_error() {
        let err = RunnerExecutionSummary::new(
            "test runner",
            RunnerExitStatus::not_available(),
            Duration::ZERO,
        )
        .with_fail_match(Some(fail_match()))
        .into_result()
        .unwrap_err();

        assert!(err.to_string().contains("Fail pattern matched"));
    }

    #[test]
    fn summary_returns_terminal_error_before_fail_match() {
        let err = RunnerExecutionSummary::new(
            "test runner",
            RunnerExitStatus::not_available(),
            Duration::ZERO,
        )
        .with_terminal_error(Some(anyhow::anyhow!("terminal timed out")))
        .with_fail_match(Some(fail_match()))
        .into_result()
        .unwrap_err();

        assert_eq!(err.to_string(), "terminal timed out");
    }

    #[test]
    fn summary_returns_global_fail_before_shell_check_error() {
        let err = RunnerExecutionSummary::new(
            "test runner",
            RunnerExitStatus::not_available(),
            Duration::ZERO,
        )
        .with_fail_match(Some(fail_match()))
        .with_shell_check_error(Some(anyhow::anyhow!("shell step failed")))
        .into_result()
        .unwrap_err();

        assert!(err.to_string().contains("Fail pattern matched"));
    }

    #[test]
    fn summary_returns_global_fail_before_completed_shell_check() {
        let err = RunnerExecutionSummary::new(
            "test runner",
            RunnerExitStatus::not_available(),
            Duration::ZERO,
        )
        .with_fail_match(Some(fail_match()))
        .with_shell_check_completed(true)
        .into_result()
        .unwrap_err();

        assert!(err.to_string().contains("Fail pattern matched"));
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
