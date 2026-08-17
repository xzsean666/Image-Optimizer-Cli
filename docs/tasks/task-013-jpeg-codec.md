# Task 013: Implement JPEG Codec Adapter

- Status: DONE
- Priority: P0
- Dependencies: 012
- Goal: Encode and decode JPEG through the codec interface using a maintained Rust binding.
- Scope: JPEG capability, quality/effort mapping, metadata stripping, deterministic output tests, and dependency documentation.
- Out of scope: MozJPEG algorithm changes, automatic format selection, and candidate search.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-codecs/src/trait.rs`.
- Files to modify: `crates/imgopt-codecs/Cargo.toml`, `crates/imgopt-codecs/src/lib.rs`.
- Files to create: `crates/imgopt-codecs/src/jpeg.rs`, fixture tests, and a short dependency note if the crate choice is non-obvious.
- Steps:
  1. Select a maintained JPEG encoder/decoder binding compatible with the pinned toolchain; verify license and native build requirements.
  2. Map normalized options to codec settings without accepting out-of-range values.
  3. Strip metadata by default and test explicit preservation flags if the interface supports them.
  4. Decode the output again to validate dimensions and color handling.
- Acceptance criteria:
  - Valid JPEG fixtures round-trip through decode and encode.
  - Transparent input is rejected or converted according to a documented policy.
  - Encoded output is deterministic for fixed input and options where the codec permits it.
- Test requirements: Fixture round trip, invalid bytes, quality bounds, metadata policy, and output decode tests.
- Risks: Native codec availability can break CI; use a pure-Rust or documented system dependency and make CI install requirements explicit.
- Next task: `task-014-png-codec`.
