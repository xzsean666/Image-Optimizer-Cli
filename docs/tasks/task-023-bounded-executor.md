# Task 023: Implement Bounded Batch Executor

- Status: DONE
- Priority: P0
- Dependencies: 021
- Goal: Process many independent images with bounded memory and per-file isolation.
- Scope: Work queue, CPU parallelism, concurrency limits, cancellation, and aggregate counters.
- Out of scope: Manifest deduplication, S3 uploads, CLI progress UI, and retry policy for storage.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/optimizer.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs` and workspace dependencies if required.
- Files to create: `crates/imgopt-core/src/executor.rs` and executor tests.
- Steps:
  1. Select Rayon, Tokio, or a small combination consistent with the I/O and CPU boundaries.
  2. Bound queued and active work explicitly; do not collect all decoded images first.
  3. Capture one result per input, including errors, and continue after a failure.
  4. Add a test-only concurrency probe to prove the configured limit is respected.
- Acceptance criteria:
  - Peak active jobs never exceed the configured bound.
  - One invalid image does not cancel independent valid images.
  - Results remain associated with their original source references.
- Test requirements: Empty batch, one failure, ordering/association, and concurrency-bound tests.
- Risks: Mixing async I/O with CPU work can starve executors; keep blocking codec work on a CPU pool.
- Next task: `task-024-result-json`.
