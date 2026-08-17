# Task 035: Enforce Security and Resource Limits

- Status: DONE
- Priority: P0
- Dependencies: 006, 016, 021
- Goal: Make malformed or hostile inputs fail within bounded CPU, memory, and filesystem behavior.
- Scope: Maximum file bytes, pixel count, dimensions, decompression safeguards, path validation, and metadata safety.
- Out of scope: Sandboxing native codecs, malware scanning, authentication design, and production deployment policy.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/scanner.rs`, `crates/imgopt-core/src/analyzer.rs`, `crates/imgopt-core/src/config.rs`.
- Files to modify: scanner, analyzer, config, and optimizer modules only where limits are enforced.
- Files to create: security-focused tests and fixtures under existing crate test directories.
- Steps:
  1. Define defaults and explicit overrides for bytes, pixels, width, and height.
  2. Check cheap file-size limits before decode and decoder-reported limits during decode.
  3. Reject path traversal/output escapes and unsafe symlink behavior.
  4. Ensure error messages do not include secrets or dump untrusted metadata.
- Acceptance criteria:
  - Oversized, high-pixel-count, and excessive-dimension fixtures fail before unbounded allocation.
  - Output paths remain within the requested output root.
  - Limits are applied consistently by inspect, optimize, and sync.
- Test requirements: Every limit boundary, traversal attempt, symlink policy, malformed metadata, and oversized header test.
- Risks: Native decoders may allocate before reporting dimensions; keep preflight byte limits and document residual risk.
- Next task: `task-036-end-to-end-test-suite`.
