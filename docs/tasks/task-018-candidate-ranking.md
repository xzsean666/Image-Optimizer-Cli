# Task 018: Define Candidate Ranking

- Status: DONE
- Priority: P0
- Dependencies: 017
- Goal: Select the smallest acceptable candidate deterministically, with quality as a hard constraint.
- Scope: Candidate record, acceptance predicate, tie-breaking, and ranking tests.
- Out of scope: Encoding, candidate generation, presets, storage, and CLI output.
- Files to read: `docs/ARCHITECTURE.md`, `crates/imgopt-quality/src/metric.rs`, `crates/imgopt-core/src/model.rs`.
- Files to modify: `crates/imgopt-core/src/lib.rs`.
- Files to create: `crates/imgopt-core/src/candidate.rs`.
- Steps:
  1. Store encoded bytes, format, quality score, effective options, and size.
  2. Define a quality target and reject candidates below it.
  3. Rank by smallest bytes, then highest quality, then stable option order.
  4. Return a typed no-acceptable-candidate error when all candidates fail.
- Acceptance criteria:
  - Ranking never chooses a candidate below the configured target.
  - Equal-size ties resolve deterministically.
  - Candidate metadata is sufficient for manifest and result JSON records.
- Test requirements: Accepted/rejected candidates, tie breaks, empty list, and no-valid-candidate tests.
- Risks: A score alone does not detect every artifact; keep artifact detection as a documented future extension.
- Next task: `task-019-candidate-search`.
