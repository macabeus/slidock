# Contributing to slidock

Thanks for your interest. Bug reports with a small deck that shows the problem help the most; pull requests are
welcome too. For a larger change, open an issue first so we can agree on the approach.

Please follow the [code of conduct](CODE_OF_CONDUCT.md). Report security issues privately, as
[SECURITY.md](SECURITY.md) says, not in a public issue.

## Setup

Install the [requirements](README.md#requirements) (rustup, Node 22.18+ with `corepack enable`, wasm-pack 0.15.0,
the Playwright browsers for the browser tests; `git`, `unzip` and `node` on `PATH`), then:

```sh
pnpm install
pnpm wasm:build           # the player, then the compiler's wasm package: run it before pnpm test, typecheck or build
pnpm dev                  # the web app on http://localhost:5173/
cargo build --release -p slidock-cli
```

## Before you send a change

The checks CI runs, all of which must pass (`cargo deny` needs [cargo-deny](https://github.com/EmbarkStudios/cargo-deny);
CI pins 0.20.2):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --target wasm32-unknown-unknown -p slidock-wasm -p slidock-bench-metrics -p slidock-bench-schema
cargo deny check
pnpm format:check
pnpm lint
pnpm typecheck
pnpm test
pnpm -r build
```

While working on one crate or package, the narrower commands are faster: `cargo clippy -p <crate> --all-targets --
-D warnings`, `cargo test -p <crate>`, `cargo build -p <crate> --target wasm32-unknown-unknown` for crates the wasm
build links, and `pnpm --filter <package> lint`, `typecheck`, `test`. A change to the player or the web app also runs
its browser suite: `pnpm --filter @slidock/player e2e`, `pnpm --filter @slidock/web e2e:smoke`.

## Code rules

The compiler builds natively and to `wasm32-unknown-unknown`, and must write the same bytes on both. Clippy and
`clippy.toml` enforce most of these rules:

- **No panics in library crates.** `unwrap`, `expect`, `panic!` and slice indexing are denied outside tests. Errors
  are `Result`s with `thiserror` types.
- **Determinism.** No `HashMap` or `HashSet` (use `IndexMap` or `BTreeMap`), no clock (`Instant` panics on wasm; use
  the injected clock), no randomness, no locale. Transcendental maths only through `slidock-math`.
- **Pure Rust for the wasm build:** no C, no threads, no file system in anything it links.
- **Dependencies** come from the root `Cargo.toml`'s `[workspace.dependencies]`, pinned to exact versions, and a
  crate uses them with `workspace = true`.
- **The output bytes are a contract.** A change to what the compiler writes is either a decision (recorded with new
  goldens, below) or a defect.
- **Idiomatic Rust**: enums, newtypes, traits and `Result` over flags and sentinels.
- **Generated code stays generated.** The TypeScript types in `packages/types/src/generated/` and
  `apps/bench-web/src/schema/` come from the Rust types (`ts-rs`): after changing those, run
  `UPDATE_GOLDEN=1 cargo test -p slidock-wasm --test typescript` (or `-p slidock-bench-schema`). Probe decks in
  `probes/out/` come from `cargo xtask probes-*`.

Comments are concise and describe current behaviour: no history ("used to", "no longer") and no conversation notes.
A module's doc comment states the rule it implements and cites the specification where that helps (ECMA-376, ODF, a
W3C spec). Match the surrounding code.

## Tests

Each rule gets a test, and each new rule a negative control: break the rule, see the test fail, restore it. Read the
`running N tests` and `test result:` lines: a run of 0 tests is a failure.

Test decks are our own: build them in code or add them to `probes/` with the script or xtask that generates them.
Never commit a deck, picture or font you do not have the right to share, and nothing derived from a private deck.

## Goldens and the bench (maintainers)

Some suites read a private corpus of real decks kept outside the repository, under `$SLIDOCK_PRIVATE` (default
`~/.slidock`): the golden files (`cargo xtask golden --check`, and each crate's `--features golden` tests), the corpus
run of the web app's end-to-end suite, and the fidelity bench (`bench/README.md`). They skip, saying so, when the
corpus is absent, so contributors can ignore them; a maintainer runs them before merging. When a change moves the
compiler's output on purpose, the maintainer records new goldens with `cargo xtask golden --record --reason "<why>"`.

## Commits and pull requests

- Commit messages follow [Conventional Commits](https://www.conventionalcommits.org): `feat(pptx): …`,
  `fix(player): …`, `docs: …`, `test: …`, `chore: …`; the scope is the crate or package.
- Keep a pull request to one concern, with tests, and say in its description what changed for users.
- Contributions are licensed under the [MIT License](LICENSE), as the project is.
