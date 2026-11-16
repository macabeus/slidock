//! One directed case per PPTX diagnostic the golden presentations leave out.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::json;
use support::fixtures::png;
use support::{
    DEFAULT_THEME, LayoutSpec, Lowered, MasterSpec, PptxSpec, SlideSpec, Sp, assert_matches, json,
    lower, pptx, rel, sp, xfrm,
};

const RECT: &str = "<a:prstGeom prst=\"rect\"><a:avLst/></a:prstGeom>";
const STYLE: &str = concat!(
    "<a:lnRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:lnRef><a:fillRef idx=\"1\"><a:schemeClr val=\"accent1\"/></a:fillRef>",
    "<a:effectRef idx=\"0\"><a:schemeClr val=\"accent1\"/></a:effectRef><a:fontRef idx=\"minor\"/>"
);

fn lowered() -> Lowered {
    let shapes = [
        // A placeholder with a shape style.
        sp(Sp {
            id: 2,
            ph: "<p:ph type=\"body\" idx=\"1\"/>".to_owned(),
            sp_pr: format!("{}{RECT}", xfrm(0.0, 0.0, 100.0, 50.0, "")),
            style: Some(STYLE.to_owned()),
            ..Sp::default()
        }),
        // A gradient line, and a colour modifier outside the IR's model.
        sp(Sp {
            id: 3,
            sp_pr: format!(
                "{}{RECT}<a:solidFill><a:srgbClr val=\"FF0000\"><a:gamma/></a:srgbClr></a:solidFill><a:ln w=\"12700\"><a:gradFill><a:gsLst><a:gs pos=\"0\"><a:srgbClr val=\"00FF00\"/></a:gs></a:gsLst></a:gradFill></a:ln>",
                xfrm(0.0, 60.0, 100.0, 50.0, "")
            ),
            ..Sp::default()
        }),
        // A picture whose shape names an unknown preset.
        format!(
            "<p:pic><p:nvPicPr><p:cNvPr id=\"4\" name=\"Picture\"/><p:cNvPicPr/><p:nvPr/></p:nvPicPr><p:blipFill><a:blip r:embed=\"rIdImg\"/><a:stretch><a:fillRect/></a:stretch></p:blipFill><p:spPr>{}<a:prstGeom prst=\"notAPreset\"><a:avLst/></a:prstGeom></p:spPr></p:pic>",
            xfrm(0.0, 120.0, 100.0, 50.0, "")
        ),
    ]
    .join("");
    // A theme that lacks most of its colour scheme.
    let start = DEFAULT_THEME.find("<a:dk2>").unwrap();
    let end = DEFAULT_THEME.rfind("</a:folHlink>").unwrap() + "</a:folHlink>".len();
    let theme = format!("{}{}", &DEFAULT_THEME[..start], &DEFAULT_THEME[end..]);
    lower(&pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            theme: Some(theme),
            layouts: vec![LayoutSpec::default()],
            ..MasterSpec::default()
        }]),
        slides: vec![SlideSpec {
            layout: Some(0),
            tree: shapes,
            rels: vec![
                rel("rIdImg", "image", "../media/image1.png"),
                // Notes missing from the package.
                rel("rIdNotes", "notesSlide", "../notesSlides/missing.xml"),
            ],
            ..SlideSpec::default()
        }],
        parts: vec![("ppt/media/image1.png".to_owned(), png(4, 2))],
        ..PptxSpec::default()
    }))
}

#[test]
fn reports_a_placeholder_that_carries_a_shape_style() {
    assert!(lowered().codes("pp-256.2").contains(&"PX2006".to_owned()));
}

#[test]
fn reports_the_colour_slots_a_theme_lacks_offices_defaults_standing_in() {
    let lowered = lowered();
    let missing = json(lowered.find("PX2008").unwrap());
    assert!(missing["container"].as_str().unwrap().starts_with("pp-"));
    assert!(
        missing["data"]["slots"]
            .as_str()
            .unwrap()
            .contains("accent1")
    );
}

#[test]
fn reports_notes_it_cannot_read_and_keeps_the_slide() {
    let lowered = lowered();
    assert_matches(
        &json(lowered.find("PX2010").unwrap()),
        json!({ "container": "pp-256", "data": { "part": "ppt/notesSlides/missing.xml" } }),
    );
    assert!(lowered.presentation().slides[0].failed.is_none());
}

#[test]
fn reports_a_gradient_line_and_a_dropped_colour_modifier() {
    let lowered = lowered();
    let on_slide = |code: &str| {
        lowered
            .diagnostics
            .iter()
            .find(|d| {
                d.code.code() == code
                    && d.container.as_ref().is_some_and(|c| c.as_str() == "pp-256")
            })
            .map(json)
            .unwrap()
    };
    assert_matches(&on_slide("PX2105"), json!({ "feature": "stroke.gradient" }));
    assert_matches(
        &on_slide("PX2107"),
        json!({ "data": { "modifier": "gamma" } }),
    );
}

#[test]
fn reports_a_picture_whose_shape_it_cannot_read() {
    assert!(lowered().codes("pp-256.4").contains(&"PX2308".to_owned()));
}
