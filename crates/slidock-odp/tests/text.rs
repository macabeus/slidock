//! Text: the cascade of shape, text and paragraph styles and spans, white space, fields and links, lists in both
//! indent models with their numbering, outline levels from the master page's outline styles, body properties, the
//! master's page number, footer and date copied onto a page that shows them, and speaker notes.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use support::{Lowered, assert_holds, compile, fixtures, js, lower};

fn text() -> Lowered {
    lower(&fixtures::text())
}

fn paragraphs(lowered: &Lowered, slide: usize, index: usize) -> Vec<Value> {
    lowered.elements(slide)[index]["text"]["paragraphs"]
        .as_array()
        .unwrap()
        .clone()
}

fn kinds(paragraph: &Value) -> Vec<String> {
    paragraph["inlines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|inline| match inline["kind"].as_str().unwrap() {
            "run" => inline["text"].as_str().unwrap().to_owned(),
            kind => kind.to_owned(),
        })
        .collect()
}

#[test]
fn collapses_white_space_keeps_counted_spaces_and_splits_runs_where_their_styles_change() {
    let lowered = text();
    let first = &paragraphs(&lowered, 1, 0)[0];
    assert_eq!(
        kinds(first),
        [
            "Plain text with ",
            "bold italic",
            " and ",
            "struck   raised"
        ]
    );
    assert_holds(
        &first["inlines"][1],
        json!({ "props": { "weight": 700, "italic": true, "underline": "double", "size": 30 } }),
    );
    // `super 58%`: raised a third of the size, drawn at 58% of it.
    assert_holds(
        &first["inlines"][3],
        json!({ "props": { "strike": "single", "baseline": 0.33, "scriptScale": 0.58, "letterSpacing": 1.417 } }),
    );
    assert_eq!(
        first["props"],
        json!({ "align": "right", "marginLeft": 36, "indent": -18, "spaceAfter": { "kind": "pt", "value": 6 } })
    );
    assert_eq!(first["endProps"], json!({ "size": 30 }));
}

#[test]
fn draws_a_raised_run_at_the_size_its_text_position_states() {
    let out = compile(&fixtures::text());
    let slide = &out.documents.unwrap().text["slides/od-p2.html"];
    let run = slide.find(">struck   raised<").unwrap();
    let span = slide[..run].rfind("<span style=\"").unwrap();
    let span_style = &slide[span..run];
    // The smallest run sizes its paragraph, and inherits that size.
    assert!(!span_style.contains("font-size"), "{span_style}");
    let paragraph = slide[..span].rfind("<p ").unwrap();
    let paragraph_tag = &slide[paragraph..slide[paragraph..].find('>').unwrap() + paragraph];
    assert!(
        paragraph_tag.contains("font-size:17.4pt"),
        "{paragraph_tag}"
    );
}

#[test]
fn reads_fonts_by_face_or_family_relative_sizes_highlights_case_and_the_automatic_colour() {
    let lowered = text();
    let second = &paragraphs(&lowered, 1, 0)[1];
    let kinds: Vec<&str> = second["inlines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["kind"].as_str().unwrap())
        .collect();
    assert_eq!(kinds, ["run", "tab", "run", "break", "run"]);
    assert_holds(
        &second["inlines"][0],
        json!({ "props": { "font": { "family": "Barlow Condensed" }, "caps": "small", "highlight": { "r": 1, "g": 1, "b": 0 } } }),
    );
    assert_holds(
        &second["inlines"][2],
        json!({ "props": { "font": { "family": "Roboto Medium" }, "caps": "all", "size": 10 } }),
    );
    assert_holds(
        &second["inlines"][4],
        json!({ "props": { "color": { "r": 0, "g": 0, "b": 0 }, "underline": "single" } }),
    );
    assert_eq!(
        second["props"],
        json!({ "lineSpacing": { "kind": "exact", "value": 36 }, "rtl": true })
    );
}

#[test]
fn reads_fields_links_and_line_spacing_and_reports_what_it_cannot_model() {
    let lowered = text();
    let all = paragraphs(&lowered, 1, 0);
    let (third, empty) = (&all[2], &all[3]);
    let fields: Vec<&Value> = third["inlines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "field")
        .collect();
    assert_eq!(
        fields,
        [
            &json!({ "kind": "field", "field": "slideNumber", "cached": "4", "props": {} }),
            &json!({ "kind": "field", "field": "slideCount", "cached": "9", "props": {} }),
            &json!({ "kind": "field", "field": "date", "cached": "13/10/2026", "props": {} }),
        ]
    );
    let links: Vec<&Value> = third["inlines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "run")
        .filter_map(|i| i["props"].get("link"))
        .collect();
    assert_eq!(
        links,
        [
            &json!({ "kind": "url", "url": "https://example.com" }),
            &json!({ "kind": "slide", "slide": "od-p1" })
        ]
    );
    assert_eq!(
        third["props"]["lineSpacing"],
        json!({ "kind": "atLeast", "value": 20 })
    );
    assert_holds(
        empty,
        json!({ "inlines": [], "props": { "align": "justify" }, "endProps": { "size": 20 } }),
    );
    let codes: Vec<&str> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.node.as_ref().is_some_and(|n| n.as_str() == "od-p2.1"))
        .map(|d| d.code.code())
        .collect();
    assert_eq!(
        codes,
        [
            "OD9201", "OD9202", "OD9204", "OD9205", "OD9206", "OD9207", "OD9210"
        ]
    );
}

#[test]
fn gives_the_body_its_shapes_complete_style_per_level_used() {
    let lowered = text();
    let text = &lowered.elements(1)[0]["text"];
    let levels: Vec<&String> = text["listStyle"].as_object().unwrap().keys().collect();
    assert_eq!(levels, ["0"]);
    assert_holds(
        &text["listStyle"]["0"]["run"],
        json!({ "size": 20, "color": { "r": 0.2, "g": 0.4, "b": 0.6 } }),
    );
    assert_eq!(
        text["body"],
        js(&json!({
            "insets": { "l": 7.2, "t": 7.2, "r": 7.2, "b": 7.2 },
            "anchor": "bottom",
            "wrap": "none",
            "autofit": { "kind": "resize", "width": false, "height": true, "anchorX": "left" }
        }))
    );
}

#[test]
fn labels_list_items_by_their_level_style_in_both_indent_models() {
    let lowered = text();
    let all = paragraphs(&lowered, 1, 1);
    let rows: Vec<Value> = all
        .iter()
        .map(|p| {
            json!([
                p["level"],
                p["props"]["bullet"]["kind"],
                p["props"]["marginLeft"],
                p["props"]["indent"]
            ])
        })
        .collect();
    assert_eq!(
        rows,
        [
            json!([0, "auto", 36, -18]),
            json!([0, "auto", 36, -18]),
            json!([0, "none", 36, 0]),
            json!([1, "auto", 54, -18]),
            json!([2, "char", 18, -18]),
            json!([0, "auto", 36, -18]),
            json!([0, "none", 36, 0]),
        ]
    );
    assert_eq!(
        all[0]["props"]["bullet"],
        json!({
            "kind": "auto",
            "scheme": "alphaLcParenBoth",
            "startAt": 3,
            "font": "text",
            "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 1, "b": 0, "a": 1 },
            "size": { "kind": "text" }
        })
    );
    assert_eq!(all[5]["props"]["bullet"]["startAt"], 7);
    assert_eq!(all[3]["props"]["bullet"]["scheme"], "romanUcPeriod");
}

#[test]
fn styles_deeper_outline_levels_from_the_master_pages_outline_styles_with_its_list_style() {
    let lowered = text();
    let text = &lowered.elements(0)[1]["text"];
    assert_eq!(text["listStyle"]["1"]["run"]["size"], js(&json!(22.4)));
    assert_eq!(
        text["listStyle"]["1"]["para"]["spaceBefore"],
        js(&json!({ "kind": "pt", "value": 11.339 }))
    );
    let bullets: Vec<Value> = text["paragraphs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["props"].get("bullet").cloned().unwrap_or_default())
        .collect();
    assert_eq!(
        bullets[0],
        js(&json!({
            "kind": "char",
            "char": "●",
            "font": { "kind": "family", "family": "StarSymbol" },
            "color": "text",
            "size": { "kind": "pct", "value": 0.45 }
        }))
    );
    assert_holds(
        &bullets[1],
        json!({ "char": "–", "size": { "kind": "pt", "value": 12 } }),
    );
    assert_eq!(bullets[2], Value::Null);
}

#[test]
fn copies_the_masters_page_number_footer_and_date_onto_a_page_that_shows_them() {
    let lowered = text();
    let copies = &lowered.elements(0)[2..];
    let rows: Vec<Value> = copies
        .iter()
        .map(|e| json!([e["id"], e["placeholder"]["role"], e["origin"]["part"]]))
        .collect();
    assert_eq!(
        rows,
        [
            json!(["od-p1.m.2", "sldNum", "styles.xml"]),
            json!(["od-p1.m.3", "ftr", "styles.xml"]),
            json!(["od-p1.m.4", "dt", "styles.xml"]),
        ]
    );
    let texts: Vec<&Value> = copies
        .iter()
        .map(|e| &e["text"]["paragraphs"][0]["inlines"][0])
        .collect();
    assert_eq!(
        texts,
        [
            &json!({ "kind": "field", "field": "slideNumber", "cached": "<number>", "props": { "size": 10 } }),
            &json!({ "kind": "run", "text": "Company", "props": { "size": 10 } }),
            &json!({ "kind": "run", "text": "13 October", "props": { "size": 10 } }),
        ]
    );
    // The second page hides them, and declares no date to show.
    assert!(
        !lowered
            .elements(1)
            .iter()
            .any(|e| e["id"].as_str().unwrap().contains(".m."))
    );
}

#[test]
fn reads_speaker_notes_and_leaves_out_notes_without_text() {
    let lowered = text();
    assert_eq!(
        lowered.json["slides"][0]["notes"],
        json!({
            "body": {},
            "paragraphs": [{ "level": 0, "props": {}, "inlines": [{ "kind": "run", "text": "Say hello", "props": {} }] }]
        })
    );
    assert!(lowered.json["slides"][1].get("notes").is_none());
    assert!(lowered.features("od-p1").contains(&"slide.notes"));
}

#[test]
fn wraps_shapes_and_frames_at_their_width_custom_shapes_as_their_wrap_option_says_and_shrinks_unfitted()
 {
    let lowered = text();
    assert_holds(
        &lowered.elements(1)[2]["text"]["body"],
        json!({ "wrap": "square", "autofit": { "kind": "shrink", "steps": "office" } }),
    );
    assert_holds(
        &lowered.elements(1)[1]["text"]["body"],
        json!({ "wrap": "none", "autofit": { "kind": "resize", "width": true, "height": false, "anchorX": "center" } }),
    );
    assert!(lowered.elements(1)[4].get("text").is_none());
}

/// A percentage over 100 can leave the ratio grid (`66.67%` is 0.6667000000000001), which would fail the container
/// (IR0007); every percentage rounds to the grid: line spacing, a bullet's relative size and a raised run's baseline.
#[test]
fn rounds_percentages_to_the_ratio_grid() {
    use support::odp::{OdpSpec, PageSpec, odp};
    let bytes = odp(&OdpSpec {
        automatic_styles: concat!(
            r#"<style:style style:name="P1" style:family="paragraph"><style:paragraph-properties "#,
            r#"fo:line-height="66.67%"/></style:style>"#,
            r#"<style:style style:name="T1" style:family="text"><style:text-properties "#,
            r#"style:text-position="33.3% 58%"/></style:style>"#,
            r#"<text:list-style style:name="L1"><text:list-level-style-bullet text:level="1" text:bullet-char="●" "#,
            r#"text:bullet-relative-size="99.99%"/></text:list-style>"#,
        )
        .to_owned(),
        ..OdpSpec::pages(vec![PageSpec::body(concat!(
            r#"<draw:frame draw:name="F" svg:x="1cm" svg:y="1cm" svg:width="8cm" svg:height="4cm"><draw:text-box>"#,
            r#"<text:list text:style-name="L1"><text:list-item><text:p text:style-name="P1">"#,
            r#"<text:span text:style-name="T1">up</text:span></text:p></text:list-item></text:list>"#,
            "</draw:text-box></draw:frame>"
        ))])
    });
    let lowered = lower(&bytes);
    assert_eq!(lowered.codes(), []);
    let paragraph = &lowered.named(0, "F")["text"]["paragraphs"][0];
    assert_holds(
        &paragraph["props"],
        json!({
            "lineSpacing": { "kind": "multiple", "value": 0.6667 },
            "bullet": { "size": { "kind": "pct", "value": 0.9999 } }
        }),
    );
    assert_holds(
        &paragraph["inlines"][0],
        json!({ "props": { "baseline": 0.333, "scriptScale": 0.58 } }),
    );
}
