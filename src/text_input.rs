use crossterm::event::KeyCode;

#[derive(Clone)]
pub struct TextInput {
    pub input: String,
    pub cursor: usize,
    pub focused: bool,
}

impl TextInput {
    pub fn new() -> Self {
        Self {
            input: "https://dogapi.dog/api/v2/breeds".to_string(),
            cursor: 0,
            focused: false,
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
            KeyCode::Enter => return true,
            _ => {}
        }
        false
    }

    fn byte_index(&self) -> usize {
        self.input
            .char_indices()
            .map(|(i, _)| i)
            .nth(self.cursor)
            .unwrap_or(self.input.len())
    }
}
