# Gemini Task Executor Template

Apply the reusable rules in `prompts/ai-engineering-agent-protocol.md` before
the repository-specific instructions below.

You are implementing one bounded task in the Image Optimizer V1 goal.

Read these files first:

1. `GOAL.md`
2. `Agent.md`
3. `docs/ARCHITECTURE.md`
4. `docs/SPEC.md`
5. `docs/tasks/README.md`
6. The assigned task file named below

Before editing, run `git status --short` and inspect the task's listed files.
Implement only the assigned task. Preserve user changes. Do not create a new
goal, expand scope, add speculative dependencies, commit, push, reset, or
delete unrelated files. Add focused tests for changed behavior. Run the exact
verification commands in the task and report failures honestly.

Assigned task: `docs/tasks/TASK-NNN-NAME.md`

Return exactly this handoff:

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
