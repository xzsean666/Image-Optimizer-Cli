# Image Optimizer Architecture Contract

This document is the implementation contract for the V1 backlog. The source
requirements are the architecture description supplied with this repository.
When a task needs a choice that is not specified here, prefer the smallest
explicit API and record the choice in the task's change summary.

## V1 Workspace

The repository is a Cargo workspace with these crates:

```text
crates/imgopt-core      domain models, hashing, analysis, optimization flow
crates/imgopt-codecs    JPEG, PNG, and WebP adapters
crates/imgopt-quality   quality metrics and candidate scoring
crates/imgopt-manifest  persistent JSON manifest and run results
crates/imgopt-storage   S3-compatible object storage
crates/imgopt-cli       command line parsing and application wiring
```

Dependency direction:

```text
imgopt-cli -> imgopt-core -> imgopt-codecs
           -> imgopt-manifest
           -> imgopt-storage
imgopt-core -> imgopt-quality
```

The core crate must not depend on the CLI, S3 SDK, filesystem layout, or a
vendor-specific storage implementation. The storage crate must expose an
S3-compatible abstraction, not an R2-specific API.

## Stable Domain Contracts

- `source_sha256` is the content identity of an input file. It never includes
  the path, filename, or modification time.
- An `optimization_key` is derived from source hash, optimizer version, preset,
  output format, and normalized optimization configuration.
- Object keys use `images/{asset_id}/optimized.{format}` and do not include the
  source path.
- A manifest write happens only after a successful upload and storage
  verification.
- A repeated operation with the same source and optimization key is idempotent.
- Batch work is bounded. Decode, encode, analysis, and quality evaluation may
  run in parallel, but the implementation must not load an unbounded number of
  images.
- Invalid input is a per-file failure. One failed file must not abort a batch.

## Pipeline State

```text
discover -> hash -> lookup -> analyze -> generate candidates -> evaluate
         -> validate -> upload -> verify -> commit manifest
```

The local `optimize` command stops after output validation. The `sync` command
continues through storage verification and manifest commit.

## V1 Supported Formats

JPEG, PNG, and WebP are required. AVIF, JPEG XL, GIF animation, HEIC/HEIF,
and TIFF optimization are out of scope for V1 unless a task explicitly adds
read-only inspection support.

## Resource and Security Boundaries

The implementation must have configurable limits for input bytes, pixel count,
and dimensions. Paths must be treated as untrusted input. Metadata is removed
by default; preserving EXIF, ICC, or XMP requires an explicit option.

## Error and Compatibility Rules

Errors should identify the stage and source path without leaking credentials.
Retryable storage failures include timeouts and server-side 5xx responses.
Invalid images, invalid configuration, and authentication failures are not
automatically retried. Public APIs should remain small, typed, and testable.
