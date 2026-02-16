use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;
use std::io::Write;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::claude_code_client::{ClaudeCodeClient, ClaudeCodeRequest};
use crate::config::Config;
use super::clarification::{self, ClarificationQuestions, QaRound};
use super::coding::{
    self, BuildTestCommands, BuildTestOutcome, BuildTestRepairResult,
    BuildTestRepairStatus, CodingPhaseState, CodingTask, CodingTaskResult,
    CodingTaskStatus, ConflictResolutionResult, ConflictResolutionStatus,
    RebaseOutcome, ReviewResult, ReviewStatus, TaskExtractionResponse,
    TaskReport, TaskWorktreeInfo,
};
use super::file_validation::{self, FileKind, FileValidationResponse};
use super::planning::{self, PlanResponseType, PlanWritingResponse};
use super::session_naming::{self, SessionNameResponse};
use super::spec_writing::{self, SpecResponseType, SpecWritingResponse};
use super::error::UiError;
use super::renderer::{USER_PREFIX, wrap_text_by_char_width};

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
    ModeSelection,
    SpecFileInput,
    PlanFileInput,
    RequirementsInput,
    AgentThinking,
    ClarificationAnswer,
    SpecClarificationAnswer,
    SpecFeedback,
    PlanClarificationAnswer,
    PlanFeedback,
    Coding,
    BuildTestCommandInput,
    Done,
}

enum AgentOutcome {
    Clarification(ClarificationQuestions),
    SpecWriting(SpecWritingResponse),
    Planning(PlanWritingResponse),
    TaskExtraction(TaskExtractionResponse),
    CodingTaskCompleted(CodingTaskResult),
    ReviewCompleted(ReviewResult),
    ConflictResolutionCompleted(ConflictResolutionResult),
    BuildTestCompleted(BuildTestOutcome),
    BuildTestRepairCompleted(BuildTestRepairResult),
    FileValidation(FileValidationResponse),
}

struct AgentThreadResult {
    client: ClaudeCodeClient,
    outcome: Result<AgentOutcome, String>,
}

enum AgentStreamMessage {
    SessionName { name: String, date_dir: String },
    StreamLine(String),
    Completed(AgentThreadResult),
}

pub struct App {
    pub messages: Vec<ChatMessage>,
    input_mode: InputMode,
    pub input_buffer: String,
    pub cursor_position: usize,
    pub terminal_width: u16,
    pub confirmed_workspace: Option<PathBuf>,
    pub confirmed_requirements: Option<String>,
    pub should_quit: bool,
    current_directory: PathBuf,
    keyboard_enhancement_enabled: bool,
    config: Config,
    claude_client: Option<ClaudeCodeClient>,
    agent_result_receiver: Option<mpsc::Receiver<AgentStreamMessage>>,
    qa_log: Vec<QaRound>,
    current_round_questions: Vec<String>,
    thinking_started_at: Instant,
    last_spec_draft: Option<String>,
    spec_clarification_questions: Vec<String>,
    last_plan_draft: Option<String>,
    plan_clarification_questions: Vec<String>,
    approved_spec: Option<String>,
    spec_revision_instructions_sent: bool,
    session_name: Option<String>,
    session_date_dir: Option<String>,
    journal_dir: Option<PathBuf>,
    coding_state: Option<CodingPhaseState>,
    pending_coding_report: Option<String>,
    review_state: Option<ReviewState>,
    pending_build_test: Option<PendingBuildTest>,
    build_test_command_phase: BuildTestCommandPhase,
    fatal_error: Option<String>,
    selected_mode_index: usize,
    imported_spec_path: Option<PathBuf>,
    imported_plan_path: Option<PathBuf>,
    pending_validation_kind: Option<FileKind>,
    pub pending_external_editor: bool,
}

struct PendingBuildTest {
    task_id: String,
    report: String,
    is_retry: bool,
}

struct ReviewState {
    task_id: String,
    report: String,
    iteration_count: usize,
    reviewer_client: Option<ClaudeCodeClient>,
    coding_client: Option<ClaudeCodeClient>,
}

const MAX_REVIEW_ITERATIONS: usize = 3;

enum BuildTestCommandPhase {
    BuildCommand,
    TestCommand,
}

impl App {
    pub fn new(config: Config) -> Result<Self, UiError> {
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
            cursor_position: 0,
            terminal_width: 80,
            confirmed_workspace: None,
            confirmed_requirements: None,
            should_quit: false,
            current_directory,
            keyboard_enhancement_enabled: false,
            config,
            claude_client: None,
            agent_result_receiver: None,
            qa_log: Vec::new(),
            current_round_questions: Vec::new(),
            thinking_started_at: Instant::now(),
            last_spec_draft: None,
            spec_clarification_questions: Vec::new(),
            last_plan_draft: None,
            plan_clarification_questions: Vec::new(),
            approved_spec: None,
            spec_revision_instructions_sent: false,
            session_name: None,
            session_date_dir: None,
            journal_dir: None,
            coding_state: None,
            pending_coding_report: None,
            review_state: None,
            pending_build_test: None,
            build_test_command_phase: BuildTestCommandPhase::BuildCommand,
            fatal_error: None,
            selected_mode_index: 0,
            imported_spec_path: None,
            imported_plan_path: None,
            pending_validation_kind: None,
            pending_external_editor: false,
        })
    }

    pub fn fatal_error(&self) -> Option<&str> {
        self.fatal_error.as_deref()
    }

    pub fn handle_key_event(&mut self, key_event: KeyEvent) {
        match self.input_mode {
            InputMode::WorkspaceConfirm => self.handle_workspace_confirm(key_event),
            InputMode::ModeSelection => self.handle_mode_selection(key_event),
            InputMode::SpecFileInput => {
                self.handle_single_line_input(key_event, Self::submit_spec_file_path);
            }
            InputMode::PlanFileInput => {
                self.handle_single_line_input(key_event, Self::submit_plan_file_path);
            }
            InputMode::RequirementsInput => {
                self.handle_multiline_input(key_event, Self::submit_requirements);
            }
            InputMode::ClarificationAnswer => {
                self.handle_multiline_input(key_event, Self::submit_clarification_answer);
            }
            InputMode::SpecClarificationAnswer => {
                self.handle_multiline_input(key_event, Self::submit_spec_clarification_answer);
            }
            InputMode::SpecFeedback => {
                if key_event.code == KeyCode::Char('a')
                    && key_event.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.approve_spec();
                } else {
                    self.handle_multiline_input(key_event, Self::submit_spec_feedback);
                }
            }
            InputMode::PlanClarificationAnswer => {
                self.handle_multiline_input(key_event, Self::submit_plan_clarification_answer);
            }
            InputMode::PlanFeedback => {
                if key_event.code == KeyCode::Char('a')
                    && key_event.modifiers.contains(KeyModifiers::CONTROL)
                {
                    self.approve_plan();
                } else {
                    self.handle_multiline_input(key_event, Self::submit_plan_feedback);
                }
            }
            InputMode::BuildTestCommandInput => {
                self.handle_multiline_input(key_event, Self::submit_build_test_command);
            }
            InputMode::AgentThinking | InputMode::Coding | InputMode::Done => {
                if key_event.code == KeyCode::Esc {
                    self.should_quit = true;
                }
            }
        }
    }

    pub fn handle_paste(&mut self, text: String) {
        match self.input_mode {
            InputMode::WorkspaceConfirm
            | InputMode::SpecFileInput
            | InputMode::PlanFileInput => {
                let cleaned = text.replace("\r\n", " ").replace(['\r', '\n'], " ");
                self.insert_text_at_cursor(&cleaned);
            }
            InputMode::ModeSelection => {}
            InputMode::RequirementsInput
            | InputMode::ClarificationAnswer
            | InputMode::SpecClarificationAnswer
            | InputMode::SpecFeedback
            | InputMode::PlanClarificationAnswer
            | InputMode::PlanFeedback
            | InputMode::BuildTestCommandInput => {
                let cleaned = text.replace("\r\n", "\n").replace('\r', "\n");
                self.insert_text_at_cursor(&cleaned);
            }
            InputMode::AgentThinking | InputMode::Coding | InputMode::Done => {}
        }
    }

    pub fn tick(&mut self) {
        self.tick_agent_result();
    }

    fn tick_agent_result(&mut self) {
        let receiver = match self.agent_result_receiver.take() {
            Some(r) => r,
            None => return,
        };

        loop {
            match receiver.try_recv() {
                Ok(AgentStreamMessage::SessionName { name, date_dir }) => {
                    if self.journal_dir.is_none()
                        && let Some(ws) = &self.confirmed_workspace
                    {
                        self.journal_dir =
                            Some(ws.join(".bear").join(&date_dir).join(&name));
                    }
                    let journal_dir = self.journal_dir();
                    if let Some(user_request) = &self.confirmed_requirements
                        && let Err(err) =
                            spec_writing::save_user_request(&journal_dir, user_request)
                    {
                        self.add_system_message(
                            &format!("사용자 요청 파일 저장 실패: {}", err),
                        );
                    }
                    self.session_name = Some(name);
                    self.session_date_dir = Some(date_dir);
                }
                Ok(AgentStreamMessage::StreamLine(line)) => {
                    self.add_system_message(&line);
                }
                Ok(AgentStreamMessage::Completed(result)) => {
                    self.claude_client = Some(result.client);
                    match result.outcome {
                        Ok(AgentOutcome::Clarification(response)) => {
                            self.handle_clarification_response(response);
                        }
                        Ok(AgentOutcome::SpecWriting(response)) => {
                            self.handle_spec_response(response);
                        }
                        Ok(AgentOutcome::Planning(response)) => {
                            self.handle_plan_response(response);
                        }
                        Ok(AgentOutcome::TaskExtraction(response)) => {
                            self.handle_task_extraction_response(response);
                        }
                        Ok(AgentOutcome::CodingTaskCompleted(result)) => {
                            self.handle_coding_task_result(result);
                        }
                        Ok(AgentOutcome::ReviewCompleted(result)) => {
                            self.handle_review_result(result);
                        }
                        Ok(AgentOutcome::ConflictResolutionCompleted(result)) => {
                            self.handle_conflict_resolution_result(result);
                        }
                        Ok(AgentOutcome::BuildTestCompleted(outcome)) => {
                            self.handle_build_test_result(outcome);
                        }
                        Ok(AgentOutcome::BuildTestRepairCompleted(result)) => {
                            self.handle_build_test_repair_result(result);
                        }
                        Ok(AgentOutcome::FileValidation(result)) => {
                            self.handle_file_validation_result(result);
                        }
                        Err(error_message) => {
                            if matches!(self.input_mode, InputMode::Coding) {
                                self.handle_coding_task_error(error_message);
                            } else {
                                self.handle_agent_error(error_message);
                            }
                        }
                    }
                    return;
                }
                Err(mpsc::TryRecvError::Empty) => {
                    self.agent_result_receiver = Some(receiver);
                    return;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.handle_agent_error("에이전트 통신이 중단되었습니다.".to_string());
                    return;
                }
            }
        }
    }

    pub fn set_keyboard_enhancement_enabled(&mut self, enabled: bool) {
        self.keyboard_enhancement_enabled = enabled;
    }

    pub fn is_waiting_for_input(&self) -> bool {
        matches!(
            self.input_mode,
            InputMode::WorkspaceConfirm
                | InputMode::SpecFileInput
                | InputMode::PlanFileInput
                | InputMode::RequirementsInput
                | InputMode::ClarificationAnswer
                | InputMode::SpecClarificationAnswer
                | InputMode::SpecFeedback
                | InputMode::PlanClarificationAnswer
                | InputMode::PlanFeedback
                | InputMode::BuildTestCommandInput
        )
    }

    pub fn is_mode_selection(&self) -> bool {
        matches!(self.input_mode, InputMode::ModeSelection)
    }

    pub fn selected_mode_index(&self) -> usize {
        self.selected_mode_index
    }

    fn journal_dir(&self) -> PathBuf {
        if let Some(dir) = &self.journal_dir {
            return dir.clone();
        }
        match (&self.confirmed_workspace, &self.session_date_dir, &self.session_name) {
            (Some(ws), Some(date), Some(name)) => ws.join(".bear").join(date).join(name),
            _ => PathBuf::new(),
        }
    }

    pub fn is_thinking(&self) -> bool {
        matches!(self.input_mode, InputMode::AgentThinking | InputMode::Coding)
    }

    pub fn thinking_indicator(&self) -> &'static str {
        let dots = (self.thinking_started_at.elapsed().as_millis() / 500) % 4;
        if matches!(self.input_mode, InputMode::Coding) {
            match dots {
                0 => "Coding",
                1 => "Coding.",
                2 => "Coding..",
                _ => "Coding...",
            }
        } else {
            match dots {
                0 => "Analyzing",
                1 => "Analyzing.",
                2 => "Analyzing..",
                _ => "Analyzing...",
            }
        }
    }

    pub fn help_text(&self) -> &str {
        match self.input_mode {
            InputMode::WorkspaceConfirm
            | InputMode::SpecFileInput
            | InputMode::PlanFileInput => "[Enter] Confirm  [Esc] Quit",
            InputMode::ModeSelection => {
                "[1-3] Select  [Up/Down] Navigate  [Enter] Confirm  [Esc] Quit"
            }
            InputMode::RequirementsInput
            | InputMode::ClarificationAnswer
            | InputMode::SpecClarificationAnswer
            | InputMode::PlanClarificationAnswer => {
                if self.keyboard_enhancement_enabled {
                    "[Enter] Submit  [Shift+Enter] New line  [Ctrl+G] Editor  [Esc] Quit"
                } else {
                    "[Enter] Submit  [Alt+Enter] New line  [Ctrl+G] Editor  [Esc] Quit"
                }
            }
            InputMode::SpecFeedback | InputMode::PlanFeedback => {
                if self.keyboard_enhancement_enabled {
                    "[Enter] Submit feedback  [Ctrl+A] Approve  [Shift+Enter] New line  [Ctrl+G] Editor  [Esc] Quit"
                } else {
                    "[Enter] Submit feedback  [Ctrl+A] Approve  [Alt+Enter] New line  [Ctrl+G] Editor  [Esc] Quit"
                }
            }
            InputMode::BuildTestCommandInput => {
                if self.keyboard_enhancement_enabled {
                    "[Enter] Submit  [Shift+Enter] New line  [Ctrl+G] Editor  [Esc] Quit"
                } else {
                    "[Enter] Submit  [Alt+Enter] New line  [Ctrl+G] Editor  [Esc] Quit"
                }
            }
            InputMode::AgentThinking | InputMode::Coding | InputMode::Done => "[Esc] Quit",
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
                        self.clear_input();
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
                self.clear_input();
                self.transition_to_mode_selection();
            }
            _ => {
                self.handle_single_line_key(key_event);
            }
        }
    }

    fn handle_multiline_input(
        &mut self,
        key_event: KeyEvent,
        submit_action: fn(&mut Self),
    ) {
        match key_event.code {
            KeyCode::Enter if self.is_newline_modifier(key_event.modifiers) => {
                self.insert_char_at_cursor('\n');
            }
            KeyCode::Enter => {
                submit_action(self);
            }
            KeyCode::Backspace => {
                self.delete_char_before_cursor();
            }
            KeyCode::Delete => {
                self.delete_char_at_cursor();
            }
            KeyCode::Left => {
                self.move_cursor_left();
            }
            KeyCode::Right => {
                self.move_cursor_right();
            }
            KeyCode::Up => {
                self.move_cursor_up();
            }
            KeyCode::Down => {
                self.move_cursor_down();
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Char('g') if key_event.modifiers.contains(KeyModifiers::CONTROL) => {
                self.pending_external_editor = true;
            }
            KeyCode::Char(c) => {
                self.insert_char_at_cursor(c);
            }
            _ => {}
        }
    }

    fn handle_single_line_key(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Backspace => self.delete_char_before_cursor(),
            KeyCode::Delete => self.delete_char_at_cursor(),
            KeyCode::Left => self.move_cursor_left(),
            KeyCode::Right => self.move_cursor_right(),
            KeyCode::Esc => self.should_quit = true,
            KeyCode::Char(c) => self.insert_char_at_cursor(c),
            _ => {}
        }
    }

    fn handle_single_line_input(
        &mut self,
        key_event: KeyEvent,
        submit_action: fn(&mut Self),
    ) {
        match key_event.code {
            KeyCode::Enter => submit_action(self),
            _ => self.handle_single_line_key(key_event),
        }
    }

    fn handle_mode_selection(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Up | KeyCode::Char('k') => {
                self.selected_mode_index = self.selected_mode_index.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.selected_mode_index = (self.selected_mode_index + 1).min(2);
            }
            KeyCode::Enter => self.select_work_mode(self.selected_mode_index),
            KeyCode::Char('1') => self.select_work_mode(0),
            KeyCode::Char('2') => self.select_work_mode(1),
            KeyCode::Char('3') => self.select_work_mode(2),
            KeyCode::Esc => self.should_quit = true,
            _ => {}
        }
    }

    fn select_work_mode(&mut self, index: usize) {
        self.selected_mode_index = index;

        let label = match index {
            0 => "처음부터 만들기",
            1 => "스펙 파일 있음",
            _ => "스펙 및 플랜 파일 있음",
        };
        self.add_user_message(label);

        match index {
            0 => self.transition_to_requirements_input(),
            1 | 2 => self.transition_to_spec_file_input(),
            _ => unreachable!(),
        }
    }

    fn transition_to_mode_selection(&mut self) {
        self.selected_mode_index = 0;
        self.add_system_message(
            "작업 모드를 선택하세요:\n\
             \n\
             1. 처음부터 만들기\n\
             2. 스펙 파일 있음\n\
             3. 스펙 및 플랜 파일 있음",
        );
        self.input_mode = InputMode::ModeSelection;
    }

    fn transition_to_spec_file_input(&mut self) {
        self.add_system_message(
            "스펙 파일 경로를 입력하세요. (절대 경로 또는 상대 경로)",
        );
        self.input_mode = InputMode::SpecFileInput;
        self.clear_input();
    }

    fn transition_to_plan_file_input(&mut self) {
        self.add_system_message(
            "플랜 파일 경로를 입력하세요. (절대 경로 또는 상대 경로)",
        );
        self.input_mode = InputMode::PlanFileInput;
        self.clear_input();
    }

    fn transition_to_requirements_input(&mut self) {
        self.add_system_message("구현할 요구사항을 입력하세요.");
        self.input_mode = InputMode::RequirementsInput;
    }

    fn submit_spec_file_path(&mut self) {
        let raw_path = self.input_buffer.trim().to_string();
        if raw_path.is_empty() {
            return;
        }

        self.add_user_message(&raw_path);
        self.clear_input();

        let workspace = self.confirmed_workspace.clone().unwrap();
        match file_validation::validate_file_locally(&raw_path, &workspace) {
            Ok(resolved_path) => {
                self.imported_spec_path = Some(resolved_path.clone());
                self.pending_validation_kind = Some(FileKind::Spec);
                self.add_system_message("스펙 파일을 검증 중입니다...");
                self.start_file_content_validation(resolved_path);
            }
            Err(error_message) => {
                self.add_system_message(&error_message);
                self.add_system_message(
                    "스펙 파일 경로를 다시 입력하세요. (절대 경로 또는 상대 경로)",
                );
            }
        }
    }

    fn submit_plan_file_path(&mut self) {
        let raw_path = self.input_buffer.trim().to_string();
        if raw_path.is_empty() {
            return;
        }

        self.add_user_message(&raw_path);
        self.clear_input();

        let workspace = self.confirmed_workspace.clone().unwrap();
        match file_validation::validate_file_locally(&raw_path, &workspace) {
            Ok(resolved_path) => {
                self.imported_plan_path = Some(resolved_path.clone());
                self.pending_validation_kind = Some(FileKind::Plan);
                self.add_system_message("플랜 파일을 검증 중입니다...");
                self.start_file_content_validation(resolved_path);
            }
            Err(error_message) => {
                self.add_system_message(&error_message);
                self.add_system_message(
                    "플랜 파일 경로를 다시 입력하세요. (절대 경로 또는 상대 경로)",
                );
            }
        }
    }

    fn start_file_content_validation(&mut self, path: PathBuf) {
        if let Err(error_message) = self.ensure_claude_client() {
            self.add_system_message(&format!("클라이언트 생성 실패: {}", error_message));
            self.input_mode = InputMode::Done;
            return;
        }

        let mut client = self.claude_client.take().expect("client must be available");
        client.reset_session();
        client.set_system_prompt(Some(file_validation::system_prompt().to_string()));

        let kind = self.pending_validation_kind.unwrap();

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::AgentThinking;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let request = ClaudeCodeRequest {
                user_prompt: file_validation::build_validation_prompt(&path, kind),
                output_schema: file_validation::validation_schema(),
            };

            let outcome = client
                .query::<FileValidationResponse>(&request)
                .map(AgentOutcome::FileValidation)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client,
                outcome,
            }));
        });
    }

    fn handle_file_validation_result(&mut self, result: FileValidationResponse) {
        let kind = self.pending_validation_kind.take().unwrap_or(FileKind::Spec);

        if !result.valid {
            self.add_system_message(&format!("파일 검증 실패: {}", result.reason));
            match kind {
                FileKind::Spec => {
                    self.imported_spec_path = None;
                    self.transition_to_spec_file_input();
                }
                FileKind::Plan => {
                    self.imported_plan_path = None;
                    self.transition_to_plan_file_input();
                }
            }
            return;
        }

        match kind {
            FileKind::Spec => {
                let spec_path = self.imported_spec_path.as_ref().unwrap();
                match std::fs::read_to_string(spec_path) {
                    Ok(content) => {
                        self.approved_spec = Some(content);
                        self.add_system_message("스펙 파일이 검증되었습니다.");

                        if self.selected_mode_index == 2 {
                            self.transition_to_plan_file_input();
                        } else {
                            self.start_imported_file_workflow();
                        }
                    }
                    Err(err) => {
                        self.add_system_message(&format!("스펙 파일 읽기 실패: {}", err));
                        self.imported_spec_path = None;
                        self.transition_to_spec_file_input();
                    }
                }
            }
            FileKind::Plan => {
                let plan_path = self.imported_plan_path.as_ref().unwrap();
                match std::fs::read_to_string(plan_path) {
                    Ok(content) => {
                        self.last_plan_draft = Some(content);
                        self.add_system_message("플랜 파일이 검증되었습니다.");
                        self.start_imported_file_workflow();
                    }
                    Err(err) => {
                        self.add_system_message(&format!("플랜 파일 읽기 실패: {}", err));
                        self.imported_plan_path = None;
                        self.transition_to_plan_file_input();
                    }
                }
            }
        }
    }

    fn start_imported_file_workflow(&mut self) {
        if let Err(error_message) = self.ensure_claude_client() {
            self.add_system_message(&format!("클라이언트 생성 실패: {}", error_message));
            self.input_mode = InputMode::Done;
            return;
        }

        let mut client = self.claude_client.take().expect("client must be available");
        client.reset_session();

        let is_plan_mode = self.selected_mode_index == 2;
        let workspace = self.confirmed_workspace.clone().unwrap();
        let spec_content = self.approved_spec.clone().unwrap_or_default();
        let imported_spec_path = self.imported_spec_path.clone().unwrap();
        let imported_plan_path = self.imported_plan_path.clone();

        // 가져온 스펙 파일이 위치한 디렉토리를 journal_dir로 설정
        let journal_dir = imported_spec_path.parent().unwrap().to_path_buf();
        self.journal_dir = Some(journal_dir.clone());

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::AgentThinking;
        self.thinking_started_at = Instant::now();

        if is_plan_mode {
            self.add_system_message(
                "세션을 초기화하고 코드 구현을 시작합니다...",
            );
        } else {
            self.add_system_message(
                "세션을 초기화하고 개발 계획을 작성합니다...",
            );
        }

        std::thread::spawn(move || {
            let spec_preview: String = spec_content.chars().take(500).collect();
            let session_name = generate_session_name(&mut client, &spec_preview);
            let date_dir = session_naming::today_date_string();
            let session_name =
                session_naming::ensure_unique_name(&workspace, &date_dir, &session_name);
            client.reset_session();

            let _ = sender.send(AgentStreamMessage::SessionName {
                name: session_name,
                date_dir,
            });

            // 가져온 파일의 디렉토리에 user-request.md 생성
            let user_request_content = format!(
                "외부 스펙 파일에서 가져옴: {}",
                imported_spec_path.display()
            );
            let user_request_path = journal_dir.join("user-request.md");
            let _ = std::fs::write(&user_request_path, &user_request_content);

            // 가져온 파일의 실제 경로를 그대로 사용
            let spec_path = imported_spec_path;
            let plan_path = imported_plan_path
                .unwrap_or_else(|| journal_dir.join("plan.md"));

            if is_plan_mode {
                // 모드 3: 태스크 추출 시작
                client.set_system_prompt(
                    Some(coding::task_extraction_system_prompt().to_string()),
                );

                let request = ClaudeCodeRequest {
                    user_prompt: coding::build_task_extraction_prompt(&plan_path),
                    output_schema: coding::task_extraction_schema(),
                };

                let stream_sender = sender.clone();
                let outcome = client
                    .query_streaming::<TaskExtractionResponse, _>(&request, |line| {
                        let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                    })
                    .map(AgentOutcome::TaskExtraction)
                    .map_err(|err| err.to_string());

                let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                    client,
                    outcome,
                }));
            } else {
                // 모드 2: 플랜 작성 시작
                client.set_system_prompt(Some(planning::system_prompt().to_string()));

                let request = ClaudeCodeRequest {
                    user_prompt: planning::build_initial_plan_prompt(
                        &user_request_path,
                        &spec_path,
                    ),
                    output_schema: planning::plan_writing_schema(),
                };

                let stream_sender = sender.clone();
                let outcome = client
                    .query_streaming::<PlanWritingResponse, _>(&request, |line| {
                        let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                    })
                    .map(AgentOutcome::Planning)
                    .map_err(|err| err.to_string());

                let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                    client,
                    outcome,
                }));
            }
        });
    }

    fn submit_requirements(&mut self) {
        let requirements = self.input_buffer.trim().to_string();
        if requirements.is_empty() {
            return;
        }

        self.add_user_message(&requirements);
        self.confirmed_requirements = Some(requirements);
        self.clear_input();

        if let Err(error_message) = self.ensure_claude_client() {
            self.add_system_message(&format!("클라이언트 생성 실패: {}", error_message));
            self.input_mode = InputMode::Done;
            return;
        }

        self.add_system_message("요구사항을 분석 중입니다. 잠시만 기다려 주세요.");
        self.start_clarification_query();
    }

    fn submit_clarification_answer(&mut self) {
        let answer = self.input_buffer.trim().to_string();
        if answer.is_empty() {
            return;
        }

        self.add_user_message(&answer);
        self.clear_input();

        let questions = std::mem::take(&mut self.current_round_questions);
        self.qa_log.push(QaRound { questions, answer });

        self.add_system_message("답변을 분석 중입니다. 잠시만 기다려 주세요.");
        self.start_clarification_query();
    }

    fn ensure_claude_client(&mut self) -> Result<(), String> {
        if self.claude_client.is_some() {
            return Ok(());
        }

        let workspace = self.confirmed_workspace.clone().unwrap();
        let client = ClaudeCodeClient::new(
            self.config.api_key().to_string(),
            vec![workspace],
            Some(clarification::system_prompt().to_string()),
        )
            .map_err(|err| err.to_string())?;

        self.claude_client = Some(client);
        Ok(())
    }

    fn start_clarification_query(&mut self) {
        let mut client = self.claude_client.take().expect("client must be available");
        let original_request = self.confirmed_requirements.clone().unwrap();
        let qa_log = self.qa_log.clone();
        let needs_session_name = self.session_name.is_none();
        let workspace = self.confirmed_workspace.clone().unwrap();

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::AgentThinking;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            if needs_session_name {
                let name = generate_session_name(&mut client, &original_request);
                let date_dir = session_naming::today_date_string();
                let name = session_naming::ensure_unique_name(&workspace, &date_dir, &name);
                client.reset_session();
                let _ = sender.send(AgentStreamMessage::SessionName { name, date_dir });
            }

            let request = ClaudeCodeRequest {
                user_prompt: clarification::build_user_prompt(&original_request, &qa_log),
                output_schema: clarification::clarification_schema(),
            };

            let stream_sender = sender.clone();
            let outcome = client
                .query_streaming::<ClarificationQuestions, _>(&request, |line| {
                    let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                })
                .map(AgentOutcome::Clarification)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult { client, outcome }));
        });
    }

    fn handle_clarification_response(&mut self, response: ClarificationQuestions) {
        if response.questions.is_empty() {
            self.add_system_message("요구사항 분석이 완료되었습니다. 스펙 문서를 작성합니다.");
            self.start_spec_writing_query(true);
            return;
        }

        let mut message = String::from("스펙 작성을 위해 다음 질문에 답변해 주세요.\n");
        for (i, question) in response.questions.iter().enumerate() {
            message.push_str(&format!("\n{}. {}", i + 1, question));
        }

        self.current_round_questions = response.questions;
        self.add_system_message(&message);
        self.input_mode = InputMode::ClarificationAnswer;
    }

    fn handle_agent_error(&mut self, error_message: String) {
        self.add_system_message(&format!("에이전트 오류: {}", error_message));
        self.fatal_error = Some(error_message);
        self.should_quit = true;
    }

    fn start_spec_writing_query(&mut self, is_initial: bool) {
        let mut client = self.claude_client.take().expect("client must be available");

        let qa_log = self.qa_log.clone();
        let user_request_path = self.journal_dir().join("user-request.md");
        let user_feedback = if is_initial {
            None
        } else {
            self.messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, MessageRole::User))
                .map(|m| m.content.clone())
        };
        let send_full_revision_instructions = if is_initial {
            false
        } else {
            let should_send = !self.spec_revision_instructions_sent;
            self.spec_revision_instructions_sent = true;
            should_send
        };

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::AgentThinking;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let user_prompt = if is_initial {
                spec_writing::build_initial_spec_prompt(&user_request_path, &qa_log)
            } else {
                let feedback = user_feedback.unwrap_or_default();
                if send_full_revision_instructions {
                    spec_writing::build_revision_prompt(&feedback)
                } else {
                    spec_writing::build_followup_revision_prompt(&feedback)
                }
            };

            let request = ClaudeCodeRequest {
                user_prompt,
                output_schema: spec_writing::spec_writing_schema(),
            };

            let stream_sender = sender.clone();
            let outcome = client
                .query_streaming::<SpecWritingResponse, _>(&request, |line| {
                    let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                })
                .map(AgentOutcome::SpecWriting)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client,
                outcome,
            }));
        });
    }

    fn handle_spec_response(&mut self, response: SpecWritingResponse) {
        match response.response_type {
            SpecResponseType::SpecDraft => {
                let draft = response.spec_draft.unwrap_or_default();

                self.add_system_message(&format!(
                    "스펙 드래프트가 작성되었습니다:\n\n{}\n\n피드백을 입력하거나, Ctrl+A를 눌러 승인하세요.",
                    draft
                ));
                self.last_spec_draft = Some(draft);
                self.input_mode = InputMode::SpecFeedback;
            }
            SpecResponseType::ClarifyingQuestions => {
                let questions = response.clarifying_questions.unwrap_or_default();

                let mut message = String::from("스펙 작성을 위해 추가 정보가 필요합니다.\n");
                for (i, question) in questions.iter().enumerate() {
                    message.push_str(&format!("\n{}. {}", i + 1, question));
                }

                self.spec_clarification_questions = questions;
                self.add_system_message(&message);
                self.input_mode = InputMode::SpecClarificationAnswer;
            }
            SpecResponseType::Approved => {
                self.approve_spec();
            }
        }
    }

    fn submit_spec_clarification_answer(&mut self) {
        let answer = self.input_buffer.trim().to_string();
        if answer.is_empty() {
            return;
        }

        self.add_user_message(&answer);
        self.clear_input();

        self.add_system_message("답변을 반영하여 스펙을 작성합니다.");
        self.start_spec_writing_query(false);
    }

    fn submit_spec_feedback(&mut self) {
        let feedback = self.input_buffer.trim().to_string();
        if feedback.is_empty() {
            return;
        }

        self.add_user_message(&feedback);
        self.clear_input();

        self.add_system_message("피드백을 반영하여 스펙을 수정합니다.");
        self.start_spec_writing_query(false);
    }

    fn approve_spec(&mut self) {
        let spec = match &self.last_spec_draft {
            Some(spec) => spec.clone(),
            None => {
                self.add_system_message("승인할 스펙이 없습니다.");
                return;
            }
        };

        self.approved_spec = Some(spec.clone());

        let journal_dir = self.journal_dir();
        if let Err(err) = spec_writing::save_approved_spec(&journal_dir, &spec) {
            self.add_system_message(&format!("스펙 파일 저장 실패: {}", err));
        }

        self.add_system_message("스펙이 승인되었습니다. 개발 계획을 작성합니다.");
        self.start_plan_writing_query(true);
    }

    fn start_plan_writing_query(&mut self, is_initial: bool) {
        let mut client = self.claude_client.take().expect("client must be available");

        if is_initial {
            client.reset_session();
            client.set_system_prompt(Some(planning::system_prompt().to_string()));
        }

        let journal_dir = self.journal_dir();
        let user_request_path = journal_dir.join("user-request.md");
        let spec_path = journal_dir.join("spec.md");
        let user_feedback = if is_initial {
            None
        } else {
            self.messages
                .iter()
                .rev()
                .find(|m| matches!(m.role, MessageRole::User))
                .map(|m| m.content.clone())
        };

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::AgentThinking;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let user_prompt = if is_initial {
                planning::build_initial_plan_prompt(&user_request_path, &spec_path)
            } else {
                let feedback = user_feedback.unwrap_or_default();
                planning::build_plan_revision_prompt(&feedback)
            };

            let request = ClaudeCodeRequest {
                user_prompt,
                output_schema: planning::plan_writing_schema(),
            };

            let stream_sender = sender.clone();
            let outcome = client
                .query_streaming::<PlanWritingResponse, _>(&request, |line| {
                    let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                })
                .map(AgentOutcome::Planning)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client,
                outcome,
            }));
        });
    }

    fn handle_plan_response(&mut self, response: PlanWritingResponse) {
        match response.response_type {
            PlanResponseType::PlanDraft => {
                let draft = response.plan_draft.unwrap_or_default();

                self.add_system_message(&format!(
                    "개발 계획 드래프트가 작성되었습니다:\n\n{}\n\n피드백을 입력하거나, Ctrl+A를 눌러 승인하세요.",
                    draft
                ));
                self.last_plan_draft = Some(draft);
                self.input_mode = InputMode::PlanFeedback;
            }
            PlanResponseType::ClarifyingQuestions => {
                let questions = response.clarifying_questions.unwrap_or_default();

                let mut message = String::from("개발 계획 작성을 위해 추가 정보가 필요합니다.\n");
                for (i, question) in questions.iter().enumerate() {
                    message.push_str(&format!("\n{}. {}", i + 1, question));
                }

                self.plan_clarification_questions = questions;
                self.add_system_message(&message);
                self.input_mode = InputMode::PlanClarificationAnswer;
            }
            PlanResponseType::Approved => {
                self.approve_plan();
            }
        }
    }

    fn submit_plan_clarification_answer(&mut self) {
        let answer = self.input_buffer.trim().to_string();
        if answer.is_empty() {
            return;
        }

        self.add_user_message(&answer);
        self.clear_input();

        self.add_system_message("답변을 반영하여 개발 계획을 작성합니다.");
        self.start_plan_writing_query(false);
    }

    fn submit_plan_feedback(&mut self) {
        let feedback = self.input_buffer.trim().to_string();
        if feedback.is_empty() {
            return;
        }

        self.add_user_message(&feedback);
        self.clear_input();

        self.add_system_message("피드백을 반영하여 개발 계획을 수정합니다.");
        self.start_plan_writing_query(false);
    }

    fn approve_plan(&mut self) {
        let plan = match &self.last_plan_draft {
            Some(plan) => plan.clone(),
            None => {
                self.add_system_message("승인할 개발 계획이 없습니다.");
                return;
            }
        };

        let journal_dir = self.journal_dir();
        if let Err(err) = planning::save_approved_plan(&journal_dir, &plan) {
            self.add_system_message(&format!("플랜 파일 저장 실패: {}", err));
        }

        self.add_system_message("개발 계획이 승인되었습니다. 작업 목록을 추출합니다.");
        self.start_task_extraction();
    }

    fn start_task_extraction(&mut self) {
        let mut client = self.claude_client.take().expect("client must be available");
        client.reset_session();
        client.set_system_prompt(Some(coding::task_extraction_system_prompt().to_string()));

        let plan_path = self.journal_dir().join("plan.md");

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::AgentThinking;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let request = ClaudeCodeRequest {
                user_prompt: coding::build_task_extraction_prompt(&plan_path),
                output_schema: coding::task_extraction_schema(),
            };

            let stream_sender = sender.clone();
            let outcome = client
                .query_streaming::<TaskExtractionResponse, _>(&request, |line| {
                    let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                })
                .map(AgentOutcome::TaskExtraction)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client,
                outcome,
            }));
        });
    }

    fn handle_task_extraction_response(&mut self, response: TaskExtractionResponse) {
        if response.tasks.is_empty() {
            self.add_system_message("추출된 작업이 없습니다.");
            self.input_mode = InputMode::Done;
            return;
        }

        let mut schedule_message = format!(
            "{}개 작업이 추출되었습니다:\n",
            response.tasks.len()
        );
        for (i, task) in response.tasks.iter().enumerate() {
            schedule_message.push_str(&format!(
                "\n{}. [{}] {}",
                i + 1,
                task.task_id,
                task.title,
            ));
            if !task.dependencies.is_empty() {
                schedule_message.push_str(&format!(
                    " (의존: {})",
                    task.dependencies.join(", "),
                ));
            }
        }
        self.add_system_message(&schedule_message);

        let workspace = self.confirmed_workspace.clone().unwrap();
        let session_name = self
            .session_name
            .clone()
            .unwrap_or_else(|| "unnamed".to_string());

        let integration_branch =
            match coding::create_integration_branch(&workspace, &session_name) {
                Ok(branch) => branch,
                Err(err) => {
                    self.add_system_message(&format!("Failed to create git branch: {}", err));
                    self.input_mode = InputMode::Done;
                    return;
                }
            };

        self.add_system_message(&format!(
            "코딩 워크스페이스 준비 완료.\n통합 브랜치: {}",
            integration_branch,
        ));

        self.coding_state = Some(CodingPhaseState {
            tasks: response.tasks,
            current_task_index: 0,
            task_reports: Vec::new(),
            integration_branch,
            current_task_worktree: None,
            build_test_commands: None,
        });

        self.start_next_coding_task();
    }

    /// 다음 코딩 태스크에 필요한 데이터를 추출한다.
    /// 남은 태스크가 없으면 None을 반환한다.
    fn extract_next_coding_task_data(
        &self,
    ) -> Option<(CodingTask, usize, usize, Vec<PathBuf>)> {
        let coding_state = self.coding_state.as_ref()?;
        if coding_state.current_task_index >= coding_state.tasks.len() {
            return None;
        }

        let task = coding_state.tasks[coding_state.current_task_index].clone();
        let total = coding_state.tasks.len();
        let index = coding_state.current_task_index;
        let upstream_report_paths =
            coding::collect_upstream_report_paths(&task, &coding_state.task_reports);

        Some((task, total, index, upstream_report_paths))
    }

    fn start_next_coding_task(&mut self) {
        let extracted = self.extract_next_coding_task_data();
        let (task, total, index, upstream_report_paths) = match extracted {
            Some(data) => data,
            None => {
                self.finish_coding_phase();
                return;
            }
        };

        self.add_system_message(&format!(
            "작업 {}/{} 시작: [{}] {}",
            index + 1,
            total,
            task.task_id,
            task.title,
        ));

        let workspace = self.confirmed_workspace.clone().unwrap();
        let integration_branch = self
            .coding_state
            .as_ref()
            .unwrap()
            .integration_branch
            .clone();

        let task_branch =
            match coding::create_task_branch(&workspace, &integration_branch, &task.task_id) {
                Ok(branch) => branch,
                Err(err) => {
                    self.add_system_message(&format!("태스크 브랜치 생성 실패: {}", err));
                    self.save_and_advance_task(
                        task.task_id.clone(),
                        CodingTaskStatus::ImplementationBlocked,
                        format!("태스크 브랜치 생성 실패: {}", err),
                    );
                    return;
                }
            };

        let worktree_path = match coding::create_worktree(&workspace, &task_branch) {
            Ok(path) => path,
            Err(err) => {
                self.add_system_message(&format!("워크트리 생성 실패: {}", err));
                let _ = coding::delete_branch(&workspace, &task_branch);
                self.save_and_advance_task(
                    task.task_id.clone(),
                    CodingTaskStatus::ImplementationBlocked,
                    format!("워크트리 생성 실패: {}", err),
                );
                return;
            }
        };

        self.add_system_message(&format!(
            "태스크 워크트리 생성: {}\n브랜치: {}",
            worktree_path.display(),
            task_branch,
        ));

        let coding_state = self.coding_state.as_mut().unwrap();
        coding_state.current_task_worktree = Some(TaskWorktreeInfo {
            worktree_path: worktree_path.clone(),
            task_branch,
        });

        let journal_dir = self.journal_dir();
        let spec_path = journal_dir.join("spec.md");
        let plan_path = journal_dir.join("plan.md");
        let api_key = self.config.api_key().to_string();

        let mut client = match ClaudeCodeClient::new(
            api_key,
            vec![worktree_path.clone()],
            Some(coding::coding_agent_system_prompt().to_string()),
        ) {
            Ok(c) => c,
            Err(err) => {
                self.add_system_message(&format!(
                    "코딩 에이전트 클라이언트 생성 실패: {}",
                    err,
                ));
                self.cleanup_current_task_worktree();
                self.save_and_advance_task(
                    task.task_id.clone(),
                    CodingTaskStatus::ImplementationBlocked,
                    format!("코딩 에이전트 클라이언트 생성 실패: {}", err),
                );
                return;
            }
        };
        client.set_working_directory(worktree_path);

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::Coding;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let user_prompt = coding::build_coding_task_prompt(
                &task,
                &spec_path,
                &plan_path,
                &upstream_report_paths,
            );

            let request = ClaudeCodeRequest {
                user_prompt,
                output_schema: coding::coding_task_result_schema(),
            };

            let stream_sender = sender.clone();
            let outcome = client
                .query_streaming::<CodingTaskResult, _>(&request, |line| {
                    let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                })
                .map(AgentOutcome::CodingTaskCompleted)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client,
                outcome,
            }));
        });
    }

    fn handle_coding_task_result(&mut self, result: CodingTaskResult) {
        let task_id = {
            let coding_state = self.coding_state.as_ref().unwrap();
            coding_state.tasks[coding_state.current_task_index]
                .task_id
                .clone()
        };

        let status_label = match &result.status {
            CodingTaskStatus::ImplementationSuccess => "SUCCESS",
            CodingTaskStatus::ImplementationBlocked => "BLOCKED",
        };
        self.add_system_message(&format!(
            "작업 [{}] 완료: {}",
            task_id, status_label,
        ));

        if result.status == CodingTaskStatus::ImplementationBlocked {
            self.review_state = None;
            self.cleanup_current_task_worktree();
            self.save_and_advance_task(task_id, result.status, result.report);
            return;
        }

        let coding_client = self.claude_client.take();

        match self.review_state.as_mut() {
            None => {
                self.review_state = Some(ReviewState {
                    task_id: task_id.clone(),
                    report: result.report.clone(),
                    iteration_count: 0,
                    reviewer_client: None,
                    coding_client,
                });
            }
            Some(rs) => {
                rs.report = result.report.clone();
                rs.coding_client = coding_client;
            }
        }

        self.start_review();
    }

    fn start_review(&mut self) {
        let review_state = self.review_state.as_ref().unwrap();
        let is_followup = review_state.iteration_count > 0;
        let task_id = review_state.task_id.clone();
        let report = review_state.report.clone();

        let coding_state = self.coding_state.as_ref().unwrap();
        let worktree_info = coding_state.current_task_worktree.as_ref().unwrap();
        let worktree_path = worktree_info.worktree_path.clone();

        let git_commit_revision = match coding::get_latest_commit_revision(&worktree_path) {
            Ok(rev) => rev,
            Err(err) => {
                self.add_system_message(&format!(
                    "[{}] git 커밋 해시 조회 실패: {}. 리뷰 건너뜀.",
                    task_id, err,
                ));
                self.finalize_review_and_proceed();
                return;
            }
        };

        let journal_dir = self.journal_dir();

        let report_path = match coding::save_task_report(
            &journal_dir, &task_id, &report,
        ) {
            Ok(path) => path,
            Err(err) => {
                self.add_system_message(&format!(
                    "[{}] 리포트 저장 실패: {}. 리뷰 건너뜀.",
                    task_id, err,
                ));
                self.finalize_review_and_proceed();
                return;
            }
        };

        let spec_path = match (&self.confirmed_workspace, &self.session_date_dir, &self.session_name) {
            (Some(ws), Some(date), Some(name)) => {
                ws.join(".bear").join(date).join(name).join("spec.md")
            }
            _ => PathBuf::new(),
        };
        let plan_path = match (&self.confirmed_workspace, &self.session_date_dir, &self.session_name) {
            (Some(ws), Some(date), Some(name)) => {
                ws.join(".bear").join(date).join(name).join("plan.md")
            }
            _ => PathBuf::new(),
        };

        let user_prompt = if is_followup {
            coding::build_followup_review_prompt(
                &spec_path, &plan_path, &report_path, &git_commit_revision,
            )
        } else {
            coding::build_initial_review_prompt(
                &spec_path, &plan_path, &report_path, &git_commit_revision,
            )
        };

        let api_key = self.config.api_key().to_string();
        let mut reviewer_client = match self.review_state.as_mut().unwrap().reviewer_client.take() {
            Some(client) => client,
            None => {
                match ClaudeCodeClient::new(
                    api_key,
                    vec![worktree_path.clone()],
                    Some(coding::review_agent_system_prompt().to_string()),
                ) {
                    Ok(mut c) => {
                        c.set_working_directory(worktree_path.clone());
                        c
                    }
                    Err(err) => {
                        self.add_system_message(&format!(
                            "[{}] 리뷰 에이전트 클라이언트 생성 실패: {}. 리뷰 건너뜀.",
                            task_id, err,
                        ));
                        self.finalize_review_and_proceed();
                        return;
                    }
                }
            }
        };
        reviewer_client.set_working_directory(worktree_path);

        let iteration_label = self.review_state.as_ref().unwrap().iteration_count + 1;
        self.add_system_message(&format!(
            "[{}] 코드 리뷰 시작 (iteration {})...",
            task_id, iteration_label,
        ));

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::Coding;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let request = ClaudeCodeRequest {
                user_prompt,
                output_schema: coding::review_result_schema(),
            };

            let stream_sender = sender.clone();
            let outcome = reviewer_client
                .query_streaming::<ReviewResult, _>(&request, |line| {
                    let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                })
                .map(AgentOutcome::ReviewCompleted)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client: reviewer_client,
                outcome,
            }));
        });
    }

    fn handle_review_result(&mut self, result: ReviewResult) {
        let reviewer_client = self.claude_client.take();
        let review_state = self.review_state.as_mut().unwrap();
        review_state.reviewer_client = reviewer_client;
        review_state.iteration_count += 1;

        let task_id = review_state.task_id.clone();

        match result.review_result {
            ReviewStatus::Approved => {
                self.add_system_message(&format!("[{}] 코드 리뷰 승인.", task_id));
                self.finalize_review_and_proceed();
            }
            ReviewStatus::RequestChanges => {
                let iteration_count = self.review_state.as_ref().unwrap().iteration_count;

                if iteration_count >= MAX_REVIEW_ITERATIONS {
                    self.add_system_message(&format!(
                        "[{}] 리뷰 최대 반복 횟수({}) 도달. 자동 승인 처리.",
                        task_id, MAX_REVIEW_ITERATIONS,
                    ));
                    self.finalize_review_and_proceed();
                    return;
                }

                self.add_system_message(&format!(
                    "[{}] 리뷰어 변경 요청 (iteration {}/{}): {}",
                    task_id, iteration_count, MAX_REVIEW_ITERATIONS,
                    result.review_comment,
                ));

                self.start_coding_revision(result.review_comment);
            }
        }
    }

    fn finalize_review_and_proceed(&mut self) {
        let review_state = self.review_state.take().unwrap();
        let task_id = review_state.task_id;
        let report = review_state.report;

        self.claude_client = review_state.coding_client;

        self.rebase_and_merge_task(task_id, report);
    }

    fn start_coding_revision(&mut self, review_comment: String) {
        let coding_state = self.coding_state.as_ref().unwrap();
        let task = coding_state.tasks[coding_state.current_task_index].clone();
        let worktree_info = coding_state.current_task_worktree.as_ref().unwrap();
        let worktree_path = worktree_info.worktree_path.clone();
        let task_id = task.task_id.clone();

        let spec_path = match (&self.confirmed_workspace, &self.session_date_dir, &self.session_name) {
            (Some(ws), Some(date), Some(name)) => {
                ws.join(".bear").join(date).join(name).join("spec.md")
            }
            _ => PathBuf::new(),
        };
        let plan_path = match (&self.confirmed_workspace, &self.session_date_dir, &self.session_name) {
            (Some(ws), Some(date), Some(name)) => {
                ws.join(".bear").join(date).join(name).join("plan.md")
            }
            _ => PathBuf::new(),
        };

        let user_prompt = coding::build_coding_revision_prompt(
            &task, &spec_path, &plan_path, &review_comment,
        );

        let mut client = match self.review_state.as_mut().unwrap().coding_client.take() {
            Some(c) => c,
            None => {
                self.add_system_message(&format!(
                    "[{}] 코딩 에이전트 세션을 찾을 수 없습니다. 리뷰 자동 승인 처리.",
                    task_id,
                ));
                self.finalize_review_and_proceed();
                return;
            }
        };
        client.set_working_directory(worktree_path);

        self.add_system_message(&format!(
            "[{}] 리뷰 피드백 반영을 위한 코딩 에이전트 재시작...",
            task_id,
        ));

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::Coding;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let request = ClaudeCodeRequest {
                user_prompt,
                output_schema: coding::coding_task_result_schema(),
            };

            let stream_sender = sender.clone();
            let outcome = client
                .query_streaming::<CodingTaskResult, _>(&request, |line| {
                    let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                })
                .map(AgentOutcome::CodingTaskCompleted)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client,
                outcome,
            }));
        });
    }

    fn rebase_and_merge_task(
        &mut self,
        task_id: String,
        report: String,
    ) {
        let coding_state = self.coding_state.as_ref().unwrap();
        let worktree_info = coding_state.current_task_worktree.as_ref().unwrap();
        let worktree_path = worktree_info.worktree_path.clone();
        let integration_branch = coding_state.integration_branch.clone();

        self.add_system_message(&format!(
            "[{}] 통합 브랜치로 리베이스 시작...",
            task_id,
        ));

        match coding::rebase_onto_integration(&worktree_path, &integration_branch) {
            Ok(RebaseOutcome::Success) => {
                self.add_system_message(&format!("[{}] 리베이스 성공.", task_id));
                self.verify_build_and_test(task_id, report);
            }
            Ok(RebaseOutcome::Conflict { conflicted_files }) => {
                self.add_system_message(&format!(
                    "[{}] 리베이스 충돌 발생 ({}개 파일). 충돌 해결 에이전트 시작...",
                    task_id,
                    conflicted_files.len(),
                ));
                self.start_conflict_resolution(
                    task_id,
                    conflicted_files,
                    report,
                );
            }
            Err(err) => {
                self.add_system_message(&format!("[{}] 리베이스 실패: {}", task_id, err));
                self.cleanup_current_task_worktree();
                self.save_and_advance_task(
                    task_id,
                    CodingTaskStatus::ImplementationBlocked,
                    format!("{}\n\n---\n리베이스 실패: {}", report, err),
                );
            }
        }
    }

    fn handle_coding_task_error(&mut self, error_message: String) {
        let task_id = {
            let coding_state = self.coding_state.as_ref().unwrap();
            coding_state.tasks[coding_state.current_task_index]
                .task_id
                .clone()
        };

        self.add_system_message(&format!(
            "Task [{}] error: {}",
            task_id, error_message,
        ));

        self.review_state = None;
        self.cleanup_current_task_worktree();

        let report = format!(
            "IMPLEMENTATION_BLOCKED\n---\nAgent error: {}",
            error_message,
        );
        let message = format!("Task [{}] error: {}", task_id, error_message);
        self.save_and_advance_task(
            task_id,
            CodingTaskStatus::ImplementationBlocked,
            report,
        );
        self.fatal_error = Some(message);
        self.should_quit = true;
    }

    fn cleanup_current_task_worktree(&mut self) {
        let workspace = self.confirmed_workspace.clone().unwrap();
        let coding_state = self.coding_state.as_mut().unwrap();
        if let Some(info) = coding_state.current_task_worktree.take() {
            if let Err(err) = coding::remove_worktree(&workspace, &info.worktree_path) {
                self.add_system_message(&format!("워크트리 제거 실패: {}", err));
            }
            if let Err(err) = coding::delete_branch(&workspace, &info.task_branch) {
                self.add_system_message(&format!("태스크 브랜치 삭제 실패: {}", err));
            }
        }
    }

    fn verify_build_and_test(
        &mut self,
        task_id: String,
        report: String,
    ) {
        let worktree_path = self
            .coding_state
            .as_ref()
            .unwrap()
            .current_task_worktree
            .as_ref()
            .unwrap()
            .worktree_path
            .clone();

        let already_detected = self
            .coding_state
            .as_ref()
            .unwrap()
            .build_test_commands
            .is_some();

        if !already_detected {
            if let Some(commands) = coding::detect_build_commands(&worktree_path) {
                self.add_system_message(&format!(
                    "[{}] 빌드 시스템 감지: build='{}', test='{}'",
                    task_id, commands.build, commands.test,
                ));
                self.coding_state.as_mut().unwrap().build_test_commands = Some(commands);
            } else {
                self.add_system_message(
                    "빌드 시스템을 자동 감지할 수 없습니다. 빌드 명령어를 입력해주세요:",
                );
                self.ask_build_command(task_id, report);
                return;
            }
        }

        self.start_build_test_execution(task_id, report, false);
    }

    fn ask_build_command(
        &mut self,
        task_id: String,
        report: String,
    ) {
        self.pending_build_test = Some(PendingBuildTest {
            task_id,
            report,
            is_retry: false,
        });
        self.build_test_command_phase = BuildTestCommandPhase::BuildCommand;
        self.input_buffer.clear();
        self.cursor_position = 0;
        self.input_mode = InputMode::BuildTestCommandInput;
    }

    fn submit_build_test_command(&mut self) {
        let command = self.input_buffer.trim().to_string();
        if command.is_empty() {
            return;
        }
        self.add_user_message(&command);
        self.input_buffer.clear();
        self.cursor_position = 0;

        match self.build_test_command_phase {
            BuildTestCommandPhase::BuildCommand => {
                let coding_state = self.coding_state.as_mut().unwrap();
                coding_state.build_test_commands = Some(BuildTestCommands {
                    build: command,
                    test: String::new(),
                });
                self.build_test_command_phase = BuildTestCommandPhase::TestCommand;
                self.add_system_message("테스트 명령어를 입력해주세요 (예: make test):");
            }
            BuildTestCommandPhase::TestCommand => {
                let coding_state = self.coding_state.as_mut().unwrap();
                if let Some(ref mut commands) = coding_state.build_test_commands {
                    commands.test = command;
                }

                let pending = self.pending_build_test.take().unwrap();
                self.start_build_test_execution(
                    pending.task_id,
                    pending.report,
                    pending.is_retry,
                );
            }
        }
    }

    fn start_build_test_execution(
        &mut self,
        task_id: String,
        report: String,
        is_retry: bool,
    ) {
        let commands = self
            .coding_state
            .as_ref()
            .unwrap()
            .build_test_commands
            .clone()
            .unwrap();
        let worktree_path = self
            .coding_state
            .as_ref()
            .unwrap()
            .current_task_worktree
            .as_ref()
            .unwrap()
            .worktree_path
            .clone();

        self.add_system_message(&format!(
            "[{}] 빌드/테스트 검증 시작...",
            task_id,
        ));

        self.pending_build_test = Some(PendingBuildTest {
            task_id,
            report,
            is_retry,
        });

        let client = self.claude_client.take().unwrap();
        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::Coding;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let outcome = coding::run_build_and_test(&worktree_path, &commands)
                .map(AgentOutcome::BuildTestCompleted);

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client,
                outcome,
            }));
        });
    }

    fn handle_build_test_result(&mut self, outcome: BuildTestOutcome) {
        let pending = self.pending_build_test.take().unwrap();

        match outcome {
            BuildTestOutcome::Success => {
                self.add_system_message(&format!(
                    "[{}] 빌드/테스트 검증 성공.",
                    pending.task_id,
                ));
                self.ff_merge_and_advance(
                    pending.task_id,
                    pending.report,
                );
            }
            BuildTestOutcome::BuildFailed { output } => {
                self.handle_build_test_failure(pending, "빌드", output);
            }
            BuildTestOutcome::TestFailed { output } => {
                self.handle_build_test_failure(pending, "테스트", output);
            }
        }
    }

    fn handle_build_test_failure(
        &mut self,
        pending: PendingBuildTest,
        failure_type: &str,
        output: String,
    ) {
        if pending.is_retry {
            self.add_system_message(&format!(
                "[{}] 수리 후 {} 재실패. 태스크 차단 처리.",
                pending.task_id, failure_type,
            ));
            self.cleanup_current_task_worktree();
            self.save_and_advance_task(
                pending.task_id,
                CodingTaskStatus::ImplementationBlocked,
                format!("{}\n\n---\n빌드/테스트 실패:\n{}", pending.report, output),
            );
        } else {
            self.add_system_message(&format!(
                "[{}] {} 실패. 수리 에이전트 시작...",
                pending.task_id, failure_type,
            ));
            self.start_build_test_repair(
                pending.task_id,
                pending.report,
                output,
            );
        }
    }

    fn start_build_test_repair(
        &mut self,
        task_id: String,
        report: String,
        error_output: String,
    ) {
        self.pending_build_test = Some(PendingBuildTest {
            task_id: task_id.clone(),
            report,
            is_retry: true,
        });

        let commands = self
            .coding_state
            .as_ref()
            .unwrap()
            .build_test_commands
            .as_ref()
            .unwrap();
        let user_prompt = coding::build_build_test_repair_prompt(
            &task_id,
            &commands.build,
            &commands.test,
            &error_output,
        );

        let mut client = match self.claude_client.take() {
            Some(c) => c,
            None => {
                self.add_system_message("수리 에이전트를 위한 세션을 찾을 수 없습니다.");
                let pending = self.pending_build_test.take().unwrap();
                self.cleanup_current_task_worktree();
                self.save_and_advance_task(
                    pending.task_id,
                    CodingTaskStatus::ImplementationBlocked,
                    format!(
                        "{}\n\n---\n빌드/테스트 실패 (수리 불가):\n{}",
                        pending.report, error_output,
                    ),
                );
                return;
            }
        };

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::Coding;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let request = ClaudeCodeRequest {
                user_prompt,
                output_schema: coding::build_test_repair_result_schema(),
            };

            let stream_sender = sender.clone();
            let outcome = client
                .query_streaming::<BuildTestRepairResult, _>(&request, |line| {
                    let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                })
                .map(AgentOutcome::BuildTestRepairCompleted)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client,
                outcome,
            }));
        });
    }

    fn handle_build_test_repair_result(&mut self, result: BuildTestRepairResult) {
        let pending = self.pending_build_test.take().unwrap();

        match result.status {
            BuildTestRepairStatus::Fixed => {
                self.add_system_message(&format!(
                    "[{}] 수리 에이전트 완료. 빌드/테스트 재검증...",
                    pending.task_id,
                ));
                self.start_build_test_execution(
                    pending.task_id,
                    pending.report,
                    true,
                );
            }
            BuildTestRepairStatus::FixFailed => {
                self.add_system_message(&format!(
                    "[{}] 수리 실패: {}",
                    pending.task_id, result.report,
                ));
                self.cleanup_current_task_worktree();
                self.save_and_advance_task(
                    pending.task_id,
                    CodingTaskStatus::ImplementationBlocked,
                    format!(
                        "{}\n\n---\n빌드/테스트 수리 실패: {}",
                        pending.report, result.report,
                    ),
                );
            }
        }
    }

    fn ff_merge_and_advance(
        &mut self,
        task_id: String,
        report: String,
    ) {
        let coding_state = self.coding_state.as_ref().unwrap();
        let worktree_info = coding_state.current_task_worktree.as_ref().unwrap();
        let worktree_path = worktree_info.worktree_path.clone();
        let task_branch = worktree_info.task_branch.clone();
        let integration_branch = coding_state.integration_branch.clone();

        let date_dir = self.session_date_dir.clone().unwrap_or_default();
        let session_name = self.session_name.clone().unwrap_or_default();

        if let Err(err) = coding::save_and_commit_task_report_in_worktree(
            &worktree_path, &date_dir, &session_name, &task_id, &report,
        ) {
            self.add_system_message(&format!(
                "[{}] 워크트리 리포트 커밋 실패: {}. 리포트 없이 진행.",
                task_id, err,
            ));
        }

        self.add_system_message(&format!(
            "[{}] 통합 브랜치로 fast-forward 머지 시작...",
            task_id,
        ));

        let workspace = self.confirmed_workspace.clone().unwrap();
        let report_file_path = workspace
            .join(".bear")
            .join(&date_dir)
            .join(&session_name)
            .join(format!("{}.md", task_id));

        match coding::fast_forward_merge_task_branch(
            &worktree_path,
            &integration_branch,
            &task_branch,
        ) {
            Ok(()) => {
                self.add_system_message(&format!("[{}] fast-forward 머지 완료.", task_id));
                self.cleanup_current_task_worktree();
                self.advance_task(
                    task_id,
                    CodingTaskStatus::ImplementationSuccess,
                    report,
                    report_file_path,
                );
            }
            Err(err) => {
                self.add_system_message(&format!(
                    "[{}] fast-forward 머지 실패: {}",
                    task_id, err
                ));
                self.cleanup_current_task_worktree();
                self.save_and_advance_task(
                    task_id,
                    CodingTaskStatus::ImplementationBlocked,
                    format!("{}\n\n---\nfast-forward 머지 실패: {}", report, err),
                );
            }
        }
    }

    fn start_conflict_resolution(
        &mut self,
        task_id: String,
        conflicted_files: Vec<String>,
        original_report: String,
    ) {
        self.pending_coding_report = Some(original_report);

        let mut client = match self.claude_client.take() {
            Some(c) => c,
            None => {
                self.add_system_message("충돌 해결을 위한 에이전트 세션을 찾을 수 없습니다.");
                self.pending_coding_report = None;
                let _ = coding::abort_rebase(
                    &self
                        .coding_state
                        .as_ref()
                        .unwrap()
                        .current_task_worktree
                        .as_ref()
                        .unwrap()
                        .worktree_path,
                );
                self.cleanup_current_task_worktree();
                self.save_and_advance_task(
                    task_id,
                    CodingTaskStatus::ImplementationBlocked,
                    "충돌 해결 세션을 찾을 수 없음".to_string(),
                );
                return;
            }
        };

        let integration_branch = self
            .coding_state
            .as_ref()
            .unwrap()
            .integration_branch
            .clone();

        let user_prompt = coding::build_conflict_resolution_prompt(
            &task_id,
            &integration_branch,
            &conflicted_files,
        );

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::Coding;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let request = ClaudeCodeRequest {
                user_prompt,
                output_schema: coding::conflict_resolution_result_schema(),
            };

            let stream_sender = sender.clone();
            let outcome = client
                .query_streaming::<ConflictResolutionResult, _>(&request, |line| {
                    let _ = stream_sender.send(AgentStreamMessage::StreamLine(line));
                })
                .map(AgentOutcome::ConflictResolutionCompleted)
                .map_err(|err| err.to_string());

            let _ = sender.send(AgentStreamMessage::Completed(AgentThreadResult {
                client,
                outcome,
            }));
        });
    }

    fn handle_conflict_resolution_result(&mut self, result: ConflictResolutionResult) {
        let task_id = {
            let coding_state = self.coding_state.as_ref().unwrap();
            coding_state.tasks[coding_state.current_task_index]
                .task_id
                .clone()
        };

        match result.status {
            ConflictResolutionStatus::ConflictResolved => {
                self.add_system_message(&format!("[{}] 충돌 해결 완료.", task_id));
                let report = self
                    .pending_coding_report
                    .take()
                    .unwrap_or(result.report);
                self.verify_build_and_test(task_id, report);
            }
            ConflictResolutionStatus::ConflictResolutionFailed => {
                self.add_system_message(&format!(
                    "[{}] 충돌 해결 실패: {}",
                    task_id, result.report,
                ));
                let worktree_path = self
                    .coding_state
                    .as_ref()
                    .unwrap()
                    .current_task_worktree
                    .as_ref()
                    .unwrap()
                    .worktree_path
                    .clone();
                let _ = coding::abort_rebase(&worktree_path);
                self.pending_coding_report = None;
                self.cleanup_current_task_worktree();
                self.save_and_advance_task(
                    task_id,
                    CodingTaskStatus::ImplementationBlocked,
                    format!("충돌 해결 실패: {}", result.report),
                );
            }
        }
    }

    fn save_and_advance_task(
        &mut self,
        task_id: String,
        status: CodingTaskStatus,
        report: String,
    ) {
        let journal_dir = self.journal_dir();
        let report_file_path = match coding::save_task_report(
            &journal_dir,
            &task_id,
            &report,
        ) {
            Ok(path) => path,
            Err(err) => {
                self.add_system_message(&format!("Failed to save report: {}", err));
                PathBuf::new()
            }
        };

        self.advance_task(task_id, status, report, report_file_path);
    }

    fn advance_task(
        &mut self,
        task_id: String,
        status: CodingTaskStatus,
        report: String,
        report_file_path: PathBuf,
    ) {
        let coding_state = self.coding_state.as_mut().unwrap();
        coding_state.task_reports.push(TaskReport {
            task_id,
            status,
            report,
            report_file_path,
        });
        coding_state.current_task_index += 1;

        self.start_next_coding_task();
    }

    fn finish_coding_phase(&mut self) {
        let coding_state = self.coding_state.as_ref().unwrap();
        let integration_branch = coding_state.integration_branch.clone();

        let success_count = coding_state
            .task_reports
            .iter()
            .filter(|r| r.status == CodingTaskStatus::ImplementationSuccess)
            .count();
        let blocked_count = coding_state
            .task_reports
            .iter()
            .filter(|r| r.status == CodingTaskStatus::ImplementationBlocked)
            .count();

        self.add_system_message(&format!(
            "코딩 단계 완료. 성공: {}, 차단: {}",
            success_count, blocked_count,
        ));

        self.add_system_message(&format!(
            "통합 브랜치가 유지됩니다: {}",
            integration_branch,
        ));

        self.input_mode = InputMode::Done;
    }

    pub fn open_external_editor(&mut self) {
        self.pending_external_editor = false;

        let temp_path = std::env::temp_dir().join(
            format!("bear-input-{}.md", uuid::Uuid::new_v4()),
        );

        if let Err(err) = std::fs::File::create(&temp_path)
            .and_then(|mut f| f.write_all(self.input_buffer.as_bytes()))
        {
            self.add_system_message(&format!("임시 파일 생성 실패: {}", err));
            return;
        }

        let editor_command = std::env::var("EDITOR").unwrap_or_else(|_| "code --wait".to_string());
        let parts: Vec<&str> = editor_command.split_whitespace().collect();
        let (program, args) = match parts.split_first() {
            Some((prog, rest)) => (*prog, rest),
            None => {
                self.add_system_message("EDITOR 환경변수가 비어 있습니다.");
                let _ = std::fs::remove_file(&temp_path);
                return;
            }
        };

        let status = std::process::Command::new(program)
            .args(args)
            .arg(&temp_path)
            .stdin(std::process::Stdio::inherit())
            .stdout(std::process::Stdio::inherit())
            .stderr(std::process::Stdio::inherit())
            .status();

        match status {
            Ok(exit_status) if exit_status.success() => {
                match std::fs::read_to_string(&temp_path) {
                    Ok(content) => {
                        self.input_buffer = content;
                        self.cursor_position = self.input_buffer.chars().count();
                    }
                    Err(err) => {
                        self.add_system_message(
                            &format!("임시 파일 읽기 실패: {}", err),
                        );
                    }
                }
            }
            Ok(_) => {
                self.add_system_message("에디터가 비정상 종료되었습니다.");
            }
            Err(err) => {
                self.add_system_message(
                    &format!("에디터 실행 실패: {} (command: {})", err, editor_command),
                );
            }
        }

        let _ = std::fs::remove_file(&temp_path);
    }

    fn is_newline_modifier(&self, modifiers: KeyModifiers) -> bool {
        if self.keyboard_enhancement_enabled {
            modifiers.contains(KeyModifiers::SHIFT)
        } else {
            modifiers.contains(KeyModifiers::ALT)
        }
    }

    fn insert_char_at_cursor(&mut self, c: char) {
        let byte_pos = char_to_byte_index(&self.input_buffer, self.cursor_position);
        self.input_buffer.insert(byte_pos, c);
        self.cursor_position += 1;
    }

    fn insert_text_at_cursor(&mut self, text: &str) {
        let byte_pos = char_to_byte_index(&self.input_buffer, self.cursor_position);
        self.input_buffer.insert_str(byte_pos, text);
        self.cursor_position += text.chars().count();
    }

    fn delete_char_before_cursor(&mut self) {
        if self.cursor_position == 0 {
            return;
        }
        self.cursor_position -= 1;
        let byte_pos = char_to_byte_index(&self.input_buffer, self.cursor_position);
        self.input_buffer.remove(byte_pos);
    }

    fn delete_char_at_cursor(&mut self) {
        let char_count = self.input_buffer.chars().count();
        if self.cursor_position >= char_count {
            return;
        }
        let byte_pos = char_to_byte_index(&self.input_buffer, self.cursor_position);
        self.input_buffer.remove(byte_pos);
    }

    fn clear_input(&mut self) {
        self.input_buffer.clear();
        self.cursor_position = 0;
    }

    fn move_cursor_left(&mut self) {
        if self.cursor_position > 0 {
            self.cursor_position -= 1;
        }
    }

    fn move_cursor_right(&mut self) {
        if self.cursor_position < self.input_buffer.chars().count() {
            self.cursor_position += 1;
        }
    }

    fn move_cursor_up(&mut self) {
        let visual_lines = self.compute_visual_lines();
        let (current_line, current_col) = find_cursor_visual_position(
            self.cursor_position,
            &visual_lines,
        );

        if current_line == 0 {
            return;
        }

        let target = &visual_lines[current_line - 1];
        let max_col = if target.is_last_of_logical {
            target.char_count
        } else {
            target.char_count.saturating_sub(1)
        };
        self.cursor_position = target.char_start + current_col.min(max_col);
    }

    fn move_cursor_down(&mut self) {
        let visual_lines = self.compute_visual_lines();
        let (current_line, current_col) = find_cursor_visual_position(
            self.cursor_position,
            &visual_lines,
        );

        if current_line >= visual_lines.len() - 1 {
            return;
        }

        let target = &visual_lines[current_line + 1];
        let max_col = if target.is_last_of_logical {
            target.char_count
        } else {
            target.char_count.saturating_sub(1)
        };
        self.cursor_position = target.char_start + current_col.min(max_col);
    }

    fn compute_visual_lines(&self) -> Vec<VisualLineInfo> {
        let cursor_reserved = 1;
        let text_width = (self.terminal_width as usize).saturating_sub(USER_PREFIX.len() + cursor_reserved);

        let logical_lines: Vec<&str> = self.input_buffer.split('\n').collect();
        let mut result = Vec::new();
        let mut global_char_offset = 0;

        for (logical_idx, logical_line) in logical_lines.iter().enumerate() {
            let wrapped = wrap_text_by_char_width(logical_line, text_width);
            let wrap_count = wrapped.len();
            let mut line_char_offset = 0;

            for (wrap_idx, visual_text) in wrapped.iter().enumerate() {
                let char_count = visual_text.chars().count();
                result.push(VisualLineInfo {
                    char_start: global_char_offset + line_char_offset,
                    char_count,
                    is_last_of_logical: wrap_idx == wrap_count - 1,
                });
                line_char_offset += char_count;
            }

            global_char_offset += logical_line.chars().count();
            if logical_idx < logical_lines.len() - 1 {
                global_char_offset += 1; // '\n'
            }
        }

        result
    }

    fn add_system_message(&mut self, content: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::System,
            content: content.to_string(),
        });
    }

    fn add_user_message(&mut self, content: &str) {
        self.messages.push(ChatMessage {
            role: MessageRole::User,
            content: content.to_string(),
        });
    }
}

struct VisualLineInfo {
    char_start: usize,
    char_count: usize,
    is_last_of_logical: bool,
}

fn char_to_byte_index(s: &str, char_idx: usize) -> usize {
    s.char_indices()
        .nth(char_idx)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
}

fn find_cursor_visual_position(
    cursor_position: usize,
    visual_lines: &[VisualLineInfo],
) -> (usize, usize) {
    for (i, vl) in visual_lines.iter().enumerate() {
        let vl_end = vl.char_start + vl.char_count;

        if cursor_position >= vl.char_start && cursor_position < vl_end {
            return (i, cursor_position - vl.char_start);
        }

        if cursor_position == vl_end && vl.is_last_of_logical {
            return (i, vl.char_count);
        }
    }

    let last = visual_lines.len().saturating_sub(1);
    (last, visual_lines.get(last).map_or(0, |vl| vl.char_count))
}

fn generate_session_name(client: &mut ClaudeCodeClient, requirements: &str) -> String {
    let request = ClaudeCodeRequest {
        user_prompt: session_naming::build_session_name_prompt(requirements),
        output_schema: session_naming::session_name_schema(),
    };

    match client.query::<SessionNameResponse>(&request) {
        Ok(response) => session_naming::sanitize_session_name(&response.session_name),
        Err(_) => "unnamed-session".to_string(),
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
