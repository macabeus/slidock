//! A table: an HTML table in the grid's widths, each row at its least height (rows grow to fit their text), each cell
//! with its insets as padding, its anchor as vertical alignment, its fill as background and its edges as collapsed
//! borders. Every fill paints the cell as drawn: an angled gradient as a CSS background, one in box fractions in an SVG
//! layer stretched over the cell, under its diagonals and its text.

use indexmap::IndexMap;
use slidock_canon::to_js_string;
use slidock_format::markers::GEOMETRY;
use slidock_format::paths::file_id;
use slidock_ir::color::ResolvedColor;
use slidock_ir::diagnostics::codes;
use slidock_ir::paint::{ResolvedCellFill, ResolvedFill};
use slidock_ir::resolved::{Cell, Table};
use slidock_ir::table::Border;
use slidock_ir::text::Anchor;
use slidock_ir::units::Pt;

use crate::attributes::{Kind, Root, common, element_attributes};
use crate::context::PrintCtx;
use crate::css::{Decl, MaybeDecl, decl, deg, pt, sides, style};
use crate::error::Result;
use crate::frame::{box_decls, opacity_decl, turn_decls};
use crate::gradient::{PaintBox, css_stops};
use crate::paint::{
    background_decls, dash_pattern, stroke_attributes, stroke_color, strokes, svg_fill,
};
use crate::svg::{path_data, rect, svg_layer};
use crate::text::{print_cell_text, text_element};
use crate::tree::{Attrs, Element, Node, lines};

type ResolvedBorder = Border<ResolvedColor>;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    Top,
    Right,
    Bottom,
    Left,
}

const SIDES: [Side; 4] = [Side::Top, Side::Right, Side::Bottom, Side::Left];

impl Side {
    fn property(self) -> &'static str {
        match self {
            Side::Top => "border-top",
            Side::Right => "border-right",
            Side::Bottom => "border-bottom",
            Side::Left => "border-left",
        }
    }

    fn opposite(self) -> Side {
        match self {
            Side::Top => Side::Bottom,
            Side::Right => Side::Left,
            Side::Bottom => Side::Top,
            Side::Left => Side::Right,
        }
    }

    fn of(self, cell: &Cell) -> Option<&ResolvedBorder> {
        let borders = cell.borders.as_ref()?;
        match self {
            Side::Top => borders.top.as_ref(),
            Side::Right => borders.right.as_ref(),
            Side::Bottom => borders.bottom.as_ref(),
            Side::Left => borders.left.as_ref(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum BorderStyle {
    Solid,
    Dotted,
    Dashed,
}

impl BorderStyle {
    fn name(self) -> &'static str {
        match self {
            BorderStyle::Solid => "solid",
            BorderStyle::Dotted => "dotted",
            BorderStyle::Dashed => "dashed",
        }
    }

    /// The pattern the CSS style draws, in multiples of the width: Chromium's dots are as long as their gaps, and its
    /// dashes change their proportions with the width, so no dash pattern is drawn exactly as dashed.
    fn drawn(self) -> Option<&'static str> {
        match self {
            BorderStyle::Solid => Some(""),
            BorderStyle::Dotted => Some("1 1"),
            BorderStyle::Dashed => None,
        }
    }
}

/// The CSS border style of a dash pattern: dotted when no dash is longer than the line is wide, else dashed.
fn border_style(pattern: &[f64]) -> BorderStyle {
    if pattern.is_empty() {
        return BorderStyle::Solid;
    }
    let dotted = pattern
        .iter()
        .enumerate()
        .all(|(index, length)| index % 2 == 1 || *length <= 1.0);
    if dotted {
        BorderStyle::Dotted
    } else {
        BorderStyle::Dashed
    }
}

/// A cell with its grid position and spans.
#[derive(Clone, Copy)]
struct Placed<'a> {
    cell: &'a Cell,
    row: usize,
    column: usize,
    rows: usize,
    columns: usize,
}

type Grid<'a> = Vec<Vec<Option<Placed<'a>>>>;

/// Each grid slot's covering cell, so a cell finds the neighbours across each of its edges.
fn cover_grid(table: &Table) -> Grid<'_> {
    let width = table.columns.len();
    let mut grid: Grid<'_> = table.rows.iter().map(|_| vec![None; width]).collect();
    for (cell, row, column) in slidock_ir::table::table_cells(&table.rows) {
        let (columns, rows) = cell.spans();
        let placed = Placed {
            cell,
            row,
            column,
            rows,
            columns,
        };
        for r in row..row + rows {
            for c in column..column + columns {
                if let Some(slot) = grid.get_mut(r).and_then(|line| line.get_mut(c)) {
                    *slot = Some(placed);
                }
            }
        }
    }
    grid
}

fn slot<'a>(grid: &Grid<'a>, row: Option<usize>, column: Option<usize>) -> Option<Placed<'a>> {
    *grid.get(row?)?.get(column?)?
}

fn edge_width(border: Option<&ResolvedBorder>) -> f64 {
    strokes(border).map_or(0.0, |border| border.width.to_f64())
}

/// The slots across one side of a placed cell.
fn across(placed: &Placed<'_>, side: Side) -> Vec<(Option<usize>, Option<usize>)> {
    let Placed {
        row,
        column,
        rows,
        columns,
        ..
    } = *placed;
    match side {
        Side::Top | Side::Bottom => {
            let r = if side == Side::Top {
                row.checked_sub(1)
            } else {
                Some(row + rows)
            };
            (0..columns).map(|k| (r, Some(column + k))).collect()
        }
        Side::Left | Side::Right => {
            let c = if side == Side::Left {
                column.checked_sub(1)
            } else {
                Some(column + columns)
            };
            (0..rows).map(|k| (Some(row + k), c)).collect()
        }
    }
}

/// The width of the collapsed border on one side: the widest of the cell's own edge and its neighbours' edges facing
/// it, as CSS resolves a shared edge.
fn collapsed_width(placed: &Placed<'_>, side: Side, grid: &Grid<'_>) -> f64 {
    across(placed, side)
        .into_iter()
        .map(|(row, column)| {
            edge_width(slot(grid, row, column).and_then(|facing| side.opposite().of(facing.cell)))
        })
        .fold(edge_width(side.of(placed.cell)), f64::max)
}

/// The dash patterns a table's borders approximate, with the CSS style each prints as, reported once each per table.
type Approximated = IndexMap<String, BorderStyle>;

fn border_decl(
    side: Side,
    border: Option<&ResolvedBorder>,
    approximated: &mut Approximated,
) -> Result<MaybeDecl> {
    let Some(border) = strokes(border) else {
        return Ok(None);
    };
    let pattern = dash_pattern(&border.dash);
    let css = border_style(&pattern);
    let written = pattern
        .iter()
        .map(|length| to_js_string(*length))
        .collect::<Vec<_>>()
        .join(" ");
    if css.drawn() != Some(written.as_str()) {
        approximated.insert(written, css);
    }
    Ok(decl(
        side.property(),
        format!(
            "{} {} {}",
            pt(border.width.to_f64())?,
            css.name(),
            stroke_color(border)?
        ),
    ))
}

/// A diagonal from corner to corner, in percentages of the cell, so it follows the cell as its row grows.
fn diagonal(border: &ResolvedBorder, down: bool) -> Result<Element> {
    Ok(Element::new(
        "line",
        Attrs::new()
            .val("x1", "0".to_owned())
            .val("y1", if down { "0" } else { "100%" }.to_owned())
            .val("x2", "100%".to_owned())
            .val("y2", if down { "100%" } else { "0" }.to_owned())
            .with(stroke_attributes(border, pt)?)
            .val("style", format!("stroke:{}", stroke_color(border)?)),
        Vec::new(),
    ))
}

/// A cell's background: a solid or image fill, or a gradient held by its angle (CSS's, a quarter turn on), its stops
/// expanded where CSS would blend otherwise than SVG ([`css_stops`]).
fn cell_background(fill: Option<&ResolvedCellFill>, ctx: &mut PrintCtx<'_>) -> Result<Vec<Decl>> {
    match fill {
        Some(ResolvedCellFill::Angled(angled)) => Ok(vec![(
            "background",
            format!(
                "linear-gradient({}, {})",
                deg(angled.angle.to_f64() + 90.0)?,
                css_stops(&angled.stops)?
            ),
        )]),
        Some(ResolvedCellFill::Fill(
            fill @ (ResolvedFill::Solid { .. } | ResolvedFill::Image { .. }),
        )) => background_decls(fill, ctx),
        _ => Ok(Vec::new()),
    }
}

/// The SVG layers under a cell's text: a gradient in box fractions, and its diagonals.
fn cell_layers(placed: &Placed<'_>, table: &Table, ctx: &mut PrintCtx<'_>) -> Result<Vec<Element>> {
    let mut layers = Vec::new();
    if let Some(ResolvedCellFill::Fill(fill)) = &placed.cell.fill
        && fill.is_gradient()
    {
        let id = format!(
            "g-{}-r{}c{}",
            file_id(table.id.as_str()),
            placed.row,
            placed.column
        );
        let paint = svg_fill(Some(fill), &PaintBox::of(1.0, 1.0), &id, ctx)?;
        let unit = Pt::from_steps(1000);
        let path = Element::new(
            "path",
            Attrs::new()
                .val("d", path_data(&rect(unit, unit), 1.0)?)
                .val("style", format!("fill:{}", paint.value)),
            Vec::new(),
        );
        let defs = Element::new("defs", Attrs::new(), lines(paint.defs));
        layers.push(svg_layer(1.0, 1.0, vec![defs, path])?);
    }
    let borders = placed.cell.borders.as_ref();
    let mut diagonals = Vec::new();
    if let Some(down) = strokes(borders.and_then(|b| b.diagonal_down.as_ref())) {
        diagonals.push(diagonal(down, true)?);
    }
    if let Some(up) = strokes(borders.and_then(|b| b.diagonal_up.as_ref())) {
        diagonals.push(diagonal(up, false)?);
    }
    if !diagonals.is_empty() {
        let css = style([
            decl("position", "absolute"),
            decl("left", "0pt"),
            decl("top", "0pt"),
            decl("overflow", "visible"),
        ]);
        layers.push(Element::new(
            "svg",
            Attrs::new()
                .flag(GEOMETRY, true)
                .val("width", "100%".to_owned())
                .val("height", "100%".to_owned())
                .val("style", css),
            lines(diagonals),
        ));
    }
    Ok(layers)
}

/// A cell's edges: one `border` where all four print alike, else a declaration per drawn side.
fn border_decls(cell: &Cell, approximated: &mut Approximated) -> Result<Vec<MaybeDecl>> {
    let sides = SIDES
        .iter()
        .map(|side| border_decl(*side, side.of(cell), approximated))
        .collect::<Result<Vec<_>>>()?;
    let values: Vec<Option<&String>> = sides
        .iter()
        .map(|decl| decl.as_ref().map(|d| &d.1))
        .collect();
    if let Some(Some(first)) = values.first()
        && values.iter().all(|value| *value == Some(*first))
    {
        return Ok(vec![decl("border", (*first).clone())]);
    }
    Ok(sides)
}

fn anchor_name(anchor: Anchor) -> &'static str {
    match anchor {
        Anchor::Top => "top",
        Anchor::Middle => "middle",
        Anchor::Bottom => "bottom",
    }
}

struct TablePrint<'t> {
    table: &'t Table,
    grid: Grid<'t>,
    approximated: Approximated,
}

fn print_cell(
    placed: &Placed<'_>,
    tp: &mut TablePrint<'_>,
    ctx: &mut PrintCtx<'_>,
) -> Result<Element> {
    let cell = placed.cell;
    let body = &cell.text.body;
    let insets = &body.insets;
    let inset = |side: Side| match side {
        Side::Top => insets.t.to_f64(),
        Side::Right => insets.r.to_f64(),
        Side::Bottom => insets.b.to_f64(),
        Side::Left => insets.l.to_f64(),
    };
    // Insets are measured from the grid lines; CSS pads inside half the collapsed border.
    let [top, right, bottom, left] = SIDES.map(|side| {
        f64::max(
            0.0,
            inset(side) - collapsed_width(placed, side, &tp.grid) / 2.0,
        )
    });
    let layers = cell_layers(placed, tp.table, ctx)?;
    let text = print_cell_text(&cell.text, &tp.table.frame, ctx, &tp.table.id)?;
    let mut decls: Vec<MaybeDecl> = vec![
        // A cell's padding starts at 1px, so it always prints.
        decl("padding", sides(top, right, bottom, left)?),
        // The IR's anchors are CSS's vertical-align keywords.
        decl("vertical-align", anchor_name(body.anchor)),
    ];
    decls.extend(
        cell_background(cell.fill.as_ref(), ctx)?
            .into_iter()
            .map(Some),
    );
    decls.extend(border_decls(cell, &mut tp.approximated)?);
    let layered = !layers.is_empty();
    decls.push(layered.then(|| ("position", "relative".to_owned())));
    let spans = Attrs::new()
        .val(
            "colspan",
            (placed.columns > 1).then(|| placed.columns.to_string()),
        )
        .val(
            "rowspan",
            (placed.rows > 1).then(|| placed.rows.to_string()),
        );
    // The cell is its own text box, its paragraphs in block flow with no whitespace between them, unless layers sit
    // under the text, the text turns apart from the table, or block flow would collapse margins the flex column
    // keeps.
    if !layered && text.turn.is_none() && !text.collapses {
        decls.extend(text.inherited);
        let attrs = spans.with(text.attributes).val("style", style(decls));
        let children: Vec<Node> = text.blocks.into_iter().map(Node::Element).collect();
        return Ok(Element::new("td", attrs, children));
    }
    let placement = vec![layered.then(|| ("position", "relative".to_owned()))];
    let mut children = layers;
    children.push(text_element(text, placement));
    Ok(Element::new(
        "td",
        spans.val("style", style(decls)),
        lines(children),
    ))
}

/// How far the grid's first lines sit inside the table's box: CSS centres the collapsed borders on the grid lines
/// and gives the table half the widest border along its left and top edges.
fn grid_offset(grid: &Grid<'_>) -> (f64, f64) {
    let half = |side: Side, slots: Vec<Option<Placed<'_>>>| {
        slots
            .iter()
            .map(|placed| {
                placed
                    .as_ref()
                    .map_or(0.0, |placed| collapsed_width(placed, side, grid) / 2.0)
            })
            .fold(0.0, f64::max)
    };
    let left = half(
        Side::Left,
        grid.iter()
            .map(|row| row.first().copied().flatten())
            .collect(),
    );
    let top = half(Side::Top, grid.first().cloned().unwrap_or_default());
    (left, top)
}

pub fn print(table: &Table, ctx: &mut PrintCtx<'_>) -> Result<Element> {
    let grid = cover_grid(table);
    let (left, top) = grid_offset(&grid);
    let mut tp = TablePrint {
        table,
        grid,
        approximated: Approximated::new(),
    };
    let cols = table
        .columns
        .iter()
        .map(|column| {
            Ok(Element::new(
                "col",
                Attrs::new().val("style", style([decl("width", pt(column.width.to_f64())?)])),
                Vec::new(),
            ))
        })
        .collect::<Result<Vec<_>>>()?;
    let columns = Element::new("colgroup", Attrs::new(), lines(cols));
    let mut rows = Vec::with_capacity(table.rows.len());
    for (r, table_row) in table.rows.iter().enumerate() {
        let mut cells = Vec::new();
        for (c, cell) in table_row.cells.iter().enumerate() {
            let placed = match cell {
                None => None,
                Some(_) => slot(&tp.grid, Some(r), Some(c)),
            };
            if let Some(placed) = placed {
                cells.push(print_cell(&placed, &mut tp, ctx)?);
            }
        }
        rows.push(Element::new(
            "tr",
            Attrs::new().val(
                "style",
                style([decl("height", pt(table_row.height.to_f64())?)]),
            ),
            lines(cells),
        ));
    }
    let mut decls: Vec<MaybeDecl> = vec![
        decl("border-collapse", "collapse"),
        decl("table-layout", "fixed"),
    ];
    // The grid lines, not the box, fall on the frame, so the outer borders straddle its edges.
    if left > 0.0 || top > 0.0 {
        decls.push(decl("margin", format!("{} 0 0 {}", pt(-top)?, pt(-left)?)));
    }
    let content = lines(vec![
        columns,
        Element::new("tbody", Attrs::new(), lines(rows)),
    ]);
    for (dash, css) in &tp.approximated {
        let site = ctx
            .site()
            .node(table.id.clone())
            .data("dash", dash.as_str())
            .data("style", css.name());
        ctx.diag(codes::HTML6006, site);
    }
    let frame = &table.frame;
    // The table is the element itself, as tall as its rows; a turned table keeps a box of the frame's height to turn.
    if turn_decls(frame)?.iter().all(Option::is_none) {
        let mut place = vec![
            decl("position", "absolute"),
            decl("left", pt(frame.x.to_f64())?),
            decl("top", pt(frame.y.to_f64())?),
            decl("width", pt(frame.w.to_f64())?),
            opacity_decl(table.opacity)?,
        ];
        place.extend(decls);
        let attrs = element_attributes(common!(table, Kind::Table), ctx, Root::Table)?
            .val("style", style(place));
        return Ok(Element::new("table", attrs, content));
    }
    let mut outer = box_decls(frame, None)?;
    outer.push(opacity_decl(table.opacity)?);
    let attrs =
        element_attributes(common!(table, Kind::Table), ctx, Root::Div)?.val("style", style(outer));
    decls.push(decl("width", "100%"));
    let inner = Element::new("table", Attrs::new().val("style", style(decls)), content);
    Ok(Element::new("div", attrs, lines(vec![inner])))
}
