# Repository Agent Instructions

The reusable process contract is
[`prompts/ai-engineering-agent-protocol.md`](prompts/ai-engineering-agent-protocol.md).
This file adds repository-specific rules and paths.

## Read First

Before making changes, read `GOAL.md`, `docs/ARCHITECTURE.md`, `docs/SPEC.md`,
`docs/tasks/README.md`, and the assigned task file. `AGENTS.md` points here for
tooling that expects the plural filename.

## One-Goal Rule

There is exactly one active repository goal: the V1 goal in `GOAL.md`. A task
is a bounded deliverable under that goal. Do not create a second goal, change
the goal, or declare the goal complete while tasks remain.

## Task Discipline

- Work on one assigned task only.
- Read the repository status before editing and preserve existing user changes.
- Touch only files listed in the task unless a required module declaration or
  focused fixture makes an additional file unavoidable.
- Follow the dependency order in `docs/tasks/TASKS.md`.
- Prefer small typed APIs and existing project conventions.
- Do not add dependencies, public commands, or architectural layers without
  recording the reason in the task handoff.
- Do not commit, push, reset, checkout, or delete unrelated files.

## Verification

Run the narrowest task checks first, then the broader checks required by the
task. Never report a check as passing unless it actually ran. Classify failures
as caused by the change, pre-existing, environmental, or unresolved.

## Handoff

Every task response must include:

```text
Goal: Image Optimizer V1
Task: task-NNN-name
Status: DONE | BLOCKED | REVIEW
Changed files:
Implementation summary:
Tests run and results:
Dependencies added or rejected:
Known risks:
Next task:
```

Use `BLOCKED` only when progress requires missing information, authorization,
or an external state change that cannot be replaced by a local test double.
