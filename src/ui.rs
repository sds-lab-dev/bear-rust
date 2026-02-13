pub mod app;
mod clarification;
mod error;
mod event;
mod renderer;
mod planning;
mod session_naming;
mod spec_writing;

pub use error::UiError;

use std::io::stdout;
use std::time::Duration;

use crossterm::cursor;
use crossterm::event::{
    DisableBracketedPaste, EnableBracketedPaste,
    Event, KeyEventKind, KeyboardEnhancementFlags,
    PopKeyboardEnhancementFlags, PushKeyboardEnhancementFlags,
};
use crossterm::terminal;

use crate::config::Config;
use app::App;
use renderer::TerminalWriter;

pub fn run(config: Config) -> Result<(), UiError> {
    terminal::enable_raw_mode()?;
    crossterm::execute!(
        stdout(),
        EnableBracketedPaste,
        cursor::Hide,
        cursor::SetCursorStyle::SteadyBlock,
    )?;

    let keyboard_enhancement_enabled = terminal::supports_keyboard_enhancement()
        .unwrap_or(false);

    if keyboard_enhancement_enabled {
        crossterm::execute!(
            stdout(),
            PushKeyboardEnhancementFlags(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        )?;
    }

    let mut app = App::new(config)?;
    app.set_keyboard_enhancement_enabled(keyboard_enhancement_enabled);

    let mut writer = TerminalWriter::new()?;
    app.terminal_width = writer.terminal_width();

    loop {
        app.tick();
        app.terminal_width = writer.terminal_width();
        writer.render(&app)?;

        if let Some(event) = event::poll_event(Duration::from_millis(100))? {
            match event {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    app.handle_key_event(key_event);
                }
                Event::Paste(text) => {
                    app.handle_paste(text);
                }
                Event::Resize(width, _) => {
                    writer.handle_resize(width);
                    app.terminal_width = width;
                }
                _ => {}
            }
        }

        if app.should_quit {
            break;
        }
    }

    writer.finalize()?;

    if keyboard_enhancement_enabled {
        crossterm::execute!(stdout(), PopKeyboardEnhancementFlags)?;
    }

    crossterm::execute!(
        stdout(),
        cursor::Show,
        cursor::SetCursorStyle::DefaultUserShape,
        DisableBracketedPaste,
    )?;
    terminal::disable_raw_mode()?;

    Ok(())
}
