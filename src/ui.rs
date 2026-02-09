pub mod app;
mod error;
mod event;
mod renderer;

pub use error::UiError;

use std::io::stdout;
use std::time::Duration;

use crossterm::event::{Event, KeyEventKind, MouseEventKind};
use crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use app::App;

pub fn run() -> Result<(), UiError> {
    terminal::enable_raw_mode()?;
    crossterm::execute!(stdout(), EnterAlternateScreen, EnableMouseCapture)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new()?;

    loop {
        app.tick();
        terminal.draw(|frame| renderer::render(frame, &mut app))?;

        if let Some(event) = event::poll_event(Duration::from_millis(100))? {
            match event {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    app.handle_key_event(key_event);
                }
                Event::Mouse(mouse_event) => match mouse_event.kind {
                    MouseEventKind::ScrollUp => app.scroll_up(),
                    MouseEventKind::ScrollDown => app.scroll_down(),
                    _ => {}
                },
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    crossterm::execute!(stdout(), DisableMouseCapture, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    Ok(())
}
