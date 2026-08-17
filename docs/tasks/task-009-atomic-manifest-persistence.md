# Task 009: Implement Atomic Manifest Persistence

- Status: DONE
- Priority: P0
- Dependencies: 008
- Goal: Load and save the JSON manifest safely across crashes and concurrent read attempts.
- Scope: File loading, missing-file initialization, schema validation, atomic replacement, and filesystem tests.
- Out of scope: Manifest lookup policy, optimization decisions, upload calls, and SQLite.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-manifest/src/model.rs`.
- Files to modify: `crates/imgopt-manifest/src/lib.rs`.
- Files to create: `crates/imgopt-manifest/src/store.rs`.
- Steps:
  1. Create parent directories explicitly when configured.
  2. Write a temporary file in the same directory, flush it, then atomically replace the target.
  3. Reject malformed JSON and unsupported schema versions with typed errors; do not overwrite the original on failure.
  4. Test interrupted or failed writes with a replaceable filesystem abstraction where practical.
- Acceptance criteria:
  - A missing manifest loads as an empty versioned manifest.
  - A malformed manifest leaves the original bytes unchanged.
  - A successful save can be loaded by a new process.
- Test requirements: Temporary-directory tests for create, reload, malformed input, and replacement failure.
- Risks: Platform-specific rename semantics need a documented fallback; keep the replacement operation narrow.
- Next task: `task-010-optimization-configuration`.
