pub mod app;
mod clarification;
mod error;
mod event;
mod renderer;
mod planning;
mod spec_writing;

pub use error::UiError;

use std::io::stdout;
use std::time::Duration;

use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste,
    Event, KeyEventKind, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal::{self, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::Terminal;
use ratatui::backend::CrosstermBackend;

use crate::config::Config;
use app::App;

pub fn run(config: Config) -> Result<(), UiError> {
    terminal::enable_raw_mode()?;
    crossterm::execute!(stdout(), EnterAlternateScreen, EnableBracketedPaste)?;

    let backend = CrosstermBackend::new(stdout());
    let mut terminal = Terminal::new(backend)?;

    let mut app = App::new(config)?;

    let keyboard_enhancement_enabled = crossterm::terminal::supports_keyboard_enhancement()
        .unwrap_or(false);

    if keyboard_enhancement_enabled {
        crossterm::execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }

    app.set_keyboard_enhancement_enabled(keyboard_enhancement_enabled);

    loop {
        app.tick();
        terminal.draw(|frame| renderer::render(frame, &mut app))?;

        if let Some(event) = event::poll_event(Duration::from_millis(100))? {
            match event {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    app.handle_key_event(key_event);
                }
                Event::Paste(text) => {
                    app.handle_paste(text);
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    if keyboard_enhancement_enabled {
        crossterm::execute!(stdout(), PopKeyboardEnhancementFlags)?;
    }

    crossterm::execute!(stdout(), DisableBracketedPaste, LeaveAlternateScreen)?;
    terminal::disable_raw_mode()?;

    Ok(())
}
