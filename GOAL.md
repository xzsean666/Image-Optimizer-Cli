# Image Optimizer Goal

## Objective

Build a Rust-native, batch-oriented, production-grade image optimizer that
minimizes output size while preserving an explicit perceptual quality target.
The product is both a CLI (`imgopt`) and embeddable Rust library (`imgopt`)
supporting JPEG, PNG (lossless + lossy palette quantization), WebP, AVIF, and
GIF / animated image optimization, image resizing, ICC-to-sRGB color
management, local incremental optimization, S3-compatible sync with CDN
cache-control & remote garbage collection, content-based deduplication,
resumable state, and benchmarkable results.

## Authority

This is the single active goal for the repository. The user owns the goal.
Tasks in `docs/tasks/` are historical V1 implementation slices under this goal.

## Success Criteria

- The workspace builds cleanly without external CLI runtime dependencies.
- `optimize`, `sync`, `inspect`, `verify`, `benchmark`, and `gc` are implemented.
- JPEG, PNG, WebP, AVIF, and GIF workflows have focused and end-to-end tests.
- SHA-256 identity, optimization keys, manifest persistence, and resume logic
  are deterministic and idempotent.
- Batch processing is bounded by both concurrency and decoded memory budgets,
  and isolates per-file failures.
- Storage is accessed through an S3-compatible abstraction with retry,
  `Cache-Control` support, listing, deletion, and post-upload verification.
- Documentation and verification evidence support a reproducible production handoff.

## Non-Goals

JPEG XL, Tauri, HTTP server, SQLite migration, ML quality estimation, cloud
provisioning, and claims of outperforming TinyPNG are outside this goal.
