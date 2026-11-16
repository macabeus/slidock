//! Tables: a dense grid of columns by rows, each slot a cell or `null` where a spanning cell covers it. The frame is
//! the grid's box (IR0015) and every slot is covered exactly once (IR0014).

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::paint::{Cap, Dash, Join, StrokePaint};
use crate::units::{Pt, Size};
use crate::validate::{Validate, Validator};

/// The most columns, and the most rows, a table may hold; the verifier also bounds their product.
pub const MAX_TABLE_TRACKS: usize = 1000;

/// A cell edge's line: a stroke without line ends.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, bound(deserialize = "C: DeserializeOwned"))]
pub struct Border<C> {
    pub width: Size,
    pub fill: StrokePaint<C>,
    pub dash: Dash,
    pub cap: Cap,
    pub join: Join,
}

impl<C: Validate> Validate for Border<C> {
    fn validate(&self, v: &mut Validator) {
        v.field("width", &self.width);
        v.field("fill", &self.fill);
        v.field("dash", &self.dash);
    }
}

/// The edges a cell draws; an absent edge draws no line. Diagonals run corner to corner across the whole cell.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "camelCase",
    bound(serialize = "C: Serialize", deserialize = "C: DeserializeOwned")
)]
pub struct CellBorders<C> {
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub top: Option<Border<C>>,
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub right: Option<Border<C>>,
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub bottom: Option<Border<C>>,
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub left: Option<Border<C>>,
    /// Top-left to bottom-right.
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub diagonal_down: Option<Border<C>>,
    /// Bottom-left to top-right.
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub diagonal_up: Option<Border<C>>,
}

/// The edge keys of `TableCell.borders`.
pub const BORDER_EDGES: [&str; 6] = [
    "top",
    "right",
    "bottom",
    "left",
    "diagonalDown",
    "diagonalUp",
];

impl<C> CellBorders<C> {
    /// The edges in [`BORDER_EDGES`] order.
    pub fn edges(&self) -> [(&'static str, Option<&Border<C>>); 6] {
        [
            ("top", self.top.as_ref()),
            ("right", self.right.as_ref()),
            ("bottom", self.bottom.as_ref()),
            ("left", self.left.as_ref()),
            ("diagonalDown", self.diagonal_down.as_ref()),
            ("diagonalUp", self.diagonal_up.as_ref()),
        ]
    }
}

impl<C: Validate> Validate for CellBorders<C> {
    fn validate(&self, v: &mut Validator) {
        for (key, border) in self.edges() {
            if let Some(border) = border {
                v.at(key, |v| border.validate(v));
            }
        }
    }
}

/// A cell: the columns and rows it spans from its own (absent for one), its fill (absent: what is behind shows),
/// its edges and its text body. `F` is the fill, `C` the colour, `T` the text body of the phase.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    deny_unknown_fields,
    rename_all = "camelCase",
    bound(
        serialize = "F: Serialize, C: Serialize, T: Serialize",
        deserialize = "F: Deserialize<'de>, C: DeserializeOwned, T: Deserialize<'de>"
    )
)]
pub struct TableCell<F, C, T> {
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub col_span: Option<u32>,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub row_span: Option<u32>,
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub fill: Option<F>,
    #[serde(
        default = "Option::default",
        skip_serializing_if = "Option::is_none",
        with = "crate::de::optional"
    )]
    pub borders: Option<CellBorders<C>>,
    pub text: T,
}

impl<F, C, T> TableCell<F, C, T> {
    /// The columns and rows the cell spans.
    pub fn spans(&self) -> (usize, usize) {
        (
            self.col_span.map_or(1, |n| n as usize),
            self.row_span.map_or(1, |n| n as usize),
        )
    }
}

fn validate_span(span: Option<u32>, key: &'static str, v: &mut Validator) {
    if let Some(span) = span {
        v.at(key, |v| v.int(i64::from(span), 2, MAX_TABLE_TRACKS as i64));
    }
}

impl<F: Validate, C: Validate, T: Validate> Validate for TableCell<F, C, T> {
    fn validate(&self, v: &mut Validator) {
        validate_span(self.col_span, "colSpan", v);
        validate_span(self.row_span, "rowSpan", v);
        v.field("fill", &self.fill);
        v.field("borders", &self.borders);
        v.field("text", &self.text);
    }
}

/// A column of the grid: its width between grid lines.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableColumn {
    pub width: Size,
}

impl Validate for TableColumn {
    fn validate(&self, v: &mut Validator) {
        v.field("width", &self.width);
    }
}

/// A row: its least height between grid lines (a row grows to fit its cells' text) and one entry per column.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TableRow<Cell> {
    pub height: Size,
    pub cells: Vec<Option<Cell>>,
}

impl<Cell: Validate> Validate for TableRow<Cell> {
    fn validate(&self, v: &mut Validator) {
        v.field("height", &self.height);
        v.at("cells", |v| {
            v.array(self.cells.len(), 0, MAX_TABLE_TRACKS);
            for (i, cell) in self.cells.iter().enumerate() {
                if let Some(cell) = cell {
                    v.at(i, |v| cell.validate(v));
                }
            }
        });
    }
}

/// Validates a table's tracks: 1 to 1000 columns and rows.
pub(crate) fn validate_tracks<Cell: Validate>(
    columns: &[TableColumn],
    rows: &[TableRow<Cell>],
    v: &mut Validator,
) {
    v.at("columns", |v| {
        v.array(columns.len(), 1, MAX_TABLE_TRACKS);
        for (i, column) in columns.iter().enumerate() {
            v.at(i, |v| column.validate(v));
        }
    });
    v.at("rows", |v| {
        v.array(rows.len(), 1, MAX_TABLE_TRACKS);
        for (i, row) in rows.iter().enumerate() {
            v.at(i, |v| row.validate(v));
        }
    });
}

/// The box a grid fills: its columns' widths and its rows' heights, each summed as JS numbers and quantised.
pub fn grid_size<Cell>(columns: &[TableColumn], rows: &[TableRow<Cell>]) -> (Pt, Pt) {
    let sum = |lengths: &mut dyn Iterator<Item = f64>| {
        let total = lengths.fold(0.0, |total, length| total + length);
        Pt::quantise(total).unwrap_or(Pt::ZERO)
    };
    (
        sum(&mut columns.iter().map(|column| column.width.to_f64())),
        sum(&mut rows.iter().map(|row| row.height.to_f64())),
    )
}

/// Each cell with its grid position (row, column), in row-major order. The grid is dense: `rows[r].cells[c]`
/// is the slot at row r, column c, so a cell sits where it is written.
pub fn table_cells<Cell>(rows: &[TableRow<Cell>]) -> impl Iterator<Item = (&Cell, usize, usize)> {
    rows.iter().enumerate().flat_map(|(r, row)| {
        row.cells
            .iter()
            .enumerate()
            .filter_map(move |(c, cell)| cell.as_ref().map(|cell| (cell, r, c)))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(height: f64, cells: Vec<Option<u8>>) -> TableRow<u8> {
        TableRow {
            height: Size::exact(height).unwrap(),
            cells,
        }
    }

    #[test]
    fn sums_the_grid_as_js_numbers() {
        let columns = [0.1, 0.2].map(|w| TableColumn {
            width: Size::exact(w).unwrap(),
        });
        let rows = [row(10.5, vec![]), row(20.25, vec![])];
        let (w, h) = grid_size(&columns, &rows);
        assert_eq!(w.to_f64(), 0.3);
        assert_eq!(h.to_f64(), 30.75);
    }

    #[test]
    fn lists_cells_in_row_major_order() {
        let rows = [
            row(1.0, vec![Some(1), None]),
            row(1.0, vec![Some(2), Some(3)]),
        ];
        let cells: Vec<_> = table_cells(&rows)
            .map(|(cell, r, c)| (*cell, r, c))
            .collect();
        assert_eq!(cells, [(1, 0, 0), (2, 1, 0), (3, 1, 1)]);
    }
}
