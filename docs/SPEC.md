# Image Optimizer V1 Specification

## Commands

```text
imgopt optimize <input> [--output <dir>]
imgopt sync <input> [--manifest <path>] [--json <path>]
imgopt inspect <input>
imgopt verify [--manifest <path>]
imgopt benchmark <dataset>
```

The exact flag names may be refined during CLI tasks, but command boundaries
must remain stable. `optimize` never uploads. `sync` performs scan, hash,
deduplication, optimization, upload, verification, and manifest commit.

## Functional Requirements

1. Recursively discover supported image files without following unsafe paths.
2. Hash source bytes with SHA-256 and deduplicate by content.
3. Generate and evaluate multiple candidates for a selected preset.
4. Select the smallest candidate that meets the configured quality target.
5. Write validated local output and preserve no unnecessary metadata by default.
6. Process batches with bounded concurrency and per-file error isolation.
7. Persist a JSON manifest atomically and resume after interruption.
8. Upload through an S3-compatible interface with timeout, retry, and verify.
9. Emit a machine-readable result JSON for a sync run.
10. Expose inspection, verification, and benchmark commands.

## Non-Functional Requirements

- `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`,
  and `cargo test --workspace` must pass before V1 is released.
- No task may introduce a dependency without recording why the dependency is
  needed and checking its license and maintenance status.
- Tests must use deterministic fixtures and temporary directories.
- Credentials must come from environment variables or an explicit config
  source; never from committed files or command output.

## V1 Exit Criteria

- A clean checkout builds the whole workspace.
- Local optimization works for JPEG, PNG, and WebP fixtures.
- Duplicate source bytes produce one asset and a skipped result on repeat runs.
- A killed and restarted sync resumes without duplicate uploads or manifest
  entries.
- Storage behavior is covered with a mock or local S3-compatible test server.
- CLI integration tests cover success, skip, invalid input, and partial failure.
- Benchmark output reports bytes, ratio, quality score, and elapsed time.
