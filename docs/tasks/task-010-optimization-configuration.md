# Task 010: Define Optimization Configuration

- Status: DONE
- Priority: P0
- Dependencies: 004
- Goal: Represent presets and explicit optimization settings without hard-coding a single quality value.
- Scope: Preset enum, quality target, search bounds, effort, output format policy, metadata flags, and validation.
- Out of scope: Candidate encoding, CLI argument parsing, automatic image classification, and benchmark tuning.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`, `crates/imgopt-core/src/model.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`.
- Files to create: `crates/imgopt-core/src/config.rs`.
- Steps:
  1. Define `maximum`, `balanced`, and `aggressive` presets as named policies.
  2. Keep codec-specific knobs in an extensible but typed options structure.
  3. Validate quality ranges, dimensions, effort, and metadata flags before processing.
  4. Provide deterministic normalization for later optimization-key generation.
- Acceptance criteria:
  - Presets do not expose undocumented magic numbers to callers.
  - Invalid ranges fail before any file is decoded.
  - Equivalent configurations normalize to identical values.
- Test requirements: Preset defaults, validation boundaries, and normalization tests.
- Risks: Preset values will need benchmark tuning; keep them centralized and easy to change.
- Next task: `task-011-optimization-key`.
