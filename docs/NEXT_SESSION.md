# Project State & Audit Handoff

## Current State

All 37 V1 backlog tasks are implemented across the 6 workspace crates (`imgopt-codecs`, `imgopt-quality`, `imgopt-core`, `imgopt-manifest`, `imgopt-storage`, `imgopt-cli`), and a full security, performance, and functional hardening audit pass has been completed.

## Key Hardening Applied

- **Security & Resource Limits**: Bounded preflight file reads, header-level dimension/pixel/allocation checks before decoding, explicit `u16` dimension guards in JPEG encoding, removal of committed test manifests containing sensitive endpoints/credentials, removal of automatic presigned URL persistence, strict S3 config/URL validation, and exclusive `tempfile` atomic writes.
- **Performance**: Parallel bounded execution (`execute_batch_scoped`) wired into `sync`, `optimize`, and `benchmark`; streaming `hash_file` before reads on `sync` skips; single-pass integer block accumulation in `compute_ssim`; lossless fast path; reusable `ureq::Agent` connection pool in `S3Client`.
- **Functionality**: Cross-channel quality projection (RGBA->JPEG, Grayscale->WebP/JPEG), EXIF orientation auto-normalization, real metadata preservation (`--preserve-metadata`), no-bloat fallback, strict 404 verification failures in `upload_and_verify` and `verify --check-storage`, and output collision disambiguation.

## Verification Gates

Run the workspace gates before releasing:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
```
