use ratatui::Frame;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::app::{App, ChatMessage, MessageRole};

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

pub fn render(frame: &mut Frame, app: &mut App) {
    let area = frame.area();

    let separator_style = Style::default().fg(Color::DarkGray);
    let user_prefix_style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let user_text_style = Style::default().fg(Color::Green);

    let mut lines: Vec<Line> = Vec::new();

    lines.extend(build_banner_lines(area.width));

    for message in &app.messages {
        lines.extend(format_message(message));
    }

    if app.is_waiting_for_input() {
        let cursor = if app.cursor_visible { "█" } else { " " };
        lines.push(Line::from(vec![
            Span::styled("You> ", user_prefix_style),
            Span::styled(app.input_buffer.as_str(), user_text_style),
            Span::styled(cursor, user_text_style),
        ]));
    } else {
        lines.push(Line::from(""));
    }

    let separator = "─".repeat(area.width as usize);
    lines.push(Line::from(Span::styled(separator, separator_style)));
    lines.push(Line::from(Span::styled(
        app.help_text(),
        separator_style,
    )));

    let total_lines = lines.len() as u16;
    let max_scroll = total_lines.saturating_sub(area.height);
    let scroll = max_scroll.saturating_sub(app.scroll_offset);

    frame.render_widget(
        Paragraph::new(lines).scroll((scroll, 0)),
        area,
    );

    // lines 소비 후 빌림이 해제되어 scroll_offset 수정 가능.
    app.scroll_offset = app.scroll_offset.min(max_scroll);
}

fn build_banner_lines(width: u16) -> Vec<Line<'static>> {
    let bear_style = Style::default().fg(Color::Yellow);
    let separator_style = Style::default().fg(Color::DarkGray);

    let right_column_width = (width as usize).saturating_sub(BEAR_COLUMN_WIDTH);
    let right_column = build_right_column(right_column_width);

    let mut lines: Vec<Line> = Vec::new();

    for (i, bear_text) in BEAR_TEXTS.iter().enumerate() {
        let padded = format!("{:<width$}", bear_text, width = BEAR_COLUMN_WIDTH);
        let mut spans = vec![Span::styled(padded, bear_style)];

        let right_offset = i.wrapping_sub(RIGHT_COLUMN_START);
        if let Some((text, color, bold)) = right_column.get(right_offset) {
            let mut style = Style::default().fg(*color);
            if *bold {
                style = style.add_modifier(Modifier::BOLD);
            }
            spans.push(Span::styled(text.clone(), style));
        }

        lines.push(Line::from(spans));
    }

    let separator = "─".repeat(width as usize);
    lines.push(Line::from(Span::styled(separator, separator_style)));

    lines
}

fn format_message(message: &ChatMessage) -> Vec<Line<'static>> {
    let system_prefix_style = Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD);
    let user_prefix_style = Style::default()
        .fg(Color::Green)
        .add_modifier(Modifier::BOLD);
    let user_text_style = Style::default().fg(Color::Green);

    let mut lines: Vec<Line<'static>> = Vec::new();

    match message.role {
        MessageRole::System => {
            for (i, text_line) in message.content.lines().enumerate() {
                if i == 0 {
                    lines.push(Line::from(vec![
                        Span::styled("Bear> ", system_prefix_style),
                        Span::styled(text_line.to_string(), Style::default()),
                    ]));
                } else {
                    lines.push(Line::from(format!("      {}", text_line)));
                }
            }
        }
        MessageRole::User => {
            lines.push(Line::from(vec![
                Span::styled("You> ", user_prefix_style),
                Span::styled(message.content.clone(), user_text_style),
            ]));
        }
    }
    lines.push(Line::from(""));

    lines
}

fn build_right_column(max_width: usize) -> Vec<(String, Color, bool)> {
    let slogan_lines = wrap_words(
        "Bear: The AI developer that saves your time.",
        max_width,
    );
    let description_lines = wrap_words(
        "Bear, your AI developer, does the heavy lifting for you; \
         you just collect your paycheck and don't worry about a thing.",
        max_width,
    );

    let mut lines: Vec<(String, Color, bool)> = Vec::new();
    for line in &slogan_lines {
        lines.push((line.clone(), Color::Cyan, true));
    }
    if !slogan_lines.is_empty() && !description_lines.is_empty() {
        lines.push((String::new(), Color::Reset, false));
    }
    for line in &description_lines {
        lines.push((line.clone(), Color::DarkGray, false));
    }
    lines
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
