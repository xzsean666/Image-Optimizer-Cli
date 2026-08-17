# Task 020: Implement Metadata and Output Validation

- Status: DONE
- Priority: P0
- Dependencies: 016
- Goal: Apply metadata policy and verify optimized bytes before they can be written or uploaded.
- Scope: Metadata default policy, output decode validation, size accounting, and validation errors.
- Out of scope: Storage verification, manifest commits, codec implementation, and user-facing flags.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/analyzer.rs`, `crates/imgopt-codecs/src/trait.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`, `crates/imgopt-codecs/src/trait.rs` only if validation needs a neutral hook.
- Files to create: `crates/imgopt-core/src/validation.rs` and tests.
- Steps:
  1. Define default removal of unnecessary metadata and explicit preservation flags.
  2. Decode candidate bytes with limits and compare expected format/dimensions/channels.
  3. Reject empty output, corrupt output, dimension changes, and unexpected format changes.
  4. Return original and optimized byte counts for downstream results.
- Acceptance criteria:
  - Corrupted candidate bytes never reach the filesystem or storage layer.
  - Metadata behavior is covered by a fixture containing removable metadata.
  - Validation is independent of paths and storage clients.
- Test requirements: Valid output, corruption, zero bytes, dimension mismatch, and metadata policy tests.
- Risks: Some codecs legitimately change color representation; validate semantic invariants rather than raw pixel bytes.
- Next task: `task-021-local-optimization-service`.
