//! What `shadow@1` rings on a row: every measured element with a shadow in the compile's
//! IR, and every element a shadow-not-drawn diagnostic of the row names. Run shadows draw inside their text box and
//! get no ring.
//!
//! A ring surrounds what the element draws: its measured box, cut to the bounds of its mask and clip paths when the
//! element is neither turned nor flipped (a Keynote crop is a mask on the uncropped frame, so the box alone would hide
//! the shadow inside it).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::Value;
use slidock_bench_metrics::{Rect, Ring, ShadowExtent, ShadowTarget, quantise, shadow_score};
use slidock_bench_schema::compare::SHADOW_NOT_DRAWN;
use slidock_bench_schema::results::ShadowMetrics;

use crate::explain::for_each_element;
use crate::renderer::session::Measure;

/// An element shadow of the IR: page offset and σ, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IrShadow {
    pub dx: f64,
    pub dy: f64,
    pub sigma: f64,
}

/// The part of an element's frame that it draws, as fractions of the frame (0 to 1, left, top, right, bottom).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Visible {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

/// What the rings read from a compile's IR, by element id.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrShadows {
    pub shadows: BTreeMap<String, IrShadow>,
    /// Elements whose mask or clip paths leave part of an unturned, unflipped frame.
    pub visible: BTreeMap<String, Visible>,
}

/// The bounds of frame-local paths (`[{"segments": [["M", x, y], …]}, …]`) over their points and control points.
fn paths_bounds(paths: &Value) -> Option<(f64, f64, f64, f64)> {
    let mut bounds: Option<(f64, f64, f64, f64)> = None;
    for path in paths.as_array()? {
        for segment in path
            .get("segments")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
        {
            let coords: Vec<f64> = segment
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_f64)
                .collect();
            for &[x, y] in coords.as_chunks::<2>().0 {
                bounds = Some(match bounds {
                    None => (x, y, x, y),
                    Some((a, b, c, d)) => (a.min(x), b.min(y), c.max(x), d.max(y)),
                });
            }
        }
    }
    bounds
}

/// The part of its frame an element draws when its mask or clip paths leave less than the frame; none for a turned
/// or flipped frame, whose measured box is the turned frame's bounds.
pub fn visible_of(element: &Value) -> Option<Visible> {
    let frame = element.get("frame")?;
    let number = |key: &str| frame.get(key).and_then(Value::as_f64);
    let flag = |key: &str| frame.get(key).and_then(Value::as_bool).unwrap_or(false);
    let (w, h) = (number("w")?, number("h")?);
    if number("rotation").unwrap_or(0.0) != 0.0
        || flag("flipH")
        || flag("flipV")
        || w <= 0.0
        || h <= 0.0
    {
        return None;
    }
    let mask = element.get("mask").and_then(paths_bounds);
    let clip = element
        .get("clip")
        .and_then(|clip| clip.get("paths"))
        .and_then(paths_bounds);
    let (x0, y0, x1, y1) = match (mask, clip) {
        (Some(a), Some(b)) => (a.0.max(b.0), a.1.max(b.1), a.2.min(b.2), a.3.min(b.3)),
        (Some(bounds), None) | (None, Some(bounds)) => bounds,
        (None, None) => return None,
    };
    let visible = Visible {
        x0: (x0 / w).clamp(0.0, 1.0),
        y0: (y0 / h).clamp(0.0, 1.0),
        x1: (x1 / w).clamp(0.0, 1.0),
        y1: (y1 / h).clamp(0.0, 1.0),
    };
    let whole = Visible {
        x0: 0.0,
        y0: 0.0,
        x1: 1.0,
        y1: 1.0,
    };
    (visible != whole && visible.x1 > visible.x0 && visible.y1 > visible.y0).then_some(visible)
}

/// The element shadows and drawn parts of a compile's IR.
pub fn ir_shadows(ir: &Value) -> IrShadows {
    let mut out = IrShadows::default();
    for_each_element(ir, &mut |element| {
        let Some(id) = element.get("id").and_then(Value::as_str) else {
            return;
        };
        if let Some(visible) = visible_of(element) {
            out.visible.entry(id.to_owned()).or_insert(visible);
        }
        let Some(shadow) = element.get("shadow").filter(|s| s.is_object()) else {
            return;
        };
        let number = |key: &str| shadow.get(key).and_then(Value::as_f64);
        if let (Some(dx), Some(dy), Some(sigma)) = (number("dx"), number("dy"), number("sigma")) {
            out.shadows
                .entry(id.to_owned())
                .or_insert(IrShadow { dx, dy, sigma });
        }
    });
    out
}

/// The IR a compiled folder (`<run>/compiled/<pair>`) was written with: `<run>/ir/<pair>.ir.json`.
pub fn ir_path_of(compiled: &Path) -> Option<PathBuf> {
    let name = compiled.file_name()?.to_str()?;
    Some(
        compiled
            .parent()?
            .parent()?
            .join("ir")
            .join(format!("{name}.ir.json")),
    )
}

/// Reads the element shadows of the IR beside a compiled folder; a missing or unreadable IR has none.
pub fn read_ir_shadows(compiled: &Path) -> IrShadows {
    ir_path_of(compiled)
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|ir| ir_shadows(&ir))
        .unwrap_or_default()
}

/// Whether a diagnostic code reports a shadow that is not drawn.
pub fn is_not_drawn(code: &str) -> bool {
    SHADOW_NOT_DRAWN.contains(&code)
}

/// The rings of a row: the measured elements with an IR shadow or named by `not_drawn` (the nodes of the row's
/// shadow-not-drawn diagnostics), each once, in measure order. An IR shadow wins over a diagnostic.
pub fn shadow_targets(ir: &IrShadows, not_drawn: &[&str], measure: &Measure) -> Vec<ShadowTarget> {
    let mut seen = std::collections::BTreeSet::new();
    measure
        .elements
        .iter()
        .filter_map(|element| {
            let extent = match ir.shadows.get(&element.el) {
                Some(s) => ShadowExtent::Drawn {
                    dx: s.dx,
                    dy: s.dy,
                    sigma: s.sigma,
                },
                None if not_drawn.contains(&element.el.as_str()) => ShadowExtent::NotDrawn,
                None => return None,
            };
            let rect = match ir.visible.get(&element.el) {
                Some(v) => Rect {
                    x: element.x + v.x0 * element.w,
                    y: element.y + v.y0 * element.h,
                    w: (v.x1 - v.x0) * element.w,
                    h: (v.y1 - v.y0) * element.h,
                },
                None => Rect {
                    x: element.x,
                    y: element.y,
                    w: element.w,
                    h: element.h,
                },
            };
            seen.insert(element.el.as_str()).then(|| ShadowTarget {
                el: element.el.clone(),
                rect,
                extent,
            })
        })
        .collect()
}

/// One ring as `explain` reports it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RingFinding {
    pub el: String,
    pub rmse: f64,
    pub pixels: usize,
    /// The element's shadow is not drawn (a shadow-not-drawn diagnostic names it).
    pub not_drawn: bool,
}

/// The rings as `explain` reports them, worst first.
pub fn ring_findings(rings: &[Ring]) -> Vec<RingFinding> {
    rings
        .iter()
        .map(|ring| RingFinding {
            el: ring.el.clone(),
            rmse: quantise(ring.rmse),
            pixels: ring.pixels,
            not_drawn: ring.not_drawn,
        })
        .collect()
}

/// The printed rings, worst first; nothing when the row has none.
pub fn rings_text(rings: &[RingFinding]) -> String {
    if rings.is_empty() {
        return String::new();
    }
    let mut lines = vec!["shadow rings (worst first):".to_owned()];
    for ring in rings {
        lines.push(format!(
            "  {}  rmse {}  {} px{}",
            ring.el,
            slidock_canon::to_js_string(ring.rmse),
            ring.pixels,
            if ring.not_drawn { "  not drawn" } else { "" }
        ));
    }
    format!("{}\n", lines.join("\n"))
}

/// A row's record of its rings, on the results grid; none without a ring.
pub fn shadow_metrics(rings: &[Ring]) -> Option<ShadowMetrics> {
    let score = shadow_score(rings)?;
    Some(ShadowMetrics {
        elements: score.elements as u32,
        worst_rmse: quantise(score.worst_rmse),
        mean_rmse: quantise(score.mean_rmse),
        worst_el: score.worst_el,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::renderer::session::MeasuredElement;

    #[test]
    fn element_shadows_come_from_slides_layouts_and_groups() {
        let ir = serde_json::json!({
            "slides": [{"id": "s", "elements": [
                {"id": "g", "kind": "group", "children": [
                    {"id": "a", "kind": "shape", "shadow": {"dx": 1.5, "dy": 2, "sigma": 3, "color": {}}},
                ]},
                {"id": "n", "kind": "shape", "shadow": null},
                {"id": "t", "kind": "text", "paragraphs": [{"runs": [{"props": {"shadow": {"dx": 1, "dy": 1, "sigma": 0}}}]}]},
            ]}],
            "layouts": {"l": {"elements": [{"id": "p", "kind": "picture", "shadow": {"dx": 0, "dy": 3.287, "sigma": 7.14}}]}},
        });
        let shadows = ir_shadows(&ir).shadows;
        assert_eq!(
            shadows.keys().map(String::as_str).collect::<Vec<_>>(),
            ["a", "p"]
        );
        assert_eq!(
            shadows["a"],
            IrShadow {
                dx: 1.5,
                dy: 2.0,
                sigma: 3.0
            }
        );
    }

    #[test]
    fn a_mask_or_clip_cuts_an_unturned_frame_to_what_it_draws() {
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| serde_json::json!([{"fill": true, "segments": [["M", x0, y0], ["L", x1, y0], ["C", x1, y0, x1, y1, x1, y1], ["L", x0, y1], ["Z"]]}]);
        let frame = |rotation: f64, flip: bool| serde_json::json!({"x": 10, "y": 10, "w": 200, "h": 100, "rotation": rotation, "flipH": flip, "flipV": false});
        let ir = serde_json::json!({"slides": [{"id": "s", "elements": [
            // A Keynote crop: a mask inside the uncropped frame.
            {"id": "m", "kind": "picture", "frame": frame(0.0, false), "mask": rect(50.0, 10.0, 150.0, 60.0)},
            // A mask and a clip: their overlap.
            {"id": "mc", "kind": "picture", "frame": frame(0.0, false), "mask": rect(50.0, 10.0, 150.0, 60.0),
             "clip": {"kind": "paths", "paths": rect(0.0, 20.0, 100.0, 100.0)}},
            // A clip as large as the frame cuts nothing.
            {"id": "c", "kind": "picture", "frame": frame(0.0, false), "clip": {"kind": "paths", "paths": rect(0.0, 0.0, 200.0, 100.0)}},
            {"id": "turned", "kind": "picture", "frame": frame(30.0, false), "mask": rect(50.0, 10.0, 150.0, 60.0)},
            {"id": "flipped", "kind": "picture", "frame": frame(0.0, true), "mask": rect(50.0, 10.0, 150.0, 60.0)},
        ]}]});
        let ir = ir_shadows(&ir);
        assert_eq!(
            ir.visible.keys().map(String::as_str).collect::<Vec<_>>(),
            ["m", "mc"]
        );
        assert_eq!(
            ir.visible["mc"],
            Visible {
                x0: 0.25,
                y0: 0.2,
                x1: 0.5,
                y1: 0.6
            }
        );
        let measure = Measure {
            elements: vec![MeasuredElement {
                x: 10.0,
                y: 10.0,
                w: 200.0,
                h: 100.0,
                el: "m".into(),
            }],
            ..Measure::default()
        };
        let targets = shadow_targets(&ir, &["m"], &measure);
        assert_eq!(
            targets[0].rect,
            Rect {
                x: 60.0,
                y: 20.0,
                w: 100.0,
                h: 50.0
            }
        );
    }

    #[test]
    fn rings_cover_ir_and_diagnosed_shadows_on_the_page() {
        let shadows = IrShadows {
            shadows: BTreeMap::from([(
                "a".to_owned(),
                IrShadow {
                    dx: 1.0,
                    dy: 2.0,
                    sigma: 3.0,
                },
            )]),
            visible: BTreeMap::new(),
        };
        let element = |el: &str| MeasuredElement {
            x: 1.0,
            y: 2.0,
            w: 3.0,
            h: 4.0,
            el: el.into(),
        };
        let measure = Measure {
            elements: vec![element("b"), element("a"), element("c"), element("a")],
            ..Measure::default()
        };
        // `a` has an IR shadow and a diagnostic; `b` only a diagnostic; `c` neither; `z` is not on the page.
        let targets = shadow_targets(&shadows, &["a", "b", "z"], &measure);
        assert_eq!(
            targets
                .iter()
                .map(|t| (t.el.as_str(), t.extent))
                .collect::<Vec<_>>(),
            [
                ("b", ShadowExtent::NotDrawn),
                (
                    "a",
                    ShadowExtent::Drawn {
                        dx: 1.0,
                        dy: 2.0,
                        sigma: 3.0
                    }
                ),
            ]
        );
    }

    #[test]
    fn the_ir_sits_beside_the_compiled_folder() {
        let paths = crate::compile::pair_paths(Path::new("/r"), "deck", "keynote");
        assert_eq!(ir_path_of(&paths.compiled), Some(paths.ir));
    }

    #[test]
    fn explain_lists_rings_worst_first() {
        let rings = [
            Ring {
                el: "kn-2".into(),
                pixels: 812,
                rmse: 0.104_49,
                not_drawn: true,
            },
            Ring {
                el: "kn-1".into(),
                pixels: 90,
                rmse: 0.002,
                not_drawn: false,
            },
        ];
        assert_eq!(
            rings_text(&ring_findings(&rings)),
            "shadow rings (worst first):\n  kn-2  rmse 0.1045  812 px  not drawn\n  kn-1  rmse 0.002  90 px\n"
        );
        assert_eq!(rings_text(&[]), "");
    }

    #[test]
    fn metrics_are_quantised() {
        let rings = [
            Ring {
                el: "x".into(),
                pixels: 4,
                rmse: 0.123_456,
                not_drawn: false,
            },
            Ring {
                el: "y".into(),
                pixels: 4,
                rmse: 0.0,
                not_drawn: true,
            },
        ];
        assert_eq!(
            shadow_metrics(&rings),
            Some(ShadowMetrics {
                elements: 2,
                worst_rmse: 0.1235,
                mean_rmse: 0.0617,
                worst_el: "x".into(),
            })
        );
        assert_eq!(shadow_metrics(&[]), None);
    }
}
