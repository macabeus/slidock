//! The pass tests that read the private IR fixtures (`--features golden`, tests/golden): each fixture lowered as the IR
//! frontend lowers it, then run through the passes before the one under test. Skips when the fixtures are absent.
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_core_api::visit::walk_elements;
use slidock_ir::diagnostics::Diagnostic;
use slidock_ir::source::{Element, Presentation};
use slidock_ir::verify::{CheckOptions, Invariant, verify_invariants};
use support::fixtures::*;
use support::*;

/// A fixture lowered and run through the passes before `name`.
fn before(fixture: &str, name: &str) -> Presentation {
    apply_passes(&passes_before(name), &lowered_fixture(fixture).presentation).presentation
}

/// A fixture lowered, run through the passes before `name`, then `name`.
fn through(fixture: &str, name: &str) -> Applied {
    apply_pass(&pass_named(name), &before(fixture, name))
}

/// The JSON of an element.
fn el(presentation: &Presentation, id: &str) -> Value {
    to_json(element(presentation, id))
}

/// A presentation changed as JSON.
fn edited(presentation: &Presentation, edit: impl FnOnce(&mut Value)) -> Presentation {
    let mut value = to_json(presentation);
    edit(&mut value);
    parse(value)
}

fn escape(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

/// The JSON pointer of the element of that id, in a presentation's JSON.
fn pointer_of(presentation: &Value, id: &str) -> String {
    fn find(elements: &Value, at: String, id: &str) -> Option<String> {
        for (index, element) in elements.as_array()?.iter().enumerate() {
            let here = format!("{at}/{index}");
            if element["id"] == id {
                return Some(here);
            }
            if let Some(found) = find(&element["children"], format!("{here}/children"), id) {
                return Some(found);
            }
        }
        None
    }
    let mut containers: Vec<(String, &Value)> = Vec::new();
    for record in ["masters", "layouts"] {
        for (key, container) in presentation[record].as_object().unwrap() {
            containers.push((
                format!("/{record}/{}/elements", escape(key)),
                &container["elements"],
            ));
        }
    }
    for (index, slide) in presentation["slides"]
        .as_array()
        .unwrap()
        .iter()
        .enumerate()
    {
        containers.push((format!("/slides/{index}/elements"), &slide["elements"]));
    }
    containers
        .into_iter()
        .find_map(|(at, elements)| find(elements, at, id))
        .unwrap_or_else(|| panic!("no element {id}"))
}

/// The element of that id, as JSON, inside a presentation's JSON.
fn element_mut<'a>(presentation: &'a mut Value, id: &str) -> &'a mut Value {
    let pointer = pointer_of(presentation, id);
    presentation.pointer_mut(&pointer).unwrap()
}

fn hex(color: &Value) -> String {
    let rgb = if color["kind"] == "theme" {
        &color["resolved"]
    } else {
        color
    };
    let channel = |c: &Value| {
        let x = c.as_f64().unwrap() * 255.0;
        format!("{:02x}", slidock_canon::js_round(x) as u32)
    };
    let mut out: String = ["r", "g", "b"].iter().map(|k| channel(&rgb[*k])).collect();
    let alpha = rgb["a"].as_f64().unwrap();
    if alpha < 1.0 {
        out.push_str(&format!("/{}", slidock_canon::to_js_string(alpha)));
    }
    out
}

fn sites(diagnostics: &[Diagnostic]) -> Vec<Value> {
    diagnostics
        .iter()
        .map(|d| json!([d.code.code(), d.severity, d.container, d.node, d.message]))
        .collect()
}

fn inherits_of(presentation: &Presentation, container: &str) -> Value {
    let container = presentation
        .containers()
        .find(|c| c.id().as_str() == container)
        .unwrap();
    let map: serde_json::Map<String, Value> = walk_elements(container.elements())
        .map(|e: &Element| (e.id().to_string(), json!(e.inherits())))
        .collect();
    Value::Object(map)
}

/// Whether `actual` holds every field of `expected`, recursively.
fn matches(actual: &Value, expected: &Value) -> bool {
    match (actual, expected) {
        (Value::Object(a), Value::Object(e)) => e
            .iter()
            .all(|(k, v)| a.get(k).is_some_and(|x| matches(x, v))),
        _ => actual == expected,
    }
}

#[test]
fn the_minimal_builder_is_the_minimal_fixture() {
    if golden().is_none() {
        return;
    }
    assert_eq!(
        to_json(&minimal()),
        to_json(&presentation_ir_fixture("source", "minimal"))
    );
}

mod normalize_frames {
    use super::*;

    #[test]
    fn keeps_frames_without_a_child_space_and_everything_else_on_the_full_fixture() {
        if golden().is_none() {
            return;
        }
        let presentation = source_fixture("full");
        let out = apply_pass(&pass_named("normalize-frames"), &presentation).presentation;
        assert_eq!(el(&out, "ir-s1.arrow"), el(&presentation, "ir-s1.arrow"));
    }
}

mod bind_placeholders {
    use super::*;

    fn input() -> Presentation {
        apply_pass(
            &pass_named("normalize-frames"),
            &lowered_fixture("inheritance").presentation,
        )
        .presentation
    }

    #[test]
    fn binds_slides_by_index_then_role_class_and_layouts_to_masters_by_role_class() {
        if golden().is_none() {
            return;
        }
        let out = apply_pass(&pass_named("bind-placeholders"), &input());
        let p = &out.presentation;
        assert_eq!(
            inherits_of(p, "ir-l.content"),
            json!({ "ir-l.content.title": "ir-m.title", "ir-l.content.body": "ir-m.body", "ir-l.content.num": "ir-m.num" })
        );
        // Layouts bind by class: ctrTitle to the master's title, subTitle to its body.
        assert_eq!(
            inherits_of(p, "ir-l.dark"),
            json!({ "ir-l.dark.title": "ir-m.title", "ir-l.dark.sub": "ir-m.body" })
        );
        assert_eq!(
            inherits_of(p, "ir-s1"),
            json!({
                "ir-s1.title": "ir-l.content.title", "ir-s1.body": "ir-l.content.body", "ir-s1.num": "ir-l.content.num",
                "ir-s1.note": null,
                // Pictures belong to the body class.
                "ir-s1.photo": "ir-l.content.body", "ir-s1.hdr": null
            })
        );
        let s2 = inherits_of(p, "ir-s2");
        assert_eq!(
            (&s2["ir-s2.title"], &s2["ir-s2.sub"]),
            (&json!("ir-l.dark.title"), &json!("ir-l.dark.sub"))
        );
        assert_eq!(inherits_of(p, "ir-master")["ir-m.title"], Value::Null);
        assert_eq!(
            sites(&out.diagnostics),
            [json!([
                "MID5101",
                "warning",
                "ir-s1",
                "ir-s1.hdr",
                "The hdr placeholder matches no placeholder of ir-l.content"
            ])]
        );
    }

    #[test]
    fn prefers_an_explicit_target_and_an_exact_role_within_the_class() {
        if golden().is_none() {
            return;
        }
        let shape = |id: &str, placeholder: Value| json!({ "id": id, "kind": "shape", "origin": agent(), "placeholder": placeholder });
        let input = edited(&input(), |p| {
            let layout = p["layouts"]["ir-l.content"]["elements"]
                .as_array_mut()
                .unwrap();
            layout.push(shape("ir-l.content.sub", json!({ "role": "subTitle" })));
            layout.push(shape("ir-l.content.obj", json!({ "role": "obj" })));
            let slide = p["slides"][0]["elements"].as_array_mut().unwrap();
            slide.push(shape(
                "ir-s1.x",
                json!({ "role": "title", "target": "ir-m.date" }),
            ));
            slide.push(shape("ir-s1.y", json!({ "role": "obj" })));
        });
        let out = apply_pass(&pass_named("bind-placeholders"), &input).presentation;
        let s1 = inherits_of(&out, "ir-s1");
        assert_eq!(s1["ir-s1.x"], "ir-m.date");
        assert_eq!(s1["ir-s1.y"], "ir-l.content.obj");
    }

    #[test]
    fn recomputes_edges_a_stale_one_replaced_and_an_unmatched_one_removed() {
        if golden().is_none() {
            return;
        }
        let input = edited(&input(), |p| {
            for element in p["slides"][0]["elements"].as_array_mut().unwrap() {
                element["inherits"] = json!("ir-m.band");
            }
            let first = p["slides"][0].clone();
            p["slides"] = json!([first]);
        });
        let out = apply_pass(&pass_named("bind-placeholders"), &input).presentation;
        let s1 = inherits_of(&out, "ir-s1");
        assert_eq!(s1["ir-s1.body"], "ir-l.content.body");
        assert_eq!(s1["ir-s1.hdr"], Value::Null);
        // Non-placeholders keep what they have: this pass binds placeholders only.
        assert_eq!(s1["ir-s1.note"], "ir-m.band");
    }

    #[test]
    fn warns_nothing_for_slides_without_a_layout() {
        if golden().is_none() {
            return;
        }
        let passes = [
            pass_named("normalize-frames"),
            pass_named("bind-placeholders"),
        ];
        assert!(
            apply_passes(&passes, &source_fixture("minimal"))
                .diagnostics
                .is_empty()
        );
    }
}

mod resolve_inheritance {
    use super::*;

    fn bound() -> Presentation {
        before("inheritance", "resolve-inheritance")
    }

    fn resolved() -> Presentation {
        apply_pass(&pass_named("resolve-inheritance"), &bound()).presentation
    }

    #[test]
    fn copies_the_frame_and_geometry_from_the_nearest_ancestor_that_has_them() {
        if golden().is_none() {
            return;
        }
        let (bound, resolved) = (bound(), resolved());
        // The slide title and its layout title both lack a frame, so the master's applies.
        assert_eq!(
            el(&resolved, "ir-s1.title")["frame"],
            el(&bound, "ir-m.title")["frame"]
        );
        assert_eq!(
            el(&resolved, "ir-s1.title")["geometry"],
            json!({ "kind": "preset", "name": "rect", "adjust": {} })
        );
        // The layout body overrides the master's frame.
        assert_eq!(
            el(&resolved, "ir-s1.body")["frame"],
            el(&bound, "ir-l.content.body")["frame"]
        );
        assert_eq!(
            el(&resolved, "ir-l.content.title")["frame"],
            el(&bound, "ir-m.title")["frame"]
        );
    }

    #[test]
    fn merges_body_properties_field_wise_and_completes_them_from_the_defaults() {
        if golden().is_none() {
            return;
        }
        let (bound, resolved) = (bound(), resolved());
        let defaults = to_json(&bound.defaults.body);
        assert_eq!(
            el(&resolved, "ir-s1.title")["text"]["body"],
            with(defaults.clone(), json!({ "anchor": "bottom" }))
        );
        assert_eq!(
            el(&resolved, "ir-s1.body")["text"]["body"],
            with(
                defaults.clone(),
                json!({ "insets": { "l": 0, "t": 0, "r": 0, "b": 0 } })
            )
        );
        assert_eq!(
            el(&resolved, "ir-s3.body")["text"]["body"]["autofit"],
            json!({ "kind": "shrink", "fontScale": 0.8, "lineSpacingReduction": 0.1 })
        );
        // Shapes outside any chain take the defaults too.
        assert_eq!(
            el(&resolved, "ir-s1.note")["text"]["body"],
            with(defaults, json!({ "anchor": "middle" }))
        );
    }

    #[test]
    fn merges_list_styles_per_level_nearest_last() {
        if golden().is_none() {
            return;
        }
        let (bound, resolved) = (bound(), resolved());
        assert_eq!(
            el(&resolved, "ir-s1.num")["text"]["listStyle"],
            json!({ "0": { "para": { "align": "right" } } })
        );
        assert_eq!(
            el(&resolved, "ir-s1.body")["text"]["listStyle"],
            json!({ "1": { "run": { "italic": true } } })
        );
        assert_eq!(
            el(&resolved, "ir-s2.sub")["text"]["listStyle"],
            el(&bound, "ir-l.dark.sub")["text"]["listStyle"]
        );
    }

    #[test]
    fn never_inherits_text_content_and_keeps_an_explicit_null_stroke() {
        if golden().is_none() {
            return;
        }
        let bound = bound();
        let input = edited(&bound, |p| {
            let body = element_mut(p, "ir-l.content.body");
            body["stroke"] = Value::Null;
            body["text"]["paragraphs"] = json!([{ "level": 0, "props": {},
                "inlines": [{ "kind": "run", "text": "Prompt", "props": {} }] }]);
            let first = p["slides"][0].clone();
            p["slides"] = json!([first]);
        });
        let out = apply_pass(&pass_named("resolve-inheritance"), &input).presentation;
        assert_eq!(
            el(&out, "ir-s1.body")["text"]["paragraphs"],
            el(&bound, "ir-s1.body")["text"]["paragraphs"]
        );
        assert_eq!(el(&out, "ir-s1.body")["stroke"], Value::Null);
        assert!(el(&out, "ir-s1.body").get("stroke").is_some());
        assert!(el(&resolved(), "ir-s1.body").get("stroke").is_none());
    }

    #[test]
    fn gives_the_same_result_when_the_chain_was_resolved_first() {
        if golden().is_none() {
            return;
        }
        let resolved = resolved();
        let twice = apply_pass(&pass_named("resolve-inheritance"), &resolved).presentation;
        assert_eq!(to_json(&twice), to_json(&resolved));
    }
}

mod resolve_theme {
    use super::*;

    fn themed() -> Applied {
        through("inheritance", "resolve-theme")
    }

    #[test]
    fn resolves_through_the_effective_colour_map() {
        if golden().is_none() {
            return;
        }
        let themed = themed().presentation;
        let p = to_json(&themed);
        assert_eq!(
            hex(&p["masters"]["ir-master"]["background"]["color"]),
            "ffffff"
        );
        // ir-s2's layout swaps the text and background roles: tx1 shows lt1.
        let badge = el(&themed, "ir-s2.badge");
        assert_eq!(hex(&badge["stroke"]["fill"]["color"]), "ffffff");
        assert_eq!(hex(&badge["fill"]["color"]), "2f5597");
        // ir-s3 overrides its own map: bg1 shows dk1, and accent1 shows accent2.
        assert_eq!(hex(&p["slides"][2]["background"]["color"]), "000000");
    }

    #[test]
    fn resolves_list_styles_bullets_and_master_text_styles_and_theme_fonts_to_families() {
        if golden().is_none() {
            return;
        }
        let themed = themed().presentation;
        let p = to_json(&themed);
        let styles = &p["masters"]["ir-master"]["textStyles"];
        assert_eq!(
            styles["title"]["0"]["run"]["font"],
            json!({ "kind": "family", "family": "Liberation Serif", "theme": "major" })
        );
        assert_eq!(hex(&styles["title"]["0"]["run"]["color"]), "44546a");
        assert_eq!(
            hex(&styles["body"]["2"]["para"]["bullet"]["color"]),
            "4472c4"
        );
        assert_eq!(
            hex(&el(&themed, "ir-l.dark.sub")["text"]["listStyle"]["0"]["run"]["color"]),
            "ffffff/0.8"
        );
    }

    #[test]
    fn recomputes_a_resolved_value_on_every_run() {
        if golden().is_none() {
            return;
        }
        let themed = themed().presentation;
        let wrong = edited(&themed, |p| {
            element_mut(p, "ir-s2.badge")["fill"]["color"]["resolved"] = rgb(1.0, 0.0, 0.0);
        });
        let again = apply_pass(&pass_named("resolve-theme"), &wrong).presentation;
        assert_eq!(hex(&el(&again, "ir-s2.badge")["fill"]["color"]), "2f5597");
    }

    #[test]
    fn falls_back_to_the_default_theme_with_a_warning_when_a_container_has_none() {
        if golden().is_none() {
            return;
        }
        let geometry = through("geometry", "resolve-theme");
        assert_eq!(
            hex(&el(&geometry.presentation, "ir-g4.none")["fill"]["color"]),
            "ffc000"
        );
        let reported: Vec<Value> = geometry
            .diagnostics
            .iter()
            .map(|d| json!([d.code.code(), d.container, d.message]))
            .collect();
        assert_eq!(
            reported,
            [json!([
                "MID5301",
                "ir-g4",
                "No theme to resolve theme references against; the defaults stand in"
            ])]
        );
        assert!(themed().diagnostics.is_empty());
    }
}

mod cascade_text {
    use super::*;

    fn cascaded() -> Applied {
        through("inheritance", "cascade-text")
    }

    fn paragraphs(p: &Presentation, id: &str) -> Vec<Value> {
        el(p, id)["text"]["paragraphs"].as_array().unwrap().clone()
    }

    fn first_run(p: &Presentation, id: &str, index: usize) -> Value {
        paragraphs(p, id)[index]["inlines"][0]["props"].clone()
    }

    #[test]
    fn layers_the_defaults_the_masters_class_styles_the_list_style_and_the_paragraph() {
        if golden().is_none() {
            return;
        }
        let p = cascaded().presentation;
        let body = paragraphs(&p, "ir-s1.body");
        assert!(
            matches(
                &body[0]["props"],
                &json!({ "marginLeft": 18, "indent": -18, "bullet": { "kind": "char", "char": "•" },
                         "spaceBefore": { "kind": "pct", "value": 0.2 }, "align": "left", "tabSize": 72 })
            ),
            "{}",
            body[0]["props"]
        );
        assert!(matches(
            &first_run(&p, "ir-s1.body", 0),
            &json!({ "size": 24, "italic": false,
                     "font": { "kind": "family", "family": "Liberation Sans", "theme": "minor" } })
        ));
        // The layout's list style makes level 1 italic.
        assert!(matches(
            &first_run(&p, "ir-s1.body", 1),
            &json!({ "size": 20, "italic": true })
        ));
        assert_eq!(
            body[1]["props"]["bullet"],
            json!({ "kind": "char", "char": "–", "size": { "kind": "pct", "value": 0.9 } })
        );
        let weights: Vec<Value> = body[2]["inlines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["props"]["weight"].clone())
            .collect();
        assert_eq!(weights, [json!(400), json!(700)]);
        assert_eq!(body[3]["props"]["bullet"], json!({ "kind": "none" }));
    }

    #[test]
    fn resolves_copied_theme_references_in_the_containers_own_colour_map() {
        if golden().is_none() {
            return;
        }
        let p = cascaded().presentation;
        // The master's title colour is tx2; on the dark layout's slide tx2 shows lt2 instead of the master's dk2.
        assert_eq!(hex(&first_run(&p, "ir-s1.title", 0)["color"]), "44546a");
        assert_eq!(hex(&first_run(&p, "ir-s2.title", 0)["color"]), "e7e6e6");
        assert!(matches(
            &first_run(&p, "ir-s2.title", 0),
            &json!({ "size": 40, "weight": 700, "font": { "family": "Liberation Serif" } })
        ));
    }

    #[test]
    fn uses_only_the_defaults_for_non_placeholders_and_the_other_styles_for_footers() {
        if golden().is_none() {
            return;
        }
        let p = cascaded().presentation;
        let note = first_run(&p, "ir-s1.note", 0);
        assert!(matches(
            &note,
            &json!({ "size": 18, "color": { "slot": "tx1" } })
        ));
        assert_eq!(hex(&note["color"]), "000000");
        assert_eq!(first_run(&p, "ir-s1.num", 0)["size"], 12);
        assert_eq!(paragraphs(&p, "ir-s1.num")[0]["props"]["align"], "right");
    }

    #[test]
    fn draws_a_raised_or_lowered_run_at_its_script_scale_and_keeps_the_scale_out() {
        if golden().is_none() {
            return;
        }
        let themed = before("inheritance", "cascade-text");
        let input = edited(&themed, |p| {
            for level in p["defaults"]["text"].as_object_mut().unwrap().values_mut() {
                level["run"]["scriptScale"] = json!(0.5);
            }
            let paragraph = &mut element_mut(p, "ir-s1.title")["text"]["paragraphs"][0];
            let run = paragraph["inlines"][0].clone();
            let mut raised = run.clone();
            raised["props"]["baseline"] = json!(0.3);
            paragraph["inlines"] = json!([raised, run]);
        });
        let out = apply_pass(&pass_named("cascade-text"), &input).presentation;
        let inlines = paragraphs(&out, "ir-s1.title")[0]["inlines"].clone();
        let size = first_run(&cascaded().presentation, "ir-s1.title", 0)["size"]
            .as_f64()
            .unwrap();
        assert_eq!(inlines[0]["props"]["size"].as_f64().unwrap(), size / 2.0);
        assert_eq!(inlines[0]["props"]["baseline"], json!(0.3));
        assert_eq!(inlines[1]["props"]["size"].as_f64().unwrap(), size);
        assert!(inlines[0]["props"].get("scriptScale").is_none());
    }

    #[test]
    fn completes_paragraph_mark_properties() {
        if golden().is_none() {
            return;
        }
        let empty = paragraphs(&cascaded().presentation, "ir-s1.body")[4].clone();
        assert_eq!(empty["inlines"], json!([]));
        assert!(matches(
            &empty["endProps"],
            &json!({ "size": 14, "weight": 400, "font": { "kind": "family" } })
        ));
    }

    #[test]
    fn completes_a_stored_shrink_as_fitted_and_leaves_a_missing_scale_to_text_metrics() {
        if golden().is_none() {
            return;
        }
        let cascaded = cascaded();
        let p = &cascaded.presentation;
        assert_eq!(
            el(p, "ir-s3.body")["text"]["body"]["autofit"],
            json!({ "kind": "shrink", "fontScale": 0.8, "lineSpacingReduction": 0.1, "fitted": true })
        );
        assert_eq!(
            el(p, "ir-s3.unfit")["text"]["body"]["autofit"],
            json!({ "kind": "shrink" })
        );
        assert!(
            cascaded.diagnostics.is_empty(),
            "{:?}",
            cascaded.diagnostics
        );
    }

    #[test]
    fn table_cell_text_cascades_from_the_defaults_alone_whatever_the_placeholder() {
        if golden().is_none() {
            return;
        }
        let table = lowered_fixture("table").presentation;
        // A default text box that shrinks its text: a cell never does, as its row grows instead.
        let shrinking = edited(&table, |p| {
            p["defaults"]["body"]["autofit"] = json!({ "kind": "shrink" });
        });
        let out = apply_passes(&passes_before("number-lists"), &shrinking);
        let grid = to_json(&out.presentation.slides[0].elements[0]);
        assert_eq!(grid["kind"], "table");
        assert_eq!(grid["inherits"], "ir-layout.content");
        let header = &grid["rows"][0]["cells"][0];
        // Not the layout placeholder's insets and list size (20 pt, 30 pt), nor the master body size (28 pt).
        assert_eq!(
            header["text"]["body"]["insets"],
            json!({ "l": 7.2, "t": 3.6, "r": 7.2, "b": 3.6 })
        );
        assert_eq!(header["text"]["body"]["autofit"], json!({ "kind": "none" }));
        let run = &header["text"]["paragraphs"][0]["inlines"][0]["props"];
        assert_eq!((&run["size"], &run["weight"]), (&json!(18), &json!(700)));
        assert!(!out.diagnostics.iter().any(|d| d.code.code() == "MID5403"
            && d.node.as_ref().map(|n| n.as_str()) == grid["id"].as_str()));
    }
}

mod number_lists {
    use super::*;

    fn labelled() -> Presentation {
        through("inheritance", "number-lists").presentation
    }

    fn bullets(p: &Presentation, id: &str) -> Vec<Value> {
        el(p, id)["text"]["paragraphs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|paragraph| paragraph["props"]["bullet"].clone())
            .collect()
    }

    fn labels(p: &Presentation, id: &str) -> Vec<Value> {
        bullets(p, id)
            .iter()
            .map(|b| {
                if b["kind"] == "label" {
                    b["text"].clone()
                } else {
                    Value::Null
                }
            })
            .collect()
    }

    #[test]
    fn numbers_with_powerpoints_continuation_and_restart_rules() {
        if golden().is_none() {
            return;
        }
        let expected = json!([
            "1.", "2.", "a)", "b)",
            // A shallower paragraph keeps its level's counter and resets the deeper one.
            "3.", "a)",
            // An unnumbered paragraph restarts its level; so does a new scheme, at its own start.
            null, "1.", "IV.", "V.", "(BB)"
        ]);
        assert_eq!(json!(labels(&labelled(), "ir-s2.list")), expected);
    }

    #[test]
    fn lets_a_paragraph_that_shows_no_text_take_no_number_and_break_no_count() {
        if golden().is_none() {
            return;
        }
        let input = edited(&before("inheritance", "number-lists"), |p| {
            let text = &mut element_mut(p, "ir-s2.list")["text"];
            let one = text["paragraphs"][0].clone();
            let two = text["paragraphs"][1].clone();
            let blank = |level: u32| {
                let mut blank = one.clone();
                blank["level"] = json!(level);
                blank["inlines"][0]["text"] = json!("");
                blank
            };
            // One blank at the list's level, one deeper: both leave the counter as it is.
            text["paragraphs"] = json!([one, blank(0), blank(1), two]);
        });
        let out = apply_pass(&pass_named("number-lists"), &input).presentation;
        assert_eq!(
            json!(labels(&out, "ir-s2.list")),
            json!(["1.", "2.", "1.", "2."])
        );
    }

    #[test]
    fn takes_the_labels_font_size_and_colour_from_the_bullet_else_from_the_first_run() {
        if golden().is_none() {
            return;
        }
        let bullets = bullets(&labelled(), "ir-s2.list");
        assert_eq!(
            bullets[0]["font"],
            json!({ "kind": "family", "family": "Liberation Sans", "theme": "minor" })
        );
        assert_eq!(bullets[0]["size"], 18);
        assert_eq!(
            bullets[0]["auto"],
            json!({ "scheme": "arabicPeriod", "startAt": 1 })
        );
        assert_eq!(bullets[9]["font"]["family"], "Liberation Serif");
        assert_eq!(bullets[9]["size"], json!(11.2));
        assert_eq!(bullets[9]["color"]["slot"], "accent2");
    }

    #[test]
    fn labels_character_bullets_with_their_character_sized_against_the_run() {
        if golden().is_none() {
            return;
        }
        let bullets = bullets(&labelled(), "ir-s1.body");
        assert!(matches(
            &bullets[0],
            &json!({ "kind": "label", "text": "•", "size": 24 })
        ));
        assert!(matches(
            &bullets[1],
            &json!({ "kind": "label", "text": "–", "size": 18 })
        ));
        assert!(matches(
            &bullets[2],
            &json!({ "kind": "label", "text": "▪", "font": { "family": "DejaVu Sans" },
                     "color": { "slot": "accent1" } })
        ));
        assert_eq!(bullets[3], json!({ "kind": "none" }));
        assert!(matches(
            &bullets[4],
            &json!({ "kind": "label", "text": "•", "size": 14 })
        ));
    }
}

mod fonts_and_metrics {
    use super::*;

    fn bound() -> Applied {
        through("inheritance", "bind-fonts")
    }

    fn measured() -> Presentation {
        apply_pass(&pass_named("text-metrics"), &bound().presentation).presentation
    }

    fn props(p: &Presentation, id: &str, index: usize) -> Value {
        el(p, id)["text"]["paragraphs"][index]["props"].clone()
    }

    fn metrics_of(bound: &Presentation, key: &str) -> Value {
        to_json(&bound.fonts[key].binding.as_ref().unwrap().metrics)
    }

    /// `Number(x.toFixed(digits))`.
    fn fixed(x: f64, digits: u8) -> Value {
        serde_json::from_str(&slidock_canon::to_fixed(x, digits).unwrap()).unwrap()
    }

    fn ratio(value: f64) -> Value {
        json!({ "kind": "ratio", "value": fixed(value, 5) })
    }

    #[test]
    fn requests_every_family_the_text_and_labels_use_with_the_faces_used() {
        if golden().is_none() {
            return;
        }
        let bound = bound().presentation;
        let keys: Vec<&String> = bound.fonts.keys().collect();
        assert_eq!(
            keys,
            [
                "arial",
                "dejavusans",
                "firacode",
                "liberationsans",
                "liberationsansmedium",
                "liberationserif"
            ]
        );
        assert_eq!(
            to_json(&bound.fonts["liberationsans"].request),
            json!({ "family": "Liberation Sans", "faces": [
                { "weight": 400, "style": "italic" }, { "weight": 400, "style": "normal" },
                { "weight": 700, "style": "normal" }
            ], "postscriptNames": [], "embedded": false })
        );
    }

    #[test]
    fn binds_installed_families_weight_names_substitutes_and_generics() {
        if golden().is_none() {
            return;
        }
        let bound = bound();
        let binding = |key: &str| {
            let b = to_json(&bound.presentation.fonts[key].binding);
            json!({ "kind": b["kind"], "family": b["family"], "weight": b["weight"], "generic": b["generic"] })
        };
        assert_eq!(
            binding("liberationserif"),
            json!({ "kind": "installed", "family": "Liberation Serif", "weight": null, "generic": "serif" })
        );
        assert_eq!(
            binding("liberationsansmedium"),
            json!({ "kind": "installed", "family": "Liberation Sans", "weight": 500, "generic": "sans-serif" })
        );
        assert_eq!(
            binding("arial"),
            json!({ "kind": "substitute", "family": "Liberation Sans", "weight": null, "generic": "sans-serif" })
        );
        assert_eq!(
            binding("firacode"),
            json!({ "kind": "generic", "family": "Liberation Mono", "weight": null, "generic": "monospace" })
        );
        assert_eq!(
            metrics_of(&bound.presentation, "arial"),
            metrics_of(&bound.presentation, "liberationsans")
        );
        let reported: Vec<Value> = bound
            .diagnostics
            .iter()
            .map(|d| json!([d.code.code(), d.severity, d.container, d.message]))
            .collect();
        assert_eq!(
            reported,
            [
                json!([
                    "MID5401",
                    "lossy",
                    "ir-s3",
                    "Arial is not available; Liberation Sans stands in"
                ]),
                json!([
                    "MID5402",
                    "lossy",
                    "ir-s3",
                    "Fira Code is not available and has no substitute; the generic Liberation Mono stands in"
                ]),
            ]
        );
    }

    #[test]
    fn computes_ratios_from_the_basis_less_a_stored_line_spacing_reduction() {
        if golden().is_none() {
            return;
        }
        let p = measured();
        assert_eq!(props(&p, "ir-s1.note", 0)["lineHeight"], ratio(1.2));
        // (1 − 0.1) × 1.2 under the stored shrink.
        assert_eq!(props(&p, "ir-s3.body", 0)["lineHeight"], ratio(1.08));
        let bound = bound().presentation;
        let sans = &bound.fonts["liberationsans"]
            .binding
            .as_ref()
            .unwrap()
            .metrics;
        let hhea = slidock_passes::basis_ratio(slidock_ir::text::LineBasis::Hhea, sans).unwrap();
        assert_eq!(props(&p, "ir-s3.grow", 3)["lineHeight"], ratio(1.15 * hhea));
        assert_eq!(props(&p, "ir-s3.grow", 4)["lineHeight"], ratio(1.08838));
        assert_eq!(props(&p, "ir-s3.grow", 5)["lineHeight"], ratio(1.11719));
    }

    /// Slide 3's `ir-s3.grow` with its first paragraph changed by `edit`, measured with these font entries.
    fn measure_grow(fonts: Option<Value>, edit: impl Fn(&mut Value)) -> Value {
        let input = edited(&bound().presentation, |p| {
            if let Some(fonts) = fonts {
                p["fonts"] = fonts;
            }
            let paragraphs = &mut element_mut(p, "ir-s3.grow")["text"]["paragraphs"];
            edit(&mut paragraphs[0]);
        });
        let out = apply_pass(&pass_named("text-metrics"), &input).presentation;
        el(&out, "ir-s3.grow")["text"]["paragraphs"][0].clone()
    }

    /// An edit giving the paragraph this line spacing and one copy of its first run per size and weight.
    fn spaced(value: f64, basis: &str, runs: &[(f64, u32)]) -> impl Fn(&mut Value) {
        let spacing = json!({ "kind": "multiple", "value": value, "basis": basis });
        let runs: Vec<(f64, u32)> = runs.to_vec();
        move |paragraph: &mut Value| {
            paragraph["props"]["lineSpacing"] = spacing.clone();
            let run = paragraph["inlines"][0].clone();
            let inlines: Vec<Value> = runs
                .iter()
                .map(|(size, weight)| {
                    let mut run = run.clone();
                    run["props"]["size"] = json!(size);
                    run["props"]["weight"] = json!(weight);
                    run
                })
                .collect();
            paragraph["inlines"] = json!(inlines);
        }
    }

    /// The font entries with Liberation Sans's metrics, or its bold face's, replaced.
    fn sans_fonts(metrics: Option<Value>, bold: Option<Value>) -> Value {
        let mut fonts = to_json(&bound().presentation.fonts);
        if let Some(metrics) = metrics {
            fonts["liberationsans"]["binding"]["metrics"] = metrics;
        }
        if let Some(bold) = bold {
            fonts["liberationsans"]["binding"]["faces"] =
                json!([{ "weight": 700, "style": "normal", "metrics": bold }]);
        }
        fonts
    }

    #[test]
    fn rounds_the_core_text_line_to_whole_points_and_keeps_its_line_gap_out_of_the_multiple() {
        if golden().is_none() {
            return;
        }
        let sans = metrics_of(&bound().presentation, "liberationsans");
        let measure = |value: f64, size: f64, metrics: Option<Value>| {
            measure_grow(
                metrics.map(|m| sans_fonts(Some(m), None)),
                spaced(value, "coretext", &[(size, 400)]),
            )["props"]
                .clone()
        };
        // 18 pt Liberation Sans: ascent 16.29 → 16, descent 3.81 → 4, gap 0.59 → 1; 1.15 × (16 + 4) + 1 = 24 pt.
        assert_eq!(measure(1.15, 18.0, None)["lineHeight"], ratio(24.0 / 18.0));
        // CoreText adds a fifth of the line when the line gap is zero, the glyphs pass the ascent or descent and
        // À rises above the ascent; the spacing scales it with the rest: 1.15 × (16 + 4 + 4) = 27.6 pt.
        let synthesized = with(
            sans.clone(),
            json!({ "hhea": with(sans["hhea"].clone(), json!({ "lineGap": 0 })),
                    "bounds": { "yMin": -900, "yMax": 2000 }, "graveTop": 1900 }),
        );
        // The single line's 4 pt of leading stays above the text, and the multiple's extra 0.15 × 24 pt goes
        // below it, where the text's height leaves it out.
        let spaced_props = measure(1.15, 18.0, Some(synthesized.clone()));
        assert_eq!(
            spaced_props["leadingSkew"],
            fixed((4.0 - 0.15 * 24.0) / 18.0, 5)
        );
        assert_eq!(spaced_props["lineTailPt"], fixed(0.15 * 24.0, 3));
        let single = measure(1.0, 18.0, None);
        assert!(single.get("leadingSkew").is_none());
        assert!(single.get("lineTailPt").is_none());
        // A multiple under 1 keeps the 4 pt of leading above the text and takes 0.2 × 24 pt off below it, and the
        // text's height still ends at its last single line.
        let tight = measure(0.8, 18.0, Some(synthesized.clone()));
        assert_eq!(tight["leadingSkew"], fixed((4.0 + 0.2 * 24.0) / 18.0, 5));
        assert_eq!(tight["lineTailPt"], fixed(-0.2 * 24.0, 3));
        assert_eq!(spaced_props["lineHeight"], ratio(27.6 / 18.0));
        for plain in [
            with(synthesized.clone(), json!({ "graveTop": 1800 })),
            with(
                synthesized.clone(),
                json!({ "bounds": { "yMin": -434, "yMax": 1854 } }),
            ),
            with(synthesized.clone(), json!({ "hhea": sans["hhea"] })),
        ] {
            assert_ne!(
                measure(1.15, 18.0, Some(plain))["lineHeight"],
                ratio(27.6 / 18.0)
            );
        }
        // 47 pt: 42.55 → 43, 9.96 → 10, 1.54 → 2; a share of space is of the single line, 55 pt.
        let large = measure(1.0, 47.0, None);
        assert_eq!(large["lineHeight"], ratio(55.0 / 47.0));
        assert_eq!(large["spaceAfterPt"], fixed(0.5 * 55.0, 3));
    }

    #[test]
    fn leaves_the_core_text_line_gap_out_of_a_table_rows_text_not_a_shapes() {
        if golden().is_none() {
            return;
        }
        let bound = bound().presentation;
        let input = edited(&bound, |p| {
            let grow = element_mut(p, "ir-s3.grow");
            let mut first = grow["text"]["paragraphs"][0].clone();
            spaced(1.0, "coretext", &[(18.0, 400)])(&mut first);
            grow["text"]["paragraphs"] = json!([first]);
            let text = grow["text"].clone();
            let shape = grow.clone();
            let table = json!({
                "id": "ir-s3.table", "kind": "table", "origin": agent(), "frame": frame(0.0, 0.0, 200.0, 30.0),
                "columns": [{ "width": 200 }], "rows": [{ "height": 30, "cells": [{ "text": text }] }]
            });
            p["slides"][2]["elements"] = json!([shape, table]);
        });
        let out = apply_pass(&pass_named("text-metrics"), &input).presentation;
        let sans = metrics_of(&bound, "liberationsans");
        let gap = sans["hhea"]["lineGap"].as_f64().unwrap() / sans["unitsPerEm"].as_f64().unwrap();
        let elements = to_json(&out.slides[2].elements);
        assert_eq!(
            elements[1]["rows"][0]["cells"][0]["text"]["paragraphs"][0]["props"]["lineTailPt"],
            fixed(gap * 18.0, 3)
        );
        assert!(
            elements[0]["text"]["paragraphs"][0]["props"]
                .get("lineTailPt")
                .is_none()
        );
    }

    fn with_taller_bold() -> Value {
        let sans = metrics_of(&bound().presentation, "liberationsans");
        let taller = with(
            sans.clone(),
            json!({ "hhea": with(sans["hhea"].clone(), json!({ "ascender": 1854 + 205 })) }),
        );
        sans_fonts(None, Some(taller))
    }

    #[test]
    fn puts_a_size_1_2_baseline_at_0_8_of_the_single_line_and_a_multiples_extra_space_below() {
        if golden().is_none() {
            return;
        }
        let fonts = with_taller_bold();
        let measure = |value: f64, weights: &[u32]| {
            let runs: Vec<(f64, u32)> = weights.iter().map(|&w| (20.0, w)).collect();
            measure_grow(Some(fonts.clone()), spaced(value, "size-1.2", &runs))
        };
        // Liberation Sans: ascent 1854 and descent 434 of 2048. A 1.2 line puts 0.96 above the baseline.
        let skew = |above: f64, below: f64| fixed(above - below, 5);
        let single = measure(1.0, &[400]);
        assert_eq!(
            single["props"]["leadingSkew"],
            skew(0.96 - 1854.0 / 2048.0, 0.24 - 434.0 / 2048.0)
        );
        // At 1.15 the first baseline stays 0.96 down and the line's extra 0.18 goes below the text.
        let wide = measure(1.15, &[400]);
        assert_eq!(
            wide["props"]["lineHeight"],
            json!({ "kind": "ratio", "value": 1.38 })
        );
        assert_eq!(
            wide["props"]["leadingSkew"],
            skew(0.96 - 1854.0 / 2048.0, 1.38 - 0.96 - 434.0 / 2048.0)
        );
        // The text's height ends at the single line: 0.15 × 1.2 × 20 pt of the last line lies below it.
        assert_eq!(wide["props"]["lineTailPt"], json!(3.6));
        assert!(single["props"].get("lineTailPt").is_none());
        // A run in a taller face keeps the same line and baseline, so less of its leading sits above its text.
        let mixed = measure(1.0, &[400, 700]);
        let skews: Vec<Value> = mixed["inlines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["leadingSkew"].clone())
            .collect();
        assert_eq!(
            skews,
            [
                skew(0.96 - 1854.0 / 2048.0, 0.24 - 434.0 / 2048.0),
                skew(0.96 - 2059.0 / 2048.0, 0.24 - 434.0 / 2048.0)
            ]
        );
    }

    #[test]
    fn gives_a_run_in_a_taller_face_its_own_line_and_measures_the_strut_in_its_face() {
        if golden().is_none() {
            return;
        }
        let fonts = with_taller_bold();
        let measure = |weights: &[u32]| {
            let runs: Vec<(f64, u32)> = weights.iter().map(|&w| (20.0, w)).collect();
            measure_grow(Some(fonts.clone()), spaced(1.0, "coretext", &runs))
        };
        // 20 pt: regular 18 + 4 + 1 = 23 pt; the bold face's ascent is 0.1 em taller, 20 + 4 + 1 = 25 pt.
        let mixed = measure(&[400, 700]);
        assert_eq!(
            mixed["props"]["lineHeight"],
            json!({ "kind": "ratio", "value": 1.15 })
        );
        let own: Vec<Value> = mixed["inlines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["lineHeight"].clone())
            .collect();
        assert_eq!(
            own,
            [Value::Null, json!({ "kind": "ratio", "value": 1.25 })]
        );
        let bold = measure(&[700]);
        assert_eq!(
            bold["props"]["lineHeight"],
            json!({ "kind": "ratio", "value": 1.25 })
        );
        assert!(bold["inlines"][0].get("lineHeight").is_none());
    }

    #[test]
    fn scales_exact_spacing_and_clamps_the_others_against_the_struts_natural_height() {
        if golden().is_none() {
            return;
        }
        let p = measured();
        let pt = |value: f64| to_json(&json!({ "kind": "pt", "value": value }));
        assert_eq!(props(&p, "ir-s3.unfit", 0)["lineHeight"], pt(20.0));
        assert_eq!(props(&p, "ir-s3.grow", 0)["lineHeight"], pt(30.0));
        // 18 pt Liberation Sans: (1854 + 434 + 67) / 2048 × 18.
        assert_eq!(props(&p, "ir-s3.grow", 1)["lineHeight"], pt(20.698));
        assert_eq!(props(&p, "ir-s3.grow", 2)["lineHeight"], pt(12.0));
    }

    #[test]
    fn turns_spacing_into_points_as_written_or_a_share_of_the_struts_scaled_line() {
        if golden().is_none() {
            return;
        }
        let p = measured();
        let at_least = props(&p, "ir-s3.grow", 0);
        assert_eq!(
            (&at_least["spaceBeforePt"], &at_least["spaceAfterPt"]),
            (&json!(6), &json!(10.349))
        );
        // 20% of a 24 pt run shrunk to 80% on a 1.2 line.
        assert_eq!(props(&p, "ir-s3.body", 0)["spaceBeforePt"], json!(4.608));
    }

    #[test]
    fn follows_the_largest_run_and_the_paragraph_mark_when_there_is_none() {
        if golden().is_none() {
            return;
        }
        // A 24 pt DejaVu run in an 18 pt Liberation paragraph on the hhea basis: DejaVu's metrics set the line.
        let paragraph = measure_grow(None, |paragraph| {
            paragraph["props"]["lineSpacing"] =
                json!({ "kind": "multiple", "value": 1, "basis": "hhea" });
            let run = paragraph["inlines"][0].clone();
            let mut dejavu = run.clone();
            dejavu["props"]["size"] = json!(24);
            dejavu["props"]["font"] = json!({ "kind": "family", "family": "DejaVu Sans" });
            paragraph["inlines"] = json!([run, dejavu]);
        });
        let bound = bound().presentation;
        let dejavu = &bound.fonts["dejavusans"].binding.as_ref().unwrap().metrics;
        let hhea = slidock_passes::basis_ratio(slidock_ir::text::LineBasis::Hhea, dejavu).unwrap();
        assert_eq!(paragraph["props"]["lineHeight"], ratio(hhea));
        let empty = props(&measured(), "ir-s1.body", 4);
        assert_eq!(empty["spaceBeforePt"], fixed(0.2 * 14.0 * 1.2, 3));
    }
}

mod lower_geometry {
    use super::*;

    fn lowered() -> Presentation {
        through("geometry", "lower-geometry").presentation
    }

    fn paths(p: &Presentation, id: &str) -> Vec<Value> {
        el(p, id)["geometry"]["paths"].as_array().unwrap().clone()
    }

    #[test]
    fn lowers_presets_in_points_and_keeps_the_preset_name() {
        if golden().is_none() {
            return;
        }
        assert_eq!(
            el(&lowered(), "ir-g1.rect")["geometry"],
            json!({ "kind": "paths", "preset": "rect", "paths": [{
                "segments": [["M", 0, 0], ["L", 150, 0], ["L", 150, 80], ["L", 0, 80], ["Z"]],
                "fill": true, "stroke": true, "fillRule": "nonzero"
            }] })
        );
    }

    #[test]
    fn scales_custom_paths_from_their_own_units_to_the_frame() {
        if golden().is_none() {
            return;
        }
        let p = lowered();
        let paths = paths(&p, "ir-g2.custom");
        assert_eq!(paths[0]["segments"][0], json!(["M", 0, 120]));
        // The curve's tail arrowhead shortens its only segment, so only its start keeps the source's numbers.
        assert_eq!(paths[0]["segments"][1][0], "C");
        assert!(matches(
            &paths[1],
            &json!({ "fill": true, "stroke": false, "fillRule": "evenodd" })
        ));
        assert_eq!(
            paths[1]["segments"].as_array().unwrap()[..3],
            [
                json!(["M", 104, 48]),
                json!(["L", 156, 48]),
                json!(["Q", 182, 60, 156, 72])
            ]
        );
        assert!(el(&p, "ir-g2.custom")["geometry"].get("preset").is_none());
    }

    #[test]
    fn adds_arrowheads_as_line_end_paths_and_stops_the_line_under_a_body() {
        if golden().is_none() {
            return;
        }
        let paths = paths(&lowered(), "ir-g2.straight");
        // DrawingML's head is the start: a small oval centred there; the tail a medium triangle at the end.
        assert_eq!(paths[0]["segments"], json!([["M", 0, 0], ["L", 234, 0]]));
        assert!(matches(
            &paths[1],
            &json!({ "role": "line-end", "fill": true, "stroke": false })
        ));
        assert_eq!(paths[1]["segments"][0], json!(["M", -2, 0]));
        assert_eq!(
            paths[2],
            json!({ "segments": [["M", 240, 0], ["L", 234, 3], ["L", 234, -3], ["Z"]], "fill": true,
                    "stroke": false, "fillRule": "nonzero", "role": "line-end" })
        );
    }

    #[test]
    fn places_a_path_end_at_its_reach_scaled_with_the_stroke_and_draws_an_open_arrow_as_a_stroke() {
        if golden().is_none() {
            return;
        }
        let p = lowered();
        let keynote = paths(&p, "ir-g2.keynote");
        // A 6-unit reach grown by 0.4 + 0.6 × the 3-pt stroke cuts the line 13.2 pt from its tip, as the crow
        // flies, and the line goes on straight half a width (1.5 pt) into the end.
        assert_eq!(
            keynote[0]["segments"],
            json!([["M", 0, 0], ["L", 287.056, 57.411], ["L", 288.527, 57.705]])
        );
        assert!(
            keynote[1]["segments"]
                .as_array()
                .unwrap()
                .contains(&json!(["L", 300, 60]))
        );
        let arrow = paths(&p, "ir-g2.curved")
            .into_iter()
            .find(|path| path["role"] == "line-end" && path["stroke"] == true)
            .unwrap();
        assert_eq!(
            (&arrow["fill"], &arrow["stroke"]),
            (&json!(false), &json!(true))
        );
    }

    #[test]
    fn lowers_the_background_gradient_and_orients_text_without_mirroring_it() {
        if golden().is_none() {
            return;
        }
        let p = lowered();
        let background = to_json(&p.slides[2].background);
        assert_eq!(background["from"], json!({ "x": 0.5, "y": 0 }));
        assert_eq!(background["to"], json!({ "x": 0.5, "y": 1 }));
        let rotations: Vec<Value> = ["ir-g4.none", "ir-g4.flipH", "ir-g4.flipV", "ir-g4.both"]
            .iter()
            .map(|id| el(&p, id)["text"]["rotation"].clone())
            .collect();
        assert_eq!(rotations, [json!(0), json!(0), json!(180), json!(180)]);
    }

    #[test]
    fn lowers_a_picture_clip_like_a_geometry() {
        if golden().is_none() {
            return;
        }
        let full = source_fixture("full");
        let input = apply_passes(&passes_before("lower-geometry"), &full).presentation;
        let out = apply_pass(&pass_named("lower-geometry"), &input).presentation;
        assert!(matches(
            &el(&out, "ir-s1.photo")["clip"],
            &json!({ "kind": "paths", "preset": "ellipse" })
        ));
    }
}

mod plan_media {
    use super::*;

    #[test]
    fn decides_by_type_and_codec_reporting_each_loss_on_the_slides_that_show_it() {
        if golden().is_none() {
            return;
        }
        let media = lowered_fixture("media-timeline").presentation;
        let out = apply_pass(&pass_named("plan-media"), &media);
        let mut actions: Vec<String> = out
            .presentation
            .media
            .values()
            .map(|e| {
                let plan = to_json(&e.plan);
                [
                    Some(e.mime.as_str()),
                    e.codec.as_deref(),
                    plan["action"].as_str(),
                ]
                .into_iter()
                .flatten()
                .collect::<Vec<_>>()
                .join(" ")
            })
            .collect();
        actions.sort();
        assert_eq!(
            actions,
            [
                "audio/mpeg omit",
                "image/gif copy",
                "image/png copy",
                "image/png copy",
                "image/png copy",
                "image/svg+xml omit",
                "image/tiff transcode",
                "video/mp4 avc1 copy",
                "video/mp4 mp4v omit",
                "video/quicktime hvc1 copy",
                "video/webm copy",
            ]
        );
        let mut reported: Vec<Value> = out
            .diagnostics
            .iter()
            .map(|d| json!([d.code.code(), d.severity, d.container, d.feature]))
            .collect();
        reported.sort_by_key(|v| v.to_string());
        assert_eq!(
            reported,
            [
                json!(["MID5601", "decline", "ir-m1", "image.vector"]),
                json!(["MID5602", "info", "ir-m1", null]),
                json!(["MID5605", "lossy", "ir-m2", null]),
                json!(["MID5606", "info", "ir-m1", null]),
                json!(["MID5607", "info", "ir-m2", null]),
            ]
        );
        let tiff = out
            .presentation
            .media
            .values()
            .find(|e| e.mime.as_str() == "image/tiff")
            .unwrap();
        assert_eq!(
            to_json(&tiff.plan)["output"],
            json!({ "type": "image/png", "file": format!("media/{}.png", tiff.sha256) })
        );
    }
}

mod normalize_timeline {
    use super::*;

    #[test]
    fn drops_a_step_whose_target_is_gone_and_keeps_the_rest_in_order() {
        if golden().is_none() {
            return;
        }
        let media = lowered_fixture("media-timeline").presentation;
        let input = edited(&media, |p| {
            let timeline = p["slides"][1]["timeline"].as_array_mut().unwrap();
            let mut gone = timeline[0].clone();
            gone["target"] = json!({ "element": "ir-m2.gone" });
            timeline.insert(0, gone);
        });
        let out = apply_pass(&pass_named("normalize-timeline"), &input);
        let targets: Vec<Value> = out.presentation.slides[1]
            .timeline
            .iter()
            .map(|s| to_json(&s.target))
            .collect();
        assert_eq!(
            targets,
            [
                json!({ "element": "ir-m2.steps", "paragraphs": [0, 1] }),
                json!({ "element": "ir-m2.steps", "paragraphs": [2, 2] }),
                json!({ "element": "ir-m2.video" }),
                json!({ "element": "ir-m2.video" }),
            ]
        );
        let reported: Vec<Value> = out
            .diagnostics
            .iter()
            .map(|d| json!([d.code.code(), d.node, d.message]))
            .collect();
        assert_eq!(
            reported,
            [
                json!([
                    "MID5701",
                    null,
                    "Build step 0 changed: its target ir-m2.gone is not on the slide"
                ]),
                json!([
                    "MID5701",
                    "ir-m2.steps",
                    "Build step 2 changed: its target ir-m2.steps has fewer paragraphs"
                ]),
                json!([
                    "MID5701",
                    "ir-m2.video",
                    "Build step 4 changed: its target ir-m2.video has none of those paragraphs"
                ]),
            ]
        );
        assert_eq!(
            json!([out.diagnostics[0].severity, out.diagnostics[0].feature]),
            json!(["warning", "slide.timeline"])
        );
    }
}

mod a11y {
    use super::*;

    fn cascaded() -> Presentation {
        before("inheritance", "a11y")
    }

    #[test]
    fn titles_a_slide_by_its_title_placeholder_and_orders_reading_by_role_index_and_z_order() {
        if golden().is_none() {
            return;
        }
        let out = apply_pass(&pass_named("a11y"), &cascaded()).presentation;
        let titles: Vec<Option<&str>> = out.slides.iter().map(|s| s.title.as_deref()).collect();
        assert_eq!(
            titles,
            [
                Some("Inheritance"),
                Some("Dark layout"),
                Some("Autofit and fonts")
            ]
        );
        assert_eq!(
            to_json(&out.slides[0].reading_order),
            json!([
                "ir-s1.title",
                "ir-s1.body",
                "ir-s1.photo",
                "ir-s1.num",
                "ir-s1.hdr",
                "ir-s1.note"
            ])
        );
    }

    #[test]
    fn falls_back_to_the_largest_text_the_topmost_of_equals_collapsed_and_cut_to_200() {
        if golden().is_none() {
            return;
        }
        let cascaded = cascaded();
        let note = el(&cascaded, "ir-s1.note");
        let text = |id: &str, y: f64, size: f64, words: &str| {
            let mut shape = note.clone();
            shape["id"] = json!(id);
            shape["frame"]["y"] = json!(y);
            let mut paragraph = shape["text"]["paragraphs"][0].clone();
            let mut run = paragraph["inlines"][0].clone();
            run["text"] = json!(words);
            run["props"]["size"] = json!(size);
            paragraph["inlines"] = json!([run]);
            shape["text"]["paragraphs"] = json!([paragraph]);
            shape
        };
        let elements = vec![
            text("ir-s1.a", 300.0, 20.0, "small"),
            text(
                "ir-s1.b",
                200.0,
                40.0,
                &format!("  big   {}", "x".repeat(300)),
            ),
            text("ir-s1.c", 100.0, 40.0, "big and higher"),
        ];
        let titled = |elements: &[Value]| {
            let input = edited(&cascaded, |p| {
                let mut first = p["slides"][0].clone();
                first["elements"] = json!(elements);
                p["slides"] = json!([first]);
            });
            apply_pass(&pass_named("a11y"), &input).presentation.slides[0]
                .title
                .clone()
                .unwrap()
        };
        assert_eq!(titled(&elements), "big and higher");
        let long = titled(&elements[..2]);
        assert_eq!(long.encode_utf16().count(), 200);
        assert!(long.starts_with("big xxx"));
    }

    #[test]
    fn titles_a_slide_by_its_tables_text_too_the_largest_cells() {
        if golden().is_none() {
            return;
        }
        let out = apply_pass(&pass_named("a11y"), &before("table", "a11y"));
        assert_eq!(
            out.presentation.slides[0].title.as_deref(),
            Some("Over a shape")
        );
        assert!(!out.diagnostics.iter().any(|d| d.code.code() == "MID5801"));
    }

    #[test]
    fn gives_a_slide_without_text_an_empty_title_and_leaves_layouts_alone() {
        if golden().is_none() {
            return;
        }
        let out = apply_pass(&pass_named("a11y"), &before("media-timeline", "a11y"));
        let third = &out.presentation.slides[2];
        assert_eq!(third.title.as_deref(), Some(""));
        assert_eq!(third.reading_order.as_deref(), Some(&[][..]));
        let reported: Vec<Value> = out
            .diagnostics
            .iter()
            .map(|d| json!([d.code.code(), d.container]))
            .collect();
        assert_eq!(reported, [json!(["MID5801", "ir-m3"])]);
        assert!(
            to_json(&out.presentation.layouts[&id("ir-l.blank")])
                .get("title")
                .is_none()
        );
    }
}

mod properties {
    use super::*;

    fn text_skeleton(body: Option<&slidock_ir::text::TextBody>) -> Value {
        body.map_or(Value::Null, |body| {
            let paragraphs: Vec<Vec<&str>> = body
                .paragraphs
                .iter()
                .map(|p| p.inlines.iter().map(|i| i.kind()).collect())
                .collect();
            json!(paragraphs)
        })
    }

    /// What every pass keeps: every node's id, origin and ext, in order, the structure of every text body (the
    /// paragraphs and the kinds of their inlines) and every table's grid.
    fn skeleton(presentation: &Presentation) -> Value {
        let containers: Vec<Value> = presentation
            .containers()
            .map(|container| {
                let notes = match container {
                    slidock_ir::source::Container::Slide(slide) => {
                        text_skeleton(slide.notes.as_ref())
                    }
                    _ => Value::Null,
                };
                let elements: Vec<Value> = walk_elements(container.elements())
                    .map(|e| {
                        let texts: Vec<Value> = slidock_ir::verify::element_texts(e)
                            .into_iter()
                            .map(|(body, _)| text_skeleton(Some(body)))
                            .collect();
                        let grid = match e {
                            Element::Table(table) => {
                                let grid: Vec<Vec<bool>> = table
                                    .rows
                                    .iter()
                                    .map(|row| row.cells.iter().map(Option::is_some).collect())
                                    .collect();
                                json!(grid)
                            }
                            _ => Value::Null,
                        };
                        json!({ "id": e.id(), "kind": e.kind(), "origin": e.origin(), "ext": e.ext(),
                                "text": texts, "grid": grid })
                    })
                    .collect();
                json!({ "id": container.id(), "ext": container.ext(), "notes": notes, "elements": elements })
            })
            .collect();
        json!(containers)
    }

    /// The IR fixtures and the full source fixture, each with the presentation every pass receives.
    fn inputs() -> Vec<(String, Vec<Presentation>)> {
        let mut sources: Vec<(String, Presentation)> = ir_fixture_names()
            .into_iter()
            .map(|name| {
                let presentation = lowered_fixture(&name).presentation;
                (name, presentation)
            })
            .collect();
        sources.push(("presentation-ir full".to_owned(), source_fixture("full")));
        sources
            .into_iter()
            .map(|(name, presentation)| {
                let mut chain = vec![presentation];
                for pass in slidock_passes::passes() {
                    let next = apply_pass(&pass, chain.last().unwrap()).presentation;
                    chain.push(next);
                }
                (name, chain)
            })
            .collect()
    }

    #[test]
    fn every_pass_is_idempotent_keeps_the_skeleton_and_leaves_its_invariants_holding() {
        if golden().is_none() {
            return;
        }
        let inputs = inputs();
        let passes = slidock_passes::passes();
        let mut provided: Vec<Invariant> = Vec::new();
        for (index, pass) in passes.iter().enumerate() {
            provided.extend(pass.info().provides.iter().copied());
            for (name, chain) in &inputs {
                let once = &chain[index + 1];
                let twice = apply_pass(pass, once).presentation;
                assert_eq!(
                    slidock_ir::canonical::to_json(&twice).unwrap(),
                    slidock_ir::canonical::to_json(once).unwrap(),
                    "{} is not idempotent on {name}",
                    pass.name()
                );
                assert_eq!(
                    skeleton(once),
                    skeleton(&chain[index]),
                    "{} on {name}",
                    pass.name()
                );
                let errors: Vec<Diagnostic> =
                    verify_invariants(once, &provided, &CheckOptions::default())
                        .into_iter()
                        .filter(|d| d.severity == slidock_ir::diagnostics::Severity::Error)
                        .collect();
                assert!(errors.is_empty(), "{} on {name}: {errors:?}", pass.name());
            }
        }
    }
}

mod resolved_fixtures {
    use super::*;
    use slidock_core::VerifyMode;

    #[test]
    fn recomputes_the_resolved_values_an_agent_wrote_saying_what_it_dropped() {
        if golden().is_none() {
            return;
        }
        let result = compile_fixture("agent", VerifyMode::Each);
        let presentation = to_json(result.presentation.as_ref().unwrap());
        let shape = &presentation["slides"][0]["elements"][0];
        // The fixture claims accent1 is red and the line height is 9; the theme says 4472c4 and the font 1.2
        // less 0.2.
        assert!(matches(
            &shape["fill"]["color"]["resolved"],
            &json!({ "r": 0.26667, "g": 0.44706, "b": 0.76863 })
        ));
        let props = &shape["text"]["paragraphs"][0]["props"];
        assert_eq!(
            props["lineHeight"],
            json!({ "kind": "ratio", "value": 0.96 })
        );
        assert_eq!(props["spaceBeforePt"], 0);
        assert_eq!(
            shape["text"]["body"]["autofit"],
            json!({ "kind": "shrink", "fontScale": 0.9, "lineSpacingReduction": 0.2, "fitted": true })
        );
        assert_eq!(
            presentation["fonts"]["liberationsans"]["binding"]["family"],
            "Liberation Sans"
        );
        assert_eq!(
            presentation["slides"][0]["title"],
            "Agent-written values are recomputed"
        );
        let dropped: Vec<String> = result
            .diagnostics
            .iter()
            .filter(|d| d.code.code() == "IR0012")
            .map(|d| d.data.as_ref().unwrap()["field"].to_js_string())
            .collect();
        for field in [
            "colour resolved value",
            "lineHeight",
            "font binding",
            "slide title",
        ] {
            assert!(dropped.iter().any(|d| d == field), "{field} in {dropped:?}");
        }
    }

    fn strip_edges(elements: &mut Value) {
        for element in elements.as_array_mut().unwrap() {
            let object = element.as_object_mut().unwrap();
            object.shift_remove("inherits");
            if let Some(placeholder) = object.get_mut("placeholder") {
                placeholder.as_object_mut().unwrap().shift_remove("target");
            }
            if let Some(children) = object.get_mut("children") {
                strip_edges(children);
            }
        }
    }

    /// A resolved presentation without its inheritance edges and explicit placeholder targets, as source.
    fn without_edges(result: &slidock_core::CompileResult) -> Presentation {
        let mut value = to_json(result.presentation.as_ref().unwrap());
        value["phase"] = json!("source");
        for record in ["masters", "layouts"] {
            for container in value[record].as_object_mut().unwrap().values_mut() {
                strip_edges(&mut container["elements"]);
            }
        }
        for slide in value["slides"].as_array_mut().unwrap() {
            strip_edges(&mut slide["elements"]);
        }
        parse(value)
    }

    #[test]
    fn inheritance_is_provenance_only_the_other_passes_leave_a_resolved_fixture_without_edges_alone()
     {
        if golden().is_none() {
            return;
        }
        let passes: Vec<_> = slidock_passes::passes()
            .into_iter()
            .filter(|p| p.name() != "bind-placeholders")
            .collect();
        for name in ir_fixture_names() {
            let bare = without_edges(&compile_fixture(&name, VerifyMode::Each));
            let again = apply_passes(&passes, &bare).presentation;
            assert_eq!(
                slidock_ir::canonical::to_json(&again).unwrap(),
                slidock_ir::canonical::to_json(&bare).unwrap(),
                "{name}"
            );
        }
    }
}
