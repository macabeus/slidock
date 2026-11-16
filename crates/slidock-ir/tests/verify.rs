//! The verifier's rules on a small synthetic presentation: each graph rule and invariant fires on its own
//! violation, at its container, and nowhere else.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::collections::BTreeSet;

use serde_json::{Value, json};
use slidock_ir::canonical::to_json;
use slidock_ir::diagnostics::{Diagnostic, DiagnosticCode, codes};
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::source::Container;
use slidock_ir::verify::{
    CheckOptions, Invariant, VerifyOptions, check_invariant, parse_presentation_json,
    verify_resolved_json, verify_source_json,
};
use slidock_ir::{parse, resolved};

fn agent() -> Value {
    json!({ "kind": "agent" })
}

fn rgb(r: f64, g: f64, b: f64) -> Value {
    json!({ "kind": "rgb", "space": "srgb", "r": r, "g": g, "b": b, "a": 1 })
}

fn photo_sha() -> String {
    Sha256::of(b"photo").to_hex()
}

fn run_props(script_scale: bool) -> Value {
    let mut props = json!({
        "font": { "kind": "family", "family": "Lato" }, "size": 18, "weight": 400, "italic": false,
        "underline": "none", "strike": "none", "color": rgb(0.0, 0.0, 0.0), "highlight": null, "baseline": 0,
        "caps": "none", "letterSpacing": 0, "link": null, "synthesizeBold": true
    });
    if script_scale {
        props["scriptScale"] = json!(1);
    }
    props
}

fn para_props(metrics: bool) -> Value {
    let mut props = json!({
        "align": "left", "marginLeft": 0, "marginRight": 0, "indent": 0,
        "spaceBefore": { "kind": "pt", "value": 0 }, "spaceAfter": { "kind": "pt", "value": 0 },
        "lineSpacing": { "kind": "multiple", "value": 1, "basis": "size-1.2" }, "tabSize": 72,
        "bullet": { "kind": "none" }, "rtl": false
    });
    if metrics {
        props["lineHeight"] = json!({ "kind": "ratio", "value": 1.2 });
        props["spaceBeforePt"] = json!(0);
        props["spaceAfterPt"] = json!(0);
    }
    props
}

fn body() -> Value {
    json!({ "insets": { "l": 7.2, "t": 3.6, "r": 7.2, "b": 3.6 }, "anchor": "top", "wrap": "square",
            "autofit": { "kind": "none" }, "overflow": "spread" })
}

fn frame(x: f64, w: f64, h: f64) -> Value {
    json!({ "x": x, "y": 0, "w": w, "h": h, "rotation": 0, "flipH": false, "flipV": false })
}

/// A text body: sparse in the source phase, complete when resolved.
fn text(resolved: bool, words: &str) -> Value {
    if resolved {
        json!({ "body": body(), "rotation": 0, "paragraphs": [{ "level": 0, "props": para_props(true),
            "inlines": [{ "kind": "run", "text": words, "props": run_props(false) }] }] })
    } else {
        json!({ "body": {}, "paragraphs": [{ "level": 0, "props": {},
            "inlines": [{ "kind": "run", "text": words, "props": {} }] }] })
    }
}

fn paths() -> Value {
    json!({ "kind": "paths", "paths": [{ "segments": [["M", 0, 0], ["L", 10, 0], ["Z"]], "fill": true, "stroke": false, "fillRule": "nonzero" }] })
}

fn cell(resolved: bool, extra: Value) -> Value {
    let mut cell = json!({ "text": text(resolved, "") });
    for (key, value) in extra.as_object().unwrap() {
        cell[key] = value.clone();
    }
    cell
}

/// A presentation of our own: a master, a layout, two slides; a title inheriting from the layout, a picture, a
/// spanning 2×2 table, a group, a timeline step and a slide link.
fn presentation(resolved: bool) -> Value {
    let levels: serde_json::Map<String, Value> = (0..9)
        .map(|i| {
            (
                i.to_string(),
                json!({ "para": para_props(false), "run": run_props(true) }),
            )
        })
        .collect();
    let colors: serde_json::Map<String, Value> = [
        "dk1", "lt1", "dk2", "lt2", "accent1", "accent2", "accent3", "accent4", "accent5",
        "accent6", "hlink", "folHlink",
    ]
    .iter()
    .map(|slot| ((*slot).to_owned(), rgb(0.5, 0.5, 0.5)))
    .collect();
    let geometry = if resolved {
        paths()
    } else {
        json!({ "kind": "preset", "name": "rect", "adjust": {} })
    };
    let maybe_frame = |x: f64| {
        if resolved {
            frame(x, 100.0, 50.0)
        } else {
            Value::Null
        }
    };
    let mut title = json!({ "id": "ir-s1.title", "origin": agent(), "kind": "shape", "placeholder": { "role": "title" },
        "inherits": "ir-layout.title", "geometry": geometry, "text": text(resolved, "Hello") });
    let mut photo = json!({ "id": "ir-s1.photo", "origin": agent(), "kind": "picture", "media": photo_sha(),
        "frame": frame(0.0, 10.0, 10.0) });
    let mut grid = json!({ "id": "ir-s1.grid", "origin": agent(), "kind": "table", "frame": frame(0.0, 100.0, 40.0),
        "columns": [{ "width": 60 }, { "width": 40 }],
        "rows": [
            { "height": 20, "cells": [cell(resolved, json!({ "colSpan": 2 })), null] },
            { "height": 20, "cells": [cell(resolved, json!({})), cell(resolved, json!({}))] }
        ] });
    let group = json!({ "id": "ir-s1.group", "origin": agent(), "kind": "group", "frame": frame(0.0, 10.0, 10.0),
        "children": [{ "id": "ir-s1.group.dot", "origin": agent(), "kind": "shape", "geometry": paths(),
            "frame": frame(0.0, 1.0, 1.0) }] });
    let mut layout_title = json!({ "id": "ir-layout.title", "origin": agent(), "kind": "shape",
        "placeholder": { "role": "title" }, "geometry": geometry, "frame": frame(10.0, 200.0, 40.0) });
    let mut note = json!({ "id": "ir-s2.note", "origin": agent(), "kind": "shape", "geometry": geometry,
        "text": text(resolved, "See the first slide") });
    note["text"]["paragraphs"][0]["inlines"][0]["props"]["link"] =
        json!({ "kind": "slide", "slide": "ir-s1" });
    if resolved {
        title["frame"] = maybe_frame(10.0);
        note["frame"] = maybe_frame(20.0);
        photo["frame"] = frame(0.0, 10.0, 10.0);
    } else {
        layout_title.as_object_mut().unwrap().shift_remove("frame");
        layout_title["frame"] = frame(10.0, 200.0, 40.0);
        grid["frame"] = frame(0.0, 100.0, 40.0);
    }
    let mut slide1 = json!({ "id": "ir-s1", "origin": agent(), "layout": "ir-layout", "hidden": false,
        "showLayout": true, "elements": [title, photo, grid, group],
        "timeline": [{ "target": { "element": "ir-s1.title" }, "trigger": "click", "delayMs": 0, "durationMs": 500,
            "kind": "entrance", "effect": { "id": "ir:fade", "fallback": "dissolve" }, "delivery": "all" }] });
    let mut slide2 = json!({ "id": "ir-s2", "origin": agent(), "layout": "ir-layout", "hidden": true,
        "showLayout": true, "elements": [note], "timeline": [] });
    let mut fonts = json!({ "lato": { "request": { "family": "Lato", "faces": [], "postscriptNames": [], "embedded": false } } });
    let mut media = json!({ photo_sha(): { "sha256": photo_sha(), "type": "image/png", "bytes": 10,
        "origin": { "kind": "source", "part": "media/photo.png" } } });
    if resolved {
        for (slide, title) in [(&mut slide1, "Hello"), (&mut slide2, "See the first slide")] {
            slide["title"] = json!(title);
            let order: Vec<Value> = slide["elements"]
                .as_array()
                .unwrap()
                .iter()
                .map(|e| e["id"].clone())
                .collect();
            slide["readingOrder"] = json!(order);
        }
        let metrics = json!({ "unitsPerEm": 1000, "hhea": { "ascender": 900, "descender": -200, "lineGap": 0 },
            "typo": { "ascender": 900, "descender": -200, "lineGap": 0 }, "win": { "ascent": 900, "descent": 200 } });
        fonts["lato"]["binding"] = json!({ "kind": "installed", "family": "Lato", "generic": "sans-serif", "metrics": metrics });
        media[photo_sha()]["plan"] = json!({ "action": "copy" });
    }
    json!({
        "size": { "w": 960, "h": 540 },
        "defaults": { "text": levels, "body": body() },
        "themes": { "ir-theme": { "id": "ir-theme", "origin": agent(), "colors": colors,
            "fonts": { "major": { "latin": "Lato" }, "minor": { "latin": "Lato" } } } },
        "phase": if resolved { "resolved" } else { "source" },
        "masters": { "ir-master": { "id": "ir-master", "origin": agent(), "theme": "ir-theme", "elements": [] } },
        "layouts": { "ir-layout": { "id": "ir-layout", "origin": agent(), "master": "ir-master", "showMaster": true,
            "elements": [layout_title] } },
        "slides": [slide1, slide2],
        "fonts": fonts,
        "media": media
    })
}

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

fn element(p: &mut Value, slide: usize, index: usize) -> &mut Value {
    &mut p["slides"][slide]["elements"][index]
}

fn remove(value: &mut Value, key: &str) {
    value.as_object_mut().unwrap().shift_remove(key);
}

fn source(p: &Value) -> Vec<String> {
    located(&verify_source_json(p, &VerifyOptions::default()))
}

#[test]
fn the_synthetic_presentation_is_valid_in_both_phases() {
    assert_eq!(source(&presentation(false)), Vec::<String>::new());
    assert_eq!(source(&presentation(true)), ["IR0009@presentation"]);
    let resolved = verify_resolved_json(&presentation(true), &VerifyOptions::default());
    assert!(
        resolved.is_ok(),
        "{:?}",
        resolved.err().map(|d| located(&d))
    );
    let refused =
        verify_resolved_json(&presentation(false), &VerifyOptions::default()).unwrap_err();
    assert!(!refused.is_empty());
}

/// Each phase's presentation read in the other's types one container at a time is what reading it whole gives.
#[test]
fn reads_a_presentation_in_the_other_phase_as_if_whole() {
    let resolved = verify_resolved_json(&presentation(true), &VerifyOptions::default()).unwrap();
    let whole: slidock_ir::source::Presentation =
        parse::from_value(serde_json::to_value(&resolved).unwrap()).unwrap();
    let source = resolved.to_source().unwrap();
    assert_eq!(source, whole);
    assert!(!source.slides.is_empty() && !source.layouts.is_empty() && !source.masters.is_empty());
    let back = resolved::Presentation::from_source(&source).unwrap();
    assert_eq!(back, resolved);
    // A container that does not read in the resolved types fails with its path from the root, as reading the whole
    // presentation does.
    let mut unresolved = slidock_ir::verify::parse_presentation_json(&presentation(false)).unwrap();
    unresolved.phase = slidock_ir::presentation::Phase::Resolved;
    let error = resolved::Presentation::from_source(&unresolved).unwrap_err();
    let whole =
        parse::from_value::<resolved::Presentation>(serde_json::to_value(&unresolved).unwrap())
            .unwrap_err();
    assert_eq!(error, whole);
    let path = error.path.to_string();
    assert!(
        ["masters", "layouts", "slides"]
            .iter()
            .any(|c| path.starts_with(c)),
        "{error}"
    );
}

type Mutation = fn(&mut Value);

#[test]
fn graph_rules_fire_on_their_violation_only() {
    let cases: Vec<(&str, Mutation, Vec<&str>)> = vec![
        (
            "IR0001 structure",
            |p| element(p, 0, 0)["kind"] = json!("blob"),
            vec!["IR0001@ir-s1"],
        ),
        (
            "IR0001 theme structure",
            |p| p["themes"]["ir-theme"]["colors"]["dk1"]["kind"] = json!("cmyk"),
            vec!["IR0001@ir-master"],
        ),
        (
            "IR0001 map key",
            |p| {
                let mut copy = p["layouts"]["ir-layout"].clone();
                copy["elements"] = json!([]);
                p["layouts"]["ir-copy"] = copy;
            },
            vec!["IR0001@ir-copy", "IR0002@presentation"],
        ),
        (
            "IR0001 font key",
            |p| {
                let entry = p["fonts"]["lato"].clone();
                p["fonts"]["lato2"] = entry;
            },
            vec!["IR0001@presentation"],
        ),
        (
            "IR0002 within a container",
            |p| element(p, 0, 1)["id"] = json!("ir-s1.title"),
            vec!["IR0002@ir-s1"],
        ),
        (
            "IR0002 across containers",
            |p| element(p, 1, 0)["id"] = json!("ir-s1.title"),
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
            |p| element(p, 0, 0)["inherits"] = json!("ir-gone"),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 placeholder target",
            |p| element(p, 0, 0)["placeholder"]["target"] = json!("ir-gone"),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 timeline target",
            |p| p["slides"][0]["timeline"][0]["target"]["element"] = json!("ir-s2.note"),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 slide link",
            |p| {
                element(p, 1, 0)["text"]["paragraphs"][0]["inlines"][0]["props"]["link"]["slide"] =
                    json!("ir-s9");
            },
            vec!["IR0003@ir-s2"],
        ),
        (
            "IR0003 media",
            |p| element(p, 0, 1)["media"] = json!(Sha256::of(b"gone").to_hex()),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0004 cycle",
            |p| {
                p["layouts"]["ir-layout"]["elements"][0]["inherits"] = json!("ir-s1.title");
            },
            vec!["IR0004@ir-layout"],
        ),
        (
            "IR0005 group depth",
            |p| {
                let mut group =
                    json!({ "id": "ir-d16", "origin": agent(), "kind": "group", "children": [] });
                for depth in (0..=15).rev() {
                    group = json!({ "id": format!("ir-d{depth}"), "origin": agent(), "kind": "group", "children": [group] });
                }
                p["slides"][1]["elements"]
                    .as_array_mut()
                    .unwrap()
                    .push(group);
            },
            vec!["IR0005@ir-s2"],
        ),
        (
            "IR0005 group depth, once beyond it",
            |p| {
                let mut group =
                    json!({ "id": "ir-d17", "origin": agent(), "kind": "group", "children": [] });
                for depth in (0..=16).rev() {
                    group = json!({ "id": format!("ir-d{depth}"), "origin": agent(), "kind": "group", "children": [group] });
                }
                p["slides"][1]["elements"]
                    .as_array_mut()
                    .unwrap()
                    .push(group);
            },
            vec!["IR0005@ir-s2"],
        ),
        (
            "IR0005 inherits chain",
            |p| {
                for i in 0..10 {
                    let mut e =
                        json!({ "id": format!("ir-c{i}"), "origin": agent(), "kind": "shape" });
                    if i > 0 {
                        e["inherits"] = json!(format!("ir-c{}", i - 1));
                    }
                    p["slides"][1]["elements"].as_array_mut().unwrap().push(e);
                }
            },
            vec!["IR0005@ir-s2"],
        ),
        (
            "IR0006 slides",
            |p| {
                for i in 0..299 {
                    p["slides"]
                        .as_array_mut()
                        .unwrap()
                        .push(json!({ "id": format!("ir-x{i}"), "origin": agent(),
                    "hidden": false, "showLayout": true, "elements": [], "timeline": [] }));
                }
            },
            vec!["IR0006@presentation"],
        ),
        (
            "IR0007 grid",
            |p| element(p, 0, 1)["frame"]["x"] = json!(1.0005),
            vec!["IR0007@ir-s1"],
        ),
        (
            "IR0008 string",
            |p| element(p, 0, 1)["name"] = json!("x".repeat(257)),
            vec!["IR0008@ir-s1"],
        ),
        (
            "IR0008 range",
            |p| element(p, 0, 1)["frame"]["x"] = json!(100_001),
            vec!["IR0008@ir-s1"],
        ),
        (
            "IR0008 ext",
            |p| element(p, 0, 1)["ext"] = json!({ "odp": { "blob": "x".repeat(17_000) } }),
            vec!["IR0008@ir-s1"],
        ),
        (
            "IR0009 phase",
            |p| p["phase"] = json!("resolved"),
            vec!["IR0009@presentation"],
        ),
        (
            "IR0011 origin",
            |p| element(p, 0, 1)["origin"] = json!({ "kind": "source", "part": "C:\\deck.pptx" }),
            vec!["IR0011@ir-s1"],
        ),
        (
            "IR0014 span past the grid",
            |p| element(p, 0, 2)["rows"][1]["cells"][1]["rowSpan"] = json!(2),
            vec!["IR0014@ir-s1"],
        ),
        (
            "IR0014 uncovered slot",
            |p| remove(&mut element(p, 0, 2)["rows"][0]["cells"][0], "colSpan"),
            vec!["IR0014@ir-s1"],
        ),
        (
            "IR0015 frame",
            |p| element(p, 0, 2)["frame"]["w"] = json!(99),
            vec!["IR0015@ir-s1"],
        ),
    ];
    for (rule, mutate, expected) in cases {
        let mut p = presentation(false);
        mutate(&mut p);
        assert_eq!(source(&p), expected, "{rule}");
    }
}

#[test]
fn bound_and_grid_issues_name_their_node_and_path() {
    let mut p = presentation(false);
    element(&mut p, 0, 3)["children"][0]["frame"]["y"] = json!(0.0001);
    let d = &verify_source_json(&p, &VerifyOptions::default())[0];
    assert_eq!(d.code, codes::IR0007);
    assert_eq!(d.node.as_ref().map(NodeId::as_str), Some("ir-s1.group.dot"));
    assert_eq!(
        d.data.as_ref().unwrap()["path"].to_js_string(),
        "slides[0].elements[3].children[0].frame.y"
    );

    let mut p = presentation(false);
    element(&mut p, 0, 3)["children"][0]["frame"]["w"] = json!(-1);
    let d = &verify_source_json(&p, &VerifyOptions::default())[0];
    assert_eq!(d.code, codes::IR0008);
    assert_eq!(d.node.as_ref().map(NodeId::as_str), Some("ir-s1.group.dot"));
    assert_eq!(
        d.message,
        "The number at slides[0].elements[3].children[0].frame.w exceeds 0"
    );
}

/// Each diagnostic's code, `data.path` and limit (or `-`).
fn issues(diagnostics: &[Diagnostic]) -> Vec<(DiagnosticCode, String, String)> {
    diagnostics
        .iter()
        .map(|d| {
            let data = d.data.as_ref().unwrap();
            let limit = data.get("limit").map_or("-".into(), |l| l.to_js_string());
            (d.code, data["path"].to_js_string(), limit)
        })
        .collect()
}

fn source_issues(mutate: impl FnOnce(&mut Value)) -> Vec<(DiagnosticCode, String, String)> {
    let mut p = presentation(false);
    mutate(&mut p);
    issues(&verify_source_json(&p, &VerifyOptions::default()))
}

fn issue(code: DiagnosticCode, path: &str, limit: &str) -> (DiagnosticCode, String, String) {
    (code, path.to_owned(), limit.to_owned())
}

#[test]
fn a_number_fails_each_check_of_its_chain() {
    let frame = "slides[0].elements[1].frame";
    assert_eq!(
        source_issues(|p| element(p, 0, 1)["frame"]["w"] = json!(-200_000)),
        [
            issue(codes::IR0008, &format!("{frame}.w"), "-100000"),
            issue(codes::IR0008, &format!("{frame}.w"), "0"),
        ]
    );
    assert_eq!(
        source_issues(|p| element(p, 0, 1)["frame"]["rotation"] = json!(200_000)),
        [
            issue(codes::IR0008, &format!("{frame}.rotation"), "100000"),
            issue(codes::IR0008, &format!("{frame}.rotation"), "360"),
        ]
    );
    assert_eq!(
        source_issues(|p| element(p, 0, 1)["frame"]["x"] = json!(100_000.000_5)),
        [
            issue(codes::IR0008, &format!("{frame}.x"), "100000"),
            issue(codes::IR0007, &format!("{frame}.x"), "-"),
        ]
    );
    assert_eq!(
        source_issues(|p| element(p, 0, 1)["frame"]["w"] = json!(-0.0005)),
        [
            issue(codes::IR0007, &format!("{frame}.w"), "-"),
            issue(codes::IR0008, &format!("{frame}.w"), "0"),
        ]
    );
    assert_eq!(
        source_issues(|p| p["defaults"]["text"]["0"]["run"]["scriptScale"] = json!(1e6)),
        [
            issue(
                codes::IR0008,
                "defaults.text[\"0\"].run.scriptScale",
                "100000"
            ),
            issue(codes::IR0008, "defaults.text[\"0\"].run.scriptScale", "1"),
        ]
    );
}

#[test]
fn structural_issues_name_zods_paths() {
    let first = |mutate: fn(&mut Value)| source_issues(mutate).remove(0).1;
    let photo = "slides[0].elements[1]";
    assert_eq!(
        first(|p| remove(&mut element(p, 0, 1)["frame"], "x")),
        format!("{photo}.frame.x")
    );
    assert_eq!(first(|p| element(p, 0, 1)["zzz"] = json!(1)), photo);
    assert_eq!(
        first(|p| element(p, 0, 1)["kind"] = json!("blob")),
        format!("{photo}.kind")
    );
    assert_eq!(
        first(|p| remove(element(p, 0, 1), "kind")),
        format!("{photo}.kind")
    );
    assert_eq!(
        first(|p| p["defaults"]["text"]["zzz"] = json!({})),
        "defaults.text"
    );
    assert_eq!(
        first(|p| element(p, 0, 1)["ext"] = json!({ "zzz": {} })),
        format!("{photo}.ext.zzz")
    );
    assert_eq!(
        first(|p| {
            element(p, 0, 0)["text"]["listStyle"] = json!({ "9": {} });
        }),
        "slides[0].elements[0].text.listStyle[\"9\"]"
    );
}

#[test]
fn only_nullable_members_take_null() {
    for resolved in [false, true] {
        let p = presentation(resolved);
        let cases = support::nulled(&p);
        assert!(cases.len() > 400, "{}", cases.len());
        for (path, input, accepted) in cases {
            let at = support::show(&path);
            assert_eq!(
                parse_presentation_json(&input).is_ok(),
                accepted,
                "null at {at}, source schema"
            );
            if resolved {
                assert_eq!(
                    parse::from_value::<resolved::Presentation>(input).is_ok(),
                    accepted,
                    "null at {at}, resolved schema"
                );
            }
        }
    }
}

#[test]
fn integers_read_whatever_their_spelling() {
    for resolved in [false, true] {
        let p = presentation(resolved);
        let expected = to_json(&parse_presentation_json(&p).unwrap()).unwrap();
        let cases = support::floated(&p);
        assert!(cases.len() > 100, "{}", cases.len());
        for (path, input) in cases {
            let at = support::show(&path);
            let parsed = parse_presentation_json(&input)
                .unwrap_or_else(|d| panic!("{at} as a float: {:?}", located(&d)));
            assert_eq!(to_json(&parsed).unwrap(), expected, "{at} as a float");
        }
    }
}

#[test]
fn records_iterate_in_js_order() {
    let mut p = presentation(false);
    let entry = p["fonts"]["lato"].clone();
    p["fonts"]["1942"] = entry.clone();
    p["fonts"]["10"] = entry;
    element(&mut p, 1, 0)["geometry"]["adjust"] = json!({ "b": 2e9, "a": 2e9 });
    element(&mut p, 0, 1)["ext"] = json!({ "pptx": { "z": 1, "2": 1, "a": 1 } });
    let parsed = parse::from_value::<slidock_ir::source::Presentation>(p.clone()).unwrap();
    let fonts: Vec<&str> = parsed.fonts.keys().map(String::as_str).collect();
    assert_eq!(fonts, ["10", "1942", "lato"]);
    let ext = parsed.slides[0].elements[1].ext().unwrap();
    let keys: Vec<&str> = ext
        .pptx
        .as_ref()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, ["2", "z", "a"]);
    let paths: Vec<String> = issues(&verify_source_json(&p, &VerifyOptions::default()))
        .into_iter()
        .filter(|(code, _, _)| *code == codes::IR0008)
        .map(|(_, path, _)| path)
        .collect();
    assert_eq!(
        paths,
        [
            "slides[1].elements[0].geometry.adjust.b",
            "slides[1].elements[0].geometry.adjust.a"
        ]
    );
}

#[test]
fn invariants_fire_on_their_violation_only() {
    let cases: Vec<(&str, Invariant, Mutation, Vec<&str>)> = vec![
        (
            "IR0105 child space",
            Invariant::FramesCanonical,
            |p| element(p, 0, 3)["childSpace"] = json!({ "x": 0, "y": 0, "w": 1, "h": 1 }),
            vec!["IR0105@ir-s1"],
        ),
        (
            "IR0105 group flip",
            Invariant::FramesCanonical,
            |p| element(p, 0, 3)["frame"]["flipH"] = json!(true),
            vec!["IR0105@ir-s1"],
        ),
        (
            "IR0108 frame",
            Invariant::InheritanceResolved,
            |p| remove(element(p, 0, 1), "frame"),
            vec!["IR0108@ir-s1"],
        ),
        (
            "IR0115 body",
            Invariant::InheritanceResolved,
            |p| remove(&mut element(p, 1, 0)["text"]["body"]["insets"], "b"),
            vec!["IR0115@ir-s2"],
        ),
        (
            "IR0101 colour",
            Invariant::ThemeResolved,
            |p| {
                element(p, 1, 0)["text"]["paragraphs"][0]["inlines"][0]["props"]["color"] =
                    json!({ "kind": "theme", "slot": "tx1", "mods": [] });
            },
            vec!["IR0101@ir-s2"],
        ),
        (
            "IR0114 font",
            Invariant::ThemeResolved,
            |p| {
                element(p, 1, 0)["text"]["paragraphs"][0]["inlines"][0]["props"]["font"] =
                    json!({ "kind": "theme", "slot": "minor" });
            },
            vec!["IR0114@ir-s2"],
        ),
        (
            "IR0102 paragraph",
            Invariant::TextCascaded,
            |p| {
                remove(
                    &mut element(p, 1, 0)["text"]["paragraphs"][0]["props"],
                    "rtl",
                )
            },
            vec!["IR0102@ir-s2"],
        ),
        (
            "IR0103 run",
            Invariant::TextCascaded,
            |p| {
                remove(
                    &mut element(p, 1, 0)["text"]["paragraphs"][0]["inlines"][0]["props"],
                    "caps",
                )
            },
            vec!["IR0103@ir-s2"],
        ),
        (
            "IR0111 bullet",
            Invariant::ListsLabelled,
            |p| {
                element(p, 1, 0)["text"]["paragraphs"][0]["props"]["bullet"] =
                    json!({ "kind": "char", "char": "-" });
            },
            vec!["IR0111@ir-s2"],
        ),
        (
            "IR0104 font",
            Invariant::FontsBound,
            |p| remove(&mut p["fonts"]["lato"], "binding"),
            vec!["IR0104@presentation", "IR0104@ir-s1", "IR0104@ir-s2"],
        ),
        (
            "IR0110 metrics",
            Invariant::TextMetrics,
            |p| {
                remove(
                    &mut element(p, 1, 0)["text"]["paragraphs"][0]["props"],
                    "spaceBeforePt",
                )
            },
            vec!["IR0110@ir-s2"],
        ),
        (
            "IR0107 shrink",
            Invariant::TextMetrics,
            |p| {
                element(p, 1, 0)["text"]["body"]["autofit"] =
                    json!({ "kind": "shrink", "fontScale": 1 })
            },
            vec!["IR0107@ir-s2"],
        ),
        (
            "IR0105 geometry",
            Invariant::GeometryLowered,
            |p| {
                element(p, 1, 0)["geometry"] =
                    json!({ "kind": "preset", "name": "rect", "adjust": {} })
            },
            vec!["IR0105@ir-s2"],
        ),
        (
            "IR0113 fill",
            Invariant::GeometryLowered,
            |p| {
                p["slides"][1]["background"] = json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "stops": [{ "pos": 0, "color": rgb(1.0, 1.0, 1.0) }] });
            },
            vec!["IR0113@ir-s2"],
        ),
        (
            "IR0113 rotation",
            Invariant::GeometryLowered,
            |p| remove(&mut element(p, 1, 0)["text"], "rotation"),
            vec!["IR0113@ir-s2"],
        ),
        (
            "IR0106 plan",
            Invariant::MediaPlanned,
            |p| remove(&mut p["media"][photo_sha()], "plan"),
            vec!["IR0106@presentation"],
        ),
        (
            "IR0109 a11y",
            Invariant::A11yDone,
            |p| remove(&mut p["slides"][1], "title"),
            vec!["IR0109@ir-s2"],
        ),
        (
            "IR0003 placeholders.bound",
            Invariant::PlaceholdersBound,
            |p| element(p, 0, 0)["inherits"] = json!("ir-gone"),
            vec!["IR0003@ir-s1"],
        ),
        (
            "IR0003 timeline.normalized",
            Invariant::TimelineNormalized,
            |p| p["slides"][0]["timeline"][0]["target"]["element"] = json!("ir-gone"),
            vec!["IR0003@ir-s1"],
        ),
    ];
    for (rule, invariant, mutate, expected) in cases {
        let mut p = presentation(true);
        mutate(&mut p);
        let diagnostics = verify_resolved_json(&p, &VerifyOptions::default())
            .map(|_| ())
            .unwrap_err();
        assert_eq!(located(&diagnostics), expected, "{rule}");
        let typed = parse_presentation_json(&p).unwrap();
        assert_eq!(
            located(&check_invariant(
                invariant,
                &typed,
                &CheckOptions::default()
            )),
            expected,
            "{rule}"
        );
        for d in &diagnostics {
            assert!(invariant.rules().contains(&d.code), "{rule}: {}", d.code);
        }
    }
}

#[test]
fn failed_containers_are_exempt_and_keep_their_source_form() {
    let mut p = presentation(true);
    let source = presentation(false);
    let mut slide = source["slides"][1].clone();
    slide["elements"] = json!([]);
    slide["failed"] = json!({ "code": "IR0908", "pass": "cascade-text" });
    slide["background"] = json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "stops": [{ "pos": 0, "color": rgb(1.0, 1.0, 1.0) }] });
    p["slides"][1] = slide;
    let resolved = verify_resolved_json(&p, &VerifyOptions::default()).unwrap();
    assert!(resolved.slides[1].body.failed().is_some());
    assert_eq!(
        slidock_ir::canonical::to_json(&resolved).unwrap(),
        slidock_canon::value_to_canonical_json(&p, slidock_canon::Layout::Compact).unwrap()
    );
    let mut with_elements = p;
    with_elements["slides"][1]["elements"] =
        json!([{ "id": "ir-s2.x", "origin": agent(), "kind": "shape" }]);
    let diagnostics = verify_resolved_json(&with_elements, &VerifyOptions::default())
        .map(|_| ())
        .unwrap_err();
    assert_eq!(located(&diagnostics), ["IR0001@ir-s2"]);
}

#[test]
fn checkers_restrict_to_the_given_containers() {
    let mut p = presentation(true);
    remove(element(&mut p, 0, 1), "frame");
    remove(element(&mut p, 1, 0), "frame");
    let typed = parse_presentation_json(&p).unwrap();
    let options = CheckOptions {
        containers: Some(BTreeSet::from([NodeId::new("ir-s2").unwrap()])),
        ..CheckOptions::default()
    };
    assert_eq!(
        located(&check_invariant(
            Invariant::InheritanceResolved,
            &typed,
            &options
        )),
        ["IR0108@ir-s2"]
    );
}

#[test]
fn slide_links_resolve_against_the_linkable_slides() {
    let mut p = presentation(false);
    p["slides"].as_array_mut().unwrap().remove(0);
    p["slides"][0]["layout"] = json!("ir-layout");
    assert_eq!(source(&p), ["IR0003@ir-s2"]);
    let options = VerifyOptions {
        linkable_slides: vec![NodeId::new("ir-s1").unwrap()],
    };
    assert_eq!(verify_source_json(&p, &options), vec![]);
}

#[test]
fn census_counts_what_containers_render() {
    let p = parse_presentation_json(&presentation(false)).unwrap();
    let census = slidock_ir::features::census_presentation(&p);
    let keys: Vec<&str> = census.keys().map(String::as_str).collect();
    assert_eq!(
        keys,
        ["presentation", "ir-master", "ir-layout", "ir-s1", "ir-s2"]
    );
    let names = |id: &str| census[id].iter().map(|f| f.as_str()).collect::<Vec<_>>();
    assert_eq!(names("ir-master"), Vec::<&str>::new());
    assert_eq!(
        names("ir-layout"),
        [
            "element.shape",
            "geometry.preset",
            "layout.elements",
            "placeholder"
        ]
    );
    assert_eq!(
        names("ir-s1"),
        [
            "element.group",
            "element.picture",
            "element.shape",
            "element.table",
            "geometry.custom",
            "geometry.preset",
            "placeholder",
            "slide.timeline",
            "text.run"
        ]
    );
    assert_eq!(
        names("ir-s2"),
        [
            "element.shape",
            "geometry.preset",
            "presentation.hidden-slide",
            "text.link",
            "text.run"
        ]
    );
    let slide = Container::Slide(&p.slides[0]);
    assert_eq!(
        slidock_ir::features::census_container(slide, &p),
        census["ir-s1"]
    );
}

fn shadow(color: Value) -> Value {
    json!({ "dx": 0, "dy": 3.287, "sigma": 7.14, "color": color })
}

fn black(a: f64) -> Value {
    json!({ "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": a })
}

fn unresolved_accent() -> Value {
    json!({ "kind": "theme", "slot": "accent1", "mods": [] })
}

/// Where the tests below put shadows: the title, the photo, the group (slide 1) and the note's run (slide 2).
fn shadow_sites(p: &mut Value) -> [&mut Value; 4] {
    let [first, second] = p["slides"].as_array_mut().unwrap().as_mut_slice() else {
        panic!("two slides")
    };
    let [title, photo, _, group] = first["elements"].as_array_mut().unwrap().as_mut_slice() else {
        panic!("four elements")
    };
    let run = &mut second["elements"][0]["text"]["paragraphs"][0]["inlines"][0]["props"];
    [title, photo, group, run]
}

#[test]
fn shadows_are_absent_null_or_a_value_and_resolved_ones_read_null_as_absent() {
    for value in [Value::Null, shadow(black(0.5)), shadow(unresolved_accent())] {
        let mut p = presentation(false);
        for site in shadow_sites(&mut p) {
            site["shadow"] = value.clone();
        }
        assert_eq!(source(&p), Vec::<String>::new(), "{value}");
        let typed = parse_presentation_json(&p).unwrap();
        assert_eq!(
            to_json(&typed).unwrap(),
            slidock_canon::value_to_canonical_json(&p, slidock_canon::Layout::Compact).unwrap(),
            "{value}"
        );
    }
    let mut p = presentation(true);
    for site in shadow_sites(&mut p) {
        site["shadow"] = shadow(black(0.5));
    }
    let resolved = verify_resolved_json(&p, &VerifyOptions::default()).unwrap();
    assert_eq!(
        to_json(&resolved).unwrap(),
        slidock_canon::value_to_canonical_json(&p, slidock_canon::Layout::Compact).unwrap()
    );
    // Once nothing inherits, `null` is none: the resolved phase reads it as absent and writes nothing.
    let mut nulled = presentation(true);
    for site in shadow_sites(&mut nulled) {
        site["shadow"] = Value::Null;
    }
    let resolved = verify_resolved_json(&nulled, &VerifyOptions::default()).unwrap();
    assert_eq!(
        to_json(&resolved).unwrap(),
        slidock_canon::value_to_canonical_json(&presentation(true), slidock_canon::Layout::Compact)
            .unwrap()
    );
    // Tables and Foreign nodes carry none.
    let mut p = presentation(false);
    element(&mut p, 0, 2)["shadow"] = shadow(black(0.5));
    assert_eq!(source(&p), ["IR0001@ir-s1"]);
}

#[test]
fn bounds_a_shadows_sigma_and_refuses_one_that_paints_nothing() {
    let site = "slides[0].elements[1].shadow";
    assert_eq!(
        source_issues(|p| {
            element(p, 0, 1)["shadow"] = with_sigma(shadow(black(0.5)), 500.001);
        }),
        [issue(codes::IR0008, &format!("{site}.sigma"), "500")]
    );
    assert_eq!(
        source_issues(|p| element(p, 0, 1)["shadow"] = shadow(black(0.0))),
        [issue(codes::IR0008, &format!("{site}.color.a"), "0")]
    );
    assert_eq!(
        source_issues(|p| {
            p["slides"][1]["elements"][0]["text"]["paragraphs"][0]["inlines"][0]["props"]["shadow"] =
                with_sigma(shadow(black(0.5)), -1.0);
        }),
        [issue(
            codes::IR0008,
            "slides[1].elements[0].text.paragraphs[0].inlines[0].props.shadow.sigma",
            "0"
        )]
    );
}

fn with_sigma(mut shadow: Value, sigma: f64) -> Value {
    shadow["sigma"] = json!(sigma);
    shadow
}

#[test]
fn an_unresolved_shadow_colour_fails_theme_resolved_on_elements_and_runs() {
    let mut p = presentation(true);
    for site in shadow_sites(&mut p) {
        site["shadow"] = shadow(unresolved_accent());
    }
    let diagnostics = verify_resolved_json(&p, &VerifyOptions::default())
        .map(|_| ())
        .unwrap_err();
    let paths: Vec<String> = diagnostics
        .iter()
        .map(|d| {
            format!(
                "{} {}",
                d.code,
                d.data.as_ref().unwrap()["path"].to_js_string()
            )
        })
        .collect();
    assert_eq!(
        paths,
        [
            "IR0101 slides[0].elements[0].shadow.color",
            "IR0101 slides[0].elements[1].shadow.color",
            "IR0101 slides[0].elements[3].shadow.color",
            "IR0101 slides[1].elements[0].text.paragraphs[0].inlines[0].props.shadow.color",
        ]
    );
}

#[test]
fn the_census_counts_an_element_shadow_and_a_run_shadow_apart() {
    let mut p = presentation(false);
    let [_, photo, _, run] = shadow_sites(&mut p);
    photo["shadow"] = shadow(black(0.5));
    run["shadow"] = shadow(black(0.5));
    let typed = parse_presentation_json(&p).unwrap();
    let census = slidock_ir::features::census_presentation(&typed);
    assert!(census["ir-s1"].contains(&slidock_ir::features::FeatureId::EffectShadow));
    assert!(!census["ir-s1"].contains(&slidock_ir::features::FeatureId::TextShadow));
    assert!(census["ir-s2"].contains(&slidock_ir::features::FeatureId::TextShadow));
    assert!(!census["ir-s2"].contains(&slidock_ir::features::FeatureId::EffectShadow));
    // A stated `null` is no shadow.
    let mut nulled = presentation(false);
    for site in shadow_sites(&mut nulled) {
        site["shadow"] = Value::Null;
    }
    let typed = parse_presentation_json(&nulled).unwrap();
    let census = slidock_ir::features::census_presentation(&typed);
    for features in census.values() {
        assert!(
            features.iter().all(|f| !f.as_str().ends_with("shadow")),
            "{features:?}"
        );
    }
}

#[test]
fn run_properties_without_a_shadow_keep_their_bytes() {
    let complete: slidock_ir::text::RunProps = serde_json::from_value(run_props(true)).unwrap();
    let resolved: slidock_ir::text::ResolvedRunProps =
        serde_json::from_value(run_props(false)).unwrap();
    let partial: slidock_ir::text::PartialRunProps =
        serde_json::from_value(run_props(true)).unwrap();
    assert!(complete.shadow.is_none() && resolved.shadow.is_none() && partial.shadow.is_none());
    for (written, given) in [
        (to_json(&complete).unwrap(), run_props(true)),
        (to_json(&resolved).unwrap(), run_props(false)),
        (to_json(&partial).unwrap(), run_props(true)),
    ] {
        assert_eq!(
            written,
            slidock_canon::value_to_canonical_json(&given, slidock_canon::Layout::Compact).unwrap()
        );
    }
    assert!(partial.missing_keys().is_empty());
}
