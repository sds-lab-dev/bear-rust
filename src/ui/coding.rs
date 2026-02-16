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

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text."#
}

const TASK_EXTRACTION_PROMPT_TEMPLATE: &str = r#"Extract all implementation tasks from the approved development plan.
Return them in topological order (dependency-first) as a JSON array.

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text.

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
- Repository: <repository name>
- Base Branch: <branch name> (the branch that this worktree is based on)
- Base Commit: <commit hash> (the commit that this worktree is based on)

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
Output MUST contain ONLY the JSON object, with no extra text.

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
  - {{UPSTREAM_REPORT_PATHS}}"#;

pub fn build_coding_task_prompt(
    task: &CodingTask,
    spec_path: &Path,
    plan_path: &Path,
    upstream_report_paths: &[PathBuf],
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
}

// ---------------------------------------------------------------------------
// Prompts – Conflict Resolution
// ---------------------------------------------------------------------------

const CONFLICT_RESOLUTION_PROMPT_TEMPLATE: &str = r#"A rebase onto the integration branch has produced merge conflicts that you must resolve.

Integration branch: {{INTEGRATION_BRANCH}}
Task ID: {{TASK_ID}}

Conflicted files:
{{CONFLICTED_FILES}}

Instructions:
1. Examine each conflicted file and resolve the merge conflicts.
2. After resolving all conflicts, stage the resolved files with `git add` for each file.
3. Complete the rebase with `git rebase --continue`.
4. If the rebase continues and produces more conflicts, resolve them and repeat.
5. If resolution is not possible, run `git rebase --abort` and report failure.

Output MUST be valid JSON conforming to the provided JSON Schema.
Output MUST contain ONLY the JSON object, with no extra text."#;

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
// Git Operations
// ---------------------------------------------------------------------------

pub fn create_integration_branch(
    workspace: &Path,
    session_name: &str,
) -> Result<String, String> {
    let branch_name = format!("bear/integration/{}-{}", session_name, Uuid::new_v4());

    let output = Command::new("git")
        .current_dir(workspace)
        .args(["branch", &branch_name])
        .output()
        .map_err(|e| format!("failed to execute git branch: {}", e))?;

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

pub fn squash_merge_task_branch(
    worktree_path: &Path,
    integration_branch: &str,
    task_branch: &str,
    commit_message: &str,
) -> Result<(), String> {
    let checkout_output = Command::new("git")
        .current_dir(worktree_path)
        .args(["checkout", integration_branch])
        .output()
        .map_err(|e| format!("failed to execute git checkout: {}", e))?;

    if !checkout_output.status.success() {
        let stderr = String::from_utf8_lossy(&checkout_output.stderr);
        return Err(format!(
            "failed to checkout integration branch: {}",
            stderr.trim()
        ));
    }

    let merge_output = Command::new("git")
        .current_dir(worktree_path)
        .args(["merge", "--squash", task_branch])
        .output()
        .map_err(|e| format!("failed to execute git merge --squash: {}", e))?;

    if !merge_output.status.success() {
        let stderr = String::from_utf8_lossy(&merge_output.stderr);
        return Err(format!("failed to squash merge: {}", stderr.trim()));
    }

    let commit_output = Command::new("git")
        .current_dir(worktree_path)
        .args(["commit", "-m", commit_message])
        .output()
        .map_err(|e| format!("failed to execute git commit: {}", e))?;

    if !commit_output.status.success() {
        let stderr = String::from_utf8_lossy(&commit_output.stderr);
        return Err(format!("failed to commit squash merge: {}", stderr.trim()));
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

// ---------------------------------------------------------------------------
// Report Management
// ---------------------------------------------------------------------------

fn session_directory(
    workspace: &Path,
    date_dir: &str,
    session_name: &str,
) -> PathBuf {
    workspace
        .join(".bear")
        .join(date_dir)
        .join(session_name)
}

pub fn save_task_report(
    workspace: &Path,
    date_dir: &str,
    session_name: &str,
    task_id: &str,
    report: &str,
) -> io::Result<PathBuf> {
    let dir = session_directory(workspace, date_dir, session_name);
    fs::create_dir_all(&dir)?;

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

        let prompt = build_coding_task_prompt(&task, spec_path, plan_path, &upstream_paths);

        assert!(prompt.contains("TASK-00"));
        assert!(prompt.contains("기본 타입 정의"));
        assert!(prompt.contains("핵심 타입을 정의합니다."));
        assert!(prompt.contains(&spec_path.display().to_string()));
        assert!(prompt.contains(&plan_path.display().to_string()));
        assert!(prompt.contains("TASK-01.md"));
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
        let prompt = build_coding_task_prompt(&task, spec_path, plan_path, &[]);

        assert!(prompt.contains("N/A"));
    }

    #[test]
    fn save_and_read_task_report() {
        let temp_dir = TempDir::new().unwrap();
        let report_content = "# Metadata\n구현 완료";

        let path = save_task_report(
            temp_dir.path(),
            "20260215",
            "test-session",
            "TASK-00",
            report_content,
        )
        .unwrap();

        let expected = temp_dir
            .path()
            .join(".bear")
            .join("20260215")
            .join("test-session")
            .join("TASK-00.md");
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
    fn squash_merge_task_branch_success() {
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

        squash_merge_task_branch(
            &worktree_path,
            &integration,
            &task_branch,
            "[TASK-00] Test squash merge",
        )
        .unwrap();

        // 통합 브랜치에 스퀘시 커밋이 1개 존재하는지 확인
        let log_output = Command::new("git")
            .current_dir(&worktree_path)
            .args(["log", "--oneline", &format!("{}..HEAD", "main")])
            .output()
            .unwrap();
        let stdout = String::from_utf8_lossy(&log_output.stdout);
        let commit_lines: Vec<&str> = stdout.lines().collect();
        assert_eq!(commit_lines.len(), 1);
        assert!(commit_lines[0].contains("[TASK-00]"));

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
}
