# Gemini Prompt Files

Start with the reusable process contract in
[`ai-engineering-agent-protocol.md`](ai-engineering-agent-protocol.md). It can
be reused for another repository by replacing the goal, task index, rules, and
task paths in its start template.

The repository has one active goal in `GOAL.md`. Each file in this directory
starts exactly one task under that goal. Give Gemini 3.7 Flash one prompt file
per session, in dependency order from `docs/tasks/TASKS.md`.

Before using a task prompt, confirm all listed dependencies are `DONE`. The
prompt tells the model to read the goal, agent rules, architecture, spec, task
file, and current repository state before editing.

After the model responds, copy its handoff into the task tracking workflow and
change only the relevant task status. A task marked `REVIEW` still requires a
human integration check; it does not advance dependent tasks automatically.
