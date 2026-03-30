use std::{
    fs,
    path::PathBuf,
    str::FromStr,
    time::{Duration, Instant},
    vec,
};

use serde::{Deserialize, Serialize};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};

use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Direction, Layout, Position},
    style::{Color, Style, Stylize},
    widgets::{
        Block, List, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Tabs, Widget, Wrap,
    },
};

use reqwest::Method;
use tokio::sync::mpsc;

pub enum AppMessage {
    RequestStarted,
    RequestCompleted(HttpResponse),
}

#[derive(Clone, Serialize, Deserialize)]
pub struct SavedRequest {
    pub name: String,
    pub url: String,
    pub method_str: String,
    pub body: String,
}

const DB_FILE: &str = "curlrsdb.json";

fn db_path() -> PathBuf {
    PathBuf::from(DB_FILE)
}

fn load_saved_requests() -> Vec<SavedRequest> {
    let path = db_path();
    if !path.exists() {
        return Vec::new();
    }
    match fs::read_to_string(&path) {
        Ok(contents) => serde_json::from_str(&contents).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

fn persist_saved_requests(requests: &[SavedRequest]) {
    if let Ok(json) = serde_json::to_string_pretty(requests) {
        let _ = fs::write(db_path(), json);
    }
}

mod text_input;

use crate::text_input::TextInput;

fn main() -> color_eyre::Result<()> {
    color_eyre::install()?;
    let rt = tokio::runtime::Runtime::new()?;
    enable_raw_mode()?;
    let mut application = App::new(&rt);
    ratatui::run(|terminal| application.run(terminal))?;
    disable_raw_mode()?;
    Ok(())
}

#[derive(Clone)]
pub struct HttpResponse {
    pub status: u16,
    pub body: String,
    pub elapsed: Duration,
    pub method_str: String,
}

pub struct App<'a> {
    exit: bool,
    left_section_w: u16,
    top_section_h: u16,
    help_window: bool,
    url_input: TextInput,
    method: Method,
    method_i: usize,
    request_body_input: TextInput,
    response: Option<HttpResponse>,
    loading: bool,
    last_request_intsant: Instant,
    last_request_elapsed: Duration,
    last_request_total: Duration,
    response_scroll: u16,
    requests_history: Vec<(String, String, HttpResponse)>, // (url, body, response)
    selected_tab: usize,
    sidebar_focused: bool,
    history_selected: usize,
    saved_requests: Vec<SavedRequest>,
    saved_selected: usize,
    rt: &'a tokio::runtime::Runtime,
    rx: mpsc::UnboundedReceiver<AppMessage>,
    tx: mpsc::UnboundedSender<AppMessage>,
}

impl<'a> App<'a> {
    fn new(rt: &'a tokio::runtime::Runtime) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            exit: false,
            left_section_w: 40,
            top_section_h: 40,
            help_window: true,
            url_input: TextInput::new("https://dogapi.dog/api/v2/breeds".to_string()),
            request_body_input: TextInput::new_multiline(
                "{\n   \"id\": 1,\n   \"name\": \"boxy\"\n}".to_string(),
            ),
            response: None,
            method: Method::GET,
            method_i: 0,
            loading: false,
            last_request_intsant: Instant::now(),
            last_request_elapsed: Duration::ZERO,
            last_request_total: Duration::ZERO,
            response_scroll: 0,
            requests_history: Vec::new(),
            selected_tab: 0,
            sidebar_focused: false,
            history_selected: 0,
            saved_requests: load_saved_requests(),
            saved_selected: 0,
            rt,
            rx,
            tx,
        }
    }

    fn send_request(&mut self, url: String) {
        self.loading = true;
        self.response = None;
        let method = self.method.clone();
        let method_str = method.as_str().to_string();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let _ = tx.send(AppMessage::RequestStarted);
            let now = Instant::now();
            let result = client.request(method, &url).send().await;
            let elapsed = now.elapsed();
            let resp = match result {
                Ok(r) => {
                    let status = r.status().as_u16();
                    let body = r.text().await.unwrap_or_else(|e| e.to_string());
                    HttpResponse {
                        status,
                        body,
                        elapsed,
                        method_str,
                    }
                }
                Err(e) => HttpResponse {
                    status: 0,
                    body: e.to_string(),
                    elapsed,
                    method_str,
                },
            };
            let _ = tx.send(AppMessage::RequestCompleted(resp));
        });
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        while !self.exit {
            // Check for messages from async tasks
            while let Ok(msg) = self.rx.try_recv() {
                match msg {
                    AppMessage::RequestStarted => {
                        self.last_request_intsant = Instant::now();
                    }
                    AppMessage::RequestCompleted(resp) => {
                        self.last_request_total = self.last_request_intsant.elapsed();
                        self.last_request_elapsed = resp.elapsed;
                        self.response_scroll = 0;
                        self.response = Some(resp);
                        self.requests_history.push((
                            String::from(self.url_input.clone().input),
                            self.request_body_input.value().to_string(),
                            self.response.clone().unwrap_or(self.empty_response()),
                        ));
                        self.loading = false;
                    }
                }
            }

            terminal.draw(|frame| self.render(frame))?;
            self.handle_events()?;
        }

        Ok(())
    }

    fn render(&self, frame: &mut Frame) {
        frame.render_widget(self, frame.area());

        // Cursor positioning needs Frame, can't be done inside Widget trait
        if self.url_input.focused {
            let area = frame.area();
            let main_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![
                    Constraint::Percentage(self.left_section_w),
                    Constraint::Percentage(100 - self.left_section_w),
                ])
                .split(area);
            let control_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![Constraint::Length(3), Constraint::Min(1)])
                .split(main_layout[1]);
            let input_area = control_layout[0];

            #[expect(clippy::cast_possible_truncation)]
            frame.set_cursor_position(Position::new(
                input_area.x + self.url_input.cursor as u16 + 1,
                input_area.y + 1,
            ));
        }

        // Cursor positioning for body input
        if self.request_body_input.focused {
            let area = frame.area();
            let main_layout = Layout::default()
                .direction(Direction::Horizontal)
                .constraints(vec![
                    Constraint::Percentage(self.left_section_w),
                    Constraint::Percentage(100 - self.left_section_w),
                ])
                .split(area);
            let control_layout = Layout::default()
                .direction(Direction::Vertical)
                .constraints(vec![
                    Constraint::Min(3),
                    Constraint::Percentage(self.top_section_h),
                    Constraint::Percentage(100 - self.top_section_h),
                ])
                .split(main_layout[1]);
            let body_area = control_layout[1];

            #[expect(clippy::cast_possible_truncation)]
            frame.set_cursor_position(Position::new(
                body_area.x + self.request_body_input.cursor_col() as u16 + 1,
                body_area.y + self.request_body_input.cursor_row() as u16 + 1,
            ));
        }
    }

    fn handle_events(&mut self) -> std::io::Result<()> {
        if self.loading {
            self.last_request_elapsed = self.last_request_intsant.elapsed();
        }
        if event::poll(Duration::from_millis(8))? {
            match event::read()? {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    if self.url_input.focused {
                        // URL input mode
                        match key_event.code {
                            KeyCode::Esc => self.url_input.blur(),
                            _ => {
                                if self.url_input.handle_key_event(key_event) {
                                    let url = self.url_input.value().to_string();
                                    if !url.is_empty() {
                                        self.send_request(url);
                                    }
                                }
                            }
                        }
                    } else if self.request_body_input.focused {
                        // Body input mode
                        match key_event.code {
                            KeyCode::Esc => self.request_body_input.blur(),
                            _ => {
                                self.request_body_input.handle_key_event(key_event);
                            }
                        }
                    } else if self.sidebar_focused {
                        // Sidebar navigation mode
                        match key_event.code {
                            KeyCode::Esc => self.sidebar_focused = false,
                            KeyCode::Tab => {
                                self.selected_tab = (self.selected_tab + 1) % 3;
                            }
                            KeyCode::BackTab => {
                                self.selected_tab = (self.selected_tab + 2) % 3;
                            }
                            KeyCode::Up => {
                                match self.selected_tab {
                                    0 => {
                                        self.history_selected =
                                            self.history_selected.saturating_sub(1);
                                    }
                                    1 => {
                                        self.saved_selected =
                                            self.saved_selected.saturating_sub(1);
                                    }
                                    _ => {}
                                }
                            }
                            KeyCode::Down => {
                                match self.selected_tab {
                                    0 => {
                                        if !self.requests_history.is_empty() {
                                            self.history_selected = (self.history_selected + 1)
                                                .min(self.requests_history.len() - 1);
                                        }
                                    }
                                    1 => {
                                        if !self.saved_requests.is_empty() {
                                            self.saved_selected = (self.saved_selected + 1)
                                                .min(self.saved_requests.len() - 1);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            KeyCode::Enter => {
                                match self.selected_tab {
                                    0 => self.load_from_history(),
                                    1 => self.load_from_saved(),
                                    _ => {}
                                }
                            }
                            KeyCode::Delete | KeyCode::Char('d') => {
                                match self.selected_tab {
                                    0 => {
                                        if !self.requests_history.is_empty() {
                                            self.requests_history
                                                .remove(self.history_selected);
                                            if self.history_selected > 0
                                                && self.history_selected
                                                    >= self.requests_history.len()
                                            {
                                                self.history_selected -= 1;
                                            }
                                        }
                                    }
                                    1 => {
                                        if !self.saved_requests.is_empty() {
                                            self.saved_requests.remove(self.saved_selected);
                                            if self.saved_selected > 0
                                                && self.saved_selected
                                                    >= self.saved_requests.len()
                                            {
                                                self.saved_selected -= 1;
                                            }
                                            persist_saved_requests(&self.saved_requests);
                                        }
                                    }
                                    _ => {}
                                }
                            }
                            KeyCode::Char('r') => self.save_response(),
                            KeyCode::Char('q') => self.exit(),
                            _ => {}
                        }
                    } else {
                        // Normal mode: resize, navigate, etc.
                        match key_event.code {
                            KeyCode::Char('q') => self.exit(),
                            KeyCode::Char('u') => {
                                self.request_body_input.blur();
                                self.url_input.focus();
                            }
                            KeyCode::Char('b') => {
                                self.url_input.blur();
                                self.request_body_input.focus();
                            }
                            KeyCode::Char('r') => self.save_response(),
                            KeyCode::Char('s') => {
                                self.sidebar_focused = true;
                            }
                            KeyCode::Char('w') => {
                                self.save_current_request();
                            }
                            KeyCode::Tab => {
                                self.selected_tab = (self.selected_tab + 1) % 3;
                            }
                            KeyCode::BackTab => {
                                self.selected_tab = (self.selected_tab + 2) % 3;
                            }
                            KeyCode::Char('m') => self.switch_method(true),
                            KeyCode::Char('n') => self.switch_method(false),
                            KeyCode::Char('h') => self.help_window = !self.help_window,
                            KeyCode::Char('c') => {
                                if self.selected_tab == 0 {
                                    self.requests_history = vec![];
                                    self.history_selected = 0;
                                }
                            }
                            KeyCode::Right => {
                                if self.left_section_w < 100 {
                                    self.left_section_w += 1;
                                }
                            }
                            KeyCode::Left => {
                                if self.left_section_w > 0 {
                                    self.left_section_w -= 1;
                                }
                            }
                            KeyCode::Down => {
                                if self.top_section_h < 100 {
                                    self.top_section_h += 5;
                                }
                            }
                            KeyCode::Up => {
                                if self.top_section_h > 0 {
                                    self.top_section_h -= 5;
                                }
                            }
                            KeyCode::PageDown => {
                                self.response_scroll = self.response_scroll.saturating_add(2);
                            }
                            KeyCode::PageUp => {
                                self.response_scroll = self.response_scroll.saturating_sub(2);
                            }
                            KeyCode::Enter => {
                                let url = self.url_input.value().to_string();
                                if !url.is_empty() {
                                    self.send_request(url);
                                }
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

    fn empty_response(&self) -> HttpResponse {
        HttpResponse {
            status: 1,
            body: String::new(),
            elapsed: Duration::ZERO,
            method_str: String::new(),
        }
    }

    fn format_short_url(&self, url: &str) -> String {
        let parts: Vec<&str> = url.split('/').collect();
        if parts.len() > 2 {
            return parts[parts.len() - 2..].join("/");
        }
        url.to_string()
    }

    fn switch_method(&mut self, forwards: bool) {
        let available_methods = ["GET", "POST", "PUT", "PATCH", "DELETE"];

        if forwards {
            if self.method_i < available_methods.len() - 1 {
                self.method_i += 1;
            } else {
                self.method_i = 0;
            }
        } else {
            if self.method_i > 0 {
                self.method_i -= 1;
            } else {
                self.method_i = available_methods.len() - 1;
            }
        }

        self.method = Method::from_str(available_methods[self.method_i]).unwrap_or(Method::GET);
    }

    fn load_from_history(&mut self) {
        if let Some((url, body, resp)) = self.requests_history.get(self.history_selected) {
            self.url_input = TextInput::new(url.clone());
            self.request_body_input = TextInput::new_multiline(body.clone());
            self.method = Method::from_str(&resp.method_str).unwrap_or(Method::GET);
            let methods = ["GET", "POST", "PUT", "PATCH", "DELETE"];
            self.method_i = methods
                .iter()
                .position(|m| *m == resp.method_str)
                .unwrap_or(0);
            self.response = Some(resp.clone());
            self.response_scroll = 0;
            self.sidebar_focused = false;
        }
    }

    fn load_from_saved(&mut self) {
        if let Some(saved) = self.saved_requests.get(self.saved_selected) {
            self.url_input = TextInput::new(saved.url.clone());
            self.request_body_input = TextInput::new_multiline(saved.body.clone());
            self.method = Method::from_str(&saved.method_str).unwrap_or(Method::GET);
            let methods = ["GET", "POST", "PUT", "PATCH", "DELETE"];
            self.method_i = methods
                .iter()
                .position(|m| *m == saved.method_str)
                .unwrap_or(0);
            self.response = None;
            self.response_scroll = 0;
            self.sidebar_focused = false;
        }
    }

    fn save_current_request(&mut self) {
        let url = self.url_input.value().to_string();
        if url.is_empty() {
            return;
        }
        let name = format!("{} {}", self.method.as_str(), self.format_short_url(&url));
        self.saved_requests.push(SavedRequest {
            name,
            url,
            method_str: self.method.as_str().to_string(),
            body: self.request_body_input.value().to_string(),
        });
        persist_saved_requests(&self.saved_requests);
        self.selected_tab = 1; // switch to Saved tab to show it was saved
    }

    fn save_response(&self) {
        let body = if let Some((_, _, _, Some(resp))) = self.preview_data() {
            &resp.body
        } else if let Some(ref resp) = self.response {
            &resp.body
        } else {
            return;
        };

        let value: serde_json::Value = serde_json::from_str(body)
            .unwrap_or(serde_json::Value::String(body.clone()));
        let pretty = serde_json::to_string_pretty(&value).unwrap_or(body.clone());

        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let filename = format!("response_{}.json", ts);
        let _ = fs::write(&filename, pretty);
    }

    /// Returns (url, method, body, response_body) for the currently highlighted sidebar item.
    fn preview_data(&self) -> Option<(&str, &str, &str, Option<&HttpResponse>)> {
        if !self.sidebar_focused {
            return None;
        }
        match self.selected_tab {
            0 => self
                .requests_history
                .get(self.history_selected)
                .map(|(url, body, resp)| {
                    (url.as_str(), resp.method_str.as_str(), body.as_str(), Some(resp))
                }),
            1 => self
                .saved_requests
                .get(self.saved_selected)
                .map(|s| (s.url.as_str(), s.method_str.as_str(), s.body.as_str(), None)),
            _ => None,
        }
    }

    fn exit(&mut self) {
        self.exit = true;
    }
}

impl Widget for &App<'_> {
    fn render(self, area: ratatui::prelude::Rect, buf: &mut ratatui::prelude::Buffer) {
        let main_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![
                Constraint::Percentage(self.left_section_w),
                Constraint::Percentage(100 - self.left_section_w),
            ])
            .split(area);

        let help_window_share = if self.help_window {
            100 - self.top_section_h
        } else {
            0
        };

        let sidebar_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Percentage(100 - help_window_share),
                Constraint::Percentage(help_window_share),
            ])
            .split(main_layout[0]);

        // Sidebar
        let sidebar_border_style = if self.sidebar_focused {
            Style::new().yellow()
        } else {
            Style::default()
        };

        let sidebar_block = Block::bordered()
            .border_style(sidebar_border_style)
            .title_bottom(if self.sidebar_focused {
                " enter:load d:del esc:back "
            } else {
                " s:focus w:save tab:switch "
            })
            .title_style(Style::new().dark_gray());

        sidebar_block.render(sidebar_layout[0], buf);

        // Render tabs header inside the border
        let tabs_area = ratatui::layout::Rect {
            x: sidebar_layout[0].x + 1,
            y: sidebar_layout[0].y + 1,
            width: sidebar_layout[0].width.saturating_sub(2),
            height: 1,
        };

        Tabs::new(vec!["History", "Saved", "Settings"])
            .style(Color::White)
            .highlight_style(Style::default().yellow().bold())
            .select(self.selected_tab)
            .divider("|")
            .padding(" ", " ")
            .render(tabs_area, buf);

        // Content area inside border, below tabs
        let content_area = ratatui::layout::Rect {
            x: sidebar_layout[0].x + 1,
            y: sidebar_layout[0].y + 2,
            width: sidebar_layout[0].width.saturating_sub(2),
            height: sidebar_layout[0].height.saturating_sub(3),
        };

        match self.selected_tab {
            0 => {
                // History tab
                let items: Vec<ratatui::text::Line> = self
                    .requests_history
                    .iter()
                    .enumerate()
                    .map(|(i, (url, _body, resp))| {
                        let text = format!(
                            "{} {} - {} {:.0?}ms",
                            resp.method_str,
                            self.format_short_url(url),
                            resp.status,
                            resp.elapsed.as_millis()
                        );
                        if self.sidebar_focused && i == self.history_selected {
                            ratatui::text::Line::from(format!("> {}", text))
                                .style(Style::new().yellow().bold())
                        } else {
                            ratatui::text::Line::from(format!("  {}", text))
                        }
                    })
                    .collect();

                if items.is_empty() {
                    Paragraph::new("  No requests yet")
                        .style(Style::new().dark_gray())
                        .render(content_area, buf);
                } else {
                    List::new(items).render(content_area, buf);
                }
            }
            1 => {
                // Saved tab
                let items: Vec<ratatui::text::Line> = self
                    .saved_requests
                    .iter()
                    .enumerate()
                    .map(|(i, saved)| {
                        let text = format!("{} {}", saved.method_str, self.format_short_url(&saved.url));
                        if self.sidebar_focused && i == self.saved_selected {
                            ratatui::text::Line::from(format!("> {}", text))
                                .style(Style::new().yellow().bold())
                        } else {
                            ratatui::text::Line::from(format!("  {}", text))
                        }
                    })
                    .collect();

                if items.is_empty() {
                    Paragraph::new("  No saved requests\n  Press <w> to save current")
                        .style(Style::new().dark_gray())
                        .render(content_area, buf);
                } else {
                    List::new(items).render(content_area, buf);
                }
            }
            2 => {
                // Settings tab
                let settings_text = vec![
                    ratatui::text::Line::from("  curlrs — Terminal HTTP Client")
                        .style(Style::new().yellow().bold()),
                    ratatui::text::Line::from(""),
                    ratatui::text::Line::from(format!(
                        "  Method:    {}",
                        self.method.as_str()
                    )),
                    ratatui::text::Line::from(format!(
                        "  History:   {} requests",
                        self.requests_history.len()
                    )),
                    ratatui::text::Line::from(format!(
                        "  Saved:     {} requests",
                        self.saved_requests.len()
                    )),
                    ratatui::text::Line::from(""),
                    ratatui::text::Line::from(format!(
                        "  Sidebar:   {}%",
                        self.left_section_w
                    ))
                    .style(Style::new().dark_gray()),
                    ratatui::text::Line::from(format!(
                        "  Body/Resp: {}% / {}%",
                        self.top_section_h,
                        100 - self.top_section_h
                    ))
                    .style(Style::new().dark_gray()),
                ];
                Paragraph::new(settings_text).render(content_area, buf);
            }
            _ => {}
        };

        Paragraph::new(vec![
            "press <h>      to toggle this window"
                .bold()
                .yellow()
                .into(),
            "press <arrows> to resize windows".bold().into(),
            "press <u>      to input URL".bold().into(),
            "press <b>      to input request body".bold().into(),
            "press <n>/<m>  to switch method back/forth".bold().into(),
            "press <enter>  to send request".bold().into(),
            "press <s>      to focus sidebar".bold().into(),
            "press <w>      to save current request".bold().into(),
            "press <tab>    to switch sidebar tabs".bold().into(),
            "press <c>      to clear history".bold().into(),
            "press <pgup/dn> to scroll response".bold().into(),
            "press <esc>    to exit input/sidebar mode"
                .bold()
                .light_red()
                .into(),
            "press <q>      to exit".italic().red().into(),
        ])
        .block(
            Block::bordered()
                .title("Help")
                .title_alignment(Alignment::Center)
                .title_style(Style::new().yellow()),
        )
        .wrap(Wrap { trim: false })
        .alignment(Alignment::Left)
        .render(sidebar_layout[1], buf);

        // Control panel: (URL input + method) + request body + response
        let control_layout = Layout::default()
            .direction(Direction::Vertical)
            .constraints(vec![
                Constraint::Min(3),
                Constraint::Percentage(self.top_section_h),
                Constraint::Percentage(100 - self.top_section_h),
            ])
            .split(main_layout[1]);

        let url_and_method_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Percentage(100), Constraint::Min(8)])
            .split(control_layout[0]);

        let preview = self.preview_data();
        let previewing = preview.is_some();

        // URL display
        let (url_text, method_text): (String, String) = if let Some((url, method, _, _)) = preview
        {
            (url.to_string(), method.to_string())
        } else {
            (
                self.url_input.value().to_string(),
                self.method.as_str().to_string(),
            )
        };

        let preview_style = Style::new().fg(Color::Cyan);

        let url_block = Block::bordered()
            .title(if previewing {
                "URL (preview)"
            } else {
                "URL (u to edit)"
            })
            .border_style(if previewing {
                Style::new().cyan()
            } else if self.url_input.focused {
                Style::new().yellow()
            } else {
                Style::default()
            });

        if previewing {
            Paragraph::new(url_text)
                .style(preview_style)
                .block(url_block)
                .render(url_and_method_layout[0], buf);
        } else {
            let url_style = if self.url_input.focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            let sel_style = Style::default().fg(Color::Black).bg(Color::Yellow);
            Paragraph::new(self.url_input.styled_text(url_style, sel_style))
                .block(url_block)
                .render(url_and_method_layout[0], buf);
        }

        // Method area
        let http_method_block = Block::bordered()
            .title(if previewing {
                "method"
            } else {
                "method (m to edit)"
            })
            .border_style(if previewing {
                Style::new().cyan()
            } else {
                Style::default()
            });

        Paragraph::new(method_text)
            .style(if previewing {
                preview_style
            } else {
                Style::new()
            })
            .block(http_method_block)
            .render(url_and_method_layout[1], buf);

        // Request body area
        let body_text = if let Some((_, _, body, _)) = preview {
            body.to_string()
        } else {
            self.request_body_input.value().to_string()
        };

        let request_body_block = Block::bordered()
            .title(if previewing {
                "Request Body (preview)"
            } else {
                "Request Body (b to edit)"
            })
            .title_alignment(Alignment::Center)
            .title_style(if previewing {
                Style::new().cyan()
            } else {
                Style::new().yellow()
            })
            .border_style(if previewing {
                Style::new().cyan()
            } else if self.request_body_input.focused {
                Style::new().yellow()
            } else {
                Style::default()
            });

        if previewing {
            Paragraph::new(body_text)
                .style(preview_style)
                .wrap(Wrap { trim: false })
                .block(request_body_block)
                .render(control_layout[1], buf);
        } else {
            let body_style = if self.request_body_input.focused {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default()
            };
            let body_sel_style = Style::default().fg(Color::Black).bg(Color::Yellow);
            Paragraph::new(
                self.request_body_input
                    .styled_text(body_style, body_sel_style),
            )
            .wrap(Wrap { trim: false })
            .block(request_body_block)
            .render(control_layout[1], buf);
        }

        // Response area
        let (response_title, response_text) = if let Some((_, _, _, resp_opt)) = preview {
            match resp_opt {
                Some(resp) => {
                    let title = format!(
                        "Response (preview) [{}] — RTT {}ms",
                        resp.status,
                        resp.elapsed.as_millis()
                    );
                    let value: serde_json::Value = serde_json::from_str(&resp.body)
                        .unwrap_or(serde_json::Value::String(resp.body.clone()));
                    let body =
                        serde_json::to_string_pretty(&value).unwrap_or(resp.body.clone());
                    (title, body)
                }
                None => ("Response (no history)".to_string(), String::new()),
            }
        } else {
            let dot_or_not = if (self.last_request_elapsed.as_millis() / 125) % 2 == 0 {
                "."
            } else {
                " "
            };
            let title = if self.loading {
                format!(
                    "loading..{} {:.0?}ms elapsed",
                    dot_or_not,
                    self.last_request_elapsed.as_millis()
                )
            } else if let Some(ref resp) = self.response {
                format!(
                    "Response [{}] — RTT {}ms | Total {}ms",
                    resp.status,
                    resp.elapsed.as_millis(),
                    self.last_request_total.as_millis()
                )
            } else {
                "Response".to_string()
            };
            let body = match &self.response {
                Some(resp) => {
                    let value: serde_json::Value = serde_json::from_str(&resp.body)
                        .unwrap_or(serde_json::Value::String(resp.body.clone()));
                    serde_json::to_string_pretty(&value).unwrap_or(resp.body.clone())
                }
                None => String::new(),
            };
            (title, body)
        };

        let has_response = if previewing {
            response_text.len() > 0
        } else {
            self.response.is_some()
        };

        let response_block = Block::bordered()
            .title(response_title)
            .title_alignment(Alignment::Center)
            .title_bottom(if has_response && !self.loading {
                " r:save response "
            } else {
                ""
            })
            .title_style(if previewing {
                Style::new().cyan()
            } else {
                Style::new().yellow()
            })
            .border_style(if previewing {
                Style::new().cyan()
            } else {
                Style::default()
            });

        let response_area = control_layout[2];
        let content_lines = response_text.lines().count() as u16;
        let visible_height = response_area.height.saturating_sub(2);

        Paragraph::new(response_text)
            .style(if previewing {
                preview_style
            } else {
                Style::default()
            })
            .block(response_block)
            .wrap(Wrap { trim: false })
            .scroll((self.response_scroll, 0))
            .render(response_area, buf);

        if content_lines > visible_height {
            let mut scrollbar_state = ScrollbarState::new(content_lines as usize)
                .position(self.response_scroll as usize)
                .viewport_content_length(visible_height as usize);
            ratatui::widgets::StatefulWidget::render(
                Scrollbar::new(ScrollbarOrientation::VerticalRight),
                response_area,
                buf,
                &mut scrollbar_state,
            );
        }
    }
}
