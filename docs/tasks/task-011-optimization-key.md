# Task 011: Implement Optimization Key

- Status: DONE
- Priority: P0
- Dependencies: 008, 010
- Goal: Identify an optimization result by all inputs that can change its bytes.
- Scope: Canonical key input, stable serialization, digest/format, optimizer version, and equality tests.
- Out of scope: Manifest lookup, candidate search, asset ID generation, and CLI config parsing.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-manifest/src/model.rs`, `crates/imgopt-core/src/config.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`, `crates/imgopt-manifest/src/model.rs` only for the key type.
- Files to create: `crates/imgopt-core/src/optimization_key.rs`.
- Steps:
  1. Include source SHA-256, optimizer version, preset, output format, and normalized config.
  2. Serialize fields in a canonical order with an explicit version/domain separator.
  3. Hash the canonical bytes and expose a human-readable form for diagnostics.
  4. Test that every relevant input changes the key and irrelevant ordering does not.
- Acceptance criteria:
  - Same inputs always produce the same key across process runs.
  - Changing output format, preset, optimizer version, or any normalized setting changes the key.
  - The key does not contain the source path.
- Test requirements: Golden key vectors and mutation tests for each input field.
- Risks: Omitting a future byte-affecting option would cause incorrect cache hits; add a test when adding options.
- Next task: `task-012-codec-interface`.
