//! The compiler runs on a 1 MiB stack, the stack of a wasm module and of a browser worker, even in a debug build
//! (`--features golden`, from `$SLIDOCK_PRIVATE/corpus` as the golden files pin it; skips when they are absent).
#![cfg(feature = "golden")]
#![allow(clippy::unwrap_used, clippy::panic, reason = "test code")]

#[path = "../../../tests/golden/support.rs"]
mod golden;

use std::path::PathBuf;

use slidock::{
    Frontends, Input, Session, SessionOptions, finish_deck, pipeline, slidock_generator,
};

/// The stack a wasm module gets by default.
const STACK: usize = 1 << 20;

/// One small corpus deck per frontend.
const DECKS: [(&str, &str); 3] = [
    ("corpus:google-export.pptx", "pptx"),
    ("corpus:google-export.odp", "odp"),
    ("corpus:keynote-basic.keynote", "keynote"),
];

/// open → finish → print → deck JSON, as a host runs a session, returning the number of slides.
fn compile(path: &PathBuf, frontend: &str) -> usize {
    let registry = Frontends::default().registry().unwrap();
    let pipeline = pipeline().unwrap();
    let reader = slidock_io::FileReader::open(path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let mut session = Session::open(
        &Input::new(name, &reader),
        SessionOptions::new(&registry, &pipeline),
    )
    .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    assert_eq!(session.provenance().format.as_str(), frontend, "{name}");
    let finished = session.finish().unwrap();
    let deck = finish_deck(&session, &finished, slidock_generator()).unwrap();
    deck.deck.to_json().unwrap();
    deck.deck.slides.len()
}

#[test]
fn compiles_a_deck_of_each_frontend_on_a_one_mebibyte_stack() {
    let Some(golden) = golden::Golden::open("slidock stack") else {
        return;
    };
    for (file, frontend) in DECKS {
        let path = golden.input(file);
        let slides = std::thread::Builder::new()
            .name(file.to_owned())
            .stack_size(STACK)
            .spawn(move || compile(&path, frontend))
            .unwrap()
            .join()
            .unwrap_or_else(|_| panic!("{file} failed on a {STACK}-byte stack"));
        assert!(slides > 0, "{file}");
    }
}
