# Task 017: Implement Quality Metrics

- Status: DONE
- Priority: P0
- Dependencies: 004
- Goal: Compare original and candidate pixels with a deterministic perceptual quality score.
- Scope: Metric interface, baseline PSNR/SSIM-style implementation or maintained crate adapter, normalization, and invalid-input handling.
- Out of scope: Saliency detection, ML quality models, threshold tuning against TinyPNG, and candidate search.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-quality/src/lib.rs`, `crates/imgopt-core/src/model.rs`.
- Files to modify: `crates/imgopt-quality/src/lib.rs`.
- Files to create: `crates/imgopt-quality/src/metric.rs` and deterministic metric tests.
- Steps:
  1. Select one baseline metric with documented assumptions about dimensions, channels, and alpha.
  2. Define a score range and an explicit failure for incomparable images.
  3. Keep original and candidate data ownership clear to avoid accidental mutation.
  4. Add known-equality and known-degradation fixtures.
- Acceptance criteria:
  - Identical images receive the maximum score.
  - A visibly degraded fixture scores lower than the original.
  - Different dimensions or unsupported channel layouts fail clearly.
- Test requirements: Identity, small perturbation, severe degradation, channel mismatch, and determinism tests.
- Risks: Metric values are not yet benchmark-calibrated; label thresholds as provisional.
- Next task: `task-018-candidate-ranking`.
