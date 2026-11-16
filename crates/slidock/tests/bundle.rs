//! `slidock::compile` end to end: an input through its frontend, the passes, the backend and the bundler into a
//! zip that slidock-io reads back, byte for byte what a host gets running the session steps itself.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use serde_json::Value;
use slidock::{
    BundleError, BundleOptions, BundleSink, CompiledDeck, Frontends, Input, MediaFiles, Options,
    Session, SessionMedia, SessionOptions, StoreMedia, VerifyMode, bundle, compile, finish_deck,
    lint_templates, pipeline, slide_templates, slidock_generator,
};
use slidock_format::dom::{Document, NodeId};
use slidock_io::BytesReader;
use slidock_io::zip::{ZipArchive, ZipLimits};
use support::*;

#[derive(Default)]
struct Collect(Vec<u8>);

impl BundleSink for Collect {
    fn write(&mut self, _: &str, bytes: &[u8]) -> Result<(), BundleError> {
        self.0.extend_from_slice(bytes);
        Ok(())
    }
}

fn options() -> Options<'static> {
    Options {
        verify: VerifyMode::Each,
        generator: slidock_generator(),
        ..Options::default()
    }
}

/// The chain presentation with a BMP on its second slide, which plan-media transcodes to PNG.
fn deck_with_media() -> (Value, MediaFiles) {
    let bmp = red_bmp();
    let mut p = chain();
    let sha = add_media(&mut p, &bmp, "image/bmp", 1, 1);
    p["slides"][1]["elements"]
        .as_array_mut()
        .unwrap()
        .push(picture(
            "ir-s2.pic",
            frame(100.0, 100.0, 100.0, 100.0),
            &sha,
        ));
    (p, media_files(&[(&bmp, "bmp")]))
}

fn compile_zip(p: &Value, media: MediaFiles) -> (slidock::Bundled, Vec<u8>) {
    let reader = BytesReader::new(bytes(p));
    let mut sink = Collect::default();
    let bundled = compile(
        &Input::new(NAME, &reader),
        Frontends::default().with_ir_media(media),
        &options(),
        &BundleOptions::default(),
        &mut sink,
    )
    .unwrap();
    (bundled, sink.0)
}

#[test]
fn compiles_an_input_into_a_zip_with_every_slide_and_its_media() {
    let (p, media) = deck_with_media();
    let (bundled, zip) = compile_zip(&p, media.clone());
    assert_eq!(compile_zip(&p, media).1, zip, "deterministic");
    assert_eq!(bundled.report.bytes, zip.len() as u64);
    assert_eq!(
        bundled.deck.player,
        Some(slidock::embedded_player(true).unwrap())
    );

    let archive = ZipArchive::open(BytesReader::new(zip), ZipLimits::default()).unwrap();
    let names: Vec<&str> = archive.entries().iter().map(|e| e.name.as_str()).collect();
    let png = format!("media/{}.png", sha256(&red_bmp()));
    assert_eq!(names, ["index.html", png.as_str(), "LICENSES.txt"]);
    let written = archive.read(&archive.entries()[1]).unwrap();
    assert!(written.starts_with(b"\x89PNG\r\n\x1a\n"));
    assert_eq!(bundled.report.files[1].bytes, written.len() as u64);

    let index = archive.read(&archive.entries()[0]).unwrap();
    let page = Document::parse(std::str::from_utf8(&index).unwrap());
    let templates: Vec<&str> = page
        .walk(NodeId::DOCUMENT)
        .into_iter()
        .filter_map(|e| page.attribute(e, "data-sd-slide"))
        .collect();
    assert_eq!(templates, ["ir-s1", "ir-s2", "ir-s3"]);
    let index = std::str::from_utf8(&index).unwrap();
    assert!(index.contains(&format!(" src=\"{png}\"")));
    // The layout's box and the master's band sit behind the first slide's title.
    let first = index
        .split("<template data-sd-slide=\"ir-s1\">")
        .nth(1)
        .unwrap();
    let first = first.split("</template>").next().unwrap();
    let order: Vec<usize> = ["ir-m.band", "ir-l.box", "ir-s1.title"]
        .iter()
        .map(|id| first.find(&format!("data-el=\"{id}\"")).unwrap())
        .collect();
    assert!(order.windows(2).all(|pair| pair[0] < pair[1]), "{order:?}");
}

#[test]
fn is_the_session_steps_a_host_runs_and_its_templates_lint_clean() {
    let (p, media) = deck_with_media();
    let (bundled, zip) = compile_zip(&p, media.clone());

    let registry = Frontends::default()
        .with_ir_media(media)
        .registry()
        .unwrap();
    let pipeline = pipeline().unwrap();
    let reader = BytesReader::new(bytes(&p));
    let input = Input::new(NAME, &reader);
    let options = options();
    let session_options = SessionOptions {
        verify: options.verify,
        ..SessionOptions::new(&registry, &pipeline)
    };
    let mut session = Session::open(&input, session_options).unwrap();
    let finished = session.finish().unwrap();
    let finished = finish_deck(&session, &finished, slidock_generator()).unwrap();
    assert_eq!(finished.diagnostics, bundled.diagnostics);
    let store = SessionMedia(&session);
    let media = StoreMedia::new(
        &store,
        &reader,
        &finished.documents,
        options.policy.transcode_pixels,
    );
    let compiled = CompiledDeck {
        deck: finished.deck,
        documents: finished.documents,
        media: &media,
        fonts: Vec::new(),
    };
    let mut sink = Collect::default();
    let report = bundle(&compiled, &BundleOptions::default(), &mut sink).unwrap();
    assert_eq!(sink.0, zip);
    assert_eq!(report, bundled.report);

    let templates = slide_templates(&compiled).unwrap();
    assert_eq!(lint_templates(&compiled.documents, &templates).unwrap(), []);
}

#[test]
fn refuses_a_presentation_that_fails_as_a_whole() {
    let reader = BytesReader::new(b"{ not json".to_vec());
    let result = compile(
        &Input::new(NAME, &reader),
        Frontends::default(),
        &options(),
        &BundleOptions::default(),
        &mut Collect::default(),
    );
    assert!(result.is_err());
}

/// The motion fixture of `slidock-ir` through `slidock::compile`, with native motion on and off, and the session
/// options a host that takes fonts uses (the wasm host's) keeping it.
#[test]
fn native_motion_reaches_the_deck_only_when_asked() {
    let fixture = std::fs::read(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../slidock-ir/tests/data/motion.presentation-ir.json"),
    )
    .unwrap();
    let deck = |native_motion| {
        let reader = BytesReader::new(fixture.clone());
        let mut sink = Collect::default();
        compile(
            &Input::new("motion.presentation-ir.json", &reader),
            Frontends::default(),
            &Options {
                native_motion,
                ..options()
            },
            &BundleOptions::default(),
            &mut sink,
        )
        .unwrap()
        .deck
    };
    assert!(!Options::default().native_motion);
    let on = serde_json::to_value(deck(true).slides).unwrap();
    assert_eq!(on[0]["timeline"][0]["effect"], "fade");
    assert_eq!(on[1]["transition"]["effect"], "morph");
    let off = serde_json::to_value(deck(false).slides).unwrap();
    assert!(off[0]["timeline"][0].get("effect").is_none());
    assert!(off[1]["transition"].get("effect").is_none());
    assert_eq!(off[0]["timeline"][0]["native"], "ir:fade");
    let registry = Frontends::default().registry().unwrap();
    let pipeline = pipeline().unwrap();
    assert!(slidock::fonts::with_fonts(&registry, &pipeline).native_motion);
    assert!(!SessionOptions::new(&registry, &pipeline).native_motion);
}
