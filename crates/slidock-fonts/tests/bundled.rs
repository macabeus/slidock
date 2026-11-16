//! The bundled substitutes as the hosts ship them (`slidock-substitutes`, the WOFF2 files the web app serves and the
//! CLI embeds): every face maps [`BUNDLED_COVERAGE`], and the last-resort face is among them.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use slidock_fonts::catalog::substitutes::LAST_RESORT;
use slidock_fonts::glyphs::{BUNDLED_COVERAGE, bundled_covers};
use slidock_fonts::sfnt::covered;
use slidock_fonts::{CodepointSet, Hint, container, read_sfnt};

/// Every bundled face's sfnt, by file name, in file name order.
fn bundled_faces() -> Vec<(String, Vec<u8>)> {
    let mut faces: Vec<&slidock_substitutes::Face> = slidock_substitutes::FACES.iter().collect();
    faces.sort_by_key(|face| face.file);
    assert_eq!(
        faces.len(),
        24,
        "Liberation ×12, Carlito, Caladea, DejaVu Sans ×4"
    );
    faces
        .into_iter()
        .map(|face| {
            let sfnt = container::unwrap(face.bytes, &Hint::default())
                .unwrap()
                .sfnt;
            (face.file.to_owned(), sfnt)
        })
        .collect()
}

fn coverage() -> CodepointSet {
    let mut set = CodepointSet::new();
    for (first, last) in BUNDLED_COVERAGE {
        set.insert_range(first, last);
    }
    set
}

#[test]
fn every_bundled_face_maps_the_bundled_coverage() {
    let wanted = coverage();
    for (name, sfnt) in bundled_faces() {
        let mapped = covered(&sfnt, 0, &wanted).unwrap();
        assert_eq!(
            wanted.difference(&mapped).ranges(),
            [],
            "{name} lacks part of BUNDLED_COVERAGE"
        );
    }
    // A code point one of them lacks is outside it.
    let mut hyphen = CodepointSet::new();
    hyphen.insert(0x2010);
    let lacking = bundled_faces()
        .into_iter()
        .filter(|(_, sfnt)| covered(sfnt, 0, &hyphen).unwrap().is_empty())
        .count();
    assert!(lacking > 0 && !bundled_covers(0x2010));
}

#[test]
fn the_last_resort_face_is_bundled_and_draws_the_corpus_bullets() {
    let (family, postscript_name) = LAST_RESORT;
    let (_, sfnt) = bundled_faces()
        .into_iter()
        .find(|(_, sfnt)| {
            read_sfnt(sfnt)
                .unwrap()
                .iter()
                .any(|face| face.postscript_name == postscript_name && face.family == family)
        })
        .expect("the last-resort face is among the bundled files");
    // ❖ ☺ ●, bullets decks set in faces that lack them.
    let bullets: CodepointSet = [0x2756, 0x263a, 0x25cf].into_iter().collect();
    assert_eq!(covered(&sfnt, 0, &bullets).unwrap(), bullets);
}
