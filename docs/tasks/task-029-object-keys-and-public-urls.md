# Task 029: Define Object Keys and Public URLs

- Status: DONE
- Priority: P0
- Dependencies: 026
- Goal: Create stable storage keys and safe public URLs independent of source paths.
- Scope: Asset ID generation, object-key formatting, URL joining, and validation tests.
- Out of scope: Uploading, CDN provisioning, URL signing, and CLI formatting.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-manifest/src/model.rs`, `crates/imgopt-storage/src/trait.rs`.
- Files to modify: `crates/imgopt-manifest/src/lib.rs`, `crates/imgopt-storage/src/lib.rs` only for shared value types.
- Files to create: `crates/imgopt-storage/src/url.rs` and deterministic key tests.
- Steps:
  1. Generate an asset ID from a documented stable source, such as a validated ULID or digest-derived identifier.
  2. Format `images/{asset_id}/optimized.{format}` with no user path components.
  3. Join a configured public base URL and object key without duplicate or missing separators.
  4. Reject control characters and invalid URL bases.
- Acceptance criteria:
  - Moving or renaming a source file does not change its object key.
  - Same asset and format produce the same key under repeated runs.
  - URL output matches the object key exactly and cannot escape the base path.
- Test requirements: Stable ID/key, path traversal attempts, slash normalization, and custom base URL tests.
- Risks: Changing ID generation later changes URLs; document the chosen stability guarantee.
- Next task: `task-030-sync-service`.
