# Task 026: Define Storage Interface

- Status: DONE
- Priority: P0
- Dependencies: 003
- Goal: Define a vendor-neutral asynchronous object storage contract.
- Scope: Object identity, upload, head/exists, delete, response metadata, and typed storage errors.
- Out of scope: AWS/R2 SDK wiring, retries, public URL construction, and CLI configuration.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-storage/src/lib.rs`, `crates/imgopt-core/src/error.rs`.
- Files to modify: `crates/imgopt-storage/src/lib.rs`.
- Files to create: `crates/imgopt-storage/src/trait.rs` and mock implementation tests.
- Steps:
  1. Define bucket/key/content type/body/checksum inputs explicitly.
  2. Keep the trait limited to operations required by V1: upload, head/exists, and delete.
  3. Make credentials and endpoint configuration external to the trait.
  4. Add an in-memory fake for later sync and retry tests.
- Acceptance criteria:
  - No public type is named after R2, AWS, or a specific SDK.
  - A caller can verify object size and checksum after upload.
  - Mock behavior covers success, missing object, and retryable/permanent errors.
- Test requirements: Trait contract tests against the fake implementation.
- Risks: Async trait choices affect all storage callers; choose the smallest stable approach supported by the toolchain.
- Next task: `task-027-s3-client-and-config`.
