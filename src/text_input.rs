use crossterm::event::KeyCode;

#[derive(Clone)]
pub struct TextInput {
    pub input: String,
    pub cursor: usize,
    pub focused: bool,
    pub multiline: bool,
}

impl TextInput {
    pub fn new(input: String) -> Self {
        Self {
            input,
            cursor: 0,
            focused: false,
            multiline: false,
        }
    }

    pub fn new_multiline(input: String) -> Self {
        Self {
            input,
            cursor: 0,
            focused: false,
            multiline: true,
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

    /// Handle a key event. Returns true if Enter was pressed (submitted).
    pub fn handle_key_event(&mut self, code: KeyCode) -> bool {
        if !self.focused {
            return false;
        }
        match code {
            KeyCode::Char(c) => {
                let idx = self.byte_index();
                self.input.insert(idx, c);
                self.cursor = self.cursor.saturating_add(1);
            }
            KeyCode::Backspace => {
                if self.cursor > 0 {
                    let current = self.cursor;
                    let before: String = self.input.chars().take(current - 1).collect();
                    let after: String = self.input.chars().skip(current).collect();
                    self.input = before + &after;
                    self.cursor -= 1;
                }
            }
            KeyCode::Left => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            KeyCode::Right => {
                self.cursor = (self.cursor + 1).min(self.input.chars().count());
            }
            KeyCode::Up if self.multiline => {
                let col = self.cursor_col();
                let row = self.cursor_row();
                if row > 0 {
                    // Find start of previous line
                    let lines: Vec<&str> = self.input.split('\n').collect();
                    let prev_line_len = lines[row - 1].chars().count();
                    let target_col = col.min(prev_line_len);
                    // Jump back: current col + 1 (the \n) + prev_line_len - target_col
                    self.cursor -= col + 1 + (prev_line_len - target_col);
                }
            }
            KeyCode::Down if self.multiline => {
                let col = self.cursor_col();
                let row = self.cursor_row();
                let lines: Vec<&str> = self.input.split('\n').collect();
                if row + 1 < lines.len() {
                    let cur_line_len = lines[row].chars().count();
                    let next_line_len = lines[row + 1].chars().count();
                    let target_col = col.min(next_line_len);
                    // Jump forward: remaining chars on current line + 1 (\n) + target_col
                    self.cursor += (cur_line_len - col) + 1 + target_col;
                }
            }
            KeyCode::Enter => {
                if self.multiline {
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
