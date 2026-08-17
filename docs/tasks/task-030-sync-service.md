# Task 030: Implement Sync Service

- Status: DONE
- Priority: P0
- Dependencies: 021, 023, 025, 028, 029
- Goal: Compose scan, hash, deduplication, optimize, upload, verify, result, and manifest commit into an idempotent service.
- Scope: Pipeline orchestration, per-file state transitions, bounded execution, upload ordering, and partial failure handling.
- Out of scope: CLI parsing, progress rendering, S3 client construction, and advanced strategy selection.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`, all dependency task modules.
- Files to modify: `crates/imgopt-core/src/lib.rs`, `crates/imgopt-manifest/src/lib.rs`, `crates/imgopt-storage/src/lib.rs` only for narrow integration traits.
- Files to create: `crates/imgopt-core/src/sync.rs` and service integration tests with fake storage.
- Steps:
  1. Scan and hash before any optimization or upload.
  2. Apply pure deduplication decisions and process only required work.
  3. Validate output, upload, verify, then commit the manifest in that order.
  4. Produce one result record per discovered input and continue after per-file failures.
  5. Make rerunning the same inputs with the same config idempotent.
- Acceptance criteria:
  - A successful file has storage verification before its manifest record is visible.
  - Duplicate and already-complete inputs are skipped with reusable URLs.
  - A failed file does not prevent other files from completing.
  - A simulated interruption leaves state that the next run can resume safely.
- Test requirements: Full fake pipeline for success, duplicate, key miss, upload failure, partial batch, and resume.
- Risks: Partial manifest updates under concurrency can race; serialize commits or use a clearly synchronized store.
- Next task: `task-031-sync-cli`.
