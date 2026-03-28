use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::{
    style::Style,
    text::{Line, Span, Text},
};

#[derive(Clone)]
pub struct TextInput {
    pub input: String,
    pub cursor: usize,
    pub focused: bool,
    pub multiline: bool,
    pub selection_anchor: Option<usize>,
}

impl TextInput {
    pub fn new(input: String) -> Self {
        Self {
            input,
            cursor: 0,
            focused: false,
            multiline: false,
            selection_anchor: None,
        }
    }

    pub fn new_multiline(input: String) -> Self {
        Self {
            input,
            cursor: 0,
            focused: false,
            multiline: true,
            selection_anchor: None,
        }
    }

    pub fn focus(&mut self) {
        self.focused = true;
    }

    pub fn blur(&mut self) {
        self.focused = false;
    }

    pub fn value(&self) -> &str {
        &self.input
    }

    /// Returns (start, end) of selection sorted, or None if no selection.
    pub fn selection_range(&self) -> Option<(usize, usize)> {
        self.selection_anchor.map(|anchor| {
            if anchor <= self.cursor {
                (anchor, self.cursor)
            } else {
                (self.cursor, anchor)
            }
        })
    }

    pub fn has_selection(&self) -> bool {
        matches!(self.selection_range(), Some((s, e)) if s != e)
    }

    fn delete_selection(&mut self) {
        if let Some((start, end)) = self.selection_range() {
            if start != end {
                let before: String = self.input.chars().take(start).collect();
                let after: String = self.input.chars().skip(end).collect();
                self.input = before + &after;
                self.cursor = start;
            }
        }
        self.selection_anchor = None;
    }

    /// Handle a key event. Returns true if Enter was pressed (submitted).
    pub fn handle_key_event(&mut self, key: KeyEvent) -> bool {
        if !self.focused {
            return false;
        }

        let code = key.code;
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);

        match code {
            KeyCode::Char(c) => {
                if self.has_selection() {
                    self.delete_selection();
                }
                let idx = self.byte_index();
                self.input.insert(idx, c);
                self.cursor = self.cursor.saturating_add(1);
            }
            KeyCode::Backspace => {
                if self.has_selection() {
                    self.delete_selection();
                } else if self.cursor > 0 {
                    let current = self.cursor;
                    let before: String = self.input.chars().take(current - 1).collect();
                    let after: String = self.input.chars().skip(current).collect();
                    self.input = before + &after;
                    self.cursor -= 1;
                }
            }
            KeyCode::Delete => {
                if self.has_selection() {
                    self.delete_selection();
                } else if self.cursor < self.input.chars().count() {
                    let before: String = self.input.chars().take(self.cursor).collect();
                    let after: String = self.input.chars().skip(self.cursor + 1).collect();
                    self.input = before + &after;
                }
            }
            KeyCode::Left => {
                if shift {
                    if self.selection_anchor.is_none() {
                        self.selection_anchor = Some(self.cursor);
                    }
                } else {
                    self.selection_anchor = None;
                }
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                if shift {
                    if self.selection_anchor.is_none() {
                        self.selection_anchor = Some(self.cursor);
                    }
                } else {
                    self.selection_anchor = None;
                }
                self.cursor = (self.cursor + 1).min(self.input.chars().count());
            }
            KeyCode::Up if self.multiline => {
                if shift {
                    if self.selection_anchor.is_none() {
                        self.selection_anchor = Some(self.cursor);
                    }
                } else {
                    self.selection_anchor = None;
                }
                let col = self.cursor_col();
                let row = self.cursor_row();
                if row > 0 {
                    let lines: Vec<&str> = self.input.split('\n').collect();
                    let prev_line_len = lines[row - 1].chars().count();
                    let target_col = col.min(prev_line_len);
                    self.cursor -= col + 1 + (prev_line_len - target_col);
                }
            }
            KeyCode::Down if self.multiline => {
                if shift {
                    if self.selection_anchor.is_none() {
                        self.selection_anchor = Some(self.cursor);
                    }
                } else {
                    self.selection_anchor = None;
                }
                let col = self.cursor_col();
                let row = self.cursor_row();
                let lines: Vec<&str> = self.input.split('\n').collect();
                if row + 1 < lines.len() {
                    let cur_line_len = lines[row].chars().count();
                    let next_line_len = lines[row + 1].chars().count();
                    let target_col = col.min(next_line_len);
                    self.cursor += (cur_line_len - col) + 1 + target_col;
                }
            }
            KeyCode::Enter => {
                if self.multiline {
                    if self.has_selection() {
                        self.delete_selection();
                    }
                    let idx = self.byte_index();
                    self.input.insert(idx, '\n');
                    self.cursor = self.cursor.saturating_add(1);
                } else {
                    return true;
                }
            }
            _ => {}
        }
        false
    }

    /// Returns styled text with selection highlighted.
    pub fn styled_text(&self, normal: Style, selection: Style) -> Text<'static> {
        let sel = self.selection_range().filter(|(s, e)| s != e);
        let chars: Vec<char> = self.input.chars().collect();

        let mut lines: Vec<Line<'static>> = Vec::new();
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut buf = String::new();
        let mut in_sel = false;

        for (i, &ch) in chars.iter().enumerate() {
            let now_sel = sel.is_some_and(|(s, e)| i >= s && i < e);

            if ch == '\n' {
                if !buf.is_empty() {
                    spans.push(Span::styled(
                        buf.clone(),
                        if in_sel { selection } else { normal },
                    ));
                    buf.clear();
                }
                lines.push(Line::from(spans));
                spans = Vec::new();
                in_sel = now_sel;
            } else {
                if now_sel != in_sel && !buf.is_empty() {
                    spans.push(Span::styled(
                        buf.clone(),
                        if in_sel { selection } else { normal },
                    ));
                    buf.clear();
                }
                in_sel = now_sel;
                buf.push(ch);
            }
        }

        if !buf.is_empty() {
            spans.push(Span::styled(buf, if in_sel { selection } else { normal }));
        }
        lines.push(Line::from(spans));

        Text::from(lines)
    }

    pub fn cursor_row(&self) -> usize {
        self.input.chars().take(self.cursor).filter(|&c| c == '\n').count()
    }

    pub fn cursor_col(&self) -> usize {
        let chars: Vec<char> = self.input.chars().take(self.cursor).collect();
        match chars.iter().rposition(|&c| c == '\n') {
            Some(pos) => self.cursor - pos - 1,
            None => self.cursor,
        }
    }

    fn byte_index(&self) -> usize {
        self.input
            .char_indices()
            .map(|(i, _)| i)
            .nth(self.cursor)
            .unwrap_or(self.input.len())
    }
}
