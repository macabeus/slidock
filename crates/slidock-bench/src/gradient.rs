//! What `gradient@1` measures on a row: every measured element whose fill in the
//! compile's IR is a gradient (a table's when one of its cells' is), the slide background as `bg` when the background
//! it shows is one, and every element a gradient-approximated diagnostic of the row names (a diagnostic that names no
//! element names the background).
//!
//! An element's region is what it draws: its measured box, cut to the bounds of its mask and clip paths as the
//! shadow rings cut it. A table with gradient cells gives a region per such cell instead, `<table>#r<row>c<column>`:
//! the cell's measured box less its words, so the rest of the table and its text stay out. The background's is the
//! slide less every measured element's box.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde_json::Value;
use slidock_bench_metrics::{GradientTarget, Rect, Region, Size, gradient_score, quantise};
use slidock_bench_schema::compare::GRADIENT_APPROXIMATED;
use slidock_bench_schema::results::GradientMetrics;

use crate::explain::for_each_element;
use crate::renderer::session::{Measure, MeasuredCell};
use crate::shadow::{Visible, ir_path_of, visible_of};

/// The element id the slide background's region takes.
pub const BACKGROUND_EL: &str = "bg";

/// The fill kinds that draw a gradient, in either phase.
const GRADIENT_KINDS: [&str; 6] = ["linear", "radial", "rect", "ellipse", "path", "odf"];

/// What the regions read from a compile's IR.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrGradients {
    /// Elements whose fill, or one of whose table cells' fill, is a gradient.
    pub elements: BTreeSet<String>,
    /// Of those, the ones whose mask or clip paths leave part of an unturned, unflipped frame.
    pub visible: BTreeMap<String, Visible>,
    /// Slides whose shown background (their own, else their shown layout's, else its master's) is a gradient.
    pub backgrounds: BTreeSet<String>,
    /// Of the tables among them, the cells whose fill is a gradient.
    pub cells: BTreeMap<String, Vec<GradientCell>>,
}

/// A table cell whose fill is a gradient: where the page prints it (its row, and its place among the row's printed
/// cells, which skip the slots spans cover), and its label `r<row>c<column>`.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientCell {
    pub row: u32,
    pub index: u32,
    pub label: String,
}

fn is_gradient(fill: Option<&Value>) -> bool {
    fill.and_then(|fill| fill.get("kind"))
        .and_then(Value::as_str)
        .is_some_and(|kind| GRADIENT_KINDS.contains(&kind))
}

/// A container's background, when it prints one (`none` prints nothing).
fn printed_background(container: Option<&Value>) -> Option<&Value> {
    container
        .and_then(|c| c.get("background"))
        .filter(|fill| fill.is_object())
        .filter(|fill| fill.get("kind").and_then(Value::as_str) != Some("none"))
}

/// A table's cells whose fill is a gradient, row by row.
fn gradient_cells(element: &Value) -> Vec<GradientCell> {
    let mut out = Vec::new();
    let rows = element.get("rows").and_then(Value::as_array);
    for (r, row) in rows.into_iter().flatten().enumerate() {
        let cells = row.get("cells").and_then(Value::as_array);
        let printed = cells
            .into_iter()
            .flatten()
            .enumerate()
            .filter(|(_, cell)| !cell.is_null());
        for (index, (c, cell)) in printed.enumerate() {
            if is_gradient(cell.get("fill")) {
                out.push(GradientCell {
                    row: r as u32,
                    index: index as u32,
                    label: format!("r{r}c{c}"),
                });
            }
        }
    }
    out
}

/// The gradient elements and backgrounds of a compile's IR.
pub fn ir_gradients(ir: &Value) -> IrGradients {
    let mut out = IrGradients::default();
    for_each_element(ir, &mut |element| {
        let Some(id) = element.get("id").and_then(Value::as_str) else {
            return;
        };
        let cells = gradient_cells(element);
        if !is_gradient(element.get("fill")) && cells.is_empty() {
            return;
        }
        out.elements.insert(id.to_owned());
        if !cells.is_empty() {
            out.cells.entry(id.to_owned()).or_insert(cells);
        }
        if let Some(visible) = visible_of(element) {
            out.visible.entry(id.to_owned()).or_insert(visible);
        }
    });
    let container =
        |key: &str, id: Option<&str>| id.and_then(|id| ir.get(key).and_then(|all| all.get(id)));
    for slide in ir
        .get("slides")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
    {
        let Some(id) = slide.get("id").and_then(Value::as_str) else {
            continue;
        };
        let shows_layout = slide.get("showLayout").and_then(Value::as_bool) != Some(false);
        let layout = container("layouts", slide.get("layout").and_then(Value::as_str))
            .filter(|_| shows_layout);
        let master = container(
            "masters",
            layout.and_then(|l| l.get("master")).and_then(Value::as_str),
        );
        let shown = printed_background(Some(slide))
            .or_else(|| printed_background(layout))
            .or_else(|| layout.and_then(|_| printed_background(master)));
        if is_gradient(shown) {
            out.backgrounds.insert(id.to_owned());
        }
    }
    out
}

/// Reads the gradients of the IR beside a compiled folder; a missing or unreadable IR has none.
pub fn read_ir_gradients(compiled: &Path) -> IrGradients {
    ir_path_of(compiled)
        .and_then(|path| std::fs::read(path).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok())
        .map(|ir| ir_gradients(&ir))
        .unwrap_or_default()
}

/// Whether a diagnostic code reports an approximated gradient.
pub fn is_approximated(code: &str) -> bool {
    GRADIENT_APPROXIMATED.contains(&code)
}

/// The regions of a row's slide `slide` of `size` points: the measured gradient elements and those `approximated`
/// names (the nodes of the row's gradient-approximated diagnostics, [`BACKGROUND_EL`] for one without a node), each
/// once, in measure order, then the background.
pub fn gradient_targets(
    ir: &IrGradients,
    approximated: &[&str],
    measure: &Measure,
    slide: &str,
    size: Size,
) -> Vec<GradientTarget> {
    let mut seen = BTreeSet::new();
    let mut targets: Vec<GradientTarget> = measure
        .elements
        .iter()
        .filter(|element| {
            ir.elements.contains(&element.el) || approximated.contains(&element.el.as_str())
        })
        .filter(|element| seen.insert(element.el.as_str()))
        .flat_map(|element| {
            let approximated = approximated.contains(&element.el.as_str());
            let cells = cell_targets(ir, measure, &element.el, approximated);
            if !cells.is_empty() {
                return cells;
            }
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
            vec![GradientTarget {
                el: element.el.clone(),
                rect,
                excluded: Vec::new(),
                approximated,
            }]
        })
        .collect();
    let named = approximated.contains(&BACKGROUND_EL);
    if ir.backgrounds.contains(slide) || named {
        targets.push(GradientTarget {
            el: BACKGROUND_EL.to_owned(),
            rect: Rect {
                x: 0.0,
                y: 0.0,
                w: size.w,
                h: size.h,
            },
            excluded: measure
                .elements
                .iter()
                .map(|element| Rect {
                    x: element.x,
                    y: element.y,
                    w: element.w,
                    h: element.h,
                })
                .collect(),
            approximated: named,
        });
    }
    targets
}

fn box_of(cell: &MeasuredCell) -> Rect {
    Rect {
        x: cell.x,
        y: cell.y,
        w: cell.w,
        h: cell.h,
    }
}

fn overlaps(a: &Rect, b: &Rect) -> bool {
    a.x < b.x + b.w && b.x < a.x + a.w && a.y < b.y + b.h && b.y < a.y + a.h
}

/// A table's gradient cells as regions: each measured cell's box, less the boxes of the table's words over it. None
/// for an element that is not such a table, or whose cells were not measured.
fn cell_targets(
    ir: &IrGradients,
    measure: &Measure,
    el: &str,
    approximated: bool,
) -> Vec<GradientTarget> {
    let Some(cells) = ir.cells.get(el) else {
        return Vec::new();
    };
    let words: Vec<Rect> = measure
        .words
        .iter()
        .filter(|word| word.el.as_deref() == Some(el))
        .map(|word| Rect {
            x: word.x,
            y: word.y,
            w: word.w,
            h: word.h,
        })
        .collect();
    cells
        .iter()
        .filter_map(|cell| {
            let measured = measure
                .cells
                .iter()
                .find(|m| m.el == el && m.row == cell.row && m.index == cell.index)?;
            let rect = box_of(measured);
            Some(GradientTarget {
                el: format!("{el}#{}", cell.label),
                rect,
                excluded: words
                    .iter()
                    .filter(|word| overlaps(word, &rect))
                    .copied()
                    .collect(),
                approximated,
            })
        })
        .collect()
}

/// One region as `explain` reports it.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RegionFinding {
    pub el: String,
    pub rmse: f64,
    pub pixels: usize,
    /// A gradient-approximated diagnostic names the element.
    pub approximated: bool,
}

/// The regions as `explain` reports them, worst first.
pub fn region_findings(regions: &[Region]) -> Vec<RegionFinding> {
    regions
        .iter()
        .map(|region| RegionFinding {
            el: region.el.clone(),
            rmse: quantise(region.rmse),
            pixels: region.pixels,
            approximated: region.approximated,
        })
        .collect()
}

/// The printed regions, worst first; nothing when the row has none.
pub fn regions_text(regions: &[RegionFinding]) -> String {
    if regions.is_empty() {
        return String::new();
    }
    let mut lines = vec!["gradient regions (worst first):".to_owned()];
    for region in regions {
        lines.push(format!(
            "  {}  rmse {}  {} px{}",
            region.el,
            slidock_canon::to_js_string(region.rmse),
            region.pixels,
            if region.approximated {
                "  approximated"
            } else {
                ""
            }
        ));
    }
    format!("{}\n", lines.join("\n"))
}

/// A row's record of its regions, on the results grid; none without a region.
pub fn gradient_metrics(regions: &[Region]) -> Option<GradientMetrics> {
    let score = gradient_score(regions)?;
    Some(GradientMetrics {
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

    fn linear() -> Value {
        serde_json::json!({"kind": "linear", "from": {"x": 0, "y": 0.5}, "to": {"x": 1, "y": 0.5}, "stops": []})
    }

    #[test]
    fn gradient_elements_come_from_fills_and_table_cells_everywhere() {
        let solid = serde_json::json!({"kind": "solid", "color": {}});
        let ir = serde_json::json!({
            "slides": [{"id": "s", "elements": [
                {"id": "g", "kind": "group", "children": [
                    {"id": "a", "kind": "shape", "fill": linear()},
                ]},
                {"id": "b", "kind": "shape", "fill": solid},
                {"id": "r", "kind": "shape", "fill": {"kind": "rect"}},
                {"id": "e", "kind": "shape", "fill": {"kind": "ellipse"}},
                {"id": "c", "kind": "shape", "fill": {"kind": "radial"}},
                {"id": "t", "kind": "table", "rows": [{"cells": [
                    {"fill": solid, "borders": {"top": {"fill": linear()}}},
                    {"fill": {"kind": "linear", "angle": 90, "stops": []}},
                ]}]},
                // A table whose only gradient is a border's paint.
                {"id": "u", "kind": "table", "rows": [{"cells": [{"borders": {"top": {"fill": linear()}}}]}]},
                {"id": "n", "kind": "shape", "fill": null},
            ]}],
            "layouts": {"l": {"elements": [{"id": "p", "kind": "shape", "fill": {"kind": "path"}}]}},
            "masters": {"m": {"elements": [{"id": "o", "kind": "shape", "fill": {"kind": "odf"}}]}},
        });
        assert_eq!(
            ir_gradients(&ir)
                .elements
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["a", "c", "e", "o", "p", "r", "t"]
        );
    }

    #[test]
    fn a_slide_shows_its_own_background_else_its_layouts_else_the_masters() {
        let solid = serde_json::json!({"kind": "solid", "color": {}});
        let none = serde_json::json!({"kind": "none"});
        let ir = serde_json::json!({
            "slides": [
                {"id": "own", "layout": "plain", "background": linear(), "elements": []},
                {"id": "solid", "layout": "graded", "background": solid, "elements": []},
                {"id": "layout", "layout": "graded", "elements": []},
                {"id": "none", "layout": "graded", "background": none, "elements": []},
                {"id": "master", "layout": "plain", "elements": []},
                {"id": "hidden", "layout": "graded", "showLayout": false, "elements": []},
                {"id": "masked", "layout": "solid", "elements": []},
            ],
            "layouts": {
                "plain": {"master": "m"},
                "graded": {"master": "m", "background": {"kind": "radial"}},
                "solid": {"master": "m", "background": solid},
            },
            "masters": {"m": {"background": {"kind": "ellipse"}}},
        });
        assert_eq!(
            ir_gradients(&ir)
                .backgrounds
                .iter()
                .map(String::as_str)
                .collect::<Vec<_>>(),
            ["layout", "master", "none", "own"]
        );
    }

    fn element(el: &str, x: f64, y: f64, w: f64, h: f64) -> MeasuredElement {
        MeasuredElement {
            x,
            y,
            w,
            h,
            el: el.into(),
        }
    }

    const SIZE: Size = Size { w: 960.0, h: 540.0 };

    #[test]
    fn regions_cover_ir_gradients_named_elements_and_the_background() {
        let ir = IrGradients {
            elements: BTreeSet::from(["a".to_owned()]),
            visible: BTreeMap::from([(
                "a".to_owned(),
                Visible {
                    x0: 0.5,
                    y0: 0.0,
                    x1: 1.0,
                    y1: 0.5,
                },
            )]),
            backgrounds: BTreeSet::from(["s".to_owned()]),
            ..IrGradients::default()
        };
        let measure = Measure {
            elements: vec![
                element("b", 0.0, 0.0, 10.0, 10.0),
                element("a", 100.0, 100.0, 40.0, 20.0),
                element("c", 5.0, 5.0, 1.0, 1.0),
                element("a", 0.0, 0.0, 1.0, 1.0),
            ],
            ..Measure::default()
        };
        // `a` has an IR gradient (cut to its visible part) and a diagnostic; `b` only a diagnostic; `c` neither; `z`
        // is not on the page.
        let targets = gradient_targets(&ir, &["a", "b", "z"], &measure, "s", SIZE);
        assert_eq!(
            targets
                .iter()
                .map(|t| (t.el.as_str(), t.approximated))
                .collect::<Vec<_>>(),
            [("b", true), ("a", true), (BACKGROUND_EL, false)]
        );
        assert_eq!(
            targets[1].rect,
            Rect {
                x: 120.0,
                y: 100.0,
                w: 20.0,
                h: 10.0
            }
        );
        let background = &targets[2];
        assert_eq!(
            background.rect,
            Rect {
                x: 0.0,
                y: 0.0,
                w: 960.0,
                h: 540.0
            }
        );
        assert_eq!(background.excluded.len(), 4);
        // Another slide shows no gradient background; a node-less diagnostic names it.
        let els = |targets: &[GradientTarget]| {
            targets
                .iter()
                .map(|t| (t.el.clone(), t.approximated))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            els(&gradient_targets(&ir, &[], &measure, "t", SIZE)),
            [("a".to_owned(), false)]
        );
        assert_eq!(
            els(&gradient_targets(
                &ir,
                &[BACKGROUND_EL],
                &measure,
                "t",
                SIZE
            )),
            [("a".to_owned(), false), (BACKGROUND_EL.to_owned(), true)]
        );
    }

    #[test]
    fn a_table_gives_a_region_per_gradient_cell_less_its_words() {
        let solid = serde_json::json!({"kind": "solid", "color": {}});
        let ir = serde_json::json!({
            "slides": [{"id": "s", "elements": [
                {"id": "t", "kind": "table", "rows": [
                    {"cells": [{"fill": linear(), "colSpan": 2}, null, {"fill": solid}]},
                    {"cells": [{"fill": solid}, null, {"fill": {"kind": "radial"}}]},
                ]},
            ]}],
        });
        let ir = ir_gradients(&ir);
        let cell = |row: u32, index: u32, label: &str| GradientCell {
            row,
            index,
            label: label.into(),
        };
        assert_eq!(ir.cells["t"], [cell(0, 0, "r0c0"), cell(1, 1, "r1c2")]);
        let measured = |row: u32, index: u32, x: f64, y: f64| MeasuredCell {
            x,
            y,
            w: 100.0,
            h: 50.0,
            el: "t".into(),
            row,
            index,
        };
        let word = |el: &str, x: f64, y: f64| crate::renderer::session::MeasuredWord {
            x,
            y,
            w: 20.0,
            h: 10.0,
            text: "w".into(),
            el: Some(el.into()),
            text_box: None,
            family: "Arial".into(),
            size: 10.0,
            color: "rgb(0, 0, 0)".into(),
        };
        let mut measure = Measure {
            elements: vec![element("t", 0.0, 0.0, 300.0, 100.0)],
            words: vec![
                word("t", 210.0, 70.0),
                word("t", 10.0, 70.0),
                word("o", 220.0, 60.0),
            ],
            ..Measure::default()
        };
        // Before the cells are measured, the table's box.
        let targets = gradient_targets(&ir, &[], &measure, "s", SIZE);
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].el, "t");
        measure.cells = vec![
            measured(0, 0, 0.0, 0.0),
            measured(0, 1, 200.0, 0.0),
            measured(1, 0, 0.0, 50.0),
            measured(1, 1, 200.0, 50.0),
        ];
        let targets = gradient_targets(&ir, &["t"], &measure, "s", SIZE);
        assert_eq!(
            targets
                .iter()
                .map(|t| (t.el.as_str(), t.rect.x, t.rect.y, t.approximated))
                .collect::<Vec<_>>(),
            [("t#r0c0", 0.0, 0.0, true), ("t#r1c2", 200.0, 50.0, true)]
        );
        // The second cell leaves out the table's word over it, not the other element's or the other cell's.
        assert!(targets[0].excluded.is_empty());
        assert_eq!(targets[1].excluded.len(), 1);
        assert_eq!(targets[1].excluded[0].x, 210.0);
    }

    #[test]
    fn explain_lists_regions_worst_first() {
        let regions = [
            Region {
                el: "kn-2".into(),
                pixels: 812,
                rmse: 0.104_49,
                approximated: true,
            },
            Region {
                el: "bg".into(),
                pixels: 90,
                rmse: 0.002,
                approximated: false,
            },
        ];
        assert_eq!(
            regions_text(&region_findings(&regions)),
            "gradient regions (worst first):\n  kn-2  rmse 0.1045  812 px  approximated\n  bg  rmse 0.002  90 px\n"
        );
        assert_eq!(regions_text(&[]), "");
    }

    #[test]
    fn metrics_are_quantised() {
        let regions = [
            Region {
                el: "x".into(),
                pixels: 4,
                rmse: 0.123_456,
                approximated: false,
            },
            Region {
                el: "y".into(),
                pixels: 4,
                rmse: 0.0,
                approximated: true,
            },
        ];
        assert_eq!(
            gradient_metrics(&regions),
            Some(GradientMetrics {
                elements: 2,
                worst_rmse: 0.1235,
                mean_rmse: 0.0617,
                worst_el: "x".into(),
            })
        );
        assert_eq!(gradient_metrics(&[]), None);
    }
}
