//! collect-glyphs on presentations built in Rust and run through the whole pass list: what each bound face draws.
//! Also the instance metrics bind-fonts reads for a variable face.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::collections::BTreeMap;

use serde_json::{Value, json};
use slidock_core::{PassManager, RunOptions, VerifyMode};
use slidock_core_api::Unverified;
use slidock_fonts::glyphs::ALWAYS_DRAWN;
use slidock_fonts::{CatalogFace, CodepointSet, FontCatalog, builtin_catalog};
use slidock_ir::diagnostics::DiagnosticCode;
use slidock_ir::fonts::{FaceStyle, FontMetrics};
use slidock_ir::ids::Sha256;
use slidock_ir::presentation::Failed;
use slidock_ir::resolved;
use slidock_passes::{FaceGlyphs, GlyphFace, collect_glyphs};
use support::*;

fn sans_metrics() -> FontMetrics {
    builtin_catalog()
        .get(&slidock_fonts::FamilyKey::of("Liberation Sans"))
        .unwrap()
        .faces[0]
        .metrics
        .clone()
}

/// A shipped face: a file hash made from its PostScript name.
fn file_face(weight: u32, style: FaceStyle, name: &str) -> CatalogFace {
    CatalogFace {
        weight,
        width: slidock_fonts::bind::NORMAL_WIDTH,
        style,
        postscript_name: name.to_owned(),
        metrics: sans_metrics(),
        variable: false,
        monospace: false,
        sha256: Some(Sha256::of(name.as_bytes())),
        variation: None,
        file: None,
    }
}

/// The built-in fonts plus Brand (regular, bold, italic) and Brand Symbols (regular).
fn catalog() -> FontCatalog {
    let added = FontCatalog::build([
        ("Brand", file_face(400, FaceStyle::Normal, "Brand-Regular")),
        ("Brand", file_face(700, FaceStyle::Normal, "Brand-Bold")),
        ("Brand", file_face(400, FaceStyle::Italic, "Brand-Italic")),
        (
            "Brand Symbols",
            file_face(400, FaceStyle::Normal, "BrandSymbols"),
        ),
    ]);
    FontCatalog::merge([builtin_catalog(), &added])
}

fn brand(extra: Value) -> Value {
    with(
        json!({ "font": { "kind": "family", "family": "Brand" } }),
        extra,
    )
}

fn shape_with(id: &str, paragraphs: Vec<Value>) -> Value {
    with(
        rect(id, frame(0.0, 0.0, 300.0, 100.0)),
        json!({ "geometry": { "kind": "preset", "name": "rect", "adjust": {} }, "text": text(paragraphs) }),
    )
}

/// Runs the whole pass list on a presentation with these slides.
fn resolve(slides: Vec<Value>, fonts: &FontCatalog) -> resolved::Presentation {
    resolve_with(slides, json!({}), json!({}), fonts)
}

/// Runs the whole pass list on a presentation with these slides, masters and layouts.
fn resolve_with(
    slides: Vec<Value>,
    masters: Value,
    layouts: Value,
    fonts: &FontCatalog,
) -> resolved::Presentation {
    let mut presentation = minimal_json();
    presentation["slides"] = json!(slides);
    presentation["masters"] = masters;
    presentation["layouts"] = layouts;
    let parsed = parse(presentation.clone());
    let mut env = Env::of(&parsed);
    env.fonts = fonts.clone();
    let manager = PassManager::new(slidock_passes::passes()).unwrap();
    let result = manager.run(
        &Unverified::Json(presentation),
        &RunOptions::new(VerifyMode::Boundaries, env.env()),
    );
    let diagnostics = &result.diagnostics;
    result
        .presentation
        .unwrap_or_else(|| panic!("the presentation resolves: {diagnostics:?}"))
}

fn face<'a>(faces: &'a BTreeMap<GlyphFace, FaceGlyphs>, name: &str) -> &'a FaceGlyphs {
    faces
        .iter()
        .find(|(face, _)| face.postscript_name == name)
        .map(|(_, glyphs)| glyphs)
        .unwrap_or_else(|| {
            panic!(
                "{name} draws nothing: {:?}",
                faces.keys().collect::<Vec<_>>()
            )
        })
}

/// The code points of `text` and the always-kept ones.
fn drawn(text: &str) -> CodepointSet {
    let mut set = CodepointSet::new();
    set.insert_text(text);
    for point in ALWAYS_DRAWN {
        set.insert(point);
    }
    set
}

#[test]
fn each_face_gets_the_code_points_drawn_with_it() {
    let slides = vec![
        slide(
            "ir-s1",
            vec![shape_with(
                "ir-s1.t",
                vec![paragraph(
                    0,
                    json!({}),
                    vec![
                        run("Hello ", brand(json!({}))),
                        run("World", brand(json!({ "weight": 700 }))),
                        run("aside", brand(json!({ "italic": true }))),
                    ],
                )],
            )],
        ),
        // A hidden slide still counts: the player can show it.
        with(
            slide(
                "ir-s2",
                vec![shape_with(
                    "ir-s2.t",
                    vec![paragraph(0, json!({}), vec![run("zzz", brand(json!({})))])],
                )],
            ),
            json!({ "hidden": true }),
        ),
    ];
    let fonts = catalog();
    let faces = collect_glyphs(&resolve(slides, &fonts), &fonts).unwrap();
    assert_eq!(face(&faces, "Brand-Regular").codepoints, drawn("Hello zzz"));
    assert_eq!(face(&faces, "Brand-Bold").codepoints, drawn("World"));
    assert_eq!(face(&faces, "Brand-Italic").codepoints, drawn("aside"));
    let regular = faces
        .keys()
        .find(|f| f.postscript_name == "Brand-Regular")
        .unwrap();
    assert_eq!(
        (regular.family.as_str(), regular.weight, regular.style()),
        ("Brand", 400, FaceStyle::Normal)
    );
    assert_eq!(regular.sha256, Some(Sha256::of(b"Brand-Regular")));
    assert_eq!(
        face(&faces, "Brand-Bold")
            .weights
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [700]
    );
    assert!(face(&faces, "Brand-Regular").requested.contains("Brand"));
}

#[test]
fn masters_and_layouts_count_and_failed_containers_draw_nothing() {
    let shape = |id: &str, words: &str| {
        shape_with(
            id,
            vec![paragraph(0, json!({}), vec![run(words, brand(json!({})))])],
        )
    };
    let masters = json!({
        "ir-m": { "id": "ir-m", "origin": agent(), "elements": [shape("ir-m.t", "master")] },
    });
    let layout = |id: &str, words: &str| {
        json!({ "id": id, "origin": agent(), "master": "ir-m", "showMaster": true,
                "elements": [shape(&format!("{id}.t"), words)] })
    };
    let layouts = json!({ "ir-l": layout("ir-l", "layout"), "ir-l2": layout("ir-l2", "QQQ") });
    let slides = vec![
        with(
            slide("ir-s1", vec![shape("ir-s1.t", "slide")]),
            json!({ "layout": "ir-l" }),
        ),
        slide("ir-s2", vec![shape("ir-s2.t", "JJJ")]),
    ];
    let fonts = catalog();
    let mut resolved = resolve_with(slides, masters, layouts, &fonts);
    let all = collect_glyphs(&resolved, &fonts).unwrap();
    assert_eq!(
        face(&all, "Brand-Regular").codepoints,
        drawn("masterlayoutslideQQQJJJ")
    );
    // A pass failed on the second layout and the second slide: they keep no elements, and draw nothing.
    fn failed<B>() -> resolved::Body<B> {
        resolved::Body::Failed {
            failed: Failed::new(DiagnosticCode::lookup("IR0908").unwrap(), None),
            background: None,
        }
    }
    let layout = resolved
        .layouts
        .iter_mut()
        .find(|(id, _)| id.as_str() == "ir-l2")
        .unwrap()
        .1;
    layout.body = failed();
    resolved.slides[1].body = failed();
    let faces = collect_glyphs(&resolved, &fonts).unwrap();
    assert_eq!(
        face(&faces, "Brand-Regular").codepoints,
        drawn("masterlayoutslide")
    );
}

#[test]
fn a_weight_without_its_face_draws_with_the_face_css_matches() {
    // Brand has no 600 or bold italic: 600 draws with Bold (heavier first above 500), bold italic with Italic.
    let slides = vec![slide(
        "ir-s1",
        vec![shape_with(
            "ir-s1.t",
            vec![paragraph(
                0,
                json!({}),
                vec![
                    run("semi", brand(json!({ "weight": 600 }))),
                    run("both", brand(json!({ "weight": 700, "italic": true }))),
                ],
            )],
        )],
    )];
    let fonts = catalog();
    let faces = collect_glyphs(&resolve(slides, &fonts), &fonts).unwrap();
    assert_eq!(face(&faces, "Brand-Bold").codepoints, drawn("semi"));
    assert_eq!(
        face(&faces, "Brand-Bold")
            .weights
            .iter()
            .copied()
            .collect::<Vec<_>>(),
        [600]
    );
    assert_eq!(face(&faces, "Brand-Italic").codepoints, drawn("both"));
    assert!(faces.keys().all(|f| f.postscript_name != "Brand-Regular"));
}

#[test]
fn caps_fields_bullets_tables_and_groups_add_what_they_draw() {
    let table = json!({
        "id": "ir-s1.table", "kind": "table", "origin": agent(), "frame": frame(0.0, 200.0, 200.0, 50.0),
        "columns": [{ "width": 200 }],
        "rows": [{ "height": 50, "cells": [{ "text": text(vec![paragraph(0, json!({}), vec![run("cell", brand(json!({})))])]) }] }],
    });
    let group = json!({
        "id": "ir-s1.group", "kind": "group", "origin": agent(), "frame": frame(0.0, 300.0, 100.0, 50.0),
        "children": [shape_with("ir-s1.child", vec![paragraph(0, json!({}), vec![run("kid", brand(json!({})))])])],
    });
    let shape = shape_with(
        "ir-s1.t",
        vec![
            paragraph(
                0,
                json!({}),
                vec![run("straße", brand(json!({ "caps": "all" })))],
            ),
            paragraph(
                0,
                json!({}),
                vec![run("mini", brand(json!({ "caps": "small" })))],
            ),
            paragraph(
                0,
                json!({}),
                vec![
                    json!({ "kind": "field", "field": "slideNumber", "cached": "1", "props": brand(json!({})) }),
                ],
            ),
            paragraph(
                0,
                json!({ "bullet": { "kind": "char", "char": "★",
                        "font": { "kind": "family", "family": "Brand Symbols" } } }),
                vec![run("item", brand(json!({ "weight": 700 })))],
            ),
        ],
    );
    let slides = vec![slide("ir-s1", vec![shape, table, group])];
    let fonts = catalog();
    let faces = collect_glyphs(&resolve(slides, &fonts), &fonts).unwrap();
    let regular = face(&faces, "Brand-Regular");
    assert_eq!(
        regular.codepoints,
        drawn("straßeSTRASSEminiMINI0123456789cellkid")
    );
    assert!(regular.small_caps);
    assert!(!face(&faces, "Brand-Bold").small_caps);
    assert_eq!(face(&faces, "Brand-Bold").codepoints, drawn("item"));
    // The label draws in Brand Symbols at 400 and at the paragraph's bold; both match its only face.
    let symbols = face(&faces, "BrandSymbols");
    assert_eq!(symbols.codepoints, drawn("★"));
    assert_eq!(
        symbols.weights.iter().copied().collect::<Vec<_>>(),
        [400, 700]
    );
}

#[test]
fn built_in_faces_have_no_file() {
    let slides = vec![slide(
        "ir-s1",
        vec![shape_with(
            "ir-s1.t",
            vec![paragraph(
                0,
                json!({}),
                vec![run(
                    "plain",
                    json!({ "font": { "kind": "family", "family": "Arial" } }),
                )],
            )],
        )],
    )];
    let fonts = catalog();
    let faces = collect_glyphs(&resolve(slides, &fonts), &fonts).unwrap();
    let (key, glyphs) = faces
        .iter()
        .find(|(f, _)| f.family == "Liberation Sans")
        .unwrap();
    assert_eq!(key.sha256, None);
    assert_eq!(glyphs.codepoints, drawn("plain"));
    assert!(glyphs.requested.contains("Arial"));
}

#[test]
fn a_catalog_other_than_the_binding_one_is_an_error() {
    let slides = vec![slide(
        "ir-s1",
        vec![shape_with(
            "ir-s1.t",
            vec![paragraph(0, json!({}), vec![run("x", brand(json!({})))])],
        )],
    )];
    let fonts = catalog();
    let resolved = resolve(slides, &fonts);
    assert!(collect_glyphs(&resolved, builtin_catalog()).is_err());
}

#[test]
fn collecting_is_deterministic() {
    let slides = || {
        vec![slide(
            "ir-s1",
            vec![shape_with(
                "ir-s1.t",
                vec![paragraph(
                    0,
                    json!({}),
                    vec![
                        run("Zürich", brand(json!({}))),
                        run("Ωmega", brand(json!({ "weight": 700 }))),
                    ],
                )],
            )],
        )]
    };
    let fonts = catalog();
    let first = collect_glyphs(&resolve(slides(), &fonts), &fonts).unwrap();
    let second = collect_glyphs(&resolve(slides(), &fonts), &fonts).unwrap();
    assert_eq!(first, second);
    // By family, then weight; the paragraph marks draw in the theme's fallback family.
    let order: Vec<&str> = first.keys().map(|f| f.postscript_name.as_str()).collect();
    assert_eq!(order, ["Brand-Regular", "Brand-Bold", "LiberationSans"]);
}

mod variable_binding {
    use super::*;
    use slidock_fonts::{FontLimits, read_sfnt};

    const OSWALD_MVAR: &[u8] =
        include_bytes!("../../slidock-fonts/tests/fonts/oswald/Oswald-Variable-MVAR.ttf");

    /// Oswald's variable file under a family the built-in table does not hold (it holds static Oswald faces).
    fn oswald(with_instances: bool) -> FontCatalog {
        let sfnt = &read_sfnt(OSWALD_MVAR).unwrap()[0];
        let face = if with_instances {
            CatalogFace::from_file(sfnt, OSWALD_MVAR, 0, &[700], &FontLimits::default())
        } else {
            CatalogFace::from_sfnt(sfnt, Some(Sha256::of(OSWALD_MVAR)))
        };
        FontCatalog::merge([builtin_catalog(), &FontCatalog::build([("Flex", face)])])
    }

    fn bound(fonts: FontCatalog) -> Applied {
        bound_as(fonts, "Flex", &[400, 700])
    }

    /// bind-fonts on a presentation requesting `family` at `weights`, upright.
    fn bound_as(fonts: FontCatalog, family: &str, weights: &[u32]) -> Applied {
        let faces: Vec<Value> = weights
            .iter()
            .map(|weight| json!({ "weight": weight, "style": "normal" }))
            .collect();
        let mut presentation = minimal_json();
        presentation["fonts"] = json!({ slidock_ir::fonts::font_key(family): { "request": {
            "family": family, "faces": faces, "postscriptNames": [], "embedded": false } } });
        let presentation = passes_before("bind-fonts")
            .iter()
            .fold(parse(presentation), |p, pass| {
                apply_pass(pass, &p).presentation
            });
        let mut env = Env::of(&presentation);
        env.fonts = fonts;
        apply_pass_in(&pass_named("bind-fonts"), &presentation, &env)
    }

    #[test]
    fn a_variable_face_binds_its_instance_at_the_weight_drawn() {
        let out = bound(oswald(true));
        let binding = out.presentation.fonts["flex"].binding.as_ref().unwrap();
        let expected =
            slidock_fonts::sfnt::metrics_at(OSWALD_MVAR, 0, 700, &FontLimits::default()).unwrap();
        let faces = binding.faces.as_ref().unwrap();
        assert_eq!(faces.len(), 1);
        assert_eq!((faces[0].weight, &faces[0].metrics), (700, &expected));
        assert_eq!(binding.metrics, read_sfnt(OSWALD_MVAR).unwrap()[0].metrics);
        assert!(out.diagnostics.iter().all(|d| d.code.code() != "MID5405"));
    }

    #[test]
    fn the_instance_is_checked_at_the_weight_drawn_not_the_weight_asked() {
        // Noto Sans Hebrew's default instance is Thin. "Hebrew Flex Medium" draws its 400 runs at 500, the
        // instance the catalog read: nothing is approximated.
        const NOTO_HEBREW: &[u8] = include_bytes!(
            "../../slidock-fonts/tests/fonts/noto-sans-hebrew/NotoSansHebrew-Variable.ttf"
        );
        let sfnt = &read_sfnt(NOTO_HEBREW).unwrap()[0];
        assert_eq!(sfnt.weight, 100);
        let catalog = |weights: &[u32]| {
            let face =
                CatalogFace::from_file(sfnt, NOTO_HEBREW, 0, weights, &FontLimits::default());
            FontCatalog::merge([
                builtin_catalog(),
                &FontCatalog::build([("Hebrew Flex", face)]),
            ])
        };
        let out = bound_as(catalog(&[500]), "Hebrew Flex Medium", &[400]);
        let binding = out.presentation.fonts[&slidock_ir::fonts::font_key("Hebrew Flex Medium")]
            .binding
            .as_ref()
            .unwrap();
        assert_eq!(
            (binding.family.as_str(), binding.weight),
            ("Hebrew Flex", Some(500))
        );
        let codes: Vec<&str> = out.diagnostics.iter().map(|d| d.code.code()).collect();
        assert!(!codes.contains(&"MID5405"), "{codes:?}");
        // Without the 500 instance, the drawn weight is approximated, and the diagnostic names it.
        let out = bound_as(catalog(&[400]), "Hebrew Flex Medium", &[400]);
        let approximated: Vec<_> = out
            .diagnostics
            .iter()
            .filter(|d| d.code.code() == "MID5405")
            .collect();
        assert_eq!(approximated.len(), 1);
        assert_eq!(
            approximated[0].data.as_ref().unwrap()["weight"],
            slidock_ir::diagnostics::DataValue::Num(500.0)
        );
    }

    #[test]
    fn without_instances_it_binds_the_default_and_reports_it() {
        let out = bound(oswald(false));
        let binding = out.presentation.fonts["flex"].binding.as_ref().unwrap();
        assert_eq!(binding.faces, None);
        let codes: Vec<&str> = out.diagnostics.iter().map(|d| d.code.code()).collect();
        assert_eq!(codes, ["MID5405"]);
    }
}

#[test]
fn the_always_kept_code_points_the_text_does_not_draw_are_forced() {
    let slides = vec![slide(
        "ir-s1",
        vec![shape_with(
            "ir-s1.t",
            vec![paragraph(
                0,
                json!({}),
                // A space and a real U+2010 hyphen: drawn, so not forced.
                vec![run("A \u{2010}B", brand(json!({})))],
            )],
        )],
    )];
    let fonts = catalog();
    let faces = collect_glyphs(&resolve(slides, &fonts), &fonts).unwrap();
    let regular = face(&faces, "Brand-Regular");
    assert_eq!(regular.codepoints, drawn("A \u{2010}B"));
    let forced: Vec<u32> = regular.forced.iter().collect();
    assert_eq!(forced, [0x2d, 0xa0, 0xc0, 0x2026]);
    for point in &forced {
        assert!(regular.codepoints.contains(*point));
    }
}
