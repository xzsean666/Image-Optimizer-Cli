# Task 034: Implement Benchmark Harness

- Status: DONE
- Priority: P1
- Dependencies: 021, 017
- Goal: Measure output size, compression ratio, quality score, and elapsed time on a deterministic dataset.
- Scope: Dataset discovery, benchmark cases, metrics aggregation, and machine-readable output.
- Out of scope: TinyPNG network comparison, dataset generation, long-term result storage, and strategy changes.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`, `crates/imgopt-core/src/optimizer.rs`, `crates/imgopt-quality/src/metric.rs`.
- Files to modify: `crates/imgopt-cli/src/main.rs` only for command registration.
- Files to create: `crates/imgopt-core/src/benchmark.rs`, `crates/imgopt-cli/tests/benchmark.rs`, and a small test fixture set.
- Steps:
  1. Discover supported files deterministically and run the optimizer with explicit config.
  2. Record original bytes, optimized bytes, ratio, quality score, and elapsed duration per file.
  3. Aggregate by format and total without including generated outputs in the input set.
  4. Emit stable JSON suitable for later comparison.
- Acceptance criteria:
  - Repeating the same benchmark reports the same bytes and scores within documented timing variance.
  - Missing or invalid files are reported as cases, not silently dropped.
  - Benchmark does not alter manifest or storage.
- Test requirements: Fixture discovery, aggregate math, invalid case, and deterministic JSON tests.
- Risks: Timing is machine-dependent; never use elapsed time as a pass/fail quality gate.
- Next task: `task-035-security-and-resource-limits`.
