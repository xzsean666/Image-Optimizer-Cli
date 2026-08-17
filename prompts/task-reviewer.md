# Gemini Task Reviewer Template

You are reviewing one completed task under the single Image Optimizer V1 goal.

Read `GOAL.md`, `Agent.md`, `docs/ARCHITECTURE.md`, `docs/SPEC.md`, the task
file, and the implementation diff. Run the task's focused checks and inspect
the tests. Look for behavior regressions, missing acceptance criteria, hidden
dependencies, unsafe input handling, and changes outside the task scope.

Do not rewrite code during review unless the user explicitly asks for a fix.
Report findings first, ordered by severity, with file and line references.
Then report verification, residual risk, and whether the task is ready to mark
`DONE`.
