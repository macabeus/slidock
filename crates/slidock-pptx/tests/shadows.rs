//! Shadows: `a:outerShdw` lowered to the IR's shadow, the precedence of own effects over the style's effect
//! reference, `phClr`, groups, pictures, movies, runs and list styles, and what is reported instead of drawn (PX2111).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_ir::features::FeatureId;
use support::fixtures::{Pic, blip, movie, pic, png};
use support::{
    DEFAULT_THEME, LayoutSpec, Lowered, MasterSpec, PptxSpec, SlideSpec, Sp, compile_bytes,
    element, json, lower, pptx, rel, slide, sp, xfrm,
};

const RECT: &str = "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>";
const WHITE: &str = "<a:solidFill><a:srgbClr val=\"FFFFFF\"/></a:solidFill>";

/// An outer shadow with these attributes, black at 50 %.
fn outer(attrs: &str) -> String {
    format!(
        "<a:outerShdw {attrs}><a:srgbClr val=\"000000\"><a:alpha val=\"50000\"/></a:srgbClr></a:outerShdw>"
    )
}

/// The probes' default shadow: 20 pt at 45°, no blur, page-fixed.
fn base() -> String {
    outer("dist=\"254000\" dir=\"2700000\" rotWithShape=\"0\" algn=\"b\"")
}

fn effects(list: &str) -> String {
    format!("<a:effectLst>{list}</a:effectLst>")
}

/// A white rectangle with these effects (inside `a:spPr`) and this shape style.
fn rect(id: u64, effects: &str, style: Option<&str>) -> String {
    sp(Sp {
        id,
        sp_pr: format!(
            "{}{RECT}{WHITE}{effects}",
            xfrm(100.0, 100.0, 240.0, 160.0, "")
        ),
        style: style.map(str::to_owned),
        ..Sp::default()
    })
}

/// A style that references effect style `idx` in `color`.
fn effect_ref(idx: u32, color: &str) -> String {
    format!(
        "<a:lnRef idx=\"0\"><a:srgbClr val=\"000000\"/></a:lnRef><a:fillRef idx=\"0\"><a:srgbClr val=\"FFFFFF\"/></a:fillRef><a:effectRef idx=\"{idx}\">{color}</a:effectRef><a:fontRef idx=\"minor\"/>"
    )
}

/// The default theme with these effect styles.
fn theme(effect_styles: &str) -> String {
    let start = DEFAULT_THEME.find("<a:effectStyleLst>").unwrap();
    let end = DEFAULT_THEME.find("</a:effectStyleLst>").unwrap() + "</a:effectStyleLst>".len();
    format!(
        "{}<a:effectStyleLst>{effect_styles}</a:effectStyleLst>{}",
        &DEFAULT_THEME[..start],
        &DEFAULT_THEME[end..]
    )
}

/// Effect styles: 1 nothing, 2 a black shadow, 3 a `phClr` shadow with a bevel.
fn effect_styles() -> String {
    format!(
        "<a:effectStyle><a:effectLst/></a:effectStyle><a:effectStyle>{}</a:effectStyle><a:effectStyle>{}<a:sp3d><a:bevelT w=\"63500\" h=\"25400\"/></a:sp3d></a:effectStyle>",
        effects(&outer(
            "blurRad=\"127000\" dist=\"38100\" dir=\"5400000\" rotWithShape=\"0\""
        )),
        effects(
            "<a:outerShdw blurRad=\"254000\" dist=\"0\"><a:schemeClr val=\"phClr\"><a:alpha val=\"40000\"/></a:schemeClr></a:outerShdw>"
        ),
    )
}

/// One slide holding `tree`, under the default master with the effect styles above.
fn deck(tree: String) -> Vec<u8> {
    pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            theme: Some(theme(&effect_styles())),
            layouts: vec![LayoutSpec::default()],
            ..MasterSpec::default()
        }]),
        slides: vec![slide(tree)],
        ..PptxSpec::default()
    })
}

fn shadow_of(lowered: &Lowered, id: &str) -> Value {
    let slide = &lowered.presentation().slides[0];
    let element = element(slide, id);
    assert!(!element.is_null(), "{id} is lowered");
    element.get("shadow").cloned().unwrap_or(json!("absent"))
}

/// The reasons of the PX2111 reports on `node`, in output order.
fn not_drawn(lowered: &Lowered, node: &str) -> Vec<String> {
    lowered
        .diagnostics
        .iter()
        .filter(|d| {
            d.code.code() == "PX2111" && d.node.as_ref().is_some_and(|n| n.as_str() == node)
        })
        .map(|d| json(&d.data)["reason"].as_str().unwrap().to_owned())
        .collect()
}

fn black(a: f64) -> Value {
    json!({ "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": a })
}

#[test]
fn lowers_an_outer_shadow_offset_blur_colour_and_rotation_with_the_shape() {
    let tree = [
        rect(10, &effects(&base()), None),
        // dir 300° at 3.287 pt, blur 14 pt; rotWithShape defaults to true.
        rect(
            11,
            &effects(&outer("blurRad=\"177800\" dist=\"41745\" dir=\"18000000\"")),
            None,
        ),
        // Every attribute at its default: no offset, no blur.
        rect(12, &effects(&outer("")), None),
        // A theme colour stays symbolic, its modifiers kept for resolve-theme.
        rect(
            13,
            &effects(
                "<a:outerShdw dist=\"127000\" dir=\"5400000\" rotWithShape=\"1\"><a:schemeClr val=\"accent1\"><a:alpha val=\"25000\"/></a:schemeClr></a:outerShdw>",
            ),
            None,
        ),
    ]
    .concat();
    let lowered = lower(&deck(tree));
    assert_eq!(
        shadow_of(&lowered, "pp-256.10"),
        json!({ "dx": 14.142, "dy": 14.142, "sigma": 0, "color": black(0.5) })
    );
    assert_eq!(
        shadow_of(&lowered, "pp-256.11"),
        json!({ "dx": 1.644, "dy": -2.847, "sigma": 6.02, "color": black(0.5), "rotateWithShape": true })
    );
    assert_eq!(
        shadow_of(&lowered, "pp-256.12"),
        json!({ "dx": 0, "dy": 0, "sigma": 0, "color": black(0.5), "rotateWithShape": true })
    );
    assert_eq!(
        shadow_of(&lowered, "pp-256.13"),
        json!({
            "dx": 0, "dy": 10, "sigma": 0, "rotateWithShape": true,
            "color": { "kind": "theme", "slot": "accent1", "mods": [{ "op": "alpha", "value": 0.25 }] },
        })
    );
    assert!(lowered.lowered.features["pp-256"].contains(&FeatureId::EffectShadow));
    assert!(lowered.find("PX2111").is_none() && lowered.find("PX2101").is_none());
}

#[test]
fn own_effects_replace_the_style_reference_and_an_empty_list_means_none() {
    let tree = [
        // The theme's black shadow, by reference.
        rect(10, "", Some(&effect_ref(2, "<a:srgbClr val=\"000000\"/>"))),
        // An empty own list beats the reference.
        rect(
            11,
            "<a:effectLst/>",
            Some(&effect_ref(2, "<a:srgbClr val=\"000000\"/>")),
        ),
        // An own shadow beats the reference's.
        rect(
            12,
            &effects(&outer("dist=\"127000\"")),
            Some(&effect_ref(2, "<a:srgbClr val=\"000000\"/>")),
        ),
        // Index 0 references no effect style: no shadow.
        rect(13, "", Some(&effect_ref(0, "<a:srgbClr val=\"000000\"/>"))),
        // A style without a shadow states none; no effects at all state nothing (the placeholder's apply).
        rect(14, "", Some(&effect_ref(1, "<a:srgbClr val=\"000000\"/>"))),
        rect(15, "", None),
    ]
    .concat();
    let lowered = lower(&deck(tree));
    assert_eq!(
        shadow_of(&lowered, "pp-256.10"),
        json!({ "dx": 0, "dy": 3, "sigma": 4.3, "color": black(0.5) })
    );
    assert_eq!(shadow_of(&lowered, "pp-256.11"), json!(null));
    assert_eq!(shadow_of(&lowered, "pp-256.12")["dx"], json!(10));
    assert_eq!(shadow_of(&lowered, "pp-256.13"), json!(null));
    assert_eq!(shadow_of(&lowered, "pp-256.14"), json!(null));
    assert_eq!(shadow_of(&lowered, "pp-256.15"), json!("absent"));
}

#[test]
fn a_referenced_phclr_shadow_takes_the_reference_colour_and_its_bevel_is_still_dropped() {
    let tree = [
        rect(10, "", Some(&effect_ref(3, "<a:srgbClr val=\"0000FF\"/>"))),
        rect(
            11,
            "",
            Some(&effect_ref(3, "<a:schemeClr val=\"accent2\"/>")),
        ),
        // An own list replaces the style's effects, not its 3D properties.
        rect(
            12,
            "<a:effectLst/>",
            Some(&effect_ref(3, "<a:schemeClr val=\"accent2\"/>")),
        ),
    ]
    .concat();
    let lowered = lower(&deck(tree));
    assert_eq!(
        shadow_of(&lowered, "pp-256.10"),
        json!({
            "dx": 0, "dy": 0, "sigma": 8.6, "rotateWithShape": true,
            "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 1, "a": 0.4 },
        })
    );
    assert_eq!(
        shadow_of(&lowered, "pp-256.11")["color"],
        json!({ "kind": "theme", "slot": "accent2", "mods": [{ "op": "alpha", "value": 0.4 }] })
    );
    assert_eq!(shadow_of(&lowered, "pp-256.12"), json!(null));
    for node in ["pp-256.10", "pp-256.11", "pp-256.12"] {
        assert_eq!(lowered.codes(node), ["PX2101"], "{node}");
    }
    assert_eq!(
        lowered.find("PX2101").unwrap().feature,
        Some(FeatureId::Effect3d)
    );
}

#[test]
fn reports_the_shadows_it_does_not_draw_and_drops_other_effects_as_before() {
    let tree = [
        rect(
            10,
            &effects("<a:innerShdw blurRad=\"63500\" dist=\"127000\"><a:srgbClr val=\"000000\"/></a:innerShdw>"),
            None,
        ),
        rect(
            11,
            &effects("<a:prstShdw prst=\"shdw13\" dist=\"254000\"><a:srgbClr val=\"000000\"/></a:prstShdw>"),
            None,
        ),
        // The outer shadow of an inner and outer pair is drawn.
        rect(
            12,
            &effects(&format!(
                "<a:innerShdw dist=\"127000\"><a:srgbClr val=\"000000\"/></a:innerShdw>{}",
                base()
            )),
            None,
        ),
        rect(13, &effects(&outer("sx=\"50000\" sy=\"50000\" algn=\"tl\" dist=\"254000\"")), None),
        rect(14, &effects(&outer("kx=\"1200000\" dist=\"254000\"")), None),
        rect(15, &format!("<a:effectDag type=\"sib\">{}</a:effectDag>", base()), None),
        // A glow beside a drawn shadow is still dropped, and only the glow.
        rect(
            16,
            &effects(&format!(
                "<a:glow rad=\"63500\"><a:srgbClr val=\"FFFF00\"/></a:glow>{}",
                base()
            )),
            None,
        ),
        rect(17, &effects("<a:outerShdw dist=\"127000\"/>"), None),
    ]
    .concat();
    let lowered = lower(&deck(tree));
    let cases = [
        ("pp-256.10", "inner shadow"),
        ("pp-256.11", "preset shadow shdw13"),
        ("pp-256.12", "inner shadow"),
        ("pp-256.13", "scaled or skewed shadow"),
        ("pp-256.14", "scaled or skewed shadow"),
        ("pp-256.15", "effect DAG"),
        ("pp-256.17", "shadow without a colour"),
    ];
    for (node, reason) in cases {
        assert_eq!(not_drawn(&lowered, node), [reason], "{node}");
        assert_eq!(lowered.codes(node), ["PX2111"], "{node}");
    }
    for node in [
        "pp-256.10",
        "pp-256.11",
        "pp-256.13",
        "pp-256.14",
        "pp-256.15",
        "pp-256.17",
    ] {
        assert_eq!(shadow_of(&lowered, node), json!(null), "{node}");
    }
    assert_eq!(shadow_of(&lowered, "pp-256.12")["dx"], json!(14.142));
    let report = lowered.find("PX2111").unwrap();
    assert_eq!(report.feature, Some(FeatureId::EffectShadow));
    assert_eq!(report.message, "A shadow is not drawn: inner shadow");
    assert_eq!(shadow_of(&lowered, "pp-256.16")["dx"], json!(14.142));
    assert_eq!(lowered.codes("pp-256.16"), ["PX2101"]);
    assert_eq!(
        json(&lowered.find("PX2101").unwrap().data),
        json!({ "effect": "glow" })
    );
}

#[test]
fn draws_an_offset_past_the_irs_bound_at_the_bound() {
    // ST_PositiveCoordinate allows a `dist` of 2 · 10⁹ EMU (157,480 pt).
    let far = outer("dist=\"2000000000\" dir=\"0\" rotWithShape=\"0\"");
    let compiled = compile_bytes(&deck(rect(10, &effects(&far), None)), "far.pptx");
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
    let html = compiled.text("slides/pp-256.html");
    assert!(
        html.contains("box-shadow:100000pt 0pt 0pt #00000080"),
        "{html}"
    );
}

#[test]
fn leaves_out_a_shadow_that_paints_nothing() {
    let transparent = "<a:outerShdw dist=\"127000\"><a:srgbClr val=\"000000\"><a:alpha val=\"0\"/></a:srgbClr></a:outerShdw>";
    let lowered = lower(&deck(rect(10, &effects(transparent), None)));
    assert_eq!(shadow_of(&lowered, "pp-256.10"), json!(null));
    assert!(lowered.codes("pp-256.10").is_empty());
    assert!(!lowered.lowered.features["pp-256"].contains(&FeatureId::EffectShadow));
}

#[test]
fn a_group_carries_its_own_shadow_and_its_children_theirs() {
    let child = |id: u64, effects: &str| {
        sp(Sp {
            id,
            sp_pr: format!("{}{RECT}{WHITE}{effects}", xfrm(10.0, 10.0, 50.0, 50.0, "")),
            ..Sp::default()
        })
    };
    let group = format!(
        "<p:grpSp><p:nvGrpSpPr><p:cNvPr id=\"10\" name=\"Group\"/><p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr>{}{}</p:grpSpPr>{}{}</p:grpSp>",
        xfrm(0.0, 0.0, 100.0, 100.0, ""),
        effects(&base()),
        child(11, ""),
        child(12, &effects(&outer("dist=\"127000\""))),
    );
    let lowered = lower(&deck(group));
    let slide = &lowered.presentation().slides[0];
    let group = json(&slide.elements[0]);
    assert_eq!(group["shadow"]["dx"], json!(14.142));
    assert!(group["children"][0].get("shadow").is_none());
    assert_eq!(group["children"][1]["shadow"]["dx"], json!(10));
}

#[test]
fn pictures_and_movies_carry_their_shadows() {
    let picture = pic(
        10,
        &blip("rIdPng", "", "<a:stretch><a:fillRect/></a:stretch>"),
        Pic {
            sp_pr: Some(format!(
                "{}{RECT}{}",
                xfrm(20.0, 20.0, 100.0, 60.0, ""),
                effects(&base())
            )),
            ..Pic::default()
        },
    );
    // A movie's shadow may come from its style.
    let video = pic(
        11,
        &blip("rIdPng", "", "<a:stretch><a:fillRect/></a:stretch>"),
        Pic {
            nv_pr: "<a:videoFile r:link=\"rIdVideo\"/>".to_owned(),
            sp_pr: Some(format!("{}{RECT}", xfrm(200.0, 20.0, 160.0, 90.0, ""))),
            ..Pic::default()
        },
    )
    .replace(
        "</p:spPr></p:pic>",
        &format!(
            "</p:spPr><p:style>{}</p:style></p:pic>",
            effect_ref(2, "<a:srgbClr val=\"000000\"/>")
        ),
    );
    let bytes = pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            theme: Some(theme(&effect_styles())),
            layouts: vec![LayoutSpec::default()],
            ..MasterSpec::default()
        }]),
        slides: vec![SlideSpec {
            rels: vec![
                rel("rIdPng", "image", "../media/image1.png"),
                rel("rIdVideo", "video", "../media/movie1.mp4"),
            ],
            ..slide(format!("{picture}{video}"))
        }],
        parts: vec![
            ("ppt/media/image1.png".to_owned(), png(4, 2)),
            (
                "ppt/media/movie1.mp4".to_owned(),
                movie("vide", "avc1", true, Some(1000)),
            ),
        ],
        ..PptxSpec::default()
    });
    let lowered = lower(&bytes);
    let slide = &lowered.presentation().slides[0];
    let picture = json(&slide.elements[0]);
    assert_eq!(picture["kind"], "picture");
    assert_eq!(picture["shadow"]["dx"], json!(14.142));
    let video = json(&slide.elements[1]);
    assert_eq!(video["kind"], "media");
    assert_eq!(
        video["shadow"],
        json!({ "dx": 0, "dy": 3, "sigma": 4.3, "color": black(0.5) })
    );
}

#[test]
fn reads_run_shadows_from_run_properties_and_list_styles() {
    let text = |id: u64, list_style: &str, r_pr: &str| {
        sp(Sp {
            id,
            sp_pr: format!("{}{RECT}<a:noFill/>", xfrm(100.0, 100.0, 400.0, 100.0, "")),
            tx_body: Some(format!(
                "<a:bodyPr/><a:lstStyle>{list_style}</a:lstStyle><a:p><a:r><a:rPr lang=\"en-US\" sz=\"7200\">{r_pr}</a:rPr><a:t>Shadow</a:t></a:r></a:p>"
            )),
            ..Sp::default()
        })
    };
    let level = format!(
        "<a:lvl1pPr><a:defRPr>{}</a:defRPr></a:lvl1pPr>",
        effects(&base())
    );
    let tree = [
        text(10, "", &effects(&base())),
        text(11, &level, ""),
        // A run's empty list says that it has none, over its list style's.
        text(12, &level, "<a:effectLst/>"),
        text(
            13,
            "",
            &effects("<a:innerShdw dist=\"127000\"><a:srgbClr val=\"000000\"/></a:innerShdw>"),
        ),
    ]
    .concat();
    let lowered = lower(&deck(tree));
    let slide = &lowered.presentation().slides[0];
    let run = |id: &str| element(slide, id)["text"]["paragraphs"][0]["inlines"][0]["props"].clone();
    let drawn = json!({ "dx": 14.142, "dy": 14.142, "sigma": 0, "color": black(0.5) });
    assert_eq!(run("pp-256.10")["shadow"], drawn);
    assert!(run("pp-256.11").get("shadow").is_none());
    assert_eq!(
        element(slide, "pp-256.11")["text"]["listStyle"]["0"]["run"]["shadow"],
        drawn
    );
    assert_eq!(run("pp-256.12")["shadow"], json!(null));
    assert_eq!(run("pp-256.13")["shadow"], json!(null));
    assert_eq!(not_drawn(&lowered, "pp-256.13"), ["inner shadow"]);
    assert_eq!(
        lowered.find("PX2111").unwrap().feature,
        Some(FeatureId::TextShadow)
    );
    // The element itself has no shadow.
    assert!(element(slide, "pp-256.10").get("shadow").is_none());
    let features = &lowered.lowered.features["pp-256"];
    assert!(features.contains(&FeatureId::TextShadow));
    assert!(!features.contains(&FeatureId::EffectShadow));
}

#[test]
fn reports_table_and_background_shadows() {
    let table = format!(
        "<p:graphicFrame><p:nvGraphicFramePr><p:cNvPr id=\"10\" name=\"Table\"/><p:cNvGraphicFramePr/><p:nvPr/></p:nvGraphicFramePr><p:xfrm><a:off x=\"0\" y=\"0\"/><a:ext cx=\"2540000\" cy=\"1270000\"/></p:xfrm><a:graphic><a:graphicData uri=\"http://schemas.openxmlformats.org/drawingml/2006/table\"><a:tbl><a:tblPr>{}</a:tblPr><a:tblGrid><a:gridCol w=\"2540000\"/></a:tblGrid><a:tr h=\"1270000\"><a:tc><a:txBody><a:bodyPr/><a:lstStyle/><a:p><a:endParaRPr lang=\"en-US\"/></a:p></a:txBody><a:tcPr/></a:tc></a:tr></a:tbl></a:graphicData></a:graphic></p:graphicFrame>",
        effects(&base())
    );
    let bytes = pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            theme: Some(theme(&effect_styles())),
            layouts: vec![LayoutSpec::default()],
            ..MasterSpec::default()
        }]),
        slides: vec![SlideSpec {
            bg: Some(format!("<p:bgPr>{WHITE}{}</p:bgPr>", effects(&base()))),
            ..slide(table)
        }],
        ..PptxSpec::default()
    });
    let lowered = lower(&bytes);
    assert_eq!(not_drawn(&lowered, "pp-256.10"), ["table shadow"]);
    let background = lowered
        .diagnostics
        .iter()
        .find(|d| d.code.code() == "PX2111" && d.node.is_none())
        .unwrap();
    assert_eq!(
        json(&background.data),
        json!({ "reason": "background shadow" })
    );
    assert_eq!(background.feature, Some(FeatureId::EffectShadow));
    assert!(lowered.find("PX2101").is_none());
}

/// A title placeholder without effects takes its layout placeholder's shadow, which the backend draws.
#[test]
fn a_placeholder_inherits_its_layout_placeholders_shadow() {
    let title = |id: u64, sp_pr: String| {
        sp(Sp {
            id,
            name: Some("Title".to_owned()),
            ph: "<p:ph type=\"title\"/>".to_owned(),
            sp_pr,
            tx_body: Some("<a:bodyPr/><a:lstStyle/><a:p><a:r><a:rPr lang=\"en-US\"/><a:t>Title</a:t></a:r></a:p>".to_owned()),
            ..Sp::default()
        })
    };
    let layout = title(
        2,
        format!(
            "{}{RECT}{WHITE}{}",
            xfrm(100.0, 100.0, 400.0, 100.0, ""),
            effects(&base())
        ),
    );
    let bytes = pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            layouts: vec![LayoutSpec {
                tree: layout,
                ..LayoutSpec::default()
            }],
            ..MasterSpec::default()
        }]),
        slides: vec![slide(
            [
                title(2, String::new()),
                title(3, "<a:effectLst/>".to_owned()),
            ]
            .concat(),
        )],
        ..PptxSpec::default()
    });
    let lowered = lower(&bytes);
    assert_eq!(shadow_of(&lowered, "pp-256.2"), json!("absent"));
    let compiled = compile_bytes(&bytes, "inherit.pptx");
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
    let html = compiled.text("slides/pp-256.html");
    // The slide's first title inherits the layout's shadow; its second states none.
    assert_eq!(
        html.matches("box-shadow:14.142pt 14.142pt 0pt #00000080")
            .count(),
        1,
        "{html}"
    );
}
