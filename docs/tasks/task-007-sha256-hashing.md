# Task 007: Implement SHA-256 Hashing

- Status: DONE
- Priority: P0
- Dependencies: 001
- Goal: Produce content identity from source bytes independent of path and filename.
- Scope: Streaming SHA-256 helper, typed digest representation, and test vectors.
- Out of scope: Manifest lookup, asset ID generation, encryption, and upload checksums.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/model.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`, `crates/imgopt-core/Cargo.toml`.
- Files to create: `crates/imgopt-core/src/hash.rs`.
- Steps:
  1. Add a maintained SHA-256 crate and pin it through the workspace dependency policy.
  2. Hash a reader in bounded chunks rather than loading the whole file.
  3. Provide lowercase hexadecimal and byte forms without ambiguous formatting.
  4. Test known vectors and identical bytes under different paths.
- Acceptance criteria:
  - Hash output matches standard SHA-256 vectors.
  - Large-reader tests demonstrate streaming behavior through a test reader.
  - No path, timestamp, or filename is included in the digest.
- Test requirements: Known vectors, empty input, chunk-boundary cases, and workspace tests.
- Risks: Changing digest formatting would invalidate manifest identity; document the chosen representation.
- Next task: `task-008-manifest-model`.
