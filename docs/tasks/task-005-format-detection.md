# Task 005: Implement Format Detection

- Status: DONE
- Priority: P0
- Dependencies: 004
- Goal: Detect supported image formats from file bytes and expose safe metadata needed by the scanner.
- Scope: A format detector in core, supported-format policy, and tests for valid and invalid headers.
- Out of scope: Recursive directory walking, full decode, animation handling, and optimization.
- Files to read: `docs/SPEC.md`, `crates/imgopt-core/src/model.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`, `crates/imgopt-core/src/model.rs` if required.
- Files to create: `crates/imgopt-core/src/detect.rs` and deterministic fixture bytes in tests.
- Steps:
  1. Use a maintained image parsing library or a narrowly scoped header probe; document the choice.
  2. Distinguish unsupported, malformed, and supported inputs.
  3. Do not trust file extensions as the source of truth.
  4. Keep probing bounded so malformed files cannot trigger unbounded allocation.
- Acceptance criteria:
  - JPEG, PNG, and WebP headers are recognized regardless of filename extension.
  - Truncated and random bytes return typed errors.
  - GIF, BMP, TIFF, and HEIC are classified as unsupported for optimization unless read-only support is explicitly documented.
- Test requirements: Header-level unit tests for all supported and representative unsupported cases.
- Risks: Header probing is not a substitute for full decode validation; later analyzer tasks must decode safely.
- Next task: `task-006-recursive-scanner`.
