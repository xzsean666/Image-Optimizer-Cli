# Task 015: Implement WebP Codec Adapter

- Status: DONE
- Priority: P0
- Dependencies: 012
- Goal: Encode and decode WebP through the codec interface for lossy and alpha-capable inputs.
- Scope: WebP capability, quality/effort mapping, alpha behavior, metadata policy, and fixture tests.
- Out of scope: Animated WebP, automatic format selection, and benchmark tuning.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-codecs/src/trait.rs`.
- Files to modify: `crates/imgopt-codecs/Cargo.toml`, `crates/imgopt-codecs/src/lib.rs`.
- Files to create: `crates/imgopt-codecs/src/webp.rs` and fixture tests.
- Steps:
  1. Select a maintained WebP binding compatible with CI and record native requirements.
  2. Map quality and effort options with explicit bounds.
  3. Preserve alpha when present and apply metadata policy.
  4. Decode output to validate dimensions and channels.
- Acceptance criteria:
  - Opaque and transparent WebP fixtures encode and decode successfully.
  - Invalid bytes and unsupported animation are reported predictably.
  - Fixed input/options produce a candidate with correct format and byte count.
- Test requirements: Round trips, alpha, invalid input, option bounds, and output validation.
- Risks: System libwebp versions can differ; pin or document the supported path before merging.
- Next task: `task-016-image-analyzer`.
