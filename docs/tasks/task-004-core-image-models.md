# Task 004: Define Core Image Models

- Status: DONE
- Priority: P0
- Dependencies: 003
- Goal: Define serializable, codec-neutral models for image identity, profile, bytes, and output.
- Scope: Format enum, source reference, image profile, decoded image handle, encoded candidate metadata, and asset identifiers.
- Out of scope: Filesystem scanning, hashing implementation, codec-specific options, and manifest persistence.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/error.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`.
- Files to create: `crates/imgopt-core/src/model.rs` and focused unit tests in the same module.
- Steps:
  1. Use strongly typed byte counts, dimensions, channels, and formats where practical.
  2. Represent alpha and metadata presence explicitly.
  3. Keep raw decoded pixels behind a type that does not expose codec or storage concerns.
  4. Derive only stable serialization traits required by manifest/result tasks.
- Acceptance criteria:
  - Models can represent JPEG, PNG, and WebP and reject unknown output formats.
  - No model contains a CLI argument type, S3 type, or filesystem-specific behavior.
  - Unit tests cover serialization round trips for public value types.
- Test requirements: Module unit tests and `cargo test --workspace`.
- Risks: Avoid prematurely freezing fields that belong only in future AVIF/JXL work.
- Next task: `task-005-format-detection`.
