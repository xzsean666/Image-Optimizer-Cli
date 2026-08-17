# Task 006: Implement Recursive Scanner

- Status: DONE
- Priority: P0
- Dependencies: 004, 005
- Goal: Discover candidate files deterministically and apply initial resource and path filters.
- Scope: Recursive directory scanning, regular-file filtering, extension-independent detection, stable ordering, and scan errors.
- Out of scope: SHA-256, decoding, manifest lookup, symlink policy beyond the documented default, and optimization.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/detect.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`.
- Files to create: `crates/imgopt-core/src/scanner.rs` and scanner tests using temporary directories.
- Steps:
  1. Add a maintained directory-walking dependency only if the standard library is insufficient.
  2. Define whether symlinks are skipped by default and test that policy.
  3. Return a stable sorted list of source references.
  4. Preserve per-path errors without silently treating inaccessible files as valid inputs.
- Acceptance criteria:
  - Nested JPEG, PNG, and WebP files are found in stable order.
  - Directories, sockets, and unsupported files are excluded.
  - A missing root and an unreadable child produce actionable typed errors.
- Test requirements: Temporary-directory tests for nesting, ordering, unsupported files, and symlinks.
- Risks: Following symlinks can escape the requested root; default to not following them.
- Next task: `task-007-sha256-hashing`.
