use std::fs::{self, OpenOptions};
use std::io::{self, Write};
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
}

pub fn spec_writing_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "response_type": {
                "type": "string",
                "enum": ["spec_draft", "clarifying_questions"]
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

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text.

Original user request (verbatim):
<<<
{{ORIGINAL_REQUEST_TEXT}}
>>>

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
- Read the spec journal file at the path below to understand the full context of prior requirements, Q&A, and previous spec drafts.

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text.

Spec journal file path:
<<<
{{JOURNAL_PATH}}
>>>

User feedback:
<<<
{{USER_FEEDBACK}}
>>>"#;

pub fn build_initial_spec_prompt(original_request: &str, qa_log: &[QaRound]) -> String {
    let qa_log_text = format_qa_log(qa_log);

    INITIAL_SPEC_PROMPT_TEMPLATE
        .replace("{{ORIGINAL_REQUEST_TEXT}}", original_request)
        .replace("{{QA_LOG_TEXT}}", &qa_log_text)
}

pub fn build_revision_prompt(user_feedback: &str, journal_path: &Path) -> String {
    REVISION_PROMPT_TEMPLATE
        .replace("{{JOURNAL_PATH}}", &journal_path.display().to_string())
        .replace("{{USER_FEEDBACK}}", user_feedback)
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

pub struct SpecJournal {
    file_path: PathBuf,
}

impl SpecJournal {
    pub fn new(workspace: &Path, date_dir: &str, session_name: &str) -> io::Result<Self> {
        let dir = workspace.join(".bear").join(date_dir).join(session_name);
        fs::create_dir_all(&dir)?;

        let file_path = dir.join("spec.journal.md");

        Ok(Self { file_path })
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn append_user_request(&self, text: &str) -> io::Result<()> {
        self.append_tagged("USER_REQUEST", text)
    }

    pub fn append_clarifying_questions(&self, questions: &[String]) -> io::Result<()> {
        let content = questions
            .iter()
            .enumerate()
            .map(|(i, q)| format!("{}. {}", i + 1, q))
            .collect::<Vec<_>>()
            .join("\n");
        self.append_tagged("CLARIFYING_QUESTIONS", &content)
    }

    pub fn append_user_answers(&self, answer: &str) -> io::Result<()> {
        self.append_tagged("USER_ANSWERS", answer)
    }

    pub fn append_qa_log(&self, qa_log: &[QaRound]) -> io::Result<()> {
        if qa_log.is_empty() {
            return Ok(());
        }
        let content = format_qa_log(qa_log);
        self.append_tagged("QA_LOG", &content)
    }

    pub fn append_spec_draft(&self, draft: &str) -> io::Result<()> {
        self.append_tagged("SPEC_DRAFT", draft)
    }

    pub fn append_user_feedback(&self, feedback: &str) -> io::Result<()> {
        self.append_tagged("USER_FEEDBACK", feedback)
    }

    pub fn append_approved_spec(&self, spec: &str) -> io::Result<()> {
        self.append_tagged("APPROVED_SPEC", spec)
    }

    fn append_tagged(&self, tag: &str, content: &str) -> io::Result<()> {
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file_path)?;

        writeln!(file, "<<<BEGIN")?;
        writeln!(file, "<{}>", tag)?;
        writeln!(file, "{}", content)?;
        writeln!(file, "</{}>", tag)?;
        writeln!(file, ">>>END")?;

        Ok(())
    }
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
    fn build_initial_prompt_contains_all_parts() {
        let qa_log = vec![QaRound {
            questions: vec!["What scope?".to_string()],
            answer: "Full scope".to_string(),
        }];

        let prompt = build_initial_spec_prompt("Build a CLI tool", &qa_log);

        assert!(prompt.contains("Build a CLI tool"));
        assert!(prompt.contains("What scope?"));
        assert!(prompt.contains("Full scope"));
    }

    #[test]
    fn build_revision_prompt_contains_feedback_and_journal_path() {
        let journal_path = Path::new("/workspace/.bear/20250101/sess-1/spec.journal.md");
        let prompt = build_revision_prompt("Please add error handling section", journal_path);

        assert!(prompt.contains("Please add error handling section"));
        assert!(prompt.contains("/workspace/.bear/20250101/sess-1/spec.journal.md"));
    }

    #[test]
    fn journal_creates_directory_and_file() {
        let temp_dir = TempDir::new().unwrap();
        let journal = SpecJournal::new(temp_dir.path(), "20250101", "test-session-123").unwrap();

        journal.append_user_request("Build something").unwrap();

        let content = fs::read_to_string(journal.file_path()).unwrap();
        assert!(content.contains("<USER_REQUEST>"));
        assert!(content.contains("Build something"));
        assert!(content.contains("</USER_REQUEST>"));
    }

    #[test]
    fn journal_appends_multiple_entries() {
        let temp_dir = TempDir::new().unwrap();
        let journal = SpecJournal::new(temp_dir.path(), "20250101", "test-session").unwrap();

        journal.append_user_request("Build a tool").unwrap();
        journal
            .append_clarifying_questions(&[
                "What language?".to_string(),
                "What platform?".to_string(),
            ])
            .unwrap();
        journal.append_user_answers("Rust, Linux").unwrap();
        journal.append_spec_draft("# Draft Spec").unwrap();
        journal.append_user_feedback("Add more details").unwrap();
        journal.append_approved_spec("# Final Spec").unwrap();

        let content = fs::read_to_string(journal.file_path()).unwrap();
        assert!(content.contains("<USER_REQUEST>"));
        assert!(content.contains("<CLARIFYING_QUESTIONS>"));
        assert!(content.contains("1. What language?"));
        assert!(content.contains("2. What platform?"));
        assert!(content.contains("<USER_ANSWERS>"));
        assert!(content.contains("<SPEC_DRAFT>"));
        assert!(content.contains("<USER_FEEDBACK>"));
        assert!(content.contains("<APPROVED_SPEC>"));
    }

    #[test]
    fn journal_file_path_structure() {
        let temp_dir = TempDir::new().unwrap();
        let journal = SpecJournal::new(temp_dir.path(), "20250101", "abc-123").unwrap();

        let expected = temp_dir
            .path()
            .join(".bear")
            .join("20250101")
            .join("abc-123")
            .join("spec.journal.md");
        assert_eq!(journal.file_path(), expected);
    }
}
