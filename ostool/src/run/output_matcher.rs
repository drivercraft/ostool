use std::time::{Duration, Instant};

use anyhow::anyhow;
use colored::Colorize;
use regex::Regex;

pub(crate) const MATCH_DRAIN_DURATION: Duration = Duration::from_millis(500);
const MAX_MATCH_WINDOW_BYTES: usize = 2048;
const MATCH_EXCERPT_CONTEXT_CHARS: usize = 120;
const MATCH_EXCERPT_MAX_CHARS: usize = 240;

#[derive(Debug, Clone)]
pub struct FailMatch {
    pub matched_regex: String,
    pub matched_text: String,
    pub deadline: Instant,
}

impl FailMatch {
    pub(crate) fn into_error(self) -> anyhow::Error {
        anyhow!(
            "Fail pattern matched '{}': {}",
            self.matched_regex,
            match_excerpt(&self.matched_text, &self.matched_regex)
        )
    }
}

pub(crate) fn compile_fail_regexes(fail_patterns: &[String]) -> anyhow::Result<Vec<Regex>> {
    fail_patterns
        .iter()
        .map(|p| Regex::new(p).map_err(|e| anyhow!("fail regex error: {e}")))
        .collect::<Result<Vec<_>, _>>()
}

pub(crate) fn print_fail_match(matched: &FailMatch) {
    println!(
        "{}",
        format!("\n=== FAIL PATTERN MATCHED: {}", matched.matched_regex).red()
    );
}

#[derive(Debug, Clone)]
enum StreamMatchState {
    Pending,
    Matched(FailMatch),
}

pub struct FailStreamMatcher {
    fail_regex: Vec<Regex>,
    match_buf: Vec<u8>,
    state: StreamMatchState,
}

impl FailStreamMatcher {
    pub fn new(fail_regex: Vec<Regex>) -> Self {
        Self {
            fail_regex,
            match_buf: Vec::with_capacity(MAX_MATCH_WINDOW_BYTES),
            state: StreamMatchState::Pending,
        }
    }

    pub fn observe_byte(&mut self, byte: u8) -> Option<FailMatch> {
        self.match_buf.push(byte);
        if self.match_buf.len() > MAX_MATCH_WINDOW_BYTES {
            let overflow = self.match_buf.len() - MAX_MATCH_WINDOW_BYTES;
            self.match_buf.drain(..overflow);
        }

        match self.state {
            StreamMatchState::Pending => {
                let text = String::from_utf8_lossy(&self.match_buf);
                let text = strip_ansi_escape_sequences(&text);

                let matched = self
                    .fail_regex
                    .iter()
                    .find(|regex| regex.is_match(&text))
                    .map(|regex| FailMatch {
                        matched_regex: regex.as_str().to_string(),
                        matched_text: text.to_string(),
                        deadline: Instant::now() + MATCH_DRAIN_DURATION,
                    });

                if let Some(matched) = matched {
                    self.state = StreamMatchState::Matched(matched.clone());
                    Some(matched)
                } else {
                    None
                }
            }
            StreamMatchState::Matched(_) => None,
        }
    }

    pub fn matched(&self) -> Option<&FailMatch> {
        match &self.state {
            StreamMatchState::Pending => None,
            StreamMatchState::Matched(matched) => Some(matched),
        }
    }

    pub fn should_stop(&self) -> bool {
        self.matched()
            .is_some_and(|matched| Instant::now() >= matched.deadline)
    }
}

fn strip_ansi_escape_sequences(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut output = Vec::with_capacity(bytes.len());
    let mut index = 0;

    while index < bytes.len() {
        if bytes[index] == 0x1b
            && let Some(next) = bytes.get(index + 1)
            && *next == b'['
        {
            index += 2;
            while index < bytes.len() {
                let byte = bytes[index];
                index += 1;
                if (0x40..=0x7e).contains(&byte) {
                    break;
                }
            }
            continue;
        }

        output.push(bytes[index]);
        index += 1;
    }

    String::from_utf8_lossy(&output).into_owned()
}

fn match_excerpt(text: &str, matched_regex: &str) -> String {
    let compact = text.replace(['\r', '\n'], " ");
    let compact = compact.split_whitespace().collect::<Vec<_>>().join(" ");

    let Some(regex) = Regex::new(matched_regex).ok() else {
        return truncate_chars(&compact, MATCH_EXCERPT_MAX_CHARS);
    };

    let Some(found) = regex.find(&compact) else {
        return truncate_chars(&compact, MATCH_EXCERPT_MAX_CHARS);
    };

    let start = char_boundary_before(&compact, found.start(), MATCH_EXCERPT_CONTEXT_CHARS);
    let end = char_boundary_after(&compact, found.end(), MATCH_EXCERPT_CONTEXT_CHARS);
    let excerpt = compact[start..end].trim();
    let mut rendered = excerpt.to_string();

    if start > 0 {
        rendered.insert_str(0, "...");
    }
    if end < compact.len() {
        rendered.push_str("...");
    }

    truncate_chars(&rendered, MATCH_EXCERPT_MAX_CHARS)
}

fn char_boundary_before(text: &str, byte_index: usize, chars: usize) -> usize {
    text[..byte_index]
        .char_indices()
        .rev()
        .nth(chars)
        .map(|(index, _)| index)
        .unwrap_or(0)
}

fn char_boundary_after(text: &str, byte_index: usize, chars: usize) -> usize {
    text[byte_index..]
        .char_indices()
        .nth(chars)
        .map(|(offset, _)| byte_index + offset)
        .unwrap_or(text.len())
}

fn truncate_chars(text: &str, max_chars: usize) -> String {
    let mut iter = text.chars();
    let truncated = iter.by_ref().take(max_chars).collect::<String>();
    if iter.next().is_some() {
        format!("{truncated}...")
    } else {
        truncated
    }
}

#[cfg(test)]
mod tests {
    use super::{
        FailStreamMatcher, compile_fail_regexes, match_excerpt, strip_ansi_escape_sequences,
    };
    use regex::Regex;

    #[test]
    fn strips_basic_csi_sequences() {
        assert_eq!(
            strip_ansi_escape_sequences("\u{1b}[31mpanicked at test\u{1b}[m"),
            "panicked at test"
        );
    }

    #[test]
    fn runtime_output_matcher_accepts_only_global_fail_patterns() {
        let fail = compile_fail_regexes(&["panic".to_string()]).unwrap();
        let mut matcher = FailStreamMatcher::new(fail);

        let matched = b"kernel panic\n"
            .iter()
            .find_map(|byte| matcher.observe_byte(*byte))
            .expect("expected global fail match");

        assert_eq!(matched.matched_regex, "panic");
    }

    #[test]
    fn fail_matcher_ignores_ansi_sequences() {
        let mut matcher =
            FailStreamMatcher::new(vec![Regex::new("(?i)\\bpanic(?:ked)?\\b").unwrap()]);

        let input = "\u{1b}[31mpanicked at os/arceos/foo.rs:1:1\n";
        let mut matched = None;
        for byte in input.bytes() {
            matched = matcher.observe_byte(byte).or(matched);
        }

        let matched = matched.expect("expected panic match");
        assert!(matched.matched_text.to_ascii_lowercase().contains("panic"));
    }

    #[test]
    fn matcher_detects_fail_pattern_across_multiple_lines() {
        let mut matcher =
            FailStreamMatcher::new(vec![Regex::new("Failed to load VM images").unwrap()]);

        let input = "line one\nline two\npanicked at foo\nFailed to load VM images: AxErrorKind::NotFound\n";
        let mut matched = None;
        for byte in input.bytes() {
            matched = matcher.observe_byte(byte).or(matched);
        }

        let matched = matched.expect("expected match");
        assert!(matched.matched_text.contains("Failed to load VM images"));
    }

    #[test]
    fn compile_fail_regexes_keeps_empty_patterns_empty() {
        let fail = compile_fail_regexes(&[]).unwrap();

        assert!(fail.is_empty());
    }

    #[test]
    fn match_excerpt_returns_local_context_only() {
        let text = "prefix text one two three panic happened in vm loader because image missing suffix text four five";
        let excerpt = match_excerpt(text, r"(?i)\bpanic\b");

        assert!(excerpt.contains("panic"));
        assert!(!excerpt.is_empty());
        assert!(excerpt.len() <= text.len() + 3);
    }
}
