# Task 016: Implement Image Analyzer

- Status: DONE
- Priority: P0
- Dependencies: 004, 005
- Goal: Extract bounded image characteristics needed by strategy selection without performing optimization.
- Scope: Dimensions, channels, alpha, color information available from the decoder, metadata presence, byte size, and basic classification hints.
- Out of scope: Quality scoring, codec selection, ML classification, and output encoding.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/model.rs`, `crates/imgopt-codecs/src/trait.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`, `crates/imgopt-core/src/model.rs`.
- Files to create: `crates/imgopt-core/src/analyzer.rs` and analyzer tests.
- Steps:
  1. Decode through the codec-neutral boundary with configured limits.
  2. Populate a profile for supported formats, including alpha and metadata flags.
  3. Keep classification heuristic fields optional and explain their meaning.
  4. Reject malformed or resource-exceeding images before allocating unbounded buffers.
- Acceptance criteria:
  - Profiles for JPEG, PNG, and WebP fixtures contain correct dimensions and channels.
  - Analyzer does not write files or call storage.
  - The same bytes produce the same profile independent of path.
- Test requirements: Fixture profiles, malformed input, alpha, and limit-boundary tests.
- Risks: Decoder-reported metadata varies by codec; expose only values with defined semantics.
- Next task: `task-017-quality-metrics`.
