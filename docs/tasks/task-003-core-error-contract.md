# Task 003: Define Core Error Contract

- Status: DONE
- Priority: P0
- Dependencies: 001
- Goal: Establish typed, stage-aware errors shared by core and application layers.
- Scope: Error enums, source context, retry classification, and conversion tests.
- Out of scope: CLI rendering, logging policy, storage retry implementation, and user-facing wording.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/lib.rs`, `crates/imgopt-storage/src/lib.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`, `crates/imgopt-storage/src/lib.rs` only for shared trait bounds.
- Files to create: `crates/imgopt-core/src/error.rs`.
- Steps:
  1. Define errors for invalid input, unsupported format, configuration, resource limits, encode/decode, quality, storage, and persistence stages.
  2. Include optional source path and stage context without embedding credentials.
  3. Provide an explicit `is_retryable` classification for errors that can be retried.
  4. Add unit tests for classification and display output.
- Acceptance criteria:
  - Callers can match error categories without parsing strings.
  - Display output identifies stage and source when available.
  - Retryability is false for invalid images, invalid config, and authentication errors.
- Test requirements: Focused unit tests plus workspace tests.
- Risks: Overly broad error conversions can hide useful context; keep conversions explicit.
- Next task: `task-004-core-image-models`.
