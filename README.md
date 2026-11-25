# Image Optimizer (`imgopt`)

A high-performance, deterministic, and idempotent Rust image optimization engine and CLI for local compression and S3-compatible cloud storage sync.

You can use this repository in **two ways**:
1. **As a standalone CLI tool (`imgopt`)** — download pre-built binaries from GitHub Releases, install directly via `cargo install --git`, or build from source.
2. **As a Rust library package (`imgopt` / workspace sub-crates)** — import directly into other Rust projects via GitHub in `Cargo.toml`.

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
crates/imgopt            Unified Rust library facade (re-exports core, codecs, quality, optional manifest/storage)
crates/imgopt-core       Domain models, format detection, hashing, analyzer, candidate search, sync pipeline
crates/imgopt-codecs     Codec implementations for JPEG, PNG, and WebP
crates/imgopt-quality    Objective quality metrics (SSIM and PSNR)
crates/imgopt-manifest   Atomic manifest store, deduplication logic, and run result models
crates/imgopt-storage    S3-compatible object storage client, exponential retries, URL formatting
crates/imgopt-cli        Command-line interface and binary entry point (`imgopt`)
```

---

## Documentation

- 📖 **[中文用户使用手册 (User Guide)](docs/USER_GUIDE.md)**: 包含完整命令指南、参数参考、实战场景与常见问题解答。
- 🏗️ **[Architecture Specification](docs/ARCHITECTURE.md)**: 系统架构与模块设计。
- ⚙️ **[Technical Specification](docs/SPEC.md)**: 算法细节与技术规范。

---

## Option A: Install & Build as CLI (`imgopt`)

### 1. Download Pre-built Binary (Recommended)
Download the standalone executable directly from [GitHub Releases](https://github.com/xzsean666/Image-Optimizer-Cli/releases):

- **Linux**: `imgopt-linux-x86_64.tar.gz` / `imgopt-linux-aarch64.tar.gz`
- **macOS**: `imgopt-macos-aarch64.tar.gz` (Apple Silicon) / `imgopt-macos-x86_64.tar.gz` (Intel)
- **Windows**: `imgopt-windows-x86_64.zip`

Each archive ships with a matching `.sha256` checksum file.

### 2. Install via Cargo from GitHub
```bash
# Latest main branch
cargo install --git https://github.com/xzsean666/Image-Optimizer-Cli.git

# Or pin to a specific release tag
cargo install --git https://github.com/xzsean666/Image-Optimizer-Cli.git --tag v0.1.0
```

### 3. Build from Source
```bash
git clone https://github.com/xzsean666/Image-Optimizer-Cli.git
cd Image-Optimizer-Cli
cargo build --release
```
The compiled binary will be located at `target/release/imgopt`.

---

## Option B: Use as a Rust Package (Library) via GitHub

Other Rust projects can import `imgopt` (or any individual workspace crate such as `imgopt-core`) directly from GitHub in `Cargo.toml`.

### 1. Add Dependency in `Cargo.toml`

**Standard usage (local optimization, codecs, SSIM/PSNR quality, batch executor):**
```toml
[dependencies]
imgopt = { git = "https://github.com/xzsean666/Image-Optimizer-Cli.git" }

# Recommended: pin to a release tag or commit
# imgopt = { git = "https://github.com/xzsean666/Image-Optimizer-Cli.git", tag = "v0.1.0" }
```

**With Manifest Deduplication & S3-Compatible Storage Sync:**
```toml
[dependencies]
imgopt = { git = "https://github.com/xzsean666/Image-Optimizer-Cli.git", features = ["full"] }
```

Available feature flags on `imgopt`:
- `default = []` — Core optimization pipeline (`imgopt::core`), codecs (`imgopt::codecs`), and quality metrics (`imgopt::quality`).
- `manifest` — Enables `imgopt::manifest` (`imgopt-manifest`).
- `storage` — Enables `imgopt::storage` (`imgopt-storage`).
- `full` — Enables both `manifest` and `storage`.

You can also depend on individual sub-crates directly from the same Git repository if needed:
```toml
[dependencies]
imgopt-core = { git = "https://github.com/xzsean666/Image-Optimizer-Cli.git" }
imgopt-codecs = { git = "https://github.com/xzsean666/Image-Optimizer-Cli.git" }
```

### 2. Rust Usage Examples

#### Optimize a Single File or In-Memory Bytes
```rust
use imgopt::{
    ImageFormat, MetadataPolicy, OptimizationConfig, OptimizationPreset,
    OptimizationService, OutputFormatPolicy,
};

fn main() -> imgopt::Result<()> {
    let service = OptimizationService::new(imgopt::VERSION);

    // Use a preset or customize quality / output format
    let mut config = OptimizationConfig::from_preset(OptimizationPreset::Balanced);
    config.output_format_policy = OutputFormatPolicy::Force(ImageFormat::Webp);
    config.metadata_policy = MetadataPolicy::StripAll;

    // 1) From file path
    let file_result = service.optimize_file("photo.png", &config)?;
    std::fs::write("photo.webp", &file_result.validated_output.bytes)?;

    // 2) Or directly from in-memory bytes
    let raw_bytes = std::fs::read("photo.png")?;
    let mem_result = service.optimize_bytes(&raw_bytes, &config)?;
    println!(
        "Optimized: {:?}, {} bytes, SSIM={:.4}",
        mem_result.effective_format,
        mem_result.validated_output.byte_size,
        mem_result.quality_score
    );

    Ok(())
}
```

#### Inspect Image & Batch Optimize a Directory
```rust
use imgopt::{
    analyze_image, execute_batch_scoped, scan_path, BatchOptions, BatchTask,
    OptimizationConfig, OptimizationPreset, OptimizationService, ScanOptions,
};

fn batch_example() -> imgopt::Result<()> {
    let service = OptimizationService::new(imgopt::VERSION);
    let config = OptimizationConfig::from_preset(OptimizationPreset::Maximum);

    // Analyze raw bytes before optimization
    let bytes = std::fs::read("photo.jpg")?;
    let analysis = analyze_image(&bytes, None)?;
    println!("Dimensions: {}x{}", analysis.profile.dimensions.width, analysis.profile.dimensions.height);

    // Scan a directory and run bounded concurrent optimization
    let scan = scan_path("./images", &ScanOptions::default())?;
    let tasks: Vec<BatchTask> = scan
        .files
        .into_iter()
        .map(|f| BatchTask {
            source_path: f.absolute_path,
            relative_path: f.relative_path,
        })
        .collect();

    let summary = execute_batch_scoped(
        &tasks,
        &BatchOptions::default(),
        |task| service.optimize_file(&task.source_path, &config),
    );
    println!("Succeeded: {}, Failed: {}", summary.succeeded, summary.failed);
    Ok(())
}
```

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

## Automated Releases (GitHub Actions)

The repository includes [`.github/workflows/release.yml`](.github/workflows/release.yml) to build and publish cross-platform CLI binaries with SHA-256 checksums to GitHub Releases on manual trigger (`workflow_dispatch`).

### Supported Release Targets
- `x86_64-unknown-linux-gnu` (`imgopt-linux-x86_64.tar.gz`)
- `aarch64-unknown-linux-gnu` (`imgopt-linux-aarch64.tar.gz`, cross-compiled via `cross`)
- `x86_64-apple-darwin` (`imgopt-macos-x86_64.tar.gz`)
- `aarch64-apple-darwin` (`imgopt-macos-aarch64.tar.gz`)
- `x86_64-pc-windows-msvc` (`imgopt-windows-x86_64.zip`)

### How to Trigger a Release
1. **Via GitHub UI**:
   Open **Actions -> Release -> Run workflow**, enter the release tag (e.g. `v0.1.1`), and click **Run workflow**.
2. **Via GitHub CLI (`gh`)**:
   ```bash
   gh workflow run release.yml -f tag=v0.1.1
   ```

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
