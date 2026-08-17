# Task 028: Implement Upload Retry and Verification

- Status: DONE
- Priority: P0
- Dependencies: 027
- Goal: Upload validated bytes reliably and verify the stored object before reporting success.
- Scope: Timeouts, exponential backoff, retry classification, checksum/size verification, and fake-client tests.
- Out of scope: Manifest writes, public URL formatting, batch scheduling, and credential management.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-storage/src/trait.rs`, `crates/imgopt-storage/src/s3.rs`.
- Files to modify: `crates/imgopt-storage/src/lib.rs`.
- Files to create: `crates/imgopt-storage/src/upload.rs` and retry tests.
- Steps:
  1. Wrap upload calls with bounded attempts, timeout, and exponential backoff with a cap.
  2. Retry only timeouts and server-side 5xx-style errors.
  3. Head the object after upload and compare size and checksum when the backend exposes one.
  4. Return a typed verification error if the object is missing or mismatched.
- Acceptance criteria:
  - A transient failure succeeds within the configured retry budget.
  - Authentication and invalid-request errors are not retried.
  - Successful upload is not reported before verification completes.
- Test requirements: Immediate success, transient success, exhausted retries, permanent failure, missing object, and mismatch.
- Risks: Retry storms can overload a service; cap attempts and delay and make them configurable.
- Next task: `task-029-object-keys-and-public-urls`.
