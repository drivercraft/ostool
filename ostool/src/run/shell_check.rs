use std::{
    sync::{Arc, Mutex},
    time::Duration,
};

use anyhow::{Result, bail};
use regex::Regex;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::project::variables::{self, VariableScope};
use crate::sterm::{TerminalHandle, WeakTerminalHandle};

pub(crate) const SHELL_CHECK_DELAY: Duration = Duration::from_millis(100);
pub(crate) const SHELL_CHECK_CHUNK_SIZE: usize = 64;
pub(crate) const SHELL_CHECK_CHUNK_DELAY: Duration = Duration::from_millis(2);
const SHELL_CHECK_PENDING_OUTPUT_LIMIT: usize = 16 * 1024 * 1024;

pub(crate) fn normalize_shell_check_steps(
    steps: &mut [ShellCheckStep],
    config_name: &str,
) -> Result<Vec<ResolvedShellCheckStep>> {
    resolve_shell_check_steps(steps, config_name)
}

fn normalize_optional_field(value: &mut Option<String>) {
    if let Some(raw) = value {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            *value = None;
        } else if trimmed.len() != raw.len() {
            *raw = trimmed.to_string();
        }
    }
}

pub(crate) fn prepare_shell_cmd(command: &str) -> Vec<u8> {
    let mut normalized = command.trim_end_matches(['\r', '\n']).as_bytes().to_vec();
    normalized.push(b'\n');
    normalized
}

/// One ordered shell command and its optional result conditions.
///
/// This is passive configuration data. Runners validate prefix inheritance
/// and result conditions before starting terminal interaction.
#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct ShellCheckStep {
    /// Shell prompt to wait for. Later steps may omit it to inherit the previous prefix.
    pub shell_prefix: Option<String>,
    /// Optional command written after the shell prefix is observed.
    #[serde(default)]
    pub shell_cmd: Option<String>,
    /// Regex patterns where any match completes the step.
    pub success_regex: Option<Vec<String>>,
    /// Regex patterns that fail the step. Requires `success_regex`.
    pub fail_regex: Option<Vec<String>>,
    /// Per-step timeout in seconds, starting after the command is flushed.
    /// Passive steps without a command must use the runner's overall timeout.
    pub timeout: Option<u64>,
}

impl ShellCheckStep {
    pub(crate) fn replace_strings(&mut self, scope: &VariableScope) -> Result<()> {
        self.shell_prefix = self
            .shell_prefix
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.shell_cmd = self
            .shell_cmd
            .as_deref()
            .map(|value| variables::expand_variables(value, scope))
            .transpose()?;
        self.success_regex = expand_patterns(self.success_regex.as_deref(), scope)?;
        self.fail_regex = expand_patterns(self.fail_regex.as_deref(), scope)?;
        Ok(())
    }
}

fn expand_patterns(
    patterns: Option<&[String]>,
    scope: &VariableScope,
) -> Result<Option<Vec<String>>> {
    patterns
        .map(|patterns| {
            patterns
                .iter()
                .map(|pattern| variables::expand_variables(pattern, scope))
                .collect::<Result<Vec<_>>>()
        })
        .transpose()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ResolvedShellCheckStep {
    pub(crate) shell_prefix: Option<String>,
    pub(crate) shell_cmd: Option<String>,
    pub(crate) success_regex: Option<Vec<String>>,
    pub(crate) fail_regex: Option<Vec<String>>,
    pub(crate) timeout: Option<u64>,
}

fn resolve_shell_check_steps(
    steps: &mut [ShellCheckStep],
    config_name: &str,
) -> Result<Vec<ResolvedShellCheckStep>> {
    let mut inherited_prefix = None;
    let mut resolved = Vec::with_capacity(steps.len());
    for (index, step) in steps.iter_mut().enumerate() {
        if step
            .shell_prefix
            .as_ref()
            .is_some_and(|prefix| prefix.trim().is_empty())
        {
            bail!(
                "`shell_check_steps[{index}].shell_prefix` must not be empty in {config_name}; \
                 omit it to inherit the previous prefix"
            );
        }
        normalize_optional_field(&mut step.shell_prefix);
        if step
            .shell_cmd
            .as_ref()
            .is_some_and(|command| command.trim().is_empty())
        {
            bail!("`shell_check_steps[{index}].shell_cmd` must not be empty in {config_name}");
        }

        let shell_prefix = match step.shell_prefix.as_ref() {
            Some(prefix) => {
                inherited_prefix = Some(prefix.clone());
                Some(prefix.clone())
            }
            None => inherited_prefix.clone(),
        };
        if step.shell_cmd.is_some() && shell_prefix.is_none() {
            bail!(
                "`shell_check_steps[{index}].shell_prefix` is required when `shell_cmd` is set in {config_name}"
            );
        }
        normalize_patterns(&mut step.success_regex, "success_regex", index, config_name)?;
        normalize_patterns(&mut step.fail_regex, "fail_regex", index, config_name)?;
        if step.shell_cmd.is_none() && step.success_regex.is_none() && step.fail_regex.is_none() {
            bail!(
                "`shell_check_steps[{index}]` without `shell_cmd` must define a success or fail \
                 condition in {config_name}"
            );
        }
        if step.success_regex.is_none() && step.fail_regex.is_some() {
            bail!(
                "`shell_check_steps[{index}]` defines `fail_regex` but has no `success_regex` \
                 completion condition in {config_name}"
            );
        }
        if step.timeout == Some(0) {
            bail!(
                "`shell_check_steps[{index}].timeout` must be greater than zero in {config_name}"
            );
        }
        if step.shell_cmd.is_none() && step.timeout.is_some() {
            bail!(
                "`shell_check_steps[{index}].timeout` is not supported on a passive step without \
                 `shell_cmd` in {config_name}; use the top-level timeout"
            );
        }
        compile_step_patterns(step, index, config_name)?;
        resolved.push(ResolvedShellCheckStep {
            shell_prefix,
            shell_cmd: step.shell_cmd.clone(),
            success_regex: step.success_regex.clone(),
            fail_regex: step.fail_regex.clone(),
            timeout: step.timeout,
        });
    }
    Ok(resolved)
}

fn normalize_patterns(
    patterns: &mut Option<Vec<String>>,
    field: &str,
    index: usize,
    config_name: &str,
) -> Result<()> {
    let Some(patterns) = patterns else {
        return Ok(());
    };
    for pattern in patterns.iter_mut() {
        *pattern = pattern.trim().to_string();
        if pattern.is_empty() {
            bail!(
                "`shell_check_steps[{index}].{field}` contains an empty pattern in {config_name}"
            );
        }
    }
    if patterns.is_empty() {
        bail!("`shell_check_steps[{index}].{field}` must not be an empty array in {config_name}");
    }
    Ok(())
}

fn compile_step_patterns(step: &ShellCheckStep, index: usize, config_name: &str) -> Result<()> {
    for (field, patterns) in [
        ("success_regex", step.success_regex.as_deref()),
        ("fail_regex", step.fail_regex.as_deref()),
    ] {
        for pattern in patterns.unwrap_or_default() {
            Regex::new(pattern).map_err(|error| {
                anyhow::anyhow!(
                    "invalid `shell_check_steps[{index}].{field}` in {config_name}: {error}"
                )
            })?;
        }
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ShellCheckEvent {
    None,
    Send(Vec<u8>),
    StepCompleted(usize),
    SequenceCompleted,
    Failed {
        step: usize,
        pattern: String,
    },
    PendingOutputLimitExceeded {
        step: usize,
        buffered: usize,
        limit: usize,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ShellCheckPhase {
    WaitingForPrefix,
    Sending,
    WaitingForResult,
    Completed,
}

#[derive(Debug, Clone)]
struct RuntimeShellCheckStep {
    config: ResolvedShellCheckStep,
    success_regex: Vec<Regex>,
    fail_regex: Vec<Regex>,
}

#[derive(Debug, Clone)]
pub(crate) struct ShellCheckMatcher {
    steps: Vec<RuntimeShellCheckStep>,
    current_step: usize,
    phase: ShellCheckPhase,
    history: Vec<u8>,
    incomplete_utf8: Vec<u8>,
    pending_output: Vec<u8>,
    sequence_completion_is_success: bool,
}

impl ShellCheckMatcher {
    pub(crate) fn from_steps(steps: Vec<ResolvedShellCheckStep>) -> Result<Self> {
        Self::build(steps)
    }

    fn build(steps: Vec<ResolvedShellCheckStep>) -> Result<Self> {
        if steps.is_empty() {
            bail!("shell-check sequence must contain at least one step");
        }
        let steps = steps
            .into_iter()
            .map(|config| {
                let success_regex = compile_patterns(config.success_regex.as_deref())?;
                let fail_regex = compile_patterns(config.fail_regex.as_deref())?;
                Ok(RuntimeShellCheckStep {
                    config,
                    success_regex,
                    fail_regex,
                })
            })
            .collect::<Result<Vec<_>>>()?;
        let prefix_capacity = steps[0]
            .config
            .shell_prefix
            .as_ref()
            .map_or(64, |prefix| prefix.len().max(64));
        let initial_phase = if steps[0].config.shell_cmd.is_some() {
            ShellCheckPhase::WaitingForPrefix
        } else {
            ShellCheckPhase::WaitingForResult
        };
        Ok(Self {
            steps,
            current_step: 0,
            phase: initial_phase,
            history: Vec::with_capacity(prefix_capacity),
            incomplete_utf8: Vec::with_capacity(3),
            pending_output: Vec::new(),
            sequence_completion_is_success: true,
        })
    }

    #[cfg(test)]
    pub(crate) fn observe_byte(&mut self, byte: u8) -> Option<Vec<u8>> {
        match self.observe_chunk(&[byte]) {
            ShellCheckEvent::Send(command) => Some(command),
            _ => None,
        }
    }

    pub(crate) fn observe_chunk(&mut self, chunk: &[u8]) -> ShellCheckEvent {
        if self.phase == ShellCheckPhase::Completed {
            return ShellCheckEvent::None;
        }
        if self.phase == ShellCheckPhase::Sending {
            return self.append_pending_output(chunk);
        }
        self.process_block(chunk.to_vec())
    }

    pub(crate) fn command_flushed(&mut self) -> ShellCheckEvent {
        if self.phase != ShellCheckPhase::Sending {
            return ShellCheckEvent::None;
        }
        let pending = std::mem::take(&mut self.pending_output);
        let step = &self.steps[self.current_step];
        let event = if step.success_regex.is_empty() && step.fail_regex.is_empty() {
            self.advance()
        } else {
            self.phase = ShellCheckPhase::WaitingForResult;
            ShellCheckEvent::None
        };
        let queued_event = self.process_block(pending);
        if queued_event == ShellCheckEvent::None {
            event
        } else {
            queued_event
        }
    }

    fn process_block(&mut self, mut block: Vec<u8>) -> ShellCheckEvent {
        block = self.normalize_utf8_block(block);
        if !self.history.is_empty() {
            let mut with_history = std::mem::take(&mut self.history);
            with_history.reserve(block.len());
            with_history.extend_from_slice(&block);
            block = with_history;
        }

        let mut cursor = 0;
        let mut transition_event = ShellCheckEvent::None;
        loop {
            if self.phase == ShellCheckPhase::Completed || cursor == block.len() {
                return transition_event;
            }
            match self.phase {
                ShellCheckPhase::WaitingForPrefix => {
                    let text = std::str::from_utf8(&block[cursor..])
                        .expect("shell-check block is normalized to UTF-8");
                    let step = &self.steps[self.current_step];
                    let Some(prefix) = step.config.shell_prefix.as_deref() else {
                        self.phase = ShellCheckPhase::WaitingForResult;
                        continue;
                    };
                    let Some(prefix_start) = text.find(prefix) else {
                        self.retain_boundary_history(&block[cursor..]);
                        return transition_event;
                    };
                    cursor += prefix_start + prefix.len();
                    let Some(command) = step.config.shell_cmd.as_deref() else {
                        self.phase = ShellCheckPhase::WaitingForResult;
                        continue;
                    };
                    let command = prepare_shell_cmd(command);
                    self.phase = ShellCheckPhase::Sending;
                    self.pending_output.clear();
                    if cursor < block.len() {
                        let pending_event = self.append_pending_output(&block[cursor..]);
                        if pending_event != ShellCheckEvent::None {
                            return pending_event;
                        }
                    }
                    return ShellCheckEvent::Send(command);
                }
                ShellCheckPhase::Sending => {
                    return self.append_pending_output(&block[cursor..]);
                }
                ShellCheckPhase::WaitingForResult => {
                    let text = std::str::from_utf8(&block[cursor..])
                        .expect("shell-check block is normalized to UTF-8");
                    let step = &self.steps[self.current_step];
                    if let Some(pattern) = step.fail_regex.iter().find(|regex| regex.is_match(text))
                    {
                        self.phase = ShellCheckPhase::Completed;
                        return ShellCheckEvent::Failed {
                            step: self.current_step,
                            pattern: pattern.as_str().to_string(),
                        };
                    }
                    let Some(success) = step
                        .success_regex
                        .iter()
                        .filter_map(|regex| regex.find(text))
                        .min_by_key(regex::Match::end)
                    else {
                        self.retain_boundary_history(&block[cursor..]);
                        return transition_event;
                    };
                    cursor += success.end();
                    transition_event = self.advance();
                }
                ShellCheckPhase::Completed => return transition_event,
            }
        }
    }

    fn append_pending_output(&mut self, output: &[u8]) -> ShellCheckEvent {
        let buffered = self.pending_output.len().saturating_add(output.len());
        if buffered > SHELL_CHECK_PENDING_OUTPUT_LIMIT {
            self.phase = ShellCheckPhase::Completed;
            return ShellCheckEvent::PendingOutputLimitExceeded {
                step: self.current_step,
                buffered,
                limit: SHELL_CHECK_PENDING_OUTPUT_LIMIT,
            };
        }
        self.pending_output.extend_from_slice(output);
        ShellCheckEvent::None
    }

    fn normalize_utf8_block(&mut self, block: Vec<u8>) -> Vec<u8> {
        let mut input = std::mem::take(&mut self.incomplete_utf8);
        input.extend_from_slice(&block);
        let mut normalized = Vec::with_capacity(input.len());
        let mut remaining = input.as_slice();

        while !remaining.is_empty() {
            match std::str::from_utf8(remaining) {
                Ok(_) => {
                    normalized.extend_from_slice(remaining);
                    break;
                }
                Err(error) => {
                    let valid_up_to = error.valid_up_to();
                    normalized.extend_from_slice(&remaining[..valid_up_to]);
                    match error.error_len() {
                        Some(invalid_len) => {
                            normalized.extend_from_slice("�".as_bytes());
                            remaining = &remaining[valid_up_to + invalid_len..];
                        }
                        None => {
                            self.incomplete_utf8
                                .extend_from_slice(&remaining[valid_up_to..]);
                            break;
                        }
                    }
                }
            }
        }

        normalized
    }

    fn advance(&mut self) -> ShellCheckEvent {
        let completed = self.current_step;
        self.history.clear();
        if self.current_step + 1 == self.steps.len() {
            self.phase = ShellCheckPhase::Completed;
            if self.sequence_completion_is_success {
                ShellCheckEvent::SequenceCompleted
            } else {
                ShellCheckEvent::None
            }
        } else {
            self.current_step += 1;
            self.phase = if self.steps[self.current_step].config.shell_cmd.is_some() {
                ShellCheckPhase::WaitingForPrefix
            } else {
                ShellCheckPhase::WaitingForResult
            };
            ShellCheckEvent::StepCompleted(completed)
        }
    }

    fn retain_boundary_history(&mut self, output: &[u8]) {
        let prefix_len = self.steps[self.current_step]
            .config
            .shell_prefix
            .as_ref()
            .map_or(0, String::len);
        let max_len = prefix_len.max(64).saturating_mul(32);
        let text = std::str::from_utf8(output).expect("shell-check history must be valid UTF-8");
        let mut retained_start = output.len().saturating_sub(max_len);
        while !text.is_char_boundary(retained_start) {
            retained_start += 1;
        }
        self.history.clear();
        self.history.extend_from_slice(&output[retained_start..]);
    }

    fn pending_result(&self) -> Option<(usize, Option<u64>)> {
        (self.phase == ShellCheckPhase::WaitingForResult).then(|| {
            let step = &self.steps[self.current_step].config;
            (self.current_step, step.timeout)
        })
    }

    fn timeout_step(&mut self, step: usize) -> bool {
        if self.current_step == step && self.phase == ShellCheckPhase::WaitingForResult {
            self.phase = ShellCheckPhase::Completed;
            true
        } else {
            false
        }
    }
}

const SHELL_CHECK_RESULT_DRAIN_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug, Default)]
struct ShellCheckDriverOutcome {
    completed: bool,
    failure: Option<String>,
}

#[derive(Clone)]
pub(crate) struct ShellCheckDriver {
    matcher: Arc<Mutex<ShellCheckMatcher>>,
    outcome: Arc<Mutex<ShellCheckDriverOutcome>>,
}

impl ShellCheckDriver {
    pub(crate) fn new(matcher: ShellCheckMatcher) -> Self {
        Self {
            matcher: Arc::new(Mutex::new(matcher)),
            outcome: Arc::new(Mutex::new(ShellCheckDriverOutcome::default())),
        }
    }

    pub(crate) fn observe_chunk(&self, handle: &TerminalHandle, chunk: &[u8]) {
        let event = self.matcher.lock().unwrap().observe_chunk(chunk);
        self.handle_event(handle, event);
    }

    pub(crate) fn completed(&self) -> bool {
        self.outcome.lock().unwrap().completed
    }

    pub(crate) fn take_failure(&self) -> Option<anyhow::Error> {
        self.outcome
            .lock()
            .unwrap()
            .failure
            .take()
            .map(anyhow::Error::msg)
    }

    pub(crate) fn completion_error(&self) -> Option<anyhow::Error> {
        if let Some(error) = self.take_failure() {
            return Some(error);
        }
        let matcher = self.matcher.lock().unwrap();
        if matcher.sequence_completion_is_success && !self.completed() {
            Some(anyhow::anyhow!(
                "shell check sequence ended before shell_check_steps[{}] completed",
                matcher.current_step
            ))
        } else {
            None
        }
    }

    fn handle_event(&self, handle: &TerminalHandle, event: ShellCheckEvent) {
        match event {
            ShellCheckEvent::None | ShellCheckEvent::StepCompleted(_) => {}
            ShellCheckEvent::Send(command) => {
                let driver = self.clone();
                handle.send_after_chunks_then(
                    SHELL_CHECK_DELAY,
                    command,
                    SHELL_CHECK_CHUNK_SIZE,
                    SHELL_CHECK_CHUNK_DELAY,
                    move |handle, result| match result {
                        Ok(()) => driver.command_flushed(handle),
                        Err(error) => driver.fail(
                            handle,
                            format!("failed to send shell check command: {error}"),
                        ),
                    },
                );
            }
            ShellCheckEvent::SequenceCompleted => {
                self.outcome.lock().unwrap().completed = true;
                handle.stop_after(SHELL_CHECK_RESULT_DRAIN_DURATION);
            }
            ShellCheckEvent::Failed { step, pattern } => {
                self.fail(
                    handle,
                    format!("shell_check_steps[{step}] matched fail_regex `{pattern}`"),
                );
            }
            ShellCheckEvent::PendingOutputLimitExceeded {
                step,
                buffered,
                limit,
            } => {
                self.fail(handle, pending_output_limit_message(step, buffered, limit));
            }
        }
    }

    fn command_flushed(&self, handle: &TerminalHandle) {
        let (event, pending) = {
            let mut matcher = self.matcher.lock().unwrap();
            let event = matcher.command_flushed();
            let pending = matcher.pending_result();
            (event, pending)
        };
        self.handle_event(handle, event);
        if let Some((step, Some(timeout))) = pending {
            self.start_step_timeout(handle.downgrade(), step, timeout);
        }
    }

    fn start_step_timeout(&self, handle: WeakTerminalHandle, step: usize, timeout: u64) {
        let driver = self.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(timeout)).await;
            if driver.complete_step_timeout(step, timeout)
                && let Some(handle) = handle.upgrade()
            {
                handle.stop_after(SHELL_CHECK_RESULT_DRAIN_DURATION);
            }
        });
    }

    fn complete_step_timeout(&self, step: usize, timeout: u64) -> bool {
        let timeout_won = self.matcher.lock().unwrap().timeout_step(step);
        if !timeout_won {
            return false;
        }
        self.record_failure(format!(
            "shell_check_steps[{step}] timed out after {timeout}s"
        ));
        true
    }

    fn fail(&self, handle: &TerminalHandle, message: String) {
        if self.record_failure(message) {
            handle.stop_after(SHELL_CHECK_RESULT_DRAIN_DURATION);
        }
    }

    fn record_failure(&self, message: String) -> bool {
        let mut outcome = self.outcome.lock().unwrap();
        if outcome.failure.is_none() {
            outcome.failure = Some(message);
            true
        } else {
            false
        }
    }
}

fn pending_output_limit_message(step: usize, buffered: usize, limit: usize) -> String {
    format!(
        "shell_check_steps[{step}] pending output would reach {buffered} bytes, exceeding \
         the {limit}-byte limit"
    )
}

fn compile_patterns(patterns: Option<&[String]>) -> Result<Vec<Regex>> {
    patterns
        .unwrap_or_default()
        .iter()
        .map(|pattern| Regex::new(pattern).map_err(anyhow::Error::from))
        .collect()
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Barrier, Mutex};

    use super::{
        SHELL_CHECK_PENDING_OUTPUT_LIMIT, ShellCheckEvent, ShellCheckMatcher, ShellCheckStep,
        normalize_shell_check_steps, pending_output_limit_message, prepare_shell_cmd,
    };

    #[test]
    fn normalize_shell_check_steps_rejects_missing_first_prefix() {
        let mut steps = vec![ShellCheckStep {
            shell_cmd: Some("echo ready".into()),
            ..Default::default()
        }];

        let err = normalize_shell_check_steps(&mut steps, "QEMU config").unwrap_err();

        assert!(err.to_string().contains("shell_prefix"));
    }

    #[test]
    fn normalize_shell_check_steps_preserves_command_whitespace() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("  login: ".into()),
            shell_cmd: Some("  printf x  \r\n".into()),
            ..Default::default()
        }];

        normalize_shell_check_steps(&mut steps, "QEMU config").unwrap();

        assert_eq!(steps[0].shell_prefix.as_deref(), Some("login:"));
        assert_eq!(steps[0].shell_cmd.as_deref(), Some("  printf x  \r\n"));
        assert_eq!(
            prepare_shell_cmd(steps[0].shell_cmd.as_deref().unwrap()),
            b"  printf x  \n"
        );
    }

    #[test]
    fn empty_shell_check_steps_normalize_to_no_shell_check() {
        let mut steps = Vec::new();

        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();

        assert!(resolved.is_empty());
    }

    #[test]
    fn normalize_shell_check_steps_rejects_explicit_empty_prefix() {
        for prefix in ["", " \t\r\n "] {
            let mut steps = vec![ShellCheckStep {
                shell_prefix: Some(prefix.into()),
                shell_cmd: Some("run".into()),
                ..Default::default()
            }];

            let error = normalize_shell_check_steps(&mut steps, "test config").unwrap_err();

            assert!(error.to_string().contains("shell_prefix"), "{error:#}");
        }
    }

    #[test]
    fn normalize_shell_check_steps_rejects_empty_command() {
        for command in ["", " \t\r\n "] {
            let mut steps = vec![ShellCheckStep {
                shell_prefix: Some("ready>".into()),
                shell_cmd: Some(command.into()),
                ..Default::default()
            }];

            let error = normalize_shell_check_steps(&mut steps, "test config").unwrap_err();

            assert!(error.to_string().contains("shell_cmd"), "{error:#}");
        }
    }

    #[test]
    fn normalize_shell_check_steps_rejects_empty_regex_arrays() {
        for (success_regex, fail_regex, field) in [
            (Some(Vec::new()), None, "success_regex"),
            (Some(vec!["PASS".into()]), Some(Vec::new()), "fail_regex"),
        ] {
            let mut steps = vec![ShellCheckStep {
                shell_prefix: Some("ready>".into()),
                shell_cmd: Some("run".into()),
                success_regex,
                fail_regex,
                ..Default::default()
            }];

            let error = normalize_shell_check_steps(&mut steps, "test config").unwrap_err();

            assert!(error.to_string().contains(field), "{error:#}");
        }
    }

    #[test]
    fn normalize_shell_check_steps_rejects_empty_regex_entries() {
        for (success_regex, fail_regex, field) in [
            (Some(vec![" \t".into()]), None, "success_regex"),
            (
                Some(vec!["PASS".into()]),
                Some(vec![" \r\n".into()]),
                "fail_regex",
            ),
        ] {
            let mut steps = vec![ShellCheckStep {
                shell_prefix: Some("ready>".into()),
                shell_cmd: Some("run".into()),
                success_regex,
                fail_regex,
                ..Default::default()
            }];

            let error = normalize_shell_check_steps(&mut steps, "test config").unwrap_err();

            assert!(error.to_string().contains(field), "{error:#}");
        }
    }

    #[test]
    fn normalize_shell_check_steps_rejects_invalid_regex() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("ready>".into()),
            shell_cmd: Some("run".into()),
            success_regex: Some(vec!["[".into()]),
            ..Default::default()
        }];

        let error = normalize_shell_check_steps(&mut steps, "test config").unwrap_err();

        assert!(error.to_string().contains("success_regex"), "{error:#}");
    }

    #[test]
    fn normalize_shell_check_steps_rejects_zero_timeout() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("ready>".into()),
            shell_cmd: Some("run".into()),
            success_regex: Some(vec!["PASS".into()]),
            timeout: Some(0),
            ..Default::default()
        }];

        let error = normalize_shell_check_steps(&mut steps, "test config").unwrap_err();

        assert!(error.to_string().contains("timeout"), "{error:#}");
    }

    #[test]
    fn normalize_shell_check_steps_rejects_timeout_without_command() {
        let mut steps = vec![ShellCheckStep {
            success_regex: Some(vec!["PASS".into()]),
            timeout: Some(3),
            ..Default::default()
        }];

        let error = normalize_shell_check_steps(&mut steps, "QEMU config").unwrap_err();

        assert!(
            error.to_string().contains("passive") && error.to_string().contains("timeout"),
            "{error:#}"
        );
    }

    #[test]
    fn prepare_shell_cmd_appends_single_newline() {
        assert_eq!(prepare_shell_cmd("root"), b"root\n");
        assert_eq!(prepare_shell_cmd("root\n"), b"root\n");
        assert_eq!(prepare_shell_cmd("root\r\n"), b"root\n");
    }

    #[test]
    fn shell_check_matcher_triggers_once() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("login:".into()),
            shell_cmd: Some("root".into()),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut matcher = ShellCheckMatcher::from_steps(resolved).unwrap();

        let mut matched = None;
        for byte in b"noise login: login:" {
            if let Some(command) = matcher.observe_byte(*byte) {
                matched = Some(command);
            }
        }

        assert_eq!(matched.as_deref(), Some(&b"root\n"[..]));
        assert_eq!(matcher.observe_byte(b':'), None);
    }

    #[test]
    fn retained_history_never_starts_inside_a_utf8_character() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("target>".into()),
            shell_cmd: Some("run".into()),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut matcher = ShellCheckMatcher::from_steps(resolved).unwrap();

        let mut output = "✓".as_bytes().to_vec();
        output.extend_from_slice(&vec![b'x'; 2047]);
        assert_eq!(matcher.observe_chunk(&output), ShellCheckEvent::None);
        assert_eq!(
            matcher.observe_chunk(b"target>"),
            ShellCheckEvent::Send(b"run\n".to_vec())
        );
    }

    #[test]
    fn prefix_matches_when_utf8_character_is_split_across_chunks() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("你好>".into()),
            shell_cmd: Some("run".into()),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut matcher = ShellCheckMatcher::from_steps(resolved).unwrap();
        let prefix = "你好>".as_bytes();

        assert_eq!(matcher.observe_chunk(&prefix[..1]), ShellCheckEvent::None);
        assert_eq!(
            matcher.observe_chunk(&prefix[1..]),
            ShellCheckEvent::Send(b"run\n".to_vec())
        );
    }

    #[test]
    fn ordered_steps_inherit_prefix_and_advance_after_success() {
        let mut steps = vec![
            ShellCheckStep {
                shell_prefix: Some("axvisor:/$".into()),
                shell_cmd: Some("vm console 1".into()),
                success_regex: Some(vec!["Attached VM\\[1\\] console".into()]),
                ..Default::default()
            },
            ShellCheckStep {
                shell_cmd: Some("echo pass".into()),
                success_regex: Some(vec!["(?m)^pass$".into()]),
                ..Default::default()
            },
        ];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        assert_eq!(resolved[1].shell_prefix.as_deref(), Some("axvisor:/$"));

        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();
        assert_eq!(
            sequence.observe_chunk(b"axvisor:/$"),
            ShellCheckEvent::Send(b"vm console 1\n".to_vec())
        );
        sequence.command_flushed();
        assert_eq!(
            sequence.observe_chunk(b"Attached VM[1] console"),
            ShellCheckEvent::StepCompleted(0)
        );
        assert_eq!(
            sequence.observe_chunk(b"axvisor:/$"),
            ShellCheckEvent::Send(b"echo pass\n".to_vec())
        );
        sequence.command_flushed();
        assert_eq!(
            sequence.observe_chunk(b"pass"),
            ShellCheckEvent::SequenceCompleted
        );
    }

    #[test]
    fn step_without_result_patterns_advances_after_command_is_sent() {
        let mut steps = vec![
            ShellCheckStep {
                shell_prefix: Some("first>".into()),
                shell_cmd: Some("next".into()),
                ..Default::default()
            },
            ShellCheckStep {
                shell_prefix: Some("second>".into()),
                shell_cmd: Some("done".into()),
                ..Default::default()
            },
        ];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();

        assert!(matches!(
            sequence.observe_chunk(b"first>"),
            ShellCheckEvent::Send(_)
        ));
        assert_eq!(
            sequence.command_flushed(),
            ShellCheckEvent::StepCompleted(0)
        );
        assert!(matches!(
            sequence.observe_chunk(b"second>"),
            ShellCheckEvent::Send(_)
        ));
        assert_eq!(
            sequence.command_flushed(),
            ShellCheckEvent::SequenceCompleted
        );
    }

    #[test]
    fn output_received_while_command_is_sending_is_replayed_after_send() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("shell>".into()),
            shell_cmd: Some("echo pass".into()),
            success_regex: Some(vec!["pass".into()]),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();

        assert_eq!(
            sequence.observe_chunk(b"shell>"),
            ShellCheckEvent::Send(b"echo pass\n".to_vec())
        );
        assert_eq!(sequence.observe_chunk(b"pass"), ShellCheckEvent::None);
        assert_eq!(
            sequence.command_flushed(),
            ShellCheckEvent::SequenceCompleted
        );
    }

    #[test]
    fn sending_output_larger_than_old_window_keeps_early_success_until_flush() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("shell>".into()),
            shell_cmd: Some("run".into()),
            success_regex: Some(vec!["PASS".into()]),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();
        sequence.observe_chunk(b"shell>");

        let mut sending_output = b"PASS".to_vec();
        sending_output.extend_from_slice(&vec![b'x'; 4096]);
        assert_eq!(
            sequence.observe_chunk(&sending_output),
            ShellCheckEvent::None
        );

        assert_eq!(
            sequence.command_flushed(),
            ShellCheckEvent::SequenceCompleted
        );
    }

    #[test]
    fn sending_output_over_fixed_limit_fails_without_truncation() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("shell>".into()),
            shell_cmd: Some("attach".into()),
            success_regex: Some(vec!["PASS".into()]),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();
        sequence.observe_chunk(b"shell>");

        let buffered = SHELL_CHECK_PENDING_OUTPUT_LIMIT + 1;
        assert_eq!(
            sequence.observe_chunk(&vec![b'x'; buffered]),
            ShellCheckEvent::PendingOutputLimitExceeded {
                step: 0,
                buffered,
                limit: SHELL_CHECK_PENDING_OUTPUT_LIMIT,
            }
        );
        assert_eq!(sequence.command_flushed(), ShellCheckEvent::None);
        assert_eq!(
            pending_output_limit_message(0, buffered, SHELL_CHECK_PENDING_OUTPUT_LIMIT),
            format!(
                "shell_check_steps[0] pending output would reach {buffered} bytes, exceeding the \
                 {}-byte limit",
                SHELL_CHECK_PENDING_OUTPUT_LIMIT
            )
        );
    }

    #[test]
    fn no_result_step_replays_sending_output_into_next_prefix_after_flush() {
        let mut steps = vec![
            ShellCheckStep {
                shell_prefix: Some("axvisor#".into()),
                shell_cmd: Some("attach".into()),
                ..Default::default()
            },
            ShellCheckStep {
                shell_prefix: Some("root#".into()),
                shell_cmd: Some("run".into()),
                ..Default::default()
            },
        ];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();

        assert_eq!(
            sequence.observe_chunk(b"axvisor#"),
            ShellCheckEvent::Send(b"attach\n".to_vec())
        );
        assert_eq!(sequence.observe_chunk(b"root#"), ShellCheckEvent::None);

        assert_eq!(
            sequence.command_flushed(),
            ShellCheckEvent::Send(b"run\n".to_vec())
        );
    }

    #[test]
    fn suffix_after_next_prefix_is_pending_for_newly_scheduled_command() {
        let mut steps = vec![
            ShellCheckStep {
                shell_prefix: Some("host#".into()),
                shell_cmd: Some("attach".into()),
                success_regex: Some(vec!["ATTACHED".into()]),
                ..Default::default()
            },
            ShellCheckStep {
                shell_prefix: Some("guest#".into()),
                shell_cmd: Some("run".into()),
                success_regex: Some(vec!["PASS".into()]),
                ..Default::default()
            },
        ];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();
        sequence.observe_chunk(b"host#");
        sequence.command_flushed();

        assert_eq!(
            sequence.observe_chunk(b"ATTACHED\nguest# PASS"),
            ShellCheckEvent::Send(b"run\n".to_vec())
        );

        assert_eq!(
            sequence.command_flushed(),
            ShellCheckEvent::SequenceCompleted
        );
    }

    #[test]
    fn step_fail_pattern_wins_over_success() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("ready>".into()),
            shell_cmd: Some("run".into()),
            success_regex: Some(vec!["PASS".into()]),
            fail_regex: Some(vec!["FAIL".into()]),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();
        sequence.observe_chunk(b"ready>");
        sequence.command_flushed();

        assert!(matches!(
            sequence.observe_chunk(b"PASS FAIL"),
            ShellCheckEvent::Failed { .. }
        ));
    }

    #[test]
    fn whole_waiting_result_chunk_checks_fail_after_earlier_success() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("ready>".into()),
            shell_cmd: Some("run".into()),
            success_regex: Some(vec!["PASS".into()]),
            fail_regex: Some(vec!["panic".into()]),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();
        sequence.observe_chunk(b"ready>");
        sequence.command_flushed();

        assert_eq!(
            sequence.observe_chunk(b"PASS before a later panic"),
            ShellCheckEvent::Failed {
                step: 0,
                pattern: "panic".into(),
            }
        );
    }

    #[test]
    fn earliest_success_end_preserves_guest_prefix_before_repeated_success() {
        let mut steps = vec![
            ShellCheckStep {
                shell_prefix: Some("host#".into()),
                shell_cmd: Some("attach".into()),
                success_regex: Some(vec!["OK".into()]),
                ..Default::default()
            },
            ShellCheckStep {
                shell_prefix: Some("guest#".into()),
                shell_cmd: Some("run".into()),
                ..Default::default()
            },
        ];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();
        sequence.observe_chunk(b"host#");
        sequence.command_flushed();

        assert_eq!(
            sequence.observe_chunk(b"OK\nguest# trailing OK"),
            ShellCheckEvent::Send(b"run\n".to_vec())
        );
    }

    #[test]
    fn next_step_prefix_in_same_chunk_as_success_is_not_lost() {
        let mut steps = vec![
            ShellCheckStep {
                shell_prefix: Some("first>".into()),
                shell_cmd: Some("switch".into()),
                success_regex: Some(vec!["attached".into()]),
                ..Default::default()
            },
            ShellCheckStep {
                shell_prefix: Some("guest#".into()),
                shell_cmd: Some("run".into()),
                ..Default::default()
            },
        ];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();
        sequence.observe_chunk(b"first>");
        sequence.command_flushed();

        assert_eq!(
            sequence.observe_chunk(b"attached\nguest# attached"),
            ShellCheckEvent::Send(b"run\n".to_vec())
        );
    }

    #[test]
    fn any_step_success_pattern_completes_the_step() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("shell>".into()),
            shell_cmd: Some("run".into()),
            success_regex: Some(vec!["primary pass".into(), "alternate pass".into()]),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut sequence = ShellCheckMatcher::from_steps(resolved).unwrap();
        sequence.observe_chunk(b"shell>");
        sequence.command_flushed();

        assert_eq!(
            sequence.observe_chunk(b"alternate pass"),
            ShellCheckEvent::SequenceCompleted
        );
    }

    #[test]
    fn step_timeout_and_success_are_mutually_exclusive_state_transitions() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("shell>".into()),
            shell_cmd: Some("run".into()),
            success_regex: Some(vec!["pass".into()]),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();

        let mut timeout_first = ShellCheckMatcher::from_steps(resolved.clone()).unwrap();
        timeout_first.observe_chunk(b"shell>");
        timeout_first.command_flushed();
        assert!(timeout_first.timeout_step(0));
        assert_eq!(timeout_first.observe_chunk(b"pass"), ShellCheckEvent::None);

        let mut success_first = ShellCheckMatcher::from_steps(resolved).unwrap();
        success_first.observe_chunk(b"shell>");
        success_first.command_flushed();
        assert_eq!(
            success_first.observe_chunk(b"pass"),
            ShellCheckEvent::SequenceCompleted
        );
        assert!(!success_first.timeout_step(0));
    }

    #[test]
    fn timeout_and_success_race_under_the_same_matcher_mutex() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("shell>".into()),
            shell_cmd: Some("run".into()),
            success_regex: Some(vec!["PASS".into()]),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut matcher = ShellCheckMatcher::from_steps(resolved).unwrap();
        matcher.observe_chunk(b"shell>");
        matcher.command_flushed();

        let matcher = Arc::new(Mutex::new(matcher));
        let start = Arc::new(Barrier::new(3));
        let timeout_thread = {
            let matcher = Arc::clone(&matcher);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                matcher.lock().unwrap().timeout_step(0)
            })
        };
        let success_thread = {
            let matcher = Arc::clone(&matcher);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                matcher.lock().unwrap().observe_chunk(b"PASS") == ShellCheckEvent::SequenceCompleted
            })
        };
        start.wait();

        let timeout_won = timeout_thread.join().unwrap();
        let success_won = success_thread.join().unwrap();
        assert_ne!(timeout_won, success_won);
    }

    #[test]
    fn timeout_winner_records_failure_without_a_live_terminal_handle() {
        let mut steps = vec![ShellCheckStep {
            shell_prefix: Some("shell>".into()),
            shell_cmd: Some("run".into()),
            success_regex: Some(vec!["PASS".into()]),
            timeout: Some(3),
            ..Default::default()
        }];
        let resolved = normalize_shell_check_steps(&mut steps, "test config").unwrap();
        let mut matcher = ShellCheckMatcher::from_steps(resolved).unwrap();
        matcher.observe_chunk(b"shell>");
        matcher.command_flushed();
        let driver = super::ShellCheckDriver::new(matcher);

        assert!(driver.complete_step_timeout(0, 3));

        let error = driver.completion_error().unwrap();
        assert_eq!(error.to_string(), "shell_check_steps[0] timed out after 3s");
    }

    #[test]
    fn steps_reject_fail_without_success() {
        let mut fail_only = vec![ShellCheckStep {
            shell_prefix: Some("ready>".into()),
            shell_cmd: Some("run".into()),
            fail_regex: Some(vec!["FAIL".into()]),
            ..Default::default()
        }];
        assert!(
            normalize_shell_check_steps(&mut fail_only, "test config")
                .unwrap_err()
                .to_string()
                .contains("success_regex")
        );
    }
}
