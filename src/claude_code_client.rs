mod binary_finder;
mod error;
mod response;

pub use error::ClaudeCodeClientError;
pub use response::CliResponse;

use std::path::PathBuf;
use std::process::Command;

use serde::de::DeserializeOwned;

pub struct ClaudeCodeRequest {
    pub system_prompt: Option<String>,
    pub user_prompt: String,
    pub model: Option<String>,
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
}

impl ClaudeCodeClient {
    pub fn new(
        api_key: String,
        additional_work_directories: Vec<PathBuf>,
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
        })
    }

    pub fn query<T: DeserializeOwned>(
        &mut self,
        request: &ClaudeCodeRequest,
    ) -> Result<T, ClaudeCodeClientError> {
        let model_effort_level = "high";
        let disable_auto_memory = "0";  // 0 = force enable.
        let disable_feedback_survey = "1";

        let mut command = Command::new(&self.binary_path);
        command
            .env("ANTHROPIC_API_KEY", &self.api_key)
            .env("CLAUDE_CODE_EFFORT_LEVEL", model_effort_level)
            .env("CLAUDE_CODE_DISABLE_AUTO_MEMORY", disable_auto_memory)
            .env("CLAUDE_CODE_DISABLE_FEEDBACK_SURVEY", disable_feedback_survey)
            .arg("-p")
            .arg("--output-format").arg("json")
            .arg("--allow-dangerously-skip-permissions")
            .arg("--permission-mode").arg("bypassPermissions")
            .arg("--tools").arg("AskUserQuestion,Bash,TaskOutput,Edit,ExitPlanMode,Glob,Grep,KillShell,MCPSearch,Read,Skill,Task,TaskCreate,TaskGet,TaskList,TaskUpdate,WebFetch,WebSearch,Write,LSP");

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

        if let Some(model) = &request.model {
            command.arg("--model").arg(model);
        }

        if let Some(system_prompt) = &request.system_prompt {
            command.arg("--append-system-prompt").arg(system_prompt);
        }

        let output_schema_string = request.output_schema.to_string();
        command.arg("--json-schema").arg(&output_schema_string);
        command.arg(&request.user_prompt);

        let output = command.output().map_err(|err| {
            ClaudeCodeClientError::CommandExecutionFailed {
                message: err.to_string(),
            }
        })?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            return Err(ClaudeCodeClientError::CommandExecutionFailed {
                message: stderr.to_string(),
            });
        }

        let command_session_id = new_session_id
            .as_deref()
            .or(self.session_id.as_deref())
            .unwrap_or("unknown");
        write_debug_log(request, command_session_id, &output.stdout);

        let parsed: ParsedOutput<T> = parse_cli_output(&output.stdout)?;

        if new_session_id.is_some() {
            self.session_id = Some(parsed.session_id);
        }

        Ok(parsed.result)
    }
}

fn write_debug_log(request: &ClaudeCodeRequest, session_id: &str, cli_stdout: &[u8]) {
    let path = format!("/tmp/bear-{}.log", session_id);
    let system_prompt = request.system_prompt.as_deref().unwrap_or("");
    let cli_output = String::from_utf8_lossy(cli_stdout);

    let content = format!(
        "<SYSTEM_PROMPT>\n{}\n</SYSTEM_PROMPT>\n\n<USER_PROMPT>\n{}\n</USER_PROMPT>\n\n<CLAUDE_CODE_CLI_OUTPUT>\n{}\n</CLAUDE_CODE_CLI_OUTPUT>\n",
        system_prompt,
        request.user_prompt,
        cli_output,
    );

    // 디버그 로그 기록 실패는 무시한다.
    let _ = std::fs::write(&path, content);
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
    fn fallback_to_result_text_when_structured_output_missing() {
        let messages = vec![
            make_result_message(
                "sess-3",
                false,
                Some(r#"{"answer": "from result text"}"#),
                None,
            ),
        ];
        let stdout = make_json_array_output(&messages);

        let parsed: ParsedOutput<TestOutput> = parse_cli_output(&stdout).unwrap();

        assert_eq!(parsed.result, TestOutput { answer: "from result text".to_string() });
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
}
