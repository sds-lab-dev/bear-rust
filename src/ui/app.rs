use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::error::UiError;

const CURSOR_BLINK_INTERVAL: Duration = Duration::from_millis(500);

pub enum MessageRole {
    System,
    User,
}

pub struct ChatMessage {
    pub role: MessageRole,
    pub content: String,
}

enum InputMode {
    WorkspaceConfirm,
    RequirementsInput,
    Done,
}

pub struct App {
    pub messages: Vec<ChatMessage>,
    input_mode: InputMode,
    pub input_buffer: String,
    pub confirmed_workspace: Option<PathBuf>,
    pub confirmed_requirements: Option<String>,
    pub should_quit: bool,
    pub cursor_visible: bool,
    cursor_blink_at: Instant,
    current_directory: PathBuf,
    pub scroll_offset: u16,
    keyboard_enhancement_enabled: bool,
}

impl App {
    pub fn new() -> Result<Self, UiError> {
        let current_directory = std::env::current_dir()?;

        let initial_message = format!(
            "워크스페이스: {}\n새로운 워크스페이스 절대 경로를 입력하거나, Enter를 눌러 현재 워크스페이스를 사용하세요.",
            current_directory.display()
        );

        let messages = vec![ChatMessage {
            role: MessageRole::System,
            content: initial_message,
        }];

        Ok(Self {
            messages,
            input_mode: InputMode::WorkspaceConfirm,
            input_buffer: String::new(),
            confirmed_workspace: None,
            confirmed_requirements: None,
            should_quit: false,
            cursor_visible: true,
            cursor_blink_at: Instant::now(),
            current_directory,
            scroll_offset: 0,
            keyboard_enhancement_enabled: false,
        })
    }

    pub fn handle_key_event(&mut self, key_event: KeyEvent) {
        self.reset_cursor_blink();

        match self.input_mode {
            InputMode::WorkspaceConfirm => self.handle_workspace_confirm(key_event),
            InputMode::RequirementsInput => self.handle_requirements_input(key_event),
            InputMode::Done => self.handle_done(key_event),
        }
    }

    pub fn tick(&mut self) {
        if self.cursor_blink_at.elapsed() >= CURSOR_BLINK_INTERVAL {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_blink_at = Instant::now();
        }
    }

    fn reset_cursor_blink(&mut self) {
        self.cursor_visible = true;
        self.cursor_blink_at = Instant::now();
    }

    pub fn scroll_up(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_add(1);
    }

    pub fn scroll_down(&mut self) {
        self.scroll_offset = self.scroll_offset.saturating_sub(1);
    }

    pub fn set_keyboard_enhancement_enabled(&mut self, enabled: bool) {
        self.keyboard_enhancement_enabled = enabled;
    }

    pub fn is_waiting_for_input(&self) -> bool {
        matches!(
            self.input_mode,
            InputMode::WorkspaceConfirm | InputMode::RequirementsInput
        )
    }

    pub fn help_text(&self) -> &str {
        match self.input_mode {
            InputMode::WorkspaceConfirm => "[Enter] 확인  [Esc] 종료",
            InputMode::RequirementsInput => {
                if self.keyboard_enhancement_enabled {
                    "[Enter] 제출  [Shift+Enter] 줄바꿈  [Esc] 종료"
                } else {
                    "[Enter] 제출  [Alt+Enter] 줄바꿈  [Esc] 종료"
                }
            }
            InputMode::Done => "[Esc] 종료",
        }
    }

    fn handle_workspace_confirm(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Enter => {
                let trimmed = self.input_buffer.trim().to_string();
                let workspace = if trimmed.is_empty() {
                    self.current_directory.clone()
                } else {
                    let path = PathBuf::from(&trimmed);
                    if let Some(error_message) = validate_workspace_path(&path) {
                        self.add_user_message(&trimmed);
                        self.add_system_message(&error_message);
                        self.input_buffer.clear();
                        return;
                    }
                    path
                };
                self.add_user_message(&workspace.display().to_string());
                self.add_system_message(&format!(
                    "워크스페이스가 설정되었습니다: {}",
                    workspace.display()
                ));
                self.confirmed_workspace = Some(workspace);
                self.input_buffer.clear();
                self.transition_to_requirements_input();
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    fn handle_requirements_input(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Enter if self.is_newline_modifier(key_event.modifiers) => {
                self.input_buffer.push('\n');
            }
            KeyCode::Enter => {
                self.submit_requirements();
            }
            KeyCode::Backspace => {
                self.input_buffer.pop();
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Char(c) => {
                self.input_buffer.push(c);
            }
            _ => {}
        }
    }

    fn handle_done(&mut self, key_event: KeyEvent) {
        if key_event.code == KeyCode::Esc {
            self.should_quit = true;
        }
    }

    fn transition_to_requirements_input(&mut self) {
        self.add_system_message("구현할 요구사항을 입력하세요. Shitft + Enter로 여러 줄 입력이 가능합니다.");
        self.input_mode = InputMode::RequirementsInput;
    }

    fn submit_requirements(&mut self) {
        let requirements = self.input_buffer.trim().to_string();
        if requirements.is_empty() {
            return;
        }

        self.add_user_message(&requirements);
        self.confirmed_requirements = Some(requirements);
        self.input_buffer.clear();
        self.add_system_message("요구사항이 접수되었습니다. 잠시만 기다려 주세요.");
        self.input_mode = InputMode::Done;
    }

    fn is_newline_modifier(&self, modifiers: KeyModifiers) -> bool {
        if self.keyboard_enhancement_enabled {
            modifiers.contains(KeyModifiers::SHIFT)
        } else {
            modifiers.contains(KeyModifiers::ALT)
        }
    }

    fn add_system_message(&mut self, content: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: content.to_string(),
        });
        self.scroll_offset = 0;
    }

    fn add_user_message(&mut self, content: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: content.to_string(),
        });
        self.scroll_offset = 0;
    }
}

/// 워크스페이스 경로 검증. 문제가 있으면 에러 메시지를, 없으면 None을 반환.
fn validate_workspace_path(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return Some(format!(
            "절대 경로를 입력해야 합니다: {}\n새로운 워크스페이스 절대 경로를 입력하거나, Enter를 눌러 현재 워크스페이스를 사용하세요.",
            path.display()
        ));
    }
    if !path.is_dir() {
        return Some(format!(
            "존재하지 않는 디렉토리입니다: {}\n새로운 워크스페이스 절대 경로를 입력하거나, Enter를 눌러 현재 워크스페이스를 사용하세요.",
            path.display()
        ));
    }
    None
}
