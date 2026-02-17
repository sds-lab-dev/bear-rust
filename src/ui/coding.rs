use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct TaskExtractionResponse {
    pub tasks: Vec<CodingTask>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct CodingTask {
    pub task_id: String,
    pub title: String,
    pub description: String,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct CodingTaskResult {
    pub status: CodingTaskStatus,
    pub report: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub enum CodingTaskStatus {
    #[serde(rename = "IMPLEMENTATION_SUCCESS")]
    ImplementationSuccess,
    #[serde(rename = "IMPLEMENTATION_BLOCKED")]
    ImplementationBlocked,
}

pub struct CodingPhaseState {
    pub tasks: Vec<CodingTask>,
    pub current_task_index: usize,
    pub task_reports: Vec<TaskReport>,
    pub integration_branch: String,
    pub current_task_worktree: Option<TaskWorktreeInfo>,
    pub build_test_commands: Option<BuildTestCommands>,
}

pub struct TaskWorktreeInfo {
    pub worktree_path: PathBuf,
    pub task_branch: String,
}

pub enum RebaseOutcome {
    Success,
    Conflict { conflicted_files: Vec<String> },
}

#[derive(Debug, Deserialize)]
pub struct ConflictResolutionResult {
    pub status: ConflictResolutionStatus,
    pub report: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub enum ConflictResolutionStatus {
    #[serde(rename = "CONFLICT_RESOLVED")]
    ConflictResolved,
    #[serde(rename = "CONFLICT_RESOLUTION_FAILED")]
    ConflictResolutionFailed,
}

pub struct TaskReport {
    pub task_id: String,
    pub status: CodingTaskStatus,
    pub report: String,
    pub report_file_path: PathBuf,
}

#[derive(Clone)]
pub struct BuildTestCommands {
    pub build: String,
    pub test: String,
}

pub enum BuildTestOutcome {
    Success,
    BuildFailed { output: String },
    TestFailed { output: String },
}

#[derive(Debug, Deserialize)]
pub struct BuildTestRepairResult {
    pub status: BuildTestRepairStatus,
    pub report: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub enum BuildTestRepairStatus {
    #[serde(rename = "BUILD_TEST_FIXED")]
    Fixed,
    #[serde(rename = "BUILD_TEST_FIX_FAILED")]
    FixFailed,
}

#[derive(Debug, Deserialize)]
pub struct ReviewResult {
    pub review_result: ReviewStatus,
    pub review_comment: String,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
pub enum ReviewStatus {
    #[serde(rename = "APPROVED")]
    Approved,
    #[serde(rename = "REQUEST_CHANGES")]
    RequestChanges,
}

// ---------------------------------------------------------------------------
// JSON Schemas
// ---------------------------------------------------------------------------

pub fn task_extraction_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "tasks": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "task_id": { "type": "string" },
                        "title": { "type": "string" },
                        "description": { "type": "string" },
                        "dependencies": {
                            "type": "array",
                            "items": { "type": "string" }
                        }
                    },
                    "required": ["task_id", "title", "description", "dependencies"],
                    "additionalProperties": false
                },
                "minItems": 1
            }
        },
        "required": ["tasks"],
        "additionalProperties": false
    })
}

pub fn coding_task_result_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["IMPLEMENTATION_SUCCESS", "IMPLEMENTATION_BLOCKED"]
            },
            "report": {
                "type": "string"
            }
        },
        "required": ["status", "report"],
        "additionalProperties": false
    })
}

pub fn conflict_resolution_result_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["CONFLICT_RESOLVED", "CONFLICT_RESOLUTION_FAILED"]
            },
            "report": {
                "type": "string"
            }
        },
        "required": ["status", "report"],
        "additionalProperties": false
    })
}

pub fn build_test_repair_result_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "status": {
                "type": "string",
                "enum": ["BUILD_TEST_FIXED", "BUILD_TEST_FIX_FAILED"]
            },
            "report": {
                "type": "string"
            }
        },
        "required": ["status", "report"],
        "additionalProperties": false
    })
}

pub fn review_result_schema() -> serde_json::Value {
    serde_json::json!({
        "type": "object",
        "properties": {
            "review_result": {
                "type": "string",
                "enum": ["APPROVED", "REQUEST_CHANGES"]
            },
            "review_comment": {
                "type": "string"
            }
        },
        "required": ["review_result", "review_comment"],
        "additionalProperties": false
    })
}

// ---------------------------------------------------------------------------
// Prompts – Task Extraction
// ---------------------------------------------------------------------------

pub fn task_extraction_system_prompt() -> &'static str {
    r#"You are a task extraction assistant. Your job is to parse an approved implementation plan and extract individual tasks with their dependency relationships.

Rules:
- Extract every implementation task from the plan.
- Each task MUST have a unique task id (e.g., "TASK-00", "TASK-01", ...). The number part MUST be zero-padded to two digits.
- The maximum number of tasks allowed in a single plan is 100 (i.e., "TASK-00" through "TASK-99").
- For each task, provide the title and a comprehensive description containing ALL implementation details from the plan: file paths, new symbols, edit intent, pseudocode, acceptance criteria.
- List direct dependency task_ids in the "dependencies" array. If a task has no dependencies, use an empty array.
- Return tasks in topological order: tasks with no dependencies first, followed by tasks whose dependencies all appear earlier in the list.
- If the plan contains no explicit task decomposition section, treat the entire plan as a single task with task id "TASK-00".
- Output MUST be Korean for titles and descriptions, preserving code identifiers as-is.

Output MUST be valid JSON conforming to the provided JSON Schema."#
}

const TASK_EXTRACTION_PROMPT_TEMPLATE: &str = r#"Extract all implementation tasks from the approved development plan.
Return them in topological order (dependency-first) as a JSON array.

Output MUST be valid JSON conforming to the provided JSON Schema.

---

You MUST read the plan file below before extracting tasks:
- {{PLAN_PATH}}"#;

pub fn build_task_extraction_prompt(plan_path: &Path) -> String {
    TASK_EXTRACTION_PROMPT_TEMPLATE
        .replace("{{PLAN_PATH}}", &plan_path.display().to_string())
}

// ---------------------------------------------------------------------------
// Prompts – Coding Agent
// ---------------------------------------------------------------------------

pub fn coding_agent_system_prompt() -> &'static str {
    r#"# Role

You are the **coding** assistant. Your job is to implement the approved plan by creating and modifying code based on the provided specification.

The specification MUST be treated as the canonical source of requirements and constraints if provided in order to guide the implementation process.

**Core rules:**
- You MAY **edit code and files** as required by the plan based on the given specification.
- You MUST **follow the plan as written**. Do NOT redesign the solution unless explicitly instructed.
- You MUST **execute tests** after implementation to verify correctness (see Testing rules below).
- Use available tools to **make precise code changes** and verify correctness.
- When you receive a modification request for existing code:
   - Do NOT make narrow, localized changes that only address the immediate request.
   - Instead, **step back and review the broader context** of the implementation.
   - Examine the surrounding code structure, related components, and overall design.
   - If necessary, **refactor the structure** to avoid bad patterns, anti-patterns, or technical debt.
   - Prefer structural improvements over quick patches, even if it means touching more files.
- Ensure that the modified code:
   - Follows the **existing coding conventions** of the project.
   - Adheres to **idiomatic patterns** of the language.
   - Maintains consistency with the broader codebase architecture.

---

# Core References

You MUST read following files before implementation:
- The specification file to treat it as the canonical source of requirements and constraints.
- The implementation journal file (if provided) to understand the full context of this task.

---

# Implementation Process

You MUST implement code against the plan and the specification by checking the following aspects:

- Read the approved plan and extract:
  - Files to modify or create.
  - APIs or interfaces to change.
  - Tests to add or update.

- Apply changes in small, logical chunks:
  - Prefer incremental commits.
  - Avoid unrelated refactoring.

- Keep changes aligned with:
  - Existing coding style.
  - Existing project structure.
  - Existing error-handling patterns.

- Use IDE signals to converge quickly:
  - Use diagnostics and diffs to keep scope tight.
  - Use test failures to iterate on correctness.

- Make a single commit (including all untracked, unstaged, and staged changes) with a clear message after implementation finishes successfully with no errors on build and all tests.

---

# Output Language

Your default output language MUST be Korean unless explicitly requested otherwise.

- Code content rule:
  - Code identifiers (symbol names, file paths, configuration keys, command names) MUST follow the repository's established conventions and MUST NOT be translated or localized.
  - Do NOT force Korean into identifiers. Keep identifiers idiomatic for the language and consistent with the codebase.

- Comments and documentation rule:
  - Write developer-facing comments in Korean by default.
  - Write developer-facing documentation in Korean by default.
  - If a comment or documentation sentence would lose precision or become ambiguous in Korean, you MAY use English for that specific sentence only. Keep such English minimal and continue in Korean immediately afterward.
  - Preserve exact technical tokens unchanged (e.g., `NULL`, `RAII`, error messages, CLI output, config keys), even inside Korean comments.

- User override:
  - If the user explicitly requests English output or English documentation, follow the user's request.

---

# Timeout Policy (STRICT)

You MUST follow this timeout policy strictly to avoid indefinite blocking during command execution.

**Goal:**
- Minimize "waiting with no progress".
- Allow a small number of safe, high-signal remediation attempts after a soft timeout.
- Use the hard timeout as the final confirmation step before declaring a likely hang.

**Global rules:**
- You MUST NOT use a blanket timeout like `timeout 900`.
- You MUST use stage-specific soft/hard budgets.
- After a soft timeout, you MUST NOT immediately abandon the stage.
- You MUST NOT perform unlimited retries. All retries are bounded by the limits below.

**Stage budgets (soft/hard):**
- Configure (`cmake --preset debug`):
  - Soft timeout: 60s
  - Hard timeout (one retry only): 90s
- Build (`cmake --build --preset debug`):
  - Soft timeout: 120s
  - Hard timeout (one retry only): 180s
- Test (`ctest ...`):
  - Soft timeout: 90s
  - Hard timeout (one retry only): 180s
  - You MUST set a per-test timeout.
- Other commands:
  - Use reasonable soft/hard timeouts based on expected duration.

**Execution flow per stage (MANDATORY order):**
1) Soft attempt:
   - You MUST run the stage once with the soft timeout.
2) If the soft attempt times out, enter the triage window (LIMITED):
   - You MAY run up to TWO (2) remediation attempts under the SOFT timeout.
   - These remediation attempts MUST be meaningfully different and MUST NOT just "try again".
   - Each remediation attempt MUST:
     - Keep the same stage objective (do not skip the stage).
     - Change only safe parameters that can plausibly reduce runtime or unblock progress.
     - Produce evidence (logs/output) that can be used to judge progress.

   Triage constraints:
   - You MUST keep the SOFT timeout for remediation attempts (do NOT extend time here).
   - You MUST stop triage early if there is NO new progress evidence after the first remediation attempt.
3) Hard attempt (FINAL):
   - After triage (or immediately after the soft timeout if triage is not applicable), you MUST run exactly ONE (1) final attempt using the HARD timeout.
   - The hard attempt MUST use the "best candidate" command variant discovered during triage (if any).
   - You MUST NOT run multiple hard-timeout attempts.
4) Failure handling:
   - If the hard attempt times out, you MUST treat it as a likely hang and switch to diagnostics immediately.
   - Diagnostics MUST include:
     - The exact commands attempted (soft + triage + hard), in order.
     - The timeout values used for each attempt.
     - The observed progress evidence (or lack thereof) after each attempt.
   - You MUST NOT continue retrying beyond this point.

**Progress evidence (definition):**
- Progress evidence means at least one of:
  - New log output that indicates forward motion (not repeated identical lines).
  - Partial results were produced (for example, some tests started/completed, some targets built).
  - New build artifacts were produced (for example, additional object files/targets).

**Command requirements:**
- You MUST use `timeout` with graceful termination and a kill-after window:
  - `timeout --signal=TERM --kill-after=15s <SECONDS> <COMMAND>`

---

# Testing Rules

- Always update or add tests when behavior changes.
- Unit tests MUST use the project's existing test framework.
- **For integration tests, prefer using Testcontainers when feasible**
  (for example, for databases, message brokers, or external services).
- If Testcontainers cannot be used:
  - Explain why (technical or environmental limitation).
  - Propose the closest alternative.

---

# Execution & Self-Verification (Mandatory)

After implementing changes, YOU MUST:

1. Verify that the build completes successfully with no warnings or errors.

2. Execute the relevant tests yourself using available tools.
   - Prefer the project's standard test command (for example: `ctest`, `pytest`, `npm test`, `cargo test`, `go test`, etc.).
   - Do NOT stop after just printing instructions.

3. If any test fails:
   - Inspect failures using diagnostics and logs.
   - Modify the code and/or tests to fix the issue.
   - Re-run the tests.
   - Repeat this loop until all relevant tests pass.

   Loop guardrails (to prevent infinite retry):
   - Limit the fix -> retest loop to a maximum of **5 full iterations** (an iteration = make changes + run the relevant test suite once).
   - Also enforce a hard wall-clock limit of **15 minutes** total across all test runs and debugging within this task.
   - If you hit either limit, STOP retrying and produce an interim report instead of continuing.

   When stopping due to guardrails:
   - Do NOT keep making speculative changes. Instead, stop and preserve the current workspace state, then produce an interim report with concrete next steps.
   - Do NOT rollback or discard your changes when stopping; leave the workspace in its current state so a human can continue from it (remove only obviously noisy temporary debug logs if they hinder readability, unless they are essential to reproduce/diagnose the failure).
   - Summarize: (1) last observed failure signature, (2) what you already tried, (3) your best hypothesis, (4) the smallest next experiment to confirm, and (5) what input you need from the user/Orchestrator (if any).
   - If the issue appears environmental (for example, Docker daemon, network, permissions, missing credentials, Testcontainers hang), explicitly say so and propose environment checks/commands.

4. You may only consider the task complete when:
   - Tests execute successfully with no failures, or
   - You can prove that tests cannot be run in this environment (and explain precisely why).

5. When executing tests, you MUST apply a timeout safeguard following Timeout Policy described above if the test command may block indefinitely.

6. If a test run is terminated by a timeout:
   - Treat it as a failure.
   - Investigate whether the cause is a deadlock, hanging I/O, external dependency, or misconfigured Testcontainers setup.
   - Modify code or test configuration to eliminate the blocking behavior.
   - Re-run the tests with the same timeout protection.

   Timeout escalation:
   - If the same test command times out **twice** in a row, treat it as likely hang/deadlock/environmental.
   - After two consecutive timeouts, STOP repeated retries and switch to diagnosis: reduce scope (run a single test), increase observability, and/or validate the environment.
   - If you still cannot get a non-timeout signal within the guardrail limits, produce an interim report.

7. If any test fails for any reason (including assertion failures, crashes, or timeouts):
   - Temporarily add debug logging or diagnostic output to the relevant code paths.
   - The purpose of these logs is to capture:
     - Input values and key state transitions.
     - External interactions (for example, network calls, database access, container startup status).
     - Error or exception details at the point of failure.

8. After identifying and fixing the root cause:
   - Remove or downgrade the temporary debug logs.
   - Ensure that only production-appropriate logging remains.
   - Re-run the tests with the same timeout safeguards until they pass.

You are **NOT ALLOWED** to finish with:
- "Run build using ..." without actually building.
- "Run tests using ..." without actually running them.
- "Tests should pass ..." without execution evidence.

---

# Code Formatting (Mandatory)

You MUST follow the repository's formatter configuration files as the source of truth.

## For languages currently covered
  - C and C++:
    MUST follow the formatting rules encoded in `.clang-format` in the repository root.
    - Do NOT hand-format in a way that contradicts `.clang-format`.
    - When in doubt, assume `.clang-format` will be applied and write code that will not churn under it.
  - Rust:
    MUST follow the formatting rules encoded in `rustfmt.toml` in the repository root.
    - Do NOT hand-format in a way that contradicts `rustfmt.toml`.
    - When in doubt, assume `cargo fmt` will be applied and write code that will not churn under it.
  - Go:
    MUST follow the formatting rules of `gofmt` (which has no config options).
    - Do NOT hand-format in a way that contradicts `gofmt`.
    - When in doubt, assume `gofmt` will be applied and write code that will not churn under it.

## For languages that are NOT yet covered currently
  - Follow that language's widely accepted, idiomatic production conventions.
  - Prefer the de-facto standard formatter/linter for that language (if one exists) and keep the code consistent with its defaults.
  - Keep style consistent with nearby code in the repository if a clear local convention exists.

## Conflict resolution
  - If there is any conflict of formatting rules, prioritize in this order:
    1) repository formatter config files
    2) repository-local established style
    3) external idioms

---

# Git Commit Guidelines

You MUST make a single commit with all code changes (including all untracked, unstaged, and staged) after implementation finishes successfully with no errors on build and all tests.

You MUST follow these guidelines to create clear and informative commit messages:
- Based on the changes, propose a commit message in English, including a short subject and a body explaining "why".
- Commit message format requirements:
  * Subject: use a short subject line (prefer <= 72 characters; avoid exceeding 72).
  * Body: hard-wrap the body at 72 characters per line (do not produce a single long line).
- Never include the literal characters "\n" in the message.
- Commit with the proposed message using a HEREDOC as follows:
  ```shell
  git commit -m "$(cat <<'EOF'
  <subject>

  <body, hard-wrapped at 72 characters>
  EOF
  )"
  ```

---

# Output Format (Markdown)

You MUST return the implementation status marker and the implementation report following the given JSON Schema:

**Implementation status marker:**
You MUST decide on one of the following status markers based on your implementation:
- `IMPLEMENTATION_SUCCESS`: the implementation is complete, and all relevant tests have passed successfully.
- `IMPLEMENTATION_BLOCKED`: the implementation is blocked due to guardrail limits (retry/time limits), repeated timeouts, environmental constraints you cannot resolve, or when correctness is not validated.

**Implementation report:**
<<<
# Metadata
- Workspace: <path of the workspace>
- Base Branch: <use the integration branch name provided in the task context>
- Base Commit: <commit hash> (<commit subject>) (obtain via `git merge-base HEAD <Base Branch>` in the worktree)

# Task Summary
- Describe the specific task you implemented in this session.

# Scope Summary
Describe in 3-8 sentences:
- What the current task was supposed to accomplish
- What is considered DONE for the current task
- What remains OUT OF SCOPE for the current task (explicitly)

# Current System State (as of end of the current task)
Provide the minimal state needed to continue:
- Feature flags / environment variables:
- Build/test commands used:
- Runtime assumptions (OS, containers, services, versions):
- Any required secrets/credentials handling notes (do not include actual secrets):

# Key Decisions (and rationale)
List the decisions that MUST be preserved, each with:
- Decision:
- Rationale:
- Alternatives considered (if any) and why rejected:

# Invariants (MUST HOLD)
List non-negotiable constraints that must remain true in next tasks.
Each invariant must be testable/verifiable.

# Prohibited Changes (DO NOT DO)
List actions that would break assumptions, expand scope, or introduce risk.
Be explicit (e.g.,): "Do NOT change public API X", "Do NOT alter schema Y", "Do NOT refactor module Z".

# What Changed in the current task
Be concrete and verifiable:
- New/modified files (paths):
- New/changed public interfaces (signatures, endpoints, CLI options):
- Behavior changes:
- Tests added/updated:
- Migrations/config changes:

# Verification (Build & Tests)
- Provide instructions to run tests or builds.
- Explicitly state which test command you executed.
- Report the result of the execution (pass/fail and scope).
- If tests were not executed, provide a concrete technical reason that proves why you could not run them.

# Timeout & Execution Safety
- State explicitly whether a timeout mechanism was used and what timeout value was applied.

# Debug / Temporary Instrumentation
- If debug logs were added temporarily, explain:
  - What was logged.
  - How it helped identify the root cause.
  - Whether the logs were removed or reduced afterward.

# Guardrails & Interim Report
- If you stopped due to the retry/time guardrails, explicitly label the output as an interim report and include:
  - The exact command(s) run (including timeout mechanism and values)
  - The failure patterns observed (with the most informative log snippets)
  - The concrete next step you recommend and why
  - Whether the next step requires user confirmation or environment changes

# Known Issues / Technical Debt
List any intentional shortcuts, open bugs, flaky tests, or follow-ups created by the current task.
Include how to reproduce and current status.

# Unfinished Work / Continuation Plan
Use this section when the task was NOT completed (either partially done, blocked, or paused). If the task is fully completed, write `NONE`.

Include if the task is incomplete:
- Remaining work items (must be concrete, ordered, and checkable).
- How to continue safely (exact next commands / files / entry points).
- What is currently blocked and why, with the minimum info needed to unblock.
- Guardrails and pitfalls to avoid (things that could silently regress behavior or waste time).

# Git Commit
Git commit created during this session, including the commit hash and subject line:
- `<commit_hash>`: `<subject line>`
>>>"#
}

const CODING_USER_PROMPT_TEMPLATE: &str = r#"Based on the given specification and plan:
- You MUST implement the assigned task by writing code changes in the workspace.
- Do NOT implement any task that is not explicitly assigned to you.

Output MUST be valid JSON conforming to the provided JSON Schema.

---

Assigned task:
<<<
Task ID: {{TASK_ID}}
Task Title: {{TASK_TITLE}}
Task Description:
{{TASK_DESCRIPTION}}
>>>

You MUST read following files for context before writing code:
- Specification:
  - {{SPEC_PATH}}
- Plan:
  - {{PLAN_PATH}}
- Implementation reports for upstream tasks (if available):
  - {{UPSTREAM_REPORT_PATHS}}

---

Worktree context:
- Integration Branch: {{INTEGRATION_BRANCH}}"#;

pub fn build_coding_task_prompt(
    task: &CodingTask,
    spec_path: &Path,
    plan_path: &Path,
    upstream_report_paths: &[PathBuf],
    integration_branch: &str,
) -> String {
    let upstream_section = if upstream_report_paths.is_empty() {
        "  - N/A".to_string()
    } else {
        upstream_report_paths
            .iter()
            .map(|p| format!("  - {}", p.display()))
            .collect::<Vec<_>>()
            .join("\n")
    };

    CODING_USER_PROMPT_TEMPLATE
        .replace("{{TASK_ID}}", &task.task_id)
        .replace("{{TASK_TITLE}}", &task.title)
        .replace("{{TASK_DESCRIPTION}}", &task.description)
        .replace("{{SPEC_PATH}}", &spec_path.display().to_string())
        .replace("{{PLAN_PATH}}", &plan_path.display().to_string())
        .replace("{{UPSTREAM_REPORT_PATHS}}", &upstream_section)
        .replace("{{INTEGRATION_BRANCH}}", integration_branch)
}

// ---------------------------------------------------------------------------
// Prompts – Conflict Resolution
// ---------------------------------------------------------------------------

const CONFLICT_RESOLUTION_PROMPT_TEMPLATE: &str = r#"# Rebase Conflict Resolution Prompt (commit-first, root-cause driven)

A rebase onto the integration branch has produced merge conflicts that you must resolve.

Integration branch: {{INTEGRATION_BRANCH}}
Task ID: {{TASK_ID}}

Conflicted files:
{{CONFLICTED_FILES}}

Hard requirement (do this before editing any file):
You MUST examine the relevant commits on BOTH sides (integration branch changes and this task's changes) and use that evidence to determine the root cause of the conflicts. Do NOT start by manually editing conflicted files.

Required Git investigation (run in this worktree):
1) Capture the exact rebase state and base:
   - `git status`
   - `git rev-parse --abbrev-ref HEAD`
   - `git rev-parse --short HEAD`
   - `git branch --show-current`

2) Identify what changed on the integration branch since this task branch diverged:
   - Find merge-base: `git merge-base HEAD {{INTEGRATION_BRANCH}}`
   - List integration commits not in this branch:
     `git log --oneline --decorate --no-merges --reverse <MERGE_BASE>..{{INTEGRATION_BRANCH}}`

3) Identify what this task changed relative to the same merge-base:
   - `git log --oneline --decorate --no-merges --reverse <MERGE_BASE>..HEAD`
   - For conflicted files, show per-file history and diffs on BOTH sides:
     - `git log --oneline --follow -- <FILE>`
     - `git diff <MERGE_BASE>..{{INTEGRATION_BRANCH}} -- <FILE>`
     - `git diff <MERGE_BASE>..HEAD -- <FILE>`

4) For the current conflict, inspect the three-way versions for EACH conflicted file:
   - Base:   `git show :1:<FILE>` (if available)
   - Ours:   `git show :2:<FILE>`
   - Theirs: `git show :3:<FILE>`
   - Optional: `git diff --ours -- <FILE>` and `git diff --theirs -- <FILE>`

Resolution rules (apply after the investigation):
A) Use the commit comparison to state the root cause for each conflicted file:
   - Which integration commit(s) touched the same lines/structures?
   - Which task commit(s) touched the same lines/structures?
   - What semantic intent do those commits appear to have?

B) Resolve conflicts by preserving BOTH intents whenever possible:
   - Prefer minimal edits that reconcile behavior, not just "make it compile".
   - If integration introduced an API change, migrate this task's code to the new API.
   - If both sides made independent improvements, combine them unless they are mutually exclusive.

C) Only after finishing the conflict edits:
   1. Stage resolved files: `git add <FILE>` for each file
   2. Continue rebase: `git rebase --continue`
   3. If more conflicts occur, repeat the same process.

Failure rule:
If you determine a correct resolution is not possible without violating the task's specification or causing regressions, abort the rebase:
- `git rebase --abort`
Then report failure with the precise reason and which commits caused irreconcilable intent.

Output requirements:
- Output MUST be valid JSON conforming to the provided JSON Schema."#;

pub fn build_conflict_resolution_prompt(
    task_id: &str,
    integration_branch: &str,
    conflicted_files: &[String],
) -> String {
    let files_section = conflicted_files
        .iter()
        .map(|f| format!("  - {}", f))
        .collect::<Vec<_>>()
        .join("\n");

    CONFLICT_RESOLUTION_PROMPT_TEMPLATE
        .replace("{{INTEGRATION_BRANCH}}", integration_branch)
        .replace("{{TASK_ID}}", task_id)
        .replace("{{CONFLICTED_FILES}}", &files_section)
}

// ---------------------------------------------------------------------------
// Prompts – Build/Test Repair
// ---------------------------------------------------------------------------

const BUILD_TEST_REPAIR_PROMPT_TEMPLATE: &str = r#"# Build/Test Failure Resolution Prompt (commit-first, regression-aware)

After rebasing onto the integration branch, the build or tests failed for task {{TASK_ID}}.

Build command: {{BUILD_COMMAND}}
Test command: {{TEST_COMMAND}}

Error output:
{{ERROR_OUTPUT}}

Hard requirement (do this before changing code):
You MUST determine whether the failure is caused by (a) integration branch changes, (b) this task's changes, or (c) an interaction between them. Do NOT start by patching files directly based only on the error text.

Required Git + diagnosis workflow:
1) Record environment and current revision:
   - `git status`
   - `git rev-parse --short HEAD`
   - `git log -1 --oneline --decorate`

2) Identify the commit ranges to compare:
   - `git merge-base HEAD {{INTEGRATION_BRANCH}}` (if {{INTEGRATION_BRANCH}} is still available locally; otherwise use the recorded pre-rebase base)
   - Integration delta (what recently landed that might affect this task):
     `git log --oneline --decorate --no-merges --reverse <MERGE_BASE>..{{INTEGRATION_BRANCH}}`
   - Task delta (what this task introduced):
     `git log --oneline --decorate --no-merges --reverse <MERGE_BASE>..HEAD`

3) Map the failure to likely areas:
   - If the error mentions a symbol/file/package/module, locate where it was changed:
     - `git log --oneline --follow -- <SUSPECT_FILE>`
     - `git blame <SUSPECT_FILE>` around failing lines
   - If the failure is test-related, identify the specific failing tests and their ownership:
     - Run the test command in a way that reveals the failing test names (adjust flags as appropriate).
   - If build-related, capture the FIRST error (not the cascade) and identify the compilation unit.

4) Perform a "cause isolation" check using commits (pick the smallest applicable approach):
   - If feasible, run build/tests at:
     A) the merge-base (`git checkout <MERGE_BASE>`) to see if it was already failing,
     B) the integration branch head,
     C) the task head,
     and compare results (return to task head afterwards).
   - If checking out is too disruptive, use:
     - `git show <COMMIT>:<FILE>` to compare before/after for suspect files.

Fix rules:
A) State a root cause hypothesis backed by commit evidence:
   - Which integration commit(s) introduced an incompatible API/behavior change?
   - Which task commit(s) rely on old assumptions?
   - Is it a deterministic failure (always) or flaky (intermittent)?

B) Apply the smallest correct fix in this task worktree:
   - Prefer adapting this task to integration's new contracts (API, schema, behavior).
   - Avoid sweeping refactors unrelated to the failure.
   - If the correct fix belongs in the integration branch (pre-existing bug), still implement the minimal fix here only if it is safe and consistent with the integration direction; otherwise report that the upstream fix is required.

C) Verify:
   1. Run `{{BUILD_COMMAND}}` and confirm success.
   2. Run `{{TEST_COMMAND}}` and confirm all tests pass.
   3. If you changed behavior, add/adjust the minimal test that proves the intended behavior (only if necessary and within the task scope).

Failure rule:
If you cannot fix the issue without changing requirements or introducing a risky cross-cutting change, report failure with:
- the suspected offending commits,
- why the failure is not safely fixable here,
- what upstream or spec decision is needed.

Output requirements:
- Output MUST be valid JSON conforming to the provided JSON Schema."#;

pub fn build_build_test_repair_prompt(
    task_id: &str,
    build_command: &str,
    test_command: &str,
    error_output: &str,
) -> String {
    BUILD_TEST_REPAIR_PROMPT_TEMPLATE
        .replace("{{TASK_ID}}", task_id)
        .replace("{{BUILD_COMMAND}}", build_command)
        .replace("{{TEST_COMMAND}}", test_command)
        .replace("{{ERROR_OUTPUT}}", error_output)
}

// ---------------------------------------------------------------------------
// Prompts – Review Agent
// ---------------------------------------------------------------------------

pub fn review_agent_system_prompt() -> &'static str {
    r#"# Role

You are the **code review** assistant. You SHOULD review the implementation against that the implementation plan and the specification.

The specification MUST be treated as the canonical source of requirements and constraints if provided in order to guide the review process.

**Core rules:**
- Do NOT implement features or rewrite code. Review and critique only.
- Compare the plan and the actual changes line by line where relevant.
- You MUST verify that the implementation adheres to the specification; plan compliance does not imply spec compliance.
- Be precise and concrete. Avoid vague feedback.

---

# Review process

You MUST review the implementation against the plan and the specification by checking the following aspects:

- Verify plan adherence:
  - Are all planned steps implemented?
  - Are there unplanned changes or scope creep?
  - Are any steps partially implemented or missing?

- Verify specification compliance:
  - Does the implementation meet all requirements stated in the specification?
  - Are there any deviations from the specification? If so, are they justified?

- Check correctness and robustness:
  - Logic correctness.
  - Error handling and edge cases.
  - Consistency with existing patterns.

- Check testing strategy:
  - Are unit tests updated or added where behavior changed?
  - For integration tests, is Testcontainers used when feasible?
  - If not used, is the stated reason technically valid?

- Check maintainability:
  - Naming and structure.
  - Readability and separation of concerns.
  - No unnecessary refactoring mixed with functional changes.
  - Unused code or dead paths.

- Check risk and impact:
  - Backward compatibility.
  - Migration or rollout risks.
  - Security or performance concerns if relevant.

---

# Output language (mandatory)

Your default output language MUST be English.

---

# Verdict criteria (mandatory)

**You MUST produce one of the following verdicts based on your review:**
- `APPROVED`: the implementation is sound, complete, and meets all requirements.
- `REQUEST_CHANGES`: the implementation has issues that must be addressed before approval.

---

# Output

When you finish you MUST produce an output as follows.
Output MUST be valid JSON conforming to the provided JSON Schema.

## Review result
`APPROVED` or `REQUEST_CHANGES`

## Review comment (Markdown)
```markdown
# Review Result
- State whether the implementation is `APPROVED` or `REQUEST_CHANGES`.

# Summary
- High-level summary.

# Findings
- Incorrect parts in the implementation.
- Bugs or logical errors.
- Risky assumptions.
- Test gaps.

# Test Review
- Unit test coverage assessment.
- Integration test strategy assessment (Testcontainers usage).

# Recommendations
- Concrete, actionable corrections.
- Ordered by importance.
```

---

# Quality bar

- Feedback must be actionable.
- Every major claim should point to:
  - An implementation item, or
  - A specific code area, or
  - A test case.

Do NOT:
- Propose a completely new design unless the current plan is invalid.
- Implement fixes yourself.
- Expand scope beyond the plan."#
}

const INITIAL_REVIEW_PROMPT_TEMPLATE: &str = r#"# Instructions for Initial Code Review

Review the given code implementation against the provided specification and plan. Your task is to determine whether the implementation is correct, complete, and meets all requirements.

You MUST read following files before starting the review:
- Specification: {{SPEC_PATH}}
- Implementation plan: {{PLAN_PATH}}
- Implementation report: {{IMPLEMENTATION_REPORT_PATH}}
- Git commit:
  - {{GIT_COMMIT_REVISION}}

You MUST read the code changes from the provided workspace files using available tools.

Output MUST be valid JSON conforming to the provided JSON Schema."#;

pub fn build_initial_review_prompt(
    spec_path: &Path,
    plan_path: &Path,
    report_path: &Path,
    git_commit_revision: &str,
) -> String {
    INITIAL_REVIEW_PROMPT_TEMPLATE
        .replace("{{SPEC_PATH}}", &spec_path.display().to_string())
        .replace("{{PLAN_PATH}}", &plan_path.display().to_string())
        .replace("{{IMPLEMENTATION_REPORT_PATH}}", &report_path.display().to_string())
        .replace("{{GIT_COMMIT_REVISION}}", git_commit_revision)
}

const FOLLOWUP_REVIEW_PROMPT_TEMPLATE: &str = r#"# Instructions for Follow-up Code Review

You are performing a follow-up review of a code implementation that has already undergone an initial review. Your task is to determine whether the revised implementation adequately addresses the issues raised in the previous review and meets all requirements.

You MUST read following files before starting the review:
- Specification: {{SPEC_PATH}}
- Implementation plan: {{PLAN_PATH}}
- Follow-up implementation report: {{IMPLEMENTATION_REPORT_PATH}}
- Git commit for the follow-up changes:
  - {{GIT_COMMIT_REVISION}}

You MUST read the code changes from the provided workspace files using available tools.

Output MUST be valid JSON conforming to the provided JSON Schema."#;

pub fn build_followup_review_prompt(
    spec_path: &Path,
    plan_path: &Path,
    report_path: &Path,
    git_commit_revision: &str,
) -> String {
    FOLLOWUP_REVIEW_PROMPT_TEMPLATE
        .replace("{{SPEC_PATH}}", &spec_path.display().to_string())
        .replace("{{PLAN_PATH}}", &plan_path.display().to_string())
        .replace("{{IMPLEMENTATION_REPORT_PATH}}", &report_path.display().to_string())
        .replace("{{GIT_COMMIT_REVISION}}", git_commit_revision)
}

// ---------------------------------------------------------------------------
// Prompts – Coding Revision
// ---------------------------------------------------------------------------

const CODING_REVISION_PROMPT_TEMPLATE: &str = r#"The reviewer has requested changes to your implementation. You MUST address the review feedback below.

This is a **revision** request, not a new implementation. Focus specifically on the issues raised in the review.

---

Review feedback:
<<<
{{REVIEW_COMMENT}}
>>>

---

Task context:
<<<
Task ID: {{TASK_ID}}
Task Title: {{TASK_TITLE}}
>>>

You MUST read following files for context before making changes:
- Specification:
  - {{SPEC_PATH}}
- Plan:
  - {{PLAN_PATH}}

---

Worktree context:
- Integration Branch: {{INTEGRATION_BRANCH}}

Instructions:
1. Carefully read the review feedback above.
2. Address each point raised by the reviewer.
3. Make the necessary code changes.
4. Run build and tests to verify your changes.
5. Make a single commit with all changes.

Output MUST be valid JSON conforming to the provided JSON Schema."#;

pub fn build_coding_revision_prompt(
    task: &CodingTask,
    spec_path: &Path,
    plan_path: &Path,
    review_comment: &str,
    integration_branch: &str,
) -> String {
    CODING_REVISION_PROMPT_TEMPLATE
        .replace("{{TASK_ID}}", &task.task_id)
        .replace("{{TASK_TITLE}}", &task.title)
        .replace("{{SPEC_PATH}}", &spec_path.display().to_string())
        .replace("{{PLAN_PATH}}", &plan_path.display().to_string())
        .replace("{{REVIEW_COMMENT}}", review_comment)
        .replace("{{INTEGRATION_BRANCH}}", integration_branch)
}

// ---------------------------------------------------------------------------
// Git Operations
// ---------------------------------------------------------------------------

pub fn create_integration_branch(
    workspace: &Path,
    session_name: &str,
) -> Result<String, String> {
    let branch_name = format!("bear/integration/{}-{}", session_name, Uuid::new_v4());

    let output = Command::new("git")
        .current_dir(workspace)
        .args(["checkout", "-b", &branch_name])
        .output()
        .map_err(|e| format!("failed to execute git checkout -b: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("failed to create integration branch: {}", stderr.trim()));
    }

    Ok(branch_name)
}

pub fn create_worktree(
    workspace: &Path,
    integration_branch: &str,
) -> Result<PathBuf, String> {
    let workspace_dir_name = workspace
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("workspace");

    let worktree_path = workspace
        .parent()
        .unwrap_or(workspace)
        .join(format!("{}-bear-worktree-{}", workspace_dir_name, Uuid::new_v4()));

    let output = Command::new("git")
        .current_dir(workspace)
        .args([
            "worktree",
            "add",
            &worktree_path.display().to_string(),
            integration_branch,
        ])
        .output()
        .map_err(|e| format!("failed to execute git worktree add: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("failed to create worktree: {}", stderr.trim()));
    }

    Ok(worktree_path)
}

pub fn remove_worktree(
    workspace: &Path,
    worktree_path: &Path,
) -> Result<(), String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args([
            "worktree",
            "remove",
            "--force",
            &worktree_path.display().to_string(),
        ])
        .output()
        .map_err(|e| format!("failed to execute git worktree remove: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("failed to remove worktree: {}", stderr.trim()));
    }

    Ok(())
}

pub fn create_task_branch(
    workspace: &Path,
    integration_branch: &str,
    task_id: &str,
) -> Result<String, String> {
    let branch_name = format!("bear/task/{}-{}", task_id, Uuid::new_v4());

    let output = Command::new("git")
        .current_dir(workspace)
        .args(["branch", &branch_name, integration_branch])
        .output()
        .map_err(|e| format!("failed to execute git branch: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("failed to create task branch: {}", stderr.trim()));
    }

    Ok(branch_name)
}

pub fn rebase_onto_integration(
    worktree_path: &Path,
    integration_branch: &str,
) -> Result<RebaseOutcome, String> {
    let output = Command::new("git")
        .current_dir(worktree_path)
        .args(["rebase", integration_branch])
        .output()
        .map_err(|e| format!("failed to execute git rebase: {}", e))?;

    if output.status.success() {
        return Ok(RebaseOutcome::Success);
    }

    let stderr = String::from_utf8_lossy(&output.stderr);
    if stderr.contains("CONFLICT") || stderr.contains("could not apply") {
        let conflicted_files = list_conflicted_files(worktree_path)?;
        return Ok(RebaseOutcome::Conflict { conflicted_files });
    }

    Err(format!("git rebase failed: {}", stderr.trim()))
}

pub fn list_conflicted_files(
    worktree_path: &Path,
) -> Result<Vec<String>, String> {
    let output = Command::new("git")
        .current_dir(worktree_path)
        .args(["diff", "--name-only", "--diff-filter=U"])
        .output()
        .map_err(|e| format!("failed to execute git diff: {}", e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let files: Vec<String> = stdout
        .lines()
        .filter(|line| !line.is_empty())
        .map(String::from)
        .collect();

    Ok(files)
}

pub fn abort_rebase(worktree_path: &Path) -> Result<(), String> {
    let output = Command::new("git")
        .current_dir(worktree_path)
        .args(["rebase", "--abort"])
        .output()
        .map_err(|e| format!("failed to execute git rebase --abort: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("failed to abort rebase: {}", stderr.trim()));
    }

    Ok(())
}

pub fn detect_build_commands(worktree_path: &Path) -> Option<BuildTestCommands> {
    let makefile_path = worktree_path.join("Makefile");
    if makefile_path.exists()
        && let Ok(content) = fs::read_to_string(&makefile_path)
    {
        let has_build = content.lines().any(|line| line.starts_with("build:"));
        let has_test = content.lines().any(|line| line.starts_with("test:"));
        if has_build && has_test {
            return Some(BuildTestCommands {
                build: "make build".to_string(),
                test: "make test".to_string(),
            });
        }
    }

    if worktree_path.join("Cargo.toml").exists() {
        return Some(BuildTestCommands {
            build: "cargo build".to_string(),
            test: "cargo test".to_string(),
        });
    }

    if let Some(commands) = detect_npm_commands(worktree_path) {
        return Some(commands);
    }

    if worktree_path.join("go.mod").exists() {
        return Some(BuildTestCommands {
            build: "go build ./...".to_string(),
            test: "go test ./...".to_string(),
        });
    }

    None
}

fn detect_npm_commands(worktree_path: &Path) -> Option<BuildTestCommands> {
    let package_json_path = worktree_path.join("package.json");
    let content = fs::read_to_string(&package_json_path).ok()?;
    let parsed: serde_json::Value = serde_json::from_str(&content).ok()?;
    let scripts = parsed.get("scripts")?;

    let has_build = scripts.get("build").is_some();
    let has_test = scripts.get("test").is_some();

    if has_build && has_test {
        Some(BuildTestCommands {
            build: "npm run build".to_string(),
            test: "npm test".to_string(),
        })
    } else {
        None
    }
}

pub fn run_build_and_test(
    worktree_path: &Path,
    commands: &BuildTestCommands,
) -> Result<BuildTestOutcome, String> {
    let build_outcome = run_shell_command(worktree_path, &commands.build)?;
    if !build_outcome.success {
        return Ok(BuildTestOutcome::BuildFailed {
            output: build_outcome.combined_output,
        });
    }

    let test_outcome = run_shell_command(worktree_path, &commands.test)?;
    if !test_outcome.success {
        return Ok(BuildTestOutcome::TestFailed {
            output: test_outcome.combined_output,
        });
    }

    Ok(BuildTestOutcome::Success)
}

struct ShellCommandResult {
    success: bool,
    combined_output: String,
}

fn run_shell_command(
    working_dir: &Path,
    command: &str,
) -> Result<ShellCommandResult, String> {
    let output = Command::new("timeout")
        .current_dir(working_dir)
        .args(["--signal=TERM", "--kill-after=15s", "180s", "sh", "-c", command])
        .output()
        .map_err(|e| format!("failed to execute '{}': {}", command, e))?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    let combined_output = format!("--- stdout ---\n{}\n--- stderr ---\n{}", stdout, stderr);

    Ok(ShellCommandResult {
        success: output.status.success(),
        combined_output,
    })
}

pub fn fast_forward_merge_task_branch(
    workspace: &Path,
    task_branch: &str,
) -> Result<(), String> {
    let merge_output = Command::new("git")
        .current_dir(workspace)
        .args(["merge", "--ff-only", task_branch])
        .output()
        .map_err(|e| format!("failed to execute git merge --ff-only: {}", e))?;

    if !merge_output.status.success() {
        let stderr = String::from_utf8_lossy(&merge_output.stderr);
        return Err(format!("failed to fast-forward merge: {}", stderr.trim()));
    }

    Ok(())
}

pub fn delete_branch(
    workspace: &Path,
    branch_name: &str,
) -> Result<(), String> {
    let output = Command::new("git")
        .current_dir(workspace)
        .args(["branch", "-D", branch_name])
        .output()
        .map_err(|e| format!("failed to execute git branch -D: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("failed to delete branch: {}", stderr.trim()));
    }

    Ok(())
}

pub fn get_latest_commit_revision(worktree_path: &Path) -> Result<String, String> {
    let output = Command::new("git")
        .current_dir(worktree_path)
        .args(["rev-parse", "HEAD"])
        .output()
        .map_err(|e| format!("failed to execute git rev-parse: {}", e))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!("failed to get latest commit: {}", stderr.trim()));
    }

    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

// ---------------------------------------------------------------------------
// Report Management
// ---------------------------------------------------------------------------

pub fn copy_artifacts_to_worktree(
    source_dir: &Path,
    target_dir: &Path,
    file_names: &[&str],
) -> Vec<String> {
    let mut errors = Vec::new();
    if let Err(err) = fs::create_dir_all(target_dir) {
        errors.push(format!("디렉토리 생성 실패: {}", err));
        return errors;
    }
    for name in file_names {
        let src = source_dir.join(name);
        if src.exists()
            && let Err(err) = fs::copy(&src, target_dir.join(name))
        {
            errors.push(format!("{} 복사 실패: {}", name, err));
        }
    }
    errors
}

pub fn save_task_report(
    dir: &Path,
    task_id: &str,
    report: &str,
) -> io::Result<PathBuf> {
    fs::create_dir_all(dir)?;

    let file_path = dir.join(format!("{}.md", task_id));
    fs::write(&file_path, report)?;

    Ok(file_path)
}

pub fn collect_upstream_report_paths(
    task: &CodingTask,
    completed_reports: &[TaskReport],
) -> Vec<PathBuf> {
    task.dependencies
        .iter()
        .filter_map(|dep_id| {
            completed_reports
                .iter()
                .find(|r| &r.task_id == dep_id)
                .map(|r| r.report_file_path.clone())
        })
        .collect()
}

pub fn save_and_commit_task_report_in_worktree(
    worktree_path: &Path,
    date_dir: &str,
    session_name: &str,
    task_id: &str,
    report: &str,
) -> Result<PathBuf, String> {
    let report_dir = worktree_path
        .join(".bear")
        .join(date_dir)
        .join(session_name);
    fs::create_dir_all(&report_dir)
        .map_err(|e| format!("failed to create report directory: {}", e))?;

    let file_path = report_dir.join(format!("{}.md", task_id));
    fs::write(&file_path, report)
        .map_err(|e| format!("failed to write report file: {}", e))?;

    let add_output = Command::new("git")
        .current_dir(worktree_path)
        .args(["add", &file_path.display().to_string()])
        .output()
        .map_err(|e| format!("failed to git add report: {}", e))?;

    if !add_output.status.success() {
        let stderr = String::from_utf8_lossy(&add_output.stderr);
        return Err(format!("failed to git add report: {}", stderr.trim()));
    }

    let commit_message = format!("Add implementation report for {}", task_id);
    let commit_output = Command::new("git")
        .current_dir(worktree_path)
        .args(["commit", "-m", &commit_message])
        .output()
        .map_err(|e| format!("failed to git commit report: {}", e))?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        return Err(format!("failed to commit report: {}", stderr.trim()));
    }

    Ok(file_path)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn task_extraction_schema_is_valid_json() {
        let schema = task_extraction_schema();
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["tasks"].is_object());

        let item_props = &schema["properties"]["tasks"]["items"]["properties"];
        assert!(item_props["task_id"].is_object());
        assert!(item_props["title"].is_object());
        assert!(item_props["description"].is_object());
        assert!(item_props["dependencies"].is_object());
    }

    #[test]
    fn coding_task_result_schema_is_valid_json() {
        let schema = coding_task_result_schema();
        assert_eq!(schema["type"], "object");

        let status_enum = schema["properties"]["status"]["enum"]
            .as_array()
            .unwrap();
        assert!(status_enum.iter().any(|v| v == "IMPLEMENTATION_SUCCESS"));
        assert!(status_enum.iter().any(|v| v == "IMPLEMENTATION_BLOCKED"));
        assert!(schema["properties"]["report"].is_object());
    }

    #[test]
    fn deserialize_task_extraction_response() {
        let json = serde_json::json!({
            "tasks": [
                {
                    "task_id": "TASK-00",
                    "title": "기본 타입 정의",
                    "description": "핵심 타입들을 정의합니다.",
                    "dependencies": []
                },
                {
                    "task_id": "TASK-01",
                    "title": "비즈니스 로직 구현",
                    "description": "핵심 로직을 구현합니다.",
                    "dependencies": ["TASK-00"]
                }
            ]
        });

        let response: TaskExtractionResponse = serde_json::from_value(json).unwrap();

        assert_eq!(response.tasks.len(), 2);
        assert_eq!(response.tasks[0].task_id, "TASK-00");
        assert!(response.tasks[0].dependencies.is_empty());
        assert_eq!(response.tasks[1].dependencies, vec!["TASK-00"]);
    }

    #[test]
    fn deserialize_coding_task_result_success() {
        let json = serde_json::json!({
            "status": "IMPLEMENTATION_SUCCESS",
            "report": "# Metadata\n구현 완료"
        });

        let result: CodingTaskResult = serde_json::from_value(json).unwrap();

        assert_eq!(result.status, CodingTaskStatus::ImplementationSuccess);
        assert!(result.report.contains("구현 완료"));
    }

    #[test]
    fn deserialize_coding_task_result_blocked() {
        let json = serde_json::json!({
            "status": "IMPLEMENTATION_BLOCKED",
            "report": "# Metadata\n테스트 실패로 차단됨"
        });

        let result: CodingTaskResult = serde_json::from_value(json).unwrap();

        assert_eq!(result.status, CodingTaskStatus::ImplementationBlocked);
    }

    #[test]
    fn task_extraction_prompt_contains_plan_path() {
        let plan_path = Path::new("/workspace/.bear/20260215/session/plan.md");
        let prompt = build_task_extraction_prompt(plan_path);

        assert!(prompt.contains(&plan_path.display().to_string()));
        assert!(prompt.contains("topological order"));
    }

    #[test]
    fn coding_task_prompt_contains_all_fields() {
        let task = CodingTask {
            task_id: "TASK-00".to_string(),
            title: "기본 타입 정의".to_string(),
            description: "핵심 타입을 정의합니다.".to_string(),
            dependencies: vec!["TASK-01".to_string()],
        };

        let spec_path = Path::new("/workspace/.bear/20260215/session/spec.md");
        let plan_path = Path::new("/workspace/.bear/20260215/session/plan.md");
        let upstream_paths = vec![PathBuf::from("/workspace/.bear/20260215/session/TASK-01.md")];

        let integration_branch = "bear/integration/test-session-abc123";
        let prompt = build_coding_task_prompt(
            &task,
            spec_path,
            plan_path,
            &upstream_paths,
            integration_branch,
        );

        assert!(prompt.contains("TASK-00"));
        assert!(prompt.contains("기본 타입 정의"));
        assert!(prompt.contains("핵심 타입을 정의합니다."));
        assert!(prompt.contains(&spec_path.display().to_string()));
        assert!(prompt.contains(&plan_path.display().to_string()));
        assert!(prompt.contains("TASK-01.md"));
        assert!(prompt.contains(integration_branch));
    }

    #[test]
    fn coding_task_prompt_without_upstream_report() {
        let task = CodingTask {
            task_id: "TASK-00".to_string(),
            title: "독립 작업".to_string(),
            description: "의존성 없는 작업".to_string(),
            dependencies: vec![],
        };

        let spec_path = Path::new("/workspace/.bear/spec.md");
        let plan_path = Path::new("/workspace/.bear/plan.md");
        let prompt =
            build_coding_task_prompt(&task, spec_path, plan_path, &[], "bear/integration/test");

        assert!(prompt.contains("N/A"));
    }

    #[test]
    fn save_and_read_task_report() {
        let temp_dir = TempDir::new().unwrap();
        let report_content = "# Metadata\n구현 완료";

        let path = save_task_report(temp_dir.path(), "TASK-00", report_content).unwrap();

        let expected = temp_dir.path().join("TASK-00.md");
        assert_eq!(path, expected);

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, report_content);
    }

    #[test]
    fn collect_upstream_report_paths_with_dependencies() {
        let task = CodingTask {
            task_id: "TASK-02".to_string(),
            title: "후속 작업".to_string(),
            description: "TASK-00, TASK-01에 의존".to_string(),
            dependencies: vec!["TASK-00".to_string(), "TASK-01".to_string()],
        };

        let reports = vec![
            TaskReport {
                task_id: "TASK-00".to_string(),
                status: CodingTaskStatus::ImplementationSuccess,
                report: "TASK-00 완료".to_string(),
                report_file_path: PathBuf::from("/tmp/TASK-00.md"),
            },
            TaskReport {
                task_id: "TASK-01".to_string(),
                status: CodingTaskStatus::ImplementationSuccess,
                report: "TASK-01 완료".to_string(),
                report_file_path: PathBuf::from("/tmp/TASK-01.md"),
            },
        ];

        let paths = collect_upstream_report_paths(&task, &reports);

        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0], PathBuf::from("/tmp/TASK-00.md"));
        assert_eq!(paths[1], PathBuf::from("/tmp/TASK-01.md"));
    }

    #[test]
    fn collect_upstream_report_paths_without_dependencies() {
        let task = CodingTask {
            task_id: "TASK-00".to_string(),
            title: "독립 작업".to_string(),
            description: "의존성 없음".to_string(),
            dependencies: vec![],
        };

        let paths = collect_upstream_report_paths(&task, &[]);

        assert!(paths.is_empty());
    }

    // -----------------------------------------------------------------------
    // Git operation tests
    // -----------------------------------------------------------------------

    fn init_git_repo(dir: &Path) {
        Command::new("git")
            .current_dir(dir)
            .args(["init"])
            .output()
            .unwrap();
        Command::new("git")
            .current_dir(dir)
            .args(["config", "user.email", "test@test.com"])
            .output()
            .unwrap();
        Command::new("git")
            .current_dir(dir)
            .args(["config", "user.name", "Test"])
            .output()
            .unwrap();
    }

    fn make_commit(dir: &Path, filename: &str, content: &str, message: &str) {
        fs::write(dir.join(filename), content).unwrap();
        Command::new("git")
            .current_dir(dir)
            .args(["add", filename])
            .output()
            .unwrap();
        Command::new("git")
            .current_dir(dir)
            .args(["commit", "-m", message])
            .output()
            .unwrap();
    }

    #[test]
    fn create_task_branch_from_integration() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        init_git_repo(workspace);
        make_commit(workspace, "init.txt", "init", "initial commit");

        let integration = create_integration_branch(workspace, "test-session").unwrap();
        let task_branch = create_task_branch(workspace, &integration, "TASK-00").unwrap();

        assert!(task_branch.starts_with("bear/task/TASK-00-"));

        let output = Command::new("git")
            .current_dir(workspace)
            .args(["branch", "--list", &task_branch])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(!stdout.trim().is_empty());
    }

    #[test]
    fn rebase_onto_integration_success() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        init_git_repo(workspace);
        make_commit(workspace, "init.txt", "init", "initial commit");

        let integration = create_integration_branch(workspace, "test").unwrap();
        let task_branch = create_task_branch(workspace, &integration, "TASK-00").unwrap();
        let worktree_path = create_worktree(workspace, &task_branch).unwrap();
        make_commit(&worktree_path, "task.txt", "task content", "task commit");

        let result = rebase_onto_integration(&worktree_path, &integration).unwrap();

        assert!(matches!(result, RebaseOutcome::Success));

        remove_worktree(workspace, &worktree_path).unwrap();
    }

    #[test]
    fn rebase_onto_integration_conflict() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        init_git_repo(workspace);
        make_commit(workspace, "shared.txt", "original", "initial commit");

        let integration = create_integration_branch(workspace, "test").unwrap();
        let task_branch = create_task_branch(workspace, &integration, "TASK-00").unwrap();
        let worktree_path = create_worktree(workspace, &task_branch).unwrap();

        // 통합 브랜치에서 같은 파일 수정 (메인 워크스페이스에서 체크아웃해서 커밋)
        Command::new("git")
            .current_dir(workspace)
            .args(["checkout", &integration])
            .output()
            .unwrap();
        make_commit(workspace, "shared.txt", "integration change", "integration commit");
        Command::new("git")
            .current_dir(workspace)
            .args(["checkout", "main"])
            .output()
            .unwrap();

        // 태스크 브랜치에서 같은 파일을 다르게 수정
        make_commit(&worktree_path, "shared.txt", "task change", "task commit");

        let result = rebase_onto_integration(&worktree_path, &integration).unwrap();

        assert!(matches!(result, RebaseOutcome::Conflict { .. }));
        if let RebaseOutcome::Conflict { conflicted_files } = result {
            assert!(conflicted_files.contains(&"shared.txt".to_string()));
        }

        abort_rebase(&worktree_path).unwrap();
        remove_worktree(workspace, &worktree_path).unwrap();
    }

    #[test]
    fn abort_rebase_restores_clean_state() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        init_git_repo(workspace);
        make_commit(workspace, "shared.txt", "original", "initial commit");

        let integration = create_integration_branch(workspace, "test").unwrap();
        let task_branch = create_task_branch(workspace, &integration, "TASK-00").unwrap();
        let worktree_path = create_worktree(workspace, &task_branch).unwrap();

        Command::new("git")
            .current_dir(workspace)
            .args(["checkout", &integration])
            .output()
            .unwrap();
        make_commit(workspace, "shared.txt", "integration", "integration commit");
        Command::new("git")
            .current_dir(workspace)
            .args(["checkout", "main"])
            .output()
            .unwrap();

        make_commit(&worktree_path, "shared.txt", "task", "task commit");
        rebase_onto_integration(&worktree_path, &integration).unwrap();
        abort_rebase(&worktree_path).unwrap();

        // 리베이스 중단 후 정상 상태 확인
        let status = Command::new("git")
            .current_dir(&worktree_path)
            .args(["status", "--porcelain"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&status.stdout);
        assert!(stdout.trim().is_empty());

        remove_worktree(workspace, &worktree_path).unwrap();
    }

    #[test]
    fn fast_forward_merge_task_branch_success() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        init_git_repo(workspace);
        make_commit(workspace, "init.txt", "init", "initial commit");

        let integration = create_integration_branch(workspace, "test").unwrap();
        let task_branch = create_task_branch(workspace, &integration, "TASK-00").unwrap();
        let worktree_path = create_worktree(workspace, &task_branch).unwrap();

        make_commit(&worktree_path, "feature.txt", "feature", "feature commit");
        make_commit(&worktree_path, "feature2.txt", "feature2", "feature2 commit");

        rebase_onto_integration(&worktree_path, &integration).unwrap();

        fast_forward_merge_task_branch(
            workspace,
            &task_branch,
        )
        .unwrap();

        // fast-forward 머지 후 태스크 브랜치의 커밋들이 그대로 통합 브랜치에 존재하는지 확인
        let log_output = Command::new("git")
            .current_dir(workspace)
            .args(["log", "--oneline", &format!("{}..HEAD", "main")])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&log_output.stdout);
        let commit_lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(commit_lines.len(), 2);
        assert!(commit_lines[0].contains("feature2 commit"));
        assert!(commit_lines[1].contains("feature commit"));

        remove_worktree(workspace, &worktree_path).unwrap();
    }

    #[test]
    fn delete_branch_removes_branch() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        init_git_repo(workspace);
        make_commit(workspace, "init.txt", "init", "initial commit");

        let integration = create_integration_branch(workspace, "test").unwrap();
        let task_branch = create_task_branch(workspace, &integration, "TASK-00").unwrap();

        delete_branch(workspace, &task_branch).unwrap();

        let output = Command::new("git")
            .current_dir(workspace)
            .args(["branch", "--list", &task_branch])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(stdout.trim().is_empty());
    }

    #[test]
    fn list_conflicted_files_returns_expected() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        init_git_repo(workspace);
        make_commit(workspace, "shared.txt", "original", "initial commit");

        let integration = create_integration_branch(workspace, "test").unwrap();
        let task_branch = create_task_branch(workspace, &integration, "TASK-00").unwrap();
        let worktree_path = create_worktree(workspace, &task_branch).unwrap();

        Command::new("git")
            .current_dir(workspace)
            .args(["checkout", &integration])
            .output()
            .unwrap();
        make_commit(workspace, "shared.txt", "integration", "integration commit");
        Command::new("git")
            .current_dir(workspace)
            .args(["checkout", "main"])
            .output()
            .unwrap();

        make_commit(&worktree_path, "shared.txt", "task", "task commit");
        rebase_onto_integration(&worktree_path, &integration).unwrap();

        let files = list_conflicted_files(&worktree_path).unwrap();
        assert_eq!(files, vec!["shared.txt"]);

        abort_rebase(&worktree_path).unwrap();
        remove_worktree(workspace, &worktree_path).unwrap();
    }

    // -----------------------------------------------------------------------
    // Conflict resolution schema / prompt / deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn conflict_resolution_result_schema_is_valid_json() {
        let schema = conflict_resolution_result_schema();
        assert_eq!(schema["type"], "object");

        let status_enum = schema["properties"]["status"]["enum"]
            .as_array()
            .unwrap();
        assert!(status_enum.iter().any(|v| v == "CONFLICT_RESOLVED"));
        assert!(status_enum
            .iter()
            .any(|v| v == "CONFLICT_RESOLUTION_FAILED"));
        assert!(schema["properties"]["report"].is_object());
    }

    #[test]
    fn deserialize_conflict_resolution_result_resolved() {
        let json = serde_json::json!({
            "status": "CONFLICT_RESOLVED",
            "report": "충돌 해결 완료"
        });

        let result: ConflictResolutionResult = serde_json::from_value(json).unwrap();
        assert_eq!(result.status, ConflictResolutionStatus::ConflictResolved);
        assert!(result.report.contains("충돌 해결 완료"));
    }

    #[test]
    fn deserialize_conflict_resolution_result_failed() {
        let json = serde_json::json!({
            "status": "CONFLICT_RESOLUTION_FAILED",
            "report": "충돌 해결 실패"
        });

        let result: ConflictResolutionResult = serde_json::from_value(json).unwrap();
        assert_eq!(
            result.status,
            ConflictResolutionStatus::ConflictResolutionFailed
        );
    }

    #[test]
    fn conflict_resolution_prompt_contains_all_fields() {
        let prompt = build_conflict_resolution_prompt(
            "TASK-01",
            "bear/integration/test-abc",
            &["src/main.rs".to_string(), "src/lib.rs".to_string()],
        );

        assert!(prompt.contains("TASK-01"));
        assert!(prompt.contains("bear/integration/test-abc"));
        assert!(prompt.contains("src/main.rs"));
        assert!(prompt.contains("src/lib.rs"));
        assert!(prompt.contains("git rebase --continue"));
    }

    // -----------------------------------------------------------------------
    // Build system detection tests
    // -----------------------------------------------------------------------

    #[test]
    fn detect_build_commands_with_makefile() {
        let temp_dir = TempDir::new().unwrap();
        let makefile_content = "build:\n\tcargo build\n\ntest:\n\tcargo test\n";
        fs::write(temp_dir.path().join("Makefile"), makefile_content).unwrap();

        let result = detect_build_commands(temp_dir.path());
        assert!(result.is_some());
        let commands = result.unwrap();
        assert_eq!(commands.build, "make build");
        assert_eq!(commands.test, "make test");
    }

    #[test]
    fn detect_build_commands_makefile_without_targets() {
        let temp_dir = TempDir::new().unwrap();
        let makefile_content = "clean:\n\trm -rf target\n";
        fs::write(temp_dir.path().join("Makefile"), makefile_content).unwrap();

        // Makefile에 build/test 타겟이 없으면 None
        let result = detect_build_commands(temp_dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn detect_build_commands_with_cargo_toml() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(
            temp_dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\n",
        )
        .unwrap();

        let result = detect_build_commands(temp_dir.path());
        assert!(result.is_some());
        let commands = result.unwrap();
        assert_eq!(commands.build, "cargo build");
        assert_eq!(commands.test, "cargo test");
    }

    #[test]
    fn detect_build_commands_with_package_json() {
        let temp_dir = TempDir::new().unwrap();
        let package_json = serde_json::json!({
            "scripts": { "build": "tsc", "test": "jest" }
        });
        fs::write(
            temp_dir.path().join("package.json"),
            serde_json::to_string(&package_json).unwrap(),
        )
        .unwrap();

        let result = detect_build_commands(temp_dir.path());
        assert!(result.is_some());
        let commands = result.unwrap();
        assert_eq!(commands.build, "npm run build");
        assert_eq!(commands.test, "npm test");
    }

    #[test]
    fn detect_build_commands_with_go_mod() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(
            temp_dir.path().join("go.mod"),
            "module example.com/test\n",
        )
        .unwrap();

        let result = detect_build_commands(temp_dir.path());
        assert!(result.is_some());
        let commands = result.unwrap();
        assert_eq!(commands.build, "go build ./...");
        assert_eq!(commands.test, "go test ./...");
    }

    #[test]
    fn detect_build_commands_returns_none_for_empty_dir() {
        let temp_dir = TempDir::new().unwrap();
        let result = detect_build_commands(temp_dir.path());
        assert!(result.is_none());
    }

    #[test]
    fn detect_build_commands_makefile_has_priority_over_cargo() {
        let temp_dir = TempDir::new().unwrap();
        fs::write(
            temp_dir.path().join("Makefile"),
            "build:\n\tcargo build\n\ntest:\n\tcargo test\n",
        )
        .unwrap();
        fs::write(
            temp_dir.path().join("Cargo.toml"),
            "[package]\nname = \"test\"\n",
        )
        .unwrap();

        let result = detect_build_commands(temp_dir.path());
        assert!(result.is_some());
        let commands = result.unwrap();
        assert_eq!(commands.build, "make build");
        assert_eq!(commands.test, "make test");
    }

    // -----------------------------------------------------------------------
    // Build/test execution tests
    // -----------------------------------------------------------------------

    #[test]
    fn run_build_and_test_success() {
        let temp_dir = TempDir::new().unwrap();
        let commands = BuildTestCommands {
            build: "true".to_string(),
            test: "true".to_string(),
        };

        let result = run_build_and_test(temp_dir.path(), &commands).unwrap();
        assert!(matches!(result, BuildTestOutcome::Success));
    }

    #[test]
    fn run_build_and_test_build_failure() {
        let temp_dir = TempDir::new().unwrap();
        let commands = BuildTestCommands {
            build: "false".to_string(),
            test: "true".to_string(),
        };

        let result = run_build_and_test(temp_dir.path(), &commands).unwrap();
        assert!(matches!(result, BuildTestOutcome::BuildFailed { .. }));
    }

    #[test]
    fn run_build_and_test_test_failure() {
        let temp_dir = TempDir::new().unwrap();
        let commands = BuildTestCommands {
            build: "true".to_string(),
            test: "false".to_string(),
        };

        let result = run_build_and_test(temp_dir.path(), &commands).unwrap();
        assert!(matches!(result, BuildTestOutcome::TestFailed { .. }));
    }

    #[test]
    fn run_build_and_test_captures_output() {
        let temp_dir = TempDir::new().unwrap();
        let commands = BuildTestCommands {
            build: "echo build_ok && exit 1".to_string(),
            test: "true".to_string(),
        };

        let result = run_build_and_test(temp_dir.path(), &commands).unwrap();
        if let BuildTestOutcome::BuildFailed { output } = result {
            assert!(output.contains("build_ok"));
        } else {
            panic!("expected BuildFailed");
        }
    }

    // -----------------------------------------------------------------------
    // Build/test repair schema and prompt tests
    // -----------------------------------------------------------------------

    #[test]
    fn build_test_repair_result_schema_is_valid_json() {
        let schema = build_test_repair_result_schema();
        assert_eq!(schema["type"], "object");
        let status_enum = schema["properties"]["status"]["enum"]
            .as_array()
            .unwrap();
        assert!(status_enum.iter().any(|v| v == "BUILD_TEST_FIXED"));
        assert!(status_enum.iter().any(|v| v == "BUILD_TEST_FIX_FAILED"));
        assert!(schema["properties"]["report"].is_object());
    }

    #[test]
    fn build_test_repair_result_deserialization() {
        let json = serde_json::json!({
            "status": "BUILD_TEST_FIXED",
            "report": "Fixed compilation error in main.rs"
        });
        let result: BuildTestRepairResult = serde_json::from_value(json).unwrap();
        assert_eq!(result.status, BuildTestRepairStatus::Fixed);
        assert!(result.report.contains("compilation error"));

        let json_failed = serde_json::json!({
            "status": "BUILD_TEST_FIX_FAILED",
            "report": "Cannot fix"
        });
        let result_failed: BuildTestRepairResult =
            serde_json::from_value(json_failed).unwrap();
        assert_eq!(result_failed.status, BuildTestRepairStatus::FixFailed);
    }

    #[test]
    fn build_test_repair_prompt_contains_context() {
        let prompt = build_build_test_repair_prompt(
            "TASK-01",
            "make build",
            "make test",
            "error: cannot find module",
        );

        assert!(prompt.contains("TASK-01"));
        assert!(prompt.contains("make build"));
        assert!(prompt.contains("make test"));
        assert!(prompt.contains("cannot find module"));
    }

    // -----------------------------------------------------------------------
    // Review schema / prompt / deserialization tests
    // -----------------------------------------------------------------------

    #[test]
    fn review_result_schema_is_valid_json() {
        let schema = review_result_schema();
        assert_eq!(schema["type"], "object");

        let result_enum = schema["properties"]["review_result"]["enum"]
            .as_array()
            .unwrap();
        assert!(result_enum.iter().any(|v| v == "APPROVED"));
        assert!(result_enum.iter().any(|v| v == "REQUEST_CHANGES"));
        assert!(schema["properties"]["review_comment"].is_object());
    }

    #[test]
    fn deserialize_review_result_approved() {
        let json = serde_json::json!({
            "review_result": "APPROVED",
            "review_comment": "코드 품질이 좋습니다."
        });

        let result: ReviewResult = serde_json::from_value(json).unwrap();
        assert_eq!(result.review_result, ReviewStatus::Approved);
        assert!(result.review_comment.contains("코드 품질"));
    }

    #[test]
    fn deserialize_review_result_request_changes() {
        let json = serde_json::json!({
            "review_result": "REQUEST_CHANGES",
            "review_comment": "에러 핸들링이 부족합니다."
        });

        let result: ReviewResult = serde_json::from_value(json).unwrap();
        assert_eq!(result.review_result, ReviewStatus::RequestChanges);
        assert!(result.review_comment.contains("에러 핸들링"));
    }

    #[test]
    fn initial_review_prompt_contains_all_fields() {
        let prompt = build_initial_review_prompt(
            Path::new("/workspace/.bear/spec.md"),
            Path::new("/workspace/.bear/plan.md"),
            Path::new("/workspace/.bear/TASK-00.md"),
            "abc1234",
        );

        assert!(prompt.contains("spec.md"));
        assert!(prompt.contains("plan.md"));
        assert!(prompt.contains("TASK-00.md"));
        assert!(prompt.contains("abc1234"));
        assert!(prompt.contains("Initial Code Review"));
    }

    #[test]
    fn followup_review_prompt_contains_all_fields() {
        let prompt = build_followup_review_prompt(
            Path::new("/workspace/.bear/spec.md"),
            Path::new("/workspace/.bear/plan.md"),
            Path::new("/workspace/.bear/TASK-01.md"),
            "def5678",
        );

        assert!(prompt.contains("spec.md"));
        assert!(prompt.contains("plan.md"));
        assert!(prompt.contains("TASK-01.md"));
        assert!(prompt.contains("def5678"));
        assert!(prompt.contains("Follow-up Code Review"));
    }

    #[test]
    fn coding_revision_prompt_contains_review_comment() {
        let task = CodingTask {
            task_id: "TASK-00".to_string(),
            title: "기본 타입 정의".to_string(),
            description: "핵심 타입을 정의합니다.".to_string(),
            dependencies: vec![],
        };

        let prompt = build_coding_revision_prompt(
            &task,
            Path::new("/workspace/.bear/spec.md"),
            Path::new("/workspace/.bear/plan.md"),
            "에러 핸들링을 추가해주세요.",
            "bear/integration/test-session-xyz",
        );

        assert!(prompt.contains("에러 핸들링을 추가해주세요."));
        assert!(prompt.contains("revision"));
        assert!(prompt.contains("TASK-00"));
        assert!(prompt.contains("기본 타입 정의"));
    }

    #[test]
    fn get_latest_commit_revision_returns_hash() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        init_git_repo(workspace);
        make_commit(workspace, "init.txt", "init", "initial commit");

        let revision = get_latest_commit_revision(workspace).unwrap();

        assert!(!revision.is_empty());
        assert_eq!(revision.len(), 40);
        assert!(revision.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn save_and_commit_task_report_in_worktree_creates_committed_file() {
        let temp_dir = TempDir::new().unwrap();
        let workspace = temp_dir.path();
        init_git_repo(workspace);
        make_commit(workspace, "init.txt", "init", "initial commit");

        let integration = create_integration_branch(workspace, "test").unwrap();
        let task_branch = create_task_branch(workspace, &integration, "TASK-00").unwrap();
        let worktree_path = create_worktree(workspace, &task_branch).unwrap();
        make_commit(&worktree_path, "feature.txt", "feature", "feature commit");

        let report_path = save_and_commit_task_report_in_worktree(
            &worktree_path,
            "20260216",
            "test-session",
            "TASK-00",
            "# Test Report\nImplementation complete.",
        )
        .unwrap();

        assert!(report_path.exists());
        let content = fs::read_to_string(&report_path).unwrap();
        assert!(content.contains("Implementation complete."));

        // 커밋되었는지 확인
        let log_output = Command::new("git")
            .current_dir(&worktree_path)
            .args(["log", "--oneline", "-1"])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&log_output.stdout);
        assert!(stdout.contains("Add implementation report for TASK-00"));

        remove_worktree(workspace, &worktree_path).unwrap();
    }
}
