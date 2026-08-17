# Task 008: Define Manifest Data Model

- Status: DONE
- Priority: P0
- Dependencies: 004, 007
- Goal: Model durable source assets, optimization results, storage locations, and run state.
- Scope: Versioned manifest schema, source records, optimization records, storage records, and status enums.
- Out of scope: Reading/writing files, S3 calls, CLI output formatting, and schema migrations beyond a version field.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`, `crates/imgopt-core/src/model.rs`, `crates/imgopt-core/src/hash.rs`.
- Files to modify: `crates/imgopt-manifest/src/lib.rs`.
- Files to create: `crates/imgopt-manifest/src/model.rs` and fixture JSON tests.
- Steps:
  1. Represent the source SHA-256 as the primary map key or a validated field, not a path.
  2. Record optimizer version, optimization key, output size/format, storage bucket/key, and public URL.
  3. Include enough state to distinguish processed, skipped, failed, and uploaded-but-not-committed work.
  4. Add a top-level schema version.
- Acceptance criteria:
  - Example manifest and result data from `docs/SPEC.md` round-trip through serde.
  - Duplicate source records cannot be represented under two paths without an explicit alias field.
  - Unknown future fields are handled according to a documented compatibility policy.
- Test requirements: Golden JSON round trips and invalid schema tests.
- Risks: Manifest shape becomes a compatibility contract; avoid storing transient decoded pixels or CLI-only values.
- Next task: `task-009-atomic-manifest-persistence`.
