//! The driver on presentations of our own: a compile prints a lint-clean file set with every loss accounted for; a
//! loss or a lint problem fails only the container it belongs to (and those showing it), and a problem outside
//! every container fails the presentation. Media without bytes print as omitted, and the session path prints what
//! the compile path prints.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use indexmap::IndexMap;
use serde_json::{Value, json};
use slidock::{
    Compiled, Emitted, LossCheckInput, MediaFiles, PrintInput, Printed, Session, SessionOptions,
    loss_check, print_finished, print_presentation, print_presentation_with, slidock_generator,
    supported_features,
};
use slidock_core_api::{EmptyMediaStore, Input, MediaStore};
use slidock_html::{EmitOptions, HtmlError, emit_documents};
use slidock_io::BytesReader;
use slidock_ir::diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticSite, Severity, Stage, codes as diagnostic_codes,
    make_diagnostic,
};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::NodeId;
use slidock_ir::resolved;
use support::*;

fn id(text: &str) -> NodeId {
    NodeId::new(text).unwrap()
}

/// What printing a compile reads, with `media` as its store.
struct Parts<'a> {
    compiled: &'a Compiled,
    emit: EmitOptions,
}

impl<'a> Parts<'a> {
    fn of(compiled: &'a Compiled) -> Self {
        let provenance = compiled.result.provenance.as_ref().unwrap();
        Parts {
            compiled,
            emit: EmitOptions {
                provenance: slidock::html_provenance(provenance),
                generator: slidock_generator(),
            },
        }
    }

    fn input<'b>(
        &'b self,
        features: &'b IndexMap<String, Vec<FeatureId>>,
        media: &'b dyn MediaStore,
    ) -> PrintInput<'b> {
        let result = &self.compiled.result;
        PrintInput {
            presentation: result.presentation.as_ref().unwrap(),
            source: result.source.as_ref().unwrap(),
            emit: &self.emit,
            features,
            diagnostics: &result.diagnostics,
            media,
        }
    }

    fn print(&self, features: &IndexMap<String, Vec<FeatureId>>) -> Printed {
        let media = self.compiled.media().unwrap();
        print_presentation(&self.input(features, media)).unwrap()
    }
}

fn failed_code(slide: &resolved::Slide) -> Option<String> {
    slide.body.failed().map(|f| f.code.as_str().to_owned())
}

fn text(compiled: &Compiled, path: &str) -> String {
    compiled.documents().unwrap().text[path].clone()
}

#[test]
fn compiles_to_a_lint_clean_file_set_with_every_loss_accounted_for() {
    for presentation in [hello(), chain(), with_bmp().0] {
        let compiled = compile(&presentation);
        let driver: Vec<&Diagnostic> = compiled
            .diagnostics
            .iter()
            .filter(|d| d.stage == Stage::Driver)
            .collect();
        assert!(driver.is_empty(), "{driver:?}");
        let errors: Vec<&Diagnostic> = compiled
            .diagnostics
            .iter()
            .filter(|d| d.severity == Severity::Error)
            .collect();
        assert!(errors.is_empty(), "{errors:?}");
        let documents = compiled.documents().unwrap();
        assert_eq!(documents.lint(), vec![]);
        assert!(documents.text.contains_key("presentation.json"));
    }
    let compiled = compile(&chain());
    let paths: Vec<&String> = compiled.documents().unwrap().text.keys().collect();
    assert_eq!(
        paths,
        [
            "layouts/ir-l.html",
            "presentation.json",
            "slides/ir-s1.html",
            "slides/ir-s2.html",
            "slides/ir-s3.html"
        ]
    );
}

#[test]
fn merges_the_compile_and_the_printer_diagnostics_in_output_order() {
    let (mut presentation, bmp) = with_bmp();
    let clip = b"not really a video";
    let sha = add_media(&mut presentation, clip, "video/mp4", 16, 9);
    presentation["slides"][0]["elements"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "ir-s1.video", "kind": "media", "mediaKind": "video", "media": sha, "origin": agent(),
            "frame": frame(200.0, 10.0, 160.0, 90.0), "loop": false, "muted": true, "autoplay": false
        }));
    let compiled = compile_with(&presentation, media_files(&[(&bmp, "bmp"), (clip, "mp4")]));
    // The backend's HTML6005 (a video with no poster) sorts among the passes' diagnostics of its slide.
    assert_eq!(
        codes(&compiled.diagnostics),
        [
            ("HTML6005".to_owned(), "ir-s1".to_owned()),
            ("MID5602".to_owned(), "ir-s1".to_owned()),
            ("MID5605".to_owned(), "ir-s1".to_owned()),
            ("MID5801".to_owned(), "ir-s1".to_owned())
        ]
    );
    let printed = compiled.printed.as_ref().unwrap();
    assert_eq!(
        codes(&printed.backend),
        [("HTML6005".to_owned(), "ir-s1".to_owned())]
    );
    assert!(printed.driver.is_empty());
}

#[test]
fn prints_media_whose_bytes_are_missing_as_omitted_so_no_document_references_them() {
    let (presentation, bmp) = with_bmp();
    let compiled = compile(&presentation);
    let documents = compiled.documents().unwrap();
    assert!(documents.media.is_empty());
    assert_eq!(documents.lint(), vec![]);
    assert!(text(&compiled, "slides/ir-s1.html").contains("Image not included"));
    let printed = compiled.presentation().unwrap();
    let plan = &printed.media.values().next().unwrap().plan;
    assert_eq!(
        serde_json::to_value(plan).unwrap(),
        json!({ "action": "omit", "reason": "unavailable" })
    );

    // With its bytes, the BMP is planned as a transcode to PNG and referenced.
    let compiled = compile_with(&presentation, media_files(&[(&bmp, "bmp")]));
    let documents = compiled.documents().unwrap();
    let (path, plan) = documents.media.iter().next().unwrap();
    assert_eq!(path, &format!("media/{}.png", sha256(&bmp)));
    assert_eq!(plan.action, slidock::MediaAction::Transcode);
    assert!(!text(&compiled, "slides/ir-s1.html").contains("Image not included"));
}

#[test]
fn forwards_trace_keeping_the_presentation_after_every_pass() {
    let compiler = slidock::Compiler::new(slidock::Frontends::default()).unwrap();
    let reader = BytesReader::new(bytes(&hello()));
    let options = slidock::Options {
        trace: true,
        ..slidock::Options::default()
    };
    let compiled = compiler
        .compile(&Input::new(NAME, &reader), &options)
        .unwrap();
    let passes: Vec<String> = compiled
        .result
        .trace
        .unwrap()
        .iter()
        .map(|snapshot| snapshot.pass.clone())
        .collect();
    let expected: Vec<String> = slidock::pass_manager()
        .unwrap()
        .passes()
        .iter()
        .map(|pass| pass.name().to_owned())
        .collect();
    assert_eq!(passes, expected);
    assert_eq!(expected.len(), 12);
}

fn named(code: DiagnosticCode, feature: FeatureId, stage: Stage, container: &str) -> Diagnostic {
    make_diagnostic(
        code,
        stage,
        "test",
        DiagnosticSite::new()
            .container(id(container))
            .feature(feature),
    )
}

/// The loss check over a compile, with `detected`, `diagnostics` and `supported` swapped in.
fn losses(
    compiled: &Compiled,
    detected: &IndexMap<String, Vec<FeatureId>>,
    presentation: Option<&slidock_ir::source::Presentation>,
    extra: &[Diagnostic],
    supported: &[FeatureId],
) -> Vec<(String, Option<String>, String)> {
    let result = &compiled.result;
    let resolved = result.presentation.as_ref().unwrap().to_source().unwrap();
    let mut diagnostics = result.diagnostics.clone();
    diagnostics.extend(compiled.printed.as_ref().unwrap().backend.clone());
    diagnostics.extend(extra.iter().cloned());
    loss_check(&LossCheckInput {
        detected,
        source: result.source.as_ref().unwrap(),
        presentation: presentation.unwrap_or(&resolved),
        diagnostics: &diagnostics,
        supported,
    })
    .into_iter()
    .map(|d| {
        let data = d.data.unwrap();
        assert_eq!((d.code, d.stage), (diagnostic_codes::IR0901, Stage::Driver));
        (
            data["feature"].to_js_string(),
            d.container.map(|c| c.as_str().to_owned()),
            data["check"].to_js_string(),
        )
    })
    .collect()
}

#[test]
fn accounts_every_loss_of_a_clean_compile() {
    for presentation in [hello(), chain(), with_bmp().0] {
        let compiled = compile(&presentation);
        assert_eq!(
            losses(
                &compiled,
                &compiled.result.features,
                None,
                &[],
                &supported_features()
            ),
            vec![]
        );
    }
}

#[test]
fn flags_a_feature_the_frontend_detected_and_did_not_lower_unless_a_frontend_diagnostic_at_its_floor_names_it()
 {
    let compiled = compile(&hello());
    let mut detected = compiled.result.features.clone();
    detected
        .get_mut("ir-s1")
        .unwrap()
        .push(FeatureId::ElementChart);
    let supported = supported_features();
    assert_eq!(
        losses(&compiled, &detected, None, &[], &supported),
        [(
            "element.chart".to_owned(),
            Some("ir-s1".to_owned()),
            "frontend".to_owned()
        )]
    );
    // MID5603 is lossy, below the decline floor of a chart; MID5601 is a decline.
    let lossy = named(
        diagnostic_codes::MID5603,
        FeatureId::ElementChart,
        Stage::Frontend,
        "ir-s1",
    );
    assert_eq!(
        losses(&compiled, &detected, None, &[lossy], &supported).len(),
        1
    );
    let decline = named(
        diagnostic_codes::MID5601,
        FeatureId::ElementChart,
        Stage::Frontend,
        "ir-s1",
    );
    assert_eq!(
        losses(&compiled, &detected, None, &[decline], &supported),
        vec![]
    );
    // The stage must match: only the frontend accounts for what the frontend dropped.
    let from_pass = named(
        diagnostic_codes::MID5601,
        FeatureId::ElementChart,
        Stage::Pass,
        "ir-s1",
    );
    assert_eq!(
        losses(&compiled, &detected, None, &[from_pass], &supported).len(),
        1
    );
    // So must the container.
    let elsewhere = named(
        diagnostic_codes::MID5601,
        FeatureId::ElementChart,
        Stage::Frontend,
        "ir-s9",
    );
    assert_eq!(
        losses(&compiled, &detected, None, &[elsewhere], &supported).len(),
        1
    );
}

#[test]
fn flags_a_feature_a_pass_dropped() {
    let compiled = compile(&hello());
    let mut dropped = compiled
        .result
        .presentation
        .as_ref()
        .unwrap()
        .to_source()
        .unwrap();
    dropped.slides[0].elements.clear();
    let supported = supported_features();
    let detected = &compiled.result.features;
    let found = losses(&compiled, detected, Some(&dropped), &[], &supported);
    assert!(found.contains(&(
        "element.shape".to_owned(),
        Some("ir-s1".to_owned()),
        "pass".to_owned()
    )));
    let accounted: Vec<Diagnostic> = found
        .iter()
        .map(|(feature, _, _)| {
            let feature: FeatureId = serde_json::from_value(json!(feature)).unwrap();
            named(diagnostic_codes::MID5601, feature, Stage::Pass, "ir-s1")
        })
        .collect();
    assert_eq!(
        losses(&compiled, detected, Some(&dropped), &accounted, &supported),
        vec![]
    );
}

#[test]
fn flags_a_feature_the_backend_does_not_print() {
    let compiled = compile(&hello());
    let supported: Vec<FeatureId> = supported_features()
        .into_iter()
        .filter(|f| *f != FeatureId::TextRun)
        .collect();
    let detected = &compiled.result.features;
    assert_eq!(
        losses(&compiled, detected, None, &[], &supported),
        [(
            "text.run".to_owned(),
            Some("ir-s1".to_owned()),
            "backend".to_owned()
        )]
    );
    let declined = named(
        diagnostic_codes::HTML6002,
        FeatureId::TextRun,
        Stage::Backend,
        "ir-s1",
    );
    assert_eq!(
        losses(&compiled, detected, None, &[declined], &supported),
        vec![]
    );
}

#[test]
fn counts_no_foreign_node_as_printed_and_takes_only_the_cores_foreign_codes_for_its_content() {
    let mut presentation = hello();
    presentation["slides"][0]["elements"]
        .as_array_mut()
        .unwrap()
        .push(json!({
            "id": "ir-s1.unread", "kind": "foreign", "origin": agent(), "dialect": "pptx", "type": "table",
            "frame": frame(0.0, 0.0, 10.0, 10.0)
        }));
    let compiled = compile(&presentation);
    let detected = &compiled.result.features;
    let supported = supported_features();
    // The compile's own IR0910 accounts for the table.
    assert!(
        compiled
            .result
            .diagnostics
            .iter()
            .any(|d| d.code == diagnostic_codes::IR0910)
    );
    assert_eq!(losses(&compiled, detected, None, &[], &supported), vec![]);

    // Without it, the backend check finds the table unprinted, although tables print elsewhere.
    let mut without = compiled.result.diagnostics.clone();
    without.retain(|d| d.code != diagnostic_codes::IR0910);
    let resolved = compiled
        .result
        .presentation
        .as_ref()
        .unwrap()
        .to_source()
        .unwrap();
    let found = loss_check(&LossCheckInput {
        detected,
        source: compiled.result.source.as_ref().unwrap(),
        presentation: &resolved,
        diagnostics: &without,
        supported: &supported,
    });
    assert_eq!(codes(&found), [("IR0901".to_owned(), "ir-s1".to_owned())]);
    assert_eq!(
        found[0].data.as_ref().unwrap()["check"].to_js_string(),
        "backend"
    );

    // The core's Foreign codes account at the backend check only: a chart the frontend dropped stays a loss.
    let mut detected = detected.clone();
    detected
        .get_mut("ir-s1")
        .unwrap()
        .push(FeatureId::ElementChart);
    let foreign = make_diagnostic(
        diagnostic_codes::IR0910,
        Stage::Frontend,
        "foreign",
        DiagnosticSite::new()
            .container(id("ir-s1"))
            .feature(FeatureId::ElementChart),
    );
    assert_eq!(
        losses(&compiled, &detected, None, &[foreign], &supported),
        [(
            "element.chart".to_owned(),
            Some("ir-s1".to_owned()),
            "frontend".to_owned()
        )]
    );
}

#[test]
fn flags_a_presentation_level_feature_lost_and_fails_the_presentation() {
    let compiled = compile(&hello());
    let mut detected = compiled.result.features.clone();
    detected
        .get_mut("presentation")
        .unwrap()
        .push(FeatureId::FontEmbedded);
    let printed = Parts::of(&compiled).print(&detected);
    assert_eq!(printed.documents, None);
    assert_eq!(
        codes(&printed.driver),
        [
            ("IR0901".to_owned(), "presentation".to_owned()),
            ("IR0907".to_owned(), "presentation".to_owned())
        ]
    );
    assert!(printed.backend.is_empty());
}

#[test]
fn fails_a_container_with_an_unaccounted_loss_and_the_slides_that_show_it() {
    let compiled = compile(&chain());
    let mut detected = compiled.result.features.clone();
    detected
        .get_mut("ir-l")
        .unwrap()
        .push(FeatureId::ElementOle);
    let printed = Parts::of(&compiled).print(&detected);
    assert_eq!(
        codes(&printed.driver),
        [
            ("IR0901".to_owned(), "ir-l".to_owned()),
            ("IR0901".to_owned(), "ir-s1".to_owned()),
            ("IR0901".to_owned(), "ir-s3".to_owned())
        ]
    );
    // Each dependent names the container it fails with.
    assert_eq!(
        printed.driver[1].data.as_ref().unwrap()["via"].to_js_string(),
        "ir-l"
    );
    let layout = &printed.presentation.layouts[&id("ir-l")];
    assert_eq!(layout.body.failed().unwrap().code.as_str(), "IR0901");
    let failed: Vec<Option<String>> = printed
        .presentation
        .slides
        .iter()
        .map(failed_code)
        .collect();
    assert_eq!(
        failed,
        [Some("IR0901".to_owned()), None, Some("IR0901".to_owned())]
    );
    let documents = printed.documents.unwrap();
    assert!(!documents.text["slides/ir-s2.html"].contains("data-failed"));
    assert!(documents.text["slides/ir-s1.html"].contains("data-failed=\"IR0901\""));
    assert_eq!(documents.lint(), vec![]);
}

/// The backend, with `slides/ir-s2.html` broken by `tamper` on the first `times` prints.
fn tampering(
    file: &'static str,
    times: usize,
    tamper: fn(&str) -> String,
) -> impl FnMut(&resolved::Presentation, &EmitOptions) -> Result<Emitted, HtmlError> {
    let mut calls = 0;
    move |presentation, options| {
        let mut out = emit_documents(presentation, options)?;
        if calls < times {
            let text = out.files.text.get_mut(file).unwrap();
            *text = tamper(text);
        }
        calls += 1;
        Ok(out)
    }
}

fn with_script(html: &str) -> String {
    html.replace("</main>", "<script>alert(1)</script>\n</main>")
}

#[test]
fn fails_only_the_slide_whose_file_breaks_the_lint_and_lints_its_failed_document_clean() {
    let compiled = compile(&chain());
    let parts = Parts::of(&compiled);
    let media = compiled.media().unwrap();
    let mut emit = tampering("slides/ir-s2.html", 1, with_script);
    let printed =
        print_presentation_with(&parts.input(&compiled.result.features, media), &mut emit).unwrap();
    assert_eq!(
        codes(&printed.driver),
        [("HTML6900".to_owned(), "ir-s2".to_owned())]
    );
    let data = printed.driver[0].data.as_ref().unwrap();
    assert_eq!(data["file"].to_js_string(), "slides/ir-s2.html");
    assert_eq!(data["rule"].to_js_string(), "script");
    let failed: Vec<Option<String>> = printed
        .presentation
        .slides
        .iter()
        .map(failed_code)
        .collect();
    assert_eq!(failed, [None, Some("HTML6900".to_owned()), None]);
    assert_eq!(printed.documents.unwrap().lint(), vec![]);
}

#[test]
fn fails_the_slides_of_a_layout_whose_file_breaks_the_lint() {
    let compiled = compile(&chain());
    let parts = Parts::of(&compiled);
    let media = compiled.media().unwrap();
    let mut emit = tampering("layouts/ir-l.html", 1, with_script);
    let printed =
        print_presentation_with(&parts.input(&compiled.result.features, media), &mut emit).unwrap();
    assert_eq!(
        codes(&printed.driver),
        [
            ("HTML6900".to_owned(), "ir-l".to_owned()),
            ("HTML6900".to_owned(), "ir-s1".to_owned()),
            ("HTML6900".to_owned(), "ir-s3".to_owned())
        ]
    );
}

#[test]
fn fails_the_presentation_when_a_problem_belongs_to_no_container_or_the_failed_documents_still_break_the_lint()
 {
    let compiled = compile(&hello());
    let parts = Parts::of(&compiled);
    let media = compiled.media().unwrap();
    let input = parts.input(&compiled.result.features, media);

    let mut manifest_broken = tampering("presentation.json", usize::MAX, |_| "{}".to_owned());
    let printed = print_presentation_with(&input, &mut manifest_broken).unwrap();
    assert_eq!(printed.documents, None);
    assert_eq!(
        codes(&printed.driver),
        [
            ("HTML6900".to_owned(), "presentation".to_owned()),
            ("IR0907".to_owned(), "presentation".to_owned())
        ]
    );

    let mut always_broken = tampering("slides/ir-s1.html", usize::MAX, |html| {
        html.replace("</main>", "<script></script></main>")
    });
    let printed = print_presentation_with(&input, &mut always_broken).unwrap();
    assert_eq!(printed.documents, None);
    assert_eq!(
        codes(&printed.driver),
        [
            ("HTML6900".to_owned(), "ir-s1".to_owned()),
            ("IR0907".to_owned(), "presentation".to_owned())
        ]
    );
}

#[test]
fn prints_media_the_store_cannot_open_as_omitted_whatever_the_frontend_verified() {
    let (presentation, bmp) = with_bmp();
    let compiled = compile_with(&presentation, media_files(&[(&bmp, "bmp")]));
    let parts = Parts::of(&compiled);
    let printed =
        print_presentation(&parts.input(&compiled.result.features, &EmptyMediaStore)).unwrap();
    assert!(printed.documents.unwrap().media.is_empty());
}

#[test]
fn prints_a_finished_session_as_the_compile_prints() {
    let (presentation, bmp) = with_bmp();
    let compiled = compile_with(&presentation, media_files(&[(&bmp, "bmp")]));

    let registry = slidock::Frontends::default()
        .with_ir_media(media_files(&[(&bmp, "bmp")]))
        .registry()
        .unwrap();
    let pipeline = slidock::pipeline().unwrap();
    let reader = BytesReader::new(bytes(&presentation));
    let mut options = SessionOptions::new(&registry, &pipeline);
    options.verify = slidock::VerifyMode::Each;
    let mut session = Session::open(&Input::new(NAME, &reader), options).unwrap();
    let finished = session.finish().unwrap();
    let printed = print_finished(&session, &finished, slidock_generator())
        .unwrap()
        .unwrap();
    assert_eq!(printed.printed.documents.as_ref(), compiled.documents());
    assert_eq!(printed.diagnostics, compiled.diagnostics);
    assert_eq!(Some(&printed.printed.presentation), compiled.presentation());
}

#[test]
fn the_pipeline_runs_the_standard_order_split_at_the_fonts() {
    let pipeline = slidock::pipeline().unwrap();
    let names: Vec<&str> = pipeline
        .manager()
        .passes()
        .iter()
        .map(|pass| pass.name())
        .collect();
    let expected: Vec<&str> = slidock_core::BEFORE_FONTS
        .iter()
        .chain(&slidock_core::AFTER_FONTS[..6])
        .copied()
        .collect();
    assert_eq!(names, expected);
    assert_eq!(pipeline.font_boundary(), 6);
}

#[test]
fn reports_provenance_diagnostics_features_and_shows() {
    let compiled = compile(&chain());
    let report: Value = serde_json::from_str(&compiled.report().unwrap()).unwrap();
    assert_eq!(report["provenance"]["format"], "ir");
    assert_eq!(report["provenance"]["name"], NAME);
    assert_eq!(
        report["shows"],
        json!({ "ir-s1": ["ir-l", "ir-m"], "ir-s2": [], "ir-s3": ["ir-l", "ir-m"] })
    );
    assert!(report["features"]["ir-s1"].is_array());
    assert!(report.get("stats").is_none());
    assert_eq!(
        compiled.report().unwrap(),
        slidock_canon::to_canonical_json(&report, slidock_canon::Layout::Indented).unwrap()
    );
}

#[test]
fn a_refused_input_compiles_to_nothing_with_its_refusal() {
    let compiler = slidock::Compiler::new(slidock::Frontends::default()).unwrap();
    let reader = BytesReader::new(b"not a presentation".to_vec());
    let compiled = compiler
        .compile(
            &Input::new("notes.txt", &reader),
            &slidock::Options::default(),
        )
        .unwrap();
    assert!(compiled.printed.is_none());
    assert_eq!(
        compiled.result.refusal.as_ref().map(|d| d.code),
        Some(diagnostic_codes::IR0902)
    );
    assert_eq!(
        codes(&compiled.diagnostics),
        [("IR0902".to_owned(), "presentation".to_owned())]
    );
    let report: Value = serde_json::from_str(&compiled.report().unwrap()).unwrap();
    assert_eq!(report["provenance"], Value::Null);
    assert!(report.get("shows").is_none());
}

/// An SVG plan writes the bytes only when they pass the allow-list again (an IR may claim any bytes are a sized SVG),
/// and writes them as a picture draws them: stretched to whatever box shows them.
#[test]
fn writes_an_svg_as_a_picture_only_when_it_passes_the_allow_list() {
    let drawn = br##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 2 2" width="4" height="2"><rect width="2" height="1" fill="#FE7867"/></svg>"##;
    let stretched = br##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2" viewBox="-1 0 4 2" preserveAspectRatio="none"><rect width="2" height="1" fill="#FE7867"/></svg>"##;
    let scripted = br#"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="2"><script>alert(1)</script></svg>"#;
    for (bytes, written_as) in [(&drawn[..], Some(&stretched[..])), (&scripted[..], None)] {
        let mut p = presentation(vec![]);
        let sha = add_media(&mut p, bytes, "image/svg+xml", 4, 2);
        p["slides"] = json!([slide(
            "ir-s1",
            vec![picture("ir-s1.pic", frame(10.0, 10.0, 100.0, 50.0), &sha)]
        )]);
        let compiled = compile_with(&p, media_files(&[(bytes, "svg")]));
        let documents = compiled.documents().unwrap();
        let (path, plan) = documents.media.iter().next().unwrap();
        assert_eq!(path.as_str(), format!("media/{sha}.svg"));
        assert_eq!(plan.action, slidock::MediaAction::Copy);
        let store = compiled.media().unwrap();
        let input = BytesReader::new(Vec::new());
        let mut out = Vec::new();
        let written = slidock::write_media_file(plan, store, &input, 0, &mut out);
        if let Some(expected) = written_as {
            written.unwrap();
            assert_eq!(
                String::from_utf8(out).unwrap(),
                String::from_utf8(expected.to_vec()).unwrap()
            );
        } else {
            let error = written.unwrap_err();
            assert!(matches!(error, slidock::MediaError::Svg { .. }), "{error}");
            assert!(out.is_empty());
        }
    }
}

#[test]
fn writes_media_files_copied_or_transcoded_to_png() {
    let (presentation, bmp) = with_bmp();
    let compiled = compile_with(&presentation, media_files(&[(&bmp, "bmp")]));
    let documents = compiled.documents().unwrap();
    let plan = documents.media.values().next().unwrap();
    let store = compiled.media().unwrap();
    let input = BytesReader::new(Vec::new());
    let mut png = Vec::new();
    slidock::write_media_file(plan, store, &input, 50_000_000, &mut png).unwrap();
    assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);
    let mut again = Vec::new();
    slidock::write_media_file(plan, store, &input, 50_000_000, &mut again).unwrap();
    assert_eq!(png, again);
    // Over the pixel limit, the transcode is refused.
    let mut refused = Vec::new();
    assert!(slidock::write_media_file(plan, store, &input, 0, &mut refused).is_err());

    let mut copy = plan.clone();
    copy.action = slidock::MediaAction::Copy;
    let mut copied = Vec::new();
    slidock::write_media_file(&copy, store, &input, 0, &mut copied).unwrap();
    assert_eq!(copied, bmp);
    let mut missing = Vec::new();
    let error =
        slidock::write_media_file(plan, &EmptyMediaStore, &input, 0, &mut missing).unwrap_err();
    assert!(
        matches!(error, slidock::MediaError::Unavailable(_)),
        "{error}"
    );
    let _ = MediaFiles::new();
}
