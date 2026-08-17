# Task Execution Guide

These tasks are intentionally small enough for one coding-model session. Give
Gemini one prompt file at a time, in dependency order. All prompts operate on
the single repository goal in `GOAL.md`; do not create a separate goal for a
task or ask the model to implement the whole backlog in one prompt.

## Before Starting a Task

1. Read `prompts/ai-engineering-agent-protocol.md`, `GOAL.md`, `Agent.md`,
   `docs/ARCHITECTURE.md`, `docs/SPEC.md`, this file, the requested task file,
   and its matching file under `prompts/`.
2. Run `git status --short` and inspect the files named by the task.
3. Confirm that every dependency is marked `DONE` in `docs/tasks/TASKS.md`.
4. If the repository differs materially from the task, stop and report the
   mismatch before editing.

## During a Task

- Touch only the files listed in the task unless a required test fixture or
  module declaration makes an additional file unavoidable.
- Prefer existing crates and project conventions. Do not add speculative
  abstractions or unrelated formatting changes.
- Add a focused regression test for changed behavior.
- Keep public APIs typed and explicit. Avoid hidden global state.
- Do not commit, push, delete user files, or change unrelated tasks.

## Required Handoff

The model must report:

```text
Task: task-NNN-name
Status: DONE | BLOCKED | REVIEW
Changed files:
Behavior implemented:
Tests run and results:
Dependencies added or rejected:
Known risks:
Next task:
```

Mark a task `BLOCKED` only for missing information, authorization, or an
external service that cannot be replaced by a local mock. A failing test is a
bug to investigate, not a reason to mark the task blocked.
