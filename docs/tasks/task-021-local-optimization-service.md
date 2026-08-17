# Task 021: Implement Local Optimization Service

- Status: DONE
- Priority: P0
- Dependencies: 010, 011, 012, 013, 014, 015, 016, 019, 020
- Goal: Compose analysis, strategy, candidate search, and validation into one storage-independent operation.
- Scope: Optimizer service API, default strategy for JPEG/PNG/WebP, output result, and per-file errors.
- Out of scope: Directory batching, manifest lookup, uploads, CLI parsing, and progress rendering.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`, `crates/imgopt-core/src/search.rs`, `crates/imgopt-core/src/validation.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`.
- Files to create: `crates/imgopt-core/src/optimizer.rs` and service tests with fake codecs/metrics.
- Steps:
  1. Accept a source reader/reference and validated optimization config.
  2. Analyze once, choose a codec, run bounded candidate search, and validate the winner.
  3. Return bytes plus all fields needed for a local output and later manifest record.
  4. Keep filesystem writes behind a caller-provided output operation or separate adapter.
- Acceptance criteria:
  - JPEG, PNG, and WebP fixture inputs produce validated candidates.
  - The service has no dependency on CLI, manifest, S3, or global configuration.
  - A single source failure is returned as an error without process exit.
- Test requirements: One fixture per format, no acceptable candidate, invalid config, and fake dependency tests.
- Risks: Default format policy may evolve after benchmarks; isolate it behind a strategy function.
- Next task: `task-022-optimize-cli`.
