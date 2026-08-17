# Task 022: Implement Optimize CLI

- Status: DONE
- Priority: P0
- Dependencies: 021
- Goal: Expose local optimization through a stable command without storage side effects.
- Scope: CLI parsing, input/output paths, config flags, output naming, exit codes, and human-readable errors.
- Out of scope: Sync, manifest persistence, S3 configuration, progress bars, and benchmark reporting.
- Files to read: `docs/SPEC.md`, `crates/imgopt-cli/src/main.rs`, `crates/imgopt-core/src/optimizer.rs`.
- Files to modify: `crates/imgopt-cli/src/main.rs`, `crates/imgopt-cli/Cargo.toml`.
- Files to create: CLI module files and integration tests under `crates/imgopt-cli/tests/`.
- Steps:
  1. Add an `optimize` subcommand with explicit input and optional output directory.
  2. Validate paths and config before decoding.
  3. Write only validated output using a temporary file and replacement.
  4. Return nonzero for invalid input/config and a clear summary for success.
- Acceptance criteria:
  - `imgopt optimize` works for one file and a directory according to the documented scope.
  - It never initializes or writes a manifest or storage object.
  - Re-running it produces valid output without corrupting an existing file.
- Test requirements: CLI success, invalid path, unsupported format, output collision, and no-storage-side-effect tests.
- Risks: CLI text is user-facing; keep machine-readable data in task 024 rather than scraping human text.
- Next task: `task-023-bounded-executor`.
