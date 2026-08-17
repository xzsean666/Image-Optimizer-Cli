# Image Optimizer V1 Goal

## Objective

Build a Rust-native, batch-oriented image optimizer that minimizes output size
while preserving an explicit perceptual quality target. The V1 product is a
CLI supporting JPEG, PNG, and WebP, local optimization, S3-compatible sync,
content-based deduplication, resumable state, and benchmarkable results.

## Authority

This is the single active goal for the repository. The user owns the goal.
Tasks in `docs/tasks/` are implementation slices under this goal; they are not
separate goals and must not redefine the product objective.

## Success Criteria

- The workspace builds from a clean checkout.
- `optimize`, `sync`, `inspect`, `verify`, and `benchmark` are implemented.
- JPEG, PNG, and WebP workflows have focused and end-to-end tests.
- SHA-256 identity, optimization keys, manifest persistence, and resume logic
  are deterministic and idempotent.
- Batch processing is bounded and isolates per-file failures.
- Storage is accessed through an S3-compatible abstraction with retry and
  post-upload verification.
- Documentation and verification evidence support a reproducible V1 handoff.

## Execution Model

Execute tasks in `docs/tasks/TASKS.md` order unless their declared dependencies
allow parallel work. Complete one task at a time per coding-model session.
Update task status only after its acceptance criteria and tests are verified.
Do not mark this goal complete until `task-037-release-hardening` passes the V1
completion checklist.

## Non-Goals

AVIF, JPEG XL, Tauri, HTTP server, SQLite migration, ML quality estimation,
cloud provisioning, and claims of outperforming TinyPNG are outside this goal.
