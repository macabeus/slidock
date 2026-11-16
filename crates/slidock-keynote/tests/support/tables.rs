//! Synthetic Keynote tables: cells packed the way Keynote packs them (storage version 5), and a table builder over
//! the presentation builder that writes the model, its lists, styles, strokes and merged ranges as Keynote does.

use serde_json::{Value, json};

use super::keynote::{Presentation, STYLESHEET, id_of, merge, rgb};

const CALCULATION_ENGINE: &str = "Index/CalculationEngine.iwa";
const DATA_LIST: &str = "Index/Tables/DataList.iwa";

/// One cell as a tile row stores it; the keys are list keys of the table.
#[derive(Clone, Debug, Default)]
pub struct TestCell {
    pub cell_type: u8,
    /// Digits, exponent, negative.
    pub decimal: Option<(u128, i32, bool)>,
    pub double: Option<f64>,
    pub seconds: Option<f64>,
    pub string: Option<u32>,
    pub rich_text: Option<u32>,
    pub cell_style: Option<u32>,
    pub text_style: Option<u32>,
    pub conditional_style: Option<u32>,
    pub number_format: Option<u32>,
    pub currency_format: Option<u32>,
    pub bool_format: Option<u32>,
}

pub const EMPTY: u8 = 0;
pub const NUMBER: u8 = 2;
pub const TEXT: u8 = 3;
pub const DATE: u8 = 5;
pub const BOOL: u8 = 6;
pub const DURATION: u8 = 7;
pub const ERROR: u8 = 8;
pub const RICH_TEXT: u8 = 9;
pub const CURRENCY: u8 = 10;

pub fn text(key: u32) -> TestCell {
    TestCell {
        cell_type: TEXT,
        string: Some(key),
        ..TestCell::default()
    }
}

pub fn number(digits: u128, exponent: i32, format: u32) -> TestCell {
    TestCell {
        cell_type: NUMBER,
        decimal: Some((digits, exponent, false)),
        number_format: Some(format),
        ..TestCell::default()
    }
}

/// IEEE 754 decimal128 in Numbers' layout.
pub fn decimal128(digits: u128, exponent: i32, negative: bool) -> [u8; 16] {
    let mut bytes = [0u8; 16];
    for (i, byte) in bytes.iter_mut().enumerate().take(14) {
        *byte = (digits >> (8 * i)) as u8;
    }
    let biased = exponent + 0x1820;
    bytes[14] = ((digits >> 112) & 1) as u8 | (((biased & 0x7f) as u8) << 1);
    bytes[15] = ((biased >> 7) & 0x7f) as u8 | if negative { 0x80 } else { 0 };
    bytes
}

/// One packed cell: its header, then the fields its flags name, in flag order.
pub fn pack_cell(cell: &TestCell) -> Vec<u8> {
    let mut flags = 0u32;
    let mut fields = Vec::new();
    if let Some((digits, exponent, negative)) = cell.decimal {
        flags |= 0x1;
        fields.extend(decimal128(digits, exponent, negative));
    }
    for (value, flag) in [(cell.double, 0x2), (cell.seconds, 0x4)] {
        if let Some(value) = value {
            flags |= flag;
            fields.extend(value.to_le_bytes());
        }
    }
    for (value, flag) in [
        (cell.string, 0x8),
        (cell.rich_text, 0x10),
        (cell.cell_style, 0x20),
        (cell.text_style, 0x40),
        (cell.conditional_style, 0x80),
        (cell.number_format, 0x2000),
        (cell.currency_format, 0x4000),
        (cell.bool_format, 0x40000),
    ] {
        if let Some(value) = value {
            flags |= flag;
            fields.extend(value.to_le_bytes());
        }
    }
    let mut out = vec![5, cell.cell_type, 0, 0, 0, 0, 0, 0];
    out.extend(flags.to_le_bytes());
    out.extend(fields);
    out
}

/// A tile row: the cells packed one after another, a uint16 offset per column (in 4-byte units when wide).
pub fn tile_row(index: usize, cells: &[Option<TestCell>], wide: bool) -> Value {
    let packed: Vec<Option<Vec<u8>>> = cells
        .iter()
        .map(|cell| cell.as_ref().map(pack_cell))
        .collect();
    let mut buffer: Vec<u8> = Vec::new();
    let mut offsets = vec![0xffu8; 2 * 256];
    for (column, bytes) in packed.iter().enumerate() {
        let Some(bytes) = bytes else { continue };
        let at = buffer.len();
        let offset = if wide { at / 4 } else { at } as u16;
        offsets[2 * column..2 * column + 2].copy_from_slice(&offset.to_le_bytes());
        buffer.extend(bytes);
    }
    let mut row = json!({
        "tile_row_index": index,
        "cell_count": packed.iter().filter(|cell| cell.is_some()).count(),
        "storage_version": 5,
        "cell_storage_buffer": buffer,
        "cell_offsets": offsets,
    });
    if wide {
        row["has_wide_offsets"] = json!(true);
    }
    row
}

pub fn solid(width: f64, color: Option<Value>) -> Value {
    json!({ "color": color.unwrap_or_else(|| rgb(0.0, 0.0, 0.0)), "width": width, "pattern": { "type": 1 } })
}

/// A merged range: inline as the calculation engine's back dependency (inclusive), or as a tile's origin and size.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestMerge {
    pub row: u32,
    pub column: u32,
    pub rows: Option<u32>,
    pub columns: Option<u32>,
    pub tiled: bool,
}

/// Sidecar layers: per side, the line index and its runs.
#[derive(Clone, Debug, Default)]
pub struct Strokes {
    pub left: Vec<(u32, Vec<Value>)>,
    pub right: Vec<(u32, Vec<Value>)>,
    pub top: Vec<(u32, Vec<Value>)>,
    pub bottom: Vec<(u32, Vec<Value>)>,
}

#[derive(Clone, Debug, Default)]
pub struct TestTable {
    pub rows: Vec<Vec<Option<TestCell>>>,
    pub widths: Vec<f64>,
    pub heights: Vec<f64>,
    pub model: Value,
    pub strings: Vec<String>,
    /// Style list entries by key, in order from 1.
    pub styles: Vec<Value>,
    pub formats: Vec<Value>,
    pub rich_texts: Vec<Value>,
    pub table_style: Value,
    pub merges: Vec<TestMerge>,
    pub strokes: Strokes,
    pub hidden_rows: Vec<usize>,
    pub hidden_columns: Vec<usize>,
    pub wide: bool,
    /// x, y and an angle.
    pub at: Option<(f64, f64, Option<f64>)>,
}

const STRING_LIST: u32 = 1;
const FORMAT_LIST: u32 = 2;
const STYLE_LIST: u32 = 4;
const RICH_TEXT_PAYLOAD_LIST: u32 = 8;

/// A table drawable and everything its model names; returns the TST.TableInfoArchive.
pub fn add_table(presentation: &mut Presentation, table: &TestTable) -> Value {
    let add = |presentation: &mut Presentation, name: &'static str, message: Value| {
        presentation.add_in(name, message, CALCULATION_ENGINE)
    };
    let list = |presentation: &mut Presentation, list_type: u32, entries: Vec<Value>| {
        let next = entries.len() + 1;
        presentation.add_in(
            "TST.TableDataList",
            json!({ "listType": list_type, "nextListID": next, "entries": entries }),
            DATA_LIST,
        )
    };
    let keyed = |values: Vec<Value>| -> Vec<Value> {
        values
            .into_iter()
            .enumerate()
            .map(|(i, entry)| merge(json!({ "key": i + 1, "refcount": 1 }), entry))
            .collect()
    };
    let headers = |presentation: &mut Presentation, sizes: &[f64], hidden: &[usize]| {
        let headers: Vec<Value> = sizes
            .iter()
            .enumerate()
            .map(|(index, size)| {
                json!({ "index": index, "size": size, "hidingState": u8::from(hidden.contains(&index)),
                        "numberOfCells": 1 })
            })
            .collect();
        presentation.add_in(
            "TST.HeaderStorageBucket",
            json!({ "bucketHashFunction": 1, "headers": headers }),
            CALCULATION_ENGINE,
        )
    };
    let rows: Vec<Value> = table
        .rows
        .iter()
        .enumerate()
        .map(|(index, cells)| tile_row(index, cells, table.wide))
        .collect();
    let tile = add(
        presentation,
        "TST.Tile",
        json!({ "numrows": table.rows.len(), "rowInfos": rows }),
    );
    // The tile's id is unique in the presentation, so it names the table's merge owner too.
    let owner = id_of(&tile);
    let owner_id = json!({ "uuid_w0": owner, "uuid_w1": 7, "uuid_w2": 11, "uuid_w3": 13 });
    let back: Vec<Value> = table
        .merges
        .iter()
        .filter(|merge| !merge.tiled)
        .enumerate()
        .map(|(i, merge)| {
            let (rows, columns) = (merge.rows.unwrap_or(1), merge.columns.unwrap_or(1));
            json!({
                "cell_coord_row": 0, "cell_coord_column": i,
                "internal_range_reference": { "owner_id": 1, "range": {
                    "top_left_column": merge.column, "top_left_row": merge.row,
                    "bottom_right_column": merge.column + columns - 1, "bottom_right_row": merge.row + rows - 1,
                } },
            })
        })
        .collect();
    let tiled: Vec<&TestMerge> = table.merges.iter().filter(|merge| merge.tiled).collect();
    let precedents = if tiled.is_empty() {
        Vec::new()
    } else {
        let ranges: Vec<Value> = tiled
            .iter()
            .enumerate()
            .map(|(i, merge)| {
                let mut size = json!({});
                if let Some(columns) = merge.columns {
                    size["num_columns"] = json!(columns);
                }
                if let Some(rows) = merge.rows {
                    size["num_rows"] = json!(rows);
                }
                json!({
                    "from_coord": { "column": i, "row": 0 },
                    "refers_to_rect": { "origin": { "column": merge.column, "row": merge.row }, "size": size },
                })
            })
            .collect();
        vec![add(
            presentation,
            "TSCE.RangePrecedentsTileArchive",
            json!({ "to_owner_id": 1, "from_to_range": ranges }),
        )]
    };
    add(
        presentation,
        "TSCE.FormulaOwnerDependenciesArchive",
        json!({
            "formula_owner_uid": { "lower": (owner | (7 << 32)).to_string(), "upper": (11u64 | (13 << 32)).to_string() },
            "internal_formula_owner_id": owner,
            "owner_kind": 5,
            "range_dependencies": { "back_dependency": back },
            "tiled_range_dependencies": { "range_precedents_tile": precedents },
        }),
    );
    let layers = |presentation: &mut Presentation, side: &[(u32, Vec<Value>)]| -> Vec<Value> {
        side.iter()
            .map(|(index, runs)| {
                add(
                    presentation,
                    "TST.StrokeLayerArchive",
                    json!({ "row_column_index": index, "stroke_runs": runs }),
                )
            })
            .collect()
    };
    let left = layers(presentation, &table.strokes.left);
    let right = layers(presentation, &table.strokes.right);
    let top = layers(presentation, &table.strokes.top);
    let bottom = layers(presentation, &table.strokes.bottom);
    let sidecar = add(
        presentation,
        "TST.StrokeSidecarArchive",
        json!({
            "max_order": 2, "column_count": table.widths.len(), "row_count": table.heights.len(),
            "left_column_stroke_layers": left, "right_column_stroke_layers": right,
            "top_row_stroke_layers": top, "bottom_row_stroke_layers": bottom,
        }),
    );
    let table_style = presentation.add_in(
        "TST.TableStyleArchive",
        json!({ "super": {}, "table_properties": if table.table_style.is_null() { json!({}) } else { table.table_style.clone() } }),
        STYLESHEET,
    );
    let row_headers = headers(presentation, &table.heights, &table.hidden_rows);
    let column_headers = headers(presentation, &table.widths, &table.hidden_columns);
    let strings = keyed(
        table
            .strings
            .iter()
            .map(|s| json!({ "string": s }))
            .collect(),
    );
    let string_table = list(presentation, STRING_LIST, strings);
    let styles = keyed(
        table
            .styles
            .iter()
            .map(|r| json!({ "reference": r }))
            .collect(),
    );
    let style_table = list(presentation, STYLE_LIST, styles);
    let formats = keyed(
        table
            .formats
            .iter()
            .map(|f| json!({ "format": f }))
            .collect(),
    );
    let format_table = list(presentation, FORMAT_LIST, formats);
    let payloads = keyed(
        table
            .rich_texts
            .iter()
            .map(|p| json!({ "rich_text_payload": p }))
            .collect(),
    );
    let rich_text_table = list(presentation, RICH_TEXT_PAYLOAD_LIST, payloads);
    let model = merge(
        json!({
            "table_id": format!("T{owner}"),
            "table_name": "Table 1",
            "number_of_rows": table.heights.len(),
            "number_of_columns": table.widths.len(),
            "default_row_height": 20,
            "default_column_width": 100,
            "table_style": table_style,
            "base_data_store": {
                "rowHeaders": { "bucketHashFunction": 1, "buckets": [row_headers] },
                "columnHeaders": column_headers,
                "tiles": { "tiles": [{ "tileid": 0, "tile": tile }], "tile_size": 256 },
                "stringTable": string_table,
                "styleTable": style_table,
                "format_table": format_table,
                "rich_text_table": rich_text_table,
                "rowTileTree": { "nodes": [{ "key": 0, "value": 0 }] },
                "columnTileTree": {},
            },
            "merge_owner": { "owner_id": owner_id },
            "stroke_sidecar": sidecar,
        }),
        table.model.clone(),
    );
    let model = add(presentation, "TST.TableModelArchive", model);
    let width: f64 = table.widths.iter().sum();
    let height: f64 = table.heights.iter().sum();
    let (x, y, angle) = table.at.unwrap_or((100.0, 100.0, None));
    let mut geometry = json!({ "position": { "x": x, "y": y }, "size": { "width": width, "height": height }, "flags": 3 });
    if let Some(angle) = angle.filter(|angle| *angle != 0.0) {
        geometry["angle"] = json!(angle);
    }
    add(
        presentation,
        "TST.TableInfoArchive",
        json!({ "super": { "geometry": geometry }, "tableModel": model }),
    )
}

/// A cell style: its properties over `parent`.
pub fn cell_style(
    presentation: &mut Presentation,
    properties: Value,
    parent: Option<Value>,
) -> Value {
    let parent = parent.map_or_else(|| json!({}), |parent| json!({ "parent": parent }));
    presentation.add_in(
        "TST.CellStyleArchive",
        json!({ "super": parent, "cell_properties": properties }),
        STYLESHEET,
    )
}

/// A rich text cell's payload around a storage.
pub fn rich_text(presentation: &mut Presentation, storage: Value) -> Value {
    presentation.add_in(
        "TST.RichTextPayloadArchive",
        json!({ "storage": storage }),
        CALCULATION_ENGINE,
    )
}
