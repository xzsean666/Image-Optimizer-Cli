# Task 012: Define Codec Interface

- Status: DONE
- Priority: P0
- Dependencies: 004
- Goal: Let the optimizer call JPEG, PNG, and WebP encoders through one typed boundary.
- Scope: Decoder input, codec capabilities, encode request/result, metadata policy, and error conversion.
- Out of scope: Concrete codec implementation, candidate search, quality metrics, and CLI behavior.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/model.rs`, `crates/imgopt-codecs/src/lib.rs`.
- Files to modify: `crates/imgopt-codecs/src/lib.rs`, `crates/imgopt-core/src/model.rs` only for shared neutral types.
- Files to create: `crates/imgopt-codecs/src/trait.rs` and interface tests with a fake codec.
- Steps:
  1. Define capability queries so callers do not guess supported formats.
  2. Use borrowed or owned buffers deliberately and document ownership/size behavior.
  3. Separate decode, encode, and metadata operations so the optimizer can test each stage.
  4. Add a fake codec that records requests for unit tests.
- Acceptance criteria:
  - The optimizer can select a codec without matching on vendor types.
  - Encode results include format, bytes, dimensions, and effective settings.
  - Codec errors convert to the core error contract without losing stage context.
- Test requirements: Fake codec tests for capabilities, request propagation, and error mapping.
- Risks: An overly generic trait can hide expensive copies; document buffer ownership and keep methods narrow.
- Next task: `task-013-jpeg-codec` (then `task-014-png-codec` and `task-015-webp-codec` can run independently).
