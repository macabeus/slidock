//! Hard limits on hostile input, the table cell budgets, the output budget, and the media policy.
//!
//! Limits are checked before the bytes they bound are read. The output budget bounds what the passes make of a
//! source that is small but expands (a preset into its path, a text body into cascaded runs, a gradient's steps into
//! bands): [`Output::of`] counts a container, and the pass manager leaves out a container whose output would pass
//! what the presentation has left. Time limits are cooperative: the pipeline checks a [`crate::Deadline`] between
//! stages, passes and the files a bundle writes, and frontends check it between parts and slides; hard wall-clock
//! and memory limits belong to the host.

use slidock_io::{XmlLimits, ZipLimits};
use slidock_ir::geometry::Geometry;
use slidock_ir::paint::{CellFill, Fill};
use slidock_ir::presentation::limits as ir;
use slidock_ir::source::{Container, Element};
use slidock_ir::table::{MAX_TABLE_TRACKS, table_cells};
use slidock_ir::text::TextBody;
use slidock_ir::units::Pt;

const MIB: u64 = 1024 * 1024;
const GIB: u64 = 1024 * MIB;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// The input file, before anything is read. A Keynote presentation can pass a gigabyte.
    pub input_bytes: u64,
    /// An agent-written Presentation IR file, parsed whole.
    pub ir_bytes: u64,
    pub zip: ZipLimits,
    pub xml: XmlLimits,
    /// Counted before lowering.
    pub slides: usize,
    pub elements_per_container: usize,
    /// The grid slots of all a container's tables.
    pub cells_per_container: u64,
    /// The grid slots of all the presentation's tables.
    pub cells_per_presentation: u64,
    pub group_depth: usize,
    pub text_per_body: usize,
    pub path_segments: usize,
    /// What the passes may make of every container together.
    pub output: Output,
    pub analyze_ms: u64,
    pub lower_ms: u64,
    /// The whole compile, from opening the input to the last file a bundle writes.
    pub compile_ms: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            input_bytes: 2 * GIB,
            ir_bytes: 64 * MIB,
            zip: ZipLimits::default(),
            xml: XmlLimits::default(),
            slides: ir::SLIDES,
            elements_per_container: ir::ELEMENTS_PER_CONTAINER,
            cells_per_container: ir::CELLS_PER_CONTAINER as u64,
            cells_per_presentation: ir::CELLS_PER_PRESENTATION as u64,
            group_depth: ir::GROUP_DEPTH,
            text_per_body: ir::TEXT_PER_BODY,
            path_segments: 100_000,
            output: Output::BUDGET,
            analyze_ms: 30_000,
            lower_ms: 300_000,
            compile_ms: 600_000,
        }
    }
}

/// What a container's output holds, counted where a small source can expand: elements (a group's children
/// included), path segments (geometry, clips and masks), text runs and fields (shapes, table cells and speaker
/// notes), and the gradient stops a backend prints (a gradient's stops, and two per band of a stepped one). As a
/// limit, what the presentation's containers may hold together.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub elements: u64,
    pub path_segments: u64,
    pub text_runs: u64,
    pub gradient_stops: u64,
}

impl Output {
    /// The default budget: 20 times what the largest real presentations hold (1,224 elements, 7,048 path segments,
    /// 7,552 runs) or more, and about 1 GB of memory for a hostile one that reaches it (5 slides of 4,900 star
    /// shapes, each with a gradient, a shadow and a line of text).
    pub const BUDGET: Output = Output {
        elements: 25_000,
        path_segments: 1_000_000,
        text_runs: 100_000,
        gradient_stops: 500_000,
    };

    /// What `container` holds.
    pub fn of(container: Container<'_>) -> Output {
        let mut out = Output::default();
        out.add_fill(container.background());
        out.add_elements(container.elements());
        if let Container::Slide(slide) = container {
            out.add_text(slide.notes.as_ref());
        }
        out
    }

    /// The sum, saturating.
    pub fn plus(&self, other: &Output) -> Output {
        Output {
            elements: self.elements.saturating_add(other.elements),
            path_segments: self.path_segments.saturating_add(other.path_segments),
            text_runs: self.text_runs.saturating_add(other.text_runs),
            gradient_stops: self.gradient_stops.saturating_add(other.gradient_stops),
        }
    }

    /// The first count past its limit in `limits`, by name, with the limit.
    pub fn over(&self, limits: &Output) -> Option<(&'static str, u64)> {
        [
            ("elements", self.elements, limits.elements),
            ("path segments", self.path_segments, limits.path_segments),
            ("text runs", self.text_runs, limits.text_runs),
            ("gradient stops", self.gradient_stops, limits.gradient_stops),
        ]
        .into_iter()
        .find(|(_, count, limit)| count > limit)
        .map(|(what, _, limit)| (what, limit))
    }

    fn add_elements(&mut self, elements: &[Element]) {
        for element in elements {
            self.elements = self.elements.saturating_add(1);
            match element {
                Element::Shape(shape) => {
                    self.add_geometry(shape.geometry.as_ref());
                    self.add_fill(shape.fill.as_ref());
                    self.add_text(shape.text.as_ref());
                }
                Element::Picture(picture) => {
                    self.add_geometry(picture.clip.as_ref());
                    for path in picture.mask.iter().flatten() {
                        self.add_segments(path.segments.len());
                    }
                }
                Element::Table(table) => {
                    for (cell, _, _) in table_cells(&table.rows) {
                        match &cell.fill {
                            Some(CellFill::Fill(fill)) => self.add_fill(Some(fill)),
                            Some(CellFill::Angled(angled)) => {
                                self.add_stops(angled.stops.len(), None)
                            }
                            None => {}
                        }
                        self.add_text(Some(&cell.text));
                    }
                }
                Element::Group(group) => self.add_elements(&group.children),
                Element::Media(_) | Element::Foreign(_) => {}
            }
        }
    }

    fn add_segments(&mut self, segments: usize) {
        self.path_segments = self.path_segments.saturating_add(segments as u64);
    }

    fn add_geometry(&mut self, geometry: Option<&Geometry>) {
        match geometry {
            Some(Geometry::Paths(paths)) => {
                for path in &paths.paths {
                    self.add_segments(path.segments.len());
                }
            }
            Some(Geometry::Custom(custom)) => {
                for path in &custom.paths {
                    self.add_segments(path.segments.len());
                }
            }
            Some(Geometry::Preset(_)) | None => {}
        }
    }

    fn add_text(&mut self, text: Option<&TextBody>) {
        for paragraph in text.iter().flat_map(|text| &text.paragraphs) {
            self.text_runs = self
                .text_runs
                .saturating_add(paragraph.inlines.len() as u64);
        }
    }

    fn add_stops(&mut self, stops: usize, steps: Option<u16>) {
        let bands = u64::from(steps.unwrap_or(0)).saturating_mul(stops.saturating_sub(1) as u64);
        let printed = (stops as u64).saturating_add(bands.saturating_mul(2));
        self.gradient_stops = self.gradient_stops.saturating_add(printed);
    }

    fn add_fill(&mut self, fill: Option<&Fill>) {
        let (stops, steps) = match fill {
            Some(Fill::LinearAngle(g)) => (g.stops.len(), None),
            Some(Fill::RadialSource(g)) => (g.stops.len(), None),
            Some(Fill::Path(g)) => (g.stops.len(), None),
            Some(Fill::Odf(g)) => (g.stops.len(), g.steps),
            Some(Fill::Linear(g)) => (g.stops.len(), g.steps),
            Some(Fill::Radial(g)) => (g.stops.len(), g.steps),
            Some(Fill::Rect(g)) => (g.stops.len(), g.steps),
            Some(Fill::Ellipse(g)) => (g.stops.len(), Some(g.steps)),
            Some(Fill::None | Fill::Solid { .. } | Fill::Image { .. }) | None => return,
        };
        self.add_stops(stops, steps);
    }
}

/// The grid slots the presentation's containers have left, in show order. Each container may lower at most what is
/// left when it starts, then is charged every slot its source's tables count, whether they lower, fail or a partial
/// lowering skips the container: a container's budget depends only on the source before it, so a slide lowered alone
/// lowers as in the whole presentation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CellBudget {
    pub cells: u64,
}

/// Which limit set a container's budget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BudgetHolder {
    Container,
    Presentation,
}

impl BudgetHolder {
    pub const fn as_str(self) -> &'static str {
        match self {
            BudgetHolder::Container => "container",
            BudgetHolder::Presentation => "presentation",
        }
    }
}

/// What one container's tables may still take as they lower, and the limit that set it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ContainerBudget {
    pub cells: u64,
    pub holder: BudgetHolder,
}

pub fn presentation_budget(limits: &Limits) -> CellBudget {
    CellBudget {
        cells: limits.cells_per_presentation,
    }
}

/// A container's budget: its own limit, or what the presentation has left when that is less.
pub fn container_budget(limits: &Limits, presentation: &CellBudget) -> ContainerBudget {
    if limits.cells_per_container <= presentation.cells {
        ContainerBudget {
            cells: limits.cells_per_container,
            holder: BudgetHolder::Container,
        }
    } else {
        ContainerBudget {
            cells: presentation.cells,
            holder: BudgetHolder::Presentation,
        }
    }
}

/// Charges a container's counted slots to the presentation, once the container is lowered or skipped.
pub fn charge_container(presentation: &mut CellBudget, slots: u64) {
    presentation.cells = presentation.cells.saturating_sub(slots);
}

/// The largest count a slot total reaches (`Number.MAX_SAFE_INTEGER`).
pub const MAX_SLOTS: u64 = (1 << 53) - 1;

/// `total` and `slots`, saturating: a hostile count stays a finite number.
pub fn add_slots(total: u64, slots: u64) -> u64 {
    total.saturating_add(slots).min(MAX_SLOTS)
}

/// The IR's coordinate range, which a table's summed tracks must stay within.
pub const COORDINATE_LIMIT: f64 = 100_000.0;

/// Why a container cannot hold a `rows` × `columns` grid, if it cannot: more tracks than a table holds, or more slots
/// than the container has left. Cheap, so a frontend asks before it expands a grid it has only counted.
pub fn grid_refusal(budget: &ContainerBudget, rows: u64, columns: u64) -> Option<String> {
    let tracks = MAX_TABLE_TRACKS as u64;
    if rows > tracks || columns > tracks {
        return Some(format!(
            "the table's {rows} × {columns} grid has more than {tracks} rows or columns"
        ));
    }
    if rows.saturating_mul(columns) > budget.cells {
        return Some(format!(
            "the table's {rows} × {columns} grid is more than the {} cells its {} has left",
            budget.cells,
            budget.holder.as_str()
        ));
    }
    None
}

/// Takes a grid of these tracks (in points) from its container's budget, or says why the container cannot hold it:
/// the [`grid_refusal`] reasons, or tracks that add up past the IR's coordinate range. A frontend keeps a refused
/// table Foreign, so the rest of its container lowers.
pub fn take_grid(budget: &mut ContainerBudget, widths: &[f64], heights: &[f64]) -> Option<String> {
    let (rows, columns) = (heights.len() as u64, widths.len() as u64);
    if let Some(refused) = grid_refusal(budget, rows, columns) {
        return Some(refused);
    }
    let sum = |lengths: &[f64]| slidock_math::quantise(lengths.iter().sum(), Pt::GRID);
    if sum(widths) > COORDINATE_LIMIT || sum(heights) > COORDINATE_LIMIT {
        return Some("the table's tracks add up past the IR's coordinate range".to_owned());
    }
    budget.cells -= rows * columns;
    None
}

/// What plan-media may keep: the budget data its decisions read. Lossless rasters are written as PNG: PNG is copied,
/// TIFF and BMP are transcoded to it, the same bytes on every host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MediaPolicy {
    /// One media item; larger ones are omitted.
    pub item_bytes: u64,
    /// The whole presentation; past it, the largest items are omitted last-in-show-order first.
    pub presentation_bytes: u64,
    /// One raster the compiler transcodes (TIFF and BMP), by pixel count; larger ones are omitted. Decoding one
    /// holds its RGBA (4 bytes a pixel) and its PNG.
    pub transcode_pixels: u64,
    /// The rasters the compiler transcodes, all together; past it, the later ones in the media table are omitted.
    pub presentation_transcode_pixels: u64,
    /// One medium a host decodes for the compiler (a HEIC photo), by pixel count: the host's browser decodes it, the
    /// compiler reads its header.
    pub host_decoded_pixels: u64,
}

impl Default for MediaPolicy {
    /// 25 million pixels a transcode (5,000 × 5,000; the largest real deck's TIFF has 13.7 million), 200 million for
    /// a presentation's (its real decks have at most 17.8 million), and 50 million a host decodes (a 48 MP photo).
    fn default() -> Self {
        MediaPolicy {
            item_bytes: 512 * MIB,
            presentation_bytes: 2 * GIB,
            transcode_pixels: 25_000_000,
            presentation_transcode_pixels: 200_000_000,
            host_decoded_pixels: 50_000_000,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_hold_the_documented_values() {
        let limits = Limits::default();
        assert_eq!(limits.input_bytes, 2_147_483_648);
        assert_eq!(limits.ir_bytes, 67_108_864);
        assert_eq!(limits.zip.entries, 20_000);
        assert_eq!(limits.xml.depth, 256);
        assert_eq!(limits.slides, 300);
        assert_eq!(limits.cells_per_container, 10_000);
        assert_eq!(limits.cells_per_presentation, 50_000);
        assert_eq!((limits.analyze_ms, limits.lower_ms), (30_000, 300_000));
        assert_eq!(limits.compile_ms, 600_000);
        assert_eq!(limits.output.elements, 25_000);
        assert_eq!(limits.output.path_segments, 1_000_000);
        assert_eq!(limits.output.text_runs, 100_000);
        assert_eq!(limits.output.gradient_stops, 500_000);
        let policy = MediaPolicy::default();
        assert_eq!(policy.transcode_pixels, 25_000_000);
        assert_eq!(policy.presentation_transcode_pixels, 200_000_000);
        assert_eq!(policy.host_decoded_pixels, 50_000_000);
    }

    #[test]
    fn counts_what_a_container_holds_where_a_source_expands() {
        use serde_json::json;
        let rgb = json!({ "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 });
        let frame = json!({ "x": 0, "y": 0, "w": 10, "h": 10, "rotation": 0, "flipH": false, "flipV": false });
        let rect = json!({ "kind": "paths", "paths": [{ "fill": true, "stroke": true, "fillRule": "nonzero",
            "segments": [["M", 0, 0], ["L", 10, 0], ["L", 10, 10], ["L", 0, 10], ["Z"]] }] });
        let run = |text: &str| json!({ "kind": "run", "text": text, "props": {} });
        let text = json!({ "body": {}, "paragraphs": [
            { "level": 0, "props": {}, "inlines": [run("a"), run("b")] },
            { "level": 0, "props": {}, "inlines": [run("c")] } ] });
        let stops = json!([{ "pos": 0, "color": rgb }, { "pos": 0.5, "color": rgb }, { "pos": 1, "color": rgb }]);
        let shape = |id: &str| {
            json!({ "id": id, "origin": { "kind": "agent" }, "kind": "shape", "frame": frame, "geometry": rect,
                    "text": text })
        };
        let slide = json!({
            "id": "ir-s1", "origin": { "kind": "agent" }, "timeline": [], "hidden": false, "showLayout": true,
            "background": { "kind": "linear", "from": { "x": 0, "y": 0 }, "to": { "x": 1, "y": 0 },
                            "stops": stops, "steps": 4 },
            "elements": [
                shape("ir-s1.a"),
                { "id": "ir-s1.g", "origin": { "kind": "agent" }, "kind": "group", "frame": frame,
                  "children": [shape("ir-s1.g.b"), shape("ir-s1.g.c")] },
            ],
            "notes": { "body": {}, "paragraphs": [{ "level": 0, "props": {}, "inlines": [run("n")] }] },
        });
        let slide: slidock_ir::source::Slide = slidock_ir::parse::from_value(slide).unwrap();
        let output = Output::of(Container::Slide(&slide));
        assert_eq!(
            output,
            Output {
                elements: 4,
                path_segments: 15,
                text_runs: 3 * 3 + 1,
                // Three stops, and two per band of the 4 × 2 bands.
                gradient_stops: 3 + 2 * 8,
            }
        );
        let budget = Output {
            text_runs: 10,
            ..Output::BUDGET
        };
        assert_eq!(output.over(&budget), None);
        assert_eq!(output.plus(&output).over(&budget), Some(("text runs", 10)));
        let saturated = Output {
            elements: u64::MAX,
            ..output
        };
        assert_eq!(saturated.plus(&output).elements, u64::MAX);
    }

    #[test]
    fn a_container_budget_is_its_limit_or_what_the_presentation_has_left() {
        let limits = Limits::default();
        let mut presentation = presentation_budget(&limits);
        assert_eq!(
            container_budget(&limits, &presentation),
            ContainerBudget {
                cells: 10_000,
                holder: BudgetHolder::Container
            }
        );
        charge_container(&mut presentation, 45_000);
        assert_eq!(
            container_budget(&limits, &presentation),
            ContainerBudget {
                cells: 5_000,
                holder: BudgetHolder::Presentation
            }
        );
        charge_container(&mut presentation, 6_000);
        assert_eq!(presentation.cells, 0);
        assert_eq!(add_slots(MAX_SLOTS - 1, 5), MAX_SLOTS);
        assert_eq!(add_slots(u64::MAX, u64::MAX), MAX_SLOTS);
        assert_eq!(add_slots(2, 3), 5);
    }

    #[test]
    fn refuses_grids_over_the_tracks_or_the_budget() {
        let budget = ContainerBudget {
            cells: 12,
            holder: BudgetHolder::Presentation,
        };
        assert_eq!(
            grid_refusal(&budget, 1001, 1).as_deref(),
            Some("the table's 1001 × 1 grid has more than 1000 rows or columns")
        );
        assert_eq!(
            grid_refusal(&budget, 4, 4).as_deref(),
            Some("the table's 4 × 4 grid is more than the 12 cells its presentation has left")
        );
        assert_eq!(grid_refusal(&budget, 3, 4), None);
    }

    #[test]
    fn takes_a_grid_from_the_budget_unless_its_tracks_overflow() {
        let mut budget = ContainerBudget {
            cells: 10,
            holder: BudgetHolder::Container,
        };
        assert_eq!(
            take_grid(&mut budget, &[10.0, 20.0], &[5.0, 5.0, 5.0]),
            None
        );
        assert_eq!(budget.cells, 4);
        assert_eq!(
            take_grid(&mut budget, &[60_000.0, 40_000.001], &[1.0]).as_deref(),
            Some("the table's tracks add up past the IR's coordinate range")
        );
        assert_eq!(take_grid(&mut budget, &[60_000.0, 40_000.0], &[1.0]), None);
        assert_eq!(budget.cells, 2);
        assert!(take_grid(&mut budget, &[1.0; 3], &[1.0]).is_some());
        assert_eq!(budget.cells, 2);
    }
}
