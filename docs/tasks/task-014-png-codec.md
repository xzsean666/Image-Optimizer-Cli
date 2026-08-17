# Task 014: Implement PNG Codec Adapter

- Status: DONE
- Priority: P0
- Dependencies: 012
- Goal: Encode and decode PNG through the codec interface while preserving required alpha behavior.
- Scope: PNG capability, lossless/lossy option mapping, alpha handling, metadata policy, and deterministic tests.
- Out of scope: Palette quantizer research, animated PNG, candidate search, and automatic format selection.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-codecs/src/trait.rs`.
- Files to modify: `crates/imgopt-codecs/Cargo.toml`, `crates/imgopt-codecs/src/lib.rs`.
- Files to create: `crates/imgopt-codecs/src/png.rs` and fixture tests.
- Steps:
  1. Choose a maintained PNG stack and record why it meets V1 needs.
  2. Support RGBA fixtures without silently dropping alpha.
  3. Apply metadata removal by default.
  4. Decode encoded output and compare dimensions, channels, and alpha presence.
- Acceptance criteria:
  - Opaque and transparent PNG fixtures round-trip.
  - Lossless mode preserves pixels exactly for supported color types.
  - Invalid and oversized input errors are typed and bounded.
- Test requirements: Pixel equality for lossless fixtures, alpha tests, invalid bytes, and metadata policy.
- Risks: Quantization can be lossy and may belong in a later benchmark task; keep the first adapter predictable.
- Next task: `task-015-webp-codec`.
