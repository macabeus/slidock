//! Tables: the dense grid (spans, covered slots, repeats, completed rows), track sizes, cell styles over table
//! templates, borders, the cell's text box, and the tables that stay Foreign.
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
use slidock_ir::ids::SlideId;
use support::odp::{OdpSpec, PageSpec, odp};
use support::{Lowered, assert_holds, by_name, fixtures, js, lower, lower_with};

fn tables() -> Lowered {
    lower(&fixtures::tables())
}

fn grouped(lowered: &Lowered) -> &Value {
    lowered.named(2, "Grouped")
}

fn clash(lowered: &Lowered) -> &Value {
    by_name(grouped(lowered)["children"].as_array().unwrap(), "Clash")
}

/// A cell's first run, or "" for an empty cell.
fn text_of(cell: &Value) -> String {
    let inline = &cell["text"]["paragraphs"][0]["inlines"][0];
    if inline["kind"] == "run" {
        inline["text"].as_str().unwrap().to_owned()
    } else {
        String::new()
    }
}

/// The grid as text and spans, `None` where a span covers the slot.
fn layout(table: &Value) -> Vec<Vec<Option<String>>> {
    table["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row["cells"]
                .as_array()
                .unwrap()
                .iter()
                .map(|cell| {
                    (!cell.is_null()).then(|| {
                        let span = |key: &str| cell.get(key).and_then(Value::as_u64).unwrap_or(1);
                        format!("{} {} {}", text_of(cell), span("colSpan"), span("rowSpan"))
                    })
                })
                .collect()
        })
        .collect()
}

fn grid(rows: &[&[Option<&str>]]) -> Vec<Vec<Option<String>>> {
    rows.iter()
        .map(|row| row.iter().map(|cell| cell.map(str::to_owned)).collect())
        .collect()
}

fn fill_of(cell: &Value) -> String {
    let fill = &cell["fill"];
    match fill["kind"].as_str() {
        Some("solid") => ["r", "g", "b"]
            .iter()
            .map(|c| {
                format!(
                    "{:02x}",
                    slidock_canon::js_round(fill["color"][c].as_f64().unwrap() * 255.0) as u32
                )
            })
            .collect(),
        Some(kind) => kind.to_owned(),
        None => "-".to_owned(),
    }
}

fn fills(row: &Value) -> Vec<String> {
    row["cells"]
        .as_array()
        .unwrap()
        .iter()
        .map(fill_of)
        .collect()
}

#[test]
fn lays_out_the_grid_spans_covered_slots_repeated_rows_and_cells_completed_rows_and_clipped_spans()
{
    let lowered = tables();
    assert_eq!(
        layout(lowered.named(0, "Merges")),
        grid(&[
            &[Some("Header 4 1"), None, None, None],
            &[Some("Tall 1 2"), Some("Block 2 2"), None, Some("Side 1 1")],
            &[None, None, None, Some("Under 1 1")],
            &[Some("Rep 1 1"), Some("Rep 1 1"), Some(" 1 1"), Some(" 1 1")],
            &[Some("Rep 1 1"), Some("Rep 1 1"), Some(" 1 1"), Some(" 1 1")],
            // A covered cell no span reaches is a cell; a span stops at the grid's edge.
            &[Some("Orphan 1 1"), Some("Wide 3 1"), None, None],
            &[Some("Down 1 1"), Some(" 1 1"), Some(" 1 1"), Some(" 1 1")],
        ])
    );
    // A span stops at a slot an earlier span covers, and a cell written at a covered slot is hidden by it.
    assert_eq!(
        layout(clash(&lowered)),
        grid(&[
            &[Some("a 1 1"), Some("b 1 1"), Some("A 1 2")],
            &[Some("B 2 1"), None, None]
        ])
    );
}

#[test]
fn sizes_tracks_as_their_styles_state_sharing_what_the_frame_leaves_and_boxes_the_frame_on_the_grid()
 {
    let lowered = tables();
    let merges = lowered.named(0, "Merges");
    let widths: Vec<&Value> = merges["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &c["width"])
        .collect();
    assert_eq!(widths, [&json!(72), &json!(72), &json!(72), &json!(144)]);
    let heights: Vec<&Value> = merges["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| &r["height"])
        .collect();
    assert_eq!(
        heights,
        [
            &json!(36),
            &json!(36),
            &json!(36),
            &json!(18),
            &json!(18),
            &json!(36),
            &json!(36)
        ]
    );
    assert_holds(
        merges,
        json!({ "frame": { "x": 72, "y": 72, "w": 360, "h": 216 } }),
    );
    // Relative widths share the frame's width.
    let styled = lowered.named(1, "Styled");
    let widths: Vec<&Value> = styled["columns"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| &c["width"])
        .collect();
    assert_eq!(widths, [&json!(72), &json!(216), &json!(72)]);
    assert_holds(
        clash(&lowered),
        json!({ "frame": { "x": 0, "y": 0, "w": 216, "h": 72 } }),
    );
    assert_holds(
        grouped(&lowered),
        json!({ "frame": { "x": 72, "y": 144, "w": 360, "h": 72 } }),
    );
}

#[test]
fn styles_each_cell_its_own_style_else_its_rows_or_columns_default_over_its_template_position() {
    let lowered = tables();
    let merges = lowered.named(0, "Merges");
    assert_eq!(fills(&merges["rows"][1]), ["abcdef", "abcdef", "-", "-"]);
    assert_eq!(
        merges["rows"][0]["cells"][0]["borders"]["top"]["width"],
        js(&json!(0.75))
    );
    // First row over first column, first column over banding, odd-row banding, and the body for even rows.
    let styled = lowered.named(1, "Styled");
    let rows: Vec<Vec<String>> = styled["rows"]
        .as_array()
        .unwrap()
        .iter()
        .map(fills)
        .collect();
    assert_eq!(
        rows,
        [
            ["003366", "003366", "003366"],
            ["ffcc00", "dddddd", "00ff00"],
            ["ffcc00", "-", "003366"],
            ["ff0000", "odf", "-"],
        ]
    );
    assert_eq!(
        styled["rows"][2]["cells"][1]["borders"]["left"]["width"],
        js(&json!(0.5))
    );
    // A shorthand replaces the sides it stands for: the cell's parent style over the template's thicker top.
    let tops: Vec<Value> = styled["rows"][2]["cells"]
        .as_array()
        .unwrap()
        .iter()
        .map(|cell| cell["borders"]["top"]["width"].clone())
        .collect();
    assert_eq!(tops, [Value::Null, json!(3), json!(1)]);
    // A template named as ODF 1.2 names it.
    assert_eq!(fills(&lowered.named(1, "Older")["rows"][0]), ["dddddd"]);
}

#[test]
fn reads_borders_from_cell_and_paragraph_properties_sides_over_shorthands_dashes_and_diagonals() {
    let lowered = tables();
    let styled = lowered.named(1, "Styled");
    let cells = &styled["rows"][3]["cells"];
    assert_holds(
        &cells[0]["borders"],
        json!({
            "top": { "width": 0.5, "dash": "solid" },
            "left": { "width": 2, "dash": "dash", "fill": { "kind": "solid", "color": { "r": 0, "g": 0, "b": 1 } } }
        }),
    );
    let ce2: Vec<&String> = cells[1]["borders"].as_object().unwrap().keys().collect();
    assert_eq!(ce2.len(), 3);
    assert_holds(
        &cells[1]["borders"],
        json!({
            "top": { "width": 1, "dash": "sysDot" },
            "right": { "width": 1, "dash": "dashDot" },
            "bottom": { "width": 0.75, "dash": "sysDash" }
        }),
    );
    assert_holds(
        &cells[2]["borders"],
        json!({
            "top": { "width": 1.5, "dash": "solid" },
            "diagonalDown": { "width": 1, "dash": "solid" },
            "diagonalUp": { "width": 1, "dash": "sysDashDotDot" }
        }),
    );
    // A hidden side over an inherited shorthand draws no line.
    let mut sides: Vec<&String> = styled["rows"][2]["cells"][2]["borders"]
        .as_object()
        .unwrap()
        .keys()
        .collect();
    sides.sort();
    assert_eq!(sides, ["left", "right", "top"]);
    assert!(
        lowered.named(0, "Merges")["rows"][1]["cells"][0]
            .get("borders")
            .is_none()
    );
    let approximated: Vec<Value> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "OD9113")
        .map(|d| js(&d.data))
        .collect();
    assert_eq!(approximated, [json!({ "style": "double" })]);
}

#[test]
fn reads_the_cells_text_box_padding_vertical_alignment_its_styles_text_and_an_empty_cell() {
    let lowered = tables();
    let cells = &lowered.named(1, "Styled")["rows"][3]["cells"];
    assert_eq!(
        cells[0]["text"]["body"],
        js(&json!({
            "insets": { "l": 7.2, "t": 7.2, "r": 7.2, "b": 7.2 },
            "anchor": "middle",
            "wrap": "square",
            "autofit": { "kind": "none" }
        }))
    );
    assert_holds(
        &cells[1]["text"]["body"],
        json!({ "anchor": "bottom", "insets": { "l": 3.6 } }),
    );
    assert_holds(
        &cells[2]["text"]["body"],
        json!({ "anchor": "top", "insets": { "l": 7.087, "t": 3.543 } }),
    );
    // The cell style is the list style; the paragraph and span styles state their own.
    assert_holds(
        &cells[0]["text"]["listStyle"]["0"],
        json!({ "para": { "align": "center" }, "run": { "size": 12 } }),
    );
    let aligns: Vec<Value> = cells[0]["text"]["paragraphs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["props"].get("align").cloned().unwrap_or_default())
        .collect();
    assert_eq!(aligns, [json!("right"), Value::Null]);
    let inlines = &cells[0]["text"]["paragraphs"][0]["inlines"];
    assert_eq!(
        inlines[0],
        json!({ "kind": "run", "text": "Plain ", "props": {} })
    );
    assert_holds(
        &inlines[1],
        json!({ "kind": "run", "text": "bold red", "props": { "weight": 700 } }),
    );
    // A cell with no paragraph keeps one, empty, sized by its style.
    assert_eq!(
        cells[1]["text"]["paragraphs"],
        json!([{ "level": 0, "props": {}, "inlines": [] }])
    );
    assert_holds(
        &cells[1]["text"]["listStyle"]["0"]["run"],
        json!({ "size": 18 }),
    );
}

#[test]
fn keeps_a_table_the_ir_cannot_hold_foreign_with_its_replacement_image_and_reports_a_missing_template()
 {
    let lowered = tables();
    let huge = lowered.named(2, "Huge");
    assert_holds(huge, json!({ "kind": "foreign", "type": "table" }));
    assert!(
        huge["payload"]["reason"]
            .as_str()
            .unwrap()
            .contains("more than 1000 rows or columns")
    );
    assert!(huge["fallback"]["media"].is_string());
    assert_holds(
        lowered.named(2, "Hollow"),
        json!({ "kind": "foreign", "type": "table", "payload": { "reason": "the table holds no cell" } }),
    );
    let missing = lowered.named(1, "Missing");
    assert_eq!(fills(&missing["rows"][0]), ["-"]);
    let reported: Vec<Value> = lowered
        .diagnostics
        .iter()
        .filter(|d| d.code.code() == "OD9112")
        .map(|d| json!([d.node.as_ref().unwrap().as_str(), js(&d.data)]))
        .collect();
    assert_eq!(reported, [json!([missing["id"], { "name": "Gone" }])]);
    // A lowered table's replacement image is not read.
    let media: Vec<&String> = lowered.json["media"].as_object().unwrap().keys().collect();
    assert_eq!(media, [huge["fallback"]["media"].as_str().unwrap()]);
}

#[test]
fn records_the_features_its_tables_hold() {
    let lowered = tables();
    let ids: Vec<String> = lowered
        .presentation
        .slides
        .iter()
        .map(|s| s.id.as_str().to_owned())
        .collect();
    let has = |slide: usize, wanted: &[&str]| {
        let features = lowered.features(&ids[slide]);
        for feature in wanted {
            assert!(features.contains(feature), "{} lacks {feature}", ids[slide]);
        }
    };
    has(0, &["element.table", "fill.solid", "stroke", "text.run"]);
    has(
        1,
        &["element.table", "fill.linear", "stroke.dash", "text.weight"],
    );
    has(2, &["element.table", "element.group"]);
}

fn square(name: &str, n: u32, column: &str) -> String {
    format!(
        concat!(
            r#"<draw:frame draw:name="{name}" draw:style-name="standard" svg:x="0in" svg:y="0in" svg:width="1in" "#,
            r#"svg:height="1in"><table:table><table:table-column{column} table:number-columns-repeated="{n}"/>"#,
            r#"<table:table-row table:number-rows-repeated="{n}"><table:table-cell table:number-columns-repeated="{n}"/>"#,
            "</table:table-row></table:table></draw:frame>"
        ),
        name = name,
        n = n,
        column = column
    )
}

fn kinds(elements: &[Value]) -> Vec<(String, String)> {
    elements
        .iter()
        .map(|e| {
            (
                e["name"].as_str().unwrap().to_owned(),
                e["kind"].as_str().unwrap().to_owned(),
            )
        })
        .collect()
}

#[test]
fn counts_every_tables_repeated_cells_against_its_containers_budget_before_it_expands_them() {
    let pages = vec![
        PageSpec::body(format!(
            "{}{}",
            square("First", 100, ""),
            square("Second", 100, "")
        )),
        PageSpec::body(square("Again", 100, "")),
        PageSpec::body(square("Far", 2, r#" table:style-name="far""#)),
    ];
    let bytes = odp(&OdpSpec {
        automatic_styles: concat!(
            r#"<style:style style:name="far" style:family="table-column">"#,
            r#"<style:table-column-properties style:column-width="1000in"/></style:style>"#
        )
        .to_owned(),
        ..OdpSpec::pages(pages)
    });
    let lowered = lower(&bytes);
    let pair = |name: &str, kind: &str| (name.to_owned(), kind.to_owned());
    assert_eq!(
        kinds(lowered.elements(0)),
        [pair("First", "table"), pair("Second", "foreign")]
    );
    assert!(
        lowered.named(0, "Second")["payload"]["reason"]
            .as_str()
            .unwrap()
            .contains("more than the 0 cells its container has left")
    );
    let far = lowered.named(2, "Far");
    assert_eq!(far["kind"], "foreign");
    assert!(
        far["payload"]["reason"]
            .as_str()
            .unwrap()
            .contains("past the IR's coordinate range")
    );
    assert_eq!(kinds(lowered.elements(1)), [pair("Again", "table")]);
}

#[test]
fn counts_every_containers_repeated_cells_against_one_presentation_budget_before_it_expands_them() {
    let pages: Vec<PageSpec> = ["First", "Second", "Third"]
        .iter()
        .map(|name| PageSpec::body(square(name, 100, "")))
        .collect();
    let bytes = odp(&OdpSpec::pages(pages));
    let limits = Limits {
        cells_per_presentation: 25_000,
        ..Limits::default()
    };
    let lowered = lower_with(&bytes, None, limits);
    let all: Vec<Value> = (0..3).flat_map(|i| lowered.elements(i).clone()).collect();
    let pair = |name: &str, kind: &str| (name.to_owned(), kind.to_owned());
    assert_eq!(
        kinds(&all),
        [
            pair("First", "table"),
            pair("Second", "table"),
            pair("Third", "foreign")
        ]
    );
    assert!(
        by_name(&all, "Third")["payload"]["reason"]
            .as_str()
            .unwrap()
            .contains("more than the 5000 cells its presentation has left")
    );
    // A slide lowered alone lowers as in the whole presentation.
    let third = SlideId::new(lowered.presentation.slides[2].id.as_str()).unwrap();
    let alone = lower_with(&bytes, Some(&[third]), limits);
    assert_eq!(alone.presentation.slides.len(), 1);
    assert_eq!(alone.elements(0)[0]["kind"], "foreign");
}
