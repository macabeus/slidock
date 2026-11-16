//! Gradients as LibreOffice 25.2 reads them (`probes/out/gradient-probe.odp`): the `odf` source
//! gradient's defaults, the unit of a unitless angle by ODF version and generator, `loext:gradient-stop`s and the
//! intensities, `draw:gradient-step-count`, and transparency gradients folded into the stop alphas or reported
//! (OD9103).
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use support::fixtures::{graphic, place};
use support::odp::{OdpSpec, PageSpec, files, replace, zip_entries};
use support::{Lowered, assert_holds, js, lower};

const LO_6: &str = "LibreOfficeDev/6.0.5.2$Linux_X86_64 LibreOffice_project/";
const LO_7: &str = "LibreOffice/7.6.4.1$Linux_X86_64 LibreOffice_project/e19e193f88cd";

fn rect(name: &str, style: &str) -> String {
    format!(
        r#"<draw:rect draw:name="{name}" draw:style-name="{style}" {}/>"#,
        place("1in", "1in", "4in", "2in")
    )
}

/// One slide of rectangles `(name, graphic properties)` over the named resources `styles`, written as ODF `version`
/// by `generator`.
fn deck_as(styles: &str, shapes: &[(&str, &str)], version: &str, generator: &str) -> Lowered {
    let automatic: String = shapes
        .iter()
        .enumerate()
        .map(|(i, (_, props))| graphic(&format!("gr{i}"), props))
        .collect();
    let body: String = shapes
        .iter()
        .enumerate()
        .map(|(i, (name, _))| rect(name, &format!("gr{i}")))
        .collect();
    let mut parts = files(&OdpSpec {
        styles: styles.to_owned(),
        automatic_styles: automatic,
        generator: Some(generator.to_owned()),
        ..OdpSpec::pages(vec![PageSpec::body(body)])
    });
    let styles_xml = parts.iter().find(|(name, _)| name == "styles.xml").unwrap();
    let text = String::from_utf8(styles_xml.1.clone()).unwrap();
    let text = text.replacen(
        r#"office:version="1.2""#,
        &format!(r#"office:version="{version}""#),
        1,
    );
    replace(&mut parts, "styles.xml", text);
    lower(&zip_entries(&parts))
}

fn deck(styles: &str, shapes: &[(&str, &str)]) -> Lowered {
    deck_as(styles, shapes, "1.3", LO_7)
}

fn fill<'l>(lowered: &'l Lowered, name: &str) -> &'l Value {
    &lowered.named(0, name)["fill"]
}

fn rgba(r: f64, g: f64, b: f64, a: f64) -> Value {
    js(&json!({ "kind": "rgb", "space": "srgb", "r": r, "g": g, "b": b, "a": a }))
}

fn reasons(lowered: &Lowered) -> Vec<(String, Value)> {
    lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "OD9103")
        .map(|d| {
            (
                d.node.as_ref().unwrap().as_str().to_owned(),
                js(&json!({ "feature": d.feature.map(|f| f.as_str()), "reason": js(&d.data)["reason"] })),
            )
        })
        .collect()
}

/// The fill features the frontend saw on the slide.
fn fills(lowered: &Lowered) -> Vec<&'static str> {
    let mut fills = lowered.features("od-p1");
    fills.retain(|f| f.starts_with("fill."));
    fills
}

fn gradient_props(name: &str) -> String {
    format!(r#"draw:fill="gradient" draw:fill-gradient-name="{name}""#)
}

#[test]
fn reads_libreoffices_defaults_black_to_black_centred_at_the_corner() {
    let lowered = deck(
        r#"<draw:gradient draw:name="Bare"/><draw:gradient draw:name="Round" draw:style="radial"/>"#,
        &[
            ("Bare", &gradient_props("Bare")),
            ("Round", &gradient_props("Round")),
        ],
    );
    let black = rgba(0.0, 0.0, 0.0, 1.0);
    let expected = |style: &str| {
        js(&json!({
            "kind": "odf", "style": style, "angle": 0, "border": 0, "center": { "x": 0, "y": 0 },
            "stops": [{ "pos": 0, "color": black }, { "pos": 1, "color": black }]
        }))
    };
    assert_eq!(js(fill(&lowered, "Bare")), expected("linear"));
    assert_eq!(js(fill(&lowered, "Round")), expected("radial"));
    assert_eq!(fills(&lowered), ["fill.linear", "fill.radial"]);
    assert!(reasons(&lowered).is_empty());
}

#[test]
fn reads_a_unitless_angle_in_tenths_only_before_odf_1_2_or_from_an_older_writer() {
    let styles = r##"<draw:gradient draw:name="G" draw:style="linear" draw:angle="300"/>"##;
    let angle = |version: &str, generator: &str| {
        let lowered = deck_as(styles, &[("Box", &gradient_props("G"))], version, generator);
        fill(&lowered, "Box")["angle"].as_f64().unwrap()
    };
    assert_eq!(angle("1.3", LO_6), 300.0);
    assert_eq!(angle("1.2", LO_7), 300.0);
    assert_eq!(angle("1.2", LO_6), 30.0);
    assert_eq!(
        angle(
            "1.1",
            "LibreOffice/25.2.3.2$MacOSX_AARCH64 LibreOffice_project/bbb0"
        ),
        30.0
    );
    // Units are read as written; whole tenths within a turn.
    let lowered = deck(
        r##"<draw:gradient draw:name="A" draw:angle="-30deg"/><draw:gradient draw:name="B" draw:angle="0.12345rad"/>"##,
        &[("A", &gradient_props("A")), ("B", &gradient_props("B"))],
    );
    assert_eq!(fill(&lowered, "A")["angle"], 330);
    assert_eq!(fill(&lowered, "B")["angle"], 7.1);
}

#[test]
fn replaces_the_start_and_end_by_loext_stops_and_blends_them_toward_black_by_offset() {
    let styles = concat!(
        r##"<draw:gradient draw:name="Multi" draw:style="linear" draw:start-color="#00ff00" draw:end-color="#00ff00" "##,
        r#"draw:start-intensity="50%" draw:end-intensity="100%">"#,
        r##"<loext:gradient-stop svg:offset="1" loext:color-type="rgb" loext:color-value="#0000ff"/>"##,
        r##"<loext:gradient-stop svg:offset="0.3" loext:color-type="rgb" loext:color-value="#00ff00"/>"##,
        r##"<loext:gradient-stop svg:offset="0" loext:color-type="rgb" loext:color-value="#ff0000"/>"##,
        r##"<loext:gradient-stop svg:offset="0.5" loext:color-type="theme" loext:color-value="#ffffff"/>"##,
        r#"</draw:gradient>"#,
    );
    let lowered = deck(styles, &[("Box", &gradient_props("Multi"))]);
    assert_eq!(
        js(&fill(&lowered, "Box")["stops"]),
        js(&json!([
            { "pos": 0, "color": rgba(0.5, 0.0, 0.0, 1.0) },
            { "pos": 0.3, "color": rgba(0.0, 0.65, 0.0, 1.0) },
            { "pos": 1, "color": rgba(0.0, 0.0, 1.0, 1.0) }
        ]))
    );
}

#[test]
fn draws_steps_up_to_libreoffices_automatic_count() {
    let styles = concat!(
        r##"<draw:gradient draw:name="BW" draw:start-color="#000000" draw:end-color="#ffffff"/>"##,
        r##"<draw:gradient draw:name="Near" draw:start-color="#000000" draw:end-color="#020202"/>"##,
    );
    let steps = |count: &str, name: &str| {
        let props = format!(
            r#"{} draw:gradient-step-count="{count}""#,
            gradient_props(name)
        );
        let lowered = deck(styles, &[("Box", &props)]);
        fill(&lowered, "Box").get("steps").cloned()
    };
    assert_eq!(steps("4", "BW"), Some(json!(4)));
    assert_eq!(steps("300", "BW"), Some(json!(128)));
    assert_eq!(steps("4", "Near"), Some(json!(1)));
    assert_eq!(steps("0", "BW"), None);
}

#[test]
fn folds_a_transparency_gradient_over_a_solid_colour_into_a_gradient_of_its_own() {
    let styles = concat!(
        r#"<draw:opacity draw:name="Fade" draw:style="linear" draw:start="100%" draw:end="0%" draw:angle="0deg"/>"#,
        r#"<draw:opacity draw:name="Ring" draw:style="radial" draw:cx="25%" draw:cy="25%" draw:start="0%" "#,
        r#"draw:end="100%" draw:border="20%"/>"#,
        r#"<draw:opacity draw:name="Frame" draw:style="rectangular" draw:start="0%" draw:end="100%" draw:border="20%"/>"#,
    );
    let solid = |opacity: &str| {
        format!(r##"draw:fill="solid" draw:fill-color="#ff0000" draw:opacity-name="{opacity}""##)
    };
    let lowered = deck(
        styles,
        &[
            ("Fade", &solid("Fade")),
            ("Ring", &solid("Ring")),
            ("Frame", &solid("Frame")),
        ],
    );
    assert_eq!(
        js(fill(&lowered, "Fade")),
        js(&json!({
            "kind": "odf", "style": "linear", "angle": 0, "border": 0, "center": { "x": 0, "y": 0 },
            "stops": [{ "pos": 0, "color": rgba(1.0, 0.0, 0.0, 1.0) }, { "pos": 1, "color": rgba(1.0, 0.0, 0.0, 0.0) }]
        }))
    );
    // LibreOffice's PDF draws a radial transparency's border on its contours and again on its stops.
    assert_holds(
        fill(&lowered, "Ring"),
        json!({
            "style": "radial", "border": 0.2, "center": { "x": 0.25, "y": 0.25 },
            "stops": [{ "pos": 0, "color": { "a": 0 } }, { "pos": 0.8, "color": { "a": 1 } }]
        }),
    );
    // And a rectangular one's on its stops only.
    assert_holds(
        fill(&lowered, "Frame"),
        json!({
            "style": "rectangular", "border": 0,
            "stops": [{ "pos": 0.2, "color": { "a": 0 } }, { "pos": 1, "color": { "a": 1 } }]
        }),
    );
    assert!(reasons(&lowered).is_empty());
    assert_eq!(fills(&lowered), ["fill.linear", "fill.path", "fill.radial"]);
}

#[test]
fn bands_an_ellipsoid_transparency_by_its_own_opacities() {
    let styles = concat!(
        r#"<draw:opacity draw:name="Oval" draw:style="ellipsoid" draw:cx="50%" draw:cy="50%" draw:start="0%" "#,
        r#"draw:end="100%"/>"#,
        r#"<draw:opacity draw:name="Faint" draw:style="ellipsoid" draw:cx="50%" draw:cy="50%" draw:start="90%" "#,
        r#"draw:end="100%"/>"#,
    );
    let solid = |opacity: &str| {
        format!(r##"draw:fill="solid" draw:fill-color="#ff0000" draw:opacity-name="{opacity}""##)
    };
    let lowered = deck(
        styles,
        &[("Oval", &solid("Oval")), ("Faint", &solid("Faint"))],
    );
    // The colour is constant: the bands come from the opacities, 0 to 100 % and 90 to 100 %.
    assert_holds(
        fill(&lowered, "Oval"),
        json!({ "style": "ellipsoid", "steps": 128 }),
    );
    assert_holds(
        fill(&lowered, "Faint"),
        json!({ "style": "ellipsoid", "steps": 13 }),
    );
}

#[test]
fn folds_a_transparency_gradient_of_the_same_contours_into_the_colour_stops() {
    let styles = concat!(
        r##"<draw:gradient draw:name="Ramp" draw:style="linear" draw:start-color="#000000" draw:end-color="#ffffff" "##,
        r#"draw:angle="90deg"/>"#,
        r#"<draw:opacity draw:name="Half" draw:style="linear" draw:start="100%" draw:end="20%" draw:angle="90deg" "#,
        r#"draw:border="50%" draw:cx="10%"/>"#,
    );
    let props = format!(r#"{} draw:opacity-name="Half""#, gradient_props("Ramp"));
    let lowered = deck(styles, &[("Box", &props)]);
    // Linear contours ignore the centre, and the border only moves the stops: the opacity holds to the middle.
    assert_eq!(
        js(fill(&lowered, "Box")),
        js(&json!({
            "kind": "odf", "style": "linear", "angle": 90, "border": 0, "center": { "x": 0, "y": 0 },
            "stops": [
                { "pos": 0, "color": rgba(0.0, 0.0, 0.0, 1.0) },
                { "pos": 0.5, "color": rgba(0.5, 0.5, 0.5, 1.0) },
                { "pos": 1, "color": rgba(1.0, 1.0, 1.0, 0.2) }
            ]
        }))
    );
    assert!(reasons(&lowered).is_empty());
}

#[test]
fn a_uniform_opacity_wins_over_the_transparency_gradient() {
    let styles = concat!(
        r##"<draw:gradient draw:name="Ramp" draw:start-color="#000000" draw:end-color="#ffffff"/>"##,
        r#"<draw:opacity draw:name="Fade" draw:start="100%" draw:end="0%"/>"#,
    );
    let half = format!(
        r#"{} draw:opacity="50%" draw:opacity-name="Fade""#,
        gradient_props("Ramp")
    );
    let full = format!(
        r#"{} draw:opacity="100%" draw:opacity-name="Fade""#,
        gradient_props("Ramp")
    );
    let lowered = deck(styles, &[("Half", &half), ("Full", &full)]);
    let alphas = |name: &str| -> Vec<f64> {
        fill(&lowered, name)["stops"]
            .as_array()
            .unwrap()
            .iter()
            .map(|stop| stop["color"]["a"].as_f64().unwrap())
            .collect()
    };
    assert_eq!(alphas("Half"), [0.5, 0.5]);
    assert_eq!(alphas("Full"), [1.0, 0.0]);
    assert!(reasons(&lowered).is_empty());
}

#[test]
fn reports_the_transparency_gradients_it_cannot_fold_and_draws_without_them() {
    let styles = concat!(
        r##"<draw:gradient draw:name="Ramp" draw:style="linear" draw:angle="90deg"/>"##,
        r#"<draw:opacity draw:name="Ring" draw:style="radial" draw:cx="25%" draw:cy="25%" draw:start="0%" "#,
        r#"draw:end="100%" draw:border="20%"/>"#,
        r#"<draw:opacity draw:name="Oval" draw:style="ellipsoid" draw:start="0%" draw:border="10%"/>"#,
        r#"<draw:opacity draw:name="Fade" draw:style="linear" draw:start="100%" draw:end="0%" draw:angle="90deg"/>"#,
        r#"<draw:gradient draw:name="Odd" draw:style="spiral"/>"#,
    );
    let shapes = [
        (
            "Own",
            format!(r#"{} draw:opacity-name="Ring""#, gradient_props("Ramp")),
        ),
        (
            "Oval",
            r##"draw:fill="solid" draw:fill-color="#ff0000" draw:opacity-name="Oval""##.to_owned(),
        ),
        (
            "Steps",
            format!(
                r#"{} draw:gradient-step-count="4" draw:opacity-name="Fade""#,
                gradient_props("Ramp")
            ),
        ),
        ("Odd", gradient_props("Odd")),
    ];
    let shapes: Vec<(&str, &str)> = shapes.iter().map(|(n, p)| (*n, p.as_str())).collect();
    let lowered = deck(styles, &shapes);
    assert_eq!(fill(&lowered, "Own")["stops"][1]["color"]["a"], 1);
    assert_eq!(fill(&lowered, "Oval")["kind"], "solid");
    assert_eq!(fill(&lowered, "Odd")["style"], "linear");
    let ids: Vec<String> = ["Own", "Oval", "Steps", "Odd"]
        .iter()
        .map(|name| lowered.named(0, name)["id"].as_str().unwrap().to_owned())
        .collect();
    let reason = |i: usize, feature: &str, reason: &str| {
        (
            ids[i].clone(),
            json!({ "feature": feature, "reason": reason }),
        )
    };
    assert_eq!(
        reasons(&lowered),
        [
            reason(
                0,
                "fill.radial",
                "a transparency gradient with its own geometry"
            ),
            reason(
                1,
                "fill.path",
                "an ellipsoid transparency gradient with a border"
            ),
            reason(
                2,
                "fill.linear",
                "a transparency gradient over a gradient with steps"
            ),
            reason(3, "fill.linear", "unknown style spiral"),
        ]
    );
}
