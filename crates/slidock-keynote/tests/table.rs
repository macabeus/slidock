//! Keynote tables: the packed cell storage, value formats, and lowering a table's model to the IR's table, with what
//! it approximates and what it cannot read.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::collections::BTreeMap;

use prost::Message;
use serde_json::{Value, json};
use slidock_core_api::Limits;
use slidock_keynote::cells::{CellRecord, CellValue, Decimal, decimal128, row_cells, value_text};
use slidock_keynote::proto::{tsk, tst};
use support::fixtures::tables_presentation;
use support::harness::{Lowered, compile, lower, lower_with};
use support::keynote::{Presentation, rgb};
use support::tables::{
    BOOL, CURRENCY, DATE, DURATION, ERROR, NUMBER, Strokes, TEXT, TestCell, TestMerge, TestTable,
    add_table, cell_style, decimal128 as write_decimal128, solid, text, tile_row,
};

fn row_info(row: &Value) -> tst::TileRowInfo {
    tst::TileRowInfo::decode(support::json::encode("TST.TileRowInfo", row).as_slice()).unwrap()
}

fn with(mut cell: TestCell, change: impl FnOnce(&mut TestCell)) -> TestCell {
    change(&mut cell);
    cell
}

#[test]
fn reads_decimal128_values_as_numbers_writes_them() {
    for (digits, exponent, negative) in [
        (99u128, -2, false),
        (0, 0, false),
        (12_345_678_901_234_567_890_123_456_789, 10, true),
        ((1 << 113) - 1, -6176, false),
    ] {
        assert_eq!(
            decimal128(&write_decimal128(digits, exponent, negative)),
            Decimal {
                digits,
                exponent,
                negative
            }
        );
    }
}

#[test]
fn reads_a_row_through_16_bit_offsets_and_through_wide_ones_alike() {
    let cells = [
        Some(with(text(1), |c| c.cell_style = Some(2))),
        None,
        Some(TestCell {
            double: Some(1.5),
            number_format: Some(3),
            ..cell(NUMBER)
        }),
    ];
    let narrow = row_cells(&row_info(&tile_row(0, &cells, false))).unwrap();
    assert_eq!(
        row_cells(&row_info(&tile_row(0, &cells, true))).unwrap(),
        narrow
    );
    let expected: BTreeMap<usize, CellRecord> = [
        (
            0,
            CellRecord {
                cell_type: TEXT,
                string: Some(1),
                cell_style: Some(2),
                ..CellRecord::default()
            },
        ),
        (
            2,
            CellRecord {
                cell_type: NUMBER,
                double: Some(1.5),
                number_format: Some(3),
                ..CellRecord::default()
            },
        ),
    ]
    .into();
    assert_eq!(narrow, expected);
}

#[test]
fn reads_a_row_keynote_14_5_wrote_each_cell_skipping_the_fields_it_does_not_keep() {
    // A row of a Keynote 14.5 table: three text cells with a cell style, a text style, a suggestion and a text
    // format.
    let buffer: Vec<u8> = [
        "BQMAAAAAAABoEAIAAQAAAAEAAAACAAAABQAAAAEAAAAFAwAAAAAAAGgQAgACAAAAAQAAAAIAAAAFAAAAAQAAAAUDAAAAAAAAaBACAA",
        "MAAAABAAAAAgAAAAUAAAABAAAA",
    ]
    .concat()
    .as_bytes()
    .to_vec();
    let buffer = base64(&buffer);
    let mut offsets = vec![0xffu8; 512];
    offsets[..6].copy_from_slice(&[0, 0, 32, 0, 64, 0]);
    let info = tst::TileRowInfo {
        storage_version: Some(5),
        cell_storage_buffer: Some(buffer),
        cell_offsets: Some(offsets),
        ..tst::TileRowInfo::default()
    };
    let cells: Vec<CellRecord> = row_cells(&info).unwrap().into_values().collect();
    let expected: Vec<CellRecord> = [1, 2, 3]
        .map(|string| CellRecord {
            cell_type: TEXT,
            string: Some(string),
            cell_style: Some(1),
            text_style: Some(2),
            ..CellRecord::default()
        })
        .to_vec();
    assert_eq!(cells, expected);
}

/// Unpadded base64 of whole 3-byte groups.
fn base64(text: &[u8]) -> Vec<u8> {
    assert_eq!(text.len() % 4, 0);
    let value = |c: u8| match c {
        b'A'..=b'Z' => c - b'A',
        b'a'..=b'z' => c - b'a' + 26,
        b'0'..=b'9' => c - b'0' + 52,
        b'+' => 62,
        _ => 63,
    };
    let mut out = Vec::new();
    for chunk in text.chunks(4) {
        let n = chunk
            .iter()
            .fold(0u32, |n, c| (n << 6) | u32::from(value(*c)));
        out.extend(&n.to_be_bytes()[1..]);
    }
    out
}

#[test]
fn refuses_rows_it_cannot_read() {
    let mut older = row_info(&tile_row(0, &[Some(text(1))], false));
    older.storage_version = Some(4);
    let mut other_cell = row_info(&tile_row(0, &[Some(text(1))], false));
    other_cell.cell_storage_buffer.as_mut().unwrap()[0] = 4;
    let mut past = row_info(&tile_row(0, &[Some(text(1))], false));
    past.cell_storage_buffer = Some(vec![5; 14]);
    for (what, row, reason) in [
        ("an older storage version", older, "cell storage version 4"),
        (
            "a cell of another version",
            other_cell,
            "cell storage version 4",
        ),
        ("a cell past its row", past, "a cell at 0 runs past its row"),
    ] {
        assert_eq!(row_cells(&row).unwrap_err().0, reason, "{what}");
    }
}

fn cell(cell_type: u8) -> TestCell {
    TestCell {
        cell_type,
        ..TestCell::default()
    }
}

fn record(cell: &TestCell) -> CellRecord {
    CellRecord {
        cell_type: cell.cell_type,
        decimal: cell.decimal.map(|(digits, exponent, negative)| Decimal {
            digits,
            exponent,
            negative,
        }),
        double: cell.double,
        seconds: cell.seconds,
        number_format: cell.number_format,
        currency_format: cell.currency_format,
        bool_format: cell.bool_format,
        ..CellRecord::default()
    }
}

#[test]
fn shows_each_value_in_its_format() {
    let formats: BTreeMap<u32, tsk::FormatStructArchive> = [
        (1, (258, Some(253), None, None)),
        (2, (256, Some(2), Some(true), None)),
        (3, (256, Some(0), None, None)),
        (4, (257, Some(2), None, None)),
        (5, (258, Some(1), None, Some(1))),
        (6, (263, None, None, None)),
    ]
    .map(
        |(key, (format_type, decimal_places, show_thousands_separator, negative_style))| {
            (
                key,
                tsk::FormatStructArchive {
                    format_type: Some(format_type),
                    decimal_places,
                    show_thousands_separator,
                    negative_style,
                    ..tsk::FormatStructArchive::default()
                },
            )
        },
    )
    .into();
    let value =
        |cell: &TestCell| value_text(&record(cell), |key| key.and_then(|key| formats.get(&key)));
    let decimal = |digits: u128, exponent: i32, format: Option<u32>, negative: bool| TestCell {
        decimal: Some((digits, exponent, negative)),
        number_format: format,
        ..cell(NUMBER)
    };
    let shown = |text: &str| {
        Some(CellValue {
            text: text.to_owned(),
            approximated: None,
        })
    };
    let approximated = |text: &str, format: &str| {
        Some(CellValue {
            text: text.to_owned(),
            approximated: Some(format.to_owned()),
        })
    };
    let double = |cell_type: u8, double: f64| TestCell {
        double: Some(double),
        ..cell(cell_type)
    };
    let date = |seconds: f64| TestCell {
        seconds: Some(seconds),
        ..cell(DATE)
    };
    for (what, cell, expected) in [
        (
            "an automatic number, exactly",
            decimal(95, -2, None, false),
            shown("0.95"),
        ),
        (
            "a double, by its shortest digits",
            double(NUMBER, 0.1 + 0.2),
            shown("0.30000000000000004"),
        ),
        (
            "a percentage",
            decimal(95, -2, Some(1), false),
            shown("95%"),
        ),
        (
            "fixed places with separators",
            decimal(12_345_675, -3, Some(2), false),
            shown("12,345.68"),
        ),
        (
            "no places, rounding half away from zero",
            decimal(25, -1, Some(3), true),
            shown("-3"),
        ),
        (
            "a negative zero as zero",
            decimal(4, -1, Some(3), true),
            shown("0"),
        ),
        (
            "a large exponent, clamped",
            decimal(1, 400, None, false),
            shown(&format!("1{}", "0".repeat(308))),
        ),
        (
            "a currency, plain",
            decimal(5, 0, Some(4), false),
            approximated("5", "currency"),
        ),
        (
            "a red negative",
            decimal(5, -1, Some(5), true),
            approximated("-50.0%", "negative number"),
        ),
        (
            "a date",
            date(86400.0 * 365.0),
            approximated("2002-01-01", "date"),
        ),
        (
            "a leap day",
            date(86400.0 * 8459.0 + 3600.0),
            approximated("2024-02-29", "date"),
        ),
        (
            "a date before the epoch",
            date(-1.0),
            approximated("2000-12-31", "date"),
        ),
        (
            "a duration",
            double(DURATION, 90.0),
            approximated("90s", "duration"),
        ),
        ("a boolean", double(BOOL, 1.0), shown("TRUE")),
        (
            "a checkbox",
            TestCell {
                bool_format: Some(6),
                ..double(BOOL, 0.0)
            },
            approximated("FALSE", "checkbox"),
        ),
        (
            "an error",
            cell(ERROR),
            approximated("#ERROR", "formula error"),
        ),
        (
            "a currency, as its plain number",
            TestCell {
                decimal: Some((1999, -2, false)),
                ..cell(CURRENCY)
            },
            approximated("19.99", "currency"),
        ),
        (
            "a currency in its format",
            TestCell {
                currency_format: Some(4),
                ..double(CURRENCY, 5.0)
            },
            approximated("5", "currency"),
        ),
        ("nothing for a type it does not know", double(4, 3.0), None),
    ] {
        assert_eq!(value(&cell), expected, "{what}");
    }
}

/// A table drawable on a slide, lowered; `at` gives a code's data on the table.
struct LoweredTable {
    element: Value,
    lowered: Lowered,
}

impl LoweredTable {
    fn at(&self, code: &str) -> Vec<Value> {
        self.lowered
            .diagnostics
            .iter()
            .filter(|d| {
                d.code.code() == code
                    && d.node.as_ref().map(|n| n.as_str()) == self.element["id"].as_str()
            })
            .map(|d| serde_json::to_value(&d.data).unwrap())
            .collect()
    }

    fn rows(&self) -> Vec<Vec<Value>> {
        self.element["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["cells"].as_array().unwrap().clone())
            .collect()
    }
}

fn table_of(elements: &[Value], index: usize) -> Value {
    elements
        .iter()
        .filter(|e| e["kind"] == "table" || e["kind"] == "foreign")
        .nth(index)
        .unwrap()
        .clone()
}

fn lower_table(build: impl FnOnce(&mut Presentation) -> TestTable) -> LoweredTable {
    let mut p = Presentation::new();
    let spec = build(&mut p);
    let table = add_table(&mut p, &spec);
    p.slide(json!({ "drawables_z_order": [table] }), json!({}), None);
    let lowered = lower(&p.bytes());
    let element = table_of(lowered.elements(0), 0);
    LoweredTable { element, lowered }
}

fn grid(rows: usize, columns: usize, cell: impl Fn(usize, usize) -> TestCell) -> TestTable {
    TestTable {
        widths: vec![50.0; columns],
        heights: vec![20.0; rows],
        strings: (0..rows * columns).map(|i| format!("t{}", i + 1)).collect(),
        rows: (0..rows)
            .map(|r| (0..columns).map(|c| Some(cell(r, c))).collect())
            .collect(),
        ..TestTable::default()
    }
}

fn numbered(columns: usize) -> impl Fn(usize, usize) -> TestCell {
    move |r, c| text((r * columns + c + 1) as u32)
}

fn cell_text(cell: &Value) -> Option<String> {
    if cell.is_null() {
        return None;
    }
    Some(
        cell["text"]["paragraphs"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|p| p["inlines"].as_array().unwrap().iter())
            .map(|inline| inline["text"].as_str().unwrap_or_default().to_owned())
            .collect(),
    )
}

#[test]
fn lowers_a_table_with_its_frame_summed_from_its_tracks_and_every_slot_covered_once() {
    let lowered = lower(&tables_presentation());
    let elements = lowered.elements(0);
    let (regions, plain) = (&elements[0], &elements[1]);
    assert_eq!(regions["kind"], "table");
    let frame = &regions["frame"];
    assert_eq!(
        [&frame["x"], &frame["y"], &frame["w"], &frame["h"]],
        [&json!(100), &json!(100), &json!(320), &json!(150)]
    );
    let spans: Vec<Vec<Value>> = regions["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row["cells"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| {
                    if cell.is_null() {
                        Value::Null
                    } else {
                        json!([
                            cell.get("rowSpan").unwrap_or(&json!(1)),
                            cell.get("colSpan").unwrap_or(&json!(1))
                        ])
                    }
                })
                .collect()
        })
        .collect();
    assert_eq!(
        json!(spans),
        json!([
            [[1, 1], [1, 1], [1, 1]],
            [[1, 1], [1, 1], [1, 1]],
            [[1, 1], [1, 1], [2, 1]],
            [[1, 1], [1, 1], null],
            [[1, 2], null, [1, 1]],
        ])
    );
    assert_eq!(
        (
            &plain["frame"]["w"],
            &plain["frame"]["h"],
            &plain["frame"]["rotation"]
        ),
        (&json!(200), &json!(40), &json!(350))
    );
    assert_eq!(plain["columns"], json!([{ "width": 200 }]));
    let cell = &plain["rows"][0]["cells"][0];
    assert_eq!(cell_text(cell).unwrap(), "Wide offsets");
    // No styles at all: Keynote's default padding, no fill, no lines.
    assert_eq!(
        cell.as_object().unwrap().keys().collect::<Vec<_>>(),
        ["text"]
    );
    assert_eq!(
        cell["text"]["body"],
        json!({ "insets": { "l": 4, "t": 4, "r": 4, "b": 4 }, "anchor": "top", "wrap": "square", "autofit": { "kind": "none" } })
    );
    // As a drawable's text, a cell's is mirrored by the table's flips.
    assert_eq!(cell["text"]["flipsWithFrame"], json!(true));
}

#[test]
fn gives_each_cell_its_region_style_and_bands_alternate_body_rows() {
    let lowered = lower(&tables_presentation());
    let regions = &lowered.elements(0)[0];
    let fills: Vec<Vec<Value>> = regions["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row["cells"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| match &cell["fill"] {
                    fill if fill["kind"] == "solid" => fill["color"]["r"].clone(),
                    fill => fill.get("kind").cloned().unwrap_or(Value::Null),
                })
                .collect()
        })
        .collect();
    assert_eq!(
        json!(fills),
        json!([
            [0.2, 0.2, 0.2],
            [0.9, null, null],
            [null, 0.95, 0.95],
            [0.9, "linear", null],
            [null, null, null],
        ])
    );
    assert_eq!(
        regions["rows"][0]["cells"][0]["text"]["paragraphs"][0]["props"]["align"],
        "center"
    );
    // Natural alignment sets numbers right and text left.
    let aligns: Vec<&Value> = regions["rows"][1]["cells"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cell| &cell["text"]["paragraphs"][0]["props"]["align"])
        .collect();
    assert_eq!(aligns, [&json!("left"), &json!("right"), &json!("right")]);
    // Keynote pads from the strokes' inner edges: the padding (2, 0, 0, 0) and half of each edge's stroke.
    let body = &regions["rows"][2]["cells"][0]["text"]["body"];
    assert_eq!(
        body["insets"],
        json!({ "l": 3, "t": 0.75, "r": 1, "b": 0.5 })
    );
    assert_eq!(
        (&body["anchor"], &body["wrap"]),
        (&json!("bottom"), &json!("none"))
    );
}

#[test]
fn draws_one_stroke_per_segment_the_run_of_higher_order_an_empty_run_as_none_else_the_table_style()
{
    let lowered = lower(&tables_presentation());
    let regions = &lowered.elements(0)[0];
    let width = |cell: &Value, edge: &str| cell["borders"][edge]["width"].clone();
    let (north, quarter) = (
        &regions["rows"][1]["cells"][0],
        &regions["rows"][1]["cells"][1],
    );
    let south = &regions["rows"][2]["cells"][0];
    // The top of row 2 (order 2, dashed red) over the bottom of row 1 (order 1, blue), stated by both cells.
    assert_eq!(north["borders"]["bottom"]["width"], 1.5);
    assert_eq!(
        north["borders"]["bottom"]["dash"],
        json!({ "custom": [4, 3] })
    );
    assert_eq!(south["borders"]["top"], north["borders"]["bottom"]);
    // The empty run between the header column and the first value: no line on either side.
    assert_eq!(
        (width(north, "right"), width(quarter, "left")),
        (Value::Null, Value::Null)
    );
    // Elsewhere, the region defaults: the header border, separator and the header column separator.
    let header = &regions["rows"][0]["cells"][0];
    assert_eq!(
        (width(header, "top"), width(header, "bottom")),
        (json!(2), json!(3))
    );
    assert_eq!(width(south, "right"), json!(2));
}

#[test]
fn reports_a_format_it_shows_plain_once_per_format() {
    let table = lower_table(|_| TestTable {
        formats: vec![json!({ "format_type": 257 })],
        ..grid(1, 3, |_, _| TestCell {
            double: Some(2.0),
            number_format: Some(1),
            ..cell(NUMBER)
        })
    });
    assert_eq!(table.at("KN1111"), [json!({ "format": "currency" })]);
}

#[test]
fn shows_a_currency_as_a_number_and_reports_the_values_of_a_type_it_does_not_know_once_per_type() {
    let table = lower_table(|_| {
        grid(1, 3, |_, c| {
            if c == 0 {
                TestCell {
                    decimal: Some((1999, -2, false)),
                    ..cell(CURRENCY)
                }
            } else {
                TestCell {
                    double: Some(3.0),
                    ..cell(4)
                }
            }
        })
    });
    let cells = &table.rows()[0];
    let texts: Vec<Option<String>> = cells.iter().map(cell_text).collect();
    assert_eq!(
        texts,
        [
            Some("19.99".to_owned()),
            Some(String::new()),
            Some(String::new())
        ]
    );
    assert_eq!(cells[0]["text"]["paragraphs"][0]["props"]["align"], "right");
    assert_eq!(table.at("KN1111"), [json!({ "format": "currency" })]);
    assert_eq!(
        table.at("KN1112"),
        [json!({ "what": "the values of its type 4 cells are not read" })]
    );
}

fn merge_of(
    row: u32,
    column: u32,
    rows: Option<u32>,
    columns: Option<u32>,
    tiled: bool,
) -> TestMerge {
    TestMerge {
        row,
        column,
        rows,
        columns,
        tiled,
    }
}

#[test]
fn reports_what_it_approximates_kn1112() {
    type Change = Box<dyn Fn(&mut TestTable)>;
    let cases: Vec<(&str, Change, &str)> = vec![
        (
            "hidden tracks",
            Box::new(|t| t.hidden_rows = vec![1]),
            "hidden rows and columns are shown",
        ),
        (
            "an overlapping merge",
            Box::new(|t| {
                t.merges = vec![
                    merge_of(0, 0, None, Some(2), false),
                    merge_of(0, 1, Some(2), None, false),
                ];
            }),
            "merged rows 0\u{2013}1, columns 1\u{2013}1 overlaps another or leaves the grid; its cells stay apart",
        ),
        (
            "a merge past the grid",
            Box::new(|t| t.merges = vec![merge_of(1, 1, Some(2), None, true)]),
            "merged rows 1\u{2013}2, columns 1\u{2013}1 overlaps another or leaves the grid; its cells stay apart",
        ),
        (
            "an unknown merge owner",
            Box::new(|t| t.model = json!({ "merge_owner": { "owner_id": { "uuid_w0": 1 } } })),
            "its merged cells are not read",
        ),
        (
            "a merged edge of two strokes",
            Box::new(|t| {
                t.merges = vec![merge_of(0, 0, None, Some(2), false)];
                t.strokes.top = vec![(
                    0,
                    vec![
                        json!({ "origin": 1, "length": 1, "order": 1, "stroke": solid(3.0, None) }),
                    ],
                )];
            }),
            "a merged cell's edge draws its first segment's stroke along its length",
        ),
        (
            "a missing string",
            Box::new(|t| t.strings = Vec::new()),
            "the text of the cell at row 0, column 0 is missing",
        ),
        (
            "conditional highlighting",
            Box::new(|t| {
                t.rows = vec![
                    vec![
                        Some(with(text(1), |c| c.conditional_style = Some(1))),
                        Some(text(2)),
                    ],
                    vec![Some(text(3)), Some(text(4))],
                ];
            }),
            "conditional highlighting is not applied",
        ),
    ];
    for (what, change, reason) in cases {
        let table = lower_table(|_| {
            let mut table = grid(2, 2, numbered(2));
            change(&mut table);
            table
        });
        assert_eq!(table.element["kind"], "table", "{what}");
        assert!(
            table.at("KN1112").contains(&json!({ "what": reason })),
            "{what}: {:?}",
            table.at("KN1112")
        );
    }
}

#[test]
fn tells_a_merged_edges_segments_apart_as_their_strokes_print_as_json() {
    const MIXED: &str = "a merged cell's edge draws its first segment's stroke along its length";
    let width = |width: &str| json!({ "width": width, "color": rgb(0.0, 0.0, 0.0), "pattern": { "type": 1 } });
    let pattern = |dashes: Value| json!({ "type": 0, "phase": 0, "count": 2, "pattern": dashes });
    let cases: Vec<(&str, Value, Value, bool)> = vec![
        ("equal", solid(1.0, None), solid(1.0, None), false),
        (
            "zero and negative zero",
            solid(1.0, Some(rgb(0.0, 0.0, 0.0))),
            solid(1.0, Some(rgb(-0.0, 0.0, -0.0))),
            false,
        ),
        (
            "two non-finite widths",
            width("NaN"),
            width("-Infinity"),
            false,
        ),
        (
            "equal dashes",
            json!({ "width": 1, "pattern": pattern(json!([2, -0.0])) }),
            json!({ "width": 1, "pattern": pattern(json!([2, 0])) }),
            false,
        ),
        ("widths", solid(1.0, None), solid(2.0, None), true),
        (
            "a finite and a non-finite width",
            solid(1.0, None),
            width("Infinity"),
            true,
        ),
        (
            "colours",
            solid(1.0, Some(rgb(0.0, 0.0, 0.0))),
            solid(1.0, Some(rgb(0.0, 0.5, 0.0))),
            true,
        ),
        (
            "a colour and none",
            solid(1.0, None),
            json!({ "width": 1, "pattern": { "type": 1 } }),
            true,
        ),
        (
            "dash lengths",
            json!({ "width": 1, "pattern": pattern(json!([2, 2])) }),
            json!({ "width": 1, "pattern": pattern(json!([2, 2, 2])) }),
            true,
        ),
        (
            "a cap",
            solid(1.0, None),
            json!({ "width": 1, "color": rgb(0.0, 0.0, 0.0), "pattern": { "type": 1 }, "cap": 1 }),
            true,
        ),
    ];
    for (what, first, second, mixed) in cases {
        let table = lower_table(|_| TestTable {
            merges: vec![merge_of(0, 0, None, Some(2), false)],
            strokes: Strokes {
                top: vec![(
                    0,
                    vec![
                        json!({ "origin": 0, "length": 1, "order": 1, "stroke": first }),
                        json!({ "origin": 1, "length": 1, "order": 2, "stroke": second }),
                    ],
                )],
                ..Strokes::default()
            },
            ..grid(2, 2, numbered(2))
        });
        assert_eq!(
            table.at("KN1112").contains(&json!({ "what": MIXED })),
            mixed,
            "{what}"
        );
    }
}

#[test]
fn keeps_an_overlapping_merge_out_and_the_first_in_so_the_grid_stays_whole() {
    let table = lower_table(|_| TestTable {
        merges: vec![
            merge_of(0, 0, None, Some(2), false),
            merge_of(0, 1, Some(2), None, false),
        ],
        ..grid(2, 2, numbered(2))
    });
    let texts: Vec<Vec<Option<String>>> = table
        .rows()
        .iter()
        .map(|row| row.iter().map(cell_text).collect())
        .collect();
    assert_eq!(
        texts,
        [
            vec![Some("t1".to_owned()), None],
            vec![Some("t3".to_owned()), Some("t4".to_owned())]
        ]
    );
}

#[test]
fn checks_a_merge_against_the_grid_before_it_visits_its_slots_whatever_area_it_claims() {
    for tiled in [false, true] {
        let table = lower_table(|_| TestTable {
            merges: vec![merge_of(0, 0, Some(6000), Some(6000), tiled)],
            ..grid(2, 2, numbered(2))
        });
        assert_eq!(table.element["kind"], "table");
        assert_eq!(
            table.at("KN1112"),
            [
                json!({ "what": "merged rows 0\u{2013}5999, columns 0\u{2013}5999 overlaps another or leaves the grid; its cells stay apart" })
            ]
        );
    }
}

#[test]
fn reports_a_shown_table_name_as_a_dropped_caption_kn1109() {
    let table = lower_table(|_| TestTable {
        model: json!({ "table_name_enabled": true }),
        ..grid(1, 1, numbered(1))
    });
    assert_eq!(table.at("KN1109"), [json!({ "what": "table name" })]);
}

#[test]
fn reads_a_cell_style_chain_and_a_fill_set_to_none_as_no_fill() {
    let table = lower_table(|p| {
        let base = cell_style(
            p,
            json!({ "cell_fill": {}, "vertical_alignment": 2, "padding": { "top": 9 } }),
            None,
        );
        let style = cell_style(p, json!({ "text_wrap": false }), Some(base));
        TestTable {
            table_style: json!({ "banded_rows": true, "banded_fill": { "color": rgb(1.0, 0.0, 0.0) } }),
            styles: vec![style],
            ..grid(1, 2, |_, c| {
                with(text(c as u32 + 1), |cell| cell.cell_style = Some(1))
            })
        }
    });
    let first = &table.rows()[0][0];
    assert!(first.get("fill").is_none());
    let body = &first["text"]["body"];
    assert_eq!(body["insets"], json!({ "l": 0, "t": 9, "r": 0, "b": 0 }));
    assert_eq!(
        (&body["anchor"], &body["wrap"]),
        (&json!("bottom"), &json!("none"))
    );
}

#[test]
fn keeps_a_table_it_cannot_read_as_a_foreign_which_the_core_declines() {
    for (what, model, reason) in [
        (
            "column tiles",
            json!({ "base_data_store": { "columnTileTree": { "nodes": [{ "key": 256, "value": 1 }] } } }),
            "the table is split into column tiles",
        ),
        (
            "more cells than a container holds",
            json!({ "number_of_rows": 101, "number_of_columns": 100 }),
            "the table's 101 × 100 grid is more than the 10000 cells its container has left",
        ),
        (
            "more rows than a table holds",
            json!({ "number_of_rows": 1001 }),
            "the table's 1001 × 1 grid has more than 1000 rows or columns",
        ),
        (
            "tracks past the coordinate range",
            json!({ "number_of_rows": 3, "default_row_height": 90_000 }),
            "the table's tracks add up past the IR's coordinate range",
        ),
        (
            "no cells",
            json!({ "number_of_rows": 0 }),
            "the table has no cells",
        ),
        (
            "a missing tile",
            json!({ "base_data_store": { "tiles": { "tiles": [{ "tileid": 0, "tile": { "identifier": "999999" } }] } } }),
            "tile 0 is missing",
        ),
    ] {
        let mut p = Presentation::new();
        let table = add_table(
            &mut p,
            &TestTable {
                model,
                ..grid(1, 1, numbered(1))
            },
        );
        p.slide(json!({ "drawables_z_order": [table] }), json!({}), None);
        let bytes = p.bytes();
        let element = table_of(lower(&bytes).elements(0), 0);
        assert_eq!(
            (&element["kind"], &element["type"]),
            (&json!("foreign"), &json!("table")),
            "{what}"
        );
        assert_eq!(element["payload"]["reason"], reason, "{what}");
        let out = compile(&bytes, "presentation.key");
        let declined: Vec<&str> = out
            .diagnostics
            .iter()
            .filter(|d| d.code.code() == "IR0910")
            .map(|d| d.node.as_ref().unwrap().as_str())
            .collect();
        assert_eq!(declined, [element["id"].as_str().unwrap()], "{what}");
    }
}

fn sized(rows: u32) -> TestTable {
    TestTable {
        model: json!({ "number_of_rows": rows, "number_of_columns": 100 }),
        ..grid(1, 1, numbered(1))
    }
}

fn kinds(elements: &[Value]) -> Vec<&str> {
    elements
        .iter()
        .map(|e| e["kind"].as_str().unwrap())
        .collect()
}

#[test]
fn counts_every_tables_cells_against_its_containers_budget() {
    let mut p = Presentation::new();
    let a = add_table(&mut p, &sized(60));
    let b = add_table(&mut p, &sized(50));
    p.slide(json!({ "drawables_z_order": [a, b] }), json!({}), None);
    let c = add_table(&mut p, &sized(50));
    p.slide(json!({ "drawables_z_order": [c] }), json!({}), None);
    let lowered = lower(&p.bytes());
    let (first, second) = (lowered.elements(0), lowered.elements(1));
    assert_eq!(kinds(first), ["table", "foreign"]);
    assert_eq!(
        table_of(first, 1)["payload"]["reason"],
        "the table's 50 × 100 grid is more than the 4000 cells its container has left"
    );
    assert_eq!(kinds(second), ["table"]);
}

#[test]
fn counts_every_containers_tables_against_one_presentation_budget() {
    let mut p = Presentation::new();
    for rows in [60, 50, 40] {
        let table = add_table(&mut p, &sized(rows));
        p.slide(json!({ "drawables_z_order": [table] }), json!({}), None);
    }
    let limits = Limits {
        cells_per_presentation: 10_000,
        ..Limits::default()
    };
    let lowered = lower_with(&p.bytes(), None, limits);
    let tables: Vec<Value> = (0..3).map(|i| table_of(lowered.elements(i), 0)).collect();
    let kinds: Vec<&Value> = tables.iter().map(|t| &t["kind"]).collect();
    assert_eq!(
        kinds,
        [&json!("table"), &json!("foreign"), &json!("foreign")]
    );
    assert_eq!(
        tables[1]["payload"]["reason"],
        "the table's 50 × 100 grid is more than the 4000 cells its presentation has left"
    );
    // A declined table charges what it counts all the same.
    assert_eq!(
        tables[2]["payload"]["reason"],
        "the table's 40 × 100 grid is more than the 0 cells its presentation has left"
    );
}

#[test]
fn lowers_a_slide_alone_as_in_the_whole_presentation_charging_the_templates_and_slides_it_skips() {
    let mut p = Presentation::new();
    let mut templates = Vec::new();
    for (i, rows) in [20, 30].into_iter().enumerate() {
        let table = add_table(&mut p, &sized(rows));
        templates.push(p.template(&format!("T{i}"), vec![table], json!({})));
    }
    for (i, rows) in [20, 30].into_iter().enumerate() {
        let table = add_table(&mut p, &sized(rows));
        p.slide(
            json!({ "template_slide": templates[i], "drawables_z_order": [table] }),
            json!({}),
            None,
        );
    }
    let bytes = p.bytes();
    let limits = Limits {
        cells_per_presentation: 9000,
        ..Limits::default()
    };
    let whole = lower_with(&bytes, None, limits);
    let kinds: Vec<Value> = (0..2)
        .map(|i| table_of(whole.elements(i), 0)["kind"].clone())
        .collect();
    assert_eq!(kinds, [json!("table"), json!("foreign")]);
    assert_eq!(
        table_of(whole.elements(1), 0)["payload"]["reason"],
        "the table's 30 × 100 grid is more than the 2000 cells its presentation has left"
    );
    for slide in &whole.presentation.slides {
        let only = [slide.id.clone()];
        let alone = lower_with(&bytes, Some(&only), limits);
        assert_eq!(alone.presentation.slides, std::slice::from_ref(slide));
    }
}

#[test]
fn counts_only_the_tables_that_lower_against_their_containers_budget_not_one_its_cells_keep_foreign()
 {
    let mut p = Presentation::new();
    let mut row = tile_row(0, &[Some(text(1))], false);
    row["storage_version"] = json!(4);
    let tile = p.add("TST.Tile", json!({ "rowInfos": [row] }));
    let unreadable = TestTable {
        model: json!({ "number_of_rows": 100, "number_of_columns": 100,
                        "base_data_store": { "tiles": { "tiles": [{ "tileid": 0, "tile": tile }] } } }),
        ..grid(1, 1, numbered(1))
    };
    let a = add_table(&mut p, &unreadable);
    let b = add_table(&mut p, &grid(1, 2, numbered(2)));
    p.slide(json!({ "drawables_z_order": [a, b] }), json!({}), None);
    let lowered = lower(&p.bytes());
    let first = table_of(lowered.elements(0), 0);
    assert_eq!(first["kind"], "foreign");
    assert_eq!(first["payload"]["reason"], "cell storage version 4");
    assert_eq!(table_of(lowered.elements(0), 1)["kind"], "table");
}

#[test]
fn keeps_a_table_whose_cells_do_not_read_as_a_foreign() {
    let table = lower_table(|p| {
        let mut row = tile_row(0, &[Some(text(1))], false);
        row["storage_version"] = json!(4);
        let tile = p.add("TST.Tile", json!({ "rowInfos": [row] }));
        TestTable {
            model: json!({ "base_data_store": { "tiles": { "tiles": [{ "tileid": 0, "tile": tile }] } } }),
            ..grid(1, 1, numbered(1))
        }
    });
    assert_eq!(
        (&table.element["kind"], &table.element["type"]),
        (&json!("foreign"), &json!("table"))
    );
    assert_eq!(table.element["payload"]["reason"], "cell storage version 4");
}

#[test]
fn compiles_the_tables_fixture_with_every_loss_accounted_for() {
    let out = compile(&tables_presentation(), "tables.key");
    let bad: Vec<&str> = out
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                d.severity,
                slidock_ir::diagnostics::Severity::Error
                    | slidock_ir::diagnostics::Severity::Decline
            )
        })
        .map(|d| d.code.code())
        .collect();
    assert_eq!(bad, Vec::<&str>::new());
}

#[test]
fn draws_the_second_of_two_runs_of_one_order_on_a_shared_segment() {
    let table = lower_table(|_| {
        let mut table = grid(2, 1, numbered(1));
        table.strokes.bottom = vec![(
            0,
            vec![json!({ "origin": 0, "length": 1, "order": 1, "stroke": solid(1.0, None) })],
        )];
        table.strokes.top = vec![(
            1,
            vec![json!({ "origin": 0, "length": 1, "order": 1, "stroke": solid(2.0, None) })],
        )];
        table
    });
    let rows = table.rows();
    // The bottom of row 0 and the top of row 1 are one segment: the top run, written second, draws.
    assert_eq!(rows[0][0]["borders"]["bottom"]["width"], 2);
    assert_eq!(rows[1][0]["borders"]["top"]["width"], 2);
}
