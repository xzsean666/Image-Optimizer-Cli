# Initial Gemini Prompt

你现在是 Image Optimizer V1 的实现代理。

先读取并遵守 `prompts/ai-engineering-agent-protocol.md`。它是通用的
AI Engineering Agent Protocol；本文件只提供本仓库第一次启动时的具体
目标和任务参数。

仓库只有一个总目标，定义在 `GOAL.md`。不要创建第二个 goal，也不要尝试一次性实现整个项目。你必须按照 `docs/tasks/TASKS.md` 的依赖顺序逐个完成任务。

开始前必须读取：

1. `GOAL.md`
2. `Agent.md`
3. `docs/ARCHITECTURE.md`
4. `docs/SPEC.md`
5. `docs/tasks/README.md`
6. `prompts/ai-engineering-agent-protocol.md`
7. `docs/tasks/task-001-bootstrap-workspace.md`

然后执行 `task-001-bootstrap-workspace`：

- 先运行 `git status --short`，保留现有用户修改。
- 只创建任务文件中列出的 Cargo workspace 骨架。
- 不实现图片处理、codec、Manifest、S3、批处理或其他后续任务。
- 不添加未声明的依赖或架构层。
- 为任务要求的行为添加最小测试。
- 运行任务要求的全部验证命令。

完成后不要继续执行 task-002。请严格按以下格式报告：

```text
Goal: Image Optimizer V1
Task: task-001-bootstrap-workspace
Status: DONE | BLOCKED | REVIEW
Changed files:
Implementation summary:
Tests run and results:
Dependencies added or rejected:
Known risks:
Next task:
```

只有在实际验证通过后才能报告 `DONE`。如果仓库状态与任务描述不一致，先停止并报告差异。
