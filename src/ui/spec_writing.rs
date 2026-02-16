use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use super::clarification::QaRound;

#[derive(Debug, Deserialize)]
pub struct SpecWritingResponse {
    pub response_type: SpecResponseType,
    pub spec_draft: Option<String>,
    pub clarifying_questions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum SpecResponseType {
    SpecDraft,
    ClarifyingQuestions,
    Approved,
}

pub fn spec_writing_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "response_type": {
                "type": "string",
                "enum": ["spec_draft", "clarifying_questions", "approved"]
            },
            "spec_draft": {
                "type": "string"
            },
            "clarifying_questions": {
                "type": "array",
                "items": { "type": "string", "minLength": 5 },
                "minItems": 1,
                "maxItems": 5
            }
        },
        "required": ["response_type"],
        "additionalProperties": false
    })
}

const INITIAL_SPEC_PROMPT_TEMPLATE: &str = r#"Based on the collected requirements and Q&A below, write a software specification document.

If you have enough information, set response_type to "spec_draft" and produce the spec in Markdown format in the spec_draft field.
If you need more clarification, set response_type to "clarifying_questions" and provide 1-5 questions in the clarifying_questions field.

The spec MUST follow this structure:
1. Overview - Brief summary of what is being built
2. Goals and Non-Goals - What is in scope and explicitly out of scope
3. Functional Requirements - Detailed behavioral requirements
4. Non-Functional Requirements - Performance, security, reliability constraints
5. Acceptance Criteria - Testable criteria for completion
6. Open Questions - Any remaining uncertainties

IMPORTANT:
- The spec describes WHAT the system must do, not HOW it is implemented internally.
- The spec MUST be testable with clear acceptance criteria.
- Inspect the workspace using available tools to understand existing code context.
- Write the spec in Korean.
- The current working directory (CWD) of this process is the workspace root. When referencing file paths in the spec, use paths relative to the workspace root. Do NOT embed absolute paths derived from the current environment. The coding agent that implements this spec will have its own CWD set as the workspace root, so relative paths will resolve correctly.

---

DECISION ESCALATION (mandatory — read this BEFORE writing any draft):

The spec describes WHAT the system must do, not HOW it is implemented. Therefore, decision escalation in this phase is limited to user-facing behavior and external contracts. Do NOT ask about internal implementation details (library choices, architecture patterns, database engines, threading models, etc.) — those decisions belong to the planning phase.

You MUST NOT make decisions on behalf of the user for the following spec-level topics. If the user's requirements or the Q&A log do not explicitly address a decision listed below, you MUST set response_type to "clarifying_questions" and ask the user to decide — even if you believe you have enough information to write the spec otherwise.

Decisions that REQUIRE user approval (never decide on your own):
- External interface contract: what kind of interface the system exposes to users or consumers (e.g., CLI vs web UI vs desktop app, REST API vs GraphQL endpoint), since this fundamentally shapes what the spec describes.
- UI/UX behavior: screen layout, navigation flow, interaction patterns, or visual design direction — these define what the user experiences and must be specified, not assumed.
- User-facing authentication / authorization flow: how users authenticate from their perspective (e.g., login with password vs SSO vs API key), since this is observable behavior.
- Breaking changes to existing public interfaces, APIs, or contracts that external consumers depend on.
- Significant trade-offs that affect observable system behavior: consistency vs availability, real-time vs batch processing, online vs offline capability, etc.
- Scope-affecting platform or environment constraints: whether the system must support specific platforms (e.g., Linux-only vs cross-platform), since this shapes the spec's non-functional requirements.

Decisions you MAY make on your own (no need to ask):
- Document structure and formatting of the spec itself.
- Wording and categorization of requirements (functional vs non-functional).
- Obvious choices that are already constrained by the existing codebase or prior user statements.

Decisions you MUST NOT ask about (these belong to the planning phase, not the spec):
- Specific library or framework selection (e.g., which TUI library, which HTTP client).
- Internal architecture patterns (e.g., monolith vs microservice, event-driven vs request-response).
- Data storage engine or persistence mechanism (e.g., PostgreSQL vs SQLite, file format).
- Concurrency / threading model.
- Deployment strategy or runtime environment details.
- Specific external dependencies to add.

How to ask the user for a decision:
When you identify a spec-level decision that requires user approval, include it in your clarifying_questions. Each question MUST:
1. Clearly state what needs to be decided and why it matters for the spec.
2. Present 2–4 concrete options, each with a brief description of pros and cons.
3. Include your recommendation and its rationale, so the user can make an informed choice quickly.
4. Be written so the user can answer with a short phrase (e.g., "Option B" or "웹 UI").

Example of a well-formed decision question:
"시스템의 사용자 인터페이스 유형을 결정해야 합니다. 선택지: (A) TUI (터미널 UI) — 터미널 환경에서 직접 사용 가능, 가볍고 빠름, 그래픽 요소 제한적. (B) 웹 UI — 브라우저 기반으로 접근성 높음, 풍부한 시각 요소, 서버 구성 필요. (C) CLI — 가장 단순하고 스크립팅에 유리, 인터랙티브 경험은 제한적. 추천: (A) TUI — 개발 도구 특성상 터미널 환경이 자연스럽고, 서버 없이 로컬에서 바로 실행 가능합니다. 어떤 것을 사용할까요?"

Handling user follow-up questions on decisions:
The user may not immediately decide. Instead, they may ask counter-questions to learn more before deciding (e.g., "웹 UI랑 TUI의 차이가 정확히 뭐야?"). When this happens:
- Answer the user's question thoroughly with enough detail for an informed decision.
- Re-present the decision options (updated if the user's question reveals new considerations) with pros/cons and your recommendation.
- Continue this cycle until the user makes a clear decision. Multiple rounds of follow-up questions are expected and must be supported.

---

Output MUST be valid JSON conforming to the provided JSON Schema.

You MUST read the original user request from the following file before proceeding:
- {{USER_REQUEST_PATH}}

Clarification Q&A log:
<<<
{{QA_LOG_TEXT}}
>>>"#;

const REVISION_PROMPT_TEMPLATE: &str = r#"The user has provided feedback on the spec draft. Please revise the spec accordingly.

If you can produce a revised spec, set response_type to "spec_draft" and provide the updated spec in the spec_draft field.
If you need more clarification before revising, set response_type to "clarifying_questions" and provide 1-5 questions in the clarifying_questions field.

IMPORTANT:
- The spec describes WHAT the system must do, not HOW it is implemented internally.
- The spec MUST be testable with clear acceptance criteria.
- Write the spec in Korean.
- The session conversation history contains all prior requirements, Q&A, and previous spec drafts. Use this context to revise the spec.
- DECISION ESCALATION: The same decision-escalation rules from the initial spec phase still apply. If the user's feedback introduces or reveals new undecided spec-level topics that require user approval (external interface contract, UI/UX behavior, user-facing auth flow, breaking changes to public contracts, observable behavior trade-offs, platform constraints), you MUST set response_type to "clarifying_questions" and ask the user to decide before revising the spec. When asking, present options with pros/cons and your recommendation. Do NOT silently incorporate your own choices into the revised spec. Remember: do NOT ask about implementation details (library choices, architecture patterns, storage engines, etc.) — those belong to the planning phase.
- USER RESPONSE CLASSIFICATION: When the previous conversation shows that the most recent model output was a set of clarifying questions (especially decision-escalation questions), you MUST classify the user's current message into one of three categories before taking any other action:

  (1) DECISION / ANSWER: The user clearly provides a decision or directly answers the pending question(s).
  → Incorporate the decision and proceed normally — write or revise the spec.

  (2) COUNTER-QUESTION: The user is asking for more information, explanation, or clarification before deciding (e.g., "웹 UI랑 TUI의 차이가 정확히 뭐야?", "SSO를 적용하면 어떤 제약이 생겨?", "각 옵션의 유지보수 비용 차이는?").
  → Set response_type to "clarifying_questions". Provide a thorough, informative answer to the user's question with enough context for them to make an informed decision. Then re-present the original decision options (updated if the user's question reveals new considerations) with pros/cons and your recommendation. The user must still make the final decision.

  (3) UNCLEAR INTENT: The user's response does not clearly fit category (1) or (2) — you cannot determine whether they made a decision, asked a question, or want something else.
  → Set response_type to "clarifying_questions". Politely acknowledge the user's message, briefly restate the pending decision, and ask them to either: (a) choose one of the presented options, (b) ask any questions they have about the options, or (c) explain what they would like to do.

  IMPORTANT: This classification applies every time the user responds after a clarifying-question round. The user may go through multiple rounds of counter-questions before making a final decision. You MUST support this without losing track of any pending decision(s). Always check the session conversation history for the full history of questions and answers.
- APPROVAL DETECTION: Before attempting any revision, first evaluate whether the user's feedback message is expressing approval or acceptance of the current draft rather than requesting changes. Examples of approval expressions include (but are not limited to): "승인합니다", "좋습니다", "진행해주세요", "괜찮습니다", "이대로 해주세요", "OK", "LGTM", "approve", "looks good". If the user's message UNAMBIGUOUSLY expresses approval with NO revision requests whatsoever, set response_type to "approved" and leave all other fields empty. If the message contains ANY specific change request, suggestion, or criticism — even if it also contains positive language (e.g., "좋은데 한 가지만 수정해주세요") — treat it as feedback and revise normally. When in doubt, treat the message as feedback requiring revision, NOT as approval.

Output MUST be valid JSON conforming to the provided JSON Schema.

User feedback:
<<<
{{USER_FEEDBACK}}
>>>"#;

pub fn build_initial_spec_prompt(user_request_path: &Path, qa_log: &[QaRound]) -> String {
    let qa_log_text = format_qa_log(qa_log);

    INITIAL_SPEC_PROMPT_TEMPLATE
        .replace("{{USER_REQUEST_PATH}}", &user_request_path.display().to_string())
        .replace("{{QA_LOG_TEXT}}", &qa_log_text)
}

pub fn build_revision_prompt(user_feedback: &str) -> String {
    REVISION_PROMPT_TEMPLATE.replace("{{USER_FEEDBACK}}", user_feedback)
}

pub fn build_followup_revision_prompt(user_feedback: &str) -> String {
    format!(
        "User feedback:\n<<<\n{}\n>>>",
        user_feedback,
    )
}

fn format_qa_log(qa_log: &[QaRound]) -> String {
    if qa_log.is_empty() {
        return String::new();
    }

    let mut result = String::new();
    for round in qa_log {
        result.push_str("Assistant's questions:\n");
        for (i, question) in round.questions.iter().enumerate() {
            result.push_str(&format!("{}. {}\n", i + 1, question));
        }
        result.push_str(&format!("\nUser's answer:\n{}\n\n", round.answer));
    }
    result
}

pub fn save_user_request(dir: &Path, user_request: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;

    let file_path = dir.join("user-request.md");
    fs::write(&file_path, user_request)?;

    Ok(file_path)
}

pub fn save_approved_spec(dir: &Path, spec_text: &str) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;

    let file_path = dir.join("spec.md");
    fs::write(&file_path, spec_text)?;

    Ok(file_path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn spec_writing_schema_is_valid_json() {
        let schema = spec_writing_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["response_type"].is_object());
        assert!(schema["properties"]["spec_draft"].is_object());
        assert!(schema["properties"]["clarifying_questions"].is_object());
    }

    #[test]
    fn deserialize_spec_draft_response() {
        let json = serde_json::json!({
            "response_type": "spec_draft",
            "spec_draft": "# Spec\n\nSome content"
        });

        let response: SpecWritingResponse = serde_json::from_value(json).unwrap();

        assert_eq!(response.response_type, SpecResponseType::SpecDraft);
        assert_eq!(response.spec_draft.unwrap(), "# Spec\n\nSome content");
        assert!(response.clarifying_questions.is_none());
    }

    #[test]
    fn deserialize_clarifying_questions_response() {
        let json = serde_json::json!({
            "response_type": "clarifying_questions",
            "clarifying_questions": ["Question 1?", "Question 2?"]
        });

        let response: SpecWritingResponse = serde_json::from_value(json).unwrap();

        assert_eq!(response.response_type, SpecResponseType::ClarifyingQuestions);
        assert!(response.spec_draft.is_none());
        assert_eq!(response.clarifying_questions.unwrap().len(), 2);
    }

    #[test]
    fn deserialize_approved_response() {
        let json = serde_json::json!({
            "response_type": "approved"
        });

        let response: SpecWritingResponse = serde_json::from_value(json).unwrap();

        assert_eq!(response.response_type, SpecResponseType::Approved);
        assert!(response.spec_draft.is_none());
        assert!(response.clarifying_questions.is_none());
    }

    #[test]
    fn spec_writing_schema_includes_approved() {
        let schema = spec_writing_schema();
        let enum_values = schema["properties"]["response_type"]["enum"]
            .as_array()
            .unwrap();
        assert!(enum_values.iter().any(|v| v == "approved"));
    }

    #[test]
    fn revision_prompt_contains_approval_detection_instruction() {
        let prompt = build_revision_prompt("some feedback");

        assert!(prompt.contains("APPROVAL DETECTION"));
    }

    #[test]
    fn build_initial_prompt_contains_all_parts() {
        let qa_log = vec![QaRound {
            questions: vec!["What scope?".to_string()],
            answer: "Full scope".to_string(),
        }];

        let user_request_path = Path::new("/workspace/.bear/20250101/session/user-request.md");
        let prompt = build_initial_spec_prompt(user_request_path, &qa_log);

        assert!(prompt.contains("/workspace/.bear/20250101/session/user-request.md"));
        assert!(prompt.contains("What scope?"));
        assert!(prompt.contains("Full scope"));
    }

    #[test]
    fn build_revision_prompt_contains_feedback() {
        let prompt = build_revision_prompt("Please add error handling section");

        assert!(prompt.contains("Please add error handling section"));
    }

    #[test]
    fn save_approved_spec_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let spec_text = "# Final Spec\n\nThis is the approved spec.";

        let path = save_approved_spec(temp_dir.path(), spec_text).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, spec_text);
    }

    #[test]
    fn save_approved_spec_file_path_structure() {
        let temp_dir = TempDir::new().unwrap();

        let path = save_approved_spec(temp_dir.path(), "spec content").unwrap();

        let expected = temp_dir.path().join("spec.md");
        assert_eq!(path, expected);
    }

    #[test]
    fn save_user_request_creates_file() {
        let temp_dir = TempDir::new().unwrap();
        let user_request = "CLI 도구를 만들어 주세요.";

        let path = save_user_request(temp_dir.path(), user_request).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, user_request);
    }

    #[test]
    fn save_user_request_file_path_structure() {
        let temp_dir = TempDir::new().unwrap();

        let path = save_user_request(temp_dir.path(), "request").unwrap();

        let expected = temp_dir.path().join("user-request.md");
        assert_eq!(path, expected);
    }

    #[test]
    fn build_followup_revision_prompt_contains_only_feedback() {
        let prompt = build_followup_revision_prompt("에러 처리 섹션을 추가해주세요");

        assert!(prompt.contains("에러 처리 섹션을 추가해주세요"));
        assert!(!prompt.contains("APPROVAL DETECTION"));
        assert!(!prompt.contains("DECISION ESCALATION"));
    }
}
