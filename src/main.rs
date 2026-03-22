// use std::{env::args, time::Instant};

// #[tokio::main]
// async fn main() {
//     let now = Instant::now();

//     let url = args().nth(1).unwrap_or("https://dogapi.dog/api/v2/breeds".to_string());

//     let client = reqwest::Client::new();

//     let response = client.get(url)
//         .send().await.unwrap();

//     let elapsed = now.elapsed();

//     println!("status: {} in {:.2}ms", response.status(), elapsed.as_millis());
// }

use std::io::{self, stdout};

use crossterm::{event::{self, Event, KeyCode}, execute, terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode}};
use ratatui::{prelude::*, widgets::*};

fn main() -> io::Result<()> {
    // setup
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    // main loop
    loop {
        terminal.draw(|frame| {
            let block = Block::default().title("curlrs").borders(Borders::ALL);
            frame.render_widget(&block, frame.area());
        })?;

        // quit on 'q'
        if let Event::Key(key) = event::read()? {
            if key.code == KeyCode::Char('q') {
                break;
            }
        }
    }

    while event::poll(std::time::Duration::from_millis(50))? {
        event::read()?;
    }

    loop {
        terminal.draw(|frame| {
            let block = Block::default().title("zxczxc!!").borders(Borders::ALL);
            frame.render_widget(&block, frame.area());
        })?;

        // quit on 'q'
        if let Event::Key(key) = event::read()? {
            if key.code == KeyCode::Char('q') {
                break;
            }
        }
    }

    // cleanup
    disable_raw_mode()?;
    execute!(stdout(), LeaveAlternateScreen)?;
    Ok(())
}