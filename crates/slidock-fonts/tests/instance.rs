//! Variable faces: metrics at the weight drawn, against fontTools' instancer (4.60.1,
//! `instantiateVariableFont` then a save, which recomputes the glyph boxes), the catalog's axis range and instance
//! metrics, and face choice over a range.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_fonts::sfnt::metrics_at;
use slidock_fonts::{CatalogFace, FontCatalog, FontLimits, WeightRange, read_sfnt};
use slidock_ir::fonts::{FaceStyle, FontMetrics, GlyphBounds, VMetrics, WinMetrics};
use support::fixtures::*;

/// `hhea`, typo, win, head yMin/yMax and À's top, as fontTools writes an instance.
fn expected(line: [i32; 3], win: [i32; 2], bounds: [i32; 2], grave_top: i32) -> FontMetrics {
    let line = VMetrics {
        ascender: line[0],
        descender: line[1],
        line_gap: line[2],
    };
    FontMetrics {
        units_per_em: 1000,
        hhea: line.clone(),
        typo: line,
        win: WinMetrics {
            ascent: win[0],
            descent: win[1],
        },
        bounds: Some(GlyphBounds {
            y_min: bounds[0],
            y_max: bounds[1],
        }),
        grave_top: Some(grave_top),
    }
}

fn at(font: &[u8], weight: u32) -> FontMetrics {
    metrics_at(font, 0, weight, &FontLimits::default()).unwrap()
}

#[test]
fn mvar_deltas_vary_the_line_and_win_metrics_as_fonttools_does() {
    let cases = [
        (
            200,
            expected([1163, -279, 0], [1305, 372], [-196, 1052], 1052),
        ),
        (
            300,
            expected([1174, -283, 0], [1313, 374], [-190, 1054], 1054),
        ),
        (
            400,
            expected([1193, -289, 0], [1325, 377], [-194, 1056], 1056),
        ),
        (
            500,
            expected([1224, -310, 13], [1366, 392], [-192, 1066], 1066),
        ),
        (
            700,
            expected([1253, -329, 25], [1405, 407], [-200, 1075], 1075),
        ),
    ];
    for (weight, metrics) in cases {
        assert_eq!(at(OSWALD_MVAR, weight), metrics, "wght {weight}");
    }
    // The default instance is the face's own metrics, and weights past the axis clamp to its ends.
    assert_eq!(
        at(OSWALD_MVAR, 400),
        read_sfnt(OSWALD_MVAR).unwrap()[0].metrics
    );
    assert_eq!(at(OSWALD_MVAR, 900), at(OSWALD_MVAR, 700));
    assert_eq!(at(OSWALD_MVAR, 100), at(OSWALD_MVAR, 200));
}

#[test]
fn glyph_boxes_follow_the_outlines_at_the_instance() {
    let oswald = [
        (200, [-240, 1293], 1052),
        (400, [-287, 1297], 1056),
        (500, [-323, 1308], 1066),
        (700, [-357, 1318], 1075),
    ];
    for (weight, bounds, grave_top) in oswald {
        let metrics = at(OSWALD_VARIABLE, weight);
        assert_eq!(
            (metrics.bounds, metrics.grave_top),
            (
                Some(GlyphBounds {
                    y_min: bounds[0],
                    y_max: bounds[1]
                }),
                Some(grave_top)
            ),
            "Oswald wght {weight}"
        );
        // No MVAR: the lines stay.
        assert_eq!(
            metrics.hhea,
            read_sfnt(OSWALD_VARIABLE).unwrap()[0].metrics.hhea
        );
    }
    for (weight, grave_top) in [(100, 952), (400, 943), (700, 933), (900, 927)] {
        let metrics = at(NOTO_HEBREW, weight);
        assert_eq!(metrics.grave_top, Some(grave_top), "Noto wght {weight}");
        assert_eq!(
            metrics.bounds,
            Some(GlyphBounds {
                y_min: -292,
                y_max: 1068
            })
        );
    }
}

#[test]
fn a_static_face_measures_the_same_at_every_weight() {
    let own = read_sfnt(OSWALD).unwrap()[0].metrics.clone();
    for weight in [100, 400, 700, 900] {
        assert_eq!(at(OSWALD, weight), own);
    }
}

#[test]
fn glyph_boxes_keep_their_defaults_past_the_glyph_limit() {
    let few = FontLimits {
        glyphs: 10,
        ..FontLimits::default()
    };
    let metrics = metrics_at(OSWALD_MVAR, 0, 700, &few).unwrap();
    let own = read_sfnt(OSWALD_MVAR).unwrap()[0].metrics.clone();
    assert_eq!(
        (metrics.bounds, metrics.grave_top),
        (own.bounds, own.grave_top)
    );
    assert_eq!(metrics.typo, at(OSWALD_MVAR, 700).typo);
}

mod catalog {
    use super::*;

    fn variable_face(weights: &[u32]) -> CatalogFace {
        let sfnt = &read_sfnt(OSWALD_MVAR).unwrap()[0];
        CatalogFace::from_file(sfnt, OSWALD_MVAR, 0, weights, &FontLimits::default())
    }

    #[test]
    fn a_variable_face_holds_its_axis_and_the_instances_asked_for() {
        let face = variable_face(&[700, 400, 300, 900, 700]);
        let variation = face.variation.as_ref().unwrap();
        assert_eq!(variation.range, WeightRange::between(200, 700));
        assert_eq!(variation.default, 400);
        // 900 clamps to 700; the default needs no instance.
        let weights: Vec<u32> = variation.instances.iter().map(|(w, _)| *w).collect();
        assert_eq!(weights, [300, 700]);
        assert_eq!(face.metrics_at(700), Some(&at(OSWALD_MVAR, 700)));
        assert_eq!(face.metrics_at(900), Some(&at(OSWALD_MVAR, 700)));
        assert_eq!(face.metrics_at(400), Some(&face.metrics));
        assert_eq!(face.metrics_at(500), None);
    }

    #[test]
    fn without_its_file_a_variable_face_has_no_instances() {
        let sfnt = &read_sfnt(OSWALD_MVAR).unwrap()[0];
        let face = CatalogFace::from_sfnt(sfnt, None);
        assert_eq!(face.metrics_at(700), None);
        // A static face measures the same everywhere.
        let regular = CatalogFace::from_sfnt(&read_sfnt(OSWALD).unwrap()[0], None);
        assert_eq!(regular.metrics_at(700), Some(&regular.metrics));
        let from_file = CatalogFace::from_file(
            &read_sfnt(OSWALD).unwrap()[0],
            OSWALD,
            0,
            &[700],
            &FontLimits::default(),
        );
        assert_eq!(from_file.variation, None);
    }

    #[test]
    fn instances_stop_at_the_limit() {
        let sfnt = &read_sfnt(OSWALD_MVAR).unwrap()[0];
        let limits = FontLimits {
            instances: 1,
            ..FontLimits::default()
        };
        let face = CatalogFace::from_file(sfnt, OSWALD_MVAR, 0, &[300, 500, 700], &limits);
        assert_eq!(face.variation.unwrap().instances.len(), 1);
    }

    #[test]
    fn css_matching_reads_the_axis_range() {
        let mut light = CatalogFace::from_sfnt(&read_sfnt(OSWALD).unwrap()[0], None);
        light.weight = 300;
        let catalog = FontCatalog::build([("Oswald", light), ("Oswald", variable_face(&[]))]);
        let family = catalog
            .get(&slidock_fonts::FamilyKey::of("Oswald"))
            .unwrap();
        // 250 and 500 lie in the axis: the variable face; 300 is the static face's exact weight and sorts first.
        assert!(family.css_match(250, FaceStyle::Normal).unwrap().variable);
        assert!(family.css_match(500, FaceStyle::Normal).unwrap().variable);
        assert!(!family.css_match(300, FaceStyle::Normal).unwrap().variable);
        // Without the axis the variable face is a point at 400: below 400, 250 looks lighter first (none), then
        // heavier, and 300 is nearer.
        let mut pointy = variable_face(&[]);
        pointy.variation = None;
        let mut light = CatalogFace::from_sfnt(&read_sfnt(OSWALD).unwrap()[0], None);
        light.weight = 300;
        let catalog = FontCatalog::build([("Oswald", light), ("Oswald", pointy)]);
        let family = catalog
            .get(&slidock_fonts::FamilyKey::of("Oswald"))
            .unwrap();
        assert!(!family.css_match(250, FaceStyle::Normal).unwrap().variable);
    }
}
