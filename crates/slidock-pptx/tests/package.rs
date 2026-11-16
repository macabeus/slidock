//! The package and the containers: sniffing, refusals, the show order, masters with their themes and text styles,
//! layouts, slides, colour maps, backgrounds, defaults, and an unreadable part failing only its container.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use indexmap::IndexMap;
use serde_json::json;
use slidock_core_api::{Frontend, FrontendError, Limits, SniffHints};
use slidock_ir::features::FeatureId;
use slidock_pptx::PptxFrontend;
use support::fixtures::basic;
use support::{
    LayoutSpec, MasterSpec, PptxSpec, SlideSpec, assert_matches, compile_bytes, json, lower,
    lower_with, pptx, unzip, zip,
};

fn refusal_of(bytes: &[u8]) -> slidock_ir::diagnostics::Diagnostic {
    match lower_with(bytes, None, &Limits::default()) {
        Err(FrontendError::Refused(diagnostic)) => *diagnostic,
        Err(other) => panic!("not a refusal: {other}"),
        Ok(_) => panic!("not refused"),
    }
}

#[test]
fn sniffs_a_zip_holding_the_presentation_part() {
    let names = |list: &[&str]| {
        list.iter()
            .map(|name| (*name).to_owned())
            .collect::<Vec<_>>()
    };
    let hints = |zip_names| SniffHints {
        name: "x.pptx",
        zip_names,
    };
    let pptx_names = names(&["ppt/presentation.xml"]);
    let odp_names = names(&["content.xml"]);
    assert!(PptxFrontend.sniff(&[], &hints(Some(&pptx_names))));
    assert!(!PptxFrontend.sniff(&[], &hints(Some(&odp_names))));
    assert!(!PptxFrontend.sniff(&[], &hints(None)));
}

#[test]
fn refuses_what_is_not_a_presentation_package() {
    assert_eq!(refusal_of(b"not a zip at all, no").code.code(), "PX2001");
    let mut word = IndexMap::new();
    word.insert("word/document.xml".to_owned(), b"<w:document/>".to_vec());
    assert_eq!(refusal_of(&zip(&word, 0)).code.code(), "PX2001");
    let mut broken = IndexMap::new();
    broken.insert(
        "_rels/.rels".to_owned(),
        b"<Relationships xmlns=\"http://schemas.openxmlformats.org/package/2006/relationships\"/>"
            .to_vec(),
    );
    broken.insert(
        "ppt/presentation.xml".to_owned(),
        b"<p:presentation".to_vec(),
    );
    let refused = refusal_of(&zip(&broken, 0));
    assert!(
        refused.message.contains("ppt/presentation.xml"),
        "{}",
        refused.message
    );
}

#[test]
fn lists_slides_in_show_order_with_their_hidden_flags_and_lowers_the_same_list() {
    let lowered = lower(&basic());
    let analyzed: Vec<(&str, bool)> = lowered
        .analysis
        .slides
        .iter()
        .map(|slide| (slide.id.as_str(), slide.hidden))
        .collect();
    assert_eq!(
        analyzed,
        [("pp-256", false), ("pp-257", false), ("pp-300", true)]
    );
    let slides = &lowered.presentation().slides;
    let ids: Vec<(&str, bool)> = slides.iter().map(|s| (s.id.as_str(), s.hidden)).collect();
    assert_eq!(ids, analyzed);
    let origins: Vec<serde_json::Value> = slides.iter().map(|slide| json(&slide.origin)).collect();
    assert_eq!(
        origins,
        [
            json!({ "kind": "source", "part": "ppt/slides/slide1.xml" }),
            json!({ "kind": "source", "part": "ppt/slides/slide3.xml" }),
            json!({ "kind": "source", "part": "ppt/slides/slide2.xml" }),
        ]
    );
    assert_eq!(json(&lowered.analysis.size), json!({ "w": 720, "h": 405 }));
    assert_eq!(
        lowered.analysis.features,
        [FeatureId::FontEmbedded, FeatureId::PresentationHiddenSlide]
    );
}

#[test]
fn keys_masters_layouts_and_themes_by_their_native_ids() {
    let lowered = lower(&basic());
    let presentation = lowered.presentation();
    let keys = |ids: Vec<&str>| ids.into_iter().map(str::to_owned).collect::<Vec<_>>();
    assert_eq!(
        keys(presentation.masters.keys().map(|id| id.as_str()).collect()),
        ["pp-2147483648"]
    );
    assert_eq!(
        keys(presentation.layouts.keys().map(|id| id.as_str()).collect()),
        ["pp-2147483649", "pp-2147483650"]
    );
    assert_eq!(
        keys(presentation.themes.keys().map(|id| id.as_str()).collect()),
        ["pp-t.2147483648"]
    );
    let master = json(&presentation.masters[0]);
    assert_eq!(master["theme"], "pp-t.2147483648");
    assert_matches(
        &json(&presentation.layouts[1]),
        json!({ "master": "pp-2147483648", "showMaster": false, "type": "obj" }),
    );
    let layouts: Vec<serde_json::Value> = presentation
        .slides
        .iter()
        .map(|slide| json(&slide.layout))
        .collect();
    assert_eq!(
        layouts,
        [
            json!("pp-2147483649"),
            json!("pp-2147483650"),
            json!("pp-2147483650")
        ]
    );
}

#[test]
fn reads_the_theme_system_colours_by_their_last_value_fonts_by_slot() {
    let lowered = lower(&basic());
    let theme = json(&lowered.presentation().themes[0]);
    assert_eq!(
        theme["colors"]["dk1"],
        json!({ "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 })
    );
    assert_matches(
        &theme["colors"]["accent1"],
        json!({ "r": 0.26667, "g": 0.44706, "b": 0.76863 }),
    );
    assert_eq!(
        theme["fonts"],
        json!({ "major": { "latin": "Liberation Serif" }, "minor": { "latin": "Liberation Sans" } })
    );
}

#[test]
fn reads_colour_maps_whole_and_overrides_only_where_a_layout_or_slide_writes_one() {
    let lowered = lower(&basic());
    let presentation = lowered.presentation();
    let master = json(&presentation.masters[0]);
    assert_matches(
        &master["colorMap"],
        json!({ "bg1": "lt1", "tx1": "dk1", "dk1": "dk1", "accent1": "accent1" }),
    );
    assert_eq!(master["colorMap"].as_object().unwrap().len(), 16);
    assert_matches(
        &json(&presentation.layouts[1].color_map_override),
        json!({ "bg1": "dk1", "tx1": "lt1" }),
    );
    assert!(presentation.slides[1].color_map_override.is_none());
}

#[test]
fn reads_master_text_styles_per_level_and_the_presentation_defaults_over_ecma() {
    let lowered = lower(&basic());
    let presentation = lowered.presentation();
    let styles = json(&presentation.masters[0].text_styles);
    assert_eq!(
        styles["title"]["0"],
        json!({
            "para": { "align": "center" },
            "run": {
                "size": 44,
                "weight": 700,
                "color": { "kind": "theme", "slot": "tx1", "mods": [] },
                "font": { "kind": "theme", "slot": "major" },
            },
        })
    );
    assert_matches(
        &styles["body"]["0"]["para"],
        json!({
            "marginLeft": 18,
            "indent": -18,
            "lineSpacing": { "kind": "multiple", "value": 0.9, "basis": "size-1.2" },
            "spaceBefore": { "kind": "pt", "value": 10 },
            "bullet": { "kind": "char", "char": "•", "font": { "kind": "family", "family": "Arial" } },
        }),
    );
    assert_matches(
        &styles["body"]["1"]["para"]["bullet"],
        json!({ "char": "–" }),
    );
    assert_eq!(
        styles["body"]["4"]["para"],
        json!({ "lineSpacing": { "kind": "multiple", "value": 0.9, "basis": "size-1.2" } })
    );
    let defaults = json(&presentation.defaults);
    assert_matches(
        &defaults["text"]["0"]["para"],
        json!({ "align": "left", "tabSize": 36, "bullet": { "kind": "none" } }),
    );
    assert_matches(
        &defaults["text"]["0"]["run"],
        json!({ "size": 18, "font": { "kind": "theme", "slot": "minor" } }),
    );
    assert_eq!(defaults["text"]["3"]["run"]["size"], 18);
    assert_eq!(
        defaults["body"],
        json!({
            "insets": { "l": 7.2, "t": 3.6, "r": 7.2, "b": 3.6 },
            "anchor": "top",
            "wrap": "square",
            "autofit": { "kind": "none" },
            "overflow": "spread",
        })
    );
}

#[test]
fn reads_backgrounds_a_scheme_colour_and_a_gradient_in_drawingml_terms() {
    let lowered = lower(&basic());
    let presentation = lowered.presentation();
    assert_eq!(
        json(&presentation.masters[0].background),
        json!({ "kind": "solid", "color": { "kind": "theme", "slot": "bg1", "mods": [] } })
    );
    assert_eq!(
        json(&presentation.slides[1].background),
        json!({
            "kind": "linear",
            "angle": 45,
            "scaled": true,
            "rotateWithShape": true,
            "stops": [
                { "pos": 0, "color": { "kind": "rgb", "space": "srgb", "r": 1, "g": 0, "b": 0, "a": 1 } },
                { "pos": 1, "color": { "kind": "theme", "slot": "accent1", "mods": [{ "op": "lumMod", "value": 0.75 }] } },
            ],
        })
    );
    assert!(lowered.lowered.features["pp-257"].contains(&FeatureId::SlideBackground));
    assert!(!presentation.slides[1].show_layout);
}

#[test]
fn records_embedded_fonts_as_requests_never_unwrapped() {
    let lowered = lower(&basic());
    assert_eq!(
        json(&lowered.presentation().fonts["roboto"]),
        json!({
            "request": {
                "family": "Roboto",
                "faces": [{ "weight": 400, "style": "normal" }, { "weight": 700, "style": "normal" }],
                "postscriptNames": [],
                "embedded": true,
            },
        })
    );
    assert_eq!(
        lowered.lowered.features["presentation"],
        [FeatureId::FontEmbedded]
    );
    assert!(lowered.find("PX2501").is_some());
    assert!(lowered.lowered.fonts.is_empty());
}

#[test]
fn fails_only_the_container_whose_part_is_malformed_or_missing_keeping_its_place_id_and_links() {
    let bytes = pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            layouts: vec![LayoutSpec::default(), LayoutSpec::default()],
            ..MasterSpec::default()
        }]),
        slides: vec![
            SlideSpec {
                id: Some(256),
                ..SlideSpec::default()
            },
            SlideSpec {
                id: Some(257),
                ..SlideSpec::default()
            },
            SlideSpec {
                id: Some(258),
                layout: Some(1),
                ..SlideSpec::default()
            },
        ],
        ..PptxSpec::default()
    });
    let mut files = unzip(&bytes);
    files.insert("ppt/slides/slide2.xml".to_owned(), b"<p:sld".to_vec());
    files.shift_remove("ppt/slideLayouts/slideLayout2.xml");
    let lowered = lower(&zip(&files, 6));
    let ids: Vec<&str> = lowered
        .analysis
        .slides
        .iter()
        .map(|slide| slide.id.as_str())
        .collect();
    assert_eq!(ids, ["pp-256", "pp-257", "pp-258"]);
    let slides = json(&lowered.presentation().slides);
    assert!(slides[0].get("failed").is_none());
    assert_matches(
        &slides[1],
        json!({ "id": "pp-257", "layout": "pp-2147483649", "failed": { "code": "PX2002" }, "elements": [] }),
    );
    assert_matches(&slides[2], json!({ "layout": "pp-2147483650" }));
    assert_matches(
        &json(&lowered.presentation().layouts[1]),
        json!({ "failed": { "code": "PX2002" }, "master": "pp-2147483648" }),
    );
    let failed: Vec<&str> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "PX2002")
        .map(|d| d.container.as_ref().unwrap().as_str())
        .collect();
    assert_eq!(failed, ["pp-2147483650", "pp-257"]);
}

#[test]
fn hashes_the_whole_input_into_the_provenance() {
    let bytes = basic();
    let lowered = lower(&bytes);
    let provenance = &lowered.lowered.provenance;
    assert_eq!(provenance.format.as_str(), "pptx");
    assert_eq!(provenance.name, "presentation.pptx");
    assert_eq!(provenance.bytes, bytes.len() as u64);
    assert_eq!(
        provenance.sha256.to_hex(),
        slidock_canon::sha256_hex(&bytes)
    );
}

#[test]
fn compiles_to_a_lint_clean_file_set() {
    let compiled = compile_bytes(&basic(), "presentation.pptx");
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
    assert!(
        compiled
            .text("presentation.json")
            .contains("\"format\": \"pptx\"")
    );
}

#[test]
fn reads_a_part_its_list_names_again_once_in_its_first_place() {
    let spec = PptxSpec {
        slides: vec![SlideSpec::default(), SlideSpec::default()],
        ..PptxSpec::default()
    };
    let mut files = unzip(&pptx(spec));
    let part = "ppt/presentation.xml";
    let text = String::from_utf8(files[part].clone()).unwrap();
    let again = r#"<p:sldId id="400" r:id="rIdS1"/><p:sldId id="401" r:id="rIdS2"/><p:sldId id="402" r:id="rIdS1"/>"#;
    let text = text
        .replace("</p:sldIdLst>", &format!("{again}</p:sldIdLst>"))
        .replace(
            "</p:sldMasterIdLst>",
            r#"<p:sldMasterId id="2147483900" r:id="rIdM1"/></p:sldMasterIdLst>"#,
        );
    files.insert(part.to_owned(), text.into_bytes());
    let lowered = lower(&zip(&files, 0));
    let ids: Vec<&str> = lowered
        .analysis
        .slides
        .iter()
        .map(|slide| slide.id.as_str())
        .collect();
    assert_eq!(ids, ["pp-256", "pp-257"]);
    let presentation = lowered.presentation();
    let slides: Vec<&str> = presentation.slides.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(slides, ids);
    assert_eq!(presentation.masters.len(), 1);
    let relisted: Vec<(String, Option<u32>)> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "PX2013")
        .map(|d| (d.message.clone(), d.count))
        .collect();
    let again = |list: &str, part: &str, count: Option<u32>| {
        (
            format!("{list} names {part} again; the part is read once, in its first place"),
            count,
        )
    };
    assert_eq!(
        relisted,
        [
            again("sldIdLst", "ppt/slides/slide1.xml", Some(2)),
            again("sldIdLst", "ppt/slides/slide2.xml", None),
            again("sldMasterIdLst", "ppt/slideMasters/slideMaster1.xml", None),
        ]
    );
}

#[test]
fn lowers_only_the_containers_that_fit_the_output_budget() {
    use slidock_core_api::Output;
    use slidock_ir::source::Container;
    let whole = lower(&basic());
    let presentation = whole.presentation();
    let total = presentation
        .containers()
        .fold(Output::default(), |sum, c| sum.plus(&Output::of(c)));
    let last = presentation
        .slides
        .iter()
        .rev()
        .find(|slide| !slide.elements.is_empty())
        .unwrap();
    let mut limits = Limits::default();
    limits.output.elements = total.elements - 1;
    let lowered = lower_with(&basic(), None, &limits).unwrap();
    let over: Vec<(&str, &str)> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "IR0914")
        .map(|d| (d.container.as_ref().unwrap().as_str(), d.message.as_str()))
        .collect();
    let message = format!(
        "The container is left out: with what lowering made of it, the presentation's elements pass its budget of {}",
        total.elements - 1
    );
    assert_eq!(over, [(last.id.as_str(), message.as_str())]);
    let kept = lowered.presentation();
    for slide in &kept.slides {
        let original = presentation
            .slides
            .iter()
            .find(|s| s.id == slide.id)
            .unwrap();
        if slide.id == last.id {
            assert!(slide.failed.is_some() && slide.elements.is_empty());
        } else {
            assert_eq!(
                Output::of(Container::Slide(slide)),
                Output::of(Container::Slide(original))
            );
        }
    }
}
