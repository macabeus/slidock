//! The corpus against the golden files (`--features golden`, tests/golden; skips when they are absent): every Keynote
//! pair, compiled natively from a file reader through the frontend, the standard passes (with the static lane's font
//! set added as user fonts) and the HTML backend, gives what `slidock compile` recorded: the resolved IR, every
//! document and medium, and the report, byte for byte. The 1 GB talk also reads in Keynote's own slide order and
//! lowers every shadow Keynote stores in it.
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

#[path = "../../../tests/golden/support.rs"]
mod golden;

#[path = "../../../tests/golden/compile.rs"]
mod golden_compile;

use golden::Golden;
use slidock_core::{FontSource, Session, SessionOptions};
use slidock_core_api::Input;
use slidock_io::FileReader;
use slidock_keynote::open_document;
use support::harness::setup;

#[test]
fn every_keynote_corpus_pair_compiles_as_recorded() {
    let Some(golden) = Golden::open("slidock-keynote corpus") else {
        return;
    };
    let pairs = golden_compile::pairs_of(&golden, "keynote");
    assert_eq!(pairs.len(), 4, "the corpus holds four Keynote decks");
    let fonts = golden_compile::font_files(&golden);
    let only = std::env::var("CORPUS_ONLY").ok();
    let mut failures = Vec::new();
    let mut compared = 0;
    for pair in &pairs {
        if only
            .as_deref()
            .is_some_and(|only| only != pair.presentation)
        {
            continue;
        }
        let source = golden.source(pair);
        let reader = FileReader::open(&source).unwrap();
        let file_name = source.file_name().unwrap().to_str().unwrap().to_owned();
        let (registry, pipeline) = setup();
        let mut session = Session::open(
            &Input::new(&file_name, &reader),
            SessionOptions::new(&registry, &pipeline),
        )
        .unwrap();
        for font in &fonts {
            session.add_font(font.clone(), FontSource::User).unwrap();
        }
        let finished = session.finish().unwrap();
        let (differences, files) =
            golden_compile::compare_pair(&golden, pair, &session, &finished, &reader);
        eprintln!("{}: {files} files, {} differ", pair.name, differences.len());
        failures.extend(differences);
        compared += 1;
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
    assert!(compared > 0);
}

/// The talk's builds that exist only as `should_merge` records, and the index files of their slides.
const MERGED_BUILDS: [(u64, &str); 3] = [
    (5_213_335, "Index/Slide-5209143.iwa"),
    (5_220_478, "Index/Slide-5220474.iwa"),
    (5_225_405, "Index/Slide-5225404.iwa"),
];

#[test]
fn the_talk_reads_in_keynotes_own_order_from_about_a_megabyte_of_its_index() {
    let Some(golden) = Golden::open("slidock-keynote talk order") else {
        return;
    };
    let path = golden.input("corpus:retro.keynote");
    let reader = FileReader::open(&path).unwrap();
    let input = Input::new("talk.key", &reader);
    let limits = slidock_core_api::Limits::default();
    let document = open_document(&input, &limits).unwrap();
    assert!(document.pkg.entries().count() > 800);
    assert!(document.index_bytes_read() <= 2 * 1024 * 1024);
    assert_eq!(document.graph.stats.decode_errors, 0);
    assert!(document.graph.stats.unknown_type_ids.is_empty());
    assert_eq!(document.graph.stats.merge_records, 3);

    let lowered = support::harness::lower_reader(&reader, None, limits);
    let presentation = &lowered.presentation;
    // Keynote's own view of the talk (AppleScript): slide number, skipped, master name.
    let order =
        std::fs::read_to_string(golden.fixture("keynote/en-retro-game-decompilation.order.tsv"))
            .unwrap();
    let expected: Vec<Vec<String>> = order
        .trim()
        .lines()
        .map(|line| line.split('\t').map(str::to_owned).collect())
        .collect();
    let actual: Vec<Vec<String>> = presentation
        .slides
        .iter()
        .enumerate()
        .map(|(index, slide)| {
            let layout = slide
                .layout
                .as_ref()
                .and_then(|layout| presentation.layouts.get(layout))
                .and_then(|layout| layout.name.clone())
                .unwrap_or_default();
            vec![(index + 1).to_string(), slide.hidden.to_string(), layout]
        })
        .collect();
    assert_eq!(actual, expected);
    assert_eq!(presentation.slides.len(), 197);
    assert_eq!(
        presentation
            .slides
            .iter()
            .filter(|slide| slide.hidden)
            .count(),
        35
    );
    assert_eq!(
        presentation
            .slides
            .iter()
            .filter(|slide| slide.notes.is_some())
            .count(),
        70
    );
    assert!(
        presentation
            .slides
            .iter()
            .all(|slide| slide.failed.is_none())
    );
    assert_eq!(presentation.media.len(), 202);
    for (build, file) in MERGED_BUILDS {
        let drawable = document
            .graph
            .get_id::<slidock_keynote::proto::kn::BuildArchive>(slidock_keynote::ObjectId(build))
            .and_then(|build| build.drawable.as_ref())
            .and_then(|drawable| drawable.identifier)
            .unwrap();
        let slide = presentation
            .slides
            .iter()
            .find(|slide| matches!(&slide.origin, slidock_ir::origin::Origin::Source { part, .. } if part == file))
            .unwrap();
        let target = format!("kn-{drawable}");
        assert!(
            slide
                .timeline
                .iter()
                .any(|step| step.target.element.as_str() == target)
        );
    }
}

#[test]
fn the_talk_lowers_every_shadow_it_draws() {
    let Some(golden) = Golden::open("slidock-keynote talk shadows") else {
        return;
    };
    let path = golden.input("corpus:retro.keynote");
    let reader = FileReader::open(&path).unwrap();
    let lowered =
        support::harness::lower_reader(&reader, None, slidock_core_api::Limits::default());
    fn count(elements: &[slidock_ir::source::Element], kinds: &mut Vec<&'static str>) {
        use slidock_ir::source::Element;
        for element in elements {
            if let Some(Some(_)) = element.shadow() {
                kinds.push(match element {
                    Element::Shape(_) => "shape",
                    Element::Picture(_) => "picture",
                    Element::Media(_) => "media",
                    _ => "other",
                });
            }
            if let Element::Group(group) = element {
                count(&group.children, kinds);
            }
        }
    }
    let mut kinds = Vec::new();
    for slide in &lowered.presentation.slides {
        count(&slide.elements, &mut kinds);
    }
    let of = |kind| kinds.iter().filter(|k| **k == kind).count();
    // Keynote's stored shadows (decoded from the file): 23 pictures, 15 text shapes, 3 movies, all drop shadows.
    assert_eq!((of("picture"), of("shape"), of("media")), (23, 15, 3));
    assert!(!lowered.codes().contains(&"KN1304"));
}
