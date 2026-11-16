//! One directed case per Keynote diagnostic the golden presentations leave out: each names its element and, where it
//! has one, the feature it loses.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use support::Type;
use support::fixtures::png;
use support::harness::lower;
use support::keynote::{Presentation, SLIDE, geometry, id_of, merge, rgb};

fn shape(p: &mut Presentation, extra: Value, style: Option<Value>) -> Value {
    let mut drawable = json!({ "geometry": geometry(10.0, 10.0, 100.0, 50.0) });
    if let Some(super_) = extra.get("super") {
        drawable = merge(drawable, super_.clone());
    }
    let mut shape = json!({ "super": drawable, "pathsource": { "scalar_path_source": { "type": 0, "scalar": 0 } } });
    if let Some(style) = style {
        shape["style"] = p.add(
            "TSD.ShapeStyleArchive",
            json!({ "shape_properties": style }),
        );
    }
    let mut rest = extra.as_object().cloned().unwrap_or_default();
    rest.remove("super");
    p.add_in("TSD.ShapeArchive", merge(shape, Value::Object(rest)), SLIDE)
}

fn text(p: &mut Presentation, content: &str, para: Value, char: Value, extra: Value) -> Value {
    let style = p.paragraph_style(char, None, para);
    let storage = p.storage(content, Some(style), extra);
    p.text_box(storage, geometry(10.0, 80.0, 200.0, 40.0), None)
}

#[test]
fn reports_each_code_on_its_drawable_with_the_feature_it_loses() {
    let mut p = Presentation::new();
    let image = p.data("fill.png", png(4, 2));
    let stops = json!([{ "fraction": 0, "color": rgb(1.0, 0.0, 0.0) }, { "fraction": 1, "color": rgb(0.0, 0.0, 1.0) }]);
    // Hiragana numbering (18), which the IR's auto-number schemes lack.
    let numbered = p.add(
        "TSWP.ListStyleArchive",
        json!({ "label_types": [3], "number_types": [18] }),
    );
    let vertical = p.add(
        "TSWP.ShapeStyleArchive",
        json!({ "shape_properties": { "vertical_text": true } }),
    );
    let callout = shape(
        &mut p,
        // A tail wider than its 4 pt body seen from its tip.
        json!({ "pathsource": { "callout_path_source": { "natural_size": { "width": 4, "height": 4 },
                "tail_position": { "x": 20, "y": 2 }, "tail_size": 5, "corner_radius": 0, "center_tail": false } } }),
        None,
    );
    let ftp_link = shape(
        &mut p,
        json!({ "super": { "hyperlink_url": "ftp://example.com/presentation" } }),
        None,
    );
    // A caption of a type the schema registers and leaves undecoded (TSA.CaptionInfoArchive).
    let caption_record = p.add_record(Type::Id(633), Vec::new());
    let caption = shape(
        &mut p,
        json!({ "super": { "caption": caption_record, "caption_hidden": false } }),
        None,
    );
    let tabs = text(
        &mut p,
        "a\tb",
        json!({ "tabs": { "tabs": [{ "position": 72 }] } }),
        json!({}),
        json!({}),
    );
    let up = p.storage("up", None, json!({}));
    let vertical = p.text_box(up, geometry(10.0, 130.0, 50.0, 200.0), Some(vertical));
    let outline = text(
        &mut p,
        "hollow",
        json!({}),
        json!({ "outline": 1 }),
        json!({}),
    );
    let numbered = text(
        &mut p,
        "one",
        json!({}),
        json!({}),
        json!({ "table_list_style": { "entries": [{ "character_index": 0, "object": numbered }] } }),
    );
    let spacing = text(
        &mut p,
        "lines",
        json!({ "line_spacing": { "mode": 4, "amount": 2 } }),
        json!({}),
        json!({}),
    );
    let gradient = shape(
        &mut p,
        json!({}),
        Some(json!({ "fill": { "gradient": { "type": 0, "stops": stops } } })),
    );
    let tinted = shape(
        &mut p,
        json!({}),
        Some(
            json!({ "fill": { "image": { "imagedata": image, "technique": 1, "tint": rgb(1.0, 0.0, 0.0) } } }),
        ),
    );
    let tiled = shape(
        &mut p,
        json!({}),
        Some(json!({ "fill": { "image": { "imagedata": image, "technique": 2 } } })),
    );
    let drawables = [
        ("KN1104", &callout, None),
        ("KN1108", &ftp_link, Some("element.link")),
        ("KN1109", &caption, None),
        ("KN1204", &tabs, Some("text.tab")),
        ("KN1206", &vertical, Some("text.vertical")),
        ("KN1207", &outline, None),
        ("KN1208", &numbered, Some("text.bullet.auto")),
        ("KN1209", &spacing, Some("text.line-spacing")),
        ("KN1301", &gradient, Some("fill.linear")),
        ("KN1302", &tinted, Some("image.recolor")),
        ("KN1305", &tiled, Some("fill.tile")),
    ];
    let refs: Vec<Value> = [
        &callout, &ftp_link, &caption, &tabs, &vertical, &outline, &numbered, &spacing, &gradient,
        &tinted, &tiled,
    ]
    .into_iter()
    .cloned()
    .collect();
    p.slide(json!({ "drawables_z_order": refs }), json!({}), None);
    let lowered = lower(&p.bytes());
    for (code, drawable, feature) in drawables {
        let node = format!("kn-{}", id_of(drawable));
        let at: Vec<(&str, Option<&str>)> = lowered
            .diagnostics
            .iter()
            .filter(|d| d.node.as_ref().map(|n| n.as_str()) == Some(node.as_str()))
            .map(|d| (d.code.code(), d.feature.map(|f| f.as_str())))
            .collect();
        assert!(at.contains(&(code, feature)), "{code} on {node}: {at:?}");
    }
}
