# V1 Task Index

The single active goal is [`GOAL.md`](../../GOAL.md). Each task is one bounded
deliverable under that goal. Every task has a matching handoff prompt under
[`prompts/`](../../prompts/).

All tasks start as `TODO`. Update only the status of the task being worked on
and its direct predecessor/successor when the handoff requires it. The task
files are the detailed source of truth; this file is the dependency index.

## Phase 0 - Repository Foundation

| ID | Task | Priority | Dependencies | Status |
|---|---|---:|---|---|
| 001 | [Bootstrap workspace](task-001-bootstrap-workspace.md) | P0 | none | DONE |
| 002 | [Quality gates](task-002-quality-gates.md) | P0 | 001 | DONE |
| 003 | [Core error contract](task-003-core-error-contract.md) | P0 | 001 | DONE |

## Phase 1 - Domain and Input

| ID | Task | Priority | Dependencies | Status |
|---|---|---:|---|---|
| 004 | [Core image models](task-004-core-image-models.md) | P0 | 003 | DONE |
| 005 | [Format detection](task-005-format-detection.md) | P0 | 004 | DONE |
| 006 | [Recursive scanner](task-006-recursive-scanner.md) | P0 | 004, 005 | DONE |
| 007 | [SHA-256 hashing](task-007-sha256-hashing.md) | P0 | 001 | DONE |
| 008 | [Manifest model](task-008-manifest-model.md) | P0 | 004, 007 | DONE |
| 009 | [Atomic manifest persistence](task-009-atomic-manifest-persistence.md) | P0 | 008 | DONE |
| 010 | [Optimization configuration](task-010-optimization-configuration.md) | P0 | 004 | DONE |
| 011 | [Optimization key](task-011-optimization-key.md) | P0 | 008, 010 | DONE |

## Phase 2 - Codecs and Optimization

| ID | Task | Priority | Dependencies | Status |
|---|---|---:|---|---|
| 012 | [Codec interface](task-012-codec-interface.md) | P0 | 004 | DONE |
| 013 | [JPEG codec](task-013-jpeg-codec.md) | P0 | 012 | DONE |
| 014 | [PNG codec](task-014-png-codec.md) | P0 | 012 | DONE |
| 015 | [WebP codec](task-015-webp-codec.md) | P0 | 012 | DONE |
| 016 | [Image analyzer](task-016-image-analyzer.md) | P0 | 004, 005 | DONE |
| 017 | [Quality metrics](task-017-quality-metrics.md) | P0 | 004 | DONE |
| 018 | [Candidate ranking](task-018-candidate-ranking.md) | P0 | 017 | DONE |
| 019 | [Candidate search](task-019-candidate-search.md) | P0 | 012, 018 | DONE |
| 020 | [Metadata and output validation](task-020-metadata-output-validation.md) | P0 | 016 | DONE |
| 021 | [Local optimization service](task-021-local-optimization-service.md) | P0 | 010, 011, 012, 013, 014, 015, 016, 019, 020 | DONE |
| 022 | [Optimize CLI](task-022-optimize-cli.md) | P0 | 021 | DONE |

## Phase 3 - Batch, State, and Storage

| ID | Task | Priority | Dependencies | Status |
|---|---|---:|---|---|
| 023 | [Bounded executor](task-023-bounded-executor.md) | P0 | 021 | DONE |
| 024 | [Result JSON](task-024-result-json.md) | P0 | 004 | DONE |
| 025 | [Deduplication and resume](task-025-deduplication-and-resume.md) | P0 | 009, 011, 021, 024 | DONE |
| 026 | [Storage interface](task-026-storage-interface.md) | P0 | 003 | DONE |
| 027 | [S3 client and configuration](task-027-s3-client-and-config.md) | P0 | 026 | DONE |
| 028 | [Upload retry and verification](task-028-upload-retry-and-verification.md) | P0 | 027 | DONE |
| 029 | [Object keys and public URLs](task-029-object-keys-and-public-urls.md) | P0 | 026 | DONE |
| 030 | [Sync service](task-030-sync-service.md) | P0 | 021, 023, 025, 028, 029 | DONE |
| 031 | [Sync CLI](task-031-sync-cli.md) | P0 | 030 | DONE |

## Phase 4 - Operational Commands and Hardening

| ID | Task | Priority | Dependencies | Status |
|---|---|---:|---|---|
| 032 | [Inspect CLI](task-032-inspect-cli.md) | P1 | 016, 022 | DONE |
| 033 | [Verify CLI](task-033-verify-cli.md) | P1 | 009, 025, 028 | DONE |
| 034 | [Benchmark harness](task-034-benchmark-harness.md) | P1 | 017, 021 | DONE |
| 035 | [Security and resource limits](task-035-security-and-resource-limits.md) | P0 | 006, 016, 021 | DONE |
| 036 | [End-to-end test suite](task-036-end-to-end-test-suite.md) | P0 | 022, 031, 032, 033, 035 | DONE |
| 037 | [Release hardening and documentation](task-037-release-hardening.md) | P0 | 002, 034, 036 | DONE |

## Execution Notes

Tasks 013, 014, and 015 can be assigned independently after 012. Tasks 013
through 019 can be developed in parallel where their declared dependencies are
already complete. Integrate and run the full workspace checks after every
phase, not only at task 037.

The main dependency spine (branches converge before sync) is:

```text
001 -> 003 -> 004 -> 005 -> 006
001 -> 007 -> 008 -> 009 -> 025
004 -> 010 -> 011
004 -> 012 -> 013/014/015
004 -> 016 -> 017 -> 018 -> 019 -> 021
021 + 023 + 025 + 028 + 029 -> 030 -> 031 -> 036 -> 037
```

## V1 Completion Checklist

- [x] Every task is `DONE` or has an explicit documented deferral.
- [ ] `cargo fmt --check` passes (run in CI; install `rustfmt` locally if missing).
- [ ] `cargo clippy --all-targets --all-features -- -D warnings` passes (run in CI; install `clippy` locally if missing).
- [x] `cargo test --workspace` passes.
- [x] Local optimize, sync, inspect, verify, and benchmark commands are covered.
- [x] Duplicate, resume, partial failure, invalid image, and storage retry cases
  have automated coverage.
- [x] No credentials, generated binaries, or benchmark datasets are committed.
