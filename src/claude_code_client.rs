mod binary_finder;
mod error;
pub mod logger;
mod response;

pub use error::ClaudeCodeClientError;
pub use response::CliResponse;

use std::path::PathBuf;
use std::io::BufRead;
use std::process::{Command, Stdio};

use serde::de::DeserializeOwned;

const TOOLS_LIST: &str = "AskUserQuestion,Bash,TaskOutput,Edit,ExitPlanMode,Glob,Grep,\
    KillShell,MCPSearch,Read,Skill,Task,TaskCreate,TaskGet,TaskList,TaskUpdate,\
    WebFetch,WebSearch,Write,LSP";

pub struct ClaudeCodeRequest {
    pub user_prompt: String,
    pub output_schema: serde_json::Value,
}

#[derive(Debug)]
struct ParsedOutput<T> {
    result: T,
    session_id: String,
}

fn parse_cli_output<T: DeserializeOwned>(
    stdout: &[u8],
) -> Result<ParsedOutput<T>, ClaudeCodeClientError> {
    // CLI 출력에서 메시지 배열을 추출한다. 표준 출력 형식은 JSON 배열이지만,
    // 단일 객체가 올 수도 있으므로 둘 다 처리한다.
    let messages: Vec<serde_json::Value> = match serde_json::from_slice(stdout) {
        Ok(messages) => messages,
        Err(_) => {
            let single: serde_json::Value = serde_json::from_slice(stdout)?;
            vec![single]
        }
    };

    let result_value = messages
        .into_iter()
        .rev()
        .find(|msg| msg.get("type").and_then(|v| v.as_str()) == Some("result"))
        .ok_or(ClaudeCodeClientError::NoResultMessage)?;

    let response: CliResponse = serde_json::from_value(result_value)?;
    if response.is_error {
        return Err(ClaudeCodeClientError::CliReturnedError {
            message: response.result.unwrap_or_default(),
        });
    }

    let output_value = match response.structured_output {
        Some(value) => value,
        None => return Err(ClaudeCodeClientError::MissingStructuredOutput),
    };

    let result: T = serde_json::from_value(output_value)?;

    Ok(ParsedOutput {
        result,
        session_id: response.session_id,
    })
}

pub struct ClaudeCodeClient {
    binary_path: PathBuf,
    api_key: String,
    additional_work_directories: Vec<PathBuf>,
    session_id: Option<String>,
    working_directory: Option<PathBuf>,
    system_prompt: Option<String>,
    pending_system_prompt: Option<String>,
}

impl ClaudeCodeClient {
    pub fn session_id(&self) -> Option<&str> {
        self.session_id.as_deref()
    }

    pub fn reset_session(&mut self) {
        self.session_id = None;
    }

    pub fn set_working_directory(&mut self, path: PathBuf) {
        self.working_directory = Some(path);
    }

    pub fn set_system_prompt(&mut self, prompt: Option<String>) {
        self.system_prompt = prompt;
    }

    pub fn append_system_prompt(&mut self, prompt: String) {
        self.pending_system_prompt = Some(prompt);
    }

    pub fn new(
        api_key: String,
        additional_work_directories: Vec<PathBuf>,
        system_prompt: Option<String>,
    ) -> Result<Self, ClaudeCodeClientError> {
        let binary_path = binary_finder::find_claude_binary()?;

        for directory in &additional_work_directories {
            if !directory.exists() {
                std::fs::create_dir_all(directory).map_err(|err| {
                    ClaudeCodeClientError::DirectoryCreationFailed {
                        path: directory.display().to_string(),
                        source: err,
                    }
                })?;
            }
        }

        Ok(Self {
            binary_path,
            api_key,
            additional_work_directories,
            session_id: None,
            working_directory: None,
            system_prompt,
            pending_system_prompt: None,
        })
    }

    fn build_base_command(&mut self, request: &ClaudeCodeRequest) -> (Command, Option<String>, Option<String>) {
        let model_effort_level = "high";
        let disable_auto_memory = "0";  // 0 = force enable.
        let disable_feedback_survey = "1";

        let mut command = Command::new(&self.binary_path);

        if let Some(dir) = &self.working_directory {
            command.current_dir(dir);
        }

        command
            .env("ANTHROPIC_API_KEY", &self.api_key)
            .env("CLAUDE_CODE_EFFORT_LEVEL", model_effort_level)
            .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", disable_auto_memory)
            .env("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY", disable_feedback_survey)
            .arg("-p")
            .arg("--allow-dangerously-skip-permissions")
            .arg("--permission-mode").arg("bypassPermissions")
            .arg("--tools").arg(TOOLS_LIST);

        // 최초 실행이면 새 세션 ID를 생성하고, 후속 실행이면 기존 세션을 재개한다.
        let new_session_id = match &self.session_id {
            Some(existing_id) => {
                command.arg("--resume").arg(existing_id);
                None
            }
            None => {
                let id = uuid::Uuid::new_v4().to_string();
                command.arg("--session-id").arg(&id);
                Some(id)
            }
        };

        if !self.additional_work_directories.is_empty() {
            command.arg("--add-dir");
            for directory in &self.additional_work_directories {
                command.arg(directory);
            }
        }

        command.arg("--model").arg("claude-opus-4-6");

        // 커스텀 시스템 프롬프트는 기존 세션 컨텍스트에 저장되지 않기 때문에 과거 세션을 불러와서
        // 재사용하는 경우에는 기존에 입력했던 커스텀 시스템 프롬프트를 다시 입력해주어야 한다.
        let mut prompt_parts: Vec<String> = Vec::new();
        if let Some(sp) = &self.system_prompt {
            prompt_parts.push(sp.clone());
        }
        if let Some(sp) = self.pending_system_prompt.take() {
            prompt_parts.push(sp);
        }
        let sent_system_prompt = if prompt_parts.is_empty() {
            None
        } else {
            let combined = prompt_parts.join("\n\n");
            command.arg("--append-system-prompt").arg(&combined);
            Some(combined)
        };

        let output_schema_string = request.output_schema.to_string();
        command.arg("--json-schema").arg(&output_schema_string);

        (command, new_session_id, sent_system_prompt)
    }

    fn log_invocation_details(
        &self,
        mode: &str,
        request: &ClaudeCodeRequest,
        new_session_id: &Option<String>,
        extra_args: &[&str],
        sent_system_prompt: &Option<String>,
    ) {
        let loc = "ClaudeCodeClient::log_invocation_details";
        let log = |msg: String| logger::write_log(loc, &msg);

        log(format!("[{}] 바이너리: {}", mode, self.binary_path.display()));

        if let Some(dir) = &self.working_directory {
            log(format!("[{}] 작업 디렉토리: {}", mode, dir.display()));
        }

        log(format!(
            "[{}] 환경 변수: ANTHROPIC_API_KEY=***, \
             CLAUDE_CODE_EFFORT_LEVEL=high, \
             CLAUDE_CODE_DISABLE_AUTO_MEMORY=0, \
             CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY=1",
            mode,
        ));

        log(format!(
            "[{}] CLI 기본 인수: -p --allow-dangerously-skip-permissions \
             --permission-mode bypassPermissions --tools {}",
            mode, TOOLS_LIST,
        ));

        let session_info = match new_session_id {
            Some(id) => format!("신규 생성 --session-id {}", id),
            None => format!(
                "기존 세션 재개 --resume {}",
                self.session_id.as_deref().unwrap_or("unknown"),
            ),
        };
        log(format!("[{}] 세션: {}", mode, session_info));

        if !self.additional_work_directories.is_empty() {
            let dirs: Vec<String> = self
                .additional_work_directories
                .iter()
                .map(|d| d.display().to_string())
                .collect();
            log(format!(
                "[{}] 추가 작업 디렉토리 (--add-dir): {}",
                mode,
                dirs.join(" "),
            ));
        }

        log(format!("[{}] 모델 (--model): claude-opus-4-6", mode));

        if !extra_args.is_empty() {
            log(format!(
                "[{}] 추가 CLI 인수: {}",
                mode,
                extra_args.join(" "),
            ));
        }

        if let Some(system_prompt) = sent_system_prompt {
            log(format!(
                "[{}] 시스템 프롬프트 (--append-system-prompt, {} bytes):\n{}",
                mode,
                system_prompt.len(),
                system_prompt,
            ));
        }

        log(format!(
            "[{}] 출력 스키마 (--json-schema): {}",
            mode, request.output_schema,
        ));

        log(format!(
            "[{}] 사용자 프롬프트 ({} bytes):\n{}",
            mode,
            request.user_prompt.len(),
            request.user_prompt,
        ));
    }

    pub fn query<T: DeserializeOwned>(
        &mut self,
        request: &ClaudeCodeRequest,
    ) -> Result<T, ClaudeCodeClientError> {
        let (mut command, new_session_id, sent_system_prompt) = self.build_base_command(request);
        command.arg("--output-format").arg("json");
        command.arg(&request.user_prompt);

        crate::cli_log!("[비스트리밍 쿼리 시작]");
        self.log_invocation_details(
            "비스트리밍 쿼리",
            request,
            &new_session_id,
            &["--output-format", "json"],
            &sent_system_prompt,
        );

        let output = command.output().map_err(|err| {
            crate::cli_log!("[비스트리밍 쿼리 실패] 명령 실행 오류: {}", err);
            ClaudeCodeClientError::CommandExecutionFailed {
                message: err.to_string(),
            }
        })?;

        crate::cli_log!("[비스트리밍 쿼리 완료] 종료 코드: {}", output.status);

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            crate::cli_log!("[비스트리밍 쿼리 실패] stderr:\n{}", stderr);
            return Err(ClaudeCodeClientError::CommandExecutionFailed {
                message: stderr.to_string(),
            });
        }

        let stdout_str = String::from_utf8_lossy(&output.stdout);
        crate::cli_log!(
            "[비스트리밍 쿼리] CLI stdout ({} bytes):\n{}",
            output.stdout.len(),
            stdout_str,
        );

        let command_session_id = new_session_id
            .as_deref()
            .or(self.session_id.as_deref())
            .unwrap_or("unknown");
        write_debug_log(&sent_system_prompt, &request.user_prompt, command_session_id, &output.stdout);

        let parsed: ParsedOutput<T> = parse_cli_output(&output.stdout)?;

        if new_session_id.is_some() {
            self.session_id = Some(parsed.session_id);
        }

        Ok(parsed.result)
    }

    pub fn query_streaming<T, F>(
        &mut self,
        request: &ClaudeCodeRequest,
        on_stream_message: F,
    ) -> Result<T, ClaudeCodeClientError>
    where
        T: DeserializeOwned,
        F: Fn(String),
    {
        let (mut command, new_session_id, sent_system_prompt) = self.build_base_command(request);
        command.arg("--output-format").arg("stream-json");
        command.arg("--verbose");
        command.arg("--include-partial-messages");
        command.arg(&request.user_prompt);

        crate::cli_log!("[스트리밍 쿼리 시작]");
        self.log_invocation_details(
            "스트리밍 쿼리",
            request,
            &new_session_id,
            &[
                "--output-format",
                "stream-json",
                "--verbose",
                "--include-partial-messages",
            ],
            &sent_system_prompt,
        );

        let mut child = command
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|err| {
                crate::cli_log!("[스트리밍 쿼리 실패] 프로세스 생성 오류: {}", err);
                ClaudeCodeClientError::CommandExecutionFailed {
                    message: err.to_string(),
                }
            })?;

        crate::cli_log!(
            "[스트리밍 쿼리] 프로세스 생성 완료 (pid: {})",
            child.id(),
        );

        let stdout = child.stdout.take().expect("stdout must be piped");
        let reader = std::io::BufReader::new(stdout);

        // 파이프 버퍼 데드락 방지를 위해 stderr를 별도 스레드에서 읽는다.
        let stderr = child.stderr.take().expect("stderr must be piped");
        let stderr_thread = std::thread::spawn(move || {
            let stderr_reader = std::io::BufReader::new(stderr);
            stderr_reader
                .lines()
                .map_while(Result::ok)
                .collect::<Vec<_>>()
                .join("\n")
        });

        let mut raw_lines: Vec<String> = Vec::new();
        let mut result_value: Option<serde_json::Value> = None;
        // result 직전의 assistant+user 메시지 쌍은 최종 결과와 중복되므로 버퍼링 후 스킵한다.
        // 새 assistant 메시지가 도착할 때만 이전 버퍼를 플러시한다.
        let mut pending_messages: Vec<String> = Vec::new();

        for line_result in reader.lines() {
            let line = line_result.map_err(|err| {
                crate::cli_log!("[스트리밍 쿼리 실패] stdout 읽기 오류: {}", err);
                ClaudeCodeClientError::CommandExecutionFailed {
                    message: format!("stdout 읽기 실패: {}", err),
                }
            })?;

            crate::cli_log!("[스트리밍 쿼리] CLI stdout 라인: {}", &line);
            raw_lines.push(line.clone());

            let json: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(_) => continue,
            };

            let msg_type = json.get("type").and_then(|v| v.as_str()).unwrap_or("");

            match msg_type {
                "assistant" => {
                    for msg in pending_messages.drain(..) {
                        on_stream_message(msg);
                    }
                    if let Some(formatted) = format_stream_message(&json) {
                        pending_messages.push(formatted);
                    }
                }
                "user" => {
                    if let Some(formatted) = format_stream_message(&json) {
                        pending_messages.push(formatted);
                    }
                }
                "result" => {
                    pending_messages.clear();
                    result_value = Some(json);
                }
                _ => {}
            }
        }

        let status = child.wait().map_err(|err| {
            crate::cli_log!("[스트리밍 쿼리 실패] 프로세스 대기 오류: {}", err);
            ClaudeCodeClientError::CommandExecutionFailed {
                message: err.to_string(),
            }
        })?;

        let stderr_content = stderr_thread.join().unwrap_or_default();

        crate::cli_log!("[스트리밍 쿼리 완료] 종료 코드: {}", status);
        if !stderr_content.is_empty() {
            crate::cli_log!("[스트리밍 쿼리] CLI stderr:\n{}", &stderr_content);
        }

        if !status.success() && result_value.is_none() {
            let message = if stderr_content.is_empty() {
                format!("프로세스 종료 코드: {}", status)
            } else {
                stderr_content
            };
            crate::cli_log!("[스트리밍 쿼리 실패] 비정상 종료: {}", &message);
            return Err(ClaudeCodeClientError::CommandExecutionFailed {
                message,
            });
        }

        let command_session_id = new_session_id
            .as_deref()
            .or(self.session_id.as_deref())
            .unwrap_or("unknown");
        let raw_output = raw_lines.join("\n");
        write_debug_log(&sent_system_prompt, &request.user_prompt, command_session_id, raw_output.as_bytes());

        let result_json = result_value.ok_or(ClaudeCodeClientError::NoResultMessage)?;
        let response: CliResponse = serde_json::from_value(result_json)?;

        if response.is_error {
            let error_message = response.result.unwrap_or_default();
            crate::cli_log!(
                "[스트리밍 쿼리 실패] CLI 오류 응답: {}",
                &error_message,
            );
            return Err(ClaudeCodeClientError::CliReturnedError {
                message: error_message,
            });
        }

        let output_value = response
            .structured_output
            .ok_or(ClaudeCodeClientError::MissingStructuredOutput)?;
        let result: T = serde_json::from_value(output_value)?;

        if new_session_id.is_some() {
            self.session_id = Some(response.session_id);
        }

        Ok(result)
    }
}

fn write_debug_log(
    system_prompt: &Option<String>,
    user_prompt: &str,
    session_id: &str,
    cli_stdout: &[u8],
) {
    let path = format!("/tmp/bear-{}.log", session_id);
    let system_prompt_text = system_prompt.as_deref().unwrap_or("");
    let cli_output = String::from_utf8_lossy(cli_stdout);

    let content = format!(
        "<SYSTEM_PROMPT>\n{}\n</SYSTEM_PROMPT>\n\n<USER_PROMPT>\n{}\n</USER_PROMPT>\n\n<CLAUDE_CODE_CLI_OUTPUT>\n{}\n</CLAUDE_CODE_CLI_OUTPUT>\n",
        system_prompt_text,
        user_prompt,
        cli_output,
    );

    // 디버그 로그 기록 실패는 무시한다.
    let _ = std::fs::write(&path, content);
}

const MAX_STREAM_DISPLAY_LINES: usize = 3;

fn format_stream_message(json: &serde_json::Value) -> Option<String> {
    let msg_type = json.get("type")?.as_str()?;
    let formatted = match msg_type {
        "assistant" => format_assistant_message(json),
        "user" => format_user_message(json),
        _ => None,
    };
    formatted.map(|text| truncate_to_max_lines(&text))
}

fn truncate_to_max_lines(text: &str) -> String {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= MAX_STREAM_DISPLAY_LINES {
        return text.to_string();
    }
    let visible: String = lines[..MAX_STREAM_DISPLAY_LINES].join("\n");
    let omitted = lines.len() - MAX_STREAM_DISPLAY_LINES;
    format!("{}\n... (+{} lines)", visible, omitted)
}

fn format_assistant_message(json: &serde_json::Value) -> Option<String> {
    let content = json.get("message")?.get("content")?.as_array()?;
    let mut parts: Vec<String> = Vec::new();

    for block in content {
        let block_type = block.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match block_type {
            "text" => {
                if let Some(text) = block.get("text").and_then(|v| v.as_str()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        parts.push(trimmed.to_string());
                    }
                }
            }
            "tool_use" => {
                let name = block.get("name").and_then(|v| v.as_str()).unwrap_or("unknown");
                let input = block.get("input").cloned().unwrap_or(serde_json::Value::Null);
                parts.push(format!("[Tool Call: {}]\n{}", name, input));
            }
            _ => {}
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

fn format_user_message(json: &serde_json::Value) -> Option<String> {
    let content = json.get("message")?.get("content")?.as_array()?;
    let mut parts: Vec<String> = Vec::new();

    for item in content {
        let item_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match item_type {
            "tool_result" => {
                if let Some(text) = item.get("content").and_then(|v| v.as_str())
                    && !text.is_empty()
                {
                    parts.push(format!("[Tool Result]\n{}", text));
                }
            }
            "text" => {
                if let Some(text) = item.get("text").and_then(|v| v.as_str()) {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        parts.push(trimmed.to_string());
                    }
                }
            }
            _ => {}
        }
    }

    if parts.is_empty() {
        None
    } else {
        Some(parts.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;

    #[derive(Debug, Deserialize, PartialEq)]
    struct TestOutput {
        answer: String,
    }

    fn make_result_message(
        session_id: &str,
        is_error: bool,
        result_text: Option<&str>,
        structured_output: Option<serde_json::Value>,
    ) -> serde_json::Value {
        let mut msg = serde_json::json!({
            "type": "result",
            "session_id": session_id,
            "is_error": is_error,
        });
        if let Some(text) = result_text {
            msg["result"] = serde_json::Value::String(text.to_string());
        }
        if let Some(output) = structured_output {
            msg["structured_output"] = output;
        }
        msg
    }

    fn make_json_array_output(messages: &[serde_json::Value]) -> Vec<u8> {
        serde_json::to_vec(messages).unwrap()
    }

    #[test]
    fn parse_json_array_with_result_message() {
        let messages = vec![
            serde_json::json!({"type": "system", "subtype": "init", "session_id": "sess-1"}),
            serde_json::json!({"type": "assistant", "message": {"role": "assistant"}}),
            make_result_message(
                "sess-1",
                false,
                Some("hello"),
                Some(serde_json::json!({"answer": "hello"})),
            ),
        ];
        let stdout = make_json_array_output(&messages);

        let parsed: ParsedOutput<TestOutput> = parse_cli_output(&stdout).unwrap();

        assert_eq!(parsed.result, TestOutput { answer: "hello".to_string() });
        assert_eq!(parsed.session_id, "sess-1");
    }

    #[test]
    fn parse_single_result_object() {
        let message = make_result_message(
            "sess-2",
            false,
            Some("world"),
            Some(serde_json::json!({"answer": "world"})),
        );
        let stdout = serde_json::to_vec(&message).unwrap();

        let parsed: ParsedOutput<TestOutput> = parse_cli_output(&stdout).unwrap();

        assert_eq!(parsed.result, TestOutput { answer: "world".to_string() });
        assert_eq!(parsed.session_id, "sess-2");
    }

    #[test]
    fn error_when_no_result_message() {
        let messages = vec![
            serde_json::json!({"type": "system", "subtype": "init"}),
            serde_json::json!({"type": "assistant", "message": {}}),
        ];
        let stdout = make_json_array_output(&messages);

        let err = parse_cli_output::<TestOutput>(&stdout).unwrap_err();

        assert!(
            matches!(err, ClaudeCodeClientError::NoResultMessage),
            "expected NoResultMessage, got: {err}",
        );
    }

    #[test]
    fn error_when_both_structured_output_and_result_text_missing() {
        let messages = vec![
            make_result_message("sess-3", false, None, None),
        ];
        let stdout = make_json_array_output(&messages);

        let err = parse_cli_output::<TestOutput>(&stdout).unwrap_err();

        assert!(
            matches!(err, ClaudeCodeClientError::MissingStructuredOutput),
            "expected MissingStructuredOutput, got: {err}",
        );
    }

    #[test]
    fn error_when_result_text_is_not_valid_json() {
        let messages = vec![
            make_result_message("sess-3", false, Some("plain text, not json"), None),
        ];
        let stdout = make_json_array_output(&messages);

        let err = parse_cli_output::<TestOutput>(&stdout).unwrap_err();

        assert!(
            matches!(err, ClaudeCodeClientError::MissingStructuredOutput),
            "expected MissingStructuredOutput, got: {err}",
        );
    }

    #[test]
    fn error_when_cli_returned_error() {
        let messages = vec![
            make_result_message("sess-4", true, Some("something went wrong"), None),
        ];
        let stdout = make_json_array_output(&messages);

        let err = parse_cli_output::<TestOutput>(&stdout).unwrap_err();

        match err {
            ClaudeCodeClientError::CliReturnedError { message } => {
                assert_eq!(message, "something went wrong");
            }
            other => panic!("expected CliReturnedError, got: {other}"),
        }
    }

    #[test]
    fn error_when_invalid_json() {
        let stdout = b"this is not json";

        let err = parse_cli_output::<TestOutput>(stdout).unwrap_err();

        assert!(
            matches!(err, ClaudeCodeClientError::JsonParsingFailed { .. }),
            "expected JsonParsingFailed, got: {err}",
        );
    }

    #[test]
    fn format_assistant_text_message() {
        let json = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "프로젝트를 분석하겠습니다."}]
            }
        });

        let result = format_stream_message(&json).unwrap();

        assert_eq!(result, "프로젝트를 분석하겠습니다.");
    }

    #[test]
    fn format_assistant_tool_use_message() {
        let json = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{
                    "type": "tool_use",
                    "name": "Bash",
                    "input": {"command": "ls /workspace", "description": "List files"}
                }]
            }
        });

        let result = format_stream_message(&json).unwrap();

        let lines: Vec<&str> = result.lines().collect();
        assert_eq!(lines[0], "[Tool Call: Bash]");
        assert!(lines[1].contains("ls /workspace"));
    }

    #[test]
    fn format_user_tool_result_message() {
        let json = serde_json::json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_123",
                    "content": "Cargo.toml\nsrc",
                    "is_error": false
                }]
            }
        });

        let result = format_stream_message(&json).unwrap();

        assert_eq!(result, "[Tool Result]\nCargo.toml\nsrc");
    }

    #[test]
    fn format_stream_ignores_system_type() {
        let json = serde_json::json!({"type": "system", "subtype": "init"});

        assert!(format_stream_message(&json).is_none());
    }

    #[test]
    fn format_stream_ignores_empty_text() {
        let json = serde_json::json!({
            "type": "assistant",
            "message": {"content": [{"type": "text", "text": "  \n  "}]}
        });

        assert!(format_stream_message(&json).is_none());
    }

    #[test]
    fn format_user_text_message() {
        let json = serde_json::json!({
            "type": "user",
            "message": {
                "content": [{"type": "text", "text": "Explore the project."}]
            }
        });

        let result = format_stream_message(&json).unwrap();

        assert_eq!(result, "Explore the project.");
    }

    #[test]
    fn truncate_long_tool_result() {
        let json = serde_json::json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_123",
                    "content": "line1\nline2\nline3\nline4\nline5",
                    "is_error": false
                }]
            }
        });

        let result = format_stream_message(&json).unwrap();
        let lines: Vec<&str> = result.lines().collect();

        assert_eq!(lines[0], "[Tool Result]");
        assert_eq!(lines[1], "line1");
        assert_eq!(lines[2], "line2");
        assert_eq!(lines[3], "... (+3 lines)");
    }

    #[test]
    fn no_truncation_within_limit() {
        let json = serde_json::json!({
            "type": "assistant",
            "message": {
                "content": [{"type": "text", "text": "line1\nline2\nline3"}]
            }
        });

        let result = format_stream_message(&json).unwrap();

        assert_eq!(result, "line1\nline2\nline3");
    }

    #[test]
    fn format_empty_tool_result_is_skipped() {
        let json = serde_json::json!({
            "type": "user",
            "message": {
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "toolu_123",
                    "content": "",
                    "is_error": false
                }]
            }
        });

        assert!(format_stream_message(&json).is_none());
    }
}
