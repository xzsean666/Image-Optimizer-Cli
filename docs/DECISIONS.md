# Architecture Decision Records (ADRs)

This document records the architectural and technical design decisions made during the development of Image Optimizer V1.

---

## ADR-001: Content-Addressed Asset Identity via Streaming SHA-256

- **Context**: Image files in local projects frequently move, get renamed, or are stored in nested directory structures. Relying on file paths as primary keys causes cache misses and duplicate uploads.
- **Decision**: Asset identity (`source_sha256`) is computed strictly from the file's raw payload bytes using streaming SHA-256 (`64KB` buffer chunks). File paths, names, and timestamps are never part of the content hash.
- **Consequences**:
  - Renaming or moving a file does not re-trigger optimization or upload.
  - Constant memory footprint during hashing regardless of file size.

---

## ADR-002: Deterministic Optimization Key

- **Context**: Changes to optimization settings (e.g. preset, target format, quality target, effort, metadata policy, or optimizer version) require re-evaluating whether an existing optimized asset satisfies the new criteria.
- **Decision**: Derive a canonical `optimization_key` formatted as:
  `{source_sha256}:{optimizer_version}:{target_format}:{preset}:{quality_target}:{effort}:{metadata_policy}`.
- **Consequences**:
  - Guaranteed idempotence: identical input + identical config produces the same key.
  - Changing any optimization parameter triggers reprocessing, preventing stale low-quality artifacts.

---

## ADR-003: Embedded Codecs and Native Acceleration Libraries

- **Context**: External CLI tools (like `cwebp`, `jpegoptim`, `oxipng` binaries) or system daemons complicate deployment and cross-platform reproducibility.
- **Decision**: Embed native codec bindings directly in `imgopt-codecs`:
  - **Decoding**: `image` crate (`zune-jpeg`, `png`, `image-webp`) with pre-allocation `DecodeLimits` (`max_width`, `max_height`, `max_pixels`, `max_alloc_bytes`) and automatic EXIF orientation normalization.
  - **JPEG Encoding**: `jpeg-encoder` (`0.7`) with optimized Huffman tables, progressive scans, 4:2:0/4:4:4 chroma subsampling, and explicit ICC/EXIF/XMP embedding when requested.
  - **PNG Encoding**: `image` `PngEncoder` paired with `oxipng` (`10.2`, linking `libdeflate-sys` C library via `cc`) for lossless filter/DEFLATE optimization and explicit chunk stripping/preservation.
  - **WebP Encoding**: `webp` (`0.3`, linking `libwebp-sys` C library via `cc`) using `WebPConfig` (`method` mapped from `effort`, lossy/lossless) and RIFF `VP8X` chunk muxing when metadata preservation is enabled.
- **Consequences**:
  - Zero runtime dependency on external CLI binaries.
  - Requires a standard C toolchain (`cc`) at build time for `libwebp-sys` and `libdeflate-sys` (`oxipng`), which is handled by the GitHub Actions CI/Release environments.
  - Consistent typed error mapping (`CodecError`) and bounded decode safeguards before large pixel allocations.

---

## ADR-004: Upload-Before-Manifest Atomic Consistency

- **Context**: Network failures or process interruptions during batch sync could leave the manifest referencing missing or partial objects in remote storage.
- **Decision**: Strictly enforce an ordered pipeline:
  1. Optimize image bytes and validate output decode.
  2. Upload object to remote storage with exponential backoff retries.
  3. Verify object presence and byte size via `head_object`.
  4. Only upon successful verification, insert entry into manifest.
  5. Commit manifest atomically via temporary file and filesystem rename.
- **Consequences**:
  - The manifest is always a consistent snapshot of verified remote storage.
  - Partial run interruptions can be safely resumed on subsequent runs.

---

## ADR-005: Bounded Resource Limits and Failure Isolation

- **Context**: Processing hostile or malformed images (decompression bombs, excessive pixel counts) could exhaust host memory and abort entire batches.
- **Decision**:
  - Enforce preflight input byte limits (`100MB` default) and decoded pixel limits (`64MP` default).
  - Enforce per-file failure isolation: an invalid image records a failure in the result JSON while allowing all valid batch items to succeed.
- **Consequences**:
  - Batch processing is resilient against individual corrupt or oversized files.
  - System memory consumption remains strictly bounded.
