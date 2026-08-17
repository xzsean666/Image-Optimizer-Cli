# Task 037: Release Hardening and Documentation

- Status: DONE
- Priority: P0
- Dependencies: 002, 034, 036
- Goal: Prepare a reproducible V1 handoff with complete docs, dependency review, and clean verification.
- Scope: User README, configuration reference, architecture links, changelog/decision notes, dependency audit, and final checks.
- Out of scope: New product features, AVIF/JPEG XL, Tauri, HTTP server, SQLite migration, and cloud provisioning.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`, `docs/tasks/TASKS.md`, all completed task handoffs.
- Files to modify: `README.md`, `docs/SPEC.md`, `docs/ARCHITECTURE.md`, `docs/tasks/TASKS.md`.
- Files to create: `docs/BUILD.md`, `docs/DECISIONS.md`, and a V1 changelog if the project convention needs one.
- Steps:
  1. Document installation, supported formats, commands, configuration precedence, storage setup, limits, and known limitations.
  2. Record non-obvious dependency and codec decisions with alternatives rejected.
  3. Review every task status and link its verification evidence.
  4. Run formatting, lint, workspace tests, e2e tests, and a smoke benchmark from a clean build.
  5. Inspect `git diff` and `git status`; remove generated artifacts from the worktree.
- Acceptance criteria:
  - A new developer can build and run local optimization from the docs alone.
  - V1 exit criteria in `docs/SPEC.md` are demonstrably satisfied or explicitly deferred.
  - No uncommitted generated binaries, credentials, or benchmark outputs remain.
- Test requirements: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace`, e2e tests, and smoke benchmark.
- Risks: Documentation can claim behavior that drifted during implementation; verify every command against the built binary.
- Next task: None; mark V1 complete only after all exit criteria pass.
