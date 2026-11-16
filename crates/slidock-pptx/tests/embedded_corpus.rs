//! The corpus deck that embeds fonts (`pptx-talk`, read from `$SLIDOCK_PRIVATE/corpus` as the golden files pin it;
//! `--features golden`, skips when they are absent): its 20 font parts are handed on with the families and
//! faces `p:embeddedFontLst` declares, each an Embedded OpenType structure.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

#[path = "../../../tests/golden/support.rs"]
mod golden;

use std::collections::BTreeMap;
use std::path::PathBuf;

use slidock_core_api::frontend::FontContainer;
use slidock_core_api::{
    Deadline, Frontend, FrontendCtx, Input, Limits, LowerOptions, MediaSource, Sink,
};
use slidock_io::FileReader;
use slidock_ir::diagnostics::Stage;
use slidock_ir::fonts::FaceStyle;
use slidock_pptx::PptxFrontend;

const DECK: &str = "corpus:pptx-talk.pptx";

/// The pinned corpus deck, or `None` when the golden files are absent.
fn corpus_file(input: &str) -> Option<PathBuf> {
    golden::Golden::open("slidock-pptx embedded fonts").map(|golden| golden.input(input))
}

#[test]
fn hands_on_the_decks_twenty_eot_parts() {
    let Some(path) = corpus_file(DECK) else {
        return;
    };
    let name = path.file_name().unwrap().to_str().unwrap().to_owned();
    let reader = FileReader::open(path).unwrap();
    let input = Input::new(&name, &reader);
    let limits = Limits::default();
    let mut out = Vec::new();
    let mut sink = Sink::new(&mut out, Stage::Frontend, "pptx");
    let mut ctx = FrontendCtx {
        limits: &limits,
        diag: &mut sink,
        deadline: Deadline::never(),
        spent: slidock_core_api::Output::default(),
    };
    let frontend = PptxFrontend;
    let analysis = frontend.analyze(&input, &mut ctx).unwrap();
    let facts = analysis.facts();
    let lowered = frontend
        .lower(
            &input,
            &mut ctx,
            &LowerOptions {
                facts: &facts,
                only: None,
                native_motion: false,
            },
        )
        .unwrap();
    assert_eq!(lowered.fonts.len(), 20);
    let mut faces: BTreeMap<&str, Vec<(u32, bool)>> = BTreeMap::new();
    for font in &lowered.fonts {
        assert_eq!(font.container, FontContainer::Eot);
        assert_eq!(font.faces.len(), 1);
        let face = &font.faces[0];
        faces
            .entry(font.family.as_str())
            .or_default()
            .push((face.weight, face.style == FaceStyle::Italic));
        // Each part is DEFLATE in this deck: handed on compressed, and an EOT once inflated.
        let MediaSource::Reader(part) = &font.source else {
            panic!("{}: {:?}", font.family, font.source);
        };
        let bytes = part.read_at(0, part.len() as usize).unwrap();
        let u32_at = |at: usize| u32::from_le_bytes(bytes[at..at + 4].try_into().unwrap());
        assert_eq!(u64::from(u32_at(0)), part.len(), "EOTSize");
        assert_eq!(u32_at(8), 0x0002_0002, "EOT version");
        assert_eq!(&bytes[34..36], &[0x4c, 0x50], "EOT magic");
    }
    let all = vec![(400, false), (700, false), (400, true), (700, true)];
    let upright = vec![(400, false), (700, false)];
    assert_eq!(
        faces,
        BTreeMap::from([
            ("Barlow Condensed", all.clone()),
            ("Oswald", upright.clone()),
            ("Oswald Medium", upright),
            ("Roboto", all.clone()),
            ("Roboto Medium", all.clone()),
            ("Roboto Mono", all),
        ])
    );
}
