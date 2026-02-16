use std::io::{Stdout, Write, stdout};

use crossterm::{cursor, queue, style, terminal};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use super::app::{App, ChatMessage, MessageRole};

pub const SYSTEM_PREFIX: &str = "Bear> ";
pub const USER_PREFIX: &str = " You> ";

const BEAR_TEXTS: [&str; 7] = [
    "",
    "       () _ _ ()",
    "      / __  __ \\",
    "/@@\\ /  o    o  \\ /@@\\",
    "\\ @ \\|     ^    |/ @ /",
    " \\   \\    ___   /   /",
    "  \\   \\________/   /",
];

const BEAR_COLUMN_WIDTH: usize = 29;
const RIGHT_COLUMN_START: usize = 3;

pub struct TerminalWriter {
    stdout: Stdout,
    live_area_line_count: u16,
    cursor_lines_above_bottom: u16,
    committed_message_count: usize,
    banner_committed: bool,
    terminal_width: u16,
}

impl TerminalWriter {
    pub fn new() -> Result<Self, std::io::Error> {
        let (width, _) = terminal::size()?;
        Ok(Self {
            stdout: stdout(),
            live_area_line_count: 0,
            cursor_lines_above_bottom: 0,
            committed_message_count: 0,
            banner_committed: false,
            terminal_width: width,
        })
    }

    pub fn terminal_width(&self) -> u16 {
        self.terminal_width
    }

    pub fn render(&mut self, app: &App) -> Result<(), std::io::Error> {
        self.erase_live_area()?;
        self.commit_new_output(app)?;
        self.draw_live_area(app)?;
        self.stdout.flush()?;
        Ok(())
    }

    pub fn handle_resize(&mut self, new_width: u16) {
        let _ = self.erase_live_area();
        let _ = self.stdout.flush();
        self.terminal_width = new_width;
    }

    pub fn finalize(&mut self) -> Result<(), std::io::Error> {
        self.erase_live_area()?;
        queue!(self.stdout, style::Print("\r\n"))?;
        self.stdout.flush()?;
        Ok(())
    }

    fn erase_live_area(&mut self) -> Result<(), std::io::Error> {
        if self.live_area_line_count == 0 {
            return Ok(());
        }

        if self.cursor_lines_above_bottom > 0 {
            queue!(self.stdout, cursor::MoveDown(self.cursor_lines_above_bottom))?;
        }

        if self.live_area_line_count > 1 {
            queue!(self.stdout, cursor::MoveUp(self.live_area_line_count - 1))?;
        }
        queue!(
            self.stdout,
            cursor::MoveToColumn(0),
            terminal::Clear(terminal::ClearType::FromCursorDown),
        )?;
        self.live_area_line_count = 0;
        self.cursor_lines_above_bottom = 0;
        Ok(())
    }

    fn commit_new_output(&mut self, app: &App) -> Result<(), std::io::Error> {
        if !self.banner_committed {
            self.write_banner()?;
            self.banner_committed = true;
        }

        while self.committed_message_count < app.messages.len() {
            let message = &app.messages[self.committed_message_count];
            self.write_message(message)?;
            self.committed_message_count += 1;
        }

        Ok(())
    }

    fn write_banner(&mut self) -> Result<(), std::io::Error> {
        let right_column_width = (self.terminal_width as usize).saturating_sub(BEAR_COLUMN_WIDTH);
        let right_column = build_right_column(right_column_width);

        for (i, bear_text) in BEAR_TEXTS.iter().enumerate() {
            let padded = format!("{:<width$}", bear_text, width = BEAR_COLUMN_WIDTH);

            queue!(
                self.stdout,
                style::SetForegroundColor(style::Color::Yellow),
                style::Print(padded),
            )?;

            let right_offset = i.wrapping_sub(RIGHT_COLUMN_START);
            if let Some((text, color, bold)) = right_column.get(right_offset) {
                queue!(self.stdout, style::SetForegroundColor(*color))?;
                if *bold {
                    queue!(self.stdout, style::SetAttribute(style::Attribute::Bold))?;
                }
                queue!(self.stdout, style::Print(text))?;
                if *bold {
                    queue!(self.stdout, style::SetAttribute(style::Attribute::NormalIntensity))?;
                }
            }

            queue!(
                self.stdout,
                style::ResetColor,
                style::Print("\r\n"),
            )?;
        }

        let separator = "─".repeat(self.terminal_width as usize);
        queue!(
            self.stdout,
            style::SetForegroundColor(style::Color::DarkGrey),
            style::Print(separator),
            style::ResetColor,
            style::Print("\r\n"),
        )?;

        Ok(())
    }

    fn write_message(&mut self, message: &ChatMessage) -> Result<(), std::io::Error> {
        let (prefix, prefix_color, text_color) = match message.role {
            MessageRole::System => (SYSTEM_PREFIX, style::Color::Cyan, style::Color::Reset),
            MessageRole::User => (USER_PREFIX, style::Color::Green, style::Color::Green),
        };

        let padding = " ".repeat(prefix.len());
        let text_width = (self.terminal_width as usize).saturating_sub(prefix.len());
        let mut is_first = true;

        for text_line in message.content.lines() {
            let is_bold_line =
                matches!(message.role, MessageRole::System) && is_tool_label(text_line);

            for visual_line in wrap_text_by_char_width(text_line, text_width) {
                if is_first {
                    queue!(
                        self.stdout,
                        style::SetForegroundColor(prefix_color),
                        style::SetAttribute(style::Attribute::Bold),
                        style::Print(prefix),
                        style::SetAttribute(style::Attribute::NormalIntensity),
                    )?;
                    is_first = false;
                } else {
                    queue!(self.stdout, style::Print(&padding))?;
                }

                queue!(self.stdout, style::SetForegroundColor(text_color))?;
                if is_bold_line {
                    queue!(self.stdout, style::SetAttribute(style::Attribute::Bold))?;
                }
                queue!(self.stdout, style::Print(&visual_line))?;
                if is_bold_line {
                    queue!(self.stdout, style::SetAttribute(style::Attribute::NormalIntensity))?;
                }
                queue!(
                    self.stdout,
                    style::ResetColor,
                    style::Print("\r\n"),
                )?;
            }
        }

        queue!(self.stdout, style::Print("\r\n"))?;
        Ok(())
    }

    fn draw_live_area(&mut self, app: &App) -> Result<(), std::io::Error> {
        let mut line_count: u16 = 0;
        let mut cursor_position_on_screen: Option<(u16, u16)> = None;

        if app.is_mode_selection() {
            line_count += write_mode_selection_menu(
                &mut self.stdout,
                app.selected_mode_index(),
            )?;
        } else if app.is_waiting_for_input() {
            let result = write_input_lines(
                &mut self.stdout,
                &app.input_buffer,
                app.cursor_position,
                self.terminal_width,
            )?;
            line_count += result.line_count;
            cursor_position_on_screen = Some((result.cursor_row, result.cursor_screen_col));
        } else if app.is_thinking() {
            queue!(
                self.stdout,
                style::SetForegroundColor(style::Color::Cyan),
                style::SetAttribute(style::Attribute::Bold),
                style::Print(SYSTEM_PREFIX),
                style::SetAttribute(style::Attribute::NormalIntensity),
                style::SetForegroundColor(style::Color::Yellow),
                style::Print(app.thinking_indicator()),
                style::ResetColor,
                style::Print("\r\n"),
            )?;
            line_count += 1;
        } else {
            queue!(self.stdout, style::Print("\r\n"))?;
            line_count += 1;
        }

        let separator = "─".repeat(self.terminal_width as usize);
        queue!(
            self.stdout,
            style::SetForegroundColor(style::Color::DarkGrey),
            style::Print(separator),
            style::Print("\r\n"),
            style::Print(app.help_text()),
            style::ResetColor,
        )?;
        line_count += 2;

        if let Some((cursor_row, cursor_col)) = cursor_position_on_screen {
            let bottom_row = line_count - 1;
            let lines_up = bottom_row - cursor_row;
            if lines_up > 0 {
                queue!(self.stdout, cursor::MoveUp(lines_up))?;
            }
            queue!(
                self.stdout,
                cursor::MoveToColumn(cursor_col),
                cursor::Show,
            )?;
            self.cursor_lines_above_bottom = lines_up;
        } else {
            queue!(self.stdout, cursor::Hide)?;
            self.cursor_lines_above_bottom = 0;
        }

        self.live_area_line_count = line_count;
        Ok(())
    }
}

struct InputRenderResult {
    line_count: u16,
    cursor_row: u16,
    cursor_screen_col: u16,
}

fn write_input_lines(
    stdout: &mut Stdout,
    input_buffer: &str,
    cursor_position: usize,
    max_width: u16,
) -> Result<InputRenderResult, std::io::Error> {
    let cursor_reserved = 1;
    let text_width = (max_width as usize).saturating_sub(USER_PREFIX.len() + cursor_reserved);
    let prefix_display_width = USER_PREFIX.len() as u16;

    let logical_lines: Vec<&str> = input_buffer.split('\n').collect();
    let mut line_count: u16 = 0;
    let mut global_char_offset = 0;
    let mut is_first_visual_line = true;
    let mut cursor_row: u16 = 0;
    let mut cursor_screen_col: u16 = prefix_display_width;

    for (logical_idx, logical_line) in logical_lines.iter().enumerate() {
        let visual_lines = wrap_text_by_char_width(logical_line, text_width);
        let visual_line_count = visual_lines.len();
        let mut line_char_offset = 0;

        for (visual_idx, visual_text) in visual_lines.iter().enumerate() {
            let visual_char_count = visual_text.chars().count();
            let visual_start = global_char_offset + line_char_offset;
            let is_last_visual_of_logical = visual_idx == visual_line_count - 1;

            if let Some(col) = cursor_column_on_visual_line(
                cursor_position,
                visual_start,
                visual_char_count,
                is_last_visual_of_logical,
            ) {
                let before_cursor: String = visual_text.chars().take(col).collect();
                cursor_row = line_count;
                cursor_screen_col = prefix_display_width + before_cursor.width() as u16;
            }

            if is_first_visual_line {
                queue!(
                    stdout,
                    style::SetForegroundColor(style::Color::Green),
                    style::SetAttribute(style::Attribute::Bold),
                    style::Print(USER_PREFIX),
                    style::SetAttribute(style::Attribute::NormalIntensity),
                )?;
            } else {
                let padding = " ".repeat(USER_PREFIX.len());
                queue!(stdout, style::Print(padding))?;
            }

            queue!(
                stdout,
                style::SetForegroundColor(style::Color::Green),
                style::Print(visual_text),
                style::ResetColor,
                style::Print("\r\n"),
            )?;

            line_count += 1;
            line_char_offset += visual_char_count;
            is_first_visual_line = false;
        }

        global_char_offset += logical_line.chars().count();
        if logical_idx < logical_lines.len() - 1 {
            global_char_offset += 1; // '\n'
        }
    }

    Ok(InputRenderResult {
        line_count,
        cursor_row,
        cursor_screen_col,
    })
}

/// 커서가 이 visual line 위에 있으면 해당 컬럼을, 아니면 None을 반환.
fn cursor_column_on_visual_line(
    cursor_position: usize,
    visual_start: usize,
    visual_char_count: usize,
    is_last_visual_of_logical: bool,
) -> Option<usize> {
    let visual_end = visual_start + visual_char_count;

    if cursor_position >= visual_start && cursor_position < visual_end {
        return Some(cursor_position - visual_start);
    }

    if cursor_position == visual_end && is_last_visual_of_logical {
        return Some(visual_char_count);
    }

    None
}

pub(super) fn wrap_text_by_char_width(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![text.to_string()];
    }

    let mut result = Vec::new();
    let mut current_line = String::new();
    let mut current_width: usize = 0;

    for ch in text.chars() {
        let char_width = UnicodeWidthChar::width(ch).unwrap_or(0);
        if current_width + char_width > max_width && current_width > 0 {
            result.push(current_line);
            current_line = String::new();
            current_width = 0;
        }
        current_line.push(ch);
        current_width += char_width;
    }

    result.push(current_line);
    result
}

fn is_tool_label(line: &str) -> bool {
    line.starts_with("[Tool Call:") || line.starts_with("[Tool Result]")
}

fn build_right_column(max_width: usize) -> Vec<(String, style::Color, bool)> {
    let slogan_lines = wrap_words(
        "Bear: The AI developer that saves your time.",
        max_width,
    );
    let description_lines = wrap_words(
        "Bear, your AI developer, does the heavy lifting for you; \
         you just collect your paycheck and don't worry about a thing.",
        max_width,
    );

    let mut lines: Vec<(String, style::Color, bool)> = Vec::new();
    for line in &slogan_lines {
        lines.push((line.clone(), style::Color::Cyan, true));
    }
    if !slogan_lines.is_empty() && !description_lines.is_empty() {
        lines.push((String::new(), style::Color::Reset, false));
    }
    for line in &description_lines {
        lines.push((line.clone(), style::Color::DarkGrey, false));
    }
    lines
}

const MODE_LABELS: [&str; 3] = [
    "1. 처음부터 만들기",
    "2. 스펙 파일 있음",
    "3. 스펙 및 플랜 파일 있음",
];

fn write_mode_selection_menu(
    stdout: &mut Stdout,
    selected_index: usize,
) -> Result<u16, std::io::Error> {
    let mut line_count: u16 = 0;

    for (i, label) in MODE_LABELS.iter().enumerate() {
        let is_selected = i == selected_index;
        let marker = if is_selected { "> " } else { "  " };

        queue!(
            stdout,
            style::SetForegroundColor(if is_selected {
                style::Color::Cyan
            } else {
                style::Color::Reset
            }),
        )?;
        if is_selected {
            queue!(stdout, style::SetAttribute(style::Attribute::Bold))?;
        }
        queue!(
            stdout,
            style::Print(marker),
            style::Print(label),
        )?;
        if is_selected {
            queue!(stdout, style::SetAttribute(style::Attribute::NormalIntensity))?;
        }
        queue!(
            stdout,
            style::ResetColor,
            style::Print("\r\n"),
        )?;
        line_count += 1;
    }

    Ok(line_count)
}

fn wrap_words(text: &str, max_width: usize) -> Vec<String> {
    if max_width == 0 {
        return vec![];
    }

    let mut lines = Vec::new();
    let mut current_line = String::new();

    for word in text.split_whitespace() {
        if current_line.is_empty() {
            current_line.push_str(word);
        } else if current_line.len() + 1 + word.len() <= max_width {
            current_line.push(' ');
            current_line.push_str(word);
        } else {
            lines.push(current_line);
            current_line = word.to_string();
        }
    }

    if !current_line.is_empty() {
        lines.push(current_line);
    }

    lines
}
