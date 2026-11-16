//! Table styles (`a:tblStyle`, §20.1.4.2), from the presentation's table styles part or inline in `a:tblPr`. A style is
//! a list of parts, each for a region of the table: the whole table, banded rows and columns, the first and last rows
//! and columns, and the corner cells; the table's flags turn the regions on. Each part sets fills, edges and text
//! properties for the cells in its region, its outer edges on the region's boundary and its inside edges between the
//! region's cells. The frontend resolves the parts into each cell's own properties: the IR has no table styles.

use std::rc::Rc;

use indexmap::IndexMap;
use slidock_ir::color::Color;
use slidock_ir::text::{PartialRunProps, ThemeFontSlot};

use crate::fill::fill_element_rc;
use crate::line::StyleLine;
use crate::package::Package;
use crate::report::Report;
use crate::style_refs::{fill_style, line_style, ref_color};
use crate::text::{font_ref, overlay_run, theme_font};
use crate::theme::FormatScheme;
use crate::xml::{El, Element, Ns};

/// The table styles a presentation defines, by style id.
pub(crate) type TableStyles = IndexMap<String, Rc<Element>>;

/// The table styles part the presentation part names; an unreadable part defines none.
pub(crate) fn read_table_styles(pkg: &Package<'_>, presentation_part: &str) -> TableStyles {
    let mut out = TableStyles::new();
    let Some(rel) = pkg.first(presentation_part, "tableStyles").ok().flatten() else {
        return out;
    };
    let Some(root) = pkg.xml(&rel.target).ok() else {
        return out;
    };
    if !root.is(Ns::A, "tblStyleLst") {
        return out;
    }
    for style in root.children(Ns::A, "tblStyle") {
        if let Some(id) = style.attr("styleId").filter(|id| !id.is_empty()) {
            out.insert(id.to_owned(), style.clone());
        }
    }
    out
}

/// `a:tblPr`'s flags: which of the style's regions apply.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TableFlags {
    pub first_row: bool,
    pub last_row: bool,
    pub first_col: bool,
    pub last_col: bool,
    pub band_row: bool,
    pub band_col: bool,
}

pub(crate) fn table_flags(tbl_pr: Option<&Element>) -> TableFlags {
    let flag = |name: &str| tbl_pr.bool_attr(name).unwrap_or(false);
    TableFlags {
        first_row: flag("firstRow"),
        last_row: flag("lastRow"),
        first_col: flag("firstCol"),
        last_col: flag("lastCol"),
        band_row: flag("bandRow"),
        band_col: flag("bandCol"),
    }
}

/// A rectangle of grid slots, its bounds inclusive.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Slots {
    pub top: usize,
    pub bottom: usize,
    pub left: usize,
    pub right: usize,
}

/// A grid's size in tracks.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Grid {
    pub rows: usize,
    pub columns: usize,
}

/// The parts that style a cell, lowest precedence first, each with the region its outer and inside edges follow.
fn parts_for(
    style: &Element,
    flags: TableFlags,
    cell: Slots,
    grid: Grid,
) -> Vec<(Rc<Element>, Slots)> {
    let all = Slots {
        top: 0,
        bottom: grid.rows.saturating_sub(1),
        left: 0,
        right: grid.columns.saturating_sub(1),
    };
    // A region of rows (or of columns) across the table, widened to the cell where the cell spans past it.
    let rows = |top: usize, bottom: usize| Slots {
        top: top.min(cell.top),
        bottom: bottom.max(cell.bottom),
        ..all
    };
    let columns = |left: usize, right: usize| Slots {
        left: left.min(cell.left),
        right: right.max(cell.right),
        ..all
    };
    let first_row = flags.first_row && cell.top == 0;
    let last_row = flags.last_row && cell.bottom == all.bottom;
    let first_col = flags.first_col && cell.left == 0;
    let last_col = flags.last_col && cell.right == all.right;
    let mut out = Vec::new();
    let mut add = |name: &str, region: Slots| {
        if let Some(part) = style.child(Ns::A, name) {
            out.push((part.clone(), region));
        }
    };
    add("wholeTbl", all);
    // Bands count from the first row (or column) after the header, which itself is not banded.
    let even = |at: usize, header: bool| (at as i64 - i64::from(header)) % 2 == 0;
    if flags.band_row && !first_row && !last_row {
        let name = if even(cell.top, flags.first_row) {
            "band1H"
        } else {
            "band2H"
        };
        add(name, rows(cell.top, cell.bottom));
    }
    if flags.band_col && !first_col && !last_col {
        let name = if even(cell.left, flags.first_col) {
            "band1V"
        } else {
            "band2V"
        };
        add(name, columns(cell.left, cell.right));
    }
    if last_col {
        add("lastCol", columns(all.right, all.right));
    }
    if first_col {
        add("firstCol", columns(0, 0));
    }
    if last_row {
        add("lastRow", rows(all.bottom, all.bottom));
    }
    if first_row {
        add("firstRow", rows(0, 0));
    }
    if last_row && last_col {
        add("seCell", cell);
    }
    if last_row && first_col {
        add("swCell", cell);
    }
    if first_row && last_col {
        add("neCell", cell);
    }
    if first_row && first_col {
        add("nwCell", cell);
    }
    out
}

/// A cell's edges, in the order the IR lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CellEdge {
    Top,
    Right,
    Bottom,
    Left,
    DiagonalDown,
    DiagonalUp,
}

pub(crate) const EDGES: [CellEdge; 6] = [
    CellEdge::Top,
    CellEdge::Right,
    CellEdge::Bottom,
    CellEdge::Left,
    CellEdge::DiagonalDown,
    CellEdge::DiagonalUp,
];

impl CellEdge {
    pub fn index(self) -> usize {
        self as usize
    }
}

/// The `a:tcBdr` child a cell's edge takes from a part: an outer edge on the region's boundary, else an inside one.
fn edge_name(edge: CellEdge, cell: Slots, region: Slots) -> &'static str {
    match edge {
        CellEdge::Top if cell.top == region.top => "top",
        CellEdge::Bottom if cell.bottom == region.bottom => "bottom",
        CellEdge::Top | CellEdge::Bottom => "insideH",
        CellEdge::Left if cell.left == region.left => "left",
        CellEdge::Right if cell.right == region.right => "right",
        CellEdge::Left | CellEdge::Right => "insideV",
        CellEdge::DiagonalDown => "tl2br",
        CellEdge::DiagonalUp => "tr2bl",
    }
}

/// A fill as a style writes it (`at`): a fill element, or a theme fill style with the colour its `phClr` stands
/// for. No element is no fill.
#[derive(Clone, Debug)]
pub(crate) struct FillSource {
    pub at: Rc<Element>,
    pub element: Option<Rc<Element>>,
    pub color: Option<Color>,
}

/// What a style gives a cell: the fill and edges beneath the cell's own, the run properties of its text.
#[derive(Clone, Debug, Default)]
pub(crate) struct CellStyle {
    pub fill: Option<FillSource>,
    pub lines: [Option<StyleLine>; 6],
    /// The 3D properties (`a:cell3D`) the style gives the cell.
    pub cell_3d: Option<Rc<Element>>,
    pub run: PartialRunProps,
}

/// `a:fill` or `a:fillRef` under a style part's `a:tcStyle` or `a:tblBg`.
pub(crate) fn style_fill(
    parent: Option<&Element>,
    formats: &FormatScheme,
    report: &mut dyn Report,
) -> Option<FillSource> {
    if let Some(own) = parent.and_then(|parent| parent.child(Ns::A, "fill")) {
        return Some(FillSource {
            at: own.clone(),
            element: fill_element_rc(own).cloned(),
            color: None,
        });
    }
    let reference = parent.and_then(|parent| parent.child(Ns::A, "fillRef"))?;
    let idx = reference.int_attr("idx").unwrap_or(0);
    let color = ref_color(Some(reference), report);
    Some(FillSource {
        at: reference.clone(),
        element: if idx > 0 {
            fill_style(idx, formats)
        } else {
            None
        },
        color,
    })
}

/// An edge of `a:tcBdr`: its own line, or a line style of the theme in the reference's colour.
fn style_line(
    edge: &Element,
    formats: &FormatScheme,
    report: &mut dyn Report,
) -> Option<StyleLine> {
    if let Some(ln) = edge.child(Ns::A, "ln") {
        return Some(StyleLine {
            element: Some(ln.clone()),
            color: None,
        });
    }
    let reference = edge.child(Ns::A, "lnRef")?;
    let idx = reference.int_attr("idx").unwrap_or(0);
    Some(StyleLine {
        element: line_style(idx, formats),
        color: ref_color(Some(reference), report),
    })
}

/// `a:tcTxStyle`: bold and italic (`def` leaves them), the font, and the text colour.
fn text_style(tc_tx_style: Option<&Element>, report: &mut dyn Report) -> PartialRunProps {
    let mut out = PartialRunProps::default();
    let Some(style) = tc_tx_style else { return out };
    let flag = |name: &str| match style.attr(name) {
        Some("on") => Some(true),
        Some("off") => Some(false),
        _ => None,
    };
    out.weight = flag("b").map(|bold| if bold { 700 } else { 400 });
    out.italic = flag("i");
    let reference = style.child(Ns::A, "fontRef").map(|rc| &**rc);
    let slot = match reference.attr("idx") {
        Some("major") => Some(ThemeFontSlot::Major),
        Some("minor") => Some(ThemeFontSlot::Minor),
        _ => None,
    };
    out.font = font_ref(
        style
            .child(Ns::A, "font")
            .child(Ns::A, "latin")
            .attr("typeface"),
    )
    .or_else(|| slot.map(theme_font));
    out.color = ref_color(Some(style), report).or_else(|| ref_color(reference, report));
    out
}

/// The style's properties for a cell, each from the last part that sets it.
pub(crate) fn cell_style(
    style: &Element,
    flags: TableFlags,
    cell: Slots,
    grid: Grid,
    formats: &FormatScheme,
    report: &mut dyn Report,
) -> CellStyle {
    let mut out = CellStyle::default();
    for (part, region) in parts_for(style, flags, cell, grid) {
        let tc_style = part.child(Ns::A, "tcStyle").map(|rc| &**rc);
        if let Some(fill) = style_fill(tc_style, formats, report) {
            out.fill = Some(fill);
        }
        let tc_bdr = tc_style.child(Ns::A, "tcBdr");
        for edge in EDGES {
            let named = tc_bdr.child(Ns::A, edge_name(edge, cell, region));
            if let Some(line) = named.and_then(|named| style_line(named, formats, report))
                && let Some(slot) = out.lines.get_mut(edge.index())
            {
                *slot = Some(line);
            }
        }
        if let Some(cell_3d) = tc_style.and_then(|tc_style| tc_style.child(Ns::A, "cell3D")) {
            out.cell_3d = Some(cell_3d.clone());
        }
        let text = text_style(part.child(Ns::A, "tcTxStyle").map(|rc| &**rc), report);
        out.run = overlay_run(&out.run, &text);
    }
    out
}
