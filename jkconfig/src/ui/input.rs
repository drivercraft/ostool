use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputBufferKind {
    Text,
    Integer,
    Number,
}

#[derive(Debug, Clone)]
pub struct InputBuffer {
    value: String,
    cursor_char_index: usize,
}

impl InputBuffer {
    pub fn new(value: impl Into<String>) -> Self {
        let value = value.into();
        let cursor_char_index = value.chars().count();
        Self {
            value,
            cursor_char_index,
        }
    }

    pub fn value(&self) -> &str {
        &self.value
    }

    pub fn move_left(&mut self) {
        self.cursor_char_index = self.cursor_char_index.saturating_sub(1);
    }

    pub fn move_right(&mut self) {
        self.cursor_char_index = self
            .cursor_char_index
            .saturating_add(1)
            .min(self.value.chars().count());
    }

    pub fn move_home(&mut self) {
        self.cursor_char_index = 0;
    }

    pub fn move_end(&mut self) {
        self.cursor_char_index = self.value.chars().count();
    }

    pub fn insert_char(&mut self, ch: char) {
        let byte_index = self.byte_index();
        self.value.insert(byte_index, ch);
        self.move_right();
    }

    pub fn delete_left(&mut self) {
        if self.cursor_char_index == 0 {
            return;
        }

        let current_index = self.cursor_char_index;
        let left_index = current_index - 1;
        let before = self.value.chars().take(left_index);
        let after = self.value.chars().skip(current_index);
        self.value = before.chain(after).collect();
        self.move_left();
    }

    pub fn visible_text_and_cursor(&self, max_width: usize) -> (String, u16) {
        if max_width == 0 {
            return (String::new(), 0);
        }

        let chars: Vec<char> = self.value.chars().collect();
        let total_width = UnicodeWidthStr::width(self.value.as_str());
        let cursor_prefix_width = display_width(chars.iter().take(self.cursor_char_index).copied());

        if total_width <= max_width {
            return (
                self.value.clone(),
                cursor_prefix_width.min(max_width) as u16,
            );
        }

        let mut start = 0usize;
        let mut start_width = 0usize;
        while start < self.cursor_char_index
            && cursor_prefix_width.saturating_sub(start_width) >= max_width
        {
            start_width += chars[start].width().unwrap_or(0);
            start += 1;
        }

        let mut visible = String::new();
        let mut current_width = 0usize;
        for ch in chars.iter().skip(start).copied() {
            let ch_width = ch.width().unwrap_or(0);
            if current_width + ch_width > max_width {
                break;
            }
            visible.push(ch);
            current_width += ch_width;
        }

        let cursor_offset = cursor_prefix_width
            .saturating_sub(start_width)
            .min(max_width) as u16;
        (visible, cursor_offset)
    }

    pub fn parse_i64(&self) -> anyhow::Result<i64> {
        self.value.trim().parse::<i64>().map_err(Into::into)
    }

    pub fn parse_f64(&self) -> anyhow::Result<f64> {
        self.value.trim().parse::<f64>().map_err(Into::into)
    }

    pub fn can_accept_char(&self, kind: InputBufferKind, ch: char) -> bool {
        match kind {
            InputBufferKind::Text => true,
            InputBufferKind::Integer => ch.is_ascii_digit() || matches!(ch, '-' | '+'),
            InputBufferKind::Number => ch.is_ascii_digit() || matches!(ch, '-' | '+' | '.'),
        }
    }

    fn byte_index(&self) -> usize {
        self.value
            .char_indices()
            .map(|(index, _)| index)
            .nth(self.cursor_char_index)
            .unwrap_or(self.value.len())
    }
}

fn display_width(chars: impl Iterator<Item = char>) -> usize {
    chars.map(|ch| ch.width().unwrap_or(0)).sum()
}

#[cfg(test)]
mod tests {
    use super::InputBuffer;

    #[test]
    fn input_buffer_inserts_and_deletes() {
        let mut buffer = InputBuffer::new("ac");
        buffer.move_left();
        buffer.insert_char('b');
        assert_eq!(buffer.value(), "abc");
        buffer.delete_left();
        assert_eq!(buffer.value(), "ac");
    }

    #[test]
    fn input_buffer_keeps_cursor_visible_for_wide_chars() {
        let mut buffer = InputBuffer::new("你好abc");
        buffer.move_end();
        let (visible, cursor) = buffer.visible_text_and_cursor(4);
        assert!(!visible.is_empty());
        assert!(cursor <= 4);
    }
}
