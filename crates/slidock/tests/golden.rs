//! The golden files through the facade (`--features golden`, tests/golden; skips when they are absent): every private
//! IR fixture, compiled with `--emit documents` semantics, prints the document set `slidock compile` recorded, the
//! backend report slidock-html recorded and the resolved IR and diagnostics slidock-passes recorded, byte for byte; the
//! full resolved fixture, printed straight from the backend, goes through the driver's print step. The rest are the
//! driver's tests on those fixtures, and the driver's loss check, lint, deck JSON and bundle over every PPTX, ODP and
//! Keynote source of the corpus (`slidock-cli`'s golden suite compares their outputs with the recorded ones).
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

#[path = "../../../tests/golden/support.rs"]
mod golden;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use golden::Golden;

use indexmap::IndexMap;
use serde::Serialize;
use serde_json::Value;
use slidock::{
    Compiled, Compiler, Emitted, Frontends, Generator, Input, MediaAction, MediaFiles, MediaHandle,
    MediaSource, MediaStore, Options, PrintInput, Printed, VerifyMode, print_presentation,
    print_presentation_with,
};
use slidock_core_api::EmptyMediaStore;
use slidock_html::{EmitOptions, HtmlError, emit_documents};
use slidock_io::BytesReader;
use slidock_ir::diagnostics::{Diagnostic, Severity, Stage};
use slidock_ir::features::{FeatureId, census_presentation};
use slidock_ir::fonts::font_key;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::{parse, resolved};

/// The golden files, opened once; `None` when they are absent.
fn golden() -> Option<&'static Golden> {
    static GOLDEN: OnceLock<Option<Golden>> = OnceLock::new();
    GOLDEN.get_or_init(|| Golden::open("slidock")).as_ref()
}

fn opened() -> &'static Golden {
    golden().expect("the golden files are present")
}

/// The IR fixture names, sorted.
fn fixture_names() -> Vec<String> {
    let names = opened().ir_fixtures();
    assert_eq!(names.len(), 6, "the private fixtures hold six IR fixtures");
    names
}

fn fixture_path(name: &str) -> PathBuf {
    opened().fixture(&format!("ir/{name}{}", slidock::IR_FILE_SUFFIX))
}

/// The `media/` folder beside the fixtures, as a media set.
fn fixture_media() -> MediaFiles {
    let mut media = MediaFiles::new();
    for name in opened().fixture_names("ir/media") {
        let path = opened().fixture(&format!("ir/media/{name}"));
        assert!(
            media.insert_bytes(&name, std::fs::read(&path).unwrap()),
            "{name}"
        );
    }
    media
}

/// The generator the CLI names.
fn generator() -> Generator {
    slidock::slidock_generator()
}

fn options() -> Options<'static> {
    Options {
        generator: generator(),
        ..Options::default()
    }
}

fn compile_fixture_with(name: &str, media: MediaFiles, options: &Options<'_>) -> Compiled {
    let path = fixture_path(name);
    let compiler = Compiler::new(Frontends::default().with_ir_media(media)).unwrap();
    let reader = BytesReader::new(std::fs::read(&path).unwrap());
    let file_name = path.file_name().unwrap().to_str().unwrap().to_owned();
    compiler
        .compile(&Input::new(&file_name, &reader), options)
        .unwrap_or_else(|e| panic!("{name}: {e}"))
}

fn compile_fixture(name: &str) -> Compiled {
    compile_fixture_with(name, fixture_media(), &options())
}

/// Compares a document set with the one recorded under `<prefix>/` (its documents, not its media, IR or report) and its
/// backend report with `report`; returns the files compared.
fn compare_documents(
    prefix: &str,
    report: &str,
    printed: &Printed,
    failures: &mut Vec<String>,
) -> usize {
    let golden = opened();
    let documents = printed
        .documents
        .as_ref()
        .unwrap_or_else(|| panic!("{prefix} printed no documents"));
    let listed: Vec<String> = golden
        .output_paths(prefix)
        .into_iter()
        .filter(|path| {
            !path.starts_with("media/") && path != "presentation-ir.json" && path != "report.json"
        })
        .collect();
    let printed_paths: Vec<&String> = documents.text.keys().collect();
    assert_eq!(
        printed_paths,
        listed.iter().collect::<Vec<_>>(),
        "{prefix}: file set"
    );
    let mut compared = 0;
    for (path, text) in &documents.text {
        compared += 1;
        failures.extend(golden.compare(&format!("{prefix}/{path}"), text.as_bytes()));
    }
    compared += 1;
    let actual = printed.documents_report().unwrap().unwrap();
    failures.extend(golden.compare(report, actual.as_bytes()));
    compared
}

/// What slidock-passes records: the resolved presentation and the compile's sorted diagnostics.
#[derive(Serialize)]
struct Resolved<'a> {
    presentation: Option<&'a resolved::Presentation>,
    diagnostics: &'a [Diagnostic],
}

fn resolved_bytes(compiled: &Compiled) -> String {
    slidock_ir::canonical::to_json_pretty(&Resolved {
        presentation: compiled.result.presentation.as_ref(),
        diagnostics: &compiled.result.diagnostics,
    })
    .unwrap()
}

#[test]
fn every_ir_fixture_reproduces_its_goldens_through_the_facade() {
    let Some(golden) = golden() else {
        return;
    };
    let mut failures = Vec::new();
    let mut compared = 0;
    let names = fixture_names();
    for name in &names {
        let compiled = compile_fixture(name);
        let driver: Vec<&Diagnostic> = compiled
            .diagnostics
            .iter()
            .filter(|d| d.stage == Stage::Driver || d.severity == Severity::Error)
            .collect();
        assert!(driver.is_empty(), "{name}: {driver:?}");
        assert_eq!(compiled.documents().unwrap().lint(), vec![], "{name}");
        compared += compare_documents(
            &format!("fixtures/{name}"),
            &format!("slidock-html/{name}.report.json"),
            compiled.printed.as_ref().unwrap(),
            &mut failures,
        );
        compared += 1;
        failures.extend(golden.compare(
            &format!("slidock-passes/resolved/{name}.json"),
            resolved_bytes(&compiled).as_bytes(),
        ));
    }
    eprintln!(
        "{} IR fixtures, {compared} files compared byte for byte, {} differ",
        names.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

/// Answers that every media item of a presentation is present, with no bytes: all printing reads of a store.
struct Present(Vec<Sha256>);

impl MediaStore for Present {
    fn open(&self, sha256: &Sha256) -> Option<MediaHandle> {
        self.0.contains(sha256).then(|| MediaHandle {
            bytes: 0,
            source: MediaSource::Bytes(Vec::new().into()),
        })
    }
}

#[test]
fn the_full_resolved_fixture_prints_its_golden_through_the_driver() {
    if golden().is_none() {
        return;
    }
    let path = opened().fixture("presentation/resolved/full.presentation-ir.json");
    let bytes = std::fs::read(&path).unwrap();
    let presentation: resolved::Presentation =
        parse::from_str(std::str::from_utf8(&bytes).unwrap()).unwrap();
    // The fixture is already resolved: its source is itself, with the core's Foreign diagnostics, and its media
    // come without bytes, so the store only answers that each item is there.
    let source = presentation.to_source().unwrap();
    let diagnostics = slidock_core::foreign_diagnostics(&source);
    let features = census_presentation(&source);
    let emit = EmitOptions {
        provenance: slidock_html::Provenance {
            format: "ir".to_owned(),
            name: "full.presentation-ir.json".to_owned(),
            sha256: slidock_canon::sha256_hex(&bytes),
            bytes: bytes.len() as u64,
        },
        generator: generator(),
    };
    let store = Present(presentation.media.keys().copied().collect());
    let printed = print_presentation(&PrintInput {
        presentation: &presentation,
        source: &source,
        emit: &emit,
        features: &features,
        diagnostics: &diagnostics,
        media: &store,
    })
    .unwrap();
    assert!(printed.driver.is_empty(), "{:?}", printed.driver);
    let mut failures = Vec::new();
    let compared = compare_documents(
        "slidock-html/full",
        "slidock-html/full.report.json",
        &printed,
        &mut failures,
    );
    eprintln!("the full resolved fixture: {compared} files compared byte for byte");
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}

#[test]
fn is_deterministic_a_second_run_in_reverse_order_under_a_moving_clock_prints_the_same_bytes() {
    if golden().is_none() {
        return;
    }
    let names = fixture_names();
    let first: Vec<_> = names
        .iter()
        .map(|name| compile_fixture(name).documents().cloned().unwrap())
        .collect();
    let now = std::cell::Cell::new(0.0);
    let clock = || {
        now.set(now.get() + 1.0);
        now.get()
    };
    for (name, expected) in names.iter().zip(&first).rev() {
        let options = Options {
            clock: Some(&clock),
            verify: VerifyMode::Each,
            ..options()
        };
        let again = compile_fixture_with(name, fixture_media(), &options);
        assert_eq!(again.documents().unwrap(), expected, "{name}");
    }
    assert!(now.get() > 0.0, "the compiler read the clock");
}

#[test]
fn prints_media_whose_bytes_are_missing_as_omitted_so_no_document_references_them() {
    if golden().is_none() {
        return;
    }
    let compiled = compile_fixture_with("media-timeline", MediaFiles::new(), &options());
    assert!(
        compiled
            .diagnostics
            .iter()
            .any(|d| d.code.code() == "IR0013")
    );
    let documents = compiled.documents().unwrap();
    assert!(documents.media.is_empty());
    assert_eq!(documents.lint(), vec![]);
    assert!(documents.text["slides/ir-m1.html"].contains("Image not included"));
}

#[test]
fn writes_the_media_the_report_lists_copied_or_transcoded_to_png() {
    if golden().is_none() {
        return;
    }
    let compiled = compile_fixture("media-timeline");
    let documents = compiled.documents().unwrap();
    let report: Value =
        serde_json::from_slice(&opened().output("slidock-html/media-timeline.report.json"))
            .unwrap();
    let listed: Vec<&String> = report["media"].as_object().unwrap().keys().collect();
    assert_eq!(documents.media.keys().collect::<Vec<_>>(), listed);
    let input = BytesReader::new(Vec::new());
    let mut transcoded = 0;
    for (path, plan) in &documents.media {
        let mut out = Vec::new();
        slidock::write_media_file(
            plan,
            compiled.media().unwrap(),
            &input,
            50_000_000,
            &mut out,
        )
        .unwrap();
        match plan.action {
            MediaAction::Copy => {
                assert_eq!(
                    slidock_canon::sha256_hex(&out),
                    plan.sha256.to_hex(),
                    "{path}"
                );
            }
            MediaAction::Transcode => {
                transcoded += 1;
                let recorded = opened().output(&format!("fixtures/media-timeline/{path}"));
                assert!(
                    out == recorded,
                    "{path}: not the PNG `slidock compile` wrote"
                );
            }
        }
    }
    assert_eq!(transcoded, 1, "the TIFF");
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<(String, String)> {
    diagnostics
        .iter()
        .map(|d| {
            (
                d.code.code().to_owned(),
                d.container
                    .as_ref()
                    .map_or("presentation".to_owned(), |c| c.as_str().to_owned()),
            )
        })
        .collect()
}

fn print_with(
    compiled: &Compiled,
    features: &IndexMap<String, Vec<FeatureId>>,
    emit: Option<&mut slidock::Emit<'_>>,
) -> Printed {
    let result = &compiled.result;
    let options = EmitOptions {
        provenance: slidock::html_provenance(result.provenance.as_ref().unwrap()),
        generator: generator(),
    };
    let input = PrintInput {
        presentation: result.presentation.as_ref().unwrap(),
        source: result.source.as_ref().unwrap(),
        emit: &options,
        features,
        diagnostics: &result.diagnostics,
        media: compiled.media().unwrap(),
    };
    match emit {
        Some(emit) => print_presentation_with(&input, emit).unwrap(),
        None => print_presentation(&input).unwrap(),
    }
}

fn failed(slide: &resolved::Slide) -> Option<String> {
    slide.body.failed().map(|f| f.code.as_str().to_owned())
}

#[test]
fn fails_a_container_with_an_unaccounted_loss_and_the_slides_that_show_it() {
    if golden().is_none() {
        return;
    }
    let compiled = compile_fixture("inheritance");
    let mut features = compiled.result.features.clone();
    features
        .entry("ir-l.content".to_owned())
        .or_default()
        .push(FeatureId::ElementOle);
    let printed = print_with(&compiled, &features, None);
    let driver: Vec<Diagnostic> = printed
        .diagnostics()
        .filter(|d| d.stage == Stage::Driver)
        .cloned()
        .collect();
    assert_eq!(
        codes(&driver),
        [
            ("IR0901".to_owned(), "ir-l.content".to_owned()),
            ("IR0901".to_owned(), "ir-s1".to_owned()),
            ("IR0901".to_owned(), "ir-s3".to_owned())
        ]
    );
    let layout = &printed.presentation.layouts[&NodeId::new("ir-l.content").unwrap()];
    assert_eq!(layout.body.failed().unwrap().code.as_str(), "IR0901");
    let slides: Vec<Option<String>> = printed.presentation.slides.iter().map(failed).collect();
    assert_eq!(
        slides,
        [Some("IR0901".to_owned()), None, Some("IR0901".to_owned())]
    );
    let documents = printed.documents.unwrap();
    assert!(!documents.text["slides/ir-s2.html"].contains("data-failed"));
    assert!(documents.text["slides/ir-s1.html"].contains("data-failed=\"IR0901\""));
    assert_eq!(documents.lint(), vec![]);
}

#[test]
fn fails_only_the_slide_whose_file_breaks_the_lint() {
    if golden().is_none() {
        return;
    }
    let compiled = compile_fixture("geometry");
    let mut calls = 0;
    let mut emit = |presentation: &resolved::Presentation,
                    options: &EmitOptions|
     -> Result<Emitted, HtmlError> {
        let mut out = emit_documents(presentation, options)?;
        if calls == 0 {
            let text = out.files.text.get_mut("slides/ir-g2.html").unwrap();
            *text = text.replace("</main>", "<script>alert(1)</script>\n</main>");
        }
        calls += 1;
        Ok(out)
    };
    let printed = print_with(&compiled, &compiled.result.features, Some(&mut emit));
    assert_eq!(
        codes(&printed.driver),
        [("HTML6900".to_owned(), "ir-g2".to_owned())]
    );
    let data = printed.driver[0].data.as_ref().unwrap();
    assert_eq!(data["file"].to_js_string(), "slides/ir-g2.html");
    assert_eq!(data["rule"].to_js_string(), "script");
    let slides: Vec<Option<String>> = printed.presentation.slides.iter().map(failed).collect();
    assert_eq!(slides, [None, Some("HTML6900".to_owned()), None, None]);
    assert_eq!(printed.documents.unwrap().lint(), vec![]);
}

#[test]
fn fails_the_presentation_when_a_problem_belongs_to_no_container() {
    if golden().is_none() {
        return;
    }
    let compiled = compile_fixture("min");
    let mut manifest_broken = |presentation: &resolved::Presentation,
                               options: &EmitOptions|
     -> Result<Emitted, HtmlError> {
        let mut out = emit_documents(presentation, options)?;
        out.files
            .text
            .insert("presentation.json".to_owned(), "{}".to_owned());
        Ok(out)
    };
    let printed = print_with(
        &compiled,
        &compiled.result.features,
        Some(&mut manifest_broken),
    );
    assert_eq!(printed.documents, None);
    assert_eq!(
        codes(&printed.driver),
        [
            ("HTML6900".to_owned(), "presentation".to_owned()),
            ("IR0907".to_owned(), "presentation".to_owned())
        ]
    );
}

#[test]
fn accounts_every_loss_on_every_fixture_and_prints_nothing_the_store_cannot_open() {
    if golden().is_none() {
        return;
    }
    for name in fixture_names() {
        let compiled = compile_fixture(&name);
        let printed = print_with(&compiled, &compiled.result.features, None);
        assert!(printed.driver.is_empty(), "{name}: {:?}", printed.driver);
        let result = &compiled.result;
        let emit = EmitOptions::new(slidock::html_provenance(
            result.provenance.as_ref().unwrap(),
        ));
        let input = PrintInput {
            presentation: result.presentation.as_ref().unwrap(),
            source: result.source.as_ref().unwrap(),
            emit: &emit,
            features: &result.features,
            diagnostics: &result.diagnostics,
            media: &EmptyMediaStore,
        };
        let bare = print_presentation(&input).unwrap();
        assert!(bare.documents.unwrap().media.is_empty(), "{name}");
    }
}

/// Every corpus source of the golden files (the repository's probes aside): (presentation id, frontend, file).
fn corpus_sources() -> Vec<(String, String, PathBuf)> {
    let golden = opened();
    golden
        .pairs()
        .iter()
        .filter(|pair| !pair.input.starts_with("repo:"))
        .map(|pair| {
            (
                pair.presentation.clone(),
                pair.frontend.clone(),
                golden.source(pair),
            )
        })
        .collect()
}

#[test]
fn accounts_every_loss_and_lints_clean_on_every_corpus_source_with_the_built_in_fonts() {
    if golden().is_none() {
        return;
    }
    let compiler = Compiler::new(Frontends::default()).unwrap();
    let sources = corpus_sources();
    assert_eq!(sources.len(), 10, "the corpus holds ten non-PDF sources");
    for (id, frontend, path) in sources {
        let reader = slidock_io::FileReader::open(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap().to_owned();
        let compiled = compiler
            .compile(&Input::new(&name, &reader), &options())
            .unwrap_or_else(|e| panic!("{id}.{frontend}: {e}"));
        let provenance = compiled.result.provenance.as_ref().unwrap();
        assert_eq!(provenance.format.as_str(), frontend, "{id}: sniffed");
        let printed = compiled.printed.as_ref().unwrap();
        assert!(
            printed.driver.is_empty(),
            "{id}.{frontend}: {:?}",
            printed.driver
        );
        let documents = printed.documents.as_ref().unwrap();
        let slides = compiled.presentation().unwrap().slides.len();
        assert_eq!(
            documents
                .text
                .keys()
                .filter(|p| p.starts_with("slides/"))
                .count(),
            slides,
            "{id}.{frontend}"
        );
    }
}

/// The families a document's `font-family` declarations name first, as match keys.
fn printed_families(html: &str) -> BTreeSet<String> {
    html.split("font-family:&#x22;")
        .skip(1)
        .filter_map(|rest| rest.split("&#x22;").next())
        .map(font_key)
        .collect()
}

/// A corpus source through a fresh session, as a host runs it: open, finish, print, deck JSON.
fn corpus_deck(path: &Path) -> slidock::FinishedDeck {
    let registry = Frontends::default().registry().unwrap();
    let pipeline = slidock::pipeline().unwrap();
    let reader = slidock_io::FileReader::open(path).unwrap();
    let name = path.file_name().unwrap().to_str().unwrap();
    let mut session = slidock::Session::open(
        &Input::new(name, &reader),
        slidock::SessionOptions::new(&registry, &pipeline),
    )
    .unwrap_or_else(|e| panic!("{name}: {e:?}"));
    let finished = session.finish().unwrap();
    slidock::finish_deck(&session, &finished, generator()).unwrap_or_else(|e| panic!("{name}: {e}"))
}

#[test]
fn builds_a_byte_stable_deck_json_for_every_corpus_source_that_agrees_with_its_manifest() {
    if golden().is_none() {
        return;
    }
    let sources = corpus_sources();
    assert_eq!(sources.len(), 10, "the corpus holds ten non-PDF sources");
    println!(
        "{:<12} {:<8} {:>6} {:>6} {:>6} {:>6} {:>11} {:>9}",
        "deck", "format", "slides", "steps", "media", "faces", "slide faces", "json"
    );
    for (id, frontend, path) in sources {
        let first = corpus_deck(&path);
        let second = corpus_deck(&path);
        let json = first.deck.to_json().unwrap();
        assert_eq!(
            json,
            second.deck.to_json().unwrap(),
            "{id}.{frontend}: two runs"
        );
        assert_eq!(
            slidock::deck::Deck::from_json(&json).unwrap(),
            first.deck,
            "{id}.{frontend}"
        );
        let deck = &first.deck;
        assert_eq!(deck.source.format, frontend, "{id}");
        let manifest =
            slidock_format::read_manifest(&first.documents.text["presentation.json"]).unwrap();
        assert_eq!(deck.title, manifest.title, "{id}.{frontend}");
        assert_eq!(
            (deck.size.w, deck.size.h),
            (manifest.size.w, manifest.size.h)
        );
        assert_eq!(deck.slides.len(), manifest.slides.len(), "{id}.{frontend}");
        let mut shown = 0;
        let mut slide_faces = 0;
        for (slide, entry) in deck.slides.iter().zip(&manifest.slides) {
            let at = format!("{id}.{frontend} {}", slide.id);
            assert_eq!(
                (
                    &slide.id,
                    &slide.title,
                    slide.hidden,
                    &slide.layout,
                    &slide.failed
                ),
                (
                    &entry.id,
                    &entry.title,
                    entry.hidden,
                    &entry.layout,
                    &entry.failed
                ),
                "{at}"
            );
            assert!(first.documents.text.contains_key(&entry.file), "{at}");
            let notes = slide.notes.join("\n");
            assert_eq!((!notes.is_empty()).then_some(notes), entry.notes, "{at}");
            match (&slide.transition, &entry.transition) {
                (Some(ours), Some(theirs)) => assert_eq!(
                    (&ours.native, ours.duration_ms),
                    (&theirs.effect, theirs.duration_ms),
                    "{at}"
                ),
                (None, None) => {}
                _ => panic!("{at}: transition"),
            }
            if !slide.hidden {
                shown += 1;
            }
            assert_eq!(slide.number, (!slide.hidden).then_some(shown), "{at}");
            // The faces the slide loads: the families its document and shown layout print first in a
            // `font-family`, wherever the deck lists the family.
            assert!(
                slide.fonts.windows(2).all(|pair| pair[0] < pair[1])
                    && slide.fonts.iter().all(|&i| (i as usize) < deck.fonts.len()),
                "{at}: {:?}",
                slide.fonts
            );
            let listed: BTreeSet<String> = deck.fonts.iter().map(|f| font_key(&f.family)).collect();
            let mut printed = printed_families(&first.documents.text[&entry.file]);
            let layout = entry.layout.as_ref().filter(|_| {
                !first.documents.text[&entry.file].contains(r#"data-show-layout="false""#)
            });
            if let Some(layout) = layout {
                let file = &manifest
                    .layouts
                    .iter()
                    .find(|l| &l.id == layout)
                    .unwrap()
                    .file;
                printed.extend(printed_families(&first.documents.text[file]));
            }
            printed.retain(|family| listed.contains(family));
            let loaded: BTreeSet<String> = slide
                .fonts
                .iter()
                .map(|&i| font_key(&deck.fonts[i as usize].family))
                .collect();
            assert_eq!(loaded, printed, "{at}");
            slide_faces += slide.fonts.len();
        }
        let media: Vec<&String> = deck.media.iter().map(|m| &m.path).collect();
        assert_eq!(
            media,
            first.documents.media.keys().collect::<Vec<_>>(),
            "{id}.{frontend}"
        );
        assert_eq!(
            deck.diagnostics.total as usize,
            first.diagnostics.len(),
            "{id}.{frontend}"
        );
        let steps: usize = deck.slides.iter().map(|s| s.timeline.len()).sum();
        println!(
            "{id:<12} {frontend:<8} {:>6} {steps:>6} {:>6} {:>6} {slide_faces:>11} {:>9}",
            deck.slides.len(),
            deck.media.len(),
            deck.fonts.len(),
            json.len()
        );
    }
}

/// Writes a bundle to a file.
struct ToFile(std::io::BufWriter<std::fs::File>);

impl slidock::BundleSink for ToFile {
    fn write(&mut self, _: &str, bytes: &[u8]) -> Result<(), slidock::BundleError> {
        assert!(bytes.len() <= slidock::CHUNK_BYTES);
        std::io::Write::write_all(&mut self.0, bytes)
            .map_err(|error| slidock::BundleError::Sink(error.to_string()))
    }
}

/// Checks a bundle against the bytes of a file, chunk by chunk.
struct SameAs {
    file: std::io::BufReader<std::fs::File>,
    at: u64,
    first_difference: Option<u64>,
}

impl slidock::BundleSink for SameAs {
    fn write(&mut self, _: &str, bytes: &[u8]) -> Result<(), slidock::BundleError> {
        let mut expected = vec![0; bytes.len()];
        let read = std::io::Read::read_exact(&mut self.file, &mut expected);
        if (read.is_err() || expected != bytes) && self.first_difference.is_none() {
            self.first_difference = Some(self.at);
        }
        self.at += bytes.len() as u64;
        Ok(())
    }
}

#[test]
fn bundles_every_corpus_source_twice_into_the_same_zip_whose_page_and_templates_check_out() {
    if golden().is_none() {
        return;
    }
    use slidock::{
        BundleOptions, CompiledDeck, SessionMedia, StoreMedia, bundle, lint_templates,
        slide_templates,
    };
    use slidock_format::dom::{Document, NodeId};
    use slidock_io::zip::{ZipArchive, ZipLimits};

    let sources = corpus_sources();
    assert_eq!(sources.len(), 10, "the corpus holds ten non-PDF sources");
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("corpus-bundles");
    std::fs::create_dir_all(&out).unwrap();
    println!(
        "{:<12} {:<8} {:>6} {:>6} {:>12} {:>12}",
        "deck", "format", "slides", "media", "index.html", "zip"
    );
    for (id, frontend, path) in sources {
        let at = format!("{id}.{frontend}");
        let registry = Frontends::default().registry().unwrap();
        let pipeline = slidock::pipeline().unwrap();
        let reader = slidock_io::FileReader::open(&path).unwrap();
        let name = path.file_name().unwrap().to_str().unwrap();
        let mut session = slidock::Session::open(
            &Input::new(name, &reader),
            slidock::SessionOptions::new(&registry, &pipeline),
        )
        .unwrap_or_else(|e| panic!("{at}: {e:?}"));
        let finished = session.finish().unwrap();
        let finished = slidock::finish_deck(&session, &finished, generator()).unwrap();
        let store = SessionMedia(&session);
        let max_pixels = slidock::MediaPolicy::default().transcode_pixels;
        let media = StoreMedia::new(&store, &reader, &finished.documents, max_pixels);
        let compiled = CompiledDeck {
            deck: finished.deck,
            documents: finished.documents,
            media: &media,
            fonts: Vec::new(),
        };

        let zip_path = out.join(format!("{at}.zip"));
        let mut to_file = ToFile(std::io::BufWriter::new(
            std::fs::File::create(&zip_path).unwrap(),
        ));
        let report = bundle(&compiled, &BundleOptions::default(), &mut to_file)
            .unwrap_or_else(|e| panic!("{at}: {e}"));
        to_file.0.into_inner().unwrap().sync_all().unwrap();
        let mut same = SameAs {
            file: std::io::BufReader::new(std::fs::File::open(&zip_path).unwrap()),
            at: 0,
            first_difference: None,
        };
        let again = bundle(&compiled, &BundleOptions::default(), &mut same).unwrap();
        assert_eq!(same.first_difference, None, "{at}: a second bundle differs");
        assert_eq!(again, report, "{at}");
        assert_eq!(
            std::fs::metadata(&zip_path).unwrap().len(),
            report.bytes,
            "{at}"
        );

        let zip = slidock_io::FileReader::open(&zip_path).unwrap();
        let archive = ZipArchive::open(zip, ZipLimits::default()).unwrap();
        let names: Vec<&str> = archive.entries().iter().map(|e| e.name.as_str()).collect();
        let reported: Vec<&str> = report.files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, reported, "{at}");
        let mut expected = vec!["index.html"];
        expected.extend(compiled.documents.media.keys().map(String::as_str));
        expected.push("LICENSES.txt");
        assert_eq!(names, expected, "{at}");

        let index = archive.read(&archive.entries()[0]).unwrap();
        let index = std::str::from_utf8(&index).unwrap();
        let page = Document::parse(index);
        let scripts: Vec<NodeId> = page
            .walk(NodeId::DOCUMENT)
            .into_iter()
            .filter(|e| page.tag_name(*e) == "script")
            .collect();
        assert_eq!(scripts.len(), 4, "{at}: only the page's own scripts");
        let deck_json = page.text_content(scripts[0]);
        let deck = slidock::deck::Deck::from_json(&deck_json).unwrap();
        assert_eq!(
            deck.player,
            Some(slidock::embedded_player(true).unwrap()),
            "{at}"
        );
        let templates: Vec<&str> = page
            .walk(NodeId::DOCUMENT)
            .into_iter()
            .filter_map(|e| page.attribute(e, "data-sd-slide"))
            .collect();
        let slides: Vec<&str> = deck.slides.iter().map(|s| s.id.as_str()).collect();
        assert_eq!(templates, slides, "{at}");

        let composed = slide_templates(&compiled).unwrap();
        for template in &composed {
            assert!(
                index.contains(&format!(
                    "<template data-sd-slide=\"{}\">{}</template>\n",
                    template.id, template.html
                )),
                "{at}: {}",
                template.id
            );
        }
        let problems: Vec<_> = lint_templates(&compiled.documents, &composed)
            .unwrap()
            .into_iter()
            .filter(slidock_format::Problem::is_error)
            .collect();
        assert!(problems.is_empty(), "{at}: {problems:?}");

        let unzip = std::process::Command::new("unzip")
            .arg("-tq")
            .arg(&zip_path)
            .output()
            .expect("unzip is installed");
        assert!(
            unzip.status.success(),
            "{at}: {}",
            String::from_utf8_lossy(&unzip.stdout)
        );
        println!(
            "{id:<12} {frontend:<8} {:>6} {:>6} {:>12} {:>12}",
            deck.slides.len(),
            compiled.documents.media.len(),
            index.len(),
            report.bytes
        );
        std::fs::remove_file(&zip_path).unwrap();
    }
}
