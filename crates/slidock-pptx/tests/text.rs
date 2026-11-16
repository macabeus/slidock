//! Text: body properties, paragraphs, runs, breaks, tabs, fields, links and bullets as declared, then through the
//! passes: placeholders cascade from the master's text styles, links take the theme's hyperlink colour.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_core_api::Limits;
use support::fixtures::text;
use support::{
    LayoutSpec, MasterSpec, PptxSpec, SlideSpec, Sp, assert_matches, body, compile_bytes, json,
    lower, lower_with, pptx, slide, sp, xfrm,
};

fn shape(lowered: &support::Lowered, id: &str) -> Value {
    let slide = &lowered.presentation().slides[0];
    json(
        slide
            .elements
            .iter()
            .find(|element| element.id().as_str() == id)
            .unwrap(),
    )
}

#[test]
fn reads_body_properties_as_declared() {
    let lowered = lower(&text());
    assert_eq!(
        shape(&lowered, "pp-256.2")["text"]["body"],
        json!({ "autofit": { "kind": "shrink", "fontScale": 0.9, "lineSpacingReduction": 0.1, "steps": "office" } })
    );
    assert_eq!(
        shape(&lowered, "pp-256.3")["text"]["body"],
        json!({
            "insets": { "l": 0 },
            "anchor": "middle",
            "autofit": { "kind": "resize", "width": false, "height": true, "anchorX": "left" },
        })
    );
    assert_eq!(
        shape(&lowered, "pp-256.10")["text"]["body"],
        json!({ "wrap": "none", "autofit": { "kind": "none" } })
    );
    let codes = lowered.codes("pp-256.10");
    for code in ["PX2201", "PX2202", "PX2203"] {
        assert!(codes.contains(&code.to_owned()), "{code} in {codes:?}");
    }
}

#[test]
fn a_shrink_stores_its_scale_when_either_attribute_is_written_and_none_when_both_are_left_out() {
    let shrinking = |id: u64, autofit: &str| {
        sp(Sp {
            id,
            sp_pr: xfrm(0.0, 0.0, 100.0, 20.0, ""),
            tx_body: Some(body(&["x"], &format!("<a:bodyPr>{autofit}</a:bodyPr>"))),
            ..Sp::default()
        })
    };
    let bytes = pptx(PptxSpec {
        slides: vec![slide(format!(
            "{}{}{}",
            shrinking(2, "<a:normAutofit/>"),
            shrinking(3, "<a:normAutofit lnSpcReduction=\"10000\"/>"),
            shrinking(4, "<a:normAutofit fontScale=\"62500\"/>")
        ))],
        ..PptxSpec::default()
    });
    let lowered = lower(&bytes);
    let autofit = |id: &str| shape(&lowered, id)["text"]["body"]["autofit"].clone();
    // PowerPoint writes the scale it fitted; a bare element leaves it to be fitted by PowerPoint's steps.
    assert_eq!(
        autofit("pp-256.2"),
        json!({ "kind": "shrink", "steps": "office" })
    );
    assert_eq!(
        autofit("pp-256.3"),
        json!({ "kind": "shrink", "fontScale": 1, "lineSpacingReduction": 0.1, "steps": "office" })
    );
    assert_eq!(
        autofit("pp-256.4"),
        json!({ "kind": "shrink", "fontScale": 0.625, "lineSpacingReduction": 0, "steps": "office" })
    );
}

#[test]
fn inherits_each_inset_side_the_slide_leaves_out_from_its_layout_and_master() {
    let placeholder = |id: u64, body_pr: &str, frame: String| {
        sp(Sp {
            id,
            ph: "<p:ph idx=\"1\"/>".to_owned(),
            sp_pr: frame,
            tx_body: Some(format!(
                "{body_pr}<a:lstStyle/><a:p><a:r><a:t>x</a:t></a:r></a:p>"
            )),
            ..Sp::default()
        })
    };
    let bytes = pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            tree: placeholder(
                2,
                "<a:bodyPr lIns=\"0\" tIns=\"0\" rIns=\"0\" bIns=\"0\"/>",
                xfrm(10.0, 10.0, 200.0, 100.0, ""),
            ),
            layouts: vec![LayoutSpec {
                tree: placeholder(2, "<a:bodyPr/>", String::new()),
                ..LayoutSpec::default()
            }],
            ..MasterSpec::default()
        }]),
        slides: vec![SlideSpec {
            layout: Some(0),
            tree: placeholder(3, "<a:bodyPr lIns=\"12700\"/>", String::new()),
            ..SlideSpec::default()
        }],
        ..PptxSpec::default()
    });
    let compiled = compile_bytes(&bytes, "presentation.pptx");
    let resolved = json(&compiled.presentation.unwrap());
    assert_eq!(
        resolved["slides"][0]["elements"][0]["text"]["body"]["insets"],
        json!({ "l": 1, "t": 0, "r": 0, "b": 0 })
    );
}

#[test]
fn fails_only_the_container_whose_text_body_passes_the_text_limit() {
    let text = |chars: &str| {
        sp(Sp {
            id: 2,
            sp_pr: xfrm(0.0, 0.0, 100.0, 20.0, ""),
            tx_body: Some(body(&[chars], "<a:bodyPr/>")),
            ..Sp::default()
        })
    };
    let bytes = pptx(PptxSpec {
        slides: vec![slide(text("eleven char")), slide(text("ten chars!"))],
        ..PptxSpec::default()
    });
    let limits = Limits {
        text_per_body: 10,
        ..Limits::default()
    };
    let lowered = lower_with(&bytes, None, &limits).unwrap();
    let failed: Vec<Value> = lowered
        .presentation()
        .slides
        .iter()
        .map(|slide| json(&slide.failed))
        .collect();
    assert_eq!(failed, [json!({ "code": "PX2901" }), json!(null)]);
}

#[test]
fn reads_run_properties_as_declared_an_invisible_underline_as_none() {
    let lowered = lower(&text());
    let inlines = shape(&lowered, "pp-256.10")["text"]["paragraphs"][0]["inlines"].clone();
    let props: Vec<Value> = inlines
        .as_array()
        .unwrap()
        .iter()
        .map(|inline| inline["props"].clone())
        .collect();
    assert_eq!(
        props,
        [
            json!({ "size": 24, "weight": 700 }),
            json!({ "italic": true }),
            json!({ "underline": "single" }),
            json!({ "underline": "single" }),
            json!({ "underline": "none" }),
            json!({ "strike": "single" }),
            json!({ "baseline": 0.3 }),
            json!({ "caps": "small", "letterSpacing": 1.5 }),
            json!({ "highlight": { "kind": "rgb", "space": "srgb", "r": 1, "g": 1, "b": 0, "a": 1 } }),
            json!({ "font": { "kind": "theme", "slot": "major" } }),
            json!({ "color": { "kind": "theme", "slot": "accent1", "mods": [] }, "font": { "kind": "family", "family": "Roboto Medium" } }),
            json!({ "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 1, "b": 0, "a": 1 } }),
        ]
    );
    let codes = lowered.codes("pp-256.10");
    for code in ["PX2207", "PX2210", "PX2211"] {
        assert!(codes.contains(&code.to_owned()), "{code} in {codes:?}");
    }
}

#[test]
fn reads_links_urls_in_the_hyperlink_colour_unless_kept_slide_jumps_and_show_actions() {
    let lowered = lower(&text());
    let inlines = shape(&lowered, "pp-256.11")["text"]["paragraphs"][0]["inlines"].clone();
    let links: Vec<(Value, Value)> = inlines
        .as_array()
        .unwrap()
        .iter()
        .map(|inline| {
            (
                inline["props"]["link"].clone(),
                inline["props"]["color"].clone(),
            )
        })
        .collect();
    let hlink = json!({ "kind": "theme", "slot": "hlink", "mods": [] });
    let mail = json!({ "kind": "url", "url": "mailto:team@example.com" });
    assert_eq!(links[0], (mail.clone(), hlink.clone()));
    assert_eq!(links[1].0, mail);
    assert_eq!(links[1].1["kind"], "rgb");
    assert_eq!(
        links[2],
        (json!({ "kind": "slide", "slide": "pp-257" }), hlink.clone())
    );
    assert_eq!(
        links[3],
        (json!({ "kind": "action", "action": "next" }), hlink)
    );
    assert_eq!(links[4], (Value::Null, Value::Null));
}

#[test]
fn reads_fields_breaks_and_tabs_an_unknown_field_is_its_stored_text() {
    let lowered = lower(&text());
    assert_eq!(
        shape(&lowered, "pp-256.11")["text"]["paragraphs"][1]["inlines"],
        json!([
            { "kind": "field", "field": "slideNumber", "cached": "3", "props": {} },
            { "kind": "tab" },
            { "kind": "run", "text": "of", "props": {} },
            { "kind": "tab" },
            { "kind": "field", "field": "date", "cached": "1/2/2026", "props": {} },
            { "kind": "break", "props": { "size": 9 } },
            { "kind": "field", "field": "time", "cached": "9:41 AM", "props": {} },
            { "kind": "run", "text": "Footer text", "props": {} },
        ])
    );
    assert_eq!(lowered.codes("pp-256.11"), ["PX2206"]);
}

#[test]
fn joins_bullet_properties_to_their_type() {
    let lowered = lower(&text());
    let paragraphs = shape(&lowered, "pp-256.3")["text"]["paragraphs"].clone();
    let bullets: Vec<Value> = paragraphs
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["props"]["bullet"].clone())
        .collect();
    assert_eq!(
        bullets,
        [
            json!({
                "kind": "char",
                "char": "§",
                "font": { "kind": "family", "family": "Wingdings" },
                "color": { "kind": "theme", "slot": "accent2", "mods": [] },
                "size": { "kind": "pct", "value": 0.8 },
            }),
            json!({ "kind": "auto", "scheme": "alphaLcParenR", "startAt": 3, "font": "text" }),
            json!({ "kind": "auto", "scheme": "arabicPeriod", "startAt": 1, "size": { "kind": "pt", "value": 14 } }),
            json!({ "kind": "char", "char": "•" }),
            Value::Null,
            Value::Null,
            Value::Null,
        ]
    );
    assert_eq!(paragraphs[1]["level"], 1);
    let mut codes = lowered.codes("pp-256.3");
    codes.sort();
    assert_eq!(codes, ["PX2204", "PX2205", "PX2208", "PX2209"]);
}

#[test]
fn reads_spacing_exact_line_spacing_in_points_spacing_before_as_a_share_of_the_line() {
    let lowered = lower(&text());
    assert_eq!(
        shape(&lowered, "pp-256.3")["text"]["paragraphs"][5]["props"],
        json!({
            "align": "justify",
            "rtl": true,
            "lineSpacing": { "kind": "exact", "value": 24 },
            "spaceBefore": { "kind": "pct", "value": 0.5 },
            "spaceAfter": { "kind": "pt", "value": 6 },
        })
    );
}

#[test]
fn applies_a_style_font_reference_only_where_the_list_style_is_silent() {
    let lowered = lower(&text());
    let list = shape(&lowered, "pp-256.12")["text"]["listStyle"].clone();
    assert_eq!(
        list["0"]["run"],
        json!({ "font": { "kind": "theme", "slot": "major" }, "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } })
    );
    assert_eq!(
        list["3"]["run"],
        json!({ "font": { "kind": "theme", "slot": "major" }, "color": { "kind": "theme", "slot": "lt1", "mods": [] } })
    );
}

#[test]
fn keeps_empty_paragraphs_and_the_size_of_their_paragraph_mark() {
    let lowered = lower(&text());
    let second = json(&lowered.presentation().slides[1].elements[0]);
    let paragraphs = second["text"]["paragraphs"].as_array().unwrap();
    assert_eq!(
        paragraphs[1..],
        [
            json!({ "level": 0, "props": {}, "inlines": [] }),
            json!({ "level": 0, "props": {}, "inlines": [], "endProps": { "size": 40 } }),
        ]
    );
}

#[test]
fn draws_a_raised_run_smaller_as_powerpoint_does() {
    let compiled = compile_bytes(&text(), "presentation.pptx");
    let presentation = json(&compiled.presentation.unwrap());
    let shape = presentation["slides"][0]["elements"]
        .as_array()
        .unwrap()
        .iter()
        .find(|element| element["id"] == "pp-256.10")
        .unwrap()
        .clone();
    let runs: Vec<Value> = shape["text"]["paragraphs"][0]["inlines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|inline| inline["kind"] == "run")
        .map(|inline| inline["props"].clone())
        .collect();
    let level = runs.iter().find(|props| props["italic"] == true).unwrap();
    let raised = runs.iter().find(|props| props["baseline"] == 0.3).unwrap();
    let expected =
        slidock_canon::js_round(level["size"].as_f64().unwrap() * 0.58 * 1000.0) / 1000.0;
    assert_eq!(raised["size"].as_f64().unwrap(), expected);
}

#[test]
fn cascades_placeholder_text_from_the_master_and_resolves_links_in_the_theme() {
    let compiled = compile_bytes(&text(), "presentation.pptx");
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
    let html = compiled.text("slides/pp-256.html").to_owned();
    let presentation = json(&compiled.presentation.unwrap());
    let element = |id: &str| {
        presentation["slides"][0]["elements"]
            .as_array()
            .unwrap()
            .iter()
            .find(|element| element["id"] == id)
            .unwrap()
            .clone()
    };
    let title = element("pp-256.2");
    assert_eq!(title["text"]["paragraphs"][0]["props"]["align"], "center");
    assert_matches(
        &title["text"]["paragraphs"][0]["inlines"][0],
        json!({ "props": { "size": 44, "weight": 700 } }),
    );
    assert_eq!(
        title["text"]["body"]["autofit"],
        json!({ "kind": "shrink", "fontScale": 0.9, "lineSpacingReduction": 0.1, "fitted": true })
    );
    let content = element("pp-256.3");
    assert_matches(
        &content["text"]["paragraphs"][0]["props"]["bullet"],
        json!({ "kind": "label", "text": "§" }),
    );
    assert_matches(
        &content["text"]["paragraphs"][1]["props"]["bullet"],
        json!({ "kind": "label", "text": "c)" }),
    );
    assert_matches(
        &content["text"]["paragraphs"][0]["inlines"][0],
        json!({ "props": { "size": 20 } }),
    );
    assert!(html.contains("href=\"mailto:team@example.com\""));
}
