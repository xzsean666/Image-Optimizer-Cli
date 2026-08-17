# Integration Boundaries

## Crates

`imgopt-cli` wires configuration and commands. `imgopt-core` owns domain
models, analysis, optimization, and pipeline decisions. `imgopt-codecs`
contains codec adapters. `imgopt-quality` computes quality scores.
`imgopt-manifest` persists JSON state and run results. `imgopt-storage` owns
the S3-compatible object storage adapter.

Core code must not depend on CLI parsing, a concrete S3 SDK, or a vendor name.
Storage code must expose upload, head/exists, and delete through a vendor-neutral
trait.

## External Services

S3-compatible storage is configured by endpoint, region, bucket, credentials,
and optional path-style behavior. R2, AWS S3, MinIO, and similar services must
fit the same interface. Tests must use an in-memory fake or local test server;
real credentials are never required for CI.

## Codec Dependencies

Codec tasks choose and document maintained Rust bindings. Native build
requirements, feature flags, license, and reproducibility must be recorded in
`docs/DECISIONS.md` before release hardening.
