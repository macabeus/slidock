//! text-metrics fitting a shrink that stores no scale. The text is Roboto Mono, whose every glyph advances 1229/2048
//! em and whose hhea line (ascender 2146, descender −555, no gap, 2048 units per em) is its content area, so each
//! expectation is worked out by hand: at size `s` a character is 0.6001·s wide and a single `hhea` line 1.3188·s
//! tall. The boxes have no insets.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::sync::Arc;

use serde_json::{Value, json};
use slidock_fonts::{CatalogFace, FaceFile, FontCatalog, builtin_catalog, read_sfnt};
use support::*;

const ROBOTO_MONO: &[u8] =
    include_bytes!("../../slidock-fonts/tests/fonts/roboto-mono/RobotoMono-Regular.ttf");

/// The built-in catalog plus Roboto Mono, its file kept.
fn catalog() -> FontCatalog {
    let face = &read_sfnt(ROBOTO_MONO).unwrap()[0];
    let added = FontCatalog::build([(
        "Roboto Mono",
        CatalogFace::from_sfnt(face, None).with_file(FaceFile::new(Arc::from(ROBOTO_MONO), 0)),
    )]);
    FontCatalog::merge([builtin_catalog(), &added])
}

/// A single-spaced paragraph on the hhea line, of one run of `text` in `family` at `size`, with `props`.
fn paragraph_of(family: &str, text: &str, size: f64, props: Value) -> Value {
    let props = with(
        json!({ "lineSpacing": { "kind": "multiple", "value": 1, "basis": "hhea" } }),
        props,
    );
    paragraph(
        0,
        props,
        vec![run(
            text,
            json!({ "font": { "kind": "family", "family": family }, "size": size }),
        )],
    )
}

fn mono(text: &str, size: f64) -> Value {
    paragraph_of("Roboto Mono", text, size, json!({}))
}

/// A `w` × `h` box of `paragraphs` that shrinks by `autofit`, with no insets.
fn shrinking(w: f64, h: f64, autofit: Value, paragraphs: Vec<Value>) -> Value {
    let mut body = text(paragraphs);
    body["body"] = json!({ "insets": { "l": 0, "t": 0, "r": 0, "b": 0 }, "autofit": autofit });
    with(
        rect("ir-s1.box", frame(10.0, 10.0, w, h)),
        json!({ "geometry": { "kind": "preset", "name": "rect", "adjust": {} }, "text": body }),
    )
}

/// The passes through text-metrics on a slide holding `element`, against `fonts`.
fn measured(element: Value, fonts: FontCatalog) -> Applied {
    let presentation = with_elements(vec![element]);
    let mut env = Env::of(&presentation);
    env.fonts = fonts;
    let mut applied = Applied {
        presentation,
        diagnostics: Vec::new(),
    };
    for pass in passes_before("lower-geometry") {
        let next = apply_pass_in(&pass, &applied.presentation, &env);
        applied.presentation = next.presentation;
        applied.diagnostics.extend(next.diagnostics);
    }
    applied
}

fn autofit(applied: &Applied) -> Value {
    to_json(element(&applied.presentation, "ir-s1.box"))["text"]["body"]["autofit"].clone()
}

fn fit(element: Value) -> Value {
    let applied = measured(element, catalog());
    assert!(
        applied
            .diagnostics
            .iter()
            .all(|d| d.code.code() != "MID5403"),
        "{:?}",
        applied.diagnostics
    );
    autofit(&applied)
}

fn percent() -> Value {
    json!({ "kind": "shrink", "steps": "percent" })
}

fn office() -> Value {
    json!({ "kind": "shrink", "steps": "office" })
}

/// A number as the IR prints it: a whole one without a fraction.
fn number(x: f64) -> Value {
    if x.fract() == 0.0 {
        json!(x as i64)
    } else {
        json!(x)
    }
}

fn scaled(font: f64, reduction: f64) -> Value {
    json!({ "kind": "shrink", "fontScale": number(font), "lineSpacingReduction": number(reduction), "fitted": true })
}

#[test]
fn text_that_fits_keeps_its_size() {
    // One 20 pt line, 26.38 pt tall, in a 30 pt box.
    let fitted = fit(shrinking(200.0, 30.0, office(), vec![mono("Fits", 20.0)]));
    assert_eq!(fitted, scaled(1.0, 0.0));
}

#[test]
fn percent_steps_take_the_largest_whole_percent_at_which_the_text_fits() {
    // One 20 pt line in a 20 pt box: 26.377·p ≤ 20 below 75.8 %.
    let fitted = fit(shrinking(200.0, 20.0, percent(), vec![mono("Fits", 20.0)]));
    assert_eq!(fitted, scaled(0.75, 0.0));
}

#[test]
fn office_steps_take_line_spacing_off_before_the_font() {
    // One 20 pt line (26.38 pt) in a 26 pt box. At line spacing 0.9 the line is 23.74 pt and the glyphs, 26.38 pt
    // tall, sit centred on it, so they reach (23.74 + 26.38) / 2 = 25.06 pt down: they fit at full size.
    let fitted = fit(shrinking(200.0, 26.0, office(), vec![mono("Fits", 20.0)]));
    assert_eq!(fitted, scaled(1.0, 0.1));
}

#[test]
fn the_leading_outside_the_first_and_last_lines_glyphs_takes_no_room() {
    // A 1.5-spaced 20 pt line is 39.57 pt tall, its glyphs 26.38 pt in its middle: they fit a 30 pt box.
    let spaced = paragraph_of(
        "Roboto Mono",
        "Spaced",
        20.0,
        json!({ "lineSpacing": { "kind": "multiple", "value": 1.5, "basis": "hhea" } }),
    );
    assert_eq!(
        fit(shrinking(200.0, 30.0, percent(), vec![spaced])),
        scaled(1.0, 0.0)
    );
}

#[test]
fn office_steps_go_down_in_powerpoint_order() {
    // Three 20 pt lines: the glyphs end at 2·L + (L + C) / 2, C = 26.38·f and L = C·(1 − r).
    // (0.925, 0.1) needs 67.1 pt and (0.925, 0.2) 61.0 pt; (0.85, 0.1), tried after it, would fit too (61.7).
    let lines = vec![mono("One", 20.0), mono("Two", 20.0), mono("Three", 20.0)];
    let fitted = fit(shrinking(200.0, 62.0, office(), lines));
    assert_eq!(fitted, scaled(0.925, 0.2));
}

#[test]
fn office_steps_end_at_the_last_when_none_fits() {
    let lines = (0..40).map(|_| mono("Overflowing", 20.0)).collect();
    let fitted = fit(shrinking(200.0, 30.0, office(), lines));
    assert_eq!(fitted, scaled(0.25, 0.2));
}

#[test]
fn lines_break_again_at_each_scale() {
    // "aaaa bbbb cccc" in 120 pt: two lines at full size (a word is 48.0 pt), one below 71.4 %, where its 14
    // characters take 168.0·p. One line fits a 20 pt box below 75.8 %, so 71 %; kept at two lines it would be 37 %.
    let fitted = fit(shrinking(
        120.0,
        20.0,
        percent(),
        vec![mono("aaaa bbbb cccc", 20.0)],
    ));
    assert_eq!(fitted, scaled(0.71, 0.0));
}

#[test]
fn a_word_wider_than_its_line_breaks_between_its_characters() {
    // Ten characters in 60 pt: 2 lines once 9 fit (p ≤ 0.555) or 8 do (0.555 < p ≤ 0.624); 2·26.377·p ≤ 30 below
    // 56.9 %. Left whole on one line, the word would fit at full size.
    let fitted = fit(shrinking(
        60.0,
        30.0,
        percent(),
        vec![mono("abcdefghij", 20.0)],
    ));
    assert_eq!(fitted, scaled(0.56, 0.0));
}

#[test]
fn the_first_line_starts_after_its_indent() {
    // "aaaa bbbb" (108.0 pt) in 110 pt is one line, but not after a 30 pt indent until 9 characters fit 80 pt
    // (p ≤ 0.7406); one line fits 30 pt at full size.
    let indented = paragraph_of("Roboto Mono", "aaaa bbbb", 20.0, json!({ "indent": 30 }));
    let fitted = fit(shrinking(110.0, 30.0, percent(), vec![indented]));
    assert_eq!(fitted, scaled(0.74, 0.0));
}

#[test]
fn spaces_at_a_line_end_hang() {
    // "aaaa " is five characters, 60.0 pt, in 50 pt: the trailing space hangs, so four fit at full size.
    let fitted = fit(shrinking(50.0, 30.0, percent(), vec![mono("aaaa ", 20.0)]));
    assert_eq!(fitted, scaled(1.0, 0.0));
}

#[test]
fn a_box_that_does_not_wrap_keeps_its_lines_whole() {
    let mut element = shrinking(60.0, 30.0, percent(), vec![mono("abcdefghij", 20.0)]);
    element["text"]["body"]["wrap"] = json!("none");
    assert_eq!(fit(element), scaled(1.0, 0.0));
}

#[test]
fn a_stored_scale_is_kept_and_not_fitted_again() {
    let stored = json!({ "kind": "shrink", "fontScale": 0.8, "steps": "office" });
    let fitted = fit(shrinking(200.0, 5.0, stored, vec![mono("Stored", 20.0)]));
    assert_eq!(fitted, scaled(0.8, 0.0));
}

#[test]
fn text_without_glyph_widths_keeps_its_size_and_says_so() {
    // Liberation Sans is in the built-in metrics table, with no file to measure.
    let lines = vec![paragraph_of(
        "Liberation Sans",
        "No widths",
        20.0,
        json!({}),
    )];
    let applied = measured(shrinking(200.0, 5.0, office(), lines), catalog());
    assert_eq!(
        autofit(&applied),
        json!({ "kind": "shrink", "fontScale": 1, "lineSpacingReduction": 0, "fitted": false })
    );
    let reported: Vec<Value> = applied
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "MID5403")
        .map(|d| {
            json!([
                d.node.as_ref().map(|n| n.as_str().to_owned()),
                to_json(&d.data.as_ref().unwrap()["family"]),
                d.message
            ])
        })
        .collect();
    assert_eq!(
        reported,
        [json!([
            "ir-s1.box",
            "Liberation Sans",
            "The text is set to shrink to fit, but the file stores no scale and no font file for Liberation Sans is available to measure it; it keeps its size"
        ])]
    );
}

#[test]
fn emoji_text_fits_by_the_platform_emoji_advance() {
    // Apple Color Emoji binds the emoji generic: no file, but every cluster one advance wide.
    let emoji = |text: &str| paragraph_of("Apple Color Emoji", text, 20.0, json!({}));
    let roomy = measured(
        shrinking(200.0, 40.0, percent(), vec![emoji("😀😀")]),
        catalog(),
    );
    assert_eq!(autofit(&roomy), scaled(1.0, 0.0));
    let narrow = measured(
        shrinking(30.0, 30.0, percent(), vec![emoji("😀😀👩‍💻🇧🇷")]),
        catalog(),
    );
    assert!(
        narrow
            .diagnostics
            .iter()
            .all(|d| d.code.code() != "MID5403")
    );
    let fitted = autofit(&narrow);
    assert_eq!(fitted["fitted"], json!(true));
    assert!(fitted["fontScale"].as_f64().unwrap() < 1.0, "{fitted}");
    // Two joined clusters fit one line, which their six characters at one advance each would not.
    let wide = measured(
        shrinking(60.0, 30.0, percent(), vec![emoji("👩‍💻👩‍💻")]),
        catalog(),
    );
    assert_eq!(autofit(&wide), scaled(1.0, 0.0));
}
