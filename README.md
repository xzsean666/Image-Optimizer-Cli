# Image Optimizer CLI (`imgopt`)

A high-performance, deterministic, and idempotent Rust image optimization engine and CLI for local compression and S3-compatible cloud storage sync.

## Key Features

- **Multi-Format Support**: Native compression and conversion for **JPEG**, **PNG**, and **WebP**.
- **Objective Quality Target**: Block-based Structural Similarity Index (**SSIM**) and Peak Signal-to-Noise Ratio (**PSNR**) candidate ranking.
- **Content-Addressed Deduplication**: Streaming SHA-256 content hashing decouples image identity from file names and directory paths.
- **Safe & Idempotent Sync**: Uploads and validates objects on S3-compatible storage (Cloudflare R2, AWS S3, MinIO) before committing manifest updates atomically.
- **Resilient & Bounded**: Exponential backoff retry for transient network errors, worker pool concurrency bounds, and memory protection limits.
- **Complete Toolset**: Includes `optimize`, `sync`, `inspect`, `verify`, and `benchmark` subcommands.

---

## Workspace Crates

```text
crates/imgopt-core       Domain models, format detection, hashing, analyzer, candidate search, sync pipeline
crates/imgopt-codecs     Codec implementations for JPEG, PNG, and WebP
crates/imgopt-quality    Objective quality metrics (SSIM and PSNR)
crates/imgopt-manifest   Atomic manifest store, deduplication logic, and run result models
crates/imgopt-storage    S3-compatible object storage client, exponential retries, URL formatting
crates/imgopt-cli        Command-line interface and binary entry point (`imgopt`)
```

---

## Installation & Build

### Prerequisites
- Rust 1.80.0+ (`rustup toolchain install stable`)
- Cargo

### Building from Source
```bash
git clone https://github.com/xzsean666/Image-Optimizer-Cli.git
cd Image-Optimizer-Cli
cargo build --release
```
The compiled binary will be located at `target/release/imgopt`.

---

## CLI Usage

### 1. Local Optimization (`optimize`)
Optimize images locally without remote storage:
```bash
# Optimize a single image with default balanced preset
imgopt optimize image.png --output optimized/image.webp --format webp

# Optimize an entire directory with maximum quality preset
imgopt optimize ./photos --output ./dist --preset maximum

# Preserve original formats with custom quality target
imgopt optimize ./photos --quality 0.95 --effort 5 --format preserve
```

### 2. Cloud Storage Sync (`sync`)
Scan, optimize, upload to S3/R2 storage, and commit manifest atomically:
```bash
# Dry run without remote credentials
imgopt sync ./photos --dry-run --manifest manifest.json --json result.json

# Production sync to Cloudflare R2 / AWS S3
export IMGOPT_S3_BUCKET="my-bucket"
export IMGOPT_S3_ENDPOINT="https://<account_id>.r2.cloudflarestorage.com"
export IMGOPT_S3_ACCESS_KEY_ID="<key_id>"
export IMGOPT_S3_SECRET_ACCESS_KEY="<secret_key>"
export IMGOPT_S3_PUBLIC_URL_BASE="https://cdn.example.com"

imgopt sync ./photos --manifest ./photos/manifest.json --preset balanced
```

### 3. Inspect Image (`inspect`)
Inspect image dimensions, format, color type, transparency, and metadata without modifying files:
```bash
imgopt inspect photo.jpg
imgopt inspect photo.png --json
```

### 4. Verify Manifest & Remote Objects (`verify`)
Verify local manifest schema, entry checksums, and optional remote S3 object integrity:
```bash
imgopt verify --manifest manifest.json
imgopt verify --manifest manifest.json --check-storage
```

### 5. Benchmark Performance (`benchmark`)
Run optimization benchmarks across an image dataset and output metrics:
```bash
imgopt benchmark ./dataset --preset maximum --json benchmark_report.json
```

---

## Configuration & Precedence

Optimization and storage options can be configured via CLI flags or environment variables (CLI flags take precedence):

| CLI Flag | Environment Variable | Default | Description |
|---|---|---|---|
| `--bucket` | `IMGOPT_S3_BUCKET` / `AWS_S3_BUCKET` | — | S3 bucket name |
| `--endpoint` | `IMGOPT_S3_ENDPOINT` / `AWS_ENDPOINT_URL` | S3 Standard | Custom S3/R2 endpoint URL |
| `--public-url-base` | `IMGOPT_PUBLIC_URL_BASE` | — | Base URL for public CDN links |
| `--preset` | — | `balanced` | Preset: `balanced`, `maximum`, `aggressive` |
| `--quality` | — | preset default | Target SSIM quality score (0.0 to 1.0) |
| `--effort` | — | preset default | Encoding effort (1 to 6) |

---

## Architecture & Design Documents

- [Architecture Specification](docs/ARCHITECTURE.md)
- [Technical Specification](docs/SPEC.md)
- [Architecture Decision Records (ADRs)](docs/DECISIONS.md)
- [Build and Verification Guide](docs/BUILD.md)
- [Task Backlog](docs/tasks/TASKS.md)

---

## License

Licensed under either of Apache License, Version 2.0 or MIT license at your option.
