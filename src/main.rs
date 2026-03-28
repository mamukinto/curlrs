use std::{
    str::FromStr,
    time::{Duration, Instant},
    vec,
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    terminal::{disable_raw_mode, enable_raw_mode},
};

use ratatui::{
    DefaultTerminal, Frame,
    layout::{Alignment, Constraint, Direction, Layout, Position},
    style::{Color, Style, Stylize},
    widgets::{Block, BorderType, List, Paragraph, Widget},
};

use reqwest::Method;
use tokio::sync::mpsc;

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
    requests_history: Vec<(String, HttpResponse)>,
    rt: &'a tokio::runtime::Runtime,
    rx: mpsc::UnboundedReceiver<HttpResponse>,
    tx: mpsc::UnboundedSender<HttpResponse>,
}

impl<'a> App<'a> {
    fn new(rt: &'a tokio::runtime::Runtime) -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            exit: false,
            left_section_w: 25,
            top_section_h: 30,
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
            requests_history: Vec::new(),
            rt,
            rx,
            tx,
        }
    }

    fn send_request(&mut self, url: String) {
        self.loading = true;
        self.last_request_intsant = Instant::now();
        self.response = None;
        let method = self.method.clone();
        let method_str = method.as_str().to_string();
        let tx = self.tx.clone();
        self.rt.spawn(async move {
            let client = reqwest::Client::new();
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
            let _ = tx.send(resp);
        });
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> std::io::Result<()> {
        while !self.exit {
            // Check for completed HTTP responses
            if let Ok(resp) = self.rx.try_recv() {
                self.response = Some(resp);

                self.requests_history.push((
                    String::from(self.url_input.clone().input),
                    self.response.clone().unwrap_or(self.empty_response()),
                ));

                self.loading = false;
                self.last_request_elapsed = self.last_request_intsant.elapsed();
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
        if event::poll(Duration::from_millis(5))? {
            match event::read()? {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    if self.url_input.focused {
                        // URL input mode
                        match key_event.code {
                            KeyCode::Esc => self.url_input.blur(),
                            code => {
                                if self.url_input.handle_key_event(code) {
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
                            code => {
                                self.request_body_input.handle_key_event(code);
                            }
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
                            KeyCode::Char('m') => self.switch_method(true),
                            KeyCode::Char('n') => self.switch_method(false),
                            KeyCode::Char('h') => self.help_window = !self.help_window,
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
        let requests_block = Block::bordered()
            .title("Requests History")
            .title_position(ratatui::widgets::TitlePosition::Top)
            .title_alignment(Alignment::Center)
            .title_style(Style::new().yellow())
            .border_style(Style::default());

        let items = self
            .requests_history
            .clone()
            .iter()
            .map(|(k, v)| {
                format!(
                    "{}: {} - {} in {:.0?}",
                    v.method_str,
                    self.format_short_url(k),
                    v.status,
                    v.elapsed
                )
            })
            .collect::<Vec<_>>();

        List::new(items)
            .block(requests_block)
            .style(Style::default())
            .highlight_style(Style::new().italic())
            .highlight_symbol(">>")
            .repeat_highlight_symbol(true)
            .render(sidebar_layout[0], buf);

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
            "press <esc>    to exit input mode"
                .bold()
                .light_red()
                .into(),
            "press <q>      to exit".italic().red().into(),
        ])
        .block(
            Block::bordered()
                .title("Help")
                .border_type(BorderType::Double),
        )
        .alignment(Alignment::Left)
        .render(sidebar_layout[1], buf);

        // Control panel: (URL input + method) + requsest body + response
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

        // URL input
        let url_block =
            Block::bordered()
                .title("URL (u to edit)")
                .border_style(if self.url_input.focused {
                    Style::new().yellow()
                } else {
                    Style::default()
                });
        let url_style = if self.url_input.focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };
        Paragraph::new(self.url_input.input.as_str())
            .style(url_style)
            .block(url_block)
            .render(url_and_method_layout[0], buf);

        // htttp method area

        let http_method_block = Block::bordered().title("method (m to edit)");

        Paragraph::new(self.method.as_str())
            .style(Style::new())
            .block(http_method_block)
            .render(url_and_method_layout[1], buf);

        // Request body area
        let request_body_block = Block::bordered()
            .title("Request Body (b to edit)")
            .title_alignment(Alignment::Center)
            .title_style(Style::new().yellow())
            .border_style(if self.request_body_input.focused {
                Style::new().yellow()
            } else {
                Style::default()
            });

        let body_style = if self.request_body_input.focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default()
        };

        Paragraph::new(self.request_body_input.input.as_str())
            .style(body_style)
            .block(request_body_block)
            .render(control_layout[1], buf);

        let dot_or_not = if (self.last_request_elapsed.as_millis() / 120) % 2 == 0 {
            "."
        } else {
            " "
        };

        // Response area
        let response_title = if self.loading {
            format!(
                "loading..{} {:.0?}ms elapsed",
                dot_or_not,
                (self.last_request_elapsed.as_millis())
            )
        } else if let Some(ref resp) = self.response {
            format!("Response in ({:.0?}) - [{}] ", resp.elapsed, resp.status,)
        } else {
            "Response".to_string()
        };

        let response_block = Block::bordered()
            .title(response_title)
            .title_alignment(Alignment::Center)
            .title_style(Style::new().yellow())
            .border_style(Style::default());

        let response_text = match &self.response {
            Some(resp) => {
                let value: serde_json::Value = serde_json::from_str(&resp.body)
                    .unwrap_or(serde_json::Value::String(resp.body.clone()));
                serde_json::to_string_pretty(&value).unwrap_or(resp.body.clone())
            }
            None => String::new(),
        };

        Paragraph::new(response_text)
            .block(response_block)
            .render(control_layout[2], buf);
    }
}
