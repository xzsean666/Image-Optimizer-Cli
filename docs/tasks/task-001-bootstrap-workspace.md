# Task 001: Bootstrap Cargo Workspace

- Status: DONE
- Priority: P0
- Dependencies: none
- Goal: Create a compiling Cargo workspace with the six planned crates and no image-processing behavior.
- Scope: Workspace metadata, crate manifests, minimal library exports, and a minimal CLI binary.
- Out of scope: Codec code, CLI subcommands, storage clients, and runtime configuration.
- Files to read: `docs/ARCHITECTURE.md`, `docs/SPEC.md`.
- Files to modify: none.
- Files to create: `Cargo.toml`, `crates/*/Cargo.toml`, each crate's `src/lib.rs` where applicable, `crates/imgopt-cli/src/main.rs`.
- Steps:
  1. Define the workspace members and resolver in the root manifest.
  2. Create `imgopt-core`, `imgopt-codecs`, `imgopt-quality`, `imgopt-manifest`, `imgopt-storage`, and `imgopt-cli`.
  3. Keep dependencies minimal; only add the CLI runtime dependency required to compile a placeholder binary.
  4. Export a placeholder version constant from library crates so dependency wiring is testable.
- Acceptance criteria:
  - `cargo metadata --no-deps` lists exactly the six planned crates.
  - `cargo build --workspace` succeeds.
  - The binary starts and exits successfully with `--help` or a placeholder message.
  - No crate depends on another crate in the reverse direction from `docs/ARCHITECTURE.md`.
- Test requirements: Run `cargo build --workspace` and `cargo test --workspace`.
- Risks: Workspace naming or crate paths become public conventions; keep names stable.
- Next task: `task-002-quality-gates`.
