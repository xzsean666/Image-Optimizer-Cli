# Task 025: Implement Deduplication and Resume Decisions

- Status: DONE
- Priority: P0
- Dependencies: 009, 011, 021, 024
- Goal: Decide skip/process/retry from source hash, optimization key, and manifest state.
- Scope: Source deduplication, optimization cache lookup, stale/incomplete record handling, and decision tests.
- Out of scope: Directory scanning, optimization implementation, upload mechanics, and CLI argument parsing.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`, `crates/imgopt-manifest/src/store.rs`, `crates/imgopt-core/src/optimization_key.rs`.
- Files to modify: `crates/imgopt-manifest/src/lib.rs`, `crates/imgopt-core/src/lib.rs` only for shared decision types.
- Files to create: `crates/imgopt-manifest/src/dedup.rs` and tests.
- Steps:
  1. Define lookup results for new source, completed optimization, incomplete upload, stale record, and prior failure.
  2. Reuse an existing asset for identical source bytes while preserving the current source path as a run result field.
  3. Require matching optimization key and verified storage state before skipping an optimization.
  4. Make decisions pure and testable before wiring them into sync.
- Acceptance criteria:
  - Same bytes at different paths deduplicate.
  - A different preset or output format does not hit the cache.
  - A manifest record without verified storage is reprocessed or verified, never blindly skipped.
- Test requirements: New source, source duplicate, optimization hit, key miss, incomplete record, and prior failure.
- Risks: Incorrect skip logic can lose output permanently; default to reprocess when state is ambiguous.
- Next task: `task-026-storage-interface`.
