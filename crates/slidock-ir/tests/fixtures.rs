//! The verifier, narrowing and census on the private full and minimal presentations of both phases (`--features
//! golden`, tests/golden; skips when they are absent), plus the verifier on every golden presentation.
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
use std::path::PathBuf;
use std::sync::OnceLock;

use golden::Golden;

use serde_json::{Value, json};
use slidock_ir::diagnostics::{Diagnostic, codes};
use slidock_ir::features::{FeatureId, census_container, census_presentation};
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::source::{self, Container};
use slidock_ir::verify::{
    self, CheckOptions, Invariant, VerifyOptions, check_invariant, parse_presentation_json,
    verify_invariants, verify_resolved_json, verify_source_json,
};

/// The golden files, opened once; `None` when they are absent.
fn golden() -> Option<&'static Golden> {
    static GOLDEN: OnceLock<Option<Golden>> = OnceLock::new();
    GOLDEN
        .get_or_init(|| Golden::open("slidock-ir fixtures"))
        .as_ref()
}

/// A private presentation fixture: `<phase>/<name>.presentation-ir.json`.
fn fixture(name: &str) -> Value {
    let path = golden().unwrap().fixture(&format!("presentation/{name}"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap()
}

const SOURCE_FIXTURES: [&str; 2] = [
    "source/minimal.presentation-ir.json",
    "source/full.presentation-ir.json",
];
const RESOLVED_FIXTURES: [&str; 2] = [
    "resolved/minimal.presentation-ir.json",
    "resolved/full.presentation-ir.json",
];

/// `code@container` per diagnostic, `presentation` for presentation-level ones.
fn located(diagnostics: &[Diagnostic]) -> Vec<String> {
    diagnostics
        .iter()
        .map(|d| {
            format!(
                "{}@{}",
                d.code,
                d.container.as_ref().map_or("presentation", NodeId::as_str)
            )
        })
        .collect()
}

fn element<'a>(presentation: &'a mut Value, slide: usize, id: &str) -> &'a mut Value {
    presentation["slides"][slide]["elements"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|e| e["id"] == id)
        .unwrap_or_else(|| panic!("fixture element {id}"))
}

fn remove(value: &mut Value, key: &str) {
    value.as_object_mut().unwrap().shift_remove(key);
}

fn agent() -> Value {
    json!({ "kind": "agent" })
}

fn sha(text: &str) -> String {
    Sha256::of(text.as_bytes()).to_hex()
}

fn verify_source(presentation: &Value) -> Vec<Diagnostic> {
    verify_source_json(presentation, &VerifyOptions::default())
}

fn verify_resolved(
    presentation: &Value,
) -> Result<slidock_ir::resolved::Presentation, Vec<Diagnostic>> {
    verify_resolved_json(presentation, &VerifyOptions::default())
}

fn typed(presentation: &Value) -> source::Presentation {
    parse_presentation_json(presentation).unwrap_or_else(|d| panic!("{:?}", located(&d)))
}

#[test]
fn verify_source_accepts_the_source_fixtures() {
    if golden().is_none() {
        return;
    }
    for name in SOURCE_FIXTURES {
        assert_eq!(verify_source(&fixture(name)), vec![], "{name}");
    }
}

#[test]
fn verify_source_reports_resolved_fixtures_as_the_wrong_phase_only() {
    if golden().is_none() {
        return;
    }
    for name in RESOLVED_FIXTURES {
        assert_eq!(
            located(&verify_source(&fixture(name))),
            ["IR0009@presentation"],
            "{name}"
        );
    }
}

type Mutation = fn(&mut Value);

fn source_rules() -> Vec<(&'static str, Mutation, Vec<&'static str>)> {
    vec![
        (
            "IR0001 schema-invalid",
            |p| element(p, 0, "ir-s1.arrow")["kind"] = json!("blob"),
            vec!["IR0001@ir-s1"],
        ),
        (
            "IR0001 for a theme, on the masters using it",
            |p| p["themes"]["ir-theme"]["colors"]["dk1"]["kind"] = json!("cmyk"),
            vec!["IR0001@ir-master"],
        ),
        (
            "IR0001 for a map key that differs from the id",
            |p| {
                let mut other = p["layouts"]["ir-layout"].clone();
                other["elements"] = json!([]);
                p["layouts"]["ir-other"] = other;
            },
            vec!["IR0001@ir-other", "IR0002@presentation"],
        ),
        (
            "IR0002 duplicate-id within a container",
            |p| element(p, 0, "ir-s1.photo")["id"] = json!("ir-s1.arrow"),
            vec!["IR0002@ir-s1"],
        ),
        (
            "IR0002 duplicate-id across containers",
            |p| element(p, 1, "ir-s2.note")["id"] = json!("ir-s1.arrow"),
            vec!["IR0002@presentation"],
        ),
        (
            "IR0003 layout",
            |p| p["slides"][0]["layout"] = json!("ir-gone"),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 master",
            |p| p["layouts"]["ir-layout"]["master"] = json!("ir-gone"),
            vec!["IR0003@ir-layout"],
        ),
        (
            "IR0003 theme",
            |p| p["masters"]["ir-master"]["theme"] = json!("ir-gone"),
            vec!["IR0003@ir-master"],
        ),
        (
            "IR0003 inherits",
            |p| element(p, 0, "ir-s1.title")["inherits"] = json!("ir-gone"),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 placeholder-target",
            |p| element(p, 0, "ir-s1.title")["placeholder"]["target"] = json!("ir-gone"),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 timeline-target",
            |p| p["slides"][0]["timeline"][0]["target"]["element"] = json!("ir-s2.note"),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 slide-link in a run",
            |p| {
                element(p, 0, "ir-s1.body")["text"]["paragraphs"][0]["inlines"][1]["props"]["link"] =
                    json!({ "kind": "slide", "slide": "ir-s9" });
            },
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 media",
            |p| element(p, 0, "ir-s1.photo")["media"] = json!(sha("missing")),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 fallback",
            |p| element(p, 0, "ir-s1.photo")["fallback"] = json!(sha("missing")),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 poster",
            |p| element(p, 0, "ir-s1.video")["poster"] = json!(sha("missing")),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0004 cycle",
            |p| {
                p["layouts"]["ir-layout"]["elements"][0]["inherits"] = json!("ir-s1.title");
                element(p, 0, "ir-s1.title")["inherits"] = json!("ir-layout.title");
            },
            vec!["IR0004@ir-layout"],
        ),
        (
            "IR0005 inherits chain",
            |p| {
                for i in 0..10 {
                    let mut e =
                        json!({ "id": format!("ir-chain{i}"), "origin": agent(), "kind": "shape" });
                    if i > 0 {
                        e["inherits"] = json!(format!("ir-chain{}", i - 1));
                    }
                    p["slides"][1]["elements"].as_array_mut().unwrap().push(e);
                }
            },
            vec!["IR0005@ir-s2"],
        ),
        (
            "IR0005 group depth",
            |p| {
                let mut group =
                    json!({ "id": "ir-g16", "origin": agent(), "kind": "group", "children": [] });
                for depth in (0..=15).rev() {
                    group = json!({ "id": format!("ir-g{depth}"), "origin": agent(), "kind": "group", "children": [group] });
                }
                p["slides"][1]["elements"]
                    .as_array_mut()
                    .unwrap()
                    .push(group);
            },
            vec!["IR0005@ir-s2"],
        ),
        (
            "IR0006 too-many-slides",
            |p| {
                for i in 0..300 {
                    p["slides"].as_array_mut().unwrap().push(json!({
                        "id": format!("ir-x{i}"), "origin": agent(), "hidden": false, "showLayout": true,
                        "elements": [], "timeline": []
                    }));
                }
            },
            vec!["IR0006@presentation"],
        ),
        (
            "IR0007 number-not-quantised",
            |p| element(p, 0, "ir-s1.photo")["frame"]["x"] = json!(10.0005),
            vec!["IR0007@ir-s1"],
        ),
        (
            "IR0007 inside a union branch",
            |p| element(p, 0, "ir-s1.arrow")["fill"]["stops"][0]["pos"] = json!(0.123456),
            vec!["IR0007@ir-s1"],
        ),
        (
            "IR0008 string",
            |p| element(p, 0, "ir-s1.photo")["alt"] = json!("x".repeat(70_000)),
            vec!["IR0008@ir-s1"],
        ),
        (
            "IR0008 elements per container",
            |p| {
                for i in 0..5001 {
                    p["slides"][1]["elements"].as_array_mut().unwrap().push(
                        json!({ "id": format!("ir-e{i}"), "origin": agent(), "kind": "shape" }),
                    );
                }
            },
            vec!["IR0008@ir-s2"],
        ),
        (
            "IR0008 text per body",
            |p| {
                let runs: Vec<Value> = (0..16)
                    .map(|_| json!({ "kind": "run", "text": "x".repeat(65_000), "props": {} }))
                    .collect();
                element(p, 1, "ir-s2.note")["text"]["paragraphs"][0]["inlines"] = json!(runs);
            },
            vec!["IR0008@ir-s2"],
        ),
        (
            "IR0008 ext per node",
            |p| {
                element(p, 0, "ir-s1.arrow")["ext"] =
                    json!({ "pptx": { "blob": "x".repeat(20_000) } })
            },
            vec!["IR0008@ir-s1"],
        ),
        (
            "IR0008 ext per presentation",
            |p| {
                for i in 0..70 {
                    p["slides"][1]["elements"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({
                            "id": format!("ir-x{i}"), "origin": agent(), "kind": "shape",
                            "ext": { "pptx": { "blob": "x".repeat(16_000) } }
                        }));
                }
            },
            vec!["IR0008@presentation"],
        ),
        (
            "IR0008 payload",
            |p| {
                let cells: Vec<String> = (0..300).map(|_| "x".repeat(1000)).collect();
                element(p, 0, "ir-s1.table")["payload"] = json!({ "cells": cells });
            },
            vec!["IR0008@ir-s1"],
        ),
        (
            "IR0008 cells per container",
            |p| {
                let grid = element(p, 1, "ir-s2.grid");
                grid["columns"] =
                    json!((0..100).map(|_| json!({ "width": 1 })).collect::<Vec<_>>());
                grid["rows"] = json!(
                    (0..51)
                        .map(|_| json!({ "height": 1, "cells": [] }))
                        .collect::<Vec<_>>()
                );
                let mut copy = grid.clone();
                copy["id"] = json!("ir-s2.grid2");
                p["slides"][1]["elements"]
                    .as_array_mut()
                    .unwrap()
                    .push(copy);
            },
            vec!["IR0008@ir-s2"],
        ),
        (
            "IR0008 cells per presentation, in containers each within their own limit",
            |p| {
                let grid = element(p, 1, "ir-s2.grid");
                grid["columns"] =
                    json!((0..100).map(|_| json!({ "width": 1 })).collect::<Vec<_>>());
                grid["rows"] = json!(
                    (0..100)
                        .map(|_| json!({ "height": 1, "cells": [] }))
                        .collect::<Vec<_>>()
                );
                let grid = grid.clone();
                for i in 0..5 {
                    let mut slide = p["slides"][1].clone();
                    slide["id"] = json!(format!("ir-c{i}"));
                    let mut copy = grid.clone();
                    copy["id"] = json!(format!("ir-c{i}.grid"));
                    slide["elements"] = json!([copy]);
                    slide["timeline"] = json!([]);
                    p["slides"].as_array_mut().unwrap().push(slide);
                }
            },
            vec!["IR0008@presentation"],
        ),
        (
            "IR0009 phase-mismatch",
            |p| p["phase"] = json!("resolved"),
            vec!["IR0009@presentation"],
        ),
        (
            "IR0011 origin-invalid",
            |p| {
                element(p, 0, "ir-s1.arrow")["origin"] =
                    json!({ "kind": "source", "part": "/Users/me/presentation.pptx" })
            },
            vec!["IR0011@ir-s1"],
        ),
        (
            "IR0014 a row with a slot per column",
            |p| {
                element(p, 1, "ir-s2.grid")["rows"][1]["cells"]
                    .as_array_mut()
                    .unwrap()
                    .pop();
            },
            vec!["IR0014@ir-s2"],
        ),
        (
            "IR0014 a span past the grid",
            |p| element(p, 1, "ir-s2.grid")["rows"][1]["cells"][1]["rowSpan"] = json!(2),
            vec!["IR0014@ir-s2"],
        ),
        (
            "IR0014 a slot covered twice",
            |p| {
                let rows = &mut element(p, 1, "ir-s2.grid")["rows"];
                rows[0]["cells"][1] = rows[1]["cells"][1].clone();
            },
            vec!["IR0014@ir-s2"],
        ),
        (
            "IR0014 a slot no cell covers",
            |p| {
                remove(
                    &mut element(p, 1, "ir-s2.grid")["rows"][0]["cells"][0],
                    "colSpan",
                )
            },
            vec!["IR0014@ir-s2"],
        ),
        (
            "IR0015 a frame wider than its columns",
            |p| element(p, 1, "ir-s2.grid")["columns"][0]["width"] = json!(90),
            vec!["IR0015@ir-s2"],
        ),
        (
            "IR0015 a frame shorter than its rows",
            |p| element(p, 1, "ir-s2.grid")["frame"]["h"] = json!(60),
            vec!["IR0015@ir-s2"],
        ),
    ]
}

#[test]
fn verify_source_rules() {
    if golden().is_none() {
        return;
    }
    for (rule, mutate, expected) in source_rules() {
        let mut presentation = fixture("source/full.presentation-ir.json");
        mutate(&mut presentation);
        assert_eq!(located(&verify_source(&presentation)), expected, "{rule}");
    }
}

fn data(diagnostic: &Diagnostic, key: &str) -> String {
    diagnostic
        .data
        .as_ref()
        .and_then(|d| d.get(key))
        .map(|v| v.to_js_string())
        .unwrap_or_default()
}

#[test]
fn names_a_table_grid_problem_by_its_slot() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("source/full.presentation-ir.json");
    let rows = &mut element(&mut presentation, 1, "ir-s2.grid")["rows"];
    rows[0]["cells"][1] = rows[1]["cells"][1].clone();
    let diagnostics = verify_source(&presentation);
    assert_eq!(diagnostics.len(), 1);
    let d = &diagnostics[0];
    assert_eq!(d.code, codes::IR0014);
    assert_eq!(d.node.as_ref().map(NodeId::as_str), Some("ir-s2.grid"));
    assert_eq!(data(d, "path"), "slides[1].elements[1].rows[0].cells[1]");
    assert_eq!(
        data(d, "reason"),
        "the slot at row 0, column 1 is covered 2 times"
    );
}

#[test]
fn names_the_table_of_an_issue_inside_a_cell() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("source/full.presentation-ir.json");
    element(&mut presentation, 1, "ir-s2.grid")["rows"][1]["cells"][0]["borders"]["right"]["width"] =
        json!(1.0005);
    let diagnostics = verify_source(&presentation);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].code, codes::IR0007);
    assert_eq!(
        diagnostics[0].node.as_ref().map(NodeId::as_str),
        Some("ir-s2.grid")
    );
    assert_eq!(
        data(&diagnostics[0], "path"),
        "slides[1].elements[1].rows[1].cells[0].borders.right.width"
    );
}

#[test]
fn names_the_node_of_an_element_level_issue() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("source/full.presentation-ir.json");
    presentation["slides"][0]["elements"][4]["children"][1]["children"][0]["frame"]["y"] =
        json!(0.0001);
    let d = &verify_source(&presentation)[0];
    assert_eq!(d.code, codes::IR0007);
    assert_eq!(d.container.as_ref().map(NodeId::as_str), Some("ir-s1"));
    assert_eq!(
        d.node.as_ref().map(NodeId::as_str),
        Some("ir-s1.group.inner.logo")
    );
    assert_eq!(
        data(d, "path"),
        "slides[0].elements[4].children[1].children[0].frame.y"
    );
    assert_eq!(d.stage, slidock_ir::diagnostics::Stage::Verify);
    assert_eq!(d.emitter, "verify-source");
}

#[test]
fn verify_resolved_accepts_the_resolved_fixtures() {
    if golden().is_none() {
        return;
    }
    for name in RESOLVED_FIXTURES {
        let verified =
            verify_resolved(&fixture(name)).unwrap_or_else(|d| panic!("{name}: {:?}", located(&d)));
        assert_eq!(
            verified.slides.len(),
            fixture(name)["slides"].as_array().unwrap().len()
        );
    }
}

#[test]
fn verify_resolved_flips_the_phase() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("resolved/minimal.presentation-ir.json");
    presentation["phase"] = json!("source");
    let verified = verify_resolved(&presentation).unwrap();
    assert_eq!(
        slidock_ir::canonical::to_json(&verified.phase).unwrap(),
        "\"resolved\""
    );
}

#[test]
fn verify_resolved_refuses_source_fixtures_with_container_attributed_violations() {
    if golden().is_none() {
        return;
    }
    for name in SOURCE_FIXTURES {
        let diagnostics = verify_resolved(&fixture(name)).unwrap_err();
        assert!(!diagnostics.is_empty(), "{name}");
        for d in &diagnostics {
            if d.code != codes::IR0106 {
                assert!(d.container.is_some(), "{name}: {d:?}");
            }
        }
    }
}

fn resolved_rules() -> Vec<(&'static str, Invariant, Mutation, Vec<&'static str>)> {
    vec![
        (
            "IR0105 child space",
            Invariant::FramesCanonical,
            |p| {
                element(p, 0, "ir-s1.group")["childSpace"] =
                    json!({ "x": 0, "y": 0, "w": 1, "h": 1 })
            },
            vec!["IR0105@ir-s1"],
        ),
        (
            "IR0105 group flip",
            Invariant::FramesCanonical,
            |p| element(p, 0, "ir-s1.group")["frame"]["flipV"] = json!(true),
            vec!["IR0105@ir-s1"],
        ),
        (
            "IR0108 frame-missing",
            Invariant::InheritanceResolved,
            |p| remove(element(p, 0, "ir-s1.photo"), "frame"),
            vec!["IR0108@ir-s1"],
        ),
        (
            "IR0115 body-props-incomplete",
            Invariant::InheritanceResolved,
            |p| remove(&mut element(p, 1, "ir-s2.note")["text"]["body"], "anchor"),
            vec!["IR0115@ir-s2"],
        ),
        (
            "IR0115 in a table cell",
            Invariant::InheritanceResolved,
            |p| {
                remove(
                    &mut element(p, 1, "ir-s2.grid")["rows"][1]["cells"][0]["text"]["body"],
                    "insets",
                )
            },
            vec!["IR0115@ir-s2"],
        ),
        (
            "IR0101 theme-color-unresolved",
            Invariant::ThemeResolved,
            |p| {
                remove(
                    &mut element(p, 0, "ir-s1.arrow")["fill"]["stops"][0]["color"],
                    "resolved",
                )
            },
            vec!["IR0101@ir-s1"],
        ),
        (
            "IR0101 in master text styles",
            Invariant::ThemeResolved,
            |p| {
                remove(
                    &mut p["masters"]["ir-master"]["textStyles"]["title"]["0"]["run"]["color"],
                    "resolved",
                )
            },
            vec!["IR0101@ir-master"],
        ),
        (
            "IR0114 theme-font-unresolved",
            Invariant::ThemeResolved,
            |p| {
                p["masters"]["ir-master"]["textStyles"]["title"]["0"]["run"]["font"] =
                    json!({ "kind": "theme", "slot": "major" });
            },
            vec!["IR0114@ir-master"],
        ),
        (
            "IR0102 paragraph-props-incomplete",
            Invariant::TextCascaded,
            |p| {
                remove(
                    &mut element(p, 0, "ir-s1.title")["text"]["paragraphs"][0]["props"],
                    "align",
                )
            },
            vec!["IR0102@ir-s1"],
        ),
        (
            "IR0102 in a table cell",
            Invariant::TextCascaded,
            |p| {
                remove(
                    &mut element(p, 1, "ir-s2.grid")["rows"][0]["cells"][0]["text"]["paragraphs"]
                        [0]["props"],
                    "indent",
                );
            },
            vec!["IR0102@ir-s2"],
        ),
        (
            "IR0103 run-props-incomplete",
            Invariant::TextCascaded,
            |p| {
                remove(
                    &mut element(p, 0, "ir-s1.body")["text"]["paragraphs"][1]["inlines"][1]["props"],
                    "size",
                )
            },
            vec!["IR0103@ir-s1"],
        ),
        (
            "IR0103 on end props",
            Invariant::TextCascaded,
            |p| {
                remove(
                    &mut element(p, 0, "ir-s1.body")["text"]["paragraphs"][1]["endProps"],
                    "color",
                )
            },
            vec!["IR0103@ir-s1"],
        ),
        (
            "IR0111 bullet-unlabelled",
            Invariant::ListsLabelled,
            |p| {
                element(p, 0, "ir-s1.body")["text"]["paragraphs"][0]["props"]["bullet"] =
                    json!({ "kind": "char", "char": "•" });
            },
            vec!["IR0111@ir-s1"],
        ),
        (
            "IR0104 font-unbound, on the map and on the run",
            Invariant::FontsBound,
            |p| remove(&mut p["fonts"]["liberationmono"], "binding"),
            vec!["IR0104@presentation", "IR0104@ir-s1"],
        ),
        (
            "IR0110 line-metrics-missing",
            Invariant::TextMetrics,
            |p| {
                remove(
                    &mut element(p, 1, "ir-s2.note")["text"]["paragraphs"][0]["props"],
                    "lineHeight",
                )
            },
            vec!["IR0110@ir-s2"],
        ),
        (
            "IR0110 in a table cell",
            Invariant::TextMetrics,
            |p| {
                remove(
                    &mut element(p, 1, "ir-s2.grid")["rows"][1]["cells"][1]["text"]["paragraphs"]
                        [0]["props"],
                    "spaceAfterPt",
                );
            },
            vec!["IR0110@ir-s2"],
        ),
        (
            "IR0107 autofit-incomplete",
            Invariant::TextMetrics,
            |p| {
                remove(
                    &mut element(p, 1, "ir-s2.note")["text"]["body"]["autofit"],
                    "fitted",
                )
            },
            vec!["IR0107@ir-s2"],
        ),
        (
            "IR0105 geometry-not-lowered",
            Invariant::GeometryLowered,
            |p| {
                element(p, 1, "ir-s2.note")["geometry"] =
                    json!({ "kind": "preset", "name": "roundRect", "adjust": {} })
            },
            vec!["IR0105@ir-s2"],
        ),
        (
            "IR0105 on a picture clip",
            Invariant::GeometryLowered,
            |p| {
                element(p, 0, "ir-s1.photo")["clip"] =
                    json!({ "kind": "preset", "name": "ellipse", "adjust": {} })
            },
            vec!["IR0105@ir-s1"],
        ),
        (
            "IR0113 source-form fill",
            Invariant::GeometryLowered,
            |p| {
                let stops = p["slides"][1]["background"]["stops"].clone();
                p["slides"][1]["background"] =
                    json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "stops": stops });
            },
            vec!["IR0113@ir-s2"],
        ),
        (
            "IR0113 text rotation",
            Invariant::GeometryLowered,
            |p| remove(&mut element(p, 0, "ir-s1.title")["text"], "rotation"),
            vec!["IR0113@ir-s1"],
        ),
        (
            "IR0113 a table cell fill",
            Invariant::GeometryLowered,
            |p| {
                let cell = &mut element(p, 1, "ir-s2.grid")["rows"][1]["cells"][1];
                let stops = cell["fill"]["stops"].clone();
                cell["fill"] = json!({ "kind": "linear", "angle": 90, "scaled": false, "rotateWithShape": true, "stops": stops });
            },
            vec!["IR0113@ir-s2"],
        ),
        (
            "IR0113 a table cell text rotation",
            Invariant::GeometryLowered,
            |p| {
                remove(
                    &mut element(p, 1, "ir-s2.grid")["rows"][0]["cells"][0]["text"],
                    "rotation",
                )
            },
            vec!["IR0113@ir-s2"],
        ),
        (
            "IR0106 media-unplanned",
            Invariant::MediaPlanned,
            |p| {
                let (_, first) = p["media"]
                    .as_object_mut()
                    .unwrap()
                    .iter_mut()
                    .next()
                    .unwrap();
                remove(first, "plan");
            },
            vec!["IR0106@presentation"],
        ),
        (
            "IR0109 a11y-missing",
            Invariant::A11yDone,
            |p| remove(&mut p["slides"][0], "readingOrder"),
            vec!["IR0109@ir-s1"],
        ),
        (
            "IR0003 inherits (placeholders.bound)",
            Invariant::PlaceholdersBound,
            |p| element(p, 0, "ir-s1.title")["inherits"] = json!("ir-gone"),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 timeline-target (timeline.normalized)",
            Invariant::TimelineNormalized,
            |p| p["slides"][0]["timeline"][0]["target"]["element"] = json!("ir-gone"),
            vec!["IR0003@ir-s1"],
        ),
    ]
}

#[test]
fn verify_resolved_rules() {
    if golden().is_none() {
        return;
    }
    for (rule, invariant, mutate, expected) in resolved_rules() {
        let mut presentation = fixture("resolved/full.presentation-ir.json");
        mutate(&mut presentation);
        let diagnostics = verify_resolved(&presentation).map(|_| ()).unwrap_err();
        assert_eq!(located(&diagnostics), expected, "{rule}");
        let checked = check_invariant(invariant, &typed(&presentation), &CheckOptions::default());
        assert_eq!(located(&checked), expected, "{rule} (checker)");
        for code in &diagnostics {
            assert!(
                invariant.rules().contains(&code.code),
                "{rule}: {}",
                code.code
            );
        }
    }
}

#[test]
fn holds_a_cell_gradient_by_its_angle_which_only_a_cell_fill_may() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("resolved/full.presentation-ir.json");
    let cell = &mut element(&mut presentation, 1, "ir-s2.grid")["rows"][1]["cells"][1];
    let angled = json!({ "kind": "linear", "angle": 90, "stops": cell["fill"]["stops"].clone() });
    cell["fill"] = angled.clone();
    assert!(verify_resolved(&presentation).is_ok());
    assert_eq!(
        check_invariant(
            Invariant::GeometryLowered,
            &typed(&presentation),
            &CheckOptions::default()
        ),
        vec![]
    );
    element(&mut presentation, 1, "ir-s2.note")["fill"] = angled;
    let diagnostics = verify_resolved(&presentation).map(|_| ()).unwrap_err();
    let set: BTreeSet<String> = located(&diagnostics).into_iter().collect();
    assert_eq!(set, BTreeSet::from(["IR0001@ir-s2".to_owned()]));
}

#[test]
fn exempts_a_failed_container_from_the_resolved_only_rules() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("resolved/full.presentation-ir.json");
    let source = fixture("source/full.presentation-ir.json");
    let mut slide = source["slides"][0].clone();
    slide["elements"] = json!([]);
    slide["failed"] = json!({ "code": "IR0908", "pass": "cascade-text" });
    slide["timeline"] = json!([]);
    presentation["slides"][0] = slide;
    let verified = verify_resolved(&presentation).unwrap_or_else(|d| panic!("{:?}", located(&d)));
    assert_eq!(
        verified.slides[0].body.failed().map(|f| f.code.as_str()),
        Some("IR0908")
    );
    let printed: Value = serde_json::to_value(&verified).unwrap();
    assert_eq!(printed["slides"][0]["failed"]["code"], "IR0908");
    assert_eq!(printed["slides"][0]["elements"], json!([]));
}

#[test]
fn leaves_to_the_resolved_schema_what_no_checker_owns() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("resolved/full.presentation-ir.json");
    presentation["slides"][1]["failed"] = json!({ "code": "IR0908" });
    let diagnostics = verify_resolved(&presentation).map(|_| ()).unwrap_err();
    assert_eq!(located(&diagnostics), ["IR0001@ir-s2"]);
}

#[test]
fn runs_every_checker_on_a_resolved_fixture_without_findings() {
    if golden().is_none() {
        return;
    }
    let presentation = typed(&fixture("resolved/full.presentation-ir.json"));
    for invariant in Invariant::ALL {
        assert_eq!(
            check_invariant(invariant, &presentation, &CheckOptions::default()),
            vec![],
            "{invariant}"
        );
    }
}

#[test]
fn restricts_a_checker_to_the_given_containers() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("resolved/full.presentation-ir.json");
    remove(element(&mut presentation, 0, "ir-s1.photo"), "frame");
    remove(element(&mut presentation, 1, "ir-s2.note"), "frame");
    let options = CheckOptions {
        containers: Some(BTreeSet::from([NodeId::new("ir-s2").unwrap()])),
        ..CheckOptions::default()
    };
    let only = check_invariant(
        Invariant::InheritanceResolved,
        &typed(&presentation),
        &options,
    );
    assert_eq!(located(&only), ["IR0108@ir-s2"]);
}

#[test]
fn verify_invariants_includes_the_graph_rules() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("resolved/full.presentation-ir.json");
    presentation["slides"][0]["layout"] = json!("ir-gone");
    remove(&mut presentation["slides"][0]["elements"][3], "frame");
    let codes: Vec<String> = verify_invariants(
        &typed(&presentation),
        &[Invariant::InheritanceResolved],
        &CheckOptions::default(),
    )
    .iter()
    .map(|d| d.code.to_string())
    .collect();
    assert_eq!(codes, ["IR0003", "IR0108"]);
}

#[test]
fn parse_presentation_returns_the_typed_presentation_or_schema_problems() {
    if golden().is_none() {
        return;
    }
    let presentation = typed(&fixture("source/minimal.presentation-ir.json"));
    assert_eq!(presentation.slides[0].id.as_str(), "ir-s1");
    let mut broken = fixture("source/minimal.presentation-ir.json");
    broken["slides"] = json!("none");
    assert_eq!(
        located(&parse_presentation_json(&broken).unwrap_err()),
        ["IR0001@presentation"]
    );
    assert!(parse_presentation_json(&json!("not a presentation")).is_err());
}

#[test]
fn resolves_slide_links_against_the_linkable_slides() {
    if golden().is_none() {
        return;
    }
    let alone = |name: &str| {
        let mut presentation = fixture(name);
        presentation["slides"]
            .as_array_mut()
            .unwrap()
            .retain(|slide| slide["id"] == "ir-s2");
        presentation
    };
    let kinds = |diagnostics: &[Diagnostic]| -> Vec<String> {
        diagnostics
            .iter()
            .filter(|d| d.code == codes::IR0003)
            .map(|d| data(d, "kind"))
            .collect()
    };
    assert_eq!(
        kinds(&verify_source(&alone("source/full.presentation-ir.json"))),
        ["slide-link"]
    );
    assert_eq!(
        kinds(&verify_resolved(&alone("resolved/full.presentation-ir.json")).unwrap_err()),
        ["slide-link"]
    );
    let options = VerifyOptions {
        linkable_slides: vec![NodeId::new("ir-s1").unwrap(), NodeId::new("ir-s2").unwrap()],
    };
    assert_eq!(
        verify_source_json(&alone("source/full.presentation-ir.json"), &options),
        vec![]
    );
    assert!(verify_resolved_json(&alone("resolved/full.presentation-ir.json"), &options).is_ok());
}

#[test]
fn presentation_schemas() {
    if golden().is_none() {
        return;
    }
    for name in SOURCE_FIXTURES {
        let mut presentation = fixture(name);
        assert!(parse_presentation_json(&presentation).is_ok());
        presentation["phase"] = json!("resolved");
        assert!(
            slidock_ir::parse::from_value::<slidock_ir::resolved::Presentation>(presentation)
                .is_err(),
            "{name}"
        );
    }
    for name in RESOLVED_FIXTURES {
        let presentation = fixture(name);
        assert!(
            slidock_ir::parse::from_value::<slidock_ir::resolved::Presentation>(
                presentation.clone()
            )
            .is_ok()
        );
        assert!(parse_presentation_json(&presentation).is_ok());
    }
    let mut unknown = fixture("source/full.presentation-ir.json");
    unknown["slides"][0]["elements"][0]["css"] = json!("color: red");
    assert!(parse_presentation_json(&unknown).is_err());
    let mut hash = fixture("source/full.presentation-ir.json");
    hash["slides"][0]["elements"][4]["children"][1]["children"][0]["media"] = json!("not-a-hash");
    assert!(parse_presentation_json(&hash).is_err());
    let mut identity = fixture("source/minimal.presentation-ir.json");
    identity["source"] = json!({ "format": "pptx" });
    assert!(parse_presentation_json(&identity).is_err());

    let mut failed = fixture("resolved/full.presentation-ir.json");
    let source = fixture("source/full.presentation-ir.json");
    let mut slide = source["slides"][0].clone();
    slide["elements"] = json!([]);
    slide["failed"] = json!({ "code": "IR0908", "pass": "cascade-text" });
    failed["slides"][0] = slide;
    assert!(
        slidock_ir::parse::from_value::<slidock_ir::resolved::Presentation>(failed.clone()).is_ok()
    );
    failed["slides"][0]["elements"] = source["slides"][0]["elements"].clone();
    assert!(slidock_ir::parse::from_value::<slidock_ir::resolved::Presentation>(failed).is_err());
}

#[test]
fn foreign_fallbacks_validate_nested_elements() {
    if golden().is_none() {
        return;
    }
    let foreign = json!({
        "id": "ir-f", "origin": agent(), "kind": "foreign", "dialect": "keynote", "type": "chart",
        "fallback": { "elements": [{ "id": "ir-f.1", "origin": agent(), "kind": "group", "children": [] }] }
    });
    assert!(slidock_ir::parse::from_value::<source::Element>(foreign.clone()).is_ok());
    let mut ir = foreign.clone();
    ir["dialect"] = json!("ir");
    assert!(slidock_ir::parse::from_value::<source::Element>(ir).is_err());
    let mut table = foreign;
    table["fallback"]["elements"][0]["kind"] = json!("table");
    assert!(slidock_ir::parse::from_value::<source::Element>(table).is_err());
}

#[test]
fn narrowing_guards() {
    if golden().is_none() {
        return;
    }
    use slidock_ir::verify::*;
    let resolved = typed(&fixture("resolved/full.presentation-ir.json"));
    let source = typed(&fixture("source/full.presentation-ir.json"));
    let source::Element::Shape(body) = &resolved.slides[0].elements[1] else {
        panic!("fixture")
    };
    let text = body.text.as_ref().unwrap();
    let paragraph = &text.paragraphs[0];
    let node = &body.id;
    let rules = |error: InvariantError| (error.invariant, error.rule);

    assert!(framed(&resolved.slides[0].elements[1]).is_ok());
    let error = framed(&source.slides[0].elements[0]).unwrap_err();
    assert_eq!(error.node.as_ref().map(NodeId::as_str), Some("ir-s1.title"));
    assert_eq!(
        rules(error),
        (Invariant::InheritanceResolved, codes::IR0108)
    );

    assert!(complete_body(&text.body, node).is_ok());
    let partial = slidock_ir::text::PartialBodyProps {
        anchor: Some(slidock_ir::text::Anchor::Top),
        ..Default::default()
    };
    assert_eq!(
        rules(complete_body(&partial, node).unwrap_err()),
        (Invariant::InheritanceResolved, codes::IR0115)
    );

    let red: slidock_ir::color::Color = serde_json::from_value(
        json!({ "kind": "rgb", "space": "srgb", "r": 1, "g": 0, "b": 0, "a": 1 }),
    )
    .unwrap();
    let theme: slidock_ir::color::Color =
        serde_json::from_value(json!({ "kind": "theme", "slot": "tx1", "mods": [] })).unwrap();
    assert!(resolved_color(&red, None).is_ok());
    assert_eq!(
        rules(resolved_color(&theme, Some(node)).unwrap_err()),
        (Invariant::ThemeResolved, codes::IR0101)
    );

    let family: slidock_ir::text::FontRef =
        serde_json::from_value(json!({ "kind": "family", "family": "Liberation Sans" })).unwrap();
    let minor: slidock_ir::text::FontRef =
        serde_json::from_value(json!({ "kind": "theme", "slot": "minor" })).unwrap();
    assert!(family_font(&family, Some(node)).is_ok());
    assert_eq!(
        rules(family_font(&minor, Some(node)).unwrap_err()),
        (Invariant::ThemeResolved, codes::IR0114)
    );

    let source_paragraph: slidock_ir::text::SourceParagraph =
        serde_json::from_value(serde_json::to_value(paragraph).unwrap()).unwrap();
    assert!(cascaded_para(&source_paragraph.props, node).is_ok());
    let align_only = slidock_ir::text::PartialParaProps {
        align: Some(slidock_ir::text::Align::Left),
        ..Default::default()
    };
    assert_eq!(
        rules(cascaded_para(&align_only, node).unwrap_err()),
        (Invariant::TextCascaded, codes::IR0102)
    );
    assert!(cascaded_run(source_paragraph.inlines[0].props(), node).is_ok());
    assert_eq!(
        rules(cascaded_run(None, node).unwrap_err()),
        (Invariant::TextCascaded, codes::IR0103)
    );

    assert!(labelled_bullet(source_paragraph.props.bullet.as_ref(), node).is_ok());
    let auto: slidock_ir::text::Bullet =
        serde_json::from_value(json!({ "kind": "auto", "scheme": "arabicPeriod", "startAt": 1 }))
            .unwrap();
    assert_eq!(
        rules(labelled_bullet(Some(&auto), node).unwrap_err()),
        (Invariant::ListsLabelled, codes::IR0111)
    );

    assert!(binding(&resolved.fonts, "Liberation Sans", node).is_ok());
    assert_eq!(
        rules(binding(&resolved.fonts, "Helvetica Neue", node).unwrap_err()),
        (Invariant::FontsBound, codes::IR0104)
    );

    assert!(line_metrics(&source_paragraph.props, node).is_ok());
    assert_eq!(
        rules(line_metrics(&align_only, node).unwrap_err()),
        (Invariant::TextMetrics, codes::IR0110)
    );

    assert!(fitted_autofit(text.body.autofit.as_ref().unwrap(), node).is_ok());
    let shrink: slidock_ir::text::Autofit =
        serde_json::from_value(json!({ "kind": "shrink", "fontScale": 0.9 })).unwrap();
    assert_eq!(
        rules(fitted_autofit(&shrink, node).unwrap_err()),
        (Invariant::TextMetrics, codes::IR0107)
    );

    assert!(lowered_geometry(body.geometry.as_ref(), node).is_ok());
    let preset: slidock_ir::geometry::Geometry =
        serde_json::from_value(json!({ "kind": "preset", "name": "rect", "adjust": {} })).unwrap();
    assert_eq!(
        rules(lowered_geometry(Some(&preset), node).unwrap_err()),
        (Invariant::GeometryLowered, codes::IR0105)
    );

    let fill_of = |p: &source::Presentation| match &p.slides[0].elements[2] {
        source::Element::Shape(shape) => shape.fill.clone().unwrap(),
        _ => panic!("fixture"),
    };
    assert!(lowered_fill(&fill_of(&resolved), Some(node)).is_ok());
    assert_eq!(
        rules(lowered_fill(&fill_of(&source), Some(node)).unwrap_err()),
        (Invariant::GeometryLowered, codes::IR0113)
    );

    assert!(text_rotation(text, node).is_ok());
    let bare: slidock_ir::text::TextBody =
        serde_json::from_value(json!({ "body": {}, "paragraphs": [] })).unwrap();
    assert_eq!(
        rules(text_rotation(&bare, node).unwrap_err()),
        (Invariant::GeometryLowered, codes::IR0113)
    );

    assert!(media_plan(resolved.media.values().next().unwrap()).is_ok());
    assert_eq!(
        rules(media_plan(source.media.values().next().unwrap()).unwrap_err()),
        (Invariant::MediaPlanned, codes::IR0106)
    );
}

#[test]
fn censuses_what_each_container_renders() {
    if golden().is_none() {
        return;
    }
    let presentation = typed(&fixture("source/full.presentation-ir.json"));
    let census = census_presentation(&presentation);
    let keys: Vec<&str> = census.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["presentation", "ir-master", "ir-layout", "ir-s1", "ir-s2"]
    );
    let names = |features: &[FeatureId]| features.iter().map(|f| f.as_str()).collect::<Vec<_>>();
    let mut s1 = vec![
        "element.alt",
        "element.group",
        "element.picture",
        "element.shape",
        "element.table",
        "element.video",
        "fill.linear",
        "fill.radial",
        "frame.flip",
        "frame.rotation",
        "geometry.custom",
        "geometry.preset",
        "image.clip",
        "image.crop",
        "image.opacity",
        "placeholder",
        "slide.notes",
        "slide.timeline",
        "slide.transition",
        "stroke",
        "stroke.dash",
        "stroke.line-end",
        "text.bullet.auto",
        "text.field",
        "text.italic",
        "text.line-spacing",
        "text.link",
        "text.run",
        "text.tab",
        "text.baseline",
        "text.break",
        "text.underline",
        "text.weight",
    ];
    s1.sort_unstable();
    assert_eq!(names(&census["ir-s1"]), s1);
    let mut s2 = vec![
        "presentation.hidden-slide",
        "element.link",
        "element.shape",
        "element.table",
        "fill.linear",
        "fill.solid",
        "geometry.preset",
        "slide.background",
        "stroke",
        "stroke.dash",
        "text.autofit.shrink",
        "text.run",
        "text.weight",
    ];
    s2.sort_unstable();
    assert_eq!(names(&census["ir-s2"]), s2);
    assert!(census["ir-master"].contains(&FeatureId::LayoutElements));
    assert!(census["presentation"].is_empty());
}

#[test]
fn census_loses_nothing_between_the_source_and_resolved_fixtures() {
    if golden().is_none() {
        return;
    }
    let source = census_presentation(&typed(&fixture("source/full.presentation-ir.json")));
    let resolved = census_presentation(&typed(&fixture("resolved/full.presentation-ir.json")));
    for (container, features) in &source {
        let lost: Vec<_> = features
            .iter()
            .filter(|f| !resolved[container].contains(f))
            .collect();
        assert!(lost.is_empty(), "{container}: {lost:?}");
    }
}

#[test]
fn census_reads_media_facts_and_presentation_level_fonts() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("source/full.presentation-ir.json");
    let logo = presentation["masters"]["ir-master"]["elements"][2]["media"]
        .as_str()
        .unwrap()
        .to_owned();
    presentation["media"][&logo]["type"] = json!("image/gif");
    presentation["media"][&logo]["frames"] = json!(12);
    let p = typed(&presentation);
    assert!(
        census_container(Container::Master(&p.masters[0]), &p).contains(&FeatureId::ImageAnimated)
    );
    presentation["media"][&logo]["type"] = json!("image/svg+xml");
    remove(&mut presentation["media"][&logo], "frames");
    let p = typed(&presentation);
    assert!(
        census_container(Container::Master(&p.masters[0]), &p).contains(&FeatureId::ImageVector)
    );
    presentation["fonts"]["carlito"] = json!({ "request": { "family": "Carlito", "faces": [], "postscriptNames": [], "embedded": true } });
    assert_eq!(
        census_presentation(&typed(&presentation))["presentation"],
        [FeatureId::FontEmbedded]
    );
}

#[test]
fn census_tells_connectors_and_foreign_types_apart() {
    if golden().is_none() {
        return;
    }
    let mut presentation = fixture("source/minimal.presentation-ir.json");
    presentation["slides"][0]["elements"] = json!([
        { "id": "ir-c", "origin": agent(), "kind": "shape", "geometry": { "kind": "preset", "name": "bentConnector3", "adjust": {} } },
        { "id": "ir-f", "origin": agent(), "kind": "foreign", "dialect": "keynote", "type": "ink" },
        { "id": "ir-a", "origin": agent(), "kind": "media", "mediaKind": "audio", "loop": false, "muted": false, "autoplay": false, "hidden": true }
    ]);
    let p = typed(&presentation);
    let names: Vec<&str> = census_container(Container::Slide(&p.slides[0]), &p)
        .iter()
        .map(|f| f.as_str())
        .collect();
    assert_eq!(
        names,
        [
            "element.audio",
            "element.hidden",
            "element.other",
            "element.shape",
            "geometry.connector",
            "slide.background"
        ]
    );
}

/// The recorded resolved presentations: every IR fixture and corpus pair as `slidock compile` wrote it.
fn recorded_presentations(golden: &Golden) -> Vec<(PathBuf, Value)> {
    let mut outputs: Vec<String> = golden
        .ir_fixtures()
        .iter()
        .map(|name| format!("fixtures/{name}/presentation-ir.json"))
        .collect();
    outputs.extend(
        golden
            .pairs()
            .iter()
            .map(|pair| format!("corpus/{}/presentation-ir.json", pair.name)),
    );
    outputs
        .into_iter()
        .map(|output| {
            let value: Value = serde_json::from_slice(&golden.output(&output)).unwrap();
            (golden.root().join(output), value)
        })
        .collect()
}

/// The private fixtures of a phase (`ir` for the IR fixtures), by path.
fn fixture_presentations(golden: &Golden, dir: &str) -> Vec<(PathBuf, Value)> {
    let out: Vec<(PathBuf, Value)> = golden
        .fixture_names(dir)
        .iter()
        .map(|name| {
            let path = golden.fixture(&format!("{dir}/{name}"));
            let value: Value =
                serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            (path, value)
        })
        .collect();
    assert!(!out.is_empty(), "{dir}");
    out
}

/// What the verifier finds on the golden presentations: nothing on an IR fixture or a source fixture, the phase only on
/// a resolved presentation read as a source one, and nothing when re-verifying a resolved one. The agent fixture is raw
/// agent IR: its self-inheriting `ir-a1.claim` is a cycle until the IR frontend strips inheritance edges.
#[test]
fn the_verifier_finds_on_every_golden_presentation_what_its_phase_allows() {
    let Some(golden) = golden() else {
        return;
    };
    let mut count = 0;
    for dir in ["ir", "presentation/source"] {
        for (path, presentation) in fixture_presentations(golden, dir) {
            let expected: Vec<&str> = if path.ends_with("agent.presentation-ir.json") {
                vec!["IR0004@ir-a1"]
            } else {
                vec![]
            };
            assert_eq!(
                located(&verify_source(&presentation)),
                expected,
                "{}",
                path.display()
            );
            count += 1;
        }
    }
    let mut resolved = fixture_presentations(golden, "presentation/resolved");
    resolved.extend(recorded_presentations(golden));
    for (path, presentation) in resolved {
        assert_eq!(
            located(&verify_source(&presentation)),
            ["IR0009@presentation"],
            "{}",
            path.display()
        );
        if let Err(d) = verify_resolved(&presentation) {
            panic!("{}: {:?}", path.display(), located(&d));
        }
        count += 1;
    }
    eprintln!("presentations verified: {count}");
    assert_eq!(
        count,
        golden.ir_fixtures().len() + 4 + recorded_presentations(golden).len()
    );
}

#[test]
fn resolved_presentations_widen_to_source() {
    if golden().is_none() {
        return;
    }
    let presentation = fixture("resolved/full.presentation-ir.json");
    let resolved = verify::verify_resolved_json(&presentation, &VerifyOptions::default()).unwrap();
    let widened = resolved.to_source().unwrap();
    assert_eq!(
        slidock_ir::canonical::to_json(&widened).unwrap(),
        slidock_ir::canonical::to_json(&resolved).unwrap()
    );
    let narrowed = slidock_ir::resolved::Presentation::try_from(widened).unwrap();
    assert_eq!(narrowed, resolved);
}
