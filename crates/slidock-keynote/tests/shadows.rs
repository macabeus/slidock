//! Shadows: a drawable's style shadow on its shape, text shape, picture or movie, a character style's on its runs, and
//! what is not drawn (contact and curved shadows, incomplete ones, bullet shadows), reported with the reason.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use serde_json::{Value, json};
use slidock_ir::resolved::{Body, Element};
use support::fixtures::png;
use support::harness::{compile, lower};
use support::keynote::{Presentation, SLIDE, geometry, movie, rgb};

/// A drop shadow as Keynote 14.5 writes one: every field set.
fn drop(angle: f64, offset: f64, radius: i32, opacity: f64) -> Value {
    json!({ "color": rgb(0.0, 0.0, 0.0), "angle": angle, "offset": offset, "radius": radius, "opacity": opacity,
            "is_enabled": true, "type": 0 })
}

/// The retro deck's text shadow: 3.287 pt down, radius 14, 53.31 %.
fn retro() -> Value {
    drop(90.0, 3.287, 14, 0.5331)
}

fn retro_ir() -> Value {
    json!({ "dx": 0, "dy": 3.287, "sigma": 7.14,
            "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 0.5331 } })
}

fn shape(p: &mut Presentation, shadow: Value) -> Value {
    let style = p.add(
        "TSD.ShapeStyleArchive",
        json!({ "shape_properties": { "fill": { "color": rgb(1.0, 1.0, 1.0) }, "shadow": shadow } }),
    );
    p.add_in(
        "TSD.ShapeArchive",
        json!({ "super": { "geometry": geometry(10.0, 10.0, 100.0, 50.0) }, "style": style,
                "pathsource": { "scalar_path_source": { "type": 0, "scalar": 0 } } }),
        SLIDE,
    )
}

fn messages(lowered: &support::harness::Lowered) -> Vec<(&str, &str)> {
    lowered
        .diagnostics
        .iter()
        .map(|d| (d.code.code(), d.message.as_str()))
        .collect()
}

#[test]
fn lowers_the_style_shadow_of_shapes_text_shapes_pictures_and_movies() {
    let mut p = Presentation::new();
    let photo = p.data("photo.png", png(4, 2));
    let clip = p.data("clip.mp4", movie("vide", "avc1", true, None));
    let poster = p.data("poster.png", png(2, 2));
    let filled = shape(&mut p, drop(128.0505, 9.5422, 5, 1.0));
    let text_style = p.add(
        "TSWP.ShapeStyleArchive",
        json!({ "super": { "shape_properties": { "shadow": retro() } } }),
    );
    let storage = p.storage("Shadow", None, json!({}));
    let text_box = p.text_box(storage, geometry(0.0, 0.0, 200.0, 40.0), Some(text_style));
    let media_style = p.add(
        "TSD.MediaStyleArchive",
        json!({ "media_properties": { "shadow": drop(60.0, 3.7839, 25, 0.5) } }),
    );
    let image = p.add(
        "TSD.ImageArchive",
        json!({ "super": { "geometry": geometry(0.0, 0.0, 80.0, 40.0) }, "data": photo, "style": media_style }),
    );
    let film = p.add(
        "TSD.MovieArchive",
        json!({ "super": { "geometry": geometry(0.0, 0.0, 80.0, 40.0) }, "movieData": clip,
                "posterImageData": poster, "style": media_style }),
    );
    p.slide(
        json!({ "drawables_z_order": [filled, text_box, image, film] }),
        json!({}),
        None,
    );
    let lowered = lower(&p.bytes());
    let shadows: Vec<&Value> = lowered.elements(0).iter().map(|e| &e["shadow"]).collect();
    let media = json!({ "dx": 1.892, "dy": 3.277, "sigma": 12.75,
                        "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 0.5 } });
    assert_eq!(
        shadows,
        [
            &json!({ "dx": -5.881, "dy": 7.514, "sigma": 2.55,
                     "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } }),
            &retro_ir(),
            &media,
            &media,
        ]
    );
    assert!(messages(&lowered).is_empty(), "{:?}", messages(&lowered));
    let slide = lowered.json["slides"][0]["id"].as_str().unwrap();
    assert!(lowered.features(slide).contains(&"effect.shadow"));
}

#[test]
fn draws_nothing_for_an_empty_or_disabled_shadow_and_reports_what_it_does_not_draw() {
    let mut p = Presentation::new();
    let mut disabled = retro();
    disabled["is_enabled"] = json!(false);
    let mut contact = retro();
    contact["type"] = json!(1);
    let mut curved = retro();
    curved["type"] = json!(2);
    let mut colorless = retro();
    colorless.as_object_mut().unwrap().remove("color");
    let drawables = vec![
        shape(&mut p, json!({})),
        shape(&mut p, disabled),
        shape(&mut p, contact),
        shape(&mut p, curved),
        shape(&mut p, colorless),
    ];
    p.slide(json!({ "drawables_z_order": drawables }), json!({}), None);
    let lowered = lower(&p.bytes());
    assert!(
        lowered
            .elements(0)
            .iter()
            .all(|e| e.get("shadow").is_none())
    );
    assert_eq!(
        messages(&lowered),
        [
            ("KN1304", "A shadow is not drawn: contact shadow"),
            ("KN1304", "A shadow is not drawn: curved shadow"),
            ("KN1304", "A shadow is not drawn: shadow without its color"),
        ]
    );
    assert!(
        lowered
            .diagnostics
            .iter()
            .all(|d| d.feature.map(|f| f.as_str()) == Some("effect.shadow"))
    );
}

#[test]
fn a_character_style_shadows_its_runs_and_a_null_shadow_ends_the_chain() {
    let mut p = Presentation::new();
    let body = p.paragraph_style(json!({ "shadow": retro() }), None, json!({}));
    let plain = p.add(
        "TSWP.CharacterStyleArchive",
        json!({ "char_properties": { "shadow_null": true } }),
    );
    let mut contact = retro();
    contact["type"] = json!(1);
    let touching = p.add(
        "TSWP.CharacterStyleArchive",
        json!({ "char_properties": { "shadow": contact } }),
    );
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["shadow plain touch"],
            "table_para_style": { "entries": [{ "character_index": 0, "object": body }] },
            "table_char_style": { "entries": [
                { "character_index": 0 }, { "character_index": 7, "object": plain },
                { "character_index": 13, "object": touching },
            ] },
        }),
    );
    let text_box = p.text_box(storage, geometry(0.0, 0.0, 400.0, 40.0), None);
    p.slide(json!({ "drawables_z_order": [text_box] }), json!({}), None);
    let lowered = lower(&p.bytes());
    let element = &lowered.elements(0)[0];
    assert!(element.get("shadow").is_none());
    let runs: Vec<(&Value, &Value)> = element["text"]["paragraphs"][0]["inlines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|run| (&run["text"], &run["props"]["shadow"]))
        .collect();
    assert_eq!(
        runs,
        [
            (&json!("shadow "), &retro_ir()),
            (&json!("plain "), &Value::Null),
            (&json!("touch"), &Value::Null),
        ]
    );
    assert_eq!(
        messages(&lowered),
        [("KN1304", "A shadow is not drawn: contact shadow")]
    );
    assert_eq!(
        lowered.diagnostics[0].feature.map(|f| f.as_str()),
        Some("text.shadow")
    );
    let slide = lowered.json["slides"][0]["id"].as_str().unwrap();
    assert!(lowered.features(slide).contains(&"text.shadow"));
}

#[test]
fn reports_a_bullet_shadow_on_a_labelled_paragraph() {
    let mut p = Presentation::new();
    let bulleted = p.add(
        "TSWP.ListStyleArchive",
        json!({ "label_types": [2, 0], "strings": ["\u{2022}", ""], "shadow": retro() }),
    );
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["dot\nnone"],
            "table_list_style": { "entries": [{ "character_index": 0, "object": bulleted }] },
            "table_para_data": { "entries": [
                { "character_index": 0, "first": 0 }, { "character_index": 4, "first": 1 },
            ] },
        }),
    );
    let text_box = p.text_box(storage, geometry(0.0, 0.0, 400.0, 40.0), None);
    p.slide(json!({ "drawables_z_order": [text_box] }), json!({}), None);
    let lowered = lower(&p.bytes());
    assert_eq!(
        messages(&lowered),
        [("KN1304", "A shadow is not drawn: bullet shadow")]
    );
    assert_eq!(
        lowered.diagnostics[0].feature.map(|f| f.as_str()),
        Some("text.shadow")
    );
}

#[test]
fn a_placeholder_without_a_shadow_takes_none_from_the_placeholder_it_binds_to() {
    let mut p = Presentation::new();
    let shadowed = p.add(
        "TSWP.ShapeStyleArchive",
        json!({ "super": { "shape_properties": { "fill": { "color": rgb(1.0, 1.0, 1.0) }, "shadow": retro() } } }),
    );
    let placeholder = |p: &mut Presentation, text: &str, style: Option<&Value>| {
        let storage = p.storage(text, None, json!({}));
        let mut shape = json!({ "super": { "geometry": geometry(100.0, 100.0, 400.0, 100.0) } });
        if let Some(style) = style {
            shape["style"] = style.clone();
        }
        p.add(
            "KN.PlaceholderArchive",
            json!({ "super": { "super": shape, "owned_storage": storage }, "kind": 2 }),
        )
    };
    let title = placeholder(&mut p, "Layout", Some(&shadowed));
    let template = p.template(
        "Title",
        vec![title.clone()],
        json!({ "titlePlaceholder": title }),
    );
    let own = placeholder(&mut p, "Own", Some(&shadowed));
    let plain = placeholder(&mut p, "Plain", None);
    p.slide(
        json!({ "template_slide": template, "drawables_z_order": [own] }),
        json!({}),
        None,
    );
    p.slide(
        json!({ "template_slide": template, "drawables_z_order": [plain] }),
        json!({}),
        None,
    );
    let lowered = lower(&p.bytes());
    assert_eq!(lowered.elements(1)[0]["shadow"], Value::Null);
    assert!(
        lowered.elements(1)[0]
            .as_object()
            .unwrap()
            .contains_key("shadow")
    );
    let compiled = compile(&p.bytes(), "placeholders.key");
    let presentation = compiled.presentation.unwrap();
    let shadow = |slide: usize| {
        let Body::Built(built) = &presentation.slides[slide].body else {
            panic!("slide {slide} failed")
        };
        match &built.elements[0] {
            Element::Shape(shape) => shape.shadow.as_ref().map(|s| s.dy.to_f64()),
            other => panic!("{other:?}"),
        }
    };
    assert_eq!((shadow(0), shadow(1)), (Some(3.287), None));
}
