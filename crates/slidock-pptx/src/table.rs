//! Tables (`a:tbl` in a graphic frame, §21.1.3): the grid's column widths and least row heights, and each cell's spans,
//! margins, anchor, fill, edges and text. A cell's own properties (`a:tcPr`) override what the table style gives it,
//! edge by edge and field by field; a cell left without a fill shows the table's background. The slots a spanning cell
//! covers are null, whatever their `hMerge` and `vMerge` say, and the frame is the grid's box.

use std::collections::BTreeMap;
use std::rc::Rc;

use indexmap::IndexSet;
use slidock_core_api::limits::{add_slots, take_grid};
use slidock_core_api::lowering::LowerError;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::paint::{CellFill, Fill};
use slidock_ir::source::{Cell, Table};
use slidock_ir::table::{Border, CellBorders, TableColumn, TableRow, grid_size};
use slidock_ir::text::{
    PartialBodyProps, PartialInsets, PartialRunProps, SourceParagraph, TextBody,
};
use slidock_ir::units::{Pt, Size};

use crate::clip;
use crate::element::{Base, BodyCx, frame_of};
use crate::fill::{PaintCx, fill_element, fill_features, read_fill};
use crate::line::{StyleLine, read_line};
use crate::shadow::table_effects;
use crate::style_refs::effect_style;
use crate::table_style::{
    CellStyle, EDGES, FillSource, Grid, Slots, TableFlags, cell_style, style_fill, table_flags,
};
use crate::text::{anchor, read_text_content, with_run_defaults};
use crate::units::emu_to_size;
use crate::xml::{El, Element, Ns};

/// The cell's own line (`a:tcPr`) for each edge, in [`EDGES`] order.
const OWN_LINES: [&str; 6] = ["lnT", "lnR", "lnB", "lnL", "lnTlToBr", "lnBlToTr"];

/// A fill that paints, with the features its element carries.
#[derive(Clone, Debug)]
struct Painted {
    fill: Fill,
    features: Vec<FeatureId>,
}

/// What lowering one table's cells shares.
struct TableRead<'l, 'a> {
    paint: PaintCx<'l, 'a>,
    style: Option<Rc<Element>>,
    flags: TableFlags,
    grid: Grid,
    background: Option<Painted>,
    /// A style's fills, read once per table: one serves many cells, and its diagnostics are the table's.
    style_fills: BTreeMap<*const Element, Option<Fill>>,
    /// Whether a cell draws on its own a background the table draws across its cells.
    background_split: bool,
    /// The effects the table's cells hold, dropped once per table.
    effects: IndexSet<FeatureId>,
}

fn painted(fill: Option<Fill>, element: &Element) -> Option<Painted> {
    fill.filter(|fill| !matches!(fill, Fill::None))
        .map(|fill| Painted {
            fill,
            features: fill_features(Some(element)),
        })
}

fn styled_fill(
    source: &FillSource,
    read: &mut TableRead<'_, '_>,
) -> Result<Option<Painted>, LowerError> {
    let Some(element) = source.element.as_deref() else {
        return Ok(None);
    };
    let key = Rc::as_ptr(&source.at);
    let fill = match read.style_fills.get(&key) {
        Some(fill) => fill.clone(),
        None => {
            let fill = read_fill(element, &mut read.paint, source.color.as_ref())?;
            read.style_fills.insert(key, fill.clone());
            fill
        }
    };
    Ok(painted(fill, element))
}

/// The cell's own fill, else its style's, else the table's background; `None` for none.
fn cell_fill(
    tc_pr: Option<&Element>,
    styled: Option<&CellStyle>,
    read: &mut TableRead<'_, '_>,
) -> Result<Option<Fill>, LowerError> {
    let fill = match fill_element(tc_pr) {
        Some(own) => painted(read_fill(own, &mut read.paint, None)?, own),
        None => match styled.and_then(|styled| styled.fill.as_ref()) {
            Some(source) => styled_fill(source, read)?,
            None => None,
        },
    };
    let own = fill.is_some();
    let Some(shown) = fill.or_else(|| read.background.clone()) else {
        return Ok(None);
    };
    if !own && !matches!(shown.fill, Fill::Solid { .. }) {
        read.background_split = true;
    }
    for feature in &shown.features {
        read.paint.ctx.seen(*feature);
    }
    Ok(Some(shown.fill))
}

/// An edge's line: the cell's own over its style's. Line ends are not drawn on cell edges.
fn border(
    own: Option<&Element>,
    base: Option<&StyleLine>,
    read: &mut TableRead<'_, '_>,
) -> Option<Border<slidock_ir::color::Color>> {
    let stroke = read_line(own, base, &mut read.paint, false).flatten()?;
    read.paint.ctx.seen(FeatureId::Stroke);
    if !stroke.dash.is_solid() {
        read.paint.ctx.seen(FeatureId::StrokeDash);
    }
    Some(Border {
        width: stroke.width,
        fill: stroke.fill,
        dash: stroke.dash,
        cap: stroke.cap,
        join: stroke.join,
    })
}

/// The cell's margins and anchor as the text body's insets and anchor; ECMA's defaults are the cell defaults.
fn cell_body(tc_pr: Option<&Element>, read: &mut TableRead<'_, '_>) -> PartialBodyProps {
    let mut out = PartialBodyProps::default();
    let margin = |name: &str| {
        tc_pr
            .int_attr(name)
            .map(|value| Pt::from_steps(emu_to_size(value).steps()))
    };
    let insets = PartialInsets {
        l: margin("marL"),
        t: margin("marT"),
        r: margin("marR"),
        b: margin("marB"),
    };
    if insets != PartialInsets::default() {
        out.insets = Some(insets);
    }
    out.anchor = anchor(tc_pr.attr("anchor").unwrap_or(""));
    let node = read.paint.node.clone();
    let vert = tc_pr.attr("vert").unwrap_or("horz");
    if vert != "horz" {
        read.paint.ctx.seen(FeatureId::TextVertical);
        if let Some(node) = &node {
            read.paint.ctx.report_on(
                node,
                codes::PX2202,
                DiagnosticSite::new().data("vert", vert),
            );
        }
    }
    if tc_pr.bool_attr("anchorCtr") == Some(true)
        && let Some(node) = &node
    {
        read.paint
            .ctx
            .report_on(node, codes::PX2203, DiagnosticSite::new());
    }
    out
}

fn empty_paragraph() -> SourceParagraph {
    SourceParagraph {
        level: 0,
        props: Default::default(),
        inlines: Vec::new(),
        end_props: None,
    }
}

/// The cell's text, with its style's text properties beneath its list style; an empty cell holds one empty
/// paragraph.
fn cell_text(
    tc: Option<&Element>,
    tc_pr: Option<&Element>,
    run: &PartialRunProps,
    read: &mut TableRead<'_, '_>,
) -> Result<TextBody, LowerError> {
    let body = cell_body(tc_pr, read);
    let (declared, paragraphs) = match tc.child(Ns::A, "txBody") {
        Some(tx_body) => {
            let node = read
                .paint
                .node
                .clone()
                .unwrap_or_else(|| read.paint.ctx.container.clone());
            let mut cx = BodyCx {
                ctx: read.paint.ctx,
                node,
            };
            read_text_content(tx_body, &mut cx)?
        }
        None => (None, Vec::new()),
    };
    let list_style = if run == &PartialRunProps::default() {
        declared
    } else {
        Some(with_run_defaults(declared.as_ref(), run))
    };
    Ok(TextBody {
        body,
        list_style,
        paragraphs: if paragraphs.is_empty() {
            vec![empty_paragraph()]
        } else {
            paragraphs
        },
        flips_with_frame: false,
        rotation: None,
    })
}

fn span(count: usize) -> Option<u32> {
    (count > 1).then(|| u32::try_from(count).unwrap_or(u32::MAX))
}

fn lower_cell(
    tc: Option<&Element>,
    slots: Slots,
    read: &mut TableRead<'_, '_>,
) -> Result<Cell, LowerError> {
    let tc_pr = tc.child(Ns::A, "tcPr");
    let styled = match read.style.clone() {
        Some(style) => {
            let formats = read.paint.ctx.formats;
            Some(cell_style(
                &style,
                read.flags,
                slots,
                read.grid,
                formats,
                read.paint.ctx,
            ))
        }
        None => None,
    };
    let fill = cell_fill(tc_pr, styled.as_ref(), read)?;
    let mut lines: [Option<Border<slidock_ir::color::Color>>; 6] = Default::default();
    for (edge, name) in EDGES.into_iter().zip(OWN_LINES) {
        let base = styled
            .as_ref()
            .and_then(|styled| styled.lines.get(edge.index()))
            .and_then(Option::as_ref);
        if let (Some(line), Some(slot)) = (
            border(tc_pr.child(Ns::A, name), base, read),
            lines.get_mut(edge.index()),
        ) {
            *slot = Some(line);
        }
    }
    let cell_3d = tc_pr
        .child(Ns::A, "cell3D")
        .or_else(|| styled.as_ref().and_then(|styled| styled.cell_3d.as_deref()));
    if cell_3d.child(Ns::A, "bevel").is_some() {
        read.effects.insert(FeatureId::Effect3d);
    }
    let run = styled.map(|styled| styled.run).unwrap_or_default();
    let text = cell_text(tc, tc_pr, &run, read)?;
    let [top, right, bottom, left, diagonal_down, diagonal_up] = lines;
    let borders = CellBorders {
        top,
        right,
        bottom,
        left,
        diagonal_down,
        diagonal_up,
    };
    let any_border = borders.edges().iter().any(|(_, border)| border.is_some());
    Ok(Cell {
        col_span: span(slots.right - slots.left + 1),
        row_span: span(slots.bottom - slots.top + 1),
        fill: fill.map(CellFill::Fill),
        borders: any_border.then_some(borders),
        text,
    })
}

/// The dense grid: each row's `a:tc` in column order, a cell's spans clipped to the grid and to the slots earlier
/// cells cover. Covering is the spans' alone, so every slot is covered exactly once.
fn lower_rows(
    trs: &[&Element],
    read: &mut TableRead<'_, '_>,
) -> Result<Vec<TableRow<Cell>>, LowerError> {
    let Grid {
        rows: row_count,
        columns,
    } = read.grid;
    let mut covered = vec![vec![false; columns]; trs.len()];
    let mut out = Vec::with_capacity(trs.len());
    for (row, tr) in trs.iter().enumerate() {
        let tcs: Vec<&Element> = tr.children(Ns::A, "tc").map(|rc| &**rc).collect();
        let mut cells = Vec::with_capacity(columns);
        for column in 0..columns {
            let is_covered = |covered: &Vec<Vec<bool>>, r: usize, c: usize| {
                covered
                    .get(r)
                    .and_then(|row| row.get(c))
                    .copied()
                    .unwrap_or(false)
            };
            if is_covered(&covered, row, column) {
                cells.push(None);
                continue;
            }
            let tc = tcs.get(column).copied();
            let wide = tc.int_attr("gridSpan").unwrap_or(1).max(1);
            let reach = usize::try_from(wide).unwrap_or(usize::MAX);
            let mut right = columns.min(column.saturating_add(reach)) - 1;
            for c in column + 1..=right {
                if is_covered(&covered, row, c) {
                    right = c - 1;
                    break;
                }
            }
            let tall = tc.int_attr("rowSpan").unwrap_or(1).max(1);
            let reach = usize::try_from(tall).unwrap_or(usize::MAX);
            let bottom = row_count.min(row.saturating_add(reach)) - 1;
            for slots in covered.iter_mut().take(bottom + 1).skip(row) {
                for slot in slots.iter_mut().take(right + 1).skip(column) {
                    *slot = true;
                }
            }
            let slots = Slots {
                top: row,
                bottom,
                left: column,
                right,
            };
            cells.push(Some(lower_cell(tc, slots, read)?));
        }
        out.push(TableRow {
            height: emu_to_size(Some(*tr).int_attr("h").unwrap_or(0)),
            cells,
        });
    }
    Ok(out)
}

/// The table's background (`a:tblPr`'s fill, else its style's `a:tblBg`), and the effects of both, dropped.
fn background(
    tbl_pr: Option<&Element>,
    style: Option<&Element>,
    read: &mut TableRead<'_, '_>,
) -> Result<Option<Painted>, LowerError> {
    let tbl_bg = style.child(Ns::A, "tblBg");
    let effect = tbl_bg.child(Ns::A, "effect");
    let effect_ref = tbl_bg
        .child(Ns::A, "effectRef")
        .int_attr("idx")
        .unwrap_or(0);
    let entry = effect
        .is_none()
        .then(|| effect_style(effect_ref, read.paint.ctx.formats))
        .flatten();
    if let Some(node) = read.paint.node.clone() {
        table_effects(tbl_pr, effect.or(entry.as_deref()), &node, read.paint.ctx);
    }
    if let Some(own) = fill_element(tbl_pr) {
        return Ok(painted(read_fill(own, &mut read.paint, None)?, own));
    }
    let formats = read.paint.ctx.formats;
    let Some(source) = style_fill(tbl_bg, formats, read.paint.ctx) else {
        return Ok(None);
    };
    styled_fill(&source, read)
}

/// A table's grid columns and rows.
fn grid_of(tbl: &Element) -> (Vec<&Element>, Vec<&Element>) {
    (
        tbl.child(Ns::A, "tblGrid").children(Ns::A, "gridCol"),
        tbl.children(Ns::A, "tr").map(|rc| &**rc).collect(),
    )
}

/// The grid slots of every table in a part's tree, counted without reading a cell.
pub(crate) fn table_slots(root: &Element) -> u64 {
    let mut total = 0;
    let mut stack = vec![root];
    while let Some(node) = stack.pop() {
        if node.is(Ns::A, "tbl") {
            let (columns, rows) = grid_of(node);
            total = add_slots(
                total,
                (columns.len() as u64).saturating_mul(rows.len() as u64),
            );
        }
        let kids: Vec<&Element> = node.kids().map(|rc| &**rc).collect();
        stack.extend(kids.into_iter().rev());
    }
    total
}

/// A graphic frame's table, or why it stays Foreign.
pub(crate) enum LoweredTable {
    Table(Box<Table>),
    /// The container cannot hold its grid, or it has none.
    Refused(String),
}

pub(crate) fn lower_table(
    tbl: &Element,
    base: &Base,
    xfrm: Option<&Element>,
    paint: PaintCx<'_, '_>,
) -> Result<LoweredTable, LowerError> {
    let (grid_columns, trs) = grid_of(tbl);
    let widths: Vec<Size> = grid_columns
        .iter()
        .map(|col| emu_to_size(Some(*col).int_attr("w").unwrap_or(0)))
        .collect();
    if widths.is_empty() || trs.is_empty() {
        return Ok(LoweredTable::Refused("the table holds no cell".to_owned()));
    }
    let heights: Vec<f64> = trs
        .iter()
        .map(|tr| emu_to_size(Some(*tr).int_attr("h").unwrap_or(0)).to_f64())
        .collect();
    let width_values: Vec<f64> = widths.iter().map(|width| width.to_f64()).collect();
    if let Some(refused) = take_grid(&mut paint.ctx.budget, &width_values, &heights) {
        return Ok(LoweredTable::Refused(refused));
    }
    paint.ctx.seen(FeatureId::ElementTable);
    let tbl_pr = tbl.child(Ns::A, "tblPr").map(|rc| &**rc);
    let style_id = tbl_pr
        .child(Ns::A, "tableStyleId")
        .map(|named| slidock_canon::js_trim(&named.text_content()).to_owned())
        .unwrap_or_default();
    let inline = tbl_pr
        .and_then(|tbl_pr| tbl_pr.child(Ns::A, "tableStyle"))
        .cloned();
    let style = inline.or_else(|| {
        (!style_id.is_empty())
            .then(|| paint.ctx.table_styles.get(&style_id).cloned())
            .flatten()
    });
    if style.is_none() && !style_id.is_empty() {
        paint.ctx.report_on(
            &base.id,
            codes::PX2011,
            DiagnosticSite::new().data("style", clip(&style_id, 64)),
        );
    }
    if tbl_pr.bool_attr("rtl") == Some(true) {
        paint
            .ctx
            .report_on(&base.id, codes::PX2012, DiagnosticSite::new());
    }
    let mut read = TableRead {
        paint,
        style: style.clone(),
        flags: table_flags(tbl_pr),
        grid: Grid {
            rows: trs.len(),
            columns: widths.len(),
        },
        background: None,
        style_fills: BTreeMap::new(),
        background_split: false,
        effects: IndexSet::new(),
    };
    read.background = background(tbl_pr, style.as_deref(), &mut read)?;
    let rows = lower_rows(&trs, &mut read)?;
    let ctx = read.paint.ctx;
    if read.background_split
        && let Some(background) = &read.background
    {
        ctx.report_on(
            &base.id,
            codes::PX2108,
            DiagnosticSite::new().data("fill", background.fill.kind()),
        );
    }
    for feature in read.effects {
        ctx.seen(feature);
        let effect = feature
            .as_str()
            .strip_prefix("effect.")
            .unwrap_or(feature.as_str());
        ctx.report_on(
            &base.id,
            codes::PX2101,
            DiagnosticSite::new()
                .feature(feature)
                .data("effect", effect),
        );
    }
    let columns: Vec<TableColumn> = widths
        .into_iter()
        .map(|width| TableColumn { width })
        .collect();
    let frame = frame_of(xfrm, base, ctx).map(|placed| {
        let (w, h) = grid_size(&columns, &rows);
        slidock_ir::frame::Frame {
            w: Size::from_steps(w.steps()),
            h: Size::from_steps(h.steps()),
            ..placed
        }
    });
    let base = base.clone();
    Ok(LoweredTable::Table(Box::new(Table {
        id: base.id,
        origin: base.origin,
        name: base.name,
        placeholder: base.placeholder,
        inherits: None,
        match_key: None,
        hidden: base.hidden,
        opacity: None,
        link: base.link,
        alt: base.alt,
        ext: None,
        frame,
        columns,
        rows,
    })))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edges_follow_the_ir_order() {
        assert_eq!(
            EDGES.map(crate::table_style::CellEdge::index),
            [0, 1, 2, 3, 4, 5]
        );
        assert_eq!(OWN_LINES.len(), EDGES.len());
    }
}
