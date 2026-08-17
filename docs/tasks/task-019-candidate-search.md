# Task 019: Implement Candidate Search

- Status: DONE
- Priority: P0
- Dependencies: 012, 018
- Goal: Generate a bounded set of codec candidates and choose the best acceptable one.
- Scope: Search range, step/binary strategy, cancellation boundary, codec invocation, quality evaluation, and search tests.
- Out of scope: Preset definition, automatic format selection across source formats, upload, and benchmark tuning.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-core/src/config.rs`, `crates/imgopt-core/src/candidate.rs`, `crates/imgopt-codecs/src/trait.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`.
- Files to create: `crates/imgopt-core/src/search.rs`.
- Steps:
  1. Generate only the configured bounded quality/effort values.
  2. Encode and score each candidate through traits, never vendor APIs.
  3. Short-circuit only when the search algorithm can prove no smaller acceptable result exists.
  4. Return diagnostics for attempted and rejected candidates.
- Acceptance criteria:
  - Search is deterministic and bounded by configuration.
  - A fake codec test proves the smallest acceptable result is selected.
  - Codec and quality failures identify the candidate that failed.
- Test requirements: Range boundaries, all-rejected case, deterministic ranking, and fake-codec call count.
- Risks: Incorrect early stopping can miss a smaller result; start with exhaustive bounded search unless proven otherwise.
- Next task: `task-020-metadata-output-validation`.
