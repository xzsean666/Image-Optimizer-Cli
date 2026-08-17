# Task 032: Implement Inspect CLI

- Status: DONE
- Priority: P1
- Dependencies: 016, 022
- Goal: Provide read-only image inspection for debugging and strategy decisions.
- Scope: Command parsing, analyzer output, JSON/human modes if already supported, and no-write guarantees.
- Out of scope: Optimization, upload, manifest mutation, and advanced classification.
- Files to read: `docs/SPEC.md`, `crates/imgopt-core/src/analyzer.rs`, `crates/imgopt-cli/src/main.rs`.
- Files to modify: `crates/imgopt-cli/src/main.rs` and command modules.
- Files to create: `crates/imgopt-cli/tests/inspect.rs`.
- Steps:
  1. Add `inspect <input>` and print format, dimensions, channels, alpha, metadata presence, and byte size.
  2. Return structured errors for malformed or unsupported files.
  3. Ensure the command never writes output, manifest, or storage state.
  4. Keep output fields aligned with the analyzer model rather than duplicating calculations.
- Acceptance criteria:
  - JPEG, PNG, and WebP fixture inspection is correct.
  - Invalid input exits nonzero with a useful error.
  - A filesystem snapshot proves no writes occur.
- Test requirements: One fixture per format, invalid file, unsupported file, and no-write integration test.
- Risks: Human output should not become an accidental API; provide stable JSON only if implemented explicitly.
- Next task: `task-033-verify-cli`.
