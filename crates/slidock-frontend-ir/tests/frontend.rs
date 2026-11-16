//! The IR frontend on presentations of our own: sniffing, analysis, the refusals, JSON read as `JSON.parse` reads it,
//! the values it drops (IR0012, once per kind per container), containers holding a pass's origin (IR0011) and their
//! failed skeletons, media verified by hash (IR0013), partial lowering, and canonical dumps that read back to the
//! same IR.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use serde_json::{Value, json};
use slidock_core::{CompileOptions, FrontendRegistry, Phase, compile};
use slidock_core_api::{
    Deadline, Frontend, FrontendCtx, FrontendError, Input, Limits, LowerOptions, LowerResult,
    SniffHints, Unverified,
};
use slidock_frontend_ir::{IrFrontend, MediaFiles, SCHEMA_URL, dump_presentation};
use slidock_io::BytesReader;
use slidock_ir::diagnostics::{Collect, Diagnostic, Stage};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::{NodeId, Sha256};

const NAME: &str = "deck.presentation-ir.json";

fn rgb(r: f64) -> Value {
    json!({ "kind": "rgb", "space": "srgb", "r": r, "g": r, "b": r, "a": 1 })
}

fn frame() -> Value {
    json!({ "x": 10, "y": 10, "w": 100, "h": 50, "rotation": 0, "flipH": false, "flipV": false })
}

fn shape(id: &str) -> Value {
    json!({
        "id": id, "origin": { "kind": "agent" }, "kind": "shape", "frame": frame(),
        "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
        "text": { "body": {}, "paragraphs": [{ "level": 0, "props": {}, "inlines": [
            { "kind": "run", "text": "Hi", "props": { "font": { "kind": "family", "family": "Liberation Sans" },
              "size": 18, "color": rgb(0.0) } }
        ] }] }
    })
}

fn picture(id: &str, sha: &str) -> Value {
    json!({ "id": id, "origin": { "kind": "agent" }, "kind": "picture", "frame": frame(), "media": sha })
}

fn slide(id: &str, elements: Vec<Value>) -> Value {
    json!({ "id": id, "origin": { "kind": "agent" }, "hidden": false, "showLayout": true, "timeline": [],
            "elements": elements })
}

fn defaults() -> Value {
    let level = json!({
        "para": { "align": "left", "bullet": { "kind": "none" }, "indent": 0,
                  "lineSpacing": { "basis": "size-1.2", "kind": "multiple", "value": 1 }, "marginLeft": 0,
                  "marginRight": 0, "rtl": false, "spaceAfter": { "kind": "pt", "value": 0 },
                  "spaceBefore": { "kind": "pt", "value": 0 }, "tabSize": 72 },
        "run": { "baseline": 0, "caps": "none", "color": rgb(0.0), "font": { "kind": "theme", "slot": "minor" },
                 "highlight": null, "italic": false, "letterSpacing": 0, "link": null, "scriptScale": 1, "size": 18,
                 "strike": "none", "synthesizeBold": true, "underline": "none", "weight": 400 }
    });
    let text: serde_json::Map<String, Value> =
        (0..9).map(|i| (i.to_string(), level.clone())).collect();
    json!({ "body": { "anchor": "top", "autofit": { "kind": "none" }, "insets": { "b": 3.6, "l": 7.2, "r": 7.2, "t": 3.6 },
                      "overflow": "spread", "wrap": "square" }, "text": text })
}

fn presentation(slides: Vec<Value>) -> Value {
    json!({ "$schema": SCHEMA_URL, "size": { "w": 720, "h": 405 }, "defaults": defaults(), "themes": {},
            "masters": {}, "layouts": {}, "fonts": {}, "media": {}, "phase": "source", "slides": slides })
}

fn media_entry(bytes: &[u8]) -> (String, Value) {
    let sha = Sha256::of(bytes).to_hex();
    let entry = json!({ "sha256": sha, "type": "image/png", "bytes": bytes.len(), "width": 2, "height": 2,
                        "origin": { "kind": "source", "part": "media/x.png" } });
    (sha, entry)
}

fn id(text: &str) -> NodeId {
    NodeId::new(text).unwrap()
}

/// The frontend's lower of `json`, with its diagnostics.
fn lower(
    frontend: &IrFrontend,
    bytes: &[u8],
    only: Option<&[NodeId]>,
) -> (Result<LowerResult, FrontendError>, Vec<Diagnostic>) {
    let reader = BytesReader::new(bytes.to_vec());
    let input = Input::new(NAME, &reader);
    let limits = Limits::default();
    let mut sink = Collect::new(Stage::Frontend, "ir");
    let analysis = {
        let mut ctx = FrontendCtx {
            limits: &limits,
            diag: &mut sink,
            deadline: Deadline::never(),
            spent: slidock_core_api::Output::default(),
        };
        frontend.analyze(&input, &mut ctx)
    };
    let lowered = analysis.and_then(|analysis| {
        let facts = analysis.facts();
        let mut ctx = FrontendCtx {
            limits: &limits,
            diag: &mut sink,
            deadline: Deadline::never(),
            spent: slidock_core_api::Output::default(),
        };
        frontend.lower(
            &input,
            &mut ctx,
            &LowerOptions {
                facts: &facts,
                only,
                native_motion: false,
            },
        )
    });
    (lowered, sink.diagnostics)
}

fn lowered_json(result: &LowerResult) -> &Value {
    match &result.presentation {
        Unverified::Json(value) => value,
        Unverified::Typed(_) => panic!("the IR frontend lowers JSON"),
    }
}

fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).unwrap()
}

/// Code, container and the data's field (or reason) of each diagnostic.
fn summary(diagnostics: &[Diagnostic]) -> Vec<(String, String, String)> {
    diagnostics
        .iter()
        .map(|d| {
            let data = d.data.as_ref();
            let detail = data
                .and_then(|data| {
                    data.get("field")
                        .or_else(|| data.get("path"))
                        .or_else(|| data.get("reason"))
                })
                .map(|value| value.to_js_string())
                .unwrap_or_default();
            (
                d.code.code().to_owned(),
                d.container
                    .as_ref()
                    .map_or(String::new(), |c| c.as_str().to_owned()),
                detail,
            )
        })
        .collect()
}

fn refusal_of(bytes: &[u8], limits: Limits) -> Option<Diagnostic> {
    let registry = FrontendRegistry::new(vec![Box::new(IrFrontend::default())]).unwrap();
    let mut options = CompileOptions::new(&registry, Phase::Source);
    options.limits = limits;
    let reader = BytesReader::new(bytes.to_vec());
    compile(&Input::new(NAME, &reader), &options).refusal
}

#[test]
fn sniffs_by_name_or_by_the_schema_head() {
    let frontend = IrFrontend::default();
    let hints = |name| SniffHints {
        name,
        zip_names: None,
    };
    let dump = format!("\u{FEFF}{{\n  \"$schema\": \"{SCHEMA_URL}\",\n  \"defaults\": {{}}");
    assert!(frontend.sniff(dump.as_bytes(), &hints("dump.json")));
    assert!(frontend.sniff(b"", &hints("x.presentation-ir.json")));
    assert!(!frontend.sniff(b"{\"size\":{}}", &hints("x.json")));
    assert!(!frontend.sniff(b"{}", &hints("x.presentation-ir.json.bak")));
    // Invalid UTF-8 after the head does not stop sniffing.
    let mut cut = format!("{{\"$schema\":\"{SCHEMA_URL}\"").into_bytes();
    cut.extend_from_slice(&[0xE2, 0x82]);
    assert!(frontend.sniff(&cut, &hints("x.json")));
}

#[test]
fn analyzes_size_show_order_features_and_declared_media_bytes() {
    let (sha, entry) = media_entry(b"png bytes");
    let mut p = presentation(vec![
        slide("ir-s1", vec![shape("ir-s1.a"), picture("ir-s1.p", &sha)]),
        slide("ir-s2", vec![]),
    ]);
    p["slides"][1]["hidden"] = json!(true);
    p["slides"][1]["title"] = json!("Second");
    p["media"][&sha] = entry;
    p["media"]["other"] = json!({ "bytes": 5.5 });
    let reader = BytesReader::new(bytes(&p));
    let registry = FrontendRegistry::new(vec![Box::new(IrFrontend::default())]).unwrap();
    let options = CompileOptions::new(&registry, Phase::Source);
    let analysis = slidock_core::analyze(&Input::new(NAME, &reader), &options.input())
        .analysis
        .unwrap();
    assert_eq!(
        (analysis.size.w.to_f64(), analysis.size.h.to_f64()),
        (720.0, 405.0)
    );
    let slides: Vec<(&str, bool, Option<&str>)> = analysis
        .slides
        .iter()
        .map(|s| (s.id.as_str(), s.hidden, s.title.as_deref()))
        .collect();
    assert_eq!(
        slides,
        [("ir-s1", false, None), ("ir-s2", true, Some("Second"))]
    );
    // The census needs a valid presentation, which the stray media entry is not.
    assert_eq!(analysis.features, vec![]);
    assert_eq!(analysis.media_bytes, 9 + 5);

    p["media"].as_object_mut().unwrap().shift_remove("other");
    let reader = BytesReader::new(bytes(&p));
    let analysis = slidock_core::analyze(&Input::new(NAME, &reader), &options.input())
        .analysis
        .unwrap();
    assert!(analysis.features.contains(&FeatureId::ElementPicture));
    assert!(
        analysis
            .features
            .contains(&FeatureId::PresentationHiddenSlide)
    );
    let mut sorted = analysis.features.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(analysis.features, sorted);
}

#[test]
fn refuses_what_is_not_a_presentation_within_the_limit() {
    let refused = |bytes: &[u8]| {
        let d = refusal_of(bytes, Limits::default()).unwrap();
        let data = d.data.unwrap();
        (
            d.code.code().to_owned(),
            data["path"].to_js_string(),
            data["reason"].to_js_string(),
        )
    };
    let (code, path, reason) = refused(b"{ not json");
    assert_eq!((code.as_str(), path.as_str()), ("IR0001", "(presentation)"));
    assert!(reason.starts_with("Not JSON: "), "{reason}");
    let (_, _, reason) = refused(b"{\"a\":\"\xff\"}");
    assert!(reason.starts_with("Not JSON: "), "{reason}");
    assert_eq!(
        refused(b"[1]"),
        (
            "IR0001".to_owned(),
            "(presentation)".to_owned(),
            "A presentation is a JSON object".to_owned()
        )
    );
    let mut p = presentation(vec![slide("ir-s1", vec![])]);
    p["slides"][0]
        .as_object_mut()
        .unwrap()
        .shift_remove("hidden");
    assert_eq!(
        refused(&bytes(&p)),
        (
            "IR0001".to_owned(),
            "slides".to_owned(),
            "Slides need a string id and a hidden flag".to_owned()
        )
    );
    let mut p = presentation(vec![]);
    p["size"]["h"] = json!("405");
    assert_eq!(
        refused(&bytes(&p)),
        (
            "IR0001".to_owned(),
            "size".to_owned(),
            "The presentation needs a numeric size".to_owned()
        )
    );
    let p = presentation(vec![slide("S1", vec![])]);
    assert_eq!(refused(&bytes(&p)).1, "slides[0].id");

    let p = bytes(&presentation(vec![]));
    let limits = Limits {
        ir_bytes: p.len() as u64 - 1,
        ..Limits::default()
    };
    let d = refusal_of(&p, limits).unwrap();
    assert_eq!(d.code.code(), "IR0903");
    assert_eq!(d.stage, Stage::Frontend);
    assert_eq!(d.emitter, "ir");
    let at_limit = Limits {
        ir_bytes: p.len() as u64,
        ..Limits::default()
    };
    assert_eq!(refusal_of(&p, at_limit), None);
}

#[test]
fn reads_a_byte_order_mark_as_no_part_of_the_text() {
    let mut with_bom = b"\xEF\xBB\xBF".to_vec();
    with_bom.extend(bytes(&presentation(vec![slide("ir-s1", vec![])])));
    assert_eq!(refusal_of(&with_bom, Limits::default()), None);
}

#[test]
fn leaves_a_size_off_the_point_grid_for_the_verifier_to_report() {
    let mut p = presentation(vec![slide("ir-s1", vec![])]);
    p["size"]["w"] = json!(720.0004);
    let registry = FrontendRegistry::new(vec![Box::new(IrFrontend::default())]).unwrap();
    let options = CompileOptions::new(&registry, Phase::Source);
    let reader = BytesReader::new(bytes(&p));
    let result = compile(&Input::new(NAME, &reader), &options);
    assert_eq!(result.refusal, None);
    assert!(result.source.is_none());
    let codes: Vec<&str> = result.diagnostics.iter().map(|d| d.code.code()).collect();
    assert!(codes.contains(&"IR0007"), "{codes:?}");
}

#[test]
fn drops_resolved_only_values_naming_each_kind_once_per_container() {
    let mut p = presentation(vec![
        slide("ir-s1", vec![shape("ir-s1.a"), shape("ir-s1.b")]),
        slide("ir-s2", vec![shape("ir-s2.a")]),
    ]);
    for slide in p["slides"].as_array_mut().unwrap() {
        for element in slide["elements"].as_array_mut().unwrap() {
            element["text"]["paragraphs"][0]["props"]["lineHeight"] =
                json!({ "kind": "ratio", "value": 1.2 });
        }
    }
    for element in p["slides"][0]["elements"].as_array_mut().unwrap() {
        element["inherits"] = json!("ir-l.x");
    }
    p["slides"][0]["title"] = json!("Agent title");
    p["slides"][0]["readingOrder"] = json!(["ir-s1.a"]);
    p["defaults"]["text"]["0"]["para"]["spaceBeforePt"] = json!(0);
    p["fonts"] = json!({ "lato": { "request": {}, "binding": {} }, "inter": { "binding": {} } });
    p["media"] = json!({ "x": { "plan": {} } });
    p["phase"] = json!("resolved");
    let (lowered, diagnostics) = lower(&IrFrontend::default(), &bytes(&p), None);
    let lowered = lowered.unwrap();
    let json = lowered_json(&lowered);
    assert_eq!(json["phase"], "source");
    let element = &json["slides"][0]["elements"][0];
    assert!(element.get("inherits").is_none());
    assert!(
        element["text"]["paragraphs"][0]["props"]
            .get("lineHeight")
            .is_none()
    );
    assert!(json["slides"][0].get("title").is_none());
    assert!(json["slides"][0].get("readingOrder").is_none());
    assert_eq!(
        json["fonts"],
        json!({ "lato": { "request": {} }, "inter": {} })
    );
    assert_eq!(json["media"], json!({ "x": {} }));
    let ir0012: Vec<(String, String, String)> = summary(&diagnostics)
        .into_iter()
        .filter(|(code, _, _)| code == "IR0012")
        .collect();
    let row = |container: &str, field: &str| {
        ("IR0012".to_owned(), container.to_owned(), field.to_owned())
    };
    assert_eq!(
        ir0012,
        [
            row("ir-s1", "lineHeight"),
            row("ir-s1", "inheritance edge"),
            row("ir-s1", "slide title"),
            row("ir-s1", "reading order"),
            row("ir-s2", "lineHeight"),
            row("", "spaceBeforePt"),
            row("", "font binding"),
            row("", "media plan"),
        ]
    );
}

#[test]
fn fails_a_container_that_holds_a_passs_origin_keeping_its_place() {
    let mut p = presentation(vec![
        slide("ir-s1", vec![shape("ir-s1.a")]),
        slide("ir-s2", vec![]),
    ]);
    p["slides"][0]["elements"][0]["origin"] =
        json!({ "kind": "synth", "pass": "bind-placeholders" });
    p["slides"][0]["hidden"] = json!(true);
    let (lowered, diagnostics) = lower(&IrFrontend::default(), &bytes(&p), None);
    let lowered = lowered.unwrap();
    assert_eq!(
        lowered_json(&lowered)["slides"][0],
        json!({ "id": "ir-s1", "origin": { "kind": "agent" }, "hidden": true, "showLayout": true,
                "timeline": [], "elements": [], "failed": { "code": "IR0011" } })
    );
    assert_eq!(
        summary(&diagnostics),
        [(
            "IR0011".to_owned(),
            "ir-s1".to_owned(),
            "elements[0].origin".to_owned()
        )]
    );

    // verify-source accepts the failed slide, and the rest of the presentation compiles.
    let registry = FrontendRegistry::new(vec![Box::new(IrFrontend::default())]).unwrap();
    let options = CompileOptions::new(&registry, Phase::Source);
    let reader = BytesReader::new(bytes(&p));
    let result = compile(&Input::new(NAME, &reader), &options);
    let source = result.source.unwrap();
    assert_eq!(
        source.slides[0].failed.as_ref().unwrap().code.as_str(),
        "IR0011"
    );
    assert!(source.slides[1].failed.is_none());
}

#[test]
fn a_failed_skeleton_keeps_the_flags_as_written_and_defaults_the_rest() {
    let synth = json!({ "kind": "synth", "pass": "bind-placeholders" });
    let mut p = presentation(vec![
        slide("ir-s1", vec![shape("ir-s1.a")]),
        slide("ir-s2", vec![shape("ir-s2.a")]),
    ]);
    p["slides"][0]["showLayout"] = json!(false);
    p["slides"][1]
        .as_object_mut()
        .unwrap()
        .shift_remove("showLayout");
    p["layouts"] = json!({
        "ir-l.a": { "id": "ir-l.a", "origin": { "kind": "agent" }, "master": "ir-m.a", "showMaster": false,
                    "elements": [shape("ir-l.a.x")] },
        "ir-l.b": { "id": "ir-l.b", "origin": { "kind": "agent" }, "master": "ir-m.a", "elements": [shape("ir-l.b.x")] },
    });
    for slide in p["slides"].as_array_mut().unwrap() {
        slide["elements"][0]["origin"] = synth.clone();
    }
    for layout in p["layouts"].as_object_mut().unwrap().values_mut() {
        layout["elements"][0]["origin"] = synth.clone();
    }
    let (lowered, _) = lower(&IrFrontend::default(), &bytes(&p), None);
    let lowered = lowered.unwrap();
    let json = lowered_json(&lowered);
    let flag = |container: &Value, key: &str| container[key].clone();
    assert_eq!(flag(&json["slides"][0], "showLayout"), json!(false));
    assert_eq!(flag(&json["slides"][1], "showLayout"), json!(true));
    assert_eq!(flag(&json["slides"][1], "hidden"), json!(false));
    assert_eq!(flag(&json["layouts"]["ir-l.a"], "showMaster"), json!(false));
    assert_eq!(
        json["layouts"]["ir-l.b"],
        json!({ "id": "ir-l.b", "origin": { "kind": "agent" }, "master": "ir-m.a", "showMaster": true,
                "elements": [], "failed": { "code": "IR0011" } })
    );
}

#[test]
fn names_the_synthesized_origin_found_first_in_javascript_key_order() {
    let mut p = presentation(vec![slide("ir-s1", vec![shape("ir-s1.a")])]);
    // Array-index keys come first, in numeric order, whatever order they were written in.
    let element = r#"{"b":{"kind":"synth","pass":"b"},"10":{"kind":"synth","pass":"a"},"9":{"kind":"synth","pass":"c"}}"#;
    p["slides"][0]["elements"]
        .as_array_mut()
        .unwrap()
        .push(json!("ELEMENT"));
    let text = String::from_utf8(bytes(&p))
        .unwrap()
        .replace("\"ELEMENT\"", element);
    let (_, diagnostics) = lower(&IrFrontend::default(), text.as_bytes(), None);
    assert_eq!(
        summary(&diagnostics),
        [(
            "IR0011".to_owned(),
            "ir-s1".to_owned(),
            "elements[1].9".to_owned()
        )]
    );
}

#[test]
fn reads_a_lone_surrogate_escape_as_the_replacement_character() {
    let p = presentation(vec![slide("ir-s1", vec![shape("ir-s1.a")])]);
    let text = String::from_utf8(bytes(&p))
        .unwrap()
        .replace("\"Hi\"", r#""Hi \ud800 \udc00\ud800 \ud83d\ude00""#);
    let (lowered, diagnostics) = lower(&IrFrontend::default(), text.as_bytes(), None);
    let lowered = lowered.unwrap();
    assert_eq!(
        lowered_json(&lowered)["slides"][0]["elements"][0]["text"]["paragraphs"][0]["inlines"][0]["text"],
        "Hi \u{FFFD} \u{FFFD}\u{FFFD} \u{1F600}"
    );
    assert_eq!(summary(&diagnostics), []);
}

#[test]
fn leaves_a_size_past_the_grid_range_for_the_verifier_to_report() {
    let registry = FrontendRegistry::new(vec![Box::new(IrFrontend::default())]).unwrap();
    let options = CompileOptions::new(&registry, Phase::Source);
    for (w, h) in [(1e300, 405.0), (720.0, -1e300), (2e12, 405.0)] {
        let mut p = presentation(vec![slide("ir-s1", vec![])]);
        p["size"] = json!({ "w": w, "h": h });
        let reader = BytesReader::new(bytes(&p));
        let result = compile(&Input::new(NAME, &reader), &options);
        assert_eq!(result.refusal, None, "{w} {h}");
        assert!(result.source.is_none());
        let codes: Vec<&str> = result.diagnostics.iter().map(|d| d.code.code()).collect();
        assert!(codes.contains(&"IR0008"), "{w} {h}: {codes:?}");
    }
}

#[test]
fn trusts_media_bytes_only_once_they_hash_to_their_name() {
    let (good, good_entry) = media_entry(b"good");
    let (bad, bad_entry) = media_entry(b"claimed");
    let (gone, gone_entry) = media_entry(b"gone");
    let mut p = presentation(vec![slide(
        "ir-s1",
        vec![
            picture("ir-s1.a", &good),
            picture("ir-s1.b", &bad),
            picture("ir-s1.c", &gone),
        ],
    )]);
    p["media"] = json!({ &good: good_entry, &bad: bad_entry, &gone: gone_entry });
    let mut media = MediaFiles::new();
    media.insert_bytes(&format!("{good}.png"), b"good".to_vec());
    media.insert_bytes(&format!("{bad}.png"), b"other bytes".to_vec());
    let (lowered, diagnostics) = lower(&IrFrontend::new(media), &bytes(&p), None);
    let lowered = lowered.unwrap();
    let actual = Sha256::of(b"other bytes").to_hex();
    let mut expected = vec![
        ("IR0013".to_owned(), String::new(), "missing".to_owned()),
        (
            "IR0013".to_owned(),
            String::new(),
            format!("not the bytes hashed ({}…)", &actual[..12]),
        ),
    ];
    if gone > bad {
        expected.reverse();
    }
    assert_eq!(summary(&diagnostics), expected);
    let store = &lowered.media;
    let handle = store.open(&Sha256::parse(&good).unwrap()).unwrap();
    assert_eq!(handle.bytes, 4);
    assert!(store.open(&Sha256::parse(&bad).unwrap()).is_none());
    assert!(store.open(&Sha256::parse(&gone).unwrap()).is_none());
}

#[test]
fn lowers_only_the_chosen_slides_with_the_containers_and_media_they_use() {
    let (one, one_entry) = media_entry(b"one");
    let (two, two_entry) = media_entry(b"two");
    let mut p = presentation(vec![
        slide("ir-s1", vec![picture("ir-s1.p", &one)]),
        slide("ir-s2", vec![picture("ir-s2.p", &two)]),
    ]);
    p["media"] = json!({ &one: one_entry, &two: two_entry });
    let only = [id("ir-s2")];
    let (lowered, _) = lower(&IrFrontend::default(), &bytes(&p), Some(&only));
    let lowered = lowered.unwrap();
    let json = lowered_json(&lowered);
    let ids: Vec<&str> = json["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["ir-s2"]);
    let media: Vec<&String> = json["media"].as_object().unwrap().keys().collect();
    assert_eq!(media, [&two]);
}

#[test]
fn records_provenance_and_the_census_of_what_it_lowered() {
    let p = bytes(&presentation(vec![slide("ir-s1", vec![shape("ir-s1.a")])]));
    let (lowered, _) = lower(&IrFrontend::default(), &p, None);
    let lowered = lowered.unwrap();
    assert_eq!(lowered.provenance.name, NAME);
    assert_eq!(lowered.provenance.sha256, Sha256::of(&p));
    assert_eq!(lowered.provenance.bytes, p.len() as u64);
    assert_eq!(lowered.provenance.format.as_str(), "ir");
    assert!(lowered.features["ir-s1"].contains(&FeatureId::ElementShape));
    assert_eq!(lowered.features["presentation"], vec![]);
    assert!(lowered.fonts.is_empty());
}

#[test]
fn a_canonical_dump_is_itself_an_input_that_reads_back_to_the_same_ir() {
    let registry = FrontendRegistry::new(vec![Box::new(IrFrontend::default())]).unwrap();
    let options = CompileOptions::new(&registry, Phase::Source);
    let source = |name: &str, bytes: Vec<u8>| {
        let reader = BytesReader::new(bytes);
        compile(&Input::new(name, &reader), &options)
            .source
            .unwrap()
    };
    let first = source(
        NAME,
        bytes(&presentation(vec![slide("ir-s1", vec![shape("ir-s1.a")])])),
    );
    let dump = dump_presentation(&first).unwrap();
    assert!(dump.starts_with(&format!("{{\n  \"$schema\": \"{SCHEMA_URL}\"")));
    // Sniffed by its head alone.
    let again = source("dump.json", dump.clone().into_bytes());
    assert_eq!(dump_presentation(&again).unwrap(), dump);
}

/// A document that repeats a member of an object, at any depth, is refused: no repeat of `$schema` (or of any member)
/// can hide a value the reader would otherwise refuse.
#[test]
fn refuses_a_document_that_repeats_a_member() {
    let p = presentation(vec![slide("ir-s1", vec![shape("ir-s1.a")])]);
    let text = String::from_utf8(bytes(&p)).unwrap();
    // The same document with every member once reads.
    assert!(refusal_of(text.as_bytes(), Limits::default()).is_none());
    let other = "https://example.com/presentation-ir.schema.json";
    let own = format!("\"$schema\":\"{SCHEMA_URL}\"");
    let repeated_schema = text.replacen(&own, &format!("\"$schema\":\"{other}\",{own}"), 1);
    assert_ne!(repeated_schema, text);
    let repeated_hidden = text.replacen("\"hidden\":false", "\"hidden\":true,\"hidden\":false", 1);
    assert_ne!(repeated_hidden, text);
    for (case, member) in [(repeated_schema, "$schema"), (repeated_hidden, "hidden")] {
        let d = refusal_of(case.as_bytes(), Limits::default())
            .unwrap_or_else(|| panic!("a repeated {member:?} is read"));
        let data = d.data.unwrap();
        assert_eq!(d.code.code(), "IR0001", "{member}");
        assert_eq!(data["path"].to_js_string(), "(presentation)");
        let reason = data["reason"].to_js_string();
        assert!(
            reason.starts_with(&format!("A JSON object repeats the member \"{member}\"")),
            "{reason}"
        );
    }
}

/// Only the IR's own `$schema` reads as IR: a document naming another, or none, is refused before it is lowered, with
/// IR0001 at `$schema` and the reason the verifier gives for that value, and one naming another does not sniff as IR
/// by its head.
#[test]
fn refuses_a_document_of_any_other_schema_or_none() {
    assert_eq!(SCHEMA_URL, "urn:slidock:presentation-ir:1");
    let registry = FrontendRegistry::new(vec![Box::new(IrFrontend::default())]).unwrap();
    let options = CompileOptions::new(&registry, Phase::Source);
    let compiled = |p: &Value| {
        let reader = BytesReader::new(bytes(p));
        compile(&Input::new(NAME, &reader), &options)
    };
    let mut p = presentation(vec![slide("ir-s1", vec![shape("ir-s1.a")])]);
    let own = compiled(&p);
    assert!(own.source.is_some(), "{:?}", own.diagnostics);
    assert!(own.refusal.is_none());
    let expected = format!("expected \"{SCHEMA_URL}\"");
    let cases = [
        (
            Some(json!("https://example.com/presentation-ir.schema.json")),
            format!(
                "invalid value: string \"https://example.com/presentation-ir.schema.json\", {expected}"
            ),
        ),
        (
            Some(json!("urn:slidock:presentation-ir:2")),
            format!("invalid value: string \"urn:slidock:presentation-ir:2\", {expected}"),
        ),
        (
            Some(json!("")),
            format!("invalid value: string \"\", {expected}"),
        ),
        (
            Some(json!(1)),
            "invalid type: integer `1`, expected a string".to_owned(),
        ),
        (
            Some(Value::Null),
            "invalid type: null, expected a string".to_owned(),
        ),
        (None, "missing field `$schema`".to_owned()),
    ];
    for (schema, reason) in cases {
        match &schema {
            Some(value) => p["$schema"] = value.clone(),
            None => {
                p.as_object_mut().unwrap().shift_remove("$schema");
            }
        }
        let result = compiled(&p);
        assert!(result.source.is_none(), "{schema:?} is read as IR");
        let refusal = result
            .refusal
            .unwrap_or_else(|| panic!("{schema:?} is not refused: {:?}", result.diagnostics));
        assert_eq!(
            (refusal.code.code(), refusal.stage, refusal.container),
            ("IR0001", Stage::Frontend, None),
            "{schema:?}"
        );
        let data = refusal.data.unwrap();
        assert_eq!(data["path"].to_js_string(), "$schema", "{schema:?}");
        assert_eq!(data["reason"].to_js_string(), reason, "{schema:?}");
        let mut sink = Collect::new(Stage::Frontend, "ir");
        let limits = Limits::default();
        let reader = BytesReader::new(bytes(&p));
        let analysis = IrFrontend::default().analyze(
            &Input::new(NAME, &reader),
            &mut FrontendCtx {
                limits: &limits,
                diag: &mut sink,
                deadline: Deadline::never(),
                spent: slidock_core_api::Output::default(),
            },
        );
        assert!(analysis.is_err(), "{schema:?} analyzes");
        if let Some(Value::String(other)) = &schema {
            let head = format!("{{\"$schema\":\"{other}\"}}");
            let hints = SniffHints {
                name: "x.json",
                zip_names: None,
            };
            assert!(
                !IrFrontend::default().sniff(head.as_bytes(), &hints),
                "{other:?}"
            );
        }
    }
}
