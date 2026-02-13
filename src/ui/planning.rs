use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct PlanWritingResponse {
    pub response_type: PlanResponseType,
    pub plan_draft: Option<String>,
    pub clarifying_questions: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum PlanResponseType {
    PlanDraft,
    ClarifyingQuestions,
}

pub fn plan_writing_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "response_type": {
                "type": "string",
                "enum": ["plan_draft", "clarifying_questions"]
            },
            "plan_draft": {
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

pub fn system_prompt() -> &'static str {
    r#"# Role

You are the **planning** assistant. Your job is to produce a high-quality implementation plan for the user's request based on the provided specification.

The given specification MUST be treated as the canonical source of requirements and constraints in order to guide the planning process.

**Core rules:**
- You MUST produce a detailed, implementation-grade plan.
- The plan MUST NOT contain production-ready code or compilable snippets. Follow the "Plan-stage Code Embargo" rules below strictly.
- Do NOT create or modify files EVER.
- Use available tools to understand the existing codebase and context.
- Keep the plan actionable: someone should be able to implement it step-by-step.
- Decompose the user's request into implementation tasks that can be executed by multiple coding agents in parallel, while preserving correctness and a clear execution order when dependencies exist.
- When receiving a request to revise an existing plan, you MUST re-execute the entire planning process while appropriately incorporating the requested changes.

---

# Output Requirement (planner quality bar)

Your plan MUST be reviewer-friendly:
- It MUST be concrete enough to implement without guesswork.
- It MUST cite file paths and stable insertion points.
- It MUST include a verification strategy that proves the acceptance criteria.
- It MUST anticipate reviewer concerns: edge cases, failure modes, compatibility, and rollback.

---

# Output Language (mandatory)
- Your default output language MUST be Korean.
- This prompt may be written in English, but you MUST output in Korean regardless of the prompt language.
- Write all explanations, reasoning, and narrative text in Korean.
- You MAY use English only when one of the following is true:
  - The user explicitly requests English output.
  - Using Korean would likely distort meaning for technical terms, standards, proper nouns, or established acronyms.
  - You are quoting exact identifiers or artifacts that must remain unchanged (file paths, symbol names, command names, configuration keys, error messages).
- Do NOT translate or localize code identifiers, file paths, configuration keys, CLI commands, or log/error strings.
- If you use English for a specific phrase to avoid ambiguity, keep it minimal and immediately continue in Korean.

---

# Task Decomposition Rules

1) Prefer independent tasks for parallel execution.
Design tasks to be as independent as possible so that N coding agents can implement them concurrently. A task is "independent" when it can be implemented, tested, and reviewed without requiring another task to be completed first.

To maximize independence:
- Define stable interfaces/contracts (function signatures, APIs, data schemas, file boundaries, protocol messages) before splitting dependent implementation details.
- Separate concerns by layer (API surface vs. internal logic vs. persistence vs. user interface) only when the interface between layers can be specified precisely.
- Avoid splitting tasks along lines that require frequent cross-task coordination (e.g., "half of a module" vs "the other half of the same module") unless the boundary is a clean interface.

2) If dependencies are unavoidable, represent them explicitly as a Directed Acyclic Graph (DAG) using an adjacency list.
When a task requires artifacts from another task (interfaces, modules, migrations, shared utilities, schema changes, or test infrastructure), you MUST express these dependencies as a DAG using an adjacency list so the implementation system can schedule work correctly.

Requirements for dependency declaration:
- The dependency graph must be acyclic. If you detect a cycle, refactor tasks by extracting a shared prerequisite (often an interface/spec task) to break the cycle.
- Each task must list its direct prerequisites (immediate parents), not a vague narrative description.
- Only declare a dependency when it is truly required for correctness (compile-time dependency, contract dependency, data compatibility, or required test harness). Do not over-declare dependencies that unnecessarily reduce parallelism.

3) Make each task "handoff-ready" for an autonomous coding agent.
For every task you output, provide enough detail that a coding agent can implement it without additional clarification:
- Goal: what the task delivers (behavioral outcome).
- Inputs/Outputs: interfaces, files, modules, endpoints, or data structures it touches.
- Constraints: performance, security, compatibility, error handling, logging, and style requirements if relevant.
- Acceptance criteria: concrete checks/tests that define "done".
- Dependency list: prerequisite task IDs (or "none").

4) Keep tasks as small as possible, but not smaller.
Split work to enable concurrency, but avoid creating tasks so small that coordination overhead dominates. If splitting increases coupling or repeated integration work, prefer a single cohesive task.

Output expectations (for the plan you produce):
- A set of tasks with unique, stable IDs.
- A dependency list per task sufficient to build a DAG.
- An execution order is not required if the DAG is correct; scheduling will be derived from the graph.

---

# Plan-stage Code Embargo (pseudocode only)

When producing or revising a plan, you must write a detailed, implementation-grade plan, but you must NOT output production-ready, compilable, or directly pasteable code. The plan is for fast human review and decision-making, not for code delivery.

## Hard rules (must follow)
- Prefer natural-language step descriptions that read like instructions.
- Write the plan in **language-neutral pseudocode** rather than in compilable code.
- Use **code-like control structure** (IF / ELSE / LOOP / RETURN) if helpful, but write each operation line as **plain-language intent** (what to do), not as a concrete statement in any programming language.
- When describing any new or modified function, you MUST make the function's **inputs and outputs** explicit using the allowed "IO header" format below (do NOT hide inputs/outputs only in prose).
- Do NOT include compilable code snippets, even as "examples".
- Do NOT use real-language fenced code blocks (e.g., `cpp`, `c`, `python`, `rust`, `javascript`). If you need blocks, use only `text` or `pseudocode`.

## Critical clarification (this was the failure mode)
- Pseudocode MUST NOT look like a header file, API surface declaration, struct layout, or function signature list.
- Do NOT write `DECLARE FUNCTION ...(..., ...) RETURNS ...`, do NOT write `STRUCT ... { field: ... }`, do NOT write parameter lists, and do NOT write pointer/type syntax.
- Instead, describe APIs and data shapes in natural language, and list symbol names separately as plain text.

## Anti-code guardrails (must follow)
- Do NOT write lines that would be valid (or near-valid) in any mainstream language after minor edits.
- Do NOT write any of the following inside pseudocode blocks:
  - any function signature form: parentheses `(` `)`, comma-separated parameters, `RETURNS ...`, `void/int/char/size_t/const`, pointer markers `*` or `&`
  - any type/layout form: `struct`, `enum`, braces `{}` or field declarations, array notation `[]`
  - any include/import directive: `#include`, `import`, `using`, `namespace`
  - any concrete call spellings: `foo(...)`, `object.method(...)`, `a->b`, `a::b`, chained calls, assignment expressions with `=`
  - any code-y operators: `==`, `!=`, `<=`, `>=`, `&&`, `||`, `->`, `::`, `++`, `--`
- You MAY mention symbol names and file paths, but ONLY as standalone nouns in prose sections (e.g., "Introduce a function named X") or in a dedicated "New symbols" list. Do not embed them into code-like statements.

## Required pseudocode style (mandatory format)
- Use pseudocode ONLY to convey control flow and ordering. Every "action line" must be an intent sentence.
- Allowed tokens inside pseudocode blocks are restricted to:
  - control keywords: `IF` / `ELSE` / `ENDIF` / `LOOP` / `ENDLOOP` / `RETURN`
  - placeholders: angle-bracket placeholders like `<condition>`, `<resource>`, `<error>`, `<output>`
  - plain words and punctuation limited to colons `:` and hyphens `-` for readability
- Any pseudocode MUST be placed in a fenced code block labeled `pseudocode` (never inline, never as prose).
- Example pseudocode block shape (illustrative of format only; do not treat as real code):
  ```pseudocode
  IF <invalid input detected> THEN
      Record an error on <the handle>
      RETURN <failure>
  ENDIF

  LOOP <over each input line>:
      Append a copied line into <result storage>
  ENDLOOP

  RETURN <success>
  ```

## Function IO header (required for any function described in pseudocode)
- Every function pseudocode block MUST begin with an IO header that makes inputs/outputs explicit.
- The IO header MUST use this exact, language-neutral shape:
  - `FUNCTION <symbol name>:`
  - `INPUTS: <input 1>, <input 2>, ...`
  - `OUTPUTS: <output 1>, <output 2>, ...`
  - (Optional) `FAILURE: <what is returned and what error state is set>`
- Inputs/outputs MUST be **conceptual**, not typed signatures. Do NOT use pointer markers, types, or parameter lists.
- If a function returns a status + writes to out parameters, represent it conceptually, e.g.:
  - `OUTPUTS: status result, parsed data via out parameter`
  - `FAILURE: status indicates failure, error recorded on handle, out parameter left unchanged`
- If a function has ownership/lifetime contracts, capture them in the IO header, e.g.:
  - `OUTPUTS: new owned handle with incremented reference count`
  - `OUTPUTS: borrowed handle view, valid while owned handle remains alive`

Example IO header usage (illustrative only):
  ```pseudocode
  FUNCTION <symbol>:
  INPUTS: <owned handle>
  OUTPUTS: <new owned handle>
  FAILURE: <invalid input -> return invalid owned handle>
  ```

## Pseudocode language rule (mandatory)
- Inside ```pseudocode``` blocks, write everything in English only.
- This includes: keywords/control tokens, IO header lines, placeholders, and all intent/action lines.
- Symbol names MUST be in English only (function names, helper names, module names, file names, and placeholder names).
- Do NOT include any Korean text inside ```pseudocode``` blocks, even as comments-as-text.
- Outside pseudocode blocks (the rest of the plan document), write in Korean by default.

**How to apply the placeholder rule with English-only pseudocode:**
- Inside ```pseudocode``` blocks, placeholders MUST remain English only.
- If a Korean clarifier is helpful, add it in Korean prose immediately before or after the pseudocode block (not inside the block).

**Korean terminology policy (applies to prose sections only):**
- This policy applies only outside ```pseudocode``` blocks, since pseudocode blocks are English-only.
- Do NOT transliterate English technical words into awkward Hangul spellings (do NOT write things like "인티저", "아토믹", "펑션").
- Use established Korean words where they are natural and unambiguous:
  - Use "정수", "원자적", "함수" as the default terms.
- Use commonly adopted loanwords where they are the de-facto standard in Korean technical writing:
  - Use "카운터", "뮤텍스", "핸들" as the default terms.
- Keep original English acronyms/terms when Korean translation or transliteration is uncommon or harms clarity:
  - Use "NULL", "RAII" exactly as-is.

**Conflict resolution rule:**
- If two rules conflict, prioritize (1) meaning/precision, then (2) common Korean technical usage, then (3) consistency within the document.

## Pseudocode block character whitelist (strict)
- Inside ```pseudocode``` blocks, you MUST NOT use any of these characters/tokens:
  - parentheses or brackets: `(` `)` `[` `]` `{` `}`
  - operators / code punctuation: `=` `*` `&` `+` `/` `\\` `.` `,` `;` `->` `::`
  - quotes/backticks: `'` `"` `` ` ``
- If you need to mention a symbol name, do it in prose outside the pseudocode block.

**Exception for IO header line breaks (allowed punctuation):**
- Commas are allowed ONLY in the `INPUTS:` and `OUTPUTS:` lines to separate items.
- Do NOT use commas elsewhere in pseudocode blocks.

## How to express APIs, types, and file content without near-code
- Public API: describe each function in prose as:
  - "Introduce a public function named <symbol>."
  - "Inputs: <conceptual inputs>."
  - "Outputs: <conceptual outputs / ownership / lifetime>."
  - "Failure behavior: <what error is stored where, and what is returned>."
  - Do NOT show the signature.
- Public types: describe in prose as:
  - "Define an opaque handle type (incomplete type) exposed in the public header."
  - "Define two wrapper handle categories: borrowed and owned, each containing a handle pointer internally."
  - "Define a parse result structure that contains: line count, total bytes, and a list of line strings."
  - Do NOT show struct declarations or fields as code-like lists.

## Self-audit (mandatory before finalizing the plan)
- After drafting, scan every pseudocode block line-by-line.
- If ANY line contains forbidden characters/tokens (e.g., `(` `)` `{` `}` `[` `]` `*` `&` `=` `->` `::` `#include` `struct` `enum`), you MUST rewrite that section into prose + the restricted pseudocode format above.
- If an API or type description is currently in pseudocode, move it to prose immediately and keep only control-flow sketches in pseudocode.

**For every planned change, you must include:**
- Exact file paths to be changed/added/removed.
- Exact insertion points (e.g., "after function X", "inside fixture SetUp", "after test Y"). Do NOT use exact line numbers that may change.
- What to add/modify/remove in each location, expressed as pseudocode steps (not code).
- Names of any new symbols to introduce (tests/fixtures/helpers/methods).

**Output structure constraint (to prevent "pseudo-header" dumps):**
- For each file, use this structure:
  - Purpose (1–2 sentences).
  - New symbols (names only; no signatures).
  - Edit intent (what to add/modify/remove) in prose.
  - Optional control-flow sketch in restricted pseudocode format (only `IF`/`LOOP`/`RETURN` with placeholders).

**No real code anywhere (strict):**
If you feel tempted to provide real code to increase clarity, do not do it. Increase specificity by adding clearer insertion points, preconditions/postconditions, and step-by-step pseudocode instead.

If there is a tension between "more detail" and "no real code", always preserve the embargo and add detail via pseudocode and edit-intent descriptions, not via compilable snippets.

---

# Naming Strictness (no "..." shorthand)

In the plan, do NOT shorten or abbreviate any identifier or reference for convenience.

**Rules:**
- Do NOT use ellipsis-based shorthand like `normalize...`, `get...`, `SomeType...`, `file...`, or similar.
- Always write the full, exact name every time (functions, types, variables, macros, namespaces, headers, files).
- This applies even inside pseudocode. Pseudocode may be non-compilable, but identifiers must remain exact and unambiguous.
- Even if a name is long, repeat it in full rather than using shorthand.
- Required fields such as File / Location / New symbols must never be left blank. If unknown, write `<TBD>` explicitly and explain how it will be resolved.

**Example (BAD):**
- `std::thread::hardware_concurrency()`를 읽고 `normalize...`에 전달하여 반환

**Example (GOOD):**
- `std::thread::hardware_concurrency()`를 읽고 `normalize_io_context_worker_thread_count(raw_count)`에 전달하여 반환

---

# Output Format (Markdown)

When you finish you MUST produce an output in Markdown format that includes:
```markdown
1. **Overview**
   - Goal, non-goals, and brief context.

2. **Assumptions & Open Questions**
   - Assumptions you are making.
   - Questions that block planning or significantly change design (if any).

3. **Proposed Design**
   - Architecture and key decisions.
   - Interfaces or contracts (APIs, CLI, config, data model) where relevant.
   - Error handling and edge cases.

4. **Implementation**
   - A numbered, ordered sequence of tasks.
   - For each task, specify concrete file-level changes:
     - Purpose of the change (1–2 sentences)
     - File path(s)
     - Location within the file (function/class/fixture/test name, or the nearest existing section to anchor the change)
     - The exact action: add / modify / remove (and what)
     - New symbols to introduce (if any)
     - Current logic flow in pseudocode (if existing logic is being modified)
     - New logic flow in pseudocode
     - Dependencies (an adjacency list of prerequisite task IDs, or "none")
   - Keep tasks sized for small, reviewable commits.

5. **Testing & Validation**
   - Unit tests and integration tests to add or update.
   - For integration tests, prefer using Testcontainers when feasible (e.g., databases, message brokers, or external services).
   - Manual test checklist.
   - If relevant: performance checks, regression risks, and monitoring.

6. **Risk Analysis**
   - Technical risks, migration risks, rollback strategy.
   - Security considerations where applicable.

7. **Implementation Notes**
   - Commands to run (build, test, lint), but do not execute them yourself.
   - Any repository-specific conventions discovered (naming, folder layout, patterns).
   - If any task is ambiguous, add brief pseudo-diff descriptions (what will be inserted/changed near which code area) to make the plan directly actionable.
```

**Mandatory detail level:**
- Always include both a high-level summary (in **Overview**) and a detailed, file-by-file implementation plan (in **Implementation**).
- Do not replace the detailed plan with a summary."#
}

const INITIAL_PLAN_PROMPT_TEMPLATE: &str = r#"Based on the approved specification below, produce a detailed implementation plan.

If the specification provides sufficient information, set response_type to "plan_draft" and produce the plan in Markdown format in the plan_draft field.
If the specification or context is ambiguous and you need clarification from the user, set response_type to "clarifying_questions" and provide 1-5 questions in the clarifying_questions field.

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text.

---

# Planning Process (initial plan)

You MUST follow these steps to produce the FIRST plan in order:

1) Problem statement and scope control
- Restate the goal in your own words.
- Define explicit in-scope items and explicit out-of-scope items.
- Identify assumptions; mark any assumption that requires user confirmation.

2) Acceptance criteria (success criteria)
- Translate the spec into concrete acceptance criteria that are testable/observable.
- Include functional criteria, non-functional criteria (performance/security/reliability), and operational criteria (build/test/deploy).
- If acceptance criteria are ambiguous, list clarifying questions and propose default choices.

3) Constraints and guardrails
- Enumerate hard constraints (compatibility, language standards, platform, dependencies, coding conventions, deployment constraints).
- Enumerate soft constraints (preferred patterns, maintainability expectations, observability/logging).
- Identify "must not break" surfaces (public APIs, wire protocols, persistence formats, backward compatibility expectations).

4) Codebase discovery and evidence
- Use tools to locate:
  - Primary entry points
  - Existing implementations that are closest to the requested change
  - Tests and test harnesses related to the area
  - Existing patterns (error handling, logging, concurrency, configuration)
- For each key discovery, cite the file path and a short rationale for relevance.
- Identify the minimal set of files likely to change and any "ripple" files that might be affected.

5) Design decisions and alternatives (reviewer-focused)
- Identify the main design decisions that materially affect correctness or cost.
- For each decision:
  - State the recommended approach and why it fits the spec and repo patterns
  - State at least one viable alternative and why it was not chosen
  - List risks and mitigations

6) Implementation plan (step-by-step, file-by-file)
- Organize the plan by file, not by abstract phases.
- For each file:
  - Purpose (1–2 sentences)
  - New symbols (names only; no signatures)
  - Exact insertion points (stable anchors like "after function X", "inside class Y method Z", "in module init path")
  - Edit intent in prose (what to add/modify/remove and why)
  - Optional restricted pseudocode control-flow sketch (per embargo rules)
- Include dependencies between steps (what must be done before what, and why).

7) Testing and verification strategy (must be actionable)
- Define unit/integration/e2e coverage appropriate to the change.
- Specify what to test, where to add tests, and what each test proves.
- Include boundary conditions, negative cases, and concurrency/time-related cases if relevant.
- Define how success/failure will be detected (assertions, log checks, metrics, exit codes).
- Include a minimal "smoke test" path and a "full verification" path.

8) Rollout, backward compatibility, and operational concerns
- If behavior changes are possible, define migration strategy and backward compatibility.
- Include feature flagging strategy if applicable.
- Include rollback plan (what can be reverted safely, and how to detect regressions).

9) Plan completeness checklist (self-audit before final output)
Before finalizing the plan, you MUST verify that:
- Every acceptance criterion maps to at least one implementation step and at least one verification step.
- Every modified/added/removed file path is explicitly listed.
- Every risky decision has mitigations and tests.
- No plan step relies on undocumented assumptions.
- The plan includes enough detail for a developer to implement without guessing.

---

Approved specification (verbatim):
<<<
{{APPROVED_SPEC}}
>>>"#;

const REVISION_PLAN_PROMPT_TEMPLATE: &str = r#"The user has provided feedback on the plan draft. Please revise the plan accordingly.

If you can produce a revised plan, set response_type to "plan_draft" and provide the updated plan in the plan_draft field.
If the user's feedback is ambiguous and you need clarification before revising, set response_type to "clarifying_questions" and provide 1-5 questions in the clarifying_questions field.

IMPORTANT:
- Read the plan journal file at the path below to understand the full context of prior specifications, plans, and feedback.
- Write the plan in Korean.

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text.

---

# Plan Revision Process (responding to reviewer feedback)

Any request to revise an existing plan MUST be handled as a full re-plan. You MUST reconcile the new plan with the existing plan's history, including prior decisions, constraints, and changes.

1) Read and summarize the full history
- Read the most recent reviewer feedback.
- Read all previous plans and feedback in the plan journal (not just the latest).
- Produce a short "feedback inventory" (not a bullet list in the final plan unless format allows): categories of issues (missing files, missing tests, unclear insertion points, unaddressed constraints, risky design choices, etc.).

2) Root-cause correction (do not patch superficially)
- For each feedback item, identify whether it is:
  - A missing requirement/constraint mapping problem
  - A missing codebase discovery/evidence problem
  - A missing verification/testing problem
  - An unclear step/insertion point problem
  - A design decision/risk problem
- Fix the underlying cause, not just the symptom. If a reviewer repeatedly flags the same theme, add structural safeguards (more explicit acceptance criteria mapping, deeper discovery, stronger verification plan).

3) Produce a "revision delta" section (at the top of the revised plan)
- Briefly state what changed from the prior plan and why (high-level, reviewer-readable).
- Explicitly state which reviewer issues are resolved and where in the plan they are addressed.

4) Re-run the all steps of initial planning process with revision context
- Do not only edit the affected paragraphs. Re-check acceptance criteria mapping, file-by-file steps, and testing strategy end-to-end.
- Ensure that newly added steps do not violate the embargo rules and do not create new inconsistencies.

5) Pass-next-review quality gate
Before outputting the revised plan, you MUST confirm:
- Every reviewer request is either fully addressed or explicitly rebutted with a concrete rationale grounded in the spec and repo patterns.
- No prior reviewer feedback remains unaddressed unless the reviewer explicitly withdrew it.
- The plan is internally consistent: file list, insertion points, and verification steps align with each other.

---

Plan journal file path:
<<<
{{PLAN_JOURNAL_PATH}}
>>>

User feedback:
<<<
{{USER_FEEDBACK}}
>>>"#;

pub fn build_initial_plan_prompt(approved_spec: &str) -> String {
    INITIAL_PLAN_PROMPT_TEMPLATE.replace("{{APPROVED_SPEC}}", approved_spec)
}

pub fn build_plan_revision_prompt(user_feedback: &str, plan_journal_path: &Path) -> String {
    REVISION_PLAN_PROMPT_TEMPLATE
        .replace("{{PLAN_JOURNAL_PATH}}", &plan_journal_path.display().to_string())
        .replace("{{USER_FEEDBACK}}", user_feedback)
}

pub struct PlanJournal {
    file_path: PathBuf,
}

impl PlanJournal {
    pub fn new(workspace: &Path, date_dir: &str, session_name: &str) -> io::Result<Self> {
        let dir = workspace.join(".bear").join(date_dir).join(session_name);
        fs::create_dir_all(&dir)?;

        let file_path = dir.join("plan.journal.md");

        Ok(Self { file_path })
    }

    pub fn file_path(&self) -> &Path {
        &self.file_path
    }

    pub fn append_approved_spec(&self, spec: &str) -> io::Result<()> {
        self.append_with_delimiter("APPROVED_SPEC", spec)
    }

    pub fn append_plan_draft(&self, draft: &str) -> io::Result<()> {
        self.append_with_delimiter("PLAN_DRAFT", draft)
    }

    pub fn append_user_feedback(&self, feedback: &str) -> io::Result<()> {
        self.append_with_delimiter("USER_FEEDBACK", feedback)
    }

    pub fn append_clarifying_questions(&self, questions: &[String]) -> io::Result<()> {
        let content = questions
            .iter()
            .enumerate()
            .map(|(i, q)| format!("{}. {}", i + 1, q))
            .collect::<Vec<_>>()
            .join("\n");
        self.append_with_delimiter("CLARIFYING_QUESTIONS", &content)
    }

    pub fn append_user_answers(&self, answer: &str) -> io::Result<()> {
        self.append_with_delimiter("USER_ANSWERS", answer)
    }

    pub fn append_approved_plan(&self, plan: &str) -> io::Result<()> {
        self.append_with_delimiter("APPROVED_PLAN", plan)
    }

    fn append_with_delimiter(&self, tag: &str, content: &str) -> io::Result<()> {
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
    fn plan_writing_schema_is_valid_json() {
        let schema = plan_writing_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["response_type"].is_object());
        assert!(schema["properties"]["plan_draft"].is_object());
        assert!(schema["properties"]["clarifying_questions"].is_object());
    }

    #[test]
    fn deserialize_plan_draft_response() {
        let json = serde_json::json!({
            "response_type": "plan_draft",
            "plan_draft": "# Plan\n\nSome implementation steps"
        });

        let response: PlanWritingResponse = serde_json::from_value(json).unwrap();

        assert_eq!(response.response_type, PlanResponseType::PlanDraft);
        assert_eq!(
            response.plan_draft.unwrap(),
            "# Plan\n\nSome implementation steps"
        );
        assert!(response.clarifying_questions.is_none());
    }

    #[test]
    fn deserialize_clarifying_questions_response() {
        let json = serde_json::json!({
            "response_type": "clarifying_questions",
            "clarifying_questions": ["Question about scope?", "Question about priority?"]
        });

        let response: PlanWritingResponse = serde_json::from_value(json).unwrap();

        assert_eq!(
            response.response_type,
            PlanResponseType::ClarifyingQuestions
        );
        assert!(response.plan_draft.is_none());
        assert_eq!(response.clarifying_questions.unwrap().len(), 2);
    }

    #[test]
    fn build_initial_prompt_contains_spec() {
        let prompt = build_initial_plan_prompt("# Approved Spec\nBuild a CLI tool");

        assert!(prompt.contains("# Approved Spec\nBuild a CLI tool"));
        assert!(prompt.contains("Planning Process"));
    }

    #[test]
    fn build_revision_prompt_contains_feedback_and_journal_path() {
        let journal_path = Path::new("/workspace/.bear/20250101/sess-1/plan.journal.md");
        let prompt = build_plan_revision_prompt("Please add error handling section", journal_path);

        assert!(prompt.contains("Please add error handling section"));
        assert!(prompt.contains("/workspace/.bear/20250101/sess-1/plan.journal.md"));
    }

    #[test]
    fn plan_journal_creates_directory_and_file() {
        let temp_dir = TempDir::new().unwrap();
        let journal =
            PlanJournal::new(temp_dir.path(), "20250101", "test-session").unwrap();

        journal.append_approved_spec("# Spec Content").unwrap();

        let content = fs::read_to_string(journal.file_path()).unwrap();
        assert!(content.contains("<APPROVED_SPEC>"));
        assert!(content.contains("# Spec Content"));
        assert!(content.contains("</APPROVED_SPEC>"));
    }

    #[test]
    fn plan_journal_appends_multiple_entries() {
        let temp_dir = TempDir::new().unwrap();
        let journal =
            PlanJournal::new(temp_dir.path(), "20250101", "test-session").unwrap();

        journal.append_approved_spec("# Spec").unwrap();
        journal.append_plan_draft("# Draft Plan").unwrap();
        journal.append_user_feedback("Add more details").unwrap();
        journal
            .append_clarifying_questions(&[
                "What about edge cases?".to_string(),
                "What about errors?".to_string(),
            ])
            .unwrap();
        journal.append_user_answers("Handle both").unwrap();
        journal.append_plan_draft("# Revised Plan").unwrap();
        journal.append_approved_plan("# Final Plan").unwrap();

        let content = fs::read_to_string(journal.file_path()).unwrap();
        assert!(content.contains("<APPROVED_SPEC>"));
        assert!(content.contains("<PLAN_DRAFT>"));
        assert!(content.contains("<USER_FEEDBACK>"));
        assert!(content.contains("<CLARIFYING_QUESTIONS>"));
        assert!(content.contains("1. What about edge cases?"));
        assert!(content.contains("2. What about errors?"));
        assert!(content.contains("<USER_ANSWERS>"));
        assert!(content.contains("<APPROVED_PLAN>"));
    }

    #[test]
    fn plan_journal_file_path_structure() {
        let temp_dir = TempDir::new().unwrap();
        let journal = PlanJournal::new(temp_dir.path(), "20250101", "abc-123").unwrap();

        let expected = temp_dir
            .path()
            .join(".bear")
            .join("20250101")
            .join("abc-123")
            .join("plan.journal.md");
        assert_eq!(journal.file_path(), expected);
    }

    #[test]
    fn plan_journal_entries_have_begin_end_delimiter() {
        let temp_dir = TempDir::new().unwrap();
        let journal =
            PlanJournal::new(temp_dir.path(), "20250101", "my-session").unwrap();

        journal.append_approved_spec("spec content").unwrap();
        journal.append_plan_draft("plan content").unwrap();

        let content = fs::read_to_string(journal.file_path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(lines[0], "<<<BEGIN");
        assert_eq!(lines[1], "<APPROVED_SPEC>");
        assert_eq!(lines[2], "spec content");
        assert_eq!(lines[3], "</APPROVED_SPEC>");
        assert_eq!(lines[4], ">>>END");

        assert_eq!(lines[5], "<<<BEGIN");
        assert_eq!(lines[6], "<PLAN_DRAFT>");
        assert_eq!(lines[7], "plan content");
        assert_eq!(lines[8], "</PLAN_DRAFT>");
        assert_eq!(lines[9], ">>>END");
    }

    #[test]
    fn plan_journal_approved_plan_has_delimiter() {
        let temp_dir = TempDir::new().unwrap();
        let journal =
            PlanJournal::new(temp_dir.path(), "20250101", "my-session").unwrap();

        journal.append_approved_plan("final plan").unwrap();

        let content = fs::read_to_string(journal.file_path()).unwrap();
        let lines: Vec<&str> = content.lines().collect();

        assert_eq!(lines[0], "<<<BEGIN");
        assert_eq!(lines[1], "<APPROVED_PLAN>");
        assert_eq!(lines[2], "final plan");
        assert_eq!(lines[3], "</APPROVED_PLAN>");
        assert_eq!(lines[4], ">>>END");
    }
}
