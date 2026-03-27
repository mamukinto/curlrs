use std::time::Duration;

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Direction, Layout, Position},
    style::{Color, Style},
    widgets::{Block, List, Paragraph, Widget},
};

pub struct TextInput {
    input: String,
    cursor: usize,
    focused: bool,
}

impl TextInput {
    pub fn new() -> Self {
        Self {
            input: String::new(),
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

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    enable_raw_mode()?;
    let mut application = App::new();
    ratatui::run(|terminal| application.run(terminal))?;
    disable_raw_mode()?;
    Ok(())
}

pub struct App {
    exit: bool,
    sidebar_w: u16,
    open: bool,
    url_input: TextInput,
}

impl App {
    fn new() -> Self {
        Self {
            exit: false,
            sidebar_w: 25,
            open: false,
            url_input: TextInput::new(),
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        while !self.exit {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_events()?;
            
            if self.open {
                self.sidebar_w += 1;
            }
        }

        Ok(())
    }

    fn render(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());

        // Cursor positioning needs Frame, can't be done inside Widget trait
        if self.url_input.focused {
            let area = frame.area();
            let h_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![
                    Constraint::Percentage(self.sidebar_w),
                    Constraint::Percentage(100 - self.sidebar_w),
                ])
                .split(area);
            let control_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Length(3), Constraint::Min(1)])
                .split(h_layout[1]);
            let input_area = control_layout[0];

            #[expect(clippy::cast_possible_truncation)]
            frame.set_cursor_position(Position::new(
                input_area.x + self.url_input.cursor as u16 + 1,
                input_area.y + 1,
            ));
        }
    }

    fn handle_events(&mut self) -> std::io::Result<()> {
        if event::poll(Duration::from_millis(16))? {
            match event::read()? {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    if self.url_input.focused {
                        // Input mode: keys go to the text input
                        match key_event.code {
                            KeyCode::Esc => self.url_input.blur(),
                            code => {
                                if self.url_input.handle_key_event(code) {
                                    // Enter pressed — TODO: make request
                                    let _url = self.url_input.value().to_string();
                                }
                            }
                        }
                    } else {
                        // Normal mode: resize, navigate, etc.
                        match key_event.code {
                            KeyCode::Char('q') => self.exit(),
                            KeyCode::Char('i') => self.url_input.focus(),
                            KeyCode::Right => {
                                if self.sidebar_w < 100 {
                                    self.sidebar_w += 1;
                                }
                            }
                            KeyCode::Left => {
                                if self.sidebar_w > 0 {
                                    self.sidebar_w -= 1;
                                }
                            }
                            KeyCode::Up => {
                                self.open = true;
                            }
                            KeyCode::Down => {
                                self.open = false;
                            }
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }

        Ok(())
    }

    fn exit(&mut self) {
        self.exit = true;
    }
}

impl Widget for &App {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer) {
        let h_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Percentage(self.sidebar_w),
                Constraint::Percentage(100 - self.sidebar_w),
            ])
            .split(area);

        // Sidebar
        let requests_block = Block::bordered()
            .title("Requests")
            .title_position(ratatui::widgets::TitlePosition::Top)
            .title_alignment(Alignment::Center)
            .title_style(Style::new().yellow().on_blue())
            .border_style(Style::new().green());

        let items = ["kvato", "mushy", "pingu", "KROLA", "KROCHA"];
        List::new(items)
            .block(requests_block)
            .style(Style::new().red())
            .highlight_style(Style::new().italic())
            .highlight_symbol(">>")
            .repeat_highlight_symbol(true)
            .render(h_layout[0], buf);

        // Control panel: URL input + response
        let control_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Length(3),
                Constraint::Min(1),
            ])
            .split(h_layout[1]);

        // URL input
        let url_block = Block::bordered()
            .title("URL (i to edit)")
            .border_style(if self.url_input.focused {
                Style::new().yellow()
            } else {
                Style::new().green()
            });
        let url_style = if self.url_input.focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        Paragraph::new(self.url_input.input.as_str())
            .style(url_style)
            .block(url_block)
            .render(control_layout[0], buf);

        // Response area
        let response_block = Block::bordered()
            .title("Response")
            .title_alignment(Alignment::Center)
            .title_style(Style::new().yellow().on_blue())
            .border_style(Style::new().green());
        response_block.render(control_layout[1], buf);
    }
}
