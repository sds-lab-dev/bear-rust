# Bear AI Developer Application

This project, named **“Bear AI Developer,”** is a tool that supports specification-driven software development on top of the Claude Code CLI. The application is written in Rust. You can build a consistent development environment using Dev Containers, and you can also deploy it easily with Docker.

## Requirements
- The Claude Code CLI must be installed, and its executable path must be available in `$PATH`.
- A valid Anthropic API key must be set in the `ANTHROPIC_API_KEY` environment variable.

## Features
- Specification writing
- Development planning based on the specification
- Code writing and modification based on the specification and development plan
- Code review
- Documentation support

## Instructions

### Build
```bash
cd $WORKSPACE_ROOT_DIR
make build
```

## Run
```bash
cd $WORKSPACE_ROOT_DIR
make run
```

### Test
```bash
cd $WORKSPACE_ROOT_DIR
make test
```

### Cleanup
```bash
cd $WORKSPACE_ROOT_DIR
make clean
```

## Operation flow
The Bear AI Developer application supports the following primary development flow:

1. Requirements gathering
2. Question and answer loop to refine requirements
3. Writing the specification document
4. Development planning based on the specification
5. Question and answer loop to refine the development plan
6. Code writing based on the specification and development plan
7. Code review
8. User feedback and approval loop
9. Documentation

### Requirements gathering
- Users can enter requirements through a terminal user interface.
- The entered requirements are analyzed via the Claude Code CLI, and additional questions are asked to enable a clear specification.
- Users provide answers to the additional questions to refine and concretize the requirements.
- The additional-question loop continues until the AI model determines that no further questions are necessary.

### Writing the specification document
- Once requirements gathering is complete, the **Specification Agent** produces a draft specification document and presents it to the user.
- The user can provide feedback on the draft specification and request revisions as needed.
- The feedback and revision loop continues until the user is satisfied with the specification.
- The final approved specification moves to the development planning stage.

### Development planning
- Based on the approved specification, the **Planning Agent** produces a draft development plan document and presents it to the user.
- The user can provide feedback on the development plan and request revisions as needed.
- The feedback and revision loop continues until the user is satisfied with the development plan.
- The individual tasks specified in the development plan are split so that AI agents can process them in parallel.
- If there are dependencies among tasks, the development plan must represent a DAG (Directed Acyclic Graph) as an adjacency list to specify the execution order.
- The final approved development plan moves to the code writing stage.

## Code writing
- Following the approved specification and development plan, *n* **Coding Agents** run in multiple threads and write code in parallel.
- A dedicated agent is assigned to each individual task in the development plan. Each agent generates code independently for its assigned task.
- If there are inter-task dependencies, agents follow the DAG specified in the development plan and execute tasks in dependency order. For tasks with dependencies, the preceding task's session content is converted into a handoff document and passed to the subsequent task agents.
- Each agent uses the Claude Code CLI to write code.

## Code review
- The written code is examined by the **Review Agent**. The Review Agent runs in parallel on the same threads in which the Coding Agents executed.
- The Review Agent checks code quality, style, and whether functional requirements are satisfied, and if necessary sends revision requests to the Coding Agents.
- Coding Agents apply the review feedback and modify the code, after which the Review Agent reviews the updated code again.
- This loop continues until the code satisfies all review criteria or the maximum iterations (default: 5) are reached.
- If the criteria does not meet after the maximum iterations:
  - Approves the code as-is if it is reasonably close to the criteria and the remaining issues are minor.
  - Otherwise, explains the blockers to the user and request a guidance on how to proceed.

## User feedback and approval loop
- After all tasks in the development plan are completed and code review passes, the process requires the user's final approval.
- The user reviews the final code and can either approve it or request additional changes.
- If the user requests additional changes, the request is handed back to the Coding Agents for revision work. The revised code is reviewed again by the Review Agent, and this loop continues until the user approves.
- Once the user approves, the development process is complete.

## Documentation
- Throughout the steps above, each agent records its work as documentation.
- These documents are provided together with the project's final deliverables and can be used later for maintenance and reference.

---

# Mandatory Rules for AI Assistants

## Language
- Always answer in Korean.

## Coding conventions
- You MUST follow the coding conventions in order:
  1. Follow the given instructions described below.
  2. Follow the project's existing coding conventions unless explicitly stated in the given instructions.
  3. Follow the idiomatic style of the programming language used in the project if no existing conventions are present and no explicit instructions are given.
- Do NOT add unnecessary comments if the code is self-explanatory on its own.
  ```rust
  // Incorrect example

  let count = items.len(); // Counts the number of items
  ```
- You MUST add comments if it is not immediately obvious what the code does by reading it.
  ```rust
  // Correct example

  // Append the LLM response to the next request.
  self.request
      .append_message(response.choices[0].message.clone())
      .map_err(|err| EmailAnalyzerError::InvalidRequest(err.to_string()))?;
  ```
- Code readability is the highest priority.
  - Performance or idiomatic language expression is a consideration only after readability is secured. 
  - If code written in multiple lines is easier to understand than a one-liner that uses syntactic sugar, you must choose the former.
- Do NOT use abbreviations just to keep names short.
  - Use long names as-is to improve readability, even if they are lengthy. 
  - Exceptions are allowed for universally understood abbreviations such as `db`, or widely used loop indices such as `i` or `j`.
- You MUST refactor the structure immediately if blocks, conditionals, loops, and similar constructs cause indentation nesting of three levels or more. 
  - Note that "three levels of indentation" is not a magic number. 
  - Depending on the case, even a single level of indentation can harm readability, so handle it appropriately on a case-by-case basis.
- Every function MUST follow the single-responsibility principle and do only one thing.
  - An exception is when the function's purpose is orchestration by composing multiple components.
- Long functions or methods are NOT allowed:
  - If it exceeds 50 lines, you MUST consider refactoring.
  - If it exceeds 100 lines, you MUST refactor it unless you can justify that it has a single, cohesive responsibility.
- Do NOT prefer creating new types first; check whether the existing types in the codebase can be used before introducing new ones.
- Do NOT avoid modifying the existing application structure.
  - If the application already has "two eyes," do NOT take the easy route by attaching a "third eye" on the side.
  - Instead, modify the structure of the existing eyes so it can be done with only the two eyes.

## Coding guardrails

### Think before coding
**Do NOT assume. Do NOT hide confusion. Surface tradeoffs.**

Before implementing:
- State your assumptions explicitly.
- If you are uncertain, ask targeted questions.
- If multiple interpretations exist, present them. Do not pick silently.
- If a simpler approach exists, say so. Push back when warranted.
- If something is unclear, distinguish:
  - Blocking ambiguity: stop and ask.
  - Non-blocking ambiguity: choose a reasonable default, state it, and proceed.

### Simplicity first
**Minimum code that solves the problem. Nothing speculative.**

- Do NOT add extra features beyond what was asked.
- Avoid abstractions whose main purpose is hypothetical future flexibility.
- Prefer small helpers when they reduce duplication, clarify intent, or improve testability.
- Keep code proportional. If you wrote 200 lines and it could be 50, rewrite it.

Error handling:
- Do NOT add elaborate handling for truly unreachable states proven by invariants.
- Do add minimal, appropriate handling for:
  - External inputs (user input, files, network, databases)
  - Boundary cases (empty, null, out-of-range)
  - Concurrency and timeouts
- If you choose to omit handling, explain the invariant that makes it safe.

Ask yourself:
"Would a senior engineer say this is overcomplicated?"
If yes, simplify.

### Surgical changes
**Touch only what you must. Clean up only your own mess.**

When editing existing code:
- Do NOT refactor unrelated code, comments, naming, or formatting.
- Match existing style and conventions.
- Exception:
  - If the code you touch (or its immediate dependencies) has a correctness bug, security weakness, or reliability hazard that affects the behavior of your change, fix it.
  - If your change reveals or triggers a pre-existing build/test failure, type error, lint failure, or static-analysis issue in the area you touch, apply the minimal fix needed to restore a green build.
  - If there is an obvious resource-safety issue in the path you modify (for example, leaks, missing cleanup, missing bounds checks, unsafe concurrency), make the smallest fix that prevents harm.
  - If a small local adjustment is required to keep the change consistent with surrounding invariants (for example, error-handling contract, nullability/ownership rules, API preconditions), make that adjustment.
- Guardrails for exceptions:
  - Keep the fix local to the files/functions you touched (or the smallest necessary surface area).
  - Do NOT do opportunistic cleanup.
  - Do NOT reformat or "improve" nearby code unless required for the fix.
  - Explain what was wrong, why it matters, and why the chosen fix is minimal.

When your changes create orphans:
- Remove imports/variables/functions that your changes made unused.
- Do NOT remove pre-existing dead code unless asked.
- Instead, mention it and point to locations if you notice unrelated dead code or suspicious logic.

The test:
Every changed line should trace directly to the user's request or to making the requested change correct and verifiable.

### Goal-driven execution
**Define success criteria. Loop until verified.**

Transform tasks into verifiable goals. For example:
- "Add validation" -> "Write tests for valid and invalid inputs, then make them pass."
- "Fix the bug" -> "Write tests that reproduce the bug and verify no regression, then make them pass."
- "Refactor X" -> "Ensure tests pass before and after, and behavior is unchanged without regressions."
- "Implement feature Y" -> "Write tests that demonstrate the feature with several scenarios, then make them pass."

Verification rules:
- Prefer automated verification (tests, type checks, linting, builds).
- Prefer Testcontainers to verify external dependencies if available.
- If you cannot run verification in your environment, say so explicitly and provide:
  - Commands to run
  - Expected outputs or assertions
  - Any risks or edge cases to double-check
- If you find issues during verification, fix them and verify again.
- The verification loop continues until all criteria are met or the maximum number of iterations (default: 5) is reached.
- If the criteria does not meet after the maximum iterations:
  - Explains the blockers to the user and request a guidance on how to proceed.