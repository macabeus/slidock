//! Keynote presentations for the frontend's tests: synthetic ones built with the presentation builder, each exercising
//! one area so every object they hold is visible here.

use std::f64::consts::PI;

use serde_json::{Value, json};
use slidock_io::{RgbaImage, encode_png};

use super::Type;
use super::keynote::{Presentation, geometry, geometry_with, movie, rgb};
use super::tables::{
    CURRENCY, EMPTY, RICH_TEXT, Strokes, TestCell, TestMerge, TestTable, add_table, cell_style,
    number, rich_text, solid, text,
};

/// A 2 × 2 opaque grey PNG of level 200 or 30, as fflate's deflate writes it at level 6, whose bytes the goldens hash.
pub fn grey_png(r: u8) -> Vec<u8> {
    let hex = match r {
        200 => concat!(
            "89504e470d0a1a0a0000000d494844520000000200000002080600000072b60d240000001149444154789c633c71e2c47f",
            "0620803300490206b166467f660000000049454e44ae426082"
        ),
        30 => concat!(
            "89504e470d0a1a0a0000000d494844520000000200000002080600000072b60d240000001149444154789c63949393fbcf",
            "00047006001b3002b544430b3b0000000049454e44ae426082"
        ),
        _ => panic!("no grey PNG of level {r}"),
    };
    (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap())
        .collect()
}

/// A small opaque PNG, red and blue, with no stated resolution.
pub fn png(width: u32, height: u32) -> Vec<u8> {
    let mut data = Vec::new();
    for i in 0..width * height {
        data.extend_from_slice(if i % 2 == 1 {
            &[0, 0, 255, 255]
        } else {
            &[255, 0, 0, 255]
        });
    }
    encode_png(&RgbaImage {
        width,
        height,
        data,
    })
    .unwrap()
}

fn placeholder(
    presentation: &mut Presentation,
    kind: i32,
    at: Value,
    text: Option<Value>,
    style: Option<Value>,
) -> Value {
    let mut shape = json!({ "super": { "geometry": at } });
    if let Some(style) = style {
        shape["style"] = style;
    }
    let mut info = json!({ "super": shape });
    if let Some(text) = text {
        info["owned_storage"] = text;
    }
    presentation.add_in(
        "KN.PlaceholderArchive",
        json!({ "super": info, "kind": kind }),
        super::keynote::SLIDE,
    )
}

/// A title slide and a bullets slide on their templates, with notes, a dissolve, a skipped slide and a slide number.
pub fn basic_presentation() -> Vec<u8> {
    let mut p = Presentation::new();
    let title_style = p.paragraph_style(
        json!({ "font_name": "HelveticaNeue-Bold", "font_size": 96 }),
        None,
        json!({ "alignment": 2 }),
    );
    let body_style = p.paragraph_style(
        json!({ "font_name": "HelveticaNeue", "font_size": 40 }),
        None,
        json!({ "space_before": 12, "line_spacing": { "mode": 0, "amount": 1.1 } }),
    );
    let bullets = p.add(
        "TSWP.ListStyleArchive",
        json!({ "label_types": [2, 2], "strings": ["\u{2022}", "\u{2013}"], "indents": [0, 48], "text_indents": [1, 1] }),
    );
    let fill = p.add(
        "KN.SlideStyleArchive",
        json!({ "slide_properties": { "fill": { "color": rgb(1.0, 1.0, 1.0) } } }),
    );
    let bar_style = p.add(
        "TSD.ShapeStyleArchive",
        json!({ "shape_properties": { "fill": { "color": rgb(0.1, 0.3, 0.8) } } }),
    );
    let bar = p.add(
        "TSD.ShapeArchive",
        json!({ "super": { "geometry": geometry(0.0, 1040.0, 1920.0, 40.0) }, "style": bar_style }),
    );
    let prototype_text = p.storage("Prototype", None, json!({}));
    let prototype = p.text_box(prototype_text, geometry(100.0, 100.0, 400.0, 80.0), None);
    let title_text = p.storage("Title", Some(title_style.clone()), json!({}));
    let template_title = placeholder(
        &mut p,
        2,
        geometry(100.0, 80.0, 1720.0, 200.0),
        Some(title_text),
        None,
    );
    let body_text = p.storage("Body", Some(body_style.clone()), json!({}));
    let template_body = placeholder(
        &mut p,
        3,
        geometry(100.0, 320.0, 1720.0, 600.0),
        Some(body_text),
        None,
    );
    let number_attachment = p.add_record(Type::Name("KN.SlideNumberAttachmentArchive"), Vec::new());
    let number_text = p.storage(
        "\u{fffc}",
        None,
        json!({ "table_attachment": { "entries": [{ "character_index": 0, "object": number_attachment }] } }),
    );
    let template_number = placeholder(
        &mut p,
        1,
        geometry(1800.0, 1000.0, 80.0, 40.0),
        Some(number_text),
        None,
    );
    let template = p.template(
        "Title & Bullets",
        vec![
            bar,
            prototype.clone(),
            template_title.clone(),
            template_body.clone(),
            template_number.clone(),
        ],
        json!({
            "style": fill,
            "sage_tag_to_info_map": [{ "tag": "Text", "info": prototype }],
            "titlePlaceholder": template_title,
            "bodyPlaceholder": template_body,
            "slideNumberPlaceholder": template_number,
        }),
    );
    let note_text = p.storage("Opening notes\nSecond line", None, json!({}));
    let note = p.add("KN.NoteArchive", json!({ "containedStorage": note_text }));
    let opening_text = p.storage("Fixture Presentation", Some(title_style.clone()), json!({}));
    let opening = placeholder(
        &mut p,
        2,
        geometry(100.0, 80.0, 1720.0, 200.0),
        Some(opening_text),
        None,
    );
    p.slide(
        json!({ "template_slide": template, "style": fill, "note": note, "drawables_z_order": [opening] }),
        json!({}),
        None,
    );
    let body = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["First point\nNested point\nSecond point"],
            "table_para_style": { "entries": [{ "character_index": 0, "object": body_style }] },
            "table_list_style": { "entries": [{ "character_index": 0, "object": bullets }] },
            "table_para_data": { "entries": [
                { "character_index": 0, "first": 0 },
                { "character_index": 12, "first": 1 },
                { "character_index": 25, "first": 0 },
            ] },
        }),
    );
    let number_node = p.add_record(Type::Name("TSWP.NumberAttachmentArchive"), Vec::new());
    let slide_number_text = p.storage(
        "\u{fffc}",
        None,
        json!({ "table_attachment": { "entries": [{ "character_index": 0, "object": number_node }] } }),
    );
    let slide_number = placeholder(
        &mut p,
        1,
        geometry(1800.0, 1000.0, 80.0, 40.0),
        Some(slide_number_text),
        None,
    );
    let second_title_text = p.storage("Second Slide", Some(title_style), json!({}));
    let second_title = placeholder(
        &mut p,
        2,
        geometry(100.0, 80.0, 1720.0, 200.0),
        Some(second_title_text),
        None,
    );
    let second_body = placeholder(
        &mut p,
        3,
        geometry(100.0, 320.0, 1720.0, 600.0),
        Some(body),
        None,
    );
    p.slide(
        json!({
            "template_slide": template,
            "drawables_z_order": [second_title, second_body, slide_number],
            "transition": { "attributes": { "animationAttributes": { "effect": "apple:dissolve", "duration": 1.5 } } },
        }),
        json!({}),
        None,
    );
    let hidden_text = p.storage("Hidden text", None, json!({}));
    let hidden = p.text_box(hidden_text, geometry(100.0, 100.0, 800.0, 100.0), None);
    p.slide(
        json!({ "template_slide": template, "drawables_z_order": [hidden] }),
        json!({ "isSkipped": true }),
        None,
    );
    p.bytes()
}

/// Shapes, outlines, paint, line ends, a group, a rotated masked image, a movie with its build, and a table.
pub fn shapes_presentation() -> Vec<u8> {
    let mut p = Presentation::new();
    let red = p.add(
        "TSD.ShapeStyleArchive",
        json!({ "shape_properties": {
            "fill": { "color": rgb(1.0, 0.0, 0.0) },
            "stroke": { "color": rgb(0.0, 0.0, 0.0), "width": 4, "pattern": { "type": 1 } },
            "opacity": 0.5,
        } }),
    );
    let arrow_head = json!({
        "path": { "elements": [
            { "type": 1, "points": [{ "x": 0, "y": 0 }] },
            { "type": 2, "points": [{ "x": 3, "y": 6 }] },
            { "type": 2, "points": [{ "x": 6, "y": 0 }] },
            { "type": 5 },
        ] },
        "end_point": { "x": 3, "y": 0 },
        "is_filled": true,
    });
    let line_style = p.add(
        "TSD.ShapeStyleArchive",
        json!({ "shape_properties": {
            "stroke": { "color": rgb(0.0, 0.0, 1.0), "width": 2, "pattern": { "type": 0, "count": 2, "pattern": [3, 1, 0, 0] } },
            "head_line_end": arrow_head,
        } }),
    );
    let gradient = p.add(
        "TSD.ShapeStyleArchive",
        json!({ "shape_properties": {
            "fill": { "gradient": {
                "type": 0,
                "stops": [{ "fraction": 0, "color": rgb(1.0, 1.0, 0.0) }, { "fraction": 1, "color": rgb(0.0, 1.0, 1.0) }],
                "anglegradient": { "gradientangle": PI / 4.0 },
            } },
            "shadow": { "is_enabled": true, "color": rgb(0.0, 0.0, 0.0) },
        } }),
    );
    let rounded = p.add(
        "TSD.ShapeArchive",
        json!({
            "super": { "geometry": geometry(100.0, 100.0, 300.0, 200.0) },
            "style": red,
            "pathsource": { "scalar_path_source": { "type": 0, "scalar": 20 } },
        }),
    );
    let arrow = p.add(
        "TSD.ShapeArchive",
        json!({
            "super": { "geometry": geometry_with(500.0, 100.0, 300.0, 150.0, 3, Some(30.0)) },
            "style": gradient,
            "pathsource": { "point_path_source": {
                "type": 1, "point": { "x": 100, "y": 0.5 }, "naturalSize": { "width": 300, "height": 150 },
            } },
        }),
    );
    let line = p.add(
        "TSD.ShapeArchive",
        json!({
            "super": { "geometry": geometry(100.0, 400.0, 600.0, 0.0), "hyperlink_url": "https://example.com" },
            "style": line_style,
            "pathsource": { "bezier_path_source": { "path": { "elements": [
                { "type": 1, "points": [{ "x": 0, "y": 0 }] },
                { "type": 2, "points": [{ "x": 100, "y": 0 }] },
            ] } } },
        }),
    );
    let child = p.add(
        "TSD.ShapeArchive",
        json!({ "super": { "geometry": geometry(10.0, 10.0, 50.0, 50.0) }, "style": red }),
    );
    let group = p.add(
        "TSD.GroupArchive",
        json!({ "super": { "geometry": geometry(900.0, 100.0, 200.0, 200.0) }, "children": [child] }),
    );
    let mask = p.add(
        "TSD.MaskArchive",
        json!({
            "super": { "geometry": geometry(10.0, 10.0, 80.0, 60.0) },
            "pathsource": { "scalar_path_source": { "type": 1, "scalar": 6 } },
        }),
    );
    let photo = p.data("photo.png", grey_png(200));
    let image = p.add(
        "TSD.ImageArchive",
        json!({
            "super": { "geometry": geometry_with(1200.0, 100.0, 100.0, 80.0, 3, Some(350.0)),
                       "accessibility_description": "A photo" },
            "data": photo,
            "mask": mask,
        }),
    );
    let clip_data = p.data("clip.mp4", movie("vide", "avc1", true, None));
    let poster = p.data("poster.png", grey_png(30));
    let clip = p.add(
        "TSD.MovieArchive",
        json!({
            "super": { "geometry": geometry(100.0, 600.0, 640.0, 360.0) },
            "movieData": clip_data,
            "posterImageData": poster,
            "startTime": 1, "endTime": 4, "loop_option": 1, "volume": 0,
        }),
    );
    let model = p.add(
        "TST.TableModelArchive",
        json!({ "table_name": "Scores", "number_of_rows": 3, "number_of_columns": 2 }),
    );
    let table = p.add(
        "TST.TableInfoArchive",
        json!({ "super": { "geometry": geometry(900.0, 600.0, 600.0, 300.0) }, "tableModel": model }),
    );
    let start = p.add(
        "KN.BuildArchive",
        json!({ "drawable": clip, "attributes": { "animationAttributes": {
            "animation_type": "In", "effect": "apple:movie-start", "duration": 0.5,
        } } }),
    );
    let fade = p.add(
        "KN.BuildArchive",
        json!({ "drawable": arrow, "attributes": { "animationAttributes": {
            "animation_type": "Out", "effect": "apple:dissolve", "duration": 1,
        } } }),
    );
    let first_chunk = p.add(
        "KN.BuildChunkArchive",
        json!({ "build": start, "automatic": false }),
    );
    let second_chunk = p.add(
        "KN.BuildChunkArchive",
        json!({ "build": fade, "automatic": true, "delay": 0.25 }),
    );
    p.slide(
        json!({
            "drawables_z_order": [rounded, arrow, line, group, image, clip, table],
            "builds": [start, fade],
            "buildChunks": [first_chunk, second_chunk],
        }),
        json!({}),
        None,
    );
    p.bytes()
}

/// Text: runs and their styles, breaks, tabs, a link, numbered lists, and boxes that grow with their text.
pub fn text_presentation() -> Vec<u8> {
    let mut p = Presentation::new();
    let style = p.paragraph_style(
        json!({ "font_name": "CourierNewPSMT", "font_size": 30, "font_color": rgb(0.2, 0.2, 0.2) }),
        None,
        json!({ "line_spacing": { "mode": 0, "amount": 1.18 }, "default_tab_stops": 48 }),
    );
    let bold = p.add(
        "TSWP.CharacterStyleArchive",
        json!({ "char_properties": { "bold": true, "underline": 1, "background_color": rgb(1.0, 1.0, 0.0) } }),
    );
    let link = p.add(
        "TSWP.HyperlinkFieldArchive",
        json!({ "url_ref": "https://example.com/docs" }),
    );
    let numbered = p.add(
        "TSWP.ListStyleArchive",
        json!({ "label_types": [3, 3], "number_types": [0, 12], "indents": [0, 40], "text_indents": [1.5, 1.5] }),
    );
    let code = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["mov r0, r1\u{2028}\tadd r0, #1\nsee the docs\nfirst\nsub a\nsub b\nsecond"],
            "table_para_style": { "entries": [{ "character_index": 0, "object": style }] },
            "table_char_style": { "entries": [{ "character_index": 0, "object": bold }, { "character_index": 3 }] },
            "table_smartfield": { "entries": [{ "character_index": 31, "object": link }, { "character_index": 35 }] },
            "table_list_style": { "entries": [{ "character_index": 0 }, { "character_index": 36, "object": numbered }] },
            "table_para_data": { "entries": [
                { "character_index": 0, "first": 0 },
                { "character_index": 42, "first": 1 },
                { "character_index": 54, "first": 0 },
            ] },
        }),
    );
    let centred = p.paragraph_style(
        json!({ "font_name": "TrumpGothicEast-Bold", "font_size": 120 }),
        None,
        json!({ "alignment": 2 }),
    );
    let middle = p.add(
        "TSWP.ShapeStyleArchive",
        json!({ "shape_properties": { "vertical_alignment": 1, "padding_null": true } }),
    );
    let code_box = p.text_box(
        code,
        geometry_with(100.0, 100.0, 1200.0, 0.0, 1, None),
        None,
    );
    let title = p.storage("Title grows\nboth ways", Some(centred), json!({}));
    let title_box = p.text_box(
        title,
        geometry_with(960.0, 800.0, 0.0, 0.0, 0, None),
        Some(middle),
    );
    p.slide(
        json!({ "drawables_z_order": [code_box, title_box] }),
        json!({}),
        None,
    );
    p.bytes()
}

fn cell(cell_type: u8) -> TestCell {
    TestCell {
        cell_type,
        ..TestCell::default()
    }
}

/// Tables: header, footer and header-column regions with their styles, banded rows, a merged range written inline
/// and one written in a tile, per-edge strokes over the table style's (a shared segment's higher order drawing, an
/// empty stroke), cell styles, rich text, number formats, and a rotated table with wide offsets and no styles at all.
pub fn tables_presentation() -> Vec<u8> {
    let mut p = Presentation::new();
    let grey = rgb(0.6, 0.6, 0.6);
    let body = p.paragraph_style(
        json!({ "font_name": "HelveticaNeue", "font_size": 20 }),
        None,
        json!({ "alignment": 4 }),
    );
    let header = p.paragraph_style(
        json!({ "bold": true, "font_color": rgb(1.0, 1.0, 1.0) }),
        Some(body.clone()),
        json!({ "alignment": 2 }),
    );
    let body_cell = cell_style(
        &mut p,
        json!({ "padding": { "left": 4, "top": 4, "right": 4, "bottom": 4 }, "vertical_alignment": 1 }),
        None,
    );
    let header_cell = cell_style(
        &mut p,
        json!({ "cell_fill": { "color": rgb(0.2, 0.3, 0.6) } }),
        Some(body_cell.clone()),
    );
    let column_cell = cell_style(
        &mut p,
        json!({ "cell_fill": { "color": rgb(0.9, 0.9, 0.8) } }),
        Some(body_cell.clone()),
    );
    let tight = cell_style(
        &mut p,
        json!({ "padding": { "left": 2 }, "vertical_alignment": 2, "text_wrap": false }),
        Some(body_cell.clone()),
    );
    let gradient = cell_style(
        &mut p,
        json!({ "cell_fill": { "gradient": {
            "type": 0,
            "stops": [{ "fraction": 0, "color": rgb(1.0, 0.75, 0.0) }, { "fraction": 1, "color": rgb(0.5, 0.35, 0.0) }],
            "anglegradient": { "gradientangle": PI * 1.5 },
        } } }),
        Some(body_cell.clone()),
    );
    let bullets = p.add(
        "TSWP.ListStyleArchive",
        json!({ "label_types": [2], "strings": ["\u{2022}"], "text_indents": [1] }),
    );
    let notes = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["One\nTwo"],
            "table_para_style": { "entries": [{ "character_index": 0, "object": body }] },
            "table_list_style": { "entries": [{ "character_index": 0, "object": bullets }] },
        }),
    );
    let with_style = |mut cell: TestCell, style: u32| {
        cell.cell_style = Some(style);
        cell
    };
    let notes_payload = rich_text(&mut p, notes);
    let regions = TestTable {
        widths: vec![120.0, 100.0, 100.0],
        heights: vec![30.0; 5],
        model: json!({
            "number_of_header_rows": 1,
            "number_of_header_columns": 1,
            "number_of_footer_rows": 1,
            "body_cell_style": body_cell,
            "header_row_style": header_cell,
            "header_column_style": column_cell,
            "footer_row_style": body_cell,
            "body_text_style": body,
            "header_row_text_style": header,
            "header_column_text_style": body,
            "footer_row_text_style": header,
        }),
        table_style: json!({
            "banded_rows": true,
            "banded_fill": { "color": rgb(0.95, 0.95, 0.95) },
            "table_body_horizontal_stroke": solid(1.0, Some(grey.clone())),
            "table_body_vertical_stroke": solid(1.0, Some(grey.clone())),
            "table_body_horizontal_border_stroke": solid(2.0, None),
            "table_body_vertical_border_stroke": solid(2.0, None),
            "header_row_border_stroke": solid(2.0, None),
            "header_row_separator_stroke": solid(3.0, None),
            "header_row_vertical_stroke": solid(1.0, Some(rgb(1.0, 1.0, 1.0))),
            "header_column_border_stroke": solid(2.0, None),
            "header_column_separator_stroke": solid(2.0, Some(grey.clone())),
            "header_column_horizontal_stroke": solid(1.0, None),
            "footer_row_border_stroke": solid(2.0, None),
            "footer_row_separator_stroke": solid(2.0, None),
            "footer_row_vertical_stroke": solid(1.0, None),
        }),
        strings: ["Region", "Q1", "Q2", "North", "South", "Total", "East"]
            .map(str::to_owned)
            .to_vec(),
        styles: vec![tight, gradient],
        formats: vec![
            json!({ "format_type": 258, "decimal_places": 253 }),
            json!({ "format_type": 256, "decimal_places": 2, "show_thousands_separator": true }),
        ],
        rich_texts: vec![notes_payload],
        rows: vec![
            vec![Some(text(1)), Some(text(2)), Some(text(3))],
            vec![
                Some(text(4)),
                Some(number(25, -2, 1)),
                Some(number(12345, -1, 2)),
            ],
            vec![
                Some(with_style(text(5), 1)),
                Some(number(5, -1, 1)),
                Some(TestCell {
                    rich_text: Some(1),
                    ..cell(RICH_TEXT)
                }),
            ],
            vec![Some(text(7)), Some(with_style(cell(EMPTY), 2)), None],
            vec![Some(text(6)), None, Some(number(1, 6, 2))],
        ],
        merges: vec![
            TestMerge {
                row: 4,
                column: 0,
                columns: Some(2),
                ..TestMerge::default()
            },
            TestMerge {
                row: 2,
                column: 2,
                rows: Some(2),
                tiled: true,
                ..TestMerge::default()
            },
        ],
        strokes: Strokes {
            top: vec![(
                2,
                vec![json!({ "origin": 0, "length": 3, "order": 2, "stroke": {
                    "color": rgb(1.0, 0.0, 0.0), "width": 1.5, "pattern": { "type": 0, "count": 2, "pattern": [4, 3] },
                } })],
            )],
            bottom: vec![(
                1,
                vec![
                    json!({ "origin": 0, "length": 3, "order": 1, "stroke": solid(1.0, Some(rgb(0.0, 0.0, 1.0))) }),
                ],
            )],
            left: vec![(
                1,
                vec![
                    json!({ "origin": 1, "length": 1, "order": 1, "stroke": { "width": 1, "pattern": { "type": 2 } } }),
                ],
            )],
            ..Strokes::default()
        },
        ..TestTable::default()
    };
    let regions = add_table(&mut p, &regions);
    let plain = TestTable {
        widths: vec![200.0],
        heights: vec![40.0],
        strings: vec!["Wide offsets".to_owned()],
        rows: vec![vec![Some(text(1))]],
        wide: true,
        at: Some((600.0, 100.0, Some(10.0))),
        ..TestTable::default()
    };
    let plain = add_table(&mut p, &plain);
    p.slide(
        json!({ "drawables_z_order": [regions, plain] }),
        json!({}),
        None,
    );
    p.bytes()
}

/// A fixture's bytes.
pub type Fixture = fn() -> Vec<u8>;

/// The synthetic fixtures by golden name (`basic.key`, saved by Keynote 15.4, is a private golden fixture).
pub fn synthetic() -> Vec<(&'static str, Fixture)> {
    vec![
        ("synthetic", basic_presentation as Fixture),
        ("shapes", shapes_presentation),
        ("text", text_presentation),
        ("tables", tables_presentation),
    ]
}

#[allow(dead_code, reason = "the cell types every table test names")]
pub const CELL_TYPES: [u8; 3] = [CURRENCY, EMPTY, RICH_TEXT];
