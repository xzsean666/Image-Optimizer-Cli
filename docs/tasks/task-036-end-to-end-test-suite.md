# Task 036: Build End-to-End Test Suite

- Status: DONE
- Priority: P0
- Dependencies: 022, 031, 032, 033, 035
- Goal: Verify user-visible workflows and cross-crate contracts from a clean temporary workspace.
- Scope: CLI process tests, fixture setup, fake S3 backend, manifest lifecycle, duplicate/resume/partial failure scenarios.
- Out of scope: Performance benchmarking, real cloud credentials, release packaging, and changing implementation to satisfy guessed tests.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`, all completed command and service tasks.
- Files to modify: Existing tests only when fixing a real contract mismatch.
- Files to create: `tests/fixtures/*` as minimal deterministic assets, `tests/e2e/*`, and any test harness module.
- Steps:
  1. Run the built CLI in temporary directories with fake storage and isolated environment variables.
  2. Cover optimize, sync, inspect, verify, and benchmark success paths.
  3. Cover duplicate content, optimization-key change, interruption/resume, invalid input, storage retry, and partial failure.
  4. Assert files, manifest, result JSON, exit codes, and storage state, not only stdout.
- Acceptance criteria:
  - Tests pass without network credentials or a developer-specific filesystem path.
  - The restart scenario proves no duplicate upload or manifest entry.
  - At least one test proves a failed file does not abort its batch.
- Test requirements: `cargo test --workspace` plus the documented e2e command.
- Risks: Brittle stdout assertions can hide behavior; assert structured files and exit status first.
- Next task: `task-037-release-hardening`.
