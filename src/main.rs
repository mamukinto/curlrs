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
    text::Span,
    widgets::{
        Block, Clear, List, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Tabs,
        Widget, Wrap,
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
    pub headers: Vec<(String, String)>,
    pub elapsed: Duration,
    pub method_str: String,
}

fn status_color(status: u16) -> Color {
    match status {
        200..=299 => Color::Green,
        300..=399 => Color::Yellow,
        400..=499 => Color::LightRed,
        500..=599 => Color::Red,
        0 => Color::Red, // connection error
        _ => Color::White,
    }
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
    show_response_headers: bool,
    url_suggestions: Vec<String>,
    url_suggestion_selected: usize,
    show_suggestions: bool,
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
            url_input: TextInput::new(String::new()),
            request_body_input: TextInput::new_multiline(String::new()),
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
            show_response_headers: false,
            url_suggestions: Vec::new(),
            url_suggestion_selected: 0,
            show_suggestions: false,
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
        let body = self.request_body_input.value().to_string();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
            let _ = tx.send(AppMessage::RequestStarted);
            let now = Instant::now();
            let mut req = client.request(method.clone(), &url);
            if method != Method::GET && !body.is_empty() {
                req = req.header("Content-Type", "application/json").body(body);
            }
            let result = req.send().await;
            let elapsed = now.elapsed();
            let resp = match result {
                Ok(r) => {
                    let status = r.status().as_u16();
                    let headers: Vec<(String, String)> = r
                        .headers()
                        .iter()
                        .map(|(k, v)| {
                            (k.as_str().to_string(), v.to_str().unwrap_or("").to_string())
                        })
                        .collect();
                    let body = r.text().await.unwrap_or_else(|e| e.to_string());
                    HttpResponse {
                        status,
                        body,
                        headers,
                        elapsed,
                        method_str,
                    }
                }
                Err(e) => HttpResponse {
                    status: 0,
                    body: e.to_string(),
                    headers: Vec::new(),
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

            // Render URL suggestions dropdown
            if self.show_suggestions && !self.url_suggestions.is_empty() {
                let max_show = 6.min(self.url_suggestions.len()) as u16;
                let popup_area = ratatui::layout::Rect {
                    x: input_area.x,
                    y: input_area.y + input_area.height,
                    width: input_area.width,
                    height: max_show + 2, // +2 for border
                };

                frame.render_widget(Clear, popup_area);

                let items: Vec<ratatui::text::Line> = self
                    .url_suggestions
                    .iter()
                    .take(6)
                    .enumerate()
                    .map(|(i, url)| {
                        if i == self.url_suggestion_selected {
                            ratatui::text::Line::from(format!("> {}", url))
                                .style(Style::new().yellow().bold())
                        } else {
                            ratatui::text::Line::from(format!("  {}", url))
                        }
                    })
                    .collect();

                let popup_block = Block::bordered()
                    .border_style(Style::new().dark_gray())
                    .title_bottom(" tab:accept esc:dismiss ")
                    .title_style(Style::new().dark_gray());

                frame.render_widget(
                    List::new(items).block(popup_block),
                    popup_area,
                );
            }
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
                            KeyCode::Esc => {
                                if self.show_suggestions {
                                    self.show_suggestions = false;
                                } else {
                                    self.url_input.blur();
                                }
                            }
                            KeyCode::Tab if self.show_suggestions => {
                                self.accept_suggestion();
                            }
                            KeyCode::Down
                                if self.show_suggestions
                                    && !self.url_suggestions.is_empty() =>
                            {
                                self.url_suggestion_selected =
                                    (self.url_suggestion_selected + 1)
                                        .min(self.url_suggestions.len() - 1);
                            }
                            KeyCode::Up if self.show_suggestions => {
                                self.url_suggestion_selected =
                                    self.url_suggestion_selected.saturating_sub(1);
                            }
                            _ => {
                                if self.url_input.handle_key_event(key_event) {
                                    self.show_suggestions = false;
                                    let url = self.url_input.value().to_string();
                                    if !url.is_empty() {
                                        self.send_request(url);
                                    }
                                } else {
                                    self.update_suggestions();
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
                                    2 => self.load_sample_requests(),
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
                            KeyCode::Char('t') => {
                                self.show_response_headers = !self.show_response_headers;
                                self.response_scroll = 0;
                            }
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
            headers: Vec::new(),
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

    fn update_suggestions(&mut self) {
        let query = self.url_input.value().to_lowercase();
        if query.is_empty() {
            self.url_suggestions.clear();
            self.show_suggestions = false;
            return;
        }

        let mut seen = std::collections::HashSet::new();
        let mut suggestions = Vec::new();

        // From history (most recent first)
        for (url, _, _) in self.requests_history.iter().rev() {
            if url.to_lowercase().contains(&query) && seen.insert(url.clone()) {
                suggestions.push(url.clone());
            }
        }
        // From saved
        for saved in &self.saved_requests {
            if saved.url.to_lowercase().contains(&query) && seen.insert(saved.url.clone()) {
                suggestions.push(saved.url.clone());
            }
        }

        // Don't show if only match is exact current input
        if suggestions.len() == 1 && suggestions[0] == self.url_input.value() {
            suggestions.clear();
        }

        self.show_suggestions = !suggestions.is_empty();
        self.url_suggestion_selected = 0;
        self.url_suggestions = suggestions;
    }

    fn accept_suggestion(&mut self) {
        if let Some(url) = self.url_suggestions.get(self.url_suggestion_selected) {
            self.url_input = TextInput::new(url.clone());
            self.url_input.focus();
            self.url_input.cursor = url.len();
            self.url_suggestions.clear();
            self.show_suggestions = false;
        }
    }

    fn load_sample_requests(&mut self) {
        let samples = vec![
            SavedRequest {
                name: "GET dogs".into(),
                url: "https://dogapi.dog/api/v2/breeds".into(),
                method_str: "GET".into(),
                body: String::new(),
            },
            SavedRequest {
                name: "GET random dog fact".into(),
                url: "https://dogapi.dog/api/v2/facts?limit=1".into(),
                method_str: "GET".into(),
                body: String::new(),
            },
            SavedRequest {
                name: "GET todos".into(),
                url: "https://jsonplaceholder.typicode.com/todos/1".into(),
                method_str: "GET".into(),
                body: String::new(),
            },
            SavedRequest {
                name: "POST create post".into(),
                url: "https://jsonplaceholder.typicode.com/posts".into(),
                method_str: "POST".into(),
                body: "{\n  \"title\": \"hello\",\n  \"body\": \"world\",\n  \"userId\": 1\n}"
                    .into(),
            },
            SavedRequest {
                name: "PUT update post".into(),
                url: "https://jsonplaceholder.typicode.com/posts/1".into(),
                method_str: "PUT".into(),
                body: "{\n  \"id\": 1,\n  \"title\": \"updated\",\n  \"body\": \"new body\",\n  \"userId\": 1\n}"
                    .into(),
            },
            SavedRequest {
                name: "DELETE post".into(),
                url: "https://jsonplaceholder.typicode.com/posts/1".into(),
                method_str: "DELETE".into(),
                body: String::new(),
            },
            SavedRequest {
                name: "GET IP info".into(),
                url: "https://httpbin.org/ip".into(),
                method_str: "GET".into(),
                body: String::new(),
            },
            SavedRequest {
                name: "GET headers echo".into(),
                url: "https://httpbin.org/headers".into(),
                method_str: "GET".into(),
                body: String::new(),
            },
        ];
        self.saved_requests.extend(samples);
        persist_saved_requests(&self.saved_requests);
        self.selected_tab = 1; // jump to Saved to show them
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
                        let prefix = if self.sidebar_focused && i == self.history_selected {
                            "> "
                        } else {
                            "  "
                        };
                        let sc = status_color(resp.status);
                        let line = ratatui::text::Line::from(vec![
                            Span::raw(prefix),
                            Span::styled(
                                format!("{} ", resp.method_str),
                                Style::new().bold(),
                            ),
                            Span::raw(format!("{} ", self.format_short_url(url))),
                            Span::styled(
                                format!("{}", resp.status),
                                Style::new().fg(sc).bold(),
                            ),
                            Span::styled(
                                format!(" {:.0?}ms", resp.elapsed.as_millis()),
                                Style::new().dark_gray(),
                            ),
                        ]);
                        if self.sidebar_focused && i == self.history_selected {
                            line.style(Style::new().yellow().bold())
                        } else {
                            line
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
                let load_label = if self.sidebar_focused {
                    ratatui::text::Line::from(
                        "  > Load sample requests (Enter)"
                    )
                    .style(Style::new().yellow().bold())
                } else {
                    ratatui::text::Line::from(
                        "  Load sample requests (focus sidebar, Enter)"
                    )
                    .style(Style::new().cyan())
                };

                let settings_text = vec![
                    ratatui::text::Line::from("  curlrs — Terminal HTTP Client")
                        .style(Style::new().yellow().bold()),
                    ratatui::text::Line::from(""),
                    load_label,
                    ratatui::text::Line::from("  Adds 8 sample API requests to Saved")
                        .style(Style::new().dark_gray()),
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
            "press <t>      to toggle response headers".bold().into(),
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
        let active_resp: Option<&HttpResponse> = if let Some((_, _, _, resp_opt)) = preview {
            resp_opt
        } else {
            self.response.as_ref()
        };

        let view_label = if self.show_response_headers {
            "Headers"
        } else {
            "Body"
        };

        let (response_title, response_text, resp_status) = match active_resp {
            Some(resp) => {
                let title = if previewing {
                    format!(
                        "Response ({}) (preview) [{}] — RTT {}ms",
                        view_label,
                        resp.status,
                        resp.elapsed.as_millis()
                    )
                } else {
                    format!(
                        "Response ({}) [{}] — RTT {}ms | Total {}ms",
                        view_label,
                        resp.status,
                        resp.elapsed.as_millis(),
                        self.last_request_total.as_millis()
                    )
                };
                let text = if self.show_response_headers {
                    resp.headers
                        .iter()
                        .map(|(k, v)| format!("{}: {}", k, v))
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    let value: serde_json::Value = serde_json::from_str(&resp.body)
                        .unwrap_or(serde_json::Value::String(resp.body.clone()));
                    serde_json::to_string_pretty(&value).unwrap_or(resp.body.clone())
                };
                (title, text, Some(resp.status))
            }
            None => {
                if self.loading {
                    let dot_or_not =
                        if (self.last_request_elapsed.as_millis() / 125) % 2 == 0 {
                            "."
                        } else {
                            " "
                        };
                    (
                        format!(
                            "loading..{} {:.0?}ms elapsed",
                            dot_or_not,
                            self.last_request_elapsed.as_millis()
                        ),
                        String::new(),
                        None,
                    )
                } else if previewing {
                    ("Response (no history)".to_string(), String::new(), None)
                } else {
                    ("Response".to_string(), String::new(), None)
                }
            }
        };

        let has_response = active_resp.is_some();

        let status_style = resp_status
            .map(|s| Style::new().fg(status_color(s)))
            .unwrap_or(Style::new().yellow());

        let response_block = Block::bordered()
            .title(response_title)
            .title_alignment(Alignment::Center)
            .title_bottom(if has_response && !self.loading {
                " r:save t:headers "
            } else {
                ""
            })
            .title_style(if previewing {
                Style::new().cyan()
            } else {
                status_style
            })
            .border_style(if previewing {
                Style::new().cyan()
            } else if let Some(s) = resp_status {
                Style::new().fg(status_color(s))
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
