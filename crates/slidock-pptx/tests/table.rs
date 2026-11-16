//! Tables: the grid and its frame, merges, the table style's regions and their precedence, a cell's own properties
//! over its style's, backgrounds, cell text and margins, the diagnostics of what the IR leaves out, and the limits.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_core_api::Limits;
use slidock_ir::features::FeatureId;
use support::fixtures::tables;
use support::{
    LayoutSpec, Lowered, MasterSpec, PptxSpec, Row, SlideSpec, Sp, TableSpec, assert_matches,
    compile_bytes, element, json, lower, lower_with, pptx, slide, sp, table, tc1, xfrm,
};

fn table_of(lowered: &Lowered, id: &str) -> Value {
    let found = element(&lowered.presentation().slides[0], id);
    assert_eq!(found["kind"], "table", "{id}");
    found
}

fn cell(table: &Value, row: usize, column: usize) -> Value {
    let cell = table["rows"][row]["cells"][column].clone();
    assert!(!cell.is_null(), "no cell at {row}, {column}");
    cell
}

fn theme(slot: &str) -> Value {
    json!({ "kind": "theme", "slot": slot, "mods": [] })
}

fn solid_of(fill: &Value) -> Value {
    if fill["kind"] == "solid" {
        fill["color"].clone()
    } else {
        fill.clone()
    }
}

fn edge(border: &Value) -> Value {
    if border.is_null() {
        return Value::Null;
    }
    let color = if border["fill"]["kind"] == "solid" {
        border["fill"]["color"].clone()
    } else {
        json!("none")
    };
    json!({ "width": border["width"], "color": color, "dash": border["dash"] })
}

fn text_of(cell: &Value) -> Vec<String> {
    cell["text"]["paragraphs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| {
            p["inlines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|inline| inline["text"].as_str().unwrap_or("").to_owned())
                .collect()
        })
        .collect()
}

#[test]
fn takes_the_frames_position_and_the_grids_size_not_the_written_size() {
    let lowered = lower(&tables());
    let styled = table_of(&lowered, "pp-256.10");
    assert_matches(
        &styled["frame"],
        json!({ "x": 20, "y": 20, "w": 400, "h": 150 }),
    );
    assert_eq!(
        styled["columns"],
        json!([{ "width": 100 }, { "width": 100 }, { "width": 100 }, { "width": 100 }])
    );
    let heights: Vec<Value> = styled["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["height"].clone())
        .collect();
    assert_eq!(
        heights,
        [json!(30), json!(30), json!(30), json!(30), json!(30)]
    );
}

#[test]
fn writes_null_for_every_slot_a_span_covers_whatever_h_merge_and_v_merge_say() {
    let lowered = lower(&tables());
    let styled = table_of(&lowered, "pp-256.10");
    let shape: Vec<Vec<String>> = styled["rows"]
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
                        "-".to_owned()
                    } else {
                        format!(
                            "{}x{}",
                            cell["colSpan"].as_u64().unwrap_or(1),
                            cell["rowSpan"].as_u64().unwrap_or(1)
                        )
                    }
                })
                .collect()
        })
        .collect();
    assert_eq!(
        shape,
        [
            ["1x1", "2x1", "-", "1x1"],
            ["1x2", "1x1", "1x1", "1x1"],
            ["-", "2x2", "-", "1x1"],
            ["1x1", "-", "-", "1x1"],
            ["1x1", "1x1", "1x1", "1x1"],
        ]
    );
    // Spans past the grid, a row short of cells and a span over a covered slot.
    let malformed = table_of(&lowered, "pp-256.13");
    let texts: Vec<Vec<Option<String>>> = malformed["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row["cells"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| (!cell.is_null()).then(|| text_of(cell)[0].clone()))
                .collect()
        })
        .collect();
    let some = |text: &str| Some(text.to_owned());
    assert_eq!(
        texts,
        [
            vec![some("Wide"), None, None],
            vec![some("B"), some("Deep"), some("")],
            vec![some("Clipped"), None, some("")],
        ]
    );
    assert_eq!(cell(&malformed, 0, 0)["colSpan"], 3);
    assert_eq!(cell(&malformed, 1, 1)["rowSpan"], 2);
    assert!(cell(&malformed, 2, 0).get("colSpan").is_none());
}

#[test]
fn fills_each_cell_from_the_last_style_region_that_sets_a_fill() {
    let lowered = lower(&tables());
    let styled = table_of(&lowered, "pp-256.10");
    let fill = |row, column| solid_of(&cell(&styled, row, column)["fill"]);
    // The corner cell over the first row over the first column over the whole table.
    assert_eq!(fill(0, 0), theme("accent6"));
    assert_eq!(fill(0, 1), theme("accent1"));
    // Bands count from the row after the header: band1H, band2H (no fill: the whole table's), band1H.
    assert_eq!(
        fill(1, 1),
        json!({ "kind": "theme", "slot": "accent1", "mods": [{ "op": "tint", "value": 0.4 }] })
    );
    assert_eq!(fill(2, 1), theme("lt1"));
    assert_eq!(fill(3, 3), fill(1, 1));
    // The last row's `noFill` shows the table background, a fill style of the theme; seCell needs lastCol.
    assert_eq!(fill(4, 1), theme("lt2"));
    assert_eq!(fill(4, 3), theme("lt2"));
}

#[test]
fn gives_an_edge_on_its_region_boundary_the_outer_line_and_between_its_cells_the_inside_line() {
    let lowered = lower(&tables());
    let styled = table_of(&lowered, "pp-256.10");
    let corner = cell(&styled, 0, 0)["borders"].clone();
    let accent1 = json!({ "width": 1, "color": theme("accent1"), "dash": "solid" });
    assert_eq!(edge(&corner["top"]), accent1);
    assert_eq!(edge(&corner["left"]), accent1);
    assert_matches(
        &edge(&corner["right"]),
        json!({ "width": 0.5, "dash": "dash" }),
    );
    // The first row's bottom edge.
    assert_eq!(
        edge(&corner["bottom"]),
        json!({ "width": 2, "color": theme("dk1"), "dash": "solid" })
    );
    // An inside edge by reference to the theme's first line style, in the reference's colour.
    let body = cell(&styled, 1, 1)["borders"].clone();
    let inside = json!({ "width": 0.5, "color": theme("accent2"), "dash": "solid" });
    assert_eq!(edge(&body["top"]), inside);
    assert_eq!(edge(&body["bottom"]), inside);
    // A cell spanning two band rows takes the whole table's inside edge at its bottom.
    assert_eq!(edge(&cell(&styled, 1, 0)["borders"]["bottom"]), inside);
    assert_eq!(edge(&cell(&styled, 1, 3)["borders"]["right"]), accent1);
    // The last row's top edge.
    assert_eq!(
        edge(&cell(&styled, 4, 1)["borders"]["top"]),
        json!({ "width": 3, "color": theme("accent1"), "dash": "solid" })
    );
}

#[test]
fn takes_a_cells_own_fill_and_edges_over_its_styles_an_own_edge_field_by_field() {
    let lowered = lower(&tables());
    let styled = table_of(&lowered, "pp-256.10");
    let own = cell(&styled, 1, 2);
    assert_matches(
        &solid_of(&own["fill"]),
        json!({ "kind": "rgb", "r": 1, "g": 0, "b": 0 }),
    );
    // `a:lnR` with `a:noFill` removes the style's right edge.
    assert!(own["borders"].get("right").is_none());
    // `a:lnT w="6350"` keeps the style's colour at its own width.
    assert_eq!(
        edge(&cell(&styled, 4, 2)["borders"]["top"]),
        json!({ "width": 0.5, "color": theme("accent1"), "dash": "solid" })
    );
}

#[test]
fn puts_the_styles_text_properties_beneath_each_level_of_the_cells_list_style() {
    let lowered = lower(&tables());
    let styled = table_of(&lowered, "pp-256.10");
    let run = |row, column| cell(&styled, row, column)["text"]["listStyle"]["0"]["run"].clone();
    assert_eq!(
        run(0, 0),
        json!({ "font": { "kind": "family", "family": "Georgia" }, "color": theme("lt1"), "weight": 700 })
    );
    assert_eq!(
        run(1, 0),
        json!({ "font": { "kind": "theme", "slot": "minor" }, "color": theme("dk1"), "weight": 700 })
    );
    assert_eq!(
        run(1, 1),
        json!({ "font": { "kind": "theme", "slot": "minor" }, "color": theme("dk1") })
    );
    assert_matches(&run(4, 0), json!({ "weight": 700, "italic": true }));
    let list = cell(&styled, 1, 1)["text"]["listStyle"].clone();
    assert_eq!(list["8"], list["0"]);
}

#[test]
fn reads_the_margins_and_anchor_into_the_text_body_an_empty_cell_holds_one_empty_paragraph() {
    let lowered = lower(&tables());
    let own = table_of(&lowered, "pp-256.11");
    assert_eq!(
        cell(&own, 0, 0)["text"]["body"],
        json!({ "insets": { "l": 0, "r": 2, "t": 1, "b": 0 }, "anchor": "bottom" })
    );
    assert_eq!(
        cell(&own, 0, 1)["text"]["body"],
        json!({ "anchor": "middle" })
    );
    assert_eq!(
        cell(&own, 0, 2)["text"],
        json!({ "body": {}, "paragraphs": [{ "level": 0, "props": {}, "inlines": [] }] })
    );
    assert_eq!(text_of(&cell(&own, 0, 0)), ["Margins", ""]);
}

#[test]
fn reads_both_diagonals_and_gradient_and_pattern_fills_a_line_end_on_an_edge_is_not_drawn() {
    let lowered = lower(&tables());
    let own = table_of(&lowered, "pp-256.11");
    let diagonals = cell(&own, 0, 1)["borders"].clone();
    assert_matches(
        &edge(&diagonals["diagonalDown"]),
        json!({ "width": 1, "dash": "sysDot" }),
    );
    assert_matches(
        &edge(&diagonals["diagonalUp"]),
        json!({ "width": 1, "dash": { "custom": [3, 1] } }),
    );
    assert_eq!(cell(&own, 1, 0)["fill"]["kind"], "linear");
    let left = cell(&own, 1, 0)["borders"]["left"].clone();
    assert_matches(
        &left,
        json!({ "width": 2, "fill": { "kind": "solid", "color": { "kind": "rgb" } }, "dash": "solid", "cap": "flat", "join": "round" }),
    );
    assert_eq!(left.as_object().unwrap().len(), 5);
    assert_matches(
        &solid_of(&cell(&own, 1, 1)["fill"]),
        json!({ "kind": "rgb", "g": 1 }),
    );
    assert_eq!(
        lowered.codes("pp-256.11"),
        ["PX2101", "PX2103", "PX2202", "PX2203"]
    );
    let effect = lowered
        .diagnostics
        .iter()
        .find(|d| {
            d.code.code() == "PX2101" && d.node.as_ref().is_some_and(|n| n.as_str() == "pp-256.11")
        })
        .unwrap();
    assert_eq!(effect.feature, Some(FeatureId::Effect3d));
}

#[test]
fn fills_cells_without_a_fill_with_the_background_and_says_where_it_is_not_solid() {
    let lowered = lower(&tables());
    let background = table_of(&lowered, "pp-256.12");
    assert_eq!(solid_of(&cell(&background, 0, 0)["fill"])["kind"], "rgb");
    assert_eq!(cell(&background, 0, 1)["fill"]["kind"], "linear");
    assert_matches(
        &json(lowered.find("PX2108").unwrap()),
        json!({ "node": "pp-256.12", "data": { "fill": "linear" } }),
    );
    // A solid background is the same in each cell.
    let solid = table_of(&lowered, "pp-256.21");
    assert_eq!(
        solid_of(&cell(&solid, 0, 1)["fill"]),
        solid_of(&cell(&solid, 0, 0)["fill"])
    );
    assert!(lowered.codes("pp-256.21").is_empty());
}

#[test]
fn reports_an_undefined_style_a_right_to_left_table_and_effects() {
    let lowered = lower(&tables());
    assert_eq!(
        lowered.codes("pp-256.12"),
        ["PX2011", "PX2012", "PX2108", "PX2111"]
    );
    assert_eq!(
        json(&lowered.find("PX2011").unwrap().data),
        json!({ "style": "{5C22544A-7EE6-4342-B048-85BDC9FD1C3A}" })
    );
}

#[test]
fn records_the_table_and_what_its_cells_draw_among_the_features_seen() {
    let lowered = lower(&tables());
    let features = &lowered.lowered.features["pp-256"];
    for feature in [
        FeatureId::ElementTable,
        FeatureId::FillSolid,
        FeatureId::FillLinear,
        FeatureId::Stroke,
        FeatureId::StrokeDash,
    ] {
        assert!(features.contains(&feature), "{feature:?}");
    }
}

/// One slide of tables under a table style; the style's parts are its inner XML.
fn styled_tables(parts: &str, tables: &[(&str, usize, usize)]) -> (Lowered, Vec<Value>) {
    let tree: String = tables
        .iter()
        .enumerate()
        .map(|(index, (flags, columns, rows))| {
            table(TableSpec {
                id: 10 + index as u64,
                frame: xfrm(0.0, index as f64 * 100.0, 100.0, 50.0, ""),
                tbl_pr: Some(format!(
                    "<a:tblPr {flags}><a:tableStyleId>{{S}}</a:tableStyleId></a:tblPr>"
                )),
                columns: vec![20.0; *columns],
                rows: (0..*rows)
                    .map(|_| Row {
                        height: 20.0,
                        cells: (0..*columns).map(|_| tc1("x")).collect(),
                    })
                    .collect(),
                ph: String::new(),
            })
        })
        .collect();
    let lowered = lower(&pptx(PptxSpec {
        table_styles: Some(format!(
            "<a:tblStyle styleId=\"{{S}}\" styleName=\"S\">{parts}</a:tblStyle>"
        )),
        slides: vec![slide(tree)],
        ..PptxSpec::default()
    }));
    let found = (0..tables.len())
        .map(|index| table_of(&lowered, &format!("pp-256.{}", 10 + index)))
        .collect();
    (lowered, found)
}

fn part(name: &str, hex: &str) -> String {
    format!(
        "<a:{name}><a:tcStyle><a:fill><a:solidFill><a:srgbClr val=\"{hex}\"/></a:solidFill></a:fill></a:tcStyle></a:{name}>"
    )
}

fn hex_of(cell: &Value) -> Option<String> {
    let color = solid_of(&cell["fill"]);
    if color["kind"] != "rgb" {
        return None;
    }
    Some(
        ["r", "g", "b"]
            .iter()
            .map(|c| {
                format!(
                    "{:02x}",
                    slidock_canon::js_round(color[*c].as_f64().unwrap() * 255.0) as u8
                )
            })
            .collect(),
    )
}

fn parts() -> String {
    [
        part("wholeTbl", "000000"),
        part("band1H", "100000"),
        part("band2H", "200000"),
        part("band1V", "000010"),
        part("band2V", "000020"),
        part("firstCol", "00aa00"),
        part("lastCol", "00bb00"),
        part("firstRow", "aa0000"),
        part("lastRow", "bb0000"),
        part("nwCell", "a0a0a0"),
        part("neCell", "b0b0b0"),
        part("swCell", "c0c0c0"),
        part("seCell", "d0d0d0"),
    ]
    .join("")
}

fn hexes(row: &Value) -> Vec<String> {
    row["cells"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cell| hex_of(cell).unwrap())
        .collect()
}

#[test]
fn bands_columns_after_the_first_column_the_first_and_last_columns_over_the_bands() {
    let (_, tables) = styled_tables(
        &parts(),
        &[("firstCol=\"1\" lastCol=\"1\" bandCol=\"1\"", 5, 1)],
    );
    assert_eq!(
        hexes(&tables[0]["rows"][0]),
        ["00aa00", "000010", "000020", "000010", "00bb00"]
    );
}

#[test]
fn bands_rows_after_the_header_the_first_and_last_rows_over_the_columns_the_corners_over_all() {
    let (_, tables) = styled_tables(
        &parts(),
        &[(
            "firstRow=\"1\" lastRow=\"1\" firstCol=\"1\" lastCol=\"1\" bandRow=\"1\"",
            3,
            5,
        )],
    );
    let rows: Vec<Vec<String>> = tables[0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(hexes)
        .collect();
    assert_eq!(
        rows,
        [
            ["a0a0a0", "aa0000", "b0b0b0"],
            ["00aa00", "100000", "00bb00"],
            ["00aa00", "200000", "00bb00"],
            ["00aa00", "100000", "00bb00"],
            ["c0c0c0", "bb0000", "d0d0d0"],
        ]
    );
}

#[test]
fn applies_no_region_whose_flag_is_off_and_the_first_row_and_column_over_the_last_in_a_single_track()
 {
    let (_, tables) = styled_tables(
        &parts(),
        &[
            ("", 2, 2),
            ("firstCol=\"1\" lastCol=\"1\"", 1, 1),
            ("firstRow=\"1\" lastRow=\"1\"", 2, 1),
        ],
    );
    let all: Vec<String> = tables[0]["rows"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(hexes)
        .collect();
    assert_eq!(all, ["000000", "000000", "000000", "000000"]);
    assert_eq!(hexes(&tables[1]["rows"][0]), ["00aa00"]);
    assert_eq!(hexes(&tables[2]["rows"][0]), ["aa0000", "aa0000"]);
}

#[test]
fn keeps_a_cells_own_list_style_over_the_styles_text_properties() {
    let tx_body = concat!(
        "<a:txBody><a:bodyPr/><a:lstStyle><a:lvl1pPr><a:defRPr b=\"0\"/></a:lvl1pPr></a:lstStyle>",
        "<a:p><a:r><a:rPr lang=\"en-US\"/><a:t>Own</a:t></a:r></a:p></a:txBody>"
    );
    let tree = table(TableSpec {
        id: 10,
        frame: xfrm(0.0, 0.0, 100.0, 20.0, ""),
        tbl_pr: Some(
            "<a:tblPr firstRow=\"1\"><a:tableStyleId>{S}</a:tableStyleId></a:tblPr>".to_owned(),
        ),
        columns: vec![50.0, 50.0],
        rows: vec![Row {
            height: 20.0,
            cells: vec![format!("<a:tc>{tx_body}<a:tcPr/></a:tc>"), tc1("Styled")],
        }],
        ph: String::new(),
    });
    let lowered = lower(&pptx(PptxSpec {
        table_styles: Some(
            "<a:tblStyle styleId=\"{S}\" styleName=\"S\"><a:firstRow><a:tcTxStyle b=\"on\" i=\"on\"/></a:firstRow></a:tblStyle>"
                .to_owned(),
        ),
        slides: vec![slide(tree)],
        ..PptxSpec::default()
    }));
    let t = table_of(&lowered, "pp-256.10");
    assert_eq!(
        cell(&t, 0, 0)["text"]["listStyle"]["0"]["run"],
        json!({ "weight": 400, "italic": true })
    );
    assert_eq!(
        cell(&t, 0, 1)["text"]["listStyle"]["0"]["run"],
        json!({ "weight": 700, "italic": true })
    );
}

#[test]
fn reads_a_style_written_inline_in_the_table_properties() {
    let tree = table(TableSpec {
        id: 10,
        frame: xfrm(0.0, 0.0, 20.0, 20.0, ""),
        tbl_pr: Some(format!(
            "<a:tblPr><a:tableStyle styleId=\"{{I}}\" styleName=\"I\">{}</a:tableStyle></a:tblPr>",
            part("wholeTbl", "123456")
        )),
        columns: vec![20.0],
        rows: vec![Row {
            height: 20.0,
            cells: vec![tc1("x")],
        }],
        ph: String::new(),
    });
    let lowered = lower(&pptx(PptxSpec {
        slides: vec![slide(tree)],
        ..PptxSpec::default()
    }));
    let t = table_of(&lowered, "pp-256.10");
    assert_eq!(hex_of(&cell(&t, 0, 0)).as_deref(), Some("123456"));
    assert!(lowered.find("PX2011").is_none());
}

#[test]
fn takes_its_layout_placeholders_position_and_its_own_grids_size_when_it_has_no_position() {
    let ph = "<p:ph type=\"tbl\" idx=\"1\"/>";
    let layout = sp(Sp {
        id: 3,
        name: Some("Table Placeholder".to_owned()),
        ph: ph.to_owned(),
        sp_pr: xfrm(100.0, 120.0, 300.0, 200.0, ""),
        ..Sp::default()
    });
    let rows = vec![Row {
        height: 20.0,
        cells: vec![tc1("a"), tc1("b")],
    }];
    let bytes = pptx(PptxSpec {
        masters: Some(vec![MasterSpec {
            layouts: vec![LayoutSpec {
                tree: layout,
                ..LayoutSpec::default()
            }],
            ..MasterSpec::default()
        }]),
        slides: vec![slide(table(TableSpec {
            id: 10,
            frame: "<a:xfrm/>".to_owned(),
            tbl_pr: None,
            columns: vec![100.0, 100.0],
            rows,
            ph: ph.to_owned(),
        }))],
        ..PptxSpec::default()
    });
    let lowered = lower(&bytes);
    assert!(
        json(&lowered.presentation().slides[0].elements[0])
            .get("frame")
            .is_none()
    );
    let compiled = compile_bytes(&bytes, "presentation.pptx");
    let resolved = json(&compiled.presentation.as_ref().unwrap());
    assert_matches(
        &resolved["slides"][0]["elements"][0],
        json!({ "kind": "table", "frame": { "x": 100, "y": 120, "w": 200, "h": 20 } }),
    );
    assert!(compiled.errors().is_empty(), "{:?}", compiled.errors());
}

fn grid(id: u64, columns: usize, rows: usize, track: f64) -> String {
    table(TableSpec {
        id,
        frame: xfrm(0.0, 0.0, 100.0, 100.0, ""),
        tbl_pr: None,
        columns: vec![track; columns],
        rows: (0..rows)
            .map(|_| Row {
                height: track,
                cells: Vec::new(),
            })
            .collect(),
        ph: String::new(),
    })
}

const TITLE: &str = "<p:sp><p:nvSpPr><p:cNvPr id=\"2\" name=\"Title\"/><p:cNvSpPr/><p:nvPr/></p:nvSpPr><p:spPr/></p:sp>";

fn kinds(slide: &Value) -> Vec<String> {
    slide["elements"]
        .as_array()
        .unwrap()
        .iter()
        .map(|element| element["kind"].as_str().unwrap().to_owned())
        .collect()
}

fn reason(element: &Value) -> String {
    element["payload"]["reason"]
        .as_str()
        .unwrap_or("")
        .to_owned()
}

#[test]
fn keeps_a_table_over_its_limits_foreign_and_the_rest_of_its_slide() {
    for (tree, why) in [
        (grid(10, 1001, 1, 1.0), "more than 1000 rows or columns"),
        (
            grid(10, 200, 51, 1.0),
            "more than the 10000 cells its container has left",
        ),
        (grid(10, 3, 2, 90_000.0), "past the IR's coordinate range"),
    ] {
        let lowered = lower(&pptx(PptxSpec {
            slides: vec![slide(format!("{TITLE}{tree}"))],
            ..PptxSpec::default()
        }));
        let slide = json(&lowered.presentation().slides[0]);
        assert!(slide.get("failed").is_none());
        assert_eq!(kinds(&slide), ["shape", "foreign"]);
        assert!(reason(&slide["elements"][1]).contains(why), "{why}");
    }
}

#[test]
fn counts_every_tables_cells_against_its_containers_budget() {
    let tree = grid(10, 100, 60, 1.0) + &grid(11, 100, 40, 1.0) + &grid(12, 1, 1, 1.0);
    let lowered = lower(&pptx(PptxSpec {
        slides: vec![slide(tree), slide(grid(10, 100, 60, 1.0))],
        ..PptxSpec::default()
    }));
    let slides = json(&lowered.presentation().slides);
    assert_eq!(kinds(&slides[0]), ["table", "table", "foreign"]);
    assert!(
        reason(&slides[0]["elements"][2]).contains("more than the 0 cells its container has left")
    );
    assert_eq!(kinds(&slides[1]), ["table"]);
}

#[test]
fn counts_every_containers_tables_against_one_presentation_budget() {
    let limits = Limits {
        cells_per_presentation: 10_000,
        ..Limits::default()
    };
    let bytes = pptx(PptxSpec {
        slides: vec![
            slide(grid(10, 100, 60, 1.0)),
            slide(grid(10, 100, 50, 1.0)),
            slide(grid(10, 100, 40, 1.0)),
        ],
        ..PptxSpec::default()
    });
    let lowered = lower_with(&bytes, None, &limits).unwrap();
    let slides = json(&lowered.presentation().slides);
    let all: Vec<Vec<String>> = slides.as_array().unwrap().iter().map(kinds).collect();
    assert_eq!(all, [vec!["table"], vec!["foreign"], vec!["foreign"]]);
    assert!(
        reason(&slides[1]["elements"][0])
            .contains("more than the 4000 cells its presentation has left")
    );
    // A declined table charges what it counts all the same.
    assert!(
        reason(&slides[2]["elements"][0])
            .contains("more than the 0 cells its presentation has left")
    );
}

#[test]
fn lowers_a_slide_alone_as_in_the_whole_presentation_charging_what_it_skips() {
    let masters = vec![
        MasterSpec {
            tree: grid(10, 100, 20, 1.0),
            layouts: vec![LayoutSpec {
                tree: grid(11, 100, 20, 1.0),
                ..LayoutSpec::default()
            }],
            ..MasterSpec::default()
        },
        MasterSpec {
            tree: grid(10, 100, 10, 1.0),
            layouts: vec![LayoutSpec {
                tree: grid(11, 100, 20, 1.0),
                ..LayoutSpec::default()
            }],
            ..MasterSpec::default()
        },
    ];
    let slides = vec![
        SlideSpec {
            layout: Some(0),
            tree: grid(13, 100, 20, 1.0),
            ..SlideSpec::default()
        },
        SlideSpec {
            layout: Some(1),
            tree: grid(14, 100, 30, 1.0),
            ..SlideSpec::default()
        },
    ];
    let bytes = pptx(PptxSpec {
        masters: Some(masters),
        slides,
        ..PptxSpec::default()
    });
    let limits = Limits {
        cells_per_presentation: 10_000,
        ..Limits::default()
    };
    let whole = lower_with(&bytes, None, &limits).unwrap();
    let slides = json(&whole.presentation().slides);
    let all: Vec<Vec<String>> = slides.as_array().unwrap().iter().map(kinds).collect();
    assert_eq!(all, [vec!["table"], vec!["foreign"]]);
    assert!(
        reason(&slides[1]["elements"][0])
            .contains("more than the 1000 cells its presentation has left")
    );
    for slide in &whole.presentation().slides {
        let only = [slide.id.clone()];
        let alone = lower_with(&bytes, Some(&only), &limits).unwrap();
        assert_eq!(alone.presentation().slides, std::slice::from_ref(slide));
    }
}
