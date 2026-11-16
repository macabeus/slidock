//! Tables: a `TST.TableInfoArchive`'s model as the IR's table, its styles resolved into each cell. The grid's track
//! sizes are the header buckets', the cells come from the tiles ([`crate::cells`]), merged ranges from the calculation
//! engine's merge owner. A cell takes its own cell and text styles, or its region's (header rows, footer rows, header
//! columns, body); body rows alternate the table style's banded fill. Each grid line segment draws the stroke set on
//! it cell by cell (the stroke sidecar), else its region's default from the table style; a segment two cells share
//! draws one stroke, which both cells state.

use std::collections::{BTreeMap, BTreeSet};

use indexmap::IndexMap;
use slidock_core_api::limits::{grid_refusal, take_grid};
use slidock_core_api::lowering::LowerError;
use slidock_ir::color::Color;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::NodeId;
use slidock_ir::paint::{CellFill, Fill};
use slidock_ir::source::Cell;
use slidock_ir::table::{Border, CellBorders, TableColumn, TableRow};
use slidock_ir::text::{Align, Autofit, PartialBodyProps, PartialInsets, TextBody, Wrap};
use slidock_ir::units::Size;

use crate::cells::{CellRecord, CellStorageError, cell_types, row_cells, value_text};
use crate::context::Lowering;
use crate::graph::ObjectGraph;
use crate::num::{clamp, finite_or, size as quantise_size};
use crate::paint::{Ends, FillBox, fill, stroke};
use crate::proto::{tsce, tsd, tsk, tsp, tst, tswp};
use crate::styles::{CellStyle, TableStyle, inherited, style_chain};
use crate::text::{DEFAULT_PADDING, FontUses, read_paragraphs, vertical};

/// A table's grid.
#[derive(Clone, Debug, PartialEq)]
pub struct TableGrid {
    pub columns: Vec<TableColumn>,
    pub rows: Vec<TableRow<Cell>>,
}

/// Why a table does not lower: one this reader cannot show (it stays a Foreign, which the core declines), or a
/// failure of its container.
#[derive(Clone, Debug, PartialEq)]
pub enum TableError {
    Unreadable(String),
    Lower(LowerError),
}

impl From<LowerError> for TableError {
    fn from(error: LowerError) -> Self {
        TableError::Lower(error)
    }
}

fn unreadable(reason: impl Into<String>) -> TableError {
    TableError::Unreadable(reason.into())
}

const DEFAULT_TILE_ROWS: u64 = 256;

fn size(x: Option<f64>) -> Size {
    quantise_size(clamp(finite_or(x, 0.0), 0.0, 100_000.0))
}

/// A table's keyed list, its segments included.
fn data_list<'g>(
    graph: &'g ObjectGraph,
    reference: Option<&tsp::Reference>,
) -> BTreeMap<u32, &'g tst::table_data_list::ListEntry> {
    let mut entries = BTreeMap::new();
    let Some(list) = graph.get::<tst::TableDataList>(reference) else {
        return entries;
    };
    let mut add = |entry: &'g tst::table_data_list::ListEntry| {
        if let Some(key) = entry.key {
            entries.insert(key, entry);
        }
    };
    list.entries.iter().for_each(&mut add);
    for segment in &list.segments {
        if let Some(segment) = graph.get::<tst::TableDataListSegment>(Some(segment)) {
            segment.entries.iter().for_each(&mut add);
        }
    }
    entries
}

/// Track sizes by index, a missing header taking the default, and how many tracks are hidden.
struct Tracks {
    sizes: Vec<Size>,
    hidden: usize,
}

fn tracks(
    buckets: &[Option<&tst::HeaderStorageBucket>],
    count: usize,
    fallback: Option<f64>,
) -> Tracks {
    let mut sizes = vec![size(fallback); count];
    let mut hidden = 0;
    for header in buckets.iter().flatten().flat_map(|bucket| &bucket.headers) {
        let Some(index) = header
            .index
            .map(|index| index as usize)
            .filter(|i| *i < count)
        else {
            continue;
        };
        if let Some(slot) = sizes.get_mut(index) {
            *slot = size(header.size.map(f64::from).or(fallback));
        }
        if header.hiding_state.unwrap_or(0) != 0 {
            hidden += 1;
        }
    }
    Tracks { sizes, hidden }
}

type Cells = BTreeMap<u64, BTreeMap<usize, CellRecord>>;

/// The cells of every tile row, by row and column.
fn tile_cells(
    graph: &ObjectGraph,
    store: &tst::DataStore,
    row_count: u64,
) -> Result<Cells, TableError> {
    let column_tiles = store
        .column_tile_tree
        .as_ref()
        .map(|tree| tree.nodes.as_slice());
    if column_tiles
        .unwrap_or_default()
        .iter()
        .any(|node| node.key.unwrap_or(0) > 0)
    {
        return Err(unreadable("the table is split into column tiles"));
    }
    let mut first_rows = BTreeMap::new();
    for node in store.row_tile_tree.iter().flat_map(|tree| &tree.nodes) {
        first_rows.insert(node.value.unwrap_or(0), node.key.unwrap_or(0));
    }
    let tile_rows = store
        .tiles
        .as_ref()
        .and_then(|tiles| tiles.tile_size)
        .map_or(DEFAULT_TILE_ROWS, u64::from);
    let mut cells = Cells::new();
    for entry in store.tiles.iter().flat_map(|tiles| &tiles.tiles) {
        let tile_id = entry.tileid.unwrap_or(0);
        let Some(tile) = graph.get::<tst::Tile>(entry.tile.as_ref()) else {
            return Err(unreadable(format!("tile {tile_id} is missing")));
        };
        let first = first_rows
            .get(&tile_id)
            .map_or(u64::from(tile_id) * tile_rows, |row| u64::from(*row));
        for info in &tile.row_infos {
            let row = first + u64::from(info.tile_row_index.unwrap_or(0));
            if row >= row_count {
                continue;
            }
            let read = row_cells(info).map_err(|CellStorageError(reason)| unreadable(reason))?;
            cells.insert(row, read);
        }
    }
    Ok(cells)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Range {
    row: i64,
    column: i64,
    rows: i64,
    columns: i64,
}

impl Range {
    fn name(&self) -> String {
        format!(
            "rows {}\u{2013}{}, columns {}\u{2013}{}",
            self.row,
            self.row + self.rows - 1,
            self.column,
            self.column + self.columns - 1
        )
    }
}

/// The formula owners by UUID; the last of a UUID wins.
fn formula_owners(
    graph: &ObjectGraph,
) -> BTreeMap<(u64, u64), &tsce::FormulaOwnerDependenciesArchive> {
    let mut owners = BTreeMap::new();
    for (_, owner, _) in graph.of_type::<tsce::FormulaOwnerDependenciesArchive>() {
        if let Some(uid) = &owner.formula_owner_uid
            && let (Some(lower), Some(upper)) = (uid.lower, uid.upper)
        {
            owners.insert((lower, upper), owner);
        }
    }
    owners
}

fn uuid_key(id: &tsp::CfuuidArchive) -> (u64, u64) {
    let half = |low: Option<u32>, high: Option<u32>| {
        u64::from(low.unwrap_or(0)) | (u64::from(high.unwrap_or(0)) << 32)
    };
    (half(id.uuid_w0, id.uuid_w1), half(id.uuid_w2, id.uuid_w3))
}

/// The table's merged ranges: the range dependencies of its merge owner, written inline and in tiles. `None` when
/// the owner is not in the document.
fn merged_ranges(
    graph: &ObjectGraph,
    owners: &BTreeMap<(u64, u64), &tsce::FormulaOwnerDependenciesArchive>,
    model: &tst::TableModelArchive,
) -> Option<Vec<Range>> {
    let Some(id) = model
        .merge_owner
        .as_ref()
        .and_then(|owner| owner.owner_id.as_ref())
    else {
        return Some(Vec::new());
    };
    let owner = owners.get(&uuid_key(id))?;
    let mut ranges = Vec::new();
    for dependency in owner
        .range_dependencies
        .iter()
        .flat_map(|dependencies| &dependencies.back_dependency)
    {
        let Some(range) = dependency
            .internal_range_reference
            .as_ref()
            .and_then(|reference| reference.range.as_ref())
        else {
            continue;
        };
        let row = i64::from(range.top_left_row.unwrap_or(0));
        let column = i64::from(range.top_left_column.unwrap_or(0));
        ranges.push(Range {
            row,
            column,
            rows: range.bottom_right_row.map_or(row, i64::from) - row + 1,
            columns: range.bottom_right_column.map_or(column, i64::from) - column + 1,
        });
    }
    for reference in owner
        .tiled_range_dependencies
        .iter()
        .flat_map(|tiled| &tiled.range_precedents_tile)
    {
        let Some(tile) = graph.get::<tsce::RangePrecedentsTileArchive>(Some(reference)) else {
            continue;
        };
        for rect in tile
            .from_to_range
            .iter()
            .filter_map(|range| range.refers_to_rect.as_ref())
        {
            let origin = rect.origin.as_ref();
            let size = rect.size.as_ref();
            ranges.push(Range {
                row: i64::from(origin.and_then(|o| o.row).unwrap_or(0)),
                column: i64::from(origin.and_then(|o| o.column).unwrap_or(0)),
                rows: i64::from(size.and_then(|s| s.num_rows).unwrap_or(1)),
                columns: i64::from(size.and_then(|s| s.num_columns).unwrap_or(1)),
            });
        }
    }
    Some(ranges)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Region {
    HeaderRow,
    FooterRow,
    HeaderColumn,
    Body,
}

/// A stroke set on a grid line segment, and the order it draws in among the segment's two.
#[derive(Clone, Copy, Debug)]
struct Run<'g> {
    stroke: &'g tsd::StrokeArchive,
    order: u32,
}

/// A sidecar's layers as runs by line index and position along the line.
struct Layers<'g> {
    lines: BTreeMap<i64, Vec<Option<Run<'g>>>>,
}

impl<'g> Layers<'g> {
    fn new(graph: &'g ObjectGraph, references: &[tsp::Reference], extent: usize) -> Self {
        let mut lines: BTreeMap<i64, Vec<Option<Run<'g>>>> = BTreeMap::new();
        for reference in references {
            let Some(layer) = graph.get::<tst::StrokeLayerArchive>(Some(reference)) else {
                continue;
            };
            let index = i64::from(layer.row_column_index.unwrap_or(0));
            let runs = lines.entry(index).or_insert_with(|| vec![None; extent]);
            for run in &layer.stroke_runs {
                let Some(stroke) = &run.stroke else {
                    continue;
                };
                let from = i64::from(run.origin.unwrap_or(0)).max(0);
                let to = (from + i64::from(run.length.unwrap_or(0))).min(extent as i64);
                for at in from..to {
                    if let Some(slot) = runs.get_mut(at as usize) {
                        *slot = Some(Run {
                            stroke,
                            order: run.order.unwrap_or(0),
                        });
                    }
                }
            }
        }
        Layers { lines }
    }

    fn at(&self, line: i64, at: usize) -> Option<Run<'g>> {
        self.lines
            .get(&line)
            .and_then(|runs| runs.get(at))
            .copied()
            .flatten()
    }
}

/// Of a segment's two runs, the one that draws: the higher order, or the second.
fn drawn<'g>(a: Option<Run<'g>>, b: Option<Run<'g>>) -> Option<Run<'g>> {
    match (a, b) {
        (Some(a), None) => Some(a),
        (Some(a), Some(b)) if a.order > b.order => Some(a),
        (_, b) => b,
    }
}

fn visible(flag: Option<&bool>) -> bool {
    flag != Some(&false)
}

/// Two decoded numbers that print the same as JSON: by value (`-0` is `0`), and every non-finite one as `null`.
fn same_number(a: Option<f32>, b: Option<f32>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a == b || !(a.is_finite() || b.is_finite()),
        (a, b) => a.is_none() && b.is_none(),
    }
}

fn same_color(a: &tsp::Color, b: &tsp::Color) -> bool {
    let tsp::Color {
        model,
        r,
        g,
        b: blue,
        rgbspace,
        a: alpha,
        c,
        m,
        y,
        k,
        w,
    } = a;
    *model == b.model
        && *rgbspace == b.rgbspace
        && [
            (r, b.r),
            (g, b.g),
            (blue, b.b),
            (alpha, b.a),
            (c, b.c),
            (m, b.m),
            (y, b.y),
            (k, b.k),
            (w, b.w),
        ]
        .into_iter()
        .all(|(a, b)| same_number(*a, b))
}

fn same_pattern(a: &tsd::StrokePatternArchive, b: &tsd::StrokePatternArchive) -> bool {
    let tsd::StrokePatternArchive {
        r#type,
        phase,
        count,
        pattern,
    } = a;
    *r#type == b.r#type
        && *count == b.count
        && same_number(*phase, b.phase)
        && pattern.len() == b.pattern.len()
        && pattern
            .iter()
            .zip(&b.pattern)
            .all(|(a, b)| same_number(Some(*a), Some(*b)))
}

/// Two optional messages that print the same as JSON, `same` comparing present ones.
fn same_message<T>(a: Option<&T>, b: Option<&T>, same: fn(&T, &T) -> bool) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => same(a, b),
        (a, b) => a.is_none() && b.is_none(),
    }
}

/// Whether two strokes print the same as JSON, the way the stroke sidecar's segments are told apart.
fn same_stroke(a: &tsd::StrokeArchive, b: &tsd::StrokeArchive) -> bool {
    let tsd::StrokeArchive {
        color,
        width,
        cap,
        join,
        miter_limit,
        pattern,
    } = a;
    *cap == b.cap
        && *join == b.join
        && same_number(*width, b.width)
        && same_number(*miter_limit, b.miter_limit)
        && same_message(color.as_ref(), b.color.as_ref(), same_color)
        && same_message(pattern.as_ref(), b.pattern.as_ref(), same_pattern)
}

/// The cell types natural alignment sets to the right.
const NUMERIC: [u8; 3] = [
    cell_types::NUMBER,
    cell_types::CURRENCY,
    cell_types::DURATION,
];

fn fill_feature(fill: &Fill) -> Option<FeatureId> {
    match fill.kind() {
        "solid" => Some(FeatureId::FillSolid),
        "linear" => Some(FeatureId::FillLinear),
        "radial" => Some(FeatureId::FillRadial),
        "image" => Some(FeatureId::FillImage),
        _ => None,
    }
}

/// The table style's properties along its chain.
struct TableProps<'g> {
    chain: Vec<&'g tst::TableStylePropertiesArchive>,
}

impl<'g> TableProps<'g> {
    fn stroke(
        &self,
        get: impl Fn(&'g tst::TableStylePropertiesArchive) -> Option<&'g tsd::StrokeArchive>,
    ) -> Option<&'g tsd::StrokeArchive> {
        inherited(&self.chain, |p| (get(p), None))
    }

    fn flag(
        &self,
        get: impl Fn(&'g tst::TableStylePropertiesArchive) -> Option<&'g bool>,
    ) -> Option<&'g bool> {
        inherited(&self.chain, |p| (get(p), None))
    }
}

/// Lowers a table's model, or says why it cannot.
pub fn read_table(
    ctx: &mut Lowering<'_>,
    fonts: &mut FontUses<'_>,
    info: &tst::TableInfoArchive,
    node: &NodeId,
) -> Result<TableGrid, TableError> {
    let graph = ctx.graph;
    let model = graph
        .get::<tst::TableModelArchive>(info.table_model.as_ref())
        .ok_or_else(|| unreadable("the table model is missing"))?;
    let row_count = u64::from(model.number_of_rows.unwrap_or(0));
    let column_count = u64::from(model.number_of_columns.unwrap_or(0));
    if row_count < 1 || column_count < 1 {
        return Err(unreadable("the table has no cells"));
    }
    if let Some(counted) = grid_refusal(&ctx.budget, row_count, column_count) {
        return Err(unreadable(counted));
    }
    // At most 1000 each, by the refusal above.
    let (rows_n, columns_n) = (row_count as usize, column_count as usize);
    let store = model
        .base_data_store
        .as_ref()
        .ok_or_else(|| unreadable("the table has no data store"))?;
    let columns = tracks(
        &[graph.get::<tst::HeaderStorageBucket>(store.column_headers.as_ref())],
        columns_n,
        model.default_column_width,
    );
    let row_buckets: Vec<Option<&tst::HeaderStorageBucket>> = store
        .row_headers
        .iter()
        .flat_map(|headers| &headers.buckets)
        .map(|bucket| graph.get::<tst::HeaderStorageBucket>(Some(bucket)))
        .collect();
    let rows = tracks(&row_buckets, rows_n, model.default_row_height);
    let cells = tile_cells(graph, store, row_count)?;
    let widths: Vec<f64> = columns.sizes.iter().map(|s| s.to_f64()).collect();
    let heights: Vec<f64> = rows.sizes.iter().map(|s| s.to_f64()).collect();
    // Taken last of all that can refuse the table, so only tables that lower count against the container's budget.
    if let Some(refused) = take_grid(&mut ctx.budget, &widths, &heights) {
        return Err(unreadable(refused));
    }
    let report = |ctx: &mut Lowering<'_>, what: String| {
        ctx.report(
            codes::KN1112,
            DiagnosticSite::new().node(node.clone()).data("what", what),
        );
    };
    if columns.hidden + rows.hidden > 0 {
        report(ctx, "hidden rows and columns are shown".to_owned());
    }
    if model.table_name_enabled == Some(true) {
        ctx.report(
            codes::KN1109,
            DiagnosticSite::new()
                .node(node.clone())
                .data("what", "table name"),
        );
    }

    let strings = data_list(graph, store.string_table.as_ref());
    let styles = data_list(graph, store.style_table.as_ref());
    let rich_texts = data_list(graph, store.rich_text_table.as_ref());
    let formats = data_list(graph, store.format_table.as_ref());
    let style = |key: Option<u32>| {
        key.and_then(|key| styles.get(&key))
            .and_then(|e| e.reference.as_ref())
    };

    // The grid: each merged range's slots, its anchor first. A range is checked against the grid before its slots are
    // visited, so each costs at most the grid's slots, whatever area the file claims for it.
    let mut spans: BTreeMap<(i64, i64), Range> = BTreeMap::new();
    let mut covered = vec![false; rows_n * columns_n];
    let slot = |r: i64, c: i64| (r * column_count as i64 + c) as usize;
    let owners = formula_owners(graph);
    let ranges = merged_ranges(graph, &owners, model);
    if ranges.is_none() {
        report(ctx, "its merged cells are not read".to_owned());
    }
    let mut merged: IndexMap<String, Range> = IndexMap::new();
    for range in ranges.unwrap_or_default() {
        merged.insert(range.name(), range);
    }
    let mut merged: Vec<Range> = merged.into_values().collect();
    merged.sort_by_key(|range| (range.row, range.column));
    for range in merged {
        if range.rows == 1 && range.columns == 1 {
            continue;
        }
        let inside = range.rows >= 1
            && range.columns >= 1
            && range.row + range.rows <= row_count as i64
            && range.column + range.columns <= column_count as i64;
        let overlaps = inside
            && (range.row..range.row + range.rows).any(|r| {
                (range.column..range.column + range.columns)
                    .any(|c| covered.get(slot(r, c)).copied().unwrap_or(false))
            });
        if !inside || overlaps {
            report(
                ctx,
                format!(
                    "merged {} overlaps another or leaves the grid; its cells stay apart",
                    range.name()
                ),
            );
            continue;
        }
        for r in range.row..range.row + range.rows {
            for c in range.column..range.column + range.columns {
                if let Some(slot) = covered.get_mut(slot(r, c)) {
                    *slot = true;
                }
            }
        }
        spans.insert((range.row, range.column), range);
    }

    // Regions and their styles.
    let header_rows = u64::from(model.number_of_header_rows.unwrap_or(0)).min(row_count);
    let footer_rows =
        u64::from(model.number_of_footer_rows.unwrap_or(0)).min(row_count - header_rows);
    let header_columns = u64::from(model.number_of_header_columns.unwrap_or(0)).min(column_count);
    let region = |r: u64, c: u64| {
        if r < header_rows {
            Region::HeaderRow
        } else if r >= row_count - footer_rows {
            Region::FooterRow
        } else if c < header_columns {
            Region::HeaderColumn
        } else {
            Region::Body
        }
    };
    let region_styles = |region: Region| match region {
        Region::HeaderRow => (&model.header_row_style, &model.header_row_text_style),
        Region::FooterRow => (&model.footer_row_style, &model.footer_row_text_style),
        Region::HeaderColumn => (&model.header_column_style, &model.header_column_text_style),
        Region::Body => (&model.body_cell_style, &model.body_text_style),
    };
    let table = TableProps {
        chain: style_chain::<TableStyle>(graph, model.table_style.as_ref()),
    };

    // Grid line segments: horizontal line y under column c, vertical line x beside row r.
    let sidecar = graph.get::<tst::StrokeSidecarArchive>(model.stroke_sidecar.as_ref());
    let layers = |get: fn(&tst::StrokeSidecarArchive) -> &Vec<tsp::Reference>, extent| {
        Layers::new(
            graph,
            sidecar.map(get).map_or(&[][..], Vec::as_slice),
            extent,
        )
    };
    let tops = layers(|s| &s.top_row_stroke_layers, columns_n);
    let bottoms = layers(|s| &s.bottom_row_stroke_layers, columns_n);
    let lefts = layers(|s| &s.left_column_stroke_layers, rows_n);
    let rights = layers(|s| &s.right_column_stroke_layers, rows_n);
    let horizontal_default = |y: u64, c: u64| -> Option<&tsd::StrokeArchive> {
        if y == 0 || y == row_count {
            if !visible(table.flag(|p| p.table_border_visible.as_ref())) {
                return None;
            }
            if y == 0 && header_rows > 0 {
                return table.stroke(|p| p.header_row_border_stroke.as_ref());
            }
            if y == row_count && footer_rows > 0 {
                return table.stroke(|p| p.footer_row_border_stroke.as_ref());
            }
            return if c < header_columns {
                table.stroke(|p| p.header_column_border_stroke.as_ref())
            } else {
                table.stroke(|p| p.table_body_horizontal_border_stroke.as_ref())
            };
        }
        if header_rows > 0 && y == header_rows {
            return if visible(table.flag(|p| p.hr_separator_visible.as_ref())) {
                table.stroke(|p| p.header_row_separator_stroke.as_ref())
            } else {
                None
            };
        }
        if footer_rows > 0 && y == row_count - footer_rows {
            return if visible(table.flag(|p| p.footer_separator_visible.as_ref())) {
                table.stroke(|p| p.footer_row_separator_stroke.as_ref())
            } else {
                None
            };
        }
        if !visible(table.flag(|p| p.h_strokes_visible.as_ref())) {
            return None;
        }
        if y < header_rows {
            return table.stroke(|p| p.header_row_horizontal_stroke.as_ref());
        }
        if y > row_count - footer_rows {
            return table.stroke(|p| p.footer_row_horizontal_stroke.as_ref());
        }
        if c < header_columns {
            table.stroke(|p| p.header_column_horizontal_stroke.as_ref())
        } else {
            table.stroke(|p| p.table_body_horizontal_stroke.as_ref())
        }
    };
    let vertical_default = |x: u64, r: u64| -> Option<&tsd::StrokeArchive> {
        let header = r < header_rows;
        let footer = !header && r >= row_count - footer_rows;
        if x == 0 || x == column_count {
            if !visible(table.flag(|p| p.table_border_visible.as_ref())) {
                return None;
            }
            if header {
                return table.stroke(|p| p.header_row_border_stroke.as_ref());
            }
            if footer {
                return table.stroke(|p| p.footer_row_border_stroke.as_ref());
            }
            let in_header_column = if x == 0 {
                header_columns > 0
            } else {
                header_columns >= column_count
            };
            return if in_header_column {
                table.stroke(|p| p.header_column_border_stroke.as_ref())
            } else {
                table.stroke(|p| p.table_body_vertical_border_stroke.as_ref())
            };
        }
        if header_columns > 0 && x == header_columns && !header && !footer {
            return if visible(table.flag(|p| p.hc_separator_visible.as_ref())) {
                table.stroke(|p| p.header_column_separator_stroke.as_ref())
            } else {
                None
            };
        }
        if !visible(table.flag(|p| p.v_strokes_visible.as_ref())) {
            return None;
        }
        if header {
            return table.stroke(|p| p.header_row_vertical_stroke.as_ref());
        }
        if footer {
            return table.stroke(|p| p.footer_row_vertical_stroke.as_ref());
        }
        if x < header_columns {
            table.stroke(|p| p.header_column_vertical_stroke.as_ref())
        } else {
            table.stroke(|p| p.table_body_vertical_stroke.as_ref())
        }
    };
    let horizontal = |y: u64, c: u64| {
        drawn(
            bottoms.at(y as i64 - 1, c as usize),
            tops.at(y as i64, c as usize),
        )
        .map(|run| run.stroke)
        .or_else(|| horizontal_default(y, c))
    };
    let vertical_at = |x: u64, r: u64| {
        drawn(
            rights.at(x as i64 - 1, r as usize),
            lefts.at(x as i64, r as usize),
        )
        .map(|run| run.stroke)
        .or_else(|| vertical_default(x, r))
    };
    let mut mixed_edge = false;
    // One cell edge's stroke: its segments', the first when they differ.
    let mut edge = |ctx: &mut Lowering<'_>,
                    segments: Vec<Option<&tsd::StrokeArchive>>|
     -> Option<Border<Color>> {
        let first = segments.first().copied().flatten();
        if !mixed_edge
            && !segments
                .iter()
                .all(|segment| same_message(*segment, first, same_stroke))
        {
            mixed_edge = true;
            ctx.report(
                codes::KN1112,
                DiagnosticSite::new().node(node.clone()).data(
                    "what",
                    "a merged cell's edge draws its first segment's stroke along its length",
                ),
            );
        }
        let line = stroke(first, Ends::default(), ctx).flatten()?;
        Some(Border {
            width: line.width,
            fill: line.fill,
            dash: line.dash,
            cap: line.cap,
            join: line.join,
        })
    };

    let banded = if table.flag(|p| p.banded_rows.as_ref()) == Some(&true) {
        let banded_fill = inherited(&table.chain, |p| (p.banded_fill.as_ref(), None));
        fill(banded_fill, FillBox::Cell, ctx, Some(node))
    } else {
        None
    };
    let format_of = |key: Option<u32>| -> Option<&tsk::FormatStructArchive> {
        key.and_then(|key| formats.get(&key))
            .and_then(|entry| entry.format.as_ref())
    };
    let mut approximated: BTreeSet<String> = BTreeSet::new();
    let mut unknown_types: BTreeSet<u8> = BTreeSet::new();
    let mut conditional = false;

    let mut out: Vec<TableRow<Cell>> = Vec::with_capacity(rows_n);
    for r in 0..row_count {
        let mut row: Vec<Option<Cell>> = Vec::with_capacity(columns_n);
        for c in 0..column_count {
            let (ri, ci) = (r as i64, c as i64);
            let range = spans.get(&(ri, ci)).copied();
            if covered.get(slot(ri, ci)).copied().unwrap_or(false) && range.is_none() {
                row.push(None);
                continue;
            }
            let empty = CellRecord::empty();
            let cell = cells
                .get(&r)
                .and_then(|row| row.get(&(c as usize)))
                .unwrap_or(&empty);
            let (region_cell, region_text) = region_styles(region(r, c));
            let chain =
                style_chain::<CellStyle>(graph, style(cell.cell_style).or(region_cell.as_ref()));
            if cell.conditional_style.is_some() {
                conditional = true;
            }

            let mut storage: Option<&tswp::StorageArchive> = None;
            let mut natural = Align::Left;
            if cell.cell_type == cell_types::RICH_TEXT {
                let payload = graph.get::<tst::RichTextPayloadArchive>(
                    cell.rich_text
                        .and_then(|key| rich_texts.get(&key))
                        .and_then(|entry| entry.rich_text_payload.as_ref()),
                );
                storage =
                    graph.get::<tswp::StorageArchive>(payload.and_then(|p| p.storage.as_ref()));
                if storage.is_none() {
                    report(
                        ctx,
                        format!("the rich text of the cell at row {r}, column {c} is missing"),
                    );
                }
            }
            let built;
            let storage = match storage {
                Some(storage) => storage,
                None => {
                    let mut text = String::new();
                    if cell.cell_type == cell_types::TEXT {
                        let entry = cell.string.and_then(|key| strings.get(&key));
                        match entry.and_then(|entry| entry.string.as_ref()) {
                            Some(string) => text.clone_from(string),
                            None => report(
                                ctx,
                                format!("the text of the cell at row {r}, column {c} is missing"),
                            ),
                        }
                    } else if cell.cell_type != cell_types::RICH_TEXT {
                        let value = value_text(cell, format_of);
                        if value.is_none() && unknown_types.insert(cell.cell_type) {
                            report(
                                ctx,
                                format!(
                                    "the values of its type {} cells are not read",
                                    cell.cell_type
                                ),
                            );
                        }
                        if let Some(format) = value.as_ref().and_then(|v| v.approximated.as_ref())
                            && approximated.insert(format.clone())
                        {
                            ctx.report(
                                codes::KN1111,
                                DiagnosticSite::new()
                                    .node(node.clone())
                                    .data("format", format.as_str()),
                            );
                        }
                        if let Some(value) = value {
                            text = value.text;
                        }
                        // Natural alignment sets numbers to the right.
                        if NUMERIC.contains(&cell.cell_type) {
                            natural = Align::Right;
                        }
                    }
                    let text_style = style(cell.text_style).or(region_text.as_ref()).cloned();
                    built = tswp::StorageArchive {
                        text: vec![text],
                        table_para_style: text_style.map(|object| tswp::ObjectAttributeTable {
                            entries: vec![tswp::object_attribute_table::ObjectAttribute {
                                character_index: Some(0),
                                object: Some(object),
                            }],
                        }),
                        ..tswp::StorageArchive::default()
                    };
                    &built
                }
            };
            let paragraphs = read_paragraphs(ctx, storage, node, fonts, natural)?;

            let padding = inherited(&chain, |p| (p.padding.as_ref(), None));
            let side = |value: Option<f32>| {
                size(Some(value.map_or(
                    if padding.is_some() {
                        0.0
                    } else {
                        DEFAULT_PADDING
                    },
                    f64::from,
                )))
            };
            let mut cell_fill = fill(
                inherited(&chain, |p| (p.cell_fill.as_ref(), None)),
                FillBox::Cell,
                ctx,
                Some(node),
            );
            if cell_fill == Some(Fill::None) {
                cell_fill = None;
            }
            if cell_fill.is_none()
                && let Some(banded) = &banded
                && *banded != Fill::None
                && region(r, c) == Region::Body
                && (r - header_rows) % 2 == 1
            {
                cell_fill = Some(banded.clone());
            }
            if let Some(feature) = cell_fill.as_ref().and_then(fill_feature) {
                ctx.see(feature);
            }

            let row_span = range.map_or(1, |range| range.rows as u64);
            let col_span = range.map_or(1, |range| range.columns as u64);
            let top = (0..col_span).map(|i| horizontal(r, c + i)).collect();
            let top = edge(ctx, top);
            let right = (0..row_span)
                .map(|i| vertical_at(c + col_span, r + i))
                .collect();
            let right = edge(ctx, right);
            let bottom = (0..col_span)
                .map(|i| horizontal(r + row_span, c + i))
                .collect();
            let bottom = edge(ctx, bottom);
            let left = (0..row_span).map(|i| vertical_at(c, r + i)).collect();
            let left = edge(ctx, left);
            // Keynote pads a cell from its strokes' inner edges; the IR's insets are from the grid lines.
            let inset = |value: Option<f32>, border: &Option<Border<Color>>| {
                let width = border.as_ref().map_or(0.0, |b| b.width.to_f64());
                size(Some(side(value).to_f64() + width / 2.0)).into()
            };
            let insets = PartialInsets {
                l: Some(inset(padding.and_then(|p| p.left), &left)),
                t: Some(inset(padding.and_then(|p| p.top), &top)),
                r: Some(inset(padding.and_then(|p| p.right), &right)),
                b: Some(inset(padding.and_then(|p| p.bottom), &bottom)),
            };
            let any_border = top.is_some() || right.is_some() || bottom.is_some() || left.is_some();
            let borders = any_border.then_some(CellBorders {
                top,
                right,
                bottom,
                left,
                diagonal_down: None,
                diagonal_up: None,
            });
            let wrap = if inherited(&chain, |p| (p.text_wrap.as_ref(), None)) == Some(&false) {
                Wrap::None
            } else {
                Wrap::Square
            };
            row.push(Some(Cell {
                col_span: (col_span > 1).then_some(col_span as u32),
                row_span: (row_span > 1).then_some(row_span as u32),
                fill: cell_fill.map(CellFill::Fill),
                borders,
                text: TextBody {
                    body: PartialBodyProps {
                        insets: Some(insets),
                        anchor: Some(vertical(
                            inherited(&chain, |p| (p.vertical_alignment.as_ref(), None)).copied(),
                        )),
                        wrap: Some(wrap),
                        autofit: Some(Autofit::None {}),
                        overflow: None,
                        max_width: None,
                    },
                    list_style: None,
                    paragraphs,
                    // As a drawable's text, mirrored by its flips.
                    flips_with_frame: true,
                    rotation: None,
                },
            }));
        }
        out.push(TableRow {
            height: rows.sizes.get(r as usize).copied().unwrap_or(Size::ZERO),
            cells: row,
        });
    }
    if conditional {
        report(ctx, "conditional highlighting is not applied".to_owned());
    }
    ctx.see(FeatureId::ElementTable);
    Ok(TableGrid {
        columns: columns
            .sizes
            .into_iter()
            .map(|width| TableColumn { width })
            .collect(),
        rows: out,
    })
}
