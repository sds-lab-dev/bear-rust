---
name: handoff
description: "Handoff agent to preseves essential state of the current session and then handoff the state to the next session for a new task so the next session can continue safely and reproducibly."
disable-model-invocation: true
allowed-tools:  AskUserQuestion, Bash, TaskOutput, Glob, Grep, KillShell, Read, Skill, WebFetch, WebSearch, LSP, Edit, Write
---

You are generating a HANDOFF DOCUMENT for the next task. The goal is to reset the chat/session while preserving ONLY the essential state so the next session can continue safely and reproducibly.

# Context

- We just finished a task and will start a new task in a fresh new session.
- This document is the SINGLE SOURCE OF TRUTH for the new task. Do not rely on the conversation transcript after this is created.

# Instructions

## Flow
1) Produce ONE Markdown file as output, and NOTHING ELSE.
2) Follow the exact template below. Do not rename sections. Do not add/remove sections.
3) Be factual and precise. Avoid speculation. If something is unknown, write "UNKNOWN" and explain what would be needed to confirm it.
4) Prefer verifiable details over narrative. When referencing code, include concrete identifiers: file paths, function/class names, flags, commands, and relevant configuration keys.
5) Keep it concise but complete.
6) Clearly separate: facts vs decisions vs recommendations. Recommendations must be explicitly labeled and must include the rationale.

## Output path of the handoff document
- Write the document to: `<WORKSPACE_ROOT>/.HANDOFF-<SUBJECT>.md`
    - where:
      - `<WORKSPACE_ROOT>`: absolute path of the workspace directory.
      - `<SUBJECT>`: short subject for the current task that should be consist of only alphanumeric, `-`, and `_` characters.
  Example:
    - `/workspace/.HANDOFF-tui-skeleton.md`
    - `/workspace/.HANDOFF-fix-broken-api.md`
- If the path already exists, create a new file by appending `-v2`, `-v3`, etc.
  Example:
    - If `/workspace/.HANDOFF-tui-skeleton.md` exists, create `/workspace/.HANDOFF-tui-skeleton-v2.md`
    - If `/workspace/.HANDOFF-tui-skeleton-v2.md` also exists, create `/workspace/.HANDOFF-tui-skeleton-v3.md`
    - Continue this pattern until you find a non-existing file name.
- Do NOT overwrite existing handoff documents EVER.

# Handoff document template

## Metadata
- Datetime: <YYYY-MM-DD HH:MM:SS>
- Repo/workspace: <path or repo name>
- Branch/commit: <branch name + commit hash if available>
- Authoring agent/session: <if applicable>

## Initial user request
- Write the exact message verbatim (1:1) that the user requested when this session begins.
- Do NOT paraphrase, summarize, translate, correct typos, change formatting, or omit any content.
- Preserve every character exactly as written, including punctuation, whitespace, line breaks, code blocks, and any special symbols.

## Scope Summary
Describe in 3–8 sentences:
- What the current task was supposed to accomplish
- What is considered DONE for the current task
- What remains OUT OF SCOPE for the current task (explicitly)

## Current System State (as of end of the current task)
Provide the minimal state needed to continue:
- Feature flags / environment variables:
- Build/test commands used:
- Runtime assumptions (OS, containers, services, versions):
- Any required secrets/credentials handling notes (do not include actual secrets):

## Key Decisions (and rationale)
List the decisions that MUST be preserved, each with:
- Decision:
- Rationale:
- Alternatives considered (if any) and why rejected:

## Invariants (MUST HOLD)
List non-negotiable constraints that must remain true in next tasks.
Each invariant must be testable/verifiable.

## Prohibited Changes (DO NOT DO)
List actions that would break assumptions, expand scope, or introduce risk.
Be explicit (e.g.,): "Do NOT change public API X", "Do NOT alter schema Y", "Do NOT refactor module Z".

## What Changed in the current task
Be concrete and verifiable:
- New/modified files (paths):
- New/changed public interfaces (signatures, endpoints, CLI options):
- Behavior changes:
- Tests added/updated:
- Migrations/config changes:

## Known Issues / Technical Debt
List any intentional shortcuts, open bugs, flaky tests, or follow-ups created by the current task.
Include how to reproduce and current status.

## Handoff Checklist
Confirm each item with YES/NO:
- All invariants listed: <YES/NO>
- All prohibited changes listed: <YES/NO>
- Concrete code changes listed: <YES/NO>
- Output saved to <HANDOFF_DOCUMENT_PATH>: <YES/NO>

End of document.

Now generate the handoff document using the template and write it to <HANDOFF_DOCUMENT_PATH>.