use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct ClarificationQuestions {
    pub questions: Vec<String>,
}

#[derive(Clone)]
pub struct QaRound {
    pub questions: Vec<String>,
    pub answer: String,
}

pub fn system_prompt() -> &'static str {
    r#"# Terminology

In this document, the term **"spec"** is used as shorthand for **"specification"**. Unless explicitly qualified, "spec" refers to a software specification (not a "code", "standard", or other non-software usage of the term).

---

# Role

You are the **specification-driven development** assistant whose sole responsibility is to produce a high-quality software specification, which satisfies the user request, and iteratively refine written specifications based on the user's feedback.

You MUST NOT perform implementation work of any kind. This includes (but is not limited to) writing or modifying source code, running commands, executing tests, changing files, generating patches, making configuration edits, creating pull requests, or taking any action that directly completes the requested task.

For every user request, you MUST respond only with specification content: clarify requirements, define scope and non-scope, document assumptions and constraints, specify interfaces and acceptance criteria, and capture open questions. If the user asks you to "just do it", you MUST convert the request into a spec and ask for any missing information instead of executing the work.

---

# High-level Philosophy (non-negotiable)

- The spec MUST describe WHAT the system/module MUST do (externally observable behavior and contracts), not HOW it is implemented internally.
- However, any strict interface SHOULD be a contract and MAY be specified concretely (function signatures, types, error model), if it has consumers (e.g., external clients or internal modules) that are hard to change without breaking compatibility.
- The spec MUST be testable: it MUST include acceptance criteria that can be validated via automated tests or reproducible manual steps.
- The spec MUST explicitly call out assumptions, non-goals, and open questions.

---

# What MUST NOT be in the Spec (unless explicitly requested by the user)

- Concrete internal implementation mechanisms such as:
  - Threading model choices (mutex/strand, executor placement, etc.)
  - Specific database schema/table names unless the schema itself is a published contract for other consumers
  - Specific file paths or class layouts
  - Specific library choices unless mandated by product constraints
- Detailed task breakdown, file-by-file change plan, or sequencing steps (these belong to the Planner/Implementer phases)

If the user asks for these, capture them in:
- `Open questions` (if undecided), or
- `Non-functional constraints` (only if it is a hard constraint)

---

# Datetime Handling Rule (mandatory)

You **MUST** get the current datetime using Python scripts from the local system in `Asia/Seoul` timezone, not from your LLM model, whenever you need current datetime or timestamp."#
}

const USER_PROMPT_TEMPLATE: &str = r#"Given the original user request AND the full clarification Q&A log so far, generate clarification questions that reduce ambiguity and de-risk implementation.

If, in your judgment, there are no remaining clarification questions that are necessary to begin writing the spec, return an empty array for "questions".

Coverage requirements (only when questions are non-empty):
When you ask questions, they MUST collectively cover the following areas across the set of questions (each area should be covered by at least one question):
1) Scope boundaries: what is explicitly in-scope vs out-of-scope.
2) Primary consumer: who will use the deliverable and in what environment/workflow.
3) Success definition: what "done" means and how success will be validated.
4) Key edge cases: the most important corner cases or tricky scenarios.
5) Error expectations: expected failure modes, error handling, and user-visible behavior.

Constraints:
- Output MUST be valid JSON that conforms to the provided JSON Schema.
- Output MUST contain ONLY the JSON object, with no extra text.
- Provide 0–5 questions total.
- Each question should be precise, answerable, and non-overlapping.
- Inspect the current workspace using the available tools. Read the files required to understand the context and to avoid asking questions that are already answered by existing files.
- Do NOT ask questions that you can infer from the workspace files.
- Do NOT ask questions that are purely preference/subjective unless they materially impact scope or correctness.

Original user request (verbatim):
<<<
{{ORIGINAL_REQUEST_TEXT}}
>>>

Clarification Q&A log so far (may be empty). Each entry is the assistant's question followed by the user's answer:
<<<
{{QA_LOG_TEXT}}
>>>

Your output MUST conform to the given JSON Schema.
"#;

pub fn build_user_prompt(original_request: &str, qa_log: &[QaRound]) -> String {
    let qa_log_text = if qa_log.is_empty() {
        String::new()
    } else {
        format_qa_log(qa_log)
    };

    USER_PROMPT_TEMPLATE
        .replace("{{ORIGINAL_REQUEST_TEXT}}", original_request)
        .replace("{{QA_LOG_TEXT}}", &qa_log_text)
}

/// CLI에 전달할 JSON Schema. 프롬프트 내 스키마(minItems: 3)와 달리
/// minItems: 0으로 설정하여 "질문 없음"(빈 배열) 응답을 허용한다.
pub fn clarification_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "questions": {
                "type": "array",
                "minItems": 0,
                "maxItems": 5,
                "items": {
                    "type": "string",
                    "minLength": 5
                }
            }
        },
        "required": ["questions"],  
        "additionalProperties": false
    })
}

fn format_qa_log(qa_log: &[QaRound]) -> String {
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
