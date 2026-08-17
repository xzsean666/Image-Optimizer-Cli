# Task 024: Define and Write Result JSON

- Status: DONE
- Priority: P0
- Dependencies: 004
- Goal: Emit a stable per-run result separate from the long-lived manifest.
- Scope: Summary counters, per-file records, serialization, optional output path, and atomic write.
- Out of scope: Manifest lookup, optimization logic, CLI parsing, and storage calls.
- Files to read: `docs/SPEC.md`, `crates/imgopt-manifest/src/model.rs`, `crates/imgopt-core/src/model.rs`.
- Files to modify: `crates/imgopt-manifest/src/lib.rs`.
- Files to create: `crates/imgopt-manifest/src/result.rs` and golden JSON tests.
- Steps:
  1. Record total, processed, skipped, failed, original bytes, optimized bytes, and saved percentage.
  2. Record source path, status, errors, output format, and URL when available without requiring every field.
  3. Compute percentages from integer byte counts with a defined zero-byte policy.
  4. Write JSON atomically when a path is supplied.
- Acceptance criteria:
  - Example result from `docs/SPEC.md` round-trips exactly under the chosen formatting policy.
  - Summary counters equal the per-file records.
  - A failed file cannot contribute optimized bytes.
- Test requirements: Empty run, all skipped, mixed statuses, zero original bytes, and golden serialization.
- Risks: Changing field names breaks automation; document any intentional schema versioning.
- Next task: `task-025-deduplication-and-resume`.
