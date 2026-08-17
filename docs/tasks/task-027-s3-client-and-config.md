# Task 027: Implement S3 Client and Configuration

- Status: DONE
- Priority: P0
- Dependencies: 026
- Goal: Connect the storage interface to an S3-compatible endpoint without vendor-specific core behavior.
- Scope: Endpoint, region, bucket, credentials, path-style option, timeout settings, and client construction.
- Out of scope: Upload retry, object-key policy, manifest commits, and CLI flags beyond config loading.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`, `crates/imgopt-storage/src/trait.rs`.
- Files to modify: `crates/imgopt-storage/Cargo.toml`, `crates/imgopt-storage/src/lib.rs`.
- Files to create: `crates/imgopt-storage/src/s3.rs`, `crates/imgopt-storage/src/config.rs`, and local client construction tests.
- Steps:
  1. Choose one maintained S3-compatible Rust client and record license/native requirements.
  2. Load endpoint and credentials from explicit config/environment names; never log secrets.
  3. Support custom endpoints for R2, MinIO, and other S3-compatible services.
  4. Map SDK errors to the storage error contract.
- Acceptance criteria:
  - Client construction is testable without network access.
  - Missing required config fails before an upload request.
  - Endpoint and path-style settings are passed exactly to the SDK.
- Test requirements: Config parsing, missing credentials, custom endpoint, and error mapping tests.
- Risks: SDK versions and feature flags can inflate compile time; enable only required features.
- Next task: `task-028-upload-retry-and-verification`.
