# Task 033: Implement Verify CLI

- Status: DONE
- Priority: P1
- Dependencies: 009, 025, 028
- Goal: Check manifest consistency and optionally verify referenced storage objects.
- Scope: Manifest loading, record validation, storage head/size/checksum checks, and exit/report policy.
- Out of scope: Repairing records, deleting objects, reprocessing images, and migration tooling.
- Files to read: `docs/SPEC.md`, `crates/imgopt-manifest/src/store.rs`, `crates/imgopt-manifest/src/dedup.rs`, `crates/imgopt-storage/src/upload.rs`.
- Files to modify: `crates/imgopt-cli/src/main.rs`, manifest/storage modules only for narrow validation helpers.
- Files to create: `crates/imgopt-cli/tests/verify.rs`.
- Steps:
  1. Validate schema, required fields, duplicate keys, and optimization-key consistency locally.
  2. Add a storage verification mode using the storage interface and fake backend in tests.
  3. Report every invalid record while continuing through the manifest.
  4. Return a nonzero exit status when any check fails.
- Acceptance criteria:
  - Valid manifest and matching fake storage pass.
  - Missing object, size mismatch, malformed record, and schema mismatch are reported.
  - Verify never mutates the manifest or storage.
- Test requirements: Valid, missing object, mismatch, malformed JSON, and no-mutation tests.
- Risks: Large manifests should stream or bound work where possible; do not load image bytes for verification.
- Next task: `task-034-benchmark-harness`.
