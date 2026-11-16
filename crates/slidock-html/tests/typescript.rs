//! The committed TypeScript declarations of the deck JSON (`packages/types/src/generated/{deck,ir}`) are the ones
//! the Rust types generate. `slidock-wasm`'s test of the same name owns the folder and rewrites it
//! (`UPDATE_GOLDEN=1 cargo test -p slidock-wasm --test typescript`).
#![allow(clippy::unwrap_used, reason = "test code")]

use std::path::Path;

use slidock_html::deck::{Deck, DeckDiagnostic};
use slidock_html::typescript::{Bindings, GENERATED_DIR};

#[test]
fn the_committed_declarations_are_the_generated_ones() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../packages/types/src")
        .join(GENERATED_DIR);
    let mut bindings = Bindings::new();
    bindings
        .add::<Deck>()
        .unwrap()
        .add::<DeckDiagnostic>()
        .unwrap();
    let generated = bindings.files();
    assert!(generated.contains_key("deck/Deck.ts") && generated.contains_key("ir/Trigger.ts"));
    let drift: Vec<&String> = generated
        .iter()
        .filter(|(path, _)| !path.ends_with("/index.ts"))
        .filter(|(path, text)| {
            std::fs::read_to_string(root.join(path)).ok().as_ref() != Some(*text)
        })
        .map(|(path, _)| path)
        .collect();
    assert!(
        drift.is_empty(),
        "packages/types drifted from the deck types in {drift:?}; rerun slidock-wasm's typescript test with \
         UPDATE_GOLDEN=1"
    );
}
