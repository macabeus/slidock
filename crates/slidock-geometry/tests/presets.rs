//! Preset shapes, and the committed vectors: every preset at six boxes and three adjustment sets, hashed.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::collections::BTreeMap;

use indexmap::IndexMap;
use serde_json::{Map, Value};
use slidock_geometry::{ShapePath, adjust_names, defined_names, preset_paths};
use slidock_ir::geometry::{PresetName, Seg};
use slidock_ir::presets::PRESET_NAMES;
use support::{adjust, digest, js, num, paths_text, rounded, rows};

fn preset(name: &str) -> PresetName {
    PresetName::parse(name).unwrap()
}

fn paths(name: &str, w: f64, h: f64, adjust: &[(&str, f64)]) -> Vec<ShapePath> {
    let adjust: IndexMap<String, f64> = adjust.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect();
    preset_paths(preset(name), w, h, &adjust)
}

#[test]
fn defines_presentation_irs_preset_names_in_its_order() {
    assert_eq!(defined_names(), PRESET_NAMES.as_slice());
}

#[test]
fn evaluates_every_preset_to_finite_paths_degenerate_boxes_included() {
    for name in PRESET_NAMES {
        for (w, h) in [(120.0, 80.0), (80.0, 120.0), (0.0, 50.0), (0.0, 0.0)] {
            let paths = paths(name, w, h, &[]);
            assert!(!paths.is_empty(), "{name}");
            for path in &paths {
                for segment in &path.segments {
                    assert!(
                        segment.coords().iter().all(|x| x.is_finite()),
                        "{name} {w}×{h}"
                    );
                }
            }
        }
    }
}

#[test]
fn matches_known_coordinates() {
    assert_eq!(
        paths("rect", 100.0, 50.0, &[]),
        [ShapePath {
            segments: vec![
                Seg::M(0.0, 0.0),
                Seg::L(100.0, 0.0),
                Seg::L(100.0, 50.0),
                Seg::L(0.0, 50.0),
                Seg::Z
            ],
            fill: true,
            stroke: true,
        }]
    );

    let round = rounded(&paths("roundRect", 100.0, 50.0, &[("adj", 20000.0)])[0].segments);
    // A 10 pt corner radius: 20% of the shorter side.
    let lines: Vec<_> = round
        .iter()
        .filter(|s| !matches!(s, Seg::C(..)))
        .copied()
        .collect();
    assert_eq!(
        lines,
        [
            Seg::M(0.0, 10.0),
            Seg::L(90.0, 0.0),
            Seg::L(100.0, 40.0),
            Seg::L(10.0, 50.0),
            Seg::Z
        ]
    );
    assert_eq!(round[1], Seg::C(0.0, 4.477, 4.477, 0.0, 10.0, 0.0));

    let ellipse = rounded(&paths("ellipse", 100.0, 50.0, &[])[0].segments);
    let ends: Vec<(&str, Vec<f64>)> = ellipse
        .iter()
        .map(|s| {
            let coords = s.coords();
            (
                s.command(),
                coords[coords.len().saturating_sub(2)..].to_vec(),
            )
        })
        .collect();
    assert_eq!(
        ends,
        [
            ("M", vec![0.0, 25.0]),
            ("C", vec![50.0, 0.0]),
            ("C", vec![100.0, 25.0]),
            ("C", vec![50.0, 50.0]),
            ("C", vec![0.0, 25.0]),
            ("Z", vec![]),
        ]
    );

    let arrow = paths(
        "rightArrow",
        200.0,
        60.0,
        &[("adj1", 50000.0), ("adj2", 50000.0)],
    );
    assert_eq!(
        rounded(&arrow[0].segments),
        [
            Seg::M(0.0, 15.0),
            Seg::L(170.0, 15.0),
            Seg::L(170.0, 0.0),
            Seg::L(200.0, 30.0),
            Seg::L(170.0, 60.0),
            Seg::L(170.0, 45.0),
            Seg::L(0.0, 45.0),
            Seg::Z
        ]
    );

    let connector = paths("bentConnector3", 100.0, 40.0, &[]);
    assert_eq!(connector.len(), 1);
    assert_eq!(
        rounded(&connector[0].segments),
        [
            Seg::M(0.0, 0.0),
            Seg::L(50.0, 0.0),
            Seg::L(50.0, 40.0),
            Seg::L(100.0, 40.0)
        ]
    );
    assert!(!connector[0].fill && connector[0].stroke);
}

#[test]
fn traces_a_cloud_in_its_own_space_so_it_closes_in_any_box() {
    for (w, h) in [(200.0, 200.0), (400.0, 100.0), (150.0, 80.0)] {
        for name in ["cloud", "cloudCallout"] {
            let outline = &paths(name, w, h, &[])[0];
            let Seg::M(mx, my) = outline.segments[0] else {
                panic!("{name}")
            };
            let Some(&Seg::C(.., x, y)) =
                outline.segments.iter().rfind(|s| matches!(s, Seg::C(..)))
            else {
                panic!("{name}")
            };
            // Within the definition's own residual, a third of a percent of its space whatever the box.
            let gap = slidock_math::hypot(x - mx, y - my);
            assert!(gap < 0.004 * w.max(h), "{name} {w}×{h}: {gap}");
        }
    }
}

#[test]
fn keeps_the_up_arrow_the_standards_file_lacks_down_arrow_upside_down() {
    let text = |segments: Vec<Seg<f64>>, flip: bool| {
        let mut out: Vec<String> = segments
            .iter()
            .map(|s| {
                let coords: Vec<String> = s
                    .coords()
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let x = if flip && i % 2 == 1 { 100.0 - x } else { *x };
                        slidock_canon::to_js_string(x)
                    })
                    .collect();
                format!("{} {}", s.command(), coords.join(","))
            })
            .collect();
        out.sort();
        out
    };
    let up = rounded(&paths("upArrow", 60.0, 100.0, &[])[0].segments);
    let down = rounded(&paths("downArrow", 60.0, 100.0, &[])[0].segments);
    assert_eq!(text(up, true), text(down, false));
}

#[test]
fn ignores_adjustments_the_preset_does_not_have_and_paints_fill_modes() {
    assert_eq!(
        paths("rect", 10.0, 10.0, &[("adj", 5.0)]),
        paths("rect", 10.0, 10.0, &[])
    );
    // Only adjustments override: a name of one of the preset's guides keeps its formula.
    assert_eq!(
        paths("roundRect", 100.0, 50.0, &[("x1", 1.0), ("a", 0.0)]),
        paths("roundRect", 100.0, 50.0, &[])
    );
    assert_ne!(
        paths("roundRect", 100.0, 50.0, &[("adj", 0.0)]),
        paths("roundRect", 100.0, 50.0, &[])
    );
    assert_eq!(adjust_names(preset("roundRect")), ["adj"]);
    assert_eq!(adjust_names(preset("rect")), Vec::<&str>::new());
    // can's top is drawn `lighten`: it fills; its outline is `fill="none"`.
    let can = paths("can", 100.0, 200.0, &[]);
    let fills: Vec<(bool, bool)> = can.iter().map(|p| (p.fill, p.stroke)).collect();
    assert_eq!(fills, [(true, false), (true, false), (false, true)]);
}

#[test]
fn matches_the_committed_vectors() {
    let cases = rows(support::PRESETS);
    let mut misses = Vec::new();
    for case in &cases {
        let name = case[0].as_str().unwrap();
        let (w, h) = (num(&case[1]), num(&case[2]));
        let paths = preset_paths(preset(name), w, h, &adjust(&case[3]));
        if digest(&paths_text(&paths)) != case[4].as_str().unwrap() {
            misses.push(format!("{name} {w}×{h} {}", case[3]));
        }
    }
    assert_eq!(misses, Vec::<String>::new());
    assert_eq!(cases.len(), 187 * 18);
}

const SIZES: [(f64, f64); 6] = [
    (120.0, 80.0),
    (80.0, 120.0),
    (0.0, 50.0),
    (0.0, 0.0),
    (1000.0, 7.5),
    (9_144_000.0, 6_858_000.0),
];

/// The generator's adjustment sets: the defaults, the defaults scaled, and hostile values with an unknown name.
fn adjust_sets(av: &[(String, f64)]) -> [IndexMap<String, f64>; 3] {
    let scaled = av
        .iter()
        .map(|(name, value)| (name.clone(), slidock_canon::js_round(value * 1.37 + 1111.0)))
        .collect();
    let mut hostile: IndexMap<String, f64> = av
        .iter()
        .zip([0.0, 100_000.0, -25_000.0, 250_000.0].into_iter().cycle())
        .map(|((name, _), value)| (name.clone(), value))
        .collect();
    hostile.insert("zzz".to_owned(), 5.0);
    [IndexMap::new(), scaled, hostile]
}

type Key = (String, String, String, Vec<(String, String)>);

fn key(name: &str, w: f64, h: f64, adjust: &IndexMap<String, f64>) -> Key {
    (
        name.to_owned(),
        js(w),
        js(h),
        adjust.iter().map(|(k, v)| (k.clone(), js(*v))).collect(),
    )
}

/// The committed vectors cover the committed table: every preset of `data/presets.json`, at six boxes and three
/// adjustment sets made from its own defaults, has a vector, and traces to it.
#[test]
fn every_preset_of_the_table_traces_to_its_vector_at_every_box_and_adjustment_set() {
    let table: Map<String, Value> =
        serde_json::from_str(include_str!("../data/presets.json")).unwrap();
    let vectors: BTreeMap<Key, String> = rows(support::PRESETS)
        .into_iter()
        .map(|row| {
            let adjust = adjust(&row[3]);
            let name = row[0].as_str().unwrap();
            (
                key(name, num(&row[1]), num(&row[2]), &adjust),
                row[4].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    let mut misses = Vec::new();
    let mut checked = 0;
    for (name, definition) in &table {
        let preset =
            PresetName::parse(name).unwrap_or_else(|| panic!("{name} is not an IR preset name"));
        let av: Vec<(String, f64)> = definition
            .get("av")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .map(|pair| {
                let formula = pair[1].as_str().unwrap();
                let value = formula.strip_prefix("val ").unwrap().parse().unwrap();
                (pair[0].as_str().unwrap().to_owned(), value)
            })
            .collect();
        for (w, h) in SIZES {
            for adjust in adjust_sets(&av) {
                let Some(expected) = vectors.get(&key(name, w, h, &adjust)) else {
                    misses.push(format!("{name} {w}×{h} {adjust:?}: no vector"));
                    continue;
                };
                if &digest(&paths_text(&preset_paths(preset, w, h, &adjust))) != expected {
                    misses.push(format!("{name} {w}×{h} {adjust:?}"));
                }
                checked += 1;
            }
        }
    }
    assert_eq!(misses, Vec::<String>::new());
    assert_eq!(table.len(), 187);
    assert_eq!(checked, 187 * 18);
}
