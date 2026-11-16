## What changed

<!-- What this changes for users, and why. Link the issue it fixes. -->

## Checks

- [ ] `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D warnings`, `cargo test --workspace`
- [ ] `pnpm format:check`, `pnpm lint`, `pnpm typecheck`, `pnpm test` (after `pnpm wasm:build`)
- [ ] A test for each new rule, with its negative control
- [ ] No deck, picture or font I do not have the right to share, and nothing derived from a private deck
- [ ] If the compiler's output bytes change on purpose, the description says so (the goldens are re-recorded by a
      maintainer)
