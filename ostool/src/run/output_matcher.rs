use std::time::{Duration, Instant};

use anyhow::anyhow;
use regex::Regex;

pub(crate) const MATCH_DRAIN_DURATION: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StreamMatchKind {
    Success,
    Fail,
}

#[derive(Debug, Clone)]
pub(crate) struct StreamMatch {
    pub(crate) kind: StreamMatchKind,
    pub(crate) matched_regex: String,
    pub(crate) matched_text: String,
    pub(crate) deadline: Instant,
}

#[derive(Debug, Clone)]
enum StreamMatchState {
    Pending,
    Matched(StreamMatch),
}

pub(crate) struct ByteStreamMatcher {
    success_regex: Vec<Regex>,
    fail_regex: Vec<Regex>,
    line_buf: Vec<u8>,
    state: StreamMatchState,
}

impl ByteStreamMatcher {
    pub(crate) fn new(success_regex: Vec<Regex>, fail_regex: Vec<Regex>) -> Self {
        Self {
            success_regex,
            fail_regex,
            line_buf: Vec::with_capacity(0x1000),
            state: StreamMatchState::Pending,
        }
    }

    pub(crate) fn observe_byte(&mut self, byte: u8) -> Option<StreamMatch> {
        self.line_buf.push(byte);

        let first_match = match self.state {
            StreamMatchState::Pending => {
                let line = String::from_utf8_lossy(&self.line_buf);

                let matched = self
                    .fail_regex
                    .iter()
                    .find(|regex| regex.is_match(&line))
                    .map(|regex| StreamMatch {
                        kind: StreamMatchKind::Fail,
                        matched_regex: regex.as_str().to_string(),
                        matched_text: line.to_string(),
                        deadline: Instant::now() + MATCH_DRAIN_DURATION,
                    })
                    .or_else(|| {
                        self.success_regex
                            .iter()
                            .find(|regex| regex.is_match(&line))
                            .map(|regex| StreamMatch {
                                kind: StreamMatchKind::Success,
                                matched_regex: regex.as_str().to_string(),
                                matched_text: line.to_string(),
                                deadline: Instant::now() + MATCH_DRAIN_DURATION,
                            })
                    });

                if let Some(matched) = matched {
                    self.state = StreamMatchState::Matched(matched.clone());
                    Some(matched)
                } else {
                    None
                }
            }
            StreamMatchState::Matched(_) => None,
        };

        if byte == b'\n' {
            self.line_buf.clear();
        }

        first_match
    }

    pub(crate) fn matched(&self) -> Option<&StreamMatch> {
        match &self.state {
            StreamMatchState::Pending => None,
            StreamMatchState::Matched(matched) => Some(matched),
        }
    }

    pub(crate) fn should_stop(&self) -> bool {
        self.matched()
            .is_some_and(|matched| Instant::now() >= matched.deadline)
    }

    pub(crate) fn final_result(&self) -> Option<anyhow::Result<()>> {
        let matched = self.matched()?;
        match matched.kind {
            StreamMatchKind::Success => Some(Ok(())),
            StreamMatchKind::Fail => Some(Err(anyhow!(
                "Detected fail pattern '{}' in output: {}",
                matched.matched_regex,
                matched.matched_text.trim_end()
            ))),
        }
    }
}
