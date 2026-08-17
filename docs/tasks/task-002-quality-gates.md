# Task 002: Add Quality Gates

- Status: DONE
- Priority: P0
- Dependencies: 001
- Goal: Make formatting, linting, and tests reproducible for every later task.
- Scope: Rust toolchain pinning, formatting configuration, CI checks, and documented commands.
- Out of scope: Release packaging, deployment, benchmark data, and application behavior.
- Files to read: `Cargo.toml`, `docs/SPEC.md`.
- Files to modify: `Cargo.toml` only if lint configuration needs a workspace setting.
- Files to create: `rust-toolchain.toml`, `rustfmt.toml` only when needed, `.github/workflows/ci.yml`.
- Steps:
  1. Pin a stable toolchain that is available in CI.
  2. Configure CI to run format check, clippy with warnings denied, and workspace tests.
  3. Use a clean checkout and cache only Cargo artifacts.
  4. Run every command locally before handing off.
- Acceptance criteria:
  - CI commands exactly match the commands in `docs/SPEC.md`.
  - A deliberate formatting or warning failure would fail CI.
  - No secrets or network services are needed for the baseline checks.
- Test requirements: `cargo fmt --check`, `cargo clippy --all-targets --all-features -- -D warnings`, `cargo test --workspace`.
- Risks: Pinning an unavailable toolchain can block all work; verify the toolchain exists before editing CI.
- Next task: `task-003-core-error-contract`.
