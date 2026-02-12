use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::claude_code_client::{ClaudeCodeClient, ClaudeCodeRequest};
use crate::config::Config;
use super::clarification::{self, ClarificationQuestions, QaRound};
use super::planning::{self, PlanJournal, PlanResponseType, PlanWritingResponse};
use super::spec_writing::{self, SpecJournal, SpecResponseType, SpecWritingResponse};
use super::error::UiError;
use super::renderer::{USER_PREFIX, wrap_text_by_char_width};

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
    AgentThinking,
    ClarificationAnswer,
    SpecClarificationAnswer,
    SpecFeedback,
    PlanClarificationAnswer,
    PlanFeedback,
    Done,
}

enum AgentOutcome {
    Clarification(ClarificationQuestions),
    SpecWriting(SpecWritingResponse),
    Planning(PlanWritingResponse),
}

struct AgentThreadResult {
    client: ClaudeCodeClient,
    outcome: Result<AgentOutcome, String>,
}

enum AgentStreamMessage {
    StreamLine(String),
    Completed(AgentThreadResult),
}

pub struct App {
    pub messages: Vec<ChatMessage>,
    input_mode: InputMode,
    pub input_buffer: String,
    pub cursor_position: usize,
    pub terminal_width: u16,
    pub terminal_height: u16,
    pub confirmed_workspace: Option<PathBuf>,
    pub confirmed_requirements: Option<String>,
    pub should_quit: bool,
    pub cursor_visible: bool,
    cursor_blink_at: Instant,
    current_directory: PathBuf,
    pub scroll_offset: u16,
    keyboard_enhancement_enabled: bool,
    config: Config,
    claude_client: Option<ClaudeCodeClient>,
    agent_result_receiver: Option<mpsc::Receiver<AgentStreamMessage>>,
    qa_log: Vec<QaRound>,
    current_round_questions: Vec<String>,
    thinking_started_at: Instant,
    journal: Option<SpecJournal>,
    last_spec_draft: Option<String>,
    spec_clarification_questions: Vec<String>,
    plan_journal: Option<PlanJournal>,
    last_plan_draft: Option<String>,
    plan_clarification_questions: Vec<String>,
    approved_spec: Option<String>,
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
            terminal_height: 24,
            confirmed_workspace: None,
            confirmed_requirements: None,
            should_quit: false,
            cursor_visible: true,
            cursor_blink_at: Instant::now(),
            current_directory,
            scroll_offset: 0,
            keyboard_enhancement_enabled: false,
            config,
            claude_client: None,
            agent_result_receiver: None,
            qa_log: Vec::new(),
            current_round_questions: Vec::new(),
            thinking_started_at: Instant::now(),
            journal: None,
            last_spec_draft: None,
            spec_clarification_questions: Vec::new(),
            plan_journal: None,
            last_plan_draft: None,
            plan_clarification_questions: Vec::new(),
            approved_spec: None,
        })
    }

    pub fn handle_key_event(&mut self, key_event: KeyEvent) {
        self.reset_cursor_blink();

        match key_event.code {
            KeyCode::PageUp => {
                self.scroll_up();
                return;
            }
            KeyCode::PageDown => {
                self.scroll_down();
                return;
            }
            _ => {}
        }

        match self.input_mode {
            InputMode::WorkspaceConfirm => self.handle_workspace_confirm(key_event),
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
            InputMode::AgentThinking | InputMode::Done => {
                if key_event.code == KeyCode::Esc {
                    self.should_quit = true;
                }
            }
        }
    }

    pub fn handle_paste(&mut self, text: String) {
        self.reset_cursor_blink();

        match self.input_mode {
            InputMode::WorkspaceConfirm => {
                let cleaned = text.replace("\r\n", " ").replace(['\r', '\n'], " ");
                self.insert_text_at_cursor(&cleaned);
            }
            InputMode::RequirementsInput
            | InputMode::ClarificationAnswer
            | InputMode::SpecClarificationAnswer
            | InputMode::SpecFeedback
            | InputMode::PlanClarificationAnswer
            | InputMode::PlanFeedback => {
                let cleaned = text.replace("\r\n", "\n").replace('\r', "\n");
                self.insert_text_at_cursor(&cleaned);
            }
            InputMode::AgentThinking | InputMode::Done => {}
        }
    }

    pub fn tick(&mut self) {
        self.tick_cursor_blink();
        self.tick_agent_result();
    }

    fn tick_cursor_blink(&mut self) {
        if self.cursor_blink_at.elapsed() >= CURSOR_BLINK_INTERVAL {
            self.cursor_visible = !self.cursor_visible;
            self.cursor_blink_at = Instant::now();
        }
    }

    fn tick_agent_result(&mut self) {
        let receiver = match self.agent_result_receiver.take() {
            Some(r) => r,
            None => return,
        };

        loop {
            match receiver.try_recv() {
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
                        Err(error_message) => self.handle_agent_error(error_message),
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

    fn reset_cursor_blink(&mut self) {
        self.cursor_visible = true;
        self.cursor_blink_at = Instant::now();
    }

    pub fn scroll_up(&mut self) {
        let page_size = self.terminal_height.saturating_sub(2);
        self.scroll_offset = self.scroll_offset.saturating_add(page_size);
    }

    pub fn scroll_down(&mut self) {
        let page_size = self.terminal_height.saturating_sub(2);
        self.scroll_offset = self.scroll_offset.saturating_sub(page_size);
    }

    pub fn set_keyboard_enhancement_enabled(&mut self, enabled: bool) {
        self.keyboard_enhancement_enabled = enabled;
    }

    pub fn is_waiting_for_input(&self) -> bool {
        matches!(
            self.input_mode,
            InputMode::WorkspaceConfirm
                | InputMode::RequirementsInput
                | InputMode::ClarificationAnswer
                | InputMode::SpecClarificationAnswer
                | InputMode::SpecFeedback
                | InputMode::PlanClarificationAnswer
                | InputMode::PlanFeedback
        )
    }

    pub fn is_thinking(&self) -> bool {
        matches!(self.input_mode, InputMode::AgentThinking)
    }

    pub fn thinking_indicator(&self) -> &'static str {
        let dots = (self.thinking_started_at.elapsed().as_millis() / 500) % 4;
        match dots {
            0 => "Analyzing",
            1 => "Analyzing.",
            2 => "Analyzing..",
            _ => "Analyzing...",
        }
    }

    pub fn help_text(&self) -> &str {
        match self.input_mode {
            InputMode::WorkspaceConfirm => "[Enter] Confirm  [PgUp/PgDn] Scroll  [Esc] Quit",
            InputMode::RequirementsInput
            | InputMode::ClarificationAnswer
            | InputMode::SpecClarificationAnswer
            | InputMode::PlanClarificationAnswer => {
                if self.keyboard_enhancement_enabled {
                    "[Enter] Submit  [Shift+Enter] New line  [PgUp/PgDn] Scroll  [Esc] Quit"
                } else {
                    "[Enter] Submit  [Alt+Enter] New line  [PgUp/PgDn] Scroll  [Esc] Quit"
                }
            }
            InputMode::SpecFeedback | InputMode::PlanFeedback => {
                if self.keyboard_enhancement_enabled {
                    "[Enter] Submit feedback  [Ctrl+A] Approve  [Shift+Enter] New line  [PgUp/PgDn] Scroll  [Esc] Quit"
                } else {
                    "[Enter] Submit feedback  [Ctrl+A] Approve  [Alt+Enter] New line  [PgUp/PgDn] Scroll  [Esc] Quit"
                }
            }
            InputMode::AgentThinking | InputMode::Done => "[PgUp/PgDn] Scroll  [Esc] Quit",
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
                self.transition_to_requirements_input();
            }
            KeyCode::Backspace => {
                self.delete_char_before_cursor();
            }
            KeyCode::Left => {
                self.move_cursor_left();
            }
            KeyCode::Right => {
                self.move_cursor_right();
            }
            KeyCode::Esc => {
                self.should_quit = true;
            }
            KeyCode::Char(c) => {
                self.insert_char_at_cursor(c);
            }
            _ => {}
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
            KeyCode::Char(c) => {
                self.insert_char_at_cursor(c);
            }
            _ => {}
        }
    }

    fn transition_to_requirements_input(&mut self) {
        self.add_system_message("구현할 요구사항을 입력하세요.");
        self.input_mode = InputMode::RequirementsInput;
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
        let client = ClaudeCodeClient::new(self.config.api_key().to_string(), vec![workspace])
            .map_err(|err| err.to_string())?;

        self.claude_client = Some(client);
        Ok(())
    }

    fn start_clarification_query(&mut self) {
        let mut client = self.claude_client.take().expect("client must be available");
        let original_request = self.confirmed_requirements.clone().unwrap();
        let qa_log = self.qa_log.clone();

        let (sender, receiver) = mpsc::channel();
        self.agent_result_receiver = Some(receiver);
        self.input_mode = InputMode::AgentThinking;
        self.thinking_started_at = Instant::now();

        std::thread::spawn(move || {
            let request = ClaudeCodeRequest {
                system_prompt: Some(clarification::system_prompt().to_string()),
                user_prompt: clarification::build_user_prompt(&original_request, &qa_log),
                model: None,
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
        self.input_mode = InputMode::Done;
    }

    fn start_spec_writing_query(&mut self, is_initial: bool) {
        let mut client = self.claude_client.take().expect("client must be available");

        if is_initial {
            client.reset_session();
        }

        let original_request = self.confirmed_requirements.clone().unwrap();
        let qa_log = self.qa_log.clone();
        let journal_path = self.journal.as_ref().map(|j| j.file_path().to_path_buf());
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
                spec_writing::build_initial_spec_prompt(&original_request, &qa_log)
            } else {
                let feedback = user_feedback.unwrap_or_default();
                let path = journal_path.as_deref().unwrap_or(Path::new("unknown"));
                spec_writing::build_revision_prompt(&feedback, path)
            };

            let request = ClaudeCodeRequest {
                system_prompt: Some(clarification::system_prompt().to_string()),
                user_prompt,
                model: None,
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
        self.ensure_journal();

        match response.response_type {
            SpecResponseType::SpecDraft => {
                let draft = response.spec_draft.unwrap_or_default();

                if let Some(journal) = &self.journal {
                    let _ = journal.append_spec_draft(&draft);
                }

                self.add_system_message(&format!(
                    "스펙 드래프트가 작성되었습니다:\n\n{}\n\n피드백을 입력하거나, Ctrl+A를 눌러 승인하세요.",
                    draft
                ));
                self.last_spec_draft = Some(draft);
                self.input_mode = InputMode::SpecFeedback;
            }
            SpecResponseType::ClarifyingQuestions => {
                let questions = response.clarifying_questions.unwrap_or_default();

                if let Some(journal) = &self.journal {
                    let _ = journal.append_clarifying_questions(&questions);
                }

                let mut message = String::from("스펙 작성을 위해 추가 정보가 필요합니다.\n");
                for (i, question) in questions.iter().enumerate() {
                    message.push_str(&format!("\n{}. {}", i + 1, question));
                }

                self.spec_clarification_questions = questions;
                self.add_system_message(&message);
                self.input_mode = InputMode::SpecClarificationAnswer;
            }
        }
    }

    fn ensure_journal(&mut self) {
        if self.journal.is_some() {
            return;
        }

        let workspace = match &self.confirmed_workspace {
            Some(w) => w.clone(),
            None => return,
        };

        let session_id = self
            .claude_client
            .as_ref()
            .and_then(|c| c.session_id())
            .unwrap_or("unknown")
            .to_string();

        match SpecJournal::new(&workspace, &session_id) {
            Ok(journal) => {
                // Phase 1 데이터를 소급 기록한다.
                if let Some(request) = &self.confirmed_requirements {
                    let _ = journal.append_user_request(request);
                }
                let _ = journal.append_qa_log(&self.qa_log);

                self.journal = Some(journal);
            }
            Err(err) => {
                self.add_system_message(&format!("저널 파일 생성 실패: {}", err));
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

        if let Some(journal) = &self.journal {
            let _ = journal.append_user_answers(&answer);
        }

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

        if let Some(journal) = &self.journal {
            let _ = journal.append_user_feedback(&feedback);
        }

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

        if let Some(journal) = &self.journal {
            let _ = journal.append_approved_spec(&spec);
        }

        self.approved_spec = Some(spec);
        self.add_system_message("스펙이 승인되었습니다. 개발 계획을 작성합니다.");
        self.start_plan_writing_query(true);
    }

    fn start_plan_writing_query(&mut self, is_initial: bool) {
        let mut client = self.claude_client.take().expect("client must be available");

        if is_initial {
            client.reset_session();
        }

        let approved_spec = self.approved_spec.clone().unwrap_or_default();
        let plan_journal_path = self.plan_journal.as_ref().map(|j| j.file_path().to_path_buf());
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
                planning::build_initial_plan_prompt(&approved_spec)
            } else {
                let feedback = user_feedback.unwrap_or_default();
                let path = plan_journal_path.as_deref().unwrap_or(Path::new("unknown"));
                planning::build_plan_revision_prompt(&feedback, path)
            };

            let request = ClaudeCodeRequest {
                system_prompt: Some(planning::system_prompt().to_string()),
                user_prompt,
                model: None,
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
        self.ensure_plan_journal();

        match response.response_type {
            PlanResponseType::PlanDraft => {
                let draft = response.plan_draft.unwrap_or_default();

                if let Some(journal) = &self.plan_journal {
                    let _ = journal.append_plan_draft(&draft);
                }

                self.add_system_message(&format!(
                    "개발 계획 드래프트가 작성되었습니다:\n\n{}\n\n피드백을 입력하거나, Ctrl+A를 눌러 승인하세요.",
                    draft
                ));
                self.last_plan_draft = Some(draft);
                self.input_mode = InputMode::PlanFeedback;
            }
            PlanResponseType::ClarifyingQuestions => {
                let questions = response.clarifying_questions.unwrap_or_default();

                if let Some(journal) = &self.plan_journal {
                    let _ = journal.append_clarifying_questions(&questions);
                }

                let mut message = String::from("개발 계획 작성을 위해 추가 정보가 필요합니다.\n");
                for (i, question) in questions.iter().enumerate() {
                    message.push_str(&format!("\n{}. {}", i + 1, question));
                }

                self.plan_clarification_questions = questions;
                self.add_system_message(&message);
                self.input_mode = InputMode::PlanClarificationAnswer;
            }
        }
    }

    fn ensure_plan_journal(&mut self) {
        if self.plan_journal.is_some() {
            return;
        }

        let workspace = match &self.confirmed_workspace {
            Some(w) => w.clone(),
            None => return,
        };

        let session_id = self
            .claude_client
            .as_ref()
            .and_then(|c| c.session_id())
            .unwrap_or("unknown")
            .to_string();

        match PlanJournal::new(&workspace, &session_id) {
            Ok(journal) => {
                // 승인된 스펙을 plan journal에 소급 기록한다.
                if let Some(spec) = &self.approved_spec {
                    let _ = journal.append_approved_spec(spec);
                }
                self.plan_journal = Some(journal);
            }
            Err(err) => {
                self.add_system_message(&format!("플랜 저널 파일 생성 실패: {}", err));
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

        if let Some(journal) = &self.plan_journal {
            let _ = journal.append_user_answers(&answer);
        }

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

        if let Some(journal) = &self.plan_journal {
            let _ = journal.append_user_feedback(&feedback);
        }

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

        if let Some(journal) = &self.plan_journal {
            let _ = journal.append_approved_plan(&plan);
        }

        self.add_system_message("개발 계획이 승인되었습니다.");
        self.input_mode = InputMode::Done;
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
