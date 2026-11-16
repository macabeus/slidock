# slidock — notes for coding agents

[CONTRIBUTING.md](CONTRIBUTING.md) is the source of the rules (setup, checks, code rules, tests, commits); this file
adds what an agent working in the repository needs. [docs/format.md](docs/format.md) specifies the deck format and
[docs/decisions.md](docs/decisions.md) the standing decisions.

## Layout

- Rust crates in `crates/` and `xtask/`; TypeScript in `packages/{types,player,wasm,ui}` and `apps/{web,bench-web}`.
- Work on one crate or package at a time and keep shared files (root `Cargo.toml`, `Cargo.lock`, `pnpm-lock.yaml`,
  `packages/types`, `.github/`) to changes the task needs. A dependency missing from `[workspace.dependencies]` goes
  into the crate only, with an exact version, and is called out in the change's description.
- Parallel agents each work in their own git worktree with `CARGO_TARGET_DIR=<worktree>/target`, commit on their own
  branch, and never push.

## Rust

- Library crates never panic: clippy denies `unwrap_used`, `expect_used`, `panic`, `indexing_slicing` (tests may; see
  `clippy.toml`). Errors are `Result`s with `thiserror` types.
- Determinism: no `HashMap`/`HashSet` (use `IndexMap`/`BTreeMap`), no clock (`Instant` panics on wasm; use the
  injected clock), no randomness, no locale. Transcendental maths only through `slidock-math` (fdlibm). All enforced
  by `clippy.toml`.
- Everything the wasm build links builds for `wasm32-unknown-unknown`: pure Rust, no C, no threads, no fs.
- The parity path's output bytes are a contract: a change to them is a decision, recorded by re-recording the goldens
  with a reason, or a defect.
- Corpus-derived data never enters the repository, and corpus decks are named by their neutral presentation id only.
  slidock's goldens (its own outputs, under `$SLIDOCK_PRIVATE/golden/`, default `~/.slidock`) are pinned by SHA-256 in
  `tests/golden/manifest.json`, which names a corpus source as `corpus:<presentation>.<frontend>`. Tests that read
  private files sit behind each crate's `golden` feature and skip, saying so, when the files are absent;
  `cargo xtask golden --check` verifies every pinned file and runs every golden suite.

## Comments

Concise, describing current behaviour. No history ("ported from", "no longer") and no conversation notes. A module
doc states the rule it implements, citing the specification (ECMA-376, ODF, a W3C spec) where that helps a
maintainer. Match the surrounding code.

## Before a commit

- `cargo fmt --check`
- `cargo clippy -p <crate> --all-targets -- -D warnings`
- `cargo test -p <crate>` (and `--features golden` where it exists), reading the `running N tests` lines: a run of 0
  tests is a failure
- `cargo build -p <crate> --target wasm32-unknown-unknown` for crates the wasm build links
- TS: `pnpm --filter <pkg> lint`, `typecheck`, `test`; `pnpm format:check`

Commits use conventional messages.
