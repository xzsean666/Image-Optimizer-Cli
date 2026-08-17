# Task 031: Implement Sync CLI

- Status: DONE
- Priority: P0
- Dependencies: 030
- Goal: Expose the complete sync pipeline with explicit local and storage configuration.
- Scope: CLI flags/config loading, manifest path, JSON result path, storage setup, summary output, and exit policy.
- Out of scope: Core pipeline logic, SDK behavior, progress UI polish, and benchmark command.
- Files to read: `docs/SPEC.md`, `crates/imgopt-core/src/sync.rs`, `crates/imgopt-storage/src/config.rs`.
- Files to modify: `crates/imgopt-cli/src/main.rs` and CLI modules.
- Files to create: `crates/imgopt-cli/tests/sync.rs` plus only required config helpers.
- Steps:
  1. Add `sync` with input root, manifest path, result JSON path, preset, and storage settings.
  2. Validate all configuration before scanning.
  3. Return a nonzero exit code only according to the documented batch policy; preserve per-file failures in result JSON.
  4. Keep human summary separate from machine-readable result output.
- Acceptance criteria:
  - Dry/local fake-storage integration can run without real credentials.
  - Successful, skipped, and failed counts match result JSON.
  - A storage configuration error occurs before any image upload.
- Test requirements: CLI parsing, config failure, fake-storage success, skip, partial failure, and result file tests.
- Risks: Environment variable precedence can surprise users; document and test precedence explicitly.
- Next task: `task-032-inspect-cli`.
