//! Tables (`table:table` in a frame) as a dense grid. Every column, row and cell element takes one slot per repeat
//! (`table:number-*-repeated`), through any column or row group; the grid is as wide as its declared columns or its
//! longest row, and a shorter row ends in empty cells. A cell spans the free slots its stated span reaches, so a slot
//! an earlier span covers is covered whatever element the source writes there. Widths and heights are their styles' (a
//! row's height is its least); tracks without one share what the frame leaves. A cell's style is its own, else its
//! row's default cell style, else its column's, over the cell style its position takes from the table's template, as
//! LibreOffice applies templates: first or last row, then first or last column, then row and column banding, then the
//! body. A grid the IR cannot hold stays a Foreign table.

use slidock_canon::utf16;
use slidock_core_api::limits::{add_slots, grid_refusal, take_grid};
use slidock_core_api::lowering::LowerError;
use slidock_ir::color::Color;
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::frame::Frame;
use slidock_ir::paint::{Cap, CellFill, Dash, DashPreset, Fill, Join, StrokePaint};
use slidock_ir::source::{Cell, Table};
use slidock_ir::table::{Border, CellBorders, TableColumn, TableRow, grid_size};
use slidock_ir::text::{Autofit, PartialBodyProps, SourceParagraph, TextBody, Wrap};
use slidock_ir::units::Size;

use crate::context::Lowering;
use crate::element::Base;
use crate::graphic::{Paint, read_fill};
use crate::names::{QName, qn};
use crate::odf::{Element, Ns};
use crate::styles::{Family, PropertySet, Props, get};
use crate::text::{ShapeText, TextContext, level_style_of, read_text, set_level, text_area};
use crate::units::{hex_color, is_space, raw_length, rgb, size, to_pt};

/// An element and the slots it takes.
#[derive(Clone, Copy)]
struct Repeated<'a> {
    element: &'a Element,
    count: u64,
}

const COLUMN_GROUPS: [&str; 3] = [
    "table-columns",
    "table-header-columns",
    "table-column-group",
];
const ROW_GROUPS: [&str; 3] = ["table-rows", "table-header-rows", "table-row-group"];
const CELLS: [&str; 2] = ["table-cell", "covered-table-cell"];

/// The elements of a kind in document order, through groups, each with its repeat count.
fn repeated<'a>(
    parent: &'a Element,
    kinds: &[&str],
    groups: &[&str],
    repeat: QName,
) -> Vec<Repeated<'a>> {
    fn visit<'a>(
        node: &'a Element,
        kinds: &[&str],
        groups: &[&str],
        repeat: QName,
        out: &mut Vec<Repeated<'a>>,
    ) {
        for element in node.kids() {
            if !element.in_namespace(Ns::Table) {
                continue;
            }
            if kinds.contains(&element.local.as_str()) {
                let count = element.int_attr(repeat).unwrap_or(1).max(1);
                out.push(Repeated {
                    element,
                    count: u64::try_from(count).unwrap_or(1),
                });
            } else if groups.contains(&element.local.as_str()) {
                visit(element, kinds, groups, repeat, out);
            }
        }
    }
    let mut out = Vec::new();
    visit(parent, kinds, groups, repeat, &mut out);
    out
}

fn slots(elements: &[Repeated<'_>]) -> u64 {
    elements
        .iter()
        .fold(0, |sum, element| sum.saturating_add(element.count))
}

struct Grid<'a> {
    column_elements: Vec<Repeated<'a>>,
    row_elements: Vec<Repeated<'a>>,
    height: u64,
    width: u64,
}

/// A table's column and row elements, and the slots its grid takes across and down.
fn grid_of(table: &Element) -> Grid<'_> {
    let column_elements = repeated(
        table,
        &["table-column"],
        &COLUMN_GROUPS,
        qn!("table:number-columns-repeated"),
    );
    let row_elements = repeated(
        table,
        &["table-row"],
        &ROW_GROUPS,
        qn!("table:number-rows-repeated"),
    );
    let longest = row_elements
        .iter()
        .map(|row| {
            slots(&repeated(
                row.element,
                &CELLS,
                &[],
                qn!("table:number-columns-repeated"),
            ))
        })
        .max()
        .unwrap_or(0);
    Grid {
        height: slots(&row_elements),
        width: slots(&column_elements).max(longest),
        column_elements,
        row_elements,
    }
}

/// The grid slots of every table under an element, counted from its repeats without expanding a cell.
pub(crate) fn table_slots(parent: &Element) -> u64 {
    fn visit(node: &Element, total: &mut u64) {
        for element in node.kids() {
            if element.is(Ns::Table, "table") {
                let grid = grid_of(element);
                *total = add_slots(*total, grid.height.saturating_mul(grid.width));
            }
            visit(element, total);
        }
    }
    let mut total = 0;
    visit(parent, &mut total);
    total
}

/// Each element once per slot it takes (only for grids the budget holds).
fn expand<'a>(elements: &[Repeated<'a>]) -> Vec<&'a Element> {
    elements
        .iter()
        .flat_map(|element| {
            std::iter::repeat_n(element.element, usize::try_from(element.count).unwrap_or(0))
        })
        .collect()
}

/// A cell placed on the grid: the element it comes from (none for a row's completing cells) and its spans.
#[derive(Clone, Copy)]
struct Placed<'a> {
    element: Option<&'a Element>,
    row: usize,
    column: usize,
    columns: usize,
    rows: usize,
}

/// The grid's slots, each a placed cell or `None` where a span covers it.
fn place<'a>(rows: &[&'a Element], width: usize) -> Vec<Vec<Option<Placed<'a>>>> {
    let mut taken = vec![vec![false; width]; rows.len()];
    let span = |element: Option<&Element>, name: QName| -> usize {
        match element {
            Some(element) if element.is(Ns::Table, "table-cell") => {
                usize::try_from(element.int_attr(name).unwrap_or(1).max(1)).unwrap_or(1)
            }
            _ => 1,
        }
    };
    let is_taken = |taken: &[Vec<bool>], row: usize, column: usize| {
        taken
            .get(row)
            .and_then(|r| r.get(column))
            .copied()
            .unwrap_or(false)
    };
    let mut grid = Vec::with_capacity(rows.len());
    for (r, row) in rows.iter().enumerate() {
        let elements = expand(&repeated(
            row,
            &CELLS,
            &[],
            qn!("table:number-columns-repeated"),
        ));
        let mut out = Vec::with_capacity(width);
        for c in 0..width {
            if is_taken(&taken, r, c) {
                out.push(None);
                continue;
            }
            let element = elements.get(c).copied();
            let want_columns = span(element, qn!("table:number-columns-spanned"));
            let want_rows = span(element, qn!("table:number-rows-spanned"));
            let mut columns = 1;
            while columns < want_columns && c + columns < width && !is_taken(&taken, r, c + columns)
            {
                columns += 1;
            }
            let free = |taken: &[Vec<bool>], row: usize| {
                (c..c + columns).all(|column| !is_taken(taken, row, column))
            };
            let mut down = 1;
            while down < want_rows && r + down < rows.len() && free(&taken, r + down) {
                down += 1;
            }
            for slot_row in taken.iter_mut().skip(r).take(down) {
                for slot in slot_row.iter_mut().skip(c).take(columns) {
                    *slot = true;
                }
            }
            out.push(Some(Placed {
                element,
                row: r,
                column: c,
                columns,
                rows: down,
            }));
        }
        grid.push(out);
    }
    grid
}

/// Track lengths: the stated ones, and the room the frame leaves shared among the rest by weight.
fn track_lengths(stated: &[Option<f64>], weights: &[f64], frame: f64) -> Vec<f64> {
    let room = (frame
        - stated
            .iter()
            .fold(0.0, |sum, length| sum + length.unwrap_or(0.0)))
    .max(0.0);
    let open: Vec<f64> = stated
        .iter()
        .zip(weights)
        .map(|(length, weight)| if length.is_none() { *weight } else { 0.0 })
        .collect();
    let total = open.iter().fold(0.0, |sum, weight| sum + weight);
    stated
        .iter()
        .zip(&open)
        .map(|(length, open)| {
            length.unwrap_or(if total > 0.0 {
                (room * open) / total
            } else {
                0.0
            })
        })
        .collect()
}

/// `1234*`: a relative column width.
fn relative_width(value: Option<&str>) -> Option<f64> {
    let number = value?.trim_matches(is_space).strip_suffix('*')?;
    let (whole, fraction) = match number.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (number, None),
    };
    let digits = |text: &str, max: usize| {
        (1..=max).contains(&text.len()) && text.bytes().all(|b| b.is_ascii_digit())
    };
    if !digits(whole, 12) || fraction.is_some_and(|f| !digits(f, 12)) {
        return None;
    }
    number.parse().ok()
}

/// A length clamped at zero, unquantised.
fn extent(value: Option<&str>) -> Option<f64> {
    raw_length(value).map(|pt| pt.max(0.0))
}

/// Column widths: stated, else shared by relative width when every open column states one, else evenly.
fn column_widths(columns: &[std::rc::Rc<PropertySet>], frame: f64) -> Vec<f64> {
    let stated: Vec<Option<f64>> = columns
        .iter()
        .map(|column| extent(get(column, qn!("style:column-width"))))
        .collect();
    let relative: Vec<Option<f64>> = columns
        .iter()
        .map(|column| relative_width(get(column, qn!("style:rel-column-width"))))
        .collect();
    let by_relative = stated
        .iter()
        .zip(&relative)
        .all(|(width, relative)| width.is_some() || relative.is_some());
    let weights: Vec<f64> = relative
        .iter()
        .map(|weight| {
            if by_relative {
                weight.unwrap_or(0.0)
            } else {
                1.0
            }
        })
        .collect();
    track_lengths(&stated, &weights, frame)
}

/// The template parts LibreOffice applies, in the order it applies them.
const TEMPLATE_PARTS: [&str; 9] = [
    "first-row",
    "last-row",
    "first-column",
    "last-column",
    "even-rows",
    "odd-rows",
    "even-columns",
    "odd-columns",
    "body",
];

/// The cell style a template gives each position.
struct TemplateStyles<'a> {
    parts: Vec<(&'static str, &'a str)>,
    flags: [bool; 6],
    rows: usize,
    columns: usize,
}

impl<'a> TemplateStyles<'a> {
    fn new(table: &Element, template: Option<&'a Element>, rows: usize, columns: usize) -> Self {
        let parts = TEMPLATE_PARTS
            .iter()
            .filter_map(|part| {
                let name = template
                    .and_then(|t| t.child(Ns::Table, part))
                    .and_then(|p| p.attr(qn!("table:style-name")))
                    .filter(|s| !s.is_empty())?;
                Some((*part, name))
            })
            .collect();
        let flags = [
            qn!("table:use-first-row-styles"),
            qn!("table:use-last-row-styles"),
            qn!("table:use-first-column-styles"),
            qn!("table:use-last-column-styles"),
            qn!("table:use-banding-rows-styles"),
            qn!("table:use-banding-columns-styles"),
        ]
        .map(|flag| table.bool_attr(flag) == Some(true));
        TemplateStyles {
            parts,
            flags,
            rows,
            columns,
        }
    }

    fn style(&self, row: usize, column: usize) -> Option<&'a str> {
        let [
            first_row,
            last_row,
            first_column,
            last_column,
            row_bands,
            column_bands,
        ] = self.flags;
        let candidates = [
            (first_row && row == 0).then_some("first-row"),
            (last_row && row + 1 == self.rows).then_some("last-row"),
            (first_column && column == 0).then_some("first-column"),
            (last_column && column + 1 == self.columns).then_some("last-column"),
            row_bands.then_some(if row.is_multiple_of(2) {
                "even-rows"
            } else {
                "odd-rows"
            }),
            column_bands.then_some(if column.is_multiple_of(2) {
                "even-columns"
            } else {
                "odd-columns"
            }),
            Some("body"),
        ];
        candidates.into_iter().flatten().find_map(|part| {
            self.parts
                .iter()
                .find(|(name, _)| *name == part)
                .map(|(_, style)| *style)
        })
    }
}

fn border_dash(style: &str) -> Option<DashPreset> {
    Some(match style {
        "solid" => DashPreset::Solid,
        "dotted" => DashPreset::SysDot,
        "dashed" => DashPreset::Dash,
        "fine-dashed" => DashPreset::SysDash,
        "dash-dot" => DashPreset::DashDot,
        "dash-dot-dot" => DashPreset::SysDashDotDot,
        _ => return None,
    })
}

/// CSS's widths for the keywords, which XSL leaves to the consumer.
fn border_width(keyword: &str) -> Option<f64> {
    match keyword {
        "thin" => Some(0.75),
        "medium" => Some(2.25),
        "thick" => Some(3.75),
        _ => None,
    }
}

/// A border as read: its line, and the style it approximates when that style has no line form of its own.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct ReadBorder {
    pub border: Option<Border<Color>>,
    pub approximated: Option<String>,
}

/// A border as `fo:border` writes it (`0.5pt solid #9e9e9e`, in any order); no border without a style, for `none`
/// and `hidden`, and at no width. A style without its own line form is drawn solid (`approximated`).
pub(crate) fn read_border(value: Option<&str>) -> ReadBorder {
    let mut width: Option<f64> = None;
    let mut style: Option<&str> = None;
    let mut color = None;
    for token in slidock_canon::js_trim(value.unwrap_or(""))
        .split(is_space)
        .filter(|t| !t.is_empty())
    {
        let points = raw_length(Some(token)).or_else(|| border_width(token));
        let hex = hex_color(Some(token));
        if let Some(points) = points {
            width = Some(points);
        } else if let Some(hex) = hex {
            color = Some(hex);
        } else if style.is_none() {
            style = Some(token);
        }
    }
    let Some(style) = style.filter(|s| *s != "none" && *s != "hidden") else {
        return ReadBorder::default();
    };
    let stroke = to_pt(width.unwrap_or(2.25).max(0.0));
    if stroke == 0.0 {
        return ReadBorder::default();
    }
    let dash = border_dash(style);
    ReadBorder {
        border: Some(Border {
            width: size(stroke),
            fill: StrokePaint::Solid {
                color: Color::Rgb(color.unwrap_or_else(|| rgb(0.0, 0.0, 0.0, 1.0))),
            },
            dash: Dash::Preset(dash.unwrap_or(DashPreset::Solid)),
            cap: Cap::Flat,
            join: Join::Miter,
        }),
        approximated: dash.is_none().then(|| utf16::clip(style, 32).to_owned()),
    }
}

/// A cell: its fill, borders and text box, from its resolved style.
fn lower_cell<'a>(
    placed: &Placed<'a>,
    props: &Props<'a>,
    node: &Base,
    ctx: &Lowering<'a, '_>,
) -> Result<Cell, LowerError> {
    let paint = Paint::of(*ctx, Some(&node.id));
    let fill = read_fill(&props.graphic, &paint)?;
    for feature in &fill.features {
        ctx.see(*feature);
    }
    let mut borders = CellBorders {
        top: None,
        right: None,
        bottom: None,
        left: None,
        diagonal_down: None,
        diagonal_up: None,
    };
    let edges = [
        (&mut borders.top, qn!("fo:border-top")),
        (&mut borders.right, qn!("fo:border-right")),
        (&mut borders.bottom, qn!("fo:border-bottom")),
        (&mut borders.left, qn!("fo:border-left")),
        (&mut borders.diagonal_down, qn!("style:diagonal-tl-br")),
        (&mut borders.diagonal_up, qn!("style:diagonal-bl-tr")),
    ];
    let mut any = false;
    for (slot, key) in edges {
        let value = get(&props.cell, key).or_else(|| {
            key.as_str()
                .starts_with("fo:border-")
                .then(|| get(&props.cell, qn!("fo:border")))
                .flatten()
        });
        let read = read_border(value);
        if let Some(style) = read.approximated {
            ctx.report(
                codes::OD9113,
                DiagnosticSite::new()
                    .node(node.id.clone())
                    .data("style", style),
            );
        }
        let Some(border) = read.border else {
            continue;
        };
        ctx.see(FeatureId::Stroke);
        if !border.dash.is_solid() {
            ctx.see(FeatureId::StrokeDash);
        }
        *slot = Some(border);
        any = true;
    }
    let text = TextContext::on(*ctx, &node.id);
    if let Some(mode) = get(&props.cell, qn!("style:writing-mode"))
        && mode.starts_with("tb")
    {
        text.report(
            codes::OD9207,
            DiagnosticSite::new()
                .feature(FeatureId::TextVertical)
                .data("mode", mode),
        );
    }
    let read = match placed.element {
        Some(element) => read_text(
            &ShapeText {
                holder: element,
                shape: element,
                style: props,
                custom_shape: false,
            },
            &text,
        )?,
        None => None,
    };
    let (list_style, paragraphs) = match read {
        Some(read) => read,
        None => {
            let mut list_style = slidock_ir::text::LevelStyles::default();
            set_level(&mut list_style, 0, level_style_of(props, &text));
            let empty = SourceParagraph {
                level: 0,
                props: Default::default(),
                inlines: Vec::new(),
                end_props: None,
            };
            (list_style, vec![empty])
        }
    };
    let (insets, anchor) = text_area(&props.graphic);
    let span = |n: usize| (n > 1).then(|| u32::try_from(n).unwrap_or(u32::MAX));
    Ok(Cell {
        col_span: span(placed.columns),
        row_span: span(placed.rows),
        fill: fill
            .fill
            .filter(|fill| !matches!(fill, Fill::None))
            .map(CellFill::Fill),
        borders: any.then_some(borders),
        text: TextBody {
            body: PartialBodyProps {
                insets: Some(insets),
                anchor: Some(anchor),
                wrap: Some(Wrap::Square),
                autofit: Some(Autofit::None {}),
                overflow: None,
                max_width: None,
            },
            list_style: Some(list_style),
            paragraphs,
            flips_with_frame: false,
            rotation: None,
        },
    })
}

/// A frame's table on the frame's box, as a dense grid; the reason it stays Foreign when its container cannot hold
/// its grid.
pub(crate) fn lower_table<'a>(
    table: &'a Element,
    base: &Base,
    frame: Option<&Frame>,
    ctx: &Lowering<'a, '_>,
) -> Result<Result<Table, String>, LowerError> {
    let grid = grid_of(table);
    if grid.width == 0 || grid.height == 0 {
        return Ok(Err("the table holds no cell".to_owned()));
    }
    if let Some(refused) = grid_refusal(&ctx.state.budget.borrow(), grid.height, grid.width) {
        return Ok(Err(refused));
    }
    let (height, width) = (
        usize::try_from(grid.height).unwrap_or(0),
        usize::try_from(grid.width).unwrap_or(0),
    );
    let styles = ctx.styles;
    let columns = expand(&grid.column_elements);
    let rows = expand(&grid.row_elements);
    let column_style = |c: usize| {
        columns
            .get(c)
            .and_then(|column| column.attr(qn!("table:style-name")))
    };
    let column_props: Vec<_> = (0..width)
        .map(|c| std::rc::Rc::clone(&styles.resolve(Family::TableColumn, column_style(c)).column))
        .collect();
    let widths = column_widths(&column_props, frame.map_or(0.0, |f| f.w.to_f64()));
    let stated_heights: Vec<Option<f64>> = rows
        .iter()
        .map(|row| {
            let props = styles.resolve(Family::TableRow, row.attr(qn!("table:style-name")));
            extent(
                get(&props.row, qn!("style:row-height"))
                    .or_else(|| get(&props.row, qn!("style:min-row-height"))),
            )
        })
        .collect();
    let heights = track_lengths(
        &stated_heights,
        &vec![1.0; rows.len()],
        frame.map_or(0.0, |f| f.h.to_f64()),
    );
    let quantised = |lengths: &[f64]| {
        lengths
            .iter()
            .map(|length| to_pt(*length))
            .collect::<Vec<f64>>()
    };
    if let Some(refused) = take_grid(
        &mut ctx.state.budget.borrow_mut(),
        &quantised(&widths),
        &quantised(&heights),
    ) {
        return Ok(Err(refused));
    }

    ctx.see(FeatureId::ElementTable);
    let template_name = table
        .attr(qn!("table:template-name"))
        .filter(|s| !s.is_empty());
    let template = template_name.and_then(|name| styles.common.table_templates.get(name).copied());
    if let Some(name) = template_name
        && template.is_none()
    {
        ctx.report(
            codes::OD9112,
            DiagnosticSite::new()
                .node(base.id.clone())
                .data("name", utf16::clip(name, 256).to_owned()),
        );
    }
    let template_styles = TemplateStyles::new(table, template, height, width);
    let placed_grid = place(&rows, width);
    let mut lowered: Vec<TableRow<Cell>> = Vec::with_capacity(height);
    for (r, slots_of_row) in placed_grid.iter().enumerate() {
        let mut cells = Vec::with_capacity(width);
        for placed in slots_of_row {
            let Some(placed) = placed else {
                cells.push(None);
                continue;
            };
            let own = placed
                .element
                .and_then(|element| element.attr(qn!("table:style-name")))
                .or_else(|| {
                    rows.get(r)
                        .and_then(|row| row.attr(qn!("table:default-cell-style-name")))
                })
                .or_else(|| {
                    columns
                        .get(placed.column)
                        .and_then(|column| column.attr(qn!("table:default-cell-style-name")))
                });
            let template_style = template_styles.style(placed.row, placed.column);
            let props = styles.over(
                Family::TableCell,
                own,
                &styles.resolve(Family::TableCell, template_style),
            );
            cells.push(Some(lower_cell(placed, &props, base, ctx)?));
        }
        lowered.push(TableRow {
            height: size(to_pt(heights.get(r).copied().unwrap_or(0.0))),
            cells,
        });
    }
    let tracks: Vec<TableColumn> = widths
        .iter()
        .map(|track| TableColumn {
            width: size(to_pt(*track)),
        })
        .collect();
    let frame = frame.map(|frame| {
        let (w, h) = grid_size(&tracks, &lowered);
        Frame {
            w: Size::from_steps(w.steps()),
            h: Size::from_steps(h.steps()),
            ..frame.clone()
        }
    });
    Ok(Ok(Table {
        id: base.id.clone(),
        origin: base.origin.clone(),
        name: base.name.clone(),
        placeholder: base.placeholder.clone(),
        inherits: None,
        match_key: None,
        hidden: base.hidden,
        opacity: None,
        link: base.link.clone(),
        alt: base.alt.clone(),
        ext: None,
        frame,
        columns: tracks,
        rows: lowered,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(value: &str) -> (Option<(f64, DashPreset)>, Option<String>) {
        let read = read_border(Some(value));
        let line = read.border.map(|border| {
            let Dash::Preset(dash) = border.dash else {
                panic!("a preset dash");
            };
            (border.width.to_f64(), dash)
        });
        (line, read.approximated)
    }

    #[test]
    fn parses_a_border_in_any_order_with_keyword_widths_and_no_line_without_a_style() {
        assert_eq!(
            read("#ff0000 dashed 2pt"),
            (Some((2.0, DashPreset::Dash)), None)
        );
        assert_eq!(read("thick solid"), (Some((3.75, DashPreset::Solid)), None));
        assert_eq!(read("1pt #000000"), (None, None));
        assert_eq!(read("1pt hidden #000000"), (None, None));
        assert_eq!(read("0pt solid #000000"), (None, None));
        assert_eq!(read_border(None), ReadBorder::default());
        assert_eq!(
            read("1pt groove #000000"),
            (Some((1.0, DashPreset::Solid)), Some("groove".to_owned()))
        );
    }

    #[test]
    fn reads_relative_widths() {
        assert_eq!(relative_width(Some("1234*")), Some(1234.0));
        assert_eq!(relative_width(Some(" 1.5* ")), Some(1.5));
        assert_eq!(relative_width(Some("1 *")), None);
        assert_eq!(relative_width(Some("*")), None);
        assert_eq!(relative_width(Some("1.*")), None);
        assert_eq!(relative_width(Some("1234567890123*")), None);
    }

    /// The template part each cell of a 3 × 3 table takes, row by row, under the table's `use-*-styles` flags.
    fn template_grid(flags: &str) -> Vec<Vec<&'static str>> {
        const TABLE: &str = "urn:oasis:names:tc:opendocument:xmlns:table:1.0";
        let table = crate::odf::parse(&format!(r#"<table:table xmlns:table="{TABLE}" {flags}/>"#));
        let parts: String = TEMPLATE_PARTS
            .iter()
            .map(|part| format!(r#"<table:{part} table:style-name="{part}"/>"#))
            .collect();
        let template = crate::odf::parse(&format!(
            r#"<table:table-template xmlns:table="{TABLE}">{parts}</table:table-template>"#
        ));
        let styles = TemplateStyles::new(&table, Some(&template), 3, 3);
        let part = |name: Option<&str>| {
            TEMPLATE_PARTS
                .iter()
                .copied()
                .find(|part| Some(*part) == name)
                .unwrap_or("none")
        };
        (0..3)
            .map(|row| {
                (0..3)
                    .map(|column| part(styles.style(row, column)))
                    .collect()
            })
            .collect()
    }

    #[test]
    fn styles_a_cell_by_its_template_position_edges_first_then_bands_then_the_body() {
        assert_eq!(template_grid(""), vec![vec!["body"; 3]; 3]);
        assert_eq!(
            template_grid(r#"table:use-last-column-styles="true""#),
            vec![vec!["body", "body", "last-column"]; 3]
        );
        assert_eq!(
            template_grid(r#"table:use-first-column-styles="true""#),
            vec![vec!["first-column", "body", "body"]; 3]
        );
        assert_eq!(
            template_grid(r#"table:use-first-row-styles="true" table:use-last-row-styles="true""#),
            [vec!["first-row"; 3], vec!["body"; 3], vec!["last-row"; 3]]
        );
        assert_eq!(
            template_grid(concat!(
                r#"table:use-first-row-styles="true" table:use-last-column-styles="true" "#,
                r#"table:use-banding-rows-styles="true""#
            )),
            [
                vec!["first-row"; 3],
                vec!["odd-rows", "odd-rows", "last-column"],
                vec!["even-rows", "even-rows", "last-column"]
            ]
        );
        assert_eq!(
            template_grid(
                r#"table:use-banding-columns-styles="true" table:use-last-row-styles="false""#
            ),
            vec![vec!["even-columns", "odd-columns", "even-columns"]; 3]
        );
    }
}
