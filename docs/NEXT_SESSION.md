# Next Session Handoff

## Current State

The repository contains the V1 goal, architecture/specification documents,
37 task definitions, and Gemini execution prompts. No Rust workspace or source
implementation exists yet.

## Next Task

Execute `task-001-bootstrap-workspace` using
`prompts/task-001-bootstrap-workspace.md`.

## Required First Actions

1. Read `GOAL.md`, `Agent.md`, `docs/ARCHITECTURE.md`, `docs/SPEC.md`, and the
   task file.
2. Check `git status --short`.
3. Create only the workspace files listed by task 001.
4. Run the task's Cargo metadata, build, and test checks.
5. Update the task status and provide the required handoff report.

## Known Constraints

Do not claim Cargo checks pass before task 001 creates the workspace. Do not
introduce AVIF, JPEG XL, storage credentials, or production infrastructure in
the bootstrap task.
