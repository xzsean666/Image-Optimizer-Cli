# Build and Verification Guide

This document describes how to build, test, and verify the `Image-Optimizer-Cli` workspace from source.

## System Prerequisites

- **Rust**: Version `1.88.0` or newer (`stable` channel managed via `rust-toolchain.toml` or `rustup`).
- **Cargo**: Bundled with standard Rust distributions.
- **Git**: For version control.

## Workspace Architecture

The workspace contains 7 member crates:
1. `crates/imgopt`: Unified Rust library facade (`imgopt`).
2. `crates/imgopt-codecs`: Image encoding and decoding adapters.
3. `crates/imgopt-quality`: SSIM and PSNR quality measurement.
4. `crates/imgopt-core`: Core pipeline, hashing, analysis, candidate search, and sync service.
5. `crates/imgopt-manifest`: Atomic manifest store, deduplication logic, and JSON results.
6. `crates/imgopt-storage`: S3/R2 object storage integration, retry backoff, and URL building.
7. `crates/imgopt-cli`: Command-line interface and binary executable (`imgopt`).

## Build Commands

### Debug Build
```bash
cargo build
```

### Release Build
```bash
cargo build --release
```
The compiled release executable will be at `target/release/imgopt`.

## Testing

### Run All Unit and Integration Tests
```bash
cargo test --workspace
```

### Run Specific Test Suites
```bash
# Core unit and integration tests
cargo test -p imgopt-core

# Storage and S3 retry tests
cargo test -p imgopt-storage

# CLI end-to-end integration tests
cargo test -p imgopt-cli --test test_e2e_full_workflow
```

## Quality and Lint Gates

### Formatting Check
```bash
cargo fmt --all -- --check
```

### Linter (Clippy)
```bash
cargo clippy --all-targets --all-features -- -D warnings
```

## Continuous Integration (CI)

The CI workflow is configured in `.github/workflows/ci.yml` and executes formatting, clippy, and full workspace tests on every pull request.
