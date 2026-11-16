//! Keynote's model as IR values: text storages, paint and outlines.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};
use std::f64::consts::PI;

use prost::Message;
use serde::Serialize;
use serde_json::{Value, json};
use slidock_core_api::Limits;
use slidock_core_api::limits::presentation_budget;
use slidock_geometry::{StrokeEnds, with_line_ends};
use slidock_ir::diagnostics::{Diagnostic, Stage};
use slidock_ir::geometry::Seg;
use slidock_ir::ids::{NodeId, Sha256};
use slidock_ir::text::Align;
use slidock_keynote::ObjectGraph;
use slidock_keynote::context::{DataId, Layer, Lowering, MediaLookup, Setup};
use slidock_keynote::geometry::SlideMap;
use slidock_keynote::graph::{IndexBudget, IndexSource, ReadMany, read_object_graph};
use slidock_keynote::paint::{Ends, FillBox, color, fill, stroke};
use slidock_keynote::paths::{instant_alpha_geometry, mask_paths, shape_geometry};
use slidock_keynote::proto::{tsd, tsp, tswp};
use slidock_keynote::text::{BoxPlace, FontUses, notes_body, read_paragraphs, text_body};
use support::json::encode;
use support::keynote::{Presentation, id_of};

struct Files(Vec<(String, Vec<u8>)>);

impl IndexSource for Files {
    fn entry_sizes(&self) -> Vec<(&str, u64)> {
        self.0
            .iter()
            .map(|(p, d)| (p.as_str(), d.len() as u64))
            .collect()
    }

    fn read_many(&self, paths: &[&str]) -> ReadMany<'_> {
        let mut out = ReadMany::default();
        for (path, data) in &self.0 {
            if paths.contains(&path.as_str()) {
                out.read
                    .insert(path.clone(), Cow::Borrowed(data.as_slice()));
            }
        }
        out
    }
}

fn graph_of(p: &mut Presentation) -> ObjectGraph {
    read_object_graph(&Files(p.files(1920.0, 1080.0)), &IndexBudget::default()).unwrap()
}

/// Media by data id: what the lowering finds in the package.
#[derive(Default)]
struct Images(BTreeMap<u64, Sha256>);

impl MediaLookup for Images {
    fn media(&mut self, data: Option<DataId>) -> Option<Sha256> {
        data.and_then(|data| self.0.get(&data.0).copied())
    }

    fn is_image(&mut self, data: Option<DataId>) -> bool {
        data.is_some_and(|data| self.0.contains_key(&data.0))
    }
}

/// A lowering of container `kn-1` over a graph, and the diagnostics it reports.
fn with_lowering<T>(
    graph: &ObjectGraph,
    images: &mut Images,
    slide_number: usize,
    run: impl FnOnce(&mut Lowering<'_>) -> T,
) -> (T, Vec<Diagnostic>) {
    let limits = Limits::default();
    let mut diagnostics = Vec::new();
    let mut sink = slidock_core_api::Sink::new(&mut diagnostics, Stage::Frontend, "keynote");
    let out = {
        let mut ctx = Lowering::new(
            Setup {
                graph,
                container: NodeId::new("kn-1").unwrap(),
                layer: Layer::Slide,
                limits: &limits,
                media: images,
                slide_width: 1920.0,
                slide_height: 1080.0,
                slide_number,
                native_motion: false,
            },
            &presentation_budget(&limits),
            &mut sink,
        );
        run(&mut ctx)
    };
    (out, diagnostics)
}

fn as_json<T: Serialize>(value: &T) -> Value {
    serde_json::from_str(
        &slidock_canon::to_canonical_json(value, slidock_canon::Layout::Compact).unwrap(),
    )
    .unwrap()
}

fn node() -> NodeId {
    NodeId::new("kn-9").unwrap()
}

/// The paragraphs of a storage, as JSON, with the fonts it asks for and draws.
fn paragraphs_of(
    ctx: &mut Lowering<'_>,
    graph: &ObjectGraph,
    storage: &Value,
) -> (Vec<Value>, Vec<String>, Vec<String>) {
    let storage = graph
        .get_id::<tswp::StorageArchive>(slidock_keynote::ObjectId(id_of(storage)))
        .unwrap();
    let (mut requested, mut drawn) = (BTreeSet::new(), BTreeSet::new());
    let paragraphs = read_paragraphs(
        ctx,
        storage,
        &node(),
        &mut FontUses {
            requested: &mut requested,
            drawn: Some(&mut drawn),
        },
        Align::Left,
    )
    .unwrap();
    let paragraphs = paragraphs.iter().map(as_json).collect();
    (
        paragraphs,
        requested.into_iter().collect(),
        drawn.into_iter().collect(),
    )
}

fn runs(paragraph: &Value) -> Vec<&Value> {
    paragraph["inlines"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["kind"] == "run")
        .collect()
}

fn texts(paragraphs: &[Value]) -> Vec<String> {
    paragraphs
        .iter()
        .map(|p| {
            p["inlines"]
                .as_array()
                .unwrap()
                .iter()
                .map(|inline| match inline["kind"].as_str().unwrap() {
                    "run" => inline["text"].as_str().unwrap().to_owned(),
                    "field" => format!("{{{}}}", inline["cached"].as_str().unwrap()),
                    _ => String::new(),
                })
                .collect()
        })
        .collect()
}

fn codes(diagnostics: &[Diagnostic]) -> Vec<&str> {
    diagnostics.iter().map(|d| d.code.code()).collect()
}

fn red() -> Value {
    json!({ "model": 1, "r": 1, "g": 0, "b": 0, "a": 1 })
}

#[test]
fn writes_what_a_run_chain_sets_a_size_set_to_0_is_set() {
    let mut p = Presentation::new();
    let body = p.paragraph_style(
        json!({ "font_size": 80, "font_name": "HelveticaNeue", "font_color": red() }),
        None,
        json!({}),
    );
    let child = p.paragraph_style(json!({ "bold": true }), Some(body), json!({}));
    let emphasis = p.add(
        "TSWP.CharacterStyleArchive",
        json!({ "char_properties": { "italic": true, "font_size": 0 } }),
    );
    let no_color = p.add(
        "TSWP.CharacterStyleArchive",
        json!({ "char_properties": { "font_color_null": true } }),
    );
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["plain em back none"],
            "table_para_style": { "entries": [{ "character_index": 0, "object": child }] },
            "table_char_style": { "entries": [
                { "character_index": 0 }, { "character_index": 6, "object": emphasis },
                { "character_index": 9 }, { "character_index": 14, "object": no_color },
            ] },
        }),
    );
    let graph = graph_of(&mut p);
    let ((paragraphs, requested, drawn), _) =
        with_lowering(&graph, &mut Images::default(), 1, |ctx| {
            paragraphs_of(ctx, &graph, &storage)
        });
    let runs = runs(&paragraphs[0]);
    let props: Vec<Value> = runs
        .iter()
        .map(|run| {
            json!([
                run["text"],
                run["props"]["size"],
                run["props"]["weight"],
                run["props"]["italic"]
            ])
        })
        .collect();
    assert_eq!(
        props,
        [
            json!(["plain ", 80, 700, null]),
            json!(["em ", 0, 700, true]),
            json!(["back ", 80, 700, null]),
            json!(["none", 80, 700, null]),
        ]
    );
    assert_eq!(
        runs[0]["props"]["font"],
        json!({ "kind": "family", "family": "Helvetica Neue" })
    );
    assert_eq!(
        (
            &runs[0]["props"]["color"]["r"],
            &runs[0]["props"]["color"]["g"]
        ),
        (&json!(1), &json!(0))
    );
    // `font_color_null` ends the walk: the presentation default, not the parent's red.
    assert!(runs[3]["props"].get("color").is_none());
    assert_eq!(requested, ["HelveticaNeue"]);
    // Bold on the regular face draws another face, whose PostScript name the presentation lacks.
    assert!(drawn.is_empty());
}

#[test]
fn records_the_faces_runs_draw_named_exact_and_with_visible_characters() {
    let mut p = Presentation::new();
    let mono = p.paragraph_style(json!({ "font_name": "Monaco" }), None, json!({}));
    let spaced = p.add(
        "TSWP.CharacterStyleArchive",
        json!({ "char_properties": { "font_name": "Monoid-Regular" } }),
    );
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["code   "],
            "table_para_style": { "entries": [{ "character_index": 0, "object": mono }] },
            "table_char_style": { "entries": [{ "character_index": 0 }, { "character_index": 4, "object": spaced }] },
        }),
    );
    let graph = graph_of(&mut p);
    let ((_, requested, drawn), _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        paragraphs_of(ctx, &graph, &storage)
    });
    assert_eq!(requested, ["Monaco", "Monoid-Regular"]);
    // Spaces draw no glyph an export would embed.
    assert_eq!(drawn, ["Monaco"]);
}

#[test]
fn tells_visible_characters_from_whitespace_as_javascript_does() {
    let mut p = Presentation::new();
    let face = |p: &mut Presentation, name: &str| {
        p.add(
            "TSWP.CharacterStyleArchive",
            json!({ "char_properties": { "font_name": name } }),
        )
    };
    let (bom, nel, ideographic) = (
        face(&mut p, "Menlo-Regular"),
        face(&mut p, "Monaco"),
        face(&mut p, "Courier"),
    );
    // U+FEFF is whitespace to JavaScript and not to Unicode; U+0085 the other way round.
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["a\u{feff}\u{85}\u{3000}"],
            "table_char_style": { "entries": [
                { "character_index": 0 }, { "character_index": 1, "object": bom },
                { "character_index": 2, "object": nel }, { "character_index": 3, "object": ideographic },
            ] },
        }),
    );
    let graph = graph_of(&mut p);
    let ((_, requested, drawn), _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        paragraphs_of(ctx, &graph, &storage)
    });
    assert_eq!(requested, ["Courier", "Menlo-Regular", "Monaco"]);
    assert_eq!(drawn, ["Monaco"]);
}

#[test]
fn cuts_a_text_link_at_2048_utf16_units() {
    // An odd prefix, so the cut splits a pair, which is dropped.
    let url = format!("https://e.x/a{}", "\u{1f600}".repeat(2048));
    let mut p = Presentation::new();
    let link = p.add("TSWP.HyperlinkFieldArchive", json!({ "url_ref": url }));
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["link"],
            "table_smartfield": { "entries": [{ "character_index": 0, "object": link }] },
        }),
    );
    let graph = graph_of(&mut p);
    let ((paragraphs, _, _), _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        paragraphs_of(ctx, &graph, &storage)
    });
    let cut = runs(&paragraphs[0])[0]["props"]["link"]["url"]
        .as_str()
        .unwrap()
        .to_owned();
    assert_eq!(
        cut,
        format!("https://e.x/a{}", "\u{1f600}".repeat((2048 - 13) / 2))
    );
}

#[test]
fn keeps_the_paragraph_style_before_an_entry_without_one_and_the_empty_line_after_a_final_separator()
 {
    let mut p = Presentation::new();
    let big = p.paragraph_style(json!({ "font_size": 80 }), None, json!({}));
    let blue = p.paragraph_style(
        json!({ "font_size": 60, "font_color": { "model": 1, "r": 0, "g": 0, "b": 1, "a": 1 } }),
        None,
        json!({}),
    );
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["one\n\ntwo\nthree\n"],
            "table_para_style": { "entries": [
                { "character_index": 0, "object": big }, { "character_index": 4 },
                { "character_index": 5, "object": blue }, { "character_index": 9 },
            ] },
        }),
    );
    let graph = graph_of(&mut p);
    let ((paragraphs, _, _), _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        paragraphs_of(ctx, &graph, &storage)
    });
    assert_eq!(texts(&paragraphs), ["one", "", "two", "three", ""]);
    let sizes: Vec<&Value> = paragraphs.iter().map(|p| &p["endProps"]["size"]).collect();
    assert_eq!(
        sizes,
        [&json!(80), &json!(80), &json!(60), &json!(60), &json!(60)]
    );
    assert_eq!(paragraphs[1]["inlines"], json!([]));
}

#[test]
fn splits_lines_tabs_and_fields_inside_runs_and_leaves_out_other_attachments() {
    let mut p = Presentation::new();
    let slide_number = p.add_record(
        support::Type::Name("TSWP.NumberAttachmentArchive"),
        Vec::new(),
    );
    let drawable = p.add_record(
        support::Type::Name("TSWP.DrawableAttachmentArchive"),
        Vec::new(),
    );
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["a\u{2028}b\tc \u{fffc} \u{fffc}"],
            "table_attachment": { "entries": [
                { "character_index": 6, "object": slide_number }, { "character_index": 8, "object": drawable },
            ] },
        }),
    );
    let graph = graph_of(&mut p);
    let ((paragraphs, _, _), diagnostics) =
        with_lowering(&graph, &mut Images::default(), 7, |ctx| {
            paragraphs_of(ctx, &graph, &storage)
        });
    let kinds: Vec<&Value> = paragraphs[0]["inlines"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| &i["kind"])
        .collect();
    assert_eq!(
        kinds,
        ["run", "break", "run", "tab", "run", "field", "run"]
            .map(|k| json!(k))
            .iter()
            .collect::<Vec<_>>()
    );
    assert_eq!(texts(&paragraphs), ["abc {7} "]);
    let reported: Vec<(&str, Value)> = diagnostics
        .iter()
        .map(|d| (d.code.code(), as_json(&d.data)))
        .collect();
    assert_eq!(
        reported,
        [(
            "KN1201",
            json!({ "attachment": "TSWP.DrawableAttachmentArchive" })
        )]
    );
}

#[test]
fn labels_bullets_and_numbers_hangs_labels_in_their_text_indent_and_keeps_links() {
    let mut p = Presentation::new();
    let style = p.paragraph_style(json!({ "font_size": 20 }), None, json!({}));
    let numbered = p.add(
        "TSWP.ListStyleArchive",
        json!({ "label_types": [3, 2, 1], "number_types": [9, 0, 0], "strings": ["", "\u{2013}", ""],
                "indents": [0, 48, 96], "text_indents": [1, 1.5, 1] }),
    );
    let link = p.add(
        "TSWP.HyperlinkFieldArchive",
        json!({ "url_ref": "https://example.com" }),
    );
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["first\nsub\npic\nplain"],
            "table_para_style": { "entries": [{ "character_index": 0, "object": style }] },
            "table_list_style": { "entries": [{ "character_index": 0, "object": numbered }, { "character_index": 14 }] },
            "table_para_data": { "entries": [
                { "character_index": 0, "first": 0 }, { "character_index": 6, "first": 1 },
                { "character_index": 10, "first": 2 }, { "character_index": 14, "first": 0 },
            ] },
            "table_smartfield": { "entries": [{ "character_index": 6, "object": link }, { "character_index": 9 }] },
        }),
    );
    let graph = graph_of(&mut p);
    let ((paragraphs, _, _), diagnostics) =
        with_lowering(&graph, &mut Images::default(), 1, |ctx| {
            paragraphs_of(ctx, &graph, &storage)
        });
    let bullets: Vec<Value> = paragraphs
        .iter()
        .map(|p| json!([p["level"], p["props"]["bullet"]]))
        .collect();
    assert_eq!(
        bullets,
        [
            json!([0, { "kind": "auto", "scheme": "alphaUcPeriod", "startAt": 1 }]),
            json!([1, { "kind": "char", "char": "\u{2013}" }]),
            json!([2, { "kind": "char", "char": "\u{2022}" }]),
            json!([0, { "kind": "none" }]),
        ]
    );
    // The label hangs in text_indents ems of the first run, after the level's indent.
    assert_eq!(
        (
            &paragraphs[1]["props"]["marginLeft"],
            &paragraphs[1]["props"]["indent"]
        ),
        (&json!(78), &json!(-30))
    );
    assert_eq!(
        runs(&paragraphs[1])[0]["props"]["link"],
        json!({ "kind": "url", "url": "https://example.com" })
    );
    assert!(runs(&paragraphs[2])[0]["props"].get("link").is_none());
    assert_eq!(codes(&diagnostics), ["KN1203"]);
}

#[test]
fn reads_paragraph_layout_and_leaves_out_the_space_before_the_first_paragraph() {
    let mut p = Presentation::new();
    let style = p.paragraph_style(
        json!({}),
        None,
        json!({ "alignment": 2, "space_before": 12, "space_after": 6, "left_indent": 10, "first_line_indent": 5,
                "line_spacing": { "mode": 0, "amount": 0.9 }, "default_tab_stops": 50 }),
    );
    let storage = p.storage("one\ntwo", Some(style), json!({}));
    let graph = graph_of(&mut p);
    let ((paragraphs, _, _), _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        paragraphs_of(ctx, &graph, &storage)
    });
    let props: Vec<&Value> = paragraphs.iter().map(|p| &p["props"]).collect();
    let expected: Vec<Value> = [0, 12]
        .map(|before| {
            json!({
                "align": "center", "marginLeft": 10, "marginRight": 0, "indent": 5,
                "spaceBefore": { "kind": "pt", "value": before }, "spaceAfter": { "kind": "pt", "value": 6 },
                "lineSpacing": { "kind": "multiple", "value": 0.9, "basis": "coretext" }, "tabSize": 50,
                "bullet": { "kind": "none" }, "rtl": false,
            })
        })
        .to_vec();
    assert_eq!(props, expected.iter().collect::<Vec<_>>());
}

#[test]
fn reads_tables_out_of_order_sorted_and_reports_them() {
    let mut p = Presentation::new();
    let bold = p.add(
        "TSWP.CharacterStyleArchive",
        json!({ "char_properties": { "bold": true } }),
    );
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({ "text": ["ab"],
                "table_char_style": { "entries": [{ "character_index": 1, "object": bold }, { "character_index": 0 }] } }),
    );
    let graph = graph_of(&mut p);
    let ((paragraphs, _, _), diagnostics) =
        with_lowering(&graph, &mut Images::default(), 1, |ctx| {
            paragraphs_of(ctx, &graph, &storage)
        });
    let weights: Vec<Value> = runs(&paragraphs[0])
        .iter()
        .map(|r| json!([r["text"], r["props"]["weight"]]))
        .collect();
    assert_eq!(weights, [json!(["a", null]), json!(["b", 700])]);
    assert_eq!(codes(&diagnostics), ["KN1202"]);
}

/// A box at `x`, `w` wide, on the slide.
fn place(x: f64, w: f64) -> BoxPlace {
    BoxPlace {
        x,
        y: 0.0,
        w,
        h: 0.0,
        to_slide: SlideMap::SLIDE,
    }
}

#[test]
fn lays_out_a_text_box_insets_anchor_shrink_and_a_box_that_grows_from_its_anchor_to_the_slide_edge()
{
    let mut p = Presentation::new();
    let centred = p.paragraph_style(json!({}), None, json!({ "alignment": 2 }));
    let storage = p.storage("Hello", Some(centred), json!({}));
    let middle = p.add(
        "TSWP.ShapeStyleArchive",
        json!({ "shape_properties": { "vertical_alignment": 1, "padding_null": true } }),
    );
    let shrink = p.add(
        "TSWP.ShapeStyleArchive",
        json!({ "shape_properties": { "shrink_to_fit": true } }),
    );
    let empty = p.storage("", None, json!({}));
    let graph = graph_of(&mut p);
    let (bodies, _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        let mut requested = BTreeSet::new();
        let mut body = |storage: &Value, style: Option<&Value>, flags: u32, place: BoxPlace| {
            let mut fonts = FontUses {
                requested: &mut requested,
                drawn: None,
            };
            let style = style.map(reference);
            text_body(
                ctx,
                Some(&reference(storage)),
                style.as_ref(),
                Some(flags),
                place,
                &node(),
                &mut fonts,
            )
            .unwrap()
            .map(|body| as_json(&body))
        };
        (
            body(&storage, Some(&middle), 0, place(600.0, 0.0)),
            body(&storage, Some(&shrink), 3, place(0.0, 100.0)),
            body(&empty, None, 3, place(0.0, 0.0)),
        )
    });
    let (grown, shrunk, empty) = bodies;
    assert_eq!(
        grown.unwrap()["body"],
        json!({ "insets": { "l": 0, "t": 0, "r": 0, "b": 0 }, "anchor": "middle", "wrap": "square",
                "autofit": { "kind": "resize", "width": true, "height": true, "anchorX": "center" }, "maxWidth": 1200 })
    );
    let shrunk = shrunk.unwrap();
    assert_eq!(
        shrunk["body"]["insets"],
        json!({ "l": 4, "t": 4, "r": 4, "b": 4 })
    );
    assert_eq!(
        (&shrunk["body"]["anchor"], &shrunk["body"]["autofit"]),
        (
            &json!("top"),
            &json!({ "kind": "shrink", "steps": "percent" })
        )
    );
    assert!(empty.is_none());
}

#[test]
fn reads_notes_as_rich_text_and_none_from_blank_notes() {
    let mut p = Presentation::new();
    let text = p.storage("Line one\rLine two", None, json!({}));
    let note = p.add("KN.NoteArchive", json!({ "containedStorage": text }));
    let blank_text = p.storage("  \n", None, json!({}));
    let blank = p.add("KN.NoteArchive", json!({ "containedStorage": blank_text }));
    let graph = graph_of(&mut p);
    let ((notes, none, seen), _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        let container = NodeId::new("kn-1").unwrap();
        let notes = notes_body(ctx, Some(&reference(&note)), &container)
            .unwrap()
            .unwrap();
        let none = notes_body(ctx, Some(&reference(&blank)), &container).unwrap();
        (as_json(&notes), none, ctx.reporter.features())
    });
    assert_eq!(
        texts(notes["paragraphs"].as_array().unwrap()),
        ["Line one", "Line two"]
    );
    assert!(none.is_none());
    // The notes' text stays apart from the slide's: only slide.notes is seen.
    assert_eq!(
        seen.iter().map(|f| f.as_str()).collect::<Vec<_>>(),
        ["slide.notes"]
    );
}

#[test]
fn trims_notes_as_javascript_does() {
    let mut p = Presentation::new();
    let notes: Vec<(&str, Value)> = [" \u{feff}", "\u{3000}\t", "\u{85}", "\u{200b}"]
        .into_iter()
        .map(|text| {
            let storage = p.storage(text, None, json!({}));
            (
                text,
                p.add("KN.NoteArchive", json!({ "containedStorage": storage })),
            )
        })
        .collect();
    let graph = graph_of(&mut p);
    let (read, _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        let container = NodeId::new("kn-1").unwrap();
        notes
            .iter()
            .map(|(text, note)| {
                let body = notes_body(ctx, Some(&reference(note)), &container).unwrap();
                (*text, body.is_some())
            })
            .collect::<Vec<_>>()
    });
    assert_eq!(
        read,
        [
            (" \u{feff}", false),
            ("\u{3000}\t", false),
            ("\u{85}", true),
            ("\u{200b}", true)
        ]
    );
}

fn reference(value: &Value) -> tsp::Reference {
    tsp::Reference {
        identifier: Some(id_of(value)),
        ..tsp::Reference::default()
    }
}

fn message<T: Message + Default>(type_name: &str, value: Value) -> T {
    T::decode(encode(type_name, &value).as_slice()).unwrap()
}

#[test]
fn keeps_a_colour_in_its_space_and_converts_cmyk_and_white() {
    let rgb =
        |value: Value| as_json(&color(Some(&message::<tsp::Color>("TSP.Color", value))).unwrap());
    assert_eq!(
        rgb(json!({ "model": 1, "r": 1, "g": 0.5, "b": 0, "rgbspace": 2 })),
        json!({ "kind": "rgb", "space": "p3", "r": 1, "g": 0.5, "b": 0, "a": 1 })
    );
    let cmyk = rgb(json!({ "model": 2, "c": 1, "m": 0, "y": 0, "k": 0.5 }));
    assert_eq!(
        (&cmyk["r"], &cmyk["g"], &cmyk["b"]),
        (&json!(0), &json!(0.5), &json!(0.5))
    );
    let white = rgb(json!({ "model": 3, "w": 0.25, "a": 0.5 }));
    assert_eq!(
        (&white["r"], &white["g"], &white["b"], &white["a"]),
        (&json!(0.25), &json!(0.25), &json!(0.25), &json!(0.5))
    );
}

#[test]
fn reads_solid_gradient_and_image_fills_a_missing_image_leaves_none() {
    let graph = graph_of(&mut Presentation::new());
    let sha = Sha256::parse(&"a".repeat(64)).unwrap();
    let mut images = Images([(7, sha)].into());
    let fill_of = |value: Value, ctx: &mut Lowering<'_>| {
        let archive = message::<tsd::FillArchive>("TSD.FillArchive", value);
        let place = FillBox::Frame { w: 300.0, h: 100.0 };
        as_json(&fill(Some(&archive), place, ctx, None).unwrap())
    };
    let (fills, diagnostics) = with_lowering(&graph, &mut images, 1, |ctx| {
        let gradient = json!({ "stops": [{ "fraction": 0 }, { "fraction": 1, "color": { "model": 3, "w": 1 } }], "opacity": 0.5 });
        // Keynote's angle is counter-clockwise, in radians.
        let upward = support::keynote::merge(
            gradient.clone(),
            json!({ "anglegradient": { "gradientangle": PI / 2.0 } }),
        );
        let radial = support::keynote::merge(
            gradient,
            json!({ "type": 1, "anglegradient": { "gradientangle": 1.75 * PI } }),
        );
        [
            fill_of(json!({ "color": { "model": 3, "w": 1 } }), ctx),
            fill_of(json!({ "gradient": upward }), ctx),
            fill_of(json!({ "gradient": radial }), ctx),
            fill_of(
                json!({ "image": { "imagedata": { "identifier": "7" }, "technique": 1 } }),
                ctx,
            ),
            fill_of(
                json!({ "image": { "imagedata": { "identifier": "8" } } }),
                ctx,
            ),
        ]
    });
    assert_eq!(fills[0]["kind"], "solid");
    let linear = &fills[1];
    assert_eq!(
        [
            &linear["kind"],
            &linear["angle"],
            &linear["scaled"],
            &linear["rotateWithShape"]
        ],
        [&json!("linear"), &json!(270), &json!(false), &json!(true)]
    );
    assert_eq!(
        (
            &linear["stops"][0]["pos"],
            &linear["stops"][0]["color"]["a"]
        ),
        (&json!(0), &json!(0.5))
    );
    assert_eq!(
        (
            &linear["stops"][1]["pos"],
            &linear["stops"][1]["color"]["r"],
            &linear["stops"][1]["color"]["a"]
        ),
        (&json!(1), &json!(1), &json!(0.5))
    );
    assert_eq!(
        (&fills[2]["kind"], &fills[2]["center"]["x"]),
        (&json!("radial"), &json!(0.5))
    );
    assert_eq!(
        fills[3],
        json!({ "kind": "image", "media": "a".repeat(64), "mode": "stretch" })
    );
    assert_eq!(fills[4], json!({ "kind": "none" }));
    assert_eq!(codes(&diagnostics), ["KN1306"]);
}

/// A gradient fill archive's lowering over `place`, and the reasons reported (KN1301 messages).
fn gradient_over(gradient: Value, place: FillBox) -> (Value, Vec<String>) {
    let graph = graph_of(&mut Presentation::new());
    let (fill, diagnostics) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        let archive =
            message::<tsd::FillArchive>("TSD.FillArchive", json!({ "gradient": gradient }));
        as_json(&fill(Some(&archive), place, ctx, Some(&node())).unwrap())
    });
    let reasons = diagnostics
        .iter()
        .map(|d| {
            assert_eq!(d.code.code(), "KN1301");
            d.message.clone()
        })
        .collect();
    (fill, reasons)
}

fn black_to_white() -> Value {
    json!([
        { "fraction": 0, "color": { "model": 1, "r": 0, "g": 0, "b": 0, "a": 1 } },
        { "fraction": 1, "color": { "model": 1, "r": 1, "g": 1, "b": 1, "a": 1 } },
    ])
}

fn transform(start: (f64, f64), end: (f64, f64), base: (f64, f64)) -> Value {
    json!({
        "start": { "x": start.0, "y": start.1 },
        "end": { "x": end.0, "y": end.1 },
        "baseNaturalSize": { "width": base.0, "height": base.1 },
    })
}

const BOX: FillBox = FillBox::Frame { w: 600.0, h: 200.0 };

#[test]
fn draws_retros_transform_gradient_with_its_inflection_as_a_p3_midpoint() {
    // kn-4245054: Keynote's 0° angle stored as a transform from (−1, 540) to (1921, 540) in a 1920 × 1080 base.
    let gradient = json!({
        "type": 0,
        "stops": [
            { "fraction": 0.48464, "inflection": 0.3914,
              "color": { "model": 1, "r": 0.33759, "g": 0.75773, "b": 1, "a": 0.3, "rgbspace": 1 } },
            { "fraction": 0.98977, "inflection": 0.5,
              "color": { "model": 1, "r": 0.08225, "g": 0.22281, "b": 0.53728, "a": 1, "rgbspace": 2 } },
        ],
        "opacity": 1,
        "advancedGradient": false,
        "transformgradient": transform((-1.0, 540.0), (1921.0, 540.0), (1920.0, 1080.0)),
    });
    let (fill, reasons) = gradient_over(
        gradient,
        FillBox::Frame {
            w: 1012.3643,
            h: 1062.0042,
        },
    );
    assert!(reasons.is_empty(), "{reasons:?}");
    assert_eq!(
        (&fill["kind"], &fill["from"], &fill["to"]),
        (
            &json!("linear"),
            &json!({ "x": -0.00052, "y": 0.5 }),
            &json!({ "x": 1.00052, "y": 0.5 })
        )
    );
    let stops = fill["stops"].as_array().unwrap();
    let positions: Vec<&Value> = stops.iter().map(|stop| &stop["pos"]).collect();
    assert_eq!(positions, [0.48464, 0.68235, 0.98977]);
    // The even mix in Display P3, where the gradient blends: the sRGB stop converted, alpha averaged.
    assert_eq!(
        stops[1]["color"],
        json!({ "kind": "rgb", "space": "p3", "r": 0.26672, "g": 0.48559, "b": 0.75767, "a": 0.65 })
    );
    // A straight segment (inflection ½) inserts nothing.
    let straight = json!({ "stops": black_to_white(), "anglegradient": { "gradientangle": 0 } });
    let (fill, _) = gradient_over(straight, BOX);
    assert_eq!(fill["stops"].as_array().unwrap().len(), 2);
}

#[test]
fn skews_a_transform_gradient_with_its_base_box_and_stretches_a_radial_one() {
    // Iso-lines perpendicular to (100, 100) in a 200 × 100 base: t = u.x + u.y/2.
    let oblique = json!({ "stops": black_to_white(),
        "transformgradient": transform((0.0, 0.0), (100.0, 100.0), (200.0, 100.0)) });
    let (fill, reasons) = gradient_over(oblique, BOX);
    assert!(reasons.is_empty());
    assert_eq!(
        (&fill["from"], &fill["to"]),
        (&json!({ "x": 0, "y": 0 }), &json!({ "x": 0.8, "y": 0.4 }))
    );
    let radial = json!({ "type": 1, "stops": black_to_white(),
        "transformgradient": transform((100.0, 50.0), (150.0, 50.0), (200.0, 100.0)) });
    let (fill, _) = gradient_over(radial, BOX);
    assert_eq!(
        fill,
        json!({ "kind": "radial", "center": { "x": 0.5, "y": 0.5 }, "radius": 0.25, "radiusY": 0.5,
                "stops": fill["stops"] })
    );
}

#[test]
fn draws_the_angle_when_both_are_stored_and_never_reads_the_advanced_flag() {
    let both = json!({ "stops": black_to_white(), "advancedGradient": true,
        "anglegradient": { "gradientangle": PI / 2.0 },
        "transformgradient": transform((0.0, 0.0), (100.0, 100.0), (200.0, 100.0)) });
    let (fill, reasons) = gradient_over(both, BOX);
    assert!(reasons.is_empty(), "{reasons:?}");
    // Keynote's counter-clockwise radians; the box pushed out 1 pt before the corners are projected.
    assert_eq!(
        [
            &fill["kind"],
            &fill["angle"],
            &fill["outset"],
            &fill["rotateWithShape"]
        ],
        [&json!("linear"), &json!(270), &json!(1), &json!(true)]
    );
}

#[test]
fn turns_an_angle_radials_centre_about_the_point_toward_the_outset_corner() {
    // G18–G23 on Keynote's PDF export of a 400 × 200 box: R = ½·hypot(402, 202) = 224.95 pt; the centre is the box's
    // at 315°, the outset bottom-right corner (401, 201) at 135°, (351, 50) at 225° and (250, 251) at 45°.
    let at = |degrees: f64| {
        let radial = json!({ "type": 1, "stops": black_to_white(),
            "anglegradient": { "gradientangle": degrees.to_radians() } });
        let (fill, reasons) = gradient_over(radial, FillBox::Frame { w: 400.0, h: 200.0 });
        assert!(reasons.is_empty());
        (
            fill["center"]["x"].as_f64().unwrap() * 400.0,
            fill["center"]["y"].as_f64().unwrap() * 200.0,
            fill["radius"].as_f64().unwrap() * 400.0,
            fill["radiusY"].as_f64().unwrap() * 200.0,
        )
    };
    let near = |(x, y, rx, ry): (f64, f64, f64, f64), (ex, ey): (f64, f64)| {
        assert!(
            (x - ex).abs() < 0.01 && (y - ey).abs() < 0.01,
            "({x}, {y}) vs ({ex}, {ey})"
        );
        assert!(
            (rx - 224.95).abs() < 0.01 && (ry - 224.95).abs() < 0.01,
            "{rx} {ry}"
        );
    };
    near(at(315.0), (200.0, 100.0));
    near(at(135.0), (401.0, 201.0));
    near(at(225.0), (351.0, 50.0));
    near(at(45.0), (250.0, 251.0));
}

#[test]
fn spans_a_cell_with_its_angle_draws_a_transform_top_to_bottom_and_a_radial_as_the_cells_ellipse() {
    let angle = json!({ "stops": black_to_white(), "anglegradient": { "gradientangle": 0 } });
    let (fill, _) = gradient_over(angle, FillBox::Cell);
    assert_eq!((&fill["angle"], fill.get("outset")), (&json!(0), None));
    let linear = json!({ "stops": black_to_white(),
        "transformgradient": transform((0.0, 0.0), (100.0, 100.0), (200.0, 100.0)) });
    let (fill, reasons) = gradient_over(linear, FillBox::Cell);
    assert!(reasons.is_empty());
    assert_eq!(
        (&fill["kind"], &fill["angle"]),
        (&json!("linear"), &json!(90))
    );
    // G38: a 300 × 100 cell's radial is the ellipse of semi-axes 212.13 × 70.71 pt about its centre.
    let radial = json!({ "type": 1, "stops": black_to_white(),
        "anglegradient": { "gradientangle": 1.75 * PI } });
    let (fill, reasons) = gradient_over(radial, FillBox::Cell);
    assert!(reasons.is_empty());
    let semi = fill["radius"].as_f64().unwrap();
    assert!((semi * 300.0 - 212.13).abs() < 0.01, "{semi}");
    assert_eq!(
        (&fill["center"], fill.get("radiusY")),
        (&json!({ "x": 0.5, "y": 0.5 }), None)
    );
}

#[test]
fn reports_what_a_gradient_draws_instead() {
    let reason = |gradient: Value| {
        let (fill, reasons) = gradient_over(gradient, BOX);
        (fill["kind"].as_str().unwrap().to_owned(), reasons)
    };
    let one = |text: &str| vec![format!("A gradient is approximated: {text}")];
    assert_eq!(
        reason(json!({ "stops": black_to_white() })),
        (
            "linear".to_owned(),
            one("it has neither an angle nor a transform")
        )
    );
    assert_eq!(
        reason(json!({ "stops": black_to_white(),
            "transformgradient": transform((5.0, 5.0), (5.0, 5.0), (10.0, 10.0)) })),
        ("solid".to_owned(), one("its start and end points coincide"))
    );
    assert_eq!(
        reason(json!({ "stops": black_to_white(),
            "transformgradient": transform((0.0, 0.0), (5.0, 5.0), (0.0, 10.0)) })),
        ("linear".to_owned(), one("its base size is empty"))
    );
    assert_eq!(
        reason(json!({ "type": 3, "stops": black_to_white(), "anglegradient": {} })),
        ("none".to_owned(), one("unknown gradient type 3"))
    );
    // 40 stops, each segment bent: 24 midpoints fit in 64 stops, 15 do not.
    let bent: Vec<Value> = (0..40)
        .map(|i| json!({ "fraction": f64::from(i) / 39.0, "inflection": 0.25 }))
        .collect();
    let (kind, reasons) = reason(json!({ "stops": bent, "anglegradient": {} }));
    assert_eq!(
        (kind.as_str(), reasons),
        ("linear", one("15 midpoints do not fit in 64 stops"))
    );
}

#[test]
fn reports_a_gradient_text_fill_and_keeps_the_texts_colour() {
    let mut p = Presentation::new();
    let body = p.paragraph_style(json!({ "font_color": red() }), None, json!({}));
    let filled = p.add(
        "TSWP.CharacterStyleArchive",
        json!({ "char_properties": { "tsd_fill": { "gradient": { "type": 1, "stops": black_to_white() } } } }),
    );
    let storage = p.add(
        "TSWP.StorageArchive",
        json!({
            "text": ["filled"],
            "table_para_style": { "entries": [{ "character_index": 0, "object": body }] },
            "table_char_style": { "entries": [{ "character_index": 0, "object": filled }] },
        }),
    );
    let graph = graph_of(&mut p);
    let ((paragraphs, _, _), diagnostics) =
        with_lowering(&graph, &mut Images::default(), 1, |ctx| {
            paragraphs_of(ctx, &graph, &storage)
        });
    assert_eq!(runs(&paragraphs[0])[0]["props"]["color"]["r"], json!(1));
    let reported: Vec<(&str, &str)> = diagnostics
        .iter()
        .map(|d| (d.code.code(), d.message.as_str()))
        .collect();
    assert_eq!(
        reported,
        [("KN1301", "A gradient is approximated: a text fill (radial)")]
    );
}

#[test]
fn reads_strokes_none_dashes_in_widths_and_keynote_line_ends_as_path_ends() {
    let graph = graph_of(&mut Presentation::new());
    let (strokes, _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        let mut of = |value: Value| {
            let archive = message::<tsd::StrokeArchive>("TSD.StrokeArchive", value);
            stroke(Some(&archive), Ends::default(), ctx).map(|s| s.map(|s| as_json(&s)))
        };
        (
            of(json!({ "pattern": { "type": 2 } })),
            of(json!({ "width": 0 })),
            of(
                json!({ "width": 3, "pattern": { "type": 0, "count": 2, "pattern": [2, 1, 9, 9] }, "cap": 1 }),
            ),
            stroke(None, Ends::default(), ctx),
        )
    });
    let (none, zero, dashed, absent) = strokes;
    assert_eq!(none, Some(None));
    assert_eq!(zero, Some(None));
    assert!(absent.is_none());
    assert_eq!(
        dashed.unwrap().unwrap(),
        json!({ "width": 3, "fill": { "kind": "solid", "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } },
                "dash": { "custom": [2, 1] }, "cap": "round", "join": "miter" })
    );
}

fn path_source(value: Value) -> tsd::PathSourceArchive {
    message("TSD.PathSourceArchive", value)
}

fn geometry_of(source: Option<Value>, width: f64, height: f64, limit: usize) -> Option<Value> {
    let source = source.map(path_source);
    shape_geometry(source.as_ref(), width, height, limit)
        .unwrap()
        .map(|g| as_json(&g))
}

const LIMIT: usize = 100_000;

fn unit_square() -> Value {
    json!({ "path": { "elements": [
        { "type": 1, "points": [{ "x": 0, "y": 0 }] }, { "type": 2, "points": [{ "x": 100, "y": 0 }] },
        { "type": 2, "points": [{ "x": 100, "y": 100 }] }, { "type": 2, "points": [{ "x": 0, "y": 100 }] },
        { "type": 5 }, { "type": 1, "points": [{ "x": 0, "y": 0 }] },
    ] } })
}

fn custom(segments: Value, w: f64, h: f64) -> Value {
    json!({ "kind": "custom", "paths": [{ "w": w, "h": h, "fill": true, "stroke": true, "fillRule": "nonzero", "segments": segments }] })
}

fn normalised(value: Value) -> Value {
    as_json(&value)
}

#[test]
fn keeps_a_rectangle_a_rounded_rectangle_and_a_path_that_is_the_box_itself_presets() {
    assert_eq!(
        geometry_of(None, 100.0, 50.0, LIMIT).unwrap(),
        json!({ "kind": "preset", "name": "rect", "adjust": {} })
    );
    assert_eq!(
        geometry_of(
            Some(json!({ "scalar_path_source": { "type": 0, "scalar": 10 } })),
            100.0,
            50.0,
            LIMIT
        )
        .unwrap(),
        json!({ "kind": "preset", "name": "roundRect", "adjust": { "adj": 20000 } })
    );
    // The radius is capped at half the shorter side.
    assert_eq!(
        geometry_of(
            Some(json!({ "scalar_path_source": { "type": 0, "scalar": 80 } })),
            100.0,
            50.0,
            LIMIT
        )
        .unwrap()["adjust"],
        json!({ "adj": 50000 })
    );
    assert_eq!(
        geometry_of(
            Some(json!({ "bezier_path_source": unit_square() })),
            200.0,
            0.0,
            LIMIT
        )
        .unwrap()["name"],
        "rect"
    );
}

#[test]
fn stretches_a_drawn_path_from_its_own_bounds_to_the_frame_without_the_trailing_move() {
    let diamond = json!({ "path": { "elements": [
        { "type": 1, "points": [{ "x": 50, "y": 0 }] }, { "type": 2, "points": [{ "x": 100, "y": 50 }] },
        { "type": 2, "points": [{ "x": 50, "y": 100 }] }, { "type": 2, "points": [{ "x": 0, "y": 50 }] },
        { "type": 5 }, { "type": 1, "points": [{ "x": 50, "y": 0 }] },
    ] } });
    assert_eq!(
        geometry_of(
            Some(json!({ "bezier_path_source": diamond })),
            200.0,
            50.0,
            LIMIT
        )
        .unwrap(),
        normalised(custom(
            json!([
                ["M", 100, 0],
                ["L", 200, 25],
                ["L", 100, 50],
                ["L", 0, 25],
                ["Z"]
            ]),
            200.0,
            50.0
        ))
    );
    let line = json!({ "path": { "elements": [
        { "type": 1, "points": [{ "x": 0, "y": 0 }] }, { "type": 2, "points": [{ "x": 100, "y": 0 }] },
    ] } });
    assert_eq!(
        geometry_of(
            Some(json!({ "bezier_path_source": line })),
            548.0,
            0.0,
            LIMIT
        )
        .unwrap(),
        normalised(custom(json!([["M", 0, 0], ["L", 548, 0]]), 548.0, 0.0))
    );
}

#[test]
fn curves_a_connection_line_through_its_middle_point_and_mirrors_a_flipped_path() {
    let elbow = json!({ "path": { "elements": [
        { "type": 1, "points": [{ "x": 0, "y": 0 }] }, { "type": 2, "points": [{ "x": 50, "y": 100 }] },
        { "type": 2, "points": [{ "x": 100, "y": 0 }] },
    ] } });
    assert_eq!(
        geometry_of(
            Some(json!({ "connection_line_path_source": { "super": elbow } })),
            100.0,
            100.0,
            LIMIT
        )
        .unwrap(),
        normalised(custom(
            json!([["M", 0, 0], ["Q", 50, 200, 100, 0]]),
            100.0,
            100.0
        ))
    );
    assert_eq!(
        geometry_of(
            Some(json!({ "connection_line_path_source": { "super": elbow, "type": 1 } })),
            100.0,
            100.0,
            LIMIT
        )
        .unwrap(),
        normalised(custom(
            json!([["M", 0, 0], ["L", 50, 100], ["L", 100, 0]]),
            100.0,
            100.0
        ))
    );
    let triangle = json!({ "path": { "elements": [
        { "type": 1, "points": [{ "x": 0, "y": 0 }] }, { "type": 2, "points": [{ "x": 10, "y": 0 }] },
        { "type": 2, "points": [{ "x": 0, "y": 10 }] }, { "type": 5 },
    ] } });
    assert_eq!(
        geometry_of(
            Some(json!({ "horizontalFlip": true, "bezier_path_source": triangle })),
            10.0,
            10.0,
            LIMIT
        )
        .unwrap(),
        normalised(custom(
            json!([["M", 10, 0], ["L", 0, 0], ["L", 10, 10], ["Z"]]),
            10.0,
            10.0
        ))
    );
}

#[test]
fn keeps_a_curved_connection_line_in_its_box_which_bounds_the_curve() {
    // retro-fidelity's kn-6284213: the curve bulges left of its middle point (x 0.777) to x 0, and Keynote's box is
    // the curve's, 38.324 wide. Keynote's PDF and PPTX export draw the points where they are (the end at x 19.609),
    // not stretched from the points' bounds (37.548 wide) to the box (the end at x 19.222).
    let line = json!({ "path": { "elements": [
        { "type": 1, "points": [{ "x": 38.32424, "y": 0 }] },
        { "type": 2, "points": [{ "x": 0.776_549_34, "y": 58.520_195 }] },
        { "type": 2, "points": [{ "x": 19.609_104, "y": 132.428_28 }] },
    ] } });
    assert_eq!(
        geometry_of(
            Some(json!({ "connection_line_path_source": { "super": line } })),
            38.32424,
            132.42828,
            LIMIT
        )
        .unwrap(),
        normalised(custom(
            json!([["M", 38.324, 0], ["Q", -27.414, 50.826, 19.609, 132.428]]),
            38.324,
            132.428
        ))
    );
}

fn polygon(sides: Value) -> Value {
    json!({ "scalar_path_source": { "type": 1, "scalar": sides } })
}

fn star(points: Option<Value>) -> Value {
    let mut point = json!({ "y": 0.5 });
    if let Some(points) = points {
        point["x"] = points;
    }
    json!({ "point_path_source": { "type": 100, "point": point } })
}

#[test]
fn draws_arrows_polygons_and_stars_and_none_for_counts_keynote_never_sets() {
    let arrow = json!({ "point_path_source": { "type": 1, "point": { "x": 40, "y": 0.5 }, "naturalSize": { "width": 100, "height": 100 } } });
    assert_eq!(
        geometry_of(Some(arrow), 200.0, 100.0, LIMIT).unwrap(),
        normalised(custom(
            json!([
                ["M", 200, 50],
                ["L", 120, 0],
                ["L", 120, 25],
                ["L", 0, 25],
                ["L", 0, 75],
                ["L", 120, 75],
                ["L", 120, 100],
                ["Z"]
            ]),
            200.0,
            100.0
        ))
    );
    assert_eq!(
        geometry_of(Some(polygon(json!(4))), 100.0, 100.0, LIMIT).unwrap(),
        normalised(custom(
            json!([
                ["M", 50, 0],
                ["L", 100, 50],
                ["L", 50, 100],
                ["L", 0, 50],
                ["Z"]
            ]),
            100.0,
            100.0
        ))
    );
    for sides in [
        json!(2),
        json!(12),
        json!(3e6),
        json!("NaN"),
        json!("Infinity"),
    ] {
        assert!(
            geometry_of(Some(polygon(sides.clone())), 100.0, 100.0, LIMIT).is_none(),
            "{sides}"
        );
    }
    for points in [Some(json!(2)), Some(json!(21)), Some(json!("NaN")), None] {
        assert!(
            geometry_of(Some(star(points.clone())), 100.0, 100.0, LIMIT).is_none(),
            "{points:?}"
        );
    }
    let source = path_source(polygon(json!(11)));
    let error = shape_geometry(Some(&source), 100.0, 100.0, 5).unwrap_err();
    assert!(error.to_string().contains("segments"), "{error}");
}

#[test]
fn places_a_keynote_arrowhead_at_the_end_the_style_names_scaled_by_the_stroke_and_stops_the_line_under_it()
 {
    // Keynote's simple arrow points along +y and meets the line at the middle of its base.
    let arrow: tsd::LineEndArchive = message(
        "TSD.LineEndArchive",
        json!({ "path": { "elements": [
            { "type": 1, "points": [{ "x": 0, "y": 0 }] }, { "type": 2, "points": [{ "x": 3, "y": 6 }] },
            { "type": 2, "points": [{ "x": 6, "y": 0 }] }, { "type": 5 },
        ] }, "end_point": { "x": 3, "y": 0 }, "is_filled": true }),
    );
    let pathless = tsd::LineEndArchive::default();
    let graph = graph_of(&mut Presentation::new());
    let width = message::<tsd::StrokeArchive>("TSD.StrokeArchive", json!({ "width": 2 }));
    let ((line, no_end), _) = with_lowering(&graph, &mut Images::default(), 1, |ctx| {
        let line = stroke(
            Some(&width),
            Ends {
                head: Some(&arrow),
                tail: None,
            },
            ctx,
        )
        .unwrap()
        .unwrap();
        let no_end = stroke(
            Some(&width),
            Ends {
                head: Some(&pathless),
                tail: None,
            },
            ctx,
        )
        .unwrap()
        .unwrap();
        (line, no_end)
    });
    // Keynote's head line end sits at the path's end: the IR's tail.
    assert!(line.head.is_none());
    assert_eq!(
        as_json(&line.tail),
        json!({ "type": "path", "segments": [["M", -3, 0], ["L", 0, 6], ["L", 3, 0], ["Z"]], "scaleWithWidth": true })
    );
    let paths = vec![slidock_geometry::ShapePath {
        segments: vec![Seg::M(0.0, 0.0), Seg::L(200.0, 0.0)],
        fill: false,
        stroke: true,
    }];
    let placed = with_line_ends(
        &paths,
        StrokeEnds {
            width: line.width.to_f64(),
            head: line.head.as_ref(),
            tail: line.tail.as_ref(),
        },
    );
    // A 2 pt stroke scales the arrow by 0.4 + 0.6 × 2: 9.6 pt long, as Keynote draws it (retro-fidelity's
    // kn-6255198); the line stops at its base and goes on half the width (1 pt) under it.
    let round = |segments: &[Seg<f64>]| -> Vec<Seg<f64>> {
        segments
            .iter()
            .map(|s| slidock_geometry::map_segment(*s, |c| (c * 1000.0).round() / 1000.0 + 0.0))
            .collect()
    };
    assert_eq!(
        round(&placed.paths[0].segments),
        [Seg::M(0.0, 0.0), Seg::L(190.4, 0.0), Seg::L(191.4, 0.0)]
    );
    assert_eq!(
        round(&placed.ends[0].segments),
        [
            Seg::M(190.4, -4.8),
            Seg::L(200.0, 0.0),
            Seg::L(190.4, 4.8),
            Seg::Z
        ]
    );
    // A pathless archive, which a style writes for "no line end", draws no end.
    assert!(no_end.tail.is_none());
}

#[test]
fn places_a_mask_in_the_image_frame_turned_about_its_centre() {
    let square = path_source(json!({ "bezier_path_source": unit_square() }));
    let at = |x: f64, y: f64, width: f64, height: f64, angle: f64| -> tsd::GeometryArchive {
        message(
            "TSD.GeometryArchive",
            json!({ "position": { "x": x, "y": y }, "size": { "width": width, "height": height }, "angle": angle }),
        )
    };
    assert_eq!(
        as_json(&mask_paths(Some(&square), Some(&at(10.0, 20.0, 30.0, 40.0, 0.0))).unwrap()),
        json!([{ "segments": [["M", 10, 20], ["L", 40, 20], ["L", 40, 60], ["L", 10, 60], ["Z"]],
                 "fill": true, "stroke": false, "fillRule": "nonzero" }])
    );
    // A half turn maps each corner onto the opposite one.
    assert_eq!(
        as_json(
            &mask_paths(Some(&square), Some(&at(0.0, 0.0, 10.0, 10.0, 180.0))).unwrap()[0].segments
        ),
        json!([
            ["M", 10, 10],
            ["L", 0, 10],
            ["L", 0, 0],
            ["L", 10, 0],
            ["Z"]
        ])
    );
    let two = path_source(polygon(json!(2)));
    assert!(mask_paths(Some(&two), Some(&at(0.0, 0.0, 10.0, 10.0, 0.0))).is_none());
}

/// An Instant Alpha outline is a custom geometry of the image's natural size (lowering stretches it to the frame with
/// the image), every element kept as written but a closing move; anything else in it is refused, with the reason.
#[test]
fn reads_an_instant_alpha_outline_in_the_images_natural_size_and_refuses_any_other() {
    let natural = |width: f64, height: f64| -> tsp::Size {
        message("TSP.Size", json!({ "width": width, "height": height }))
    };
    let path =
        |elements: Value| -> tsp::Path { message("TSP.Path", json!({ "elements": elements })) };
    let outline = path(json!([
        { "type": 1, "points": [{ "x": 10, "y": 5 }] },
        { "type": 4, "points": [{ "x": 20, "y": 5 }, { "x": 30, "y": 10 }, { "x": 30, "y": 20 }] },
        { "type": 3, "points": [{ "x": 30, "y": 30 }, { "x": 20, "y": 30 }] },
        { "type": 2, "points": [{ "x": 10, "y": 30 }] },
        { "type": 5 },
        { "type": 1, "points": [{ "x": 10, "y": 5 }] },
    ]));
    let read = |p: &tsp::Path, size: Option<&tsp::Size>, limit: usize| {
        instant_alpha_geometry(p, size, limit).map(|g| as_json(&g))
    };
    assert_eq!(
        read(&outline, Some(&natural(40.0, 32.0)), LIMIT).unwrap(),
        normalised(custom(
            json!([
                ["M", 10, 5],
                ["C", 20, 5, 30, 10, 30, 20],
                ["Q", 30, 30, 20, 30],
                ["L", 10, 30],
                ["Z"]
            ]),
            40.0,
            32.0
        ))
    );
    let refused =
        |p: &tsp::Path, size: Option<&tsp::Size>, limit: usize| read(p, size, limit).unwrap_err();
    let size = natural(40.0, 32.0);
    assert_eq!(
        refused(&outline, None, LIMIT),
        "the image has no natural size"
    );
    assert_eq!(
        refused(&outline, Some(&natural(0.0, 32.0)), LIMIT),
        "the image has no natural size"
    );
    assert_eq!(
        refused(&outline, Some(&natural(f64::from(f32::NAN), 32.0)), LIMIT),
        "the image has no natural size"
    );
    // A size that rounds to 0 on the path grid would leave the outline unscaled, in the image's pixels.
    assert_eq!(
        refused(&outline, Some(&natural(0.0004, 32.0)), LIMIT),
        "the image has no natural size"
    );
    assert_eq!(
        refused(&outline, Some(&natural(40.0, 0.0001)), LIMIT),
        "the image has no natural size"
    );
    assert!(read(&outline, Some(&natural(0.001, 0.001)), LIMIT).is_ok());
    assert_eq!(refused(&outline, Some(&size), 5), "it has 6 elements");
    let mut nan = outline.clone();
    nan.elements[1].points[2].y = Some(f32::INFINITY);
    assert_eq!(refused(&nan, Some(&size), LIMIT), "a point is not a number");
    for (element, point, missing_x) in [(0, 0, true), (3, 0, false), (1, 1, true)] {
        let mut partial = outline.clone();
        let p = &mut partial.elements[element].points[point];
        if missing_x {
            p.x = None;
        } else {
            p.y = None;
        }
        assert_eq!(
            refused(&partial, Some(&size), LIMIT),
            "a point has no x or y",
            "element {element}, point {point}"
        );
    }
    let short = path(json!([
        { "type": 1, "points": [{ "x": 0, "y": 0 }] },
        { "type": 4, "points": [{ "x": 1, "y": 1 }, { "x": 2, "y": 2 }] },
    ]));
    assert_eq!(
        refused(&short, Some(&size), LIMIT),
        "an element of type 4 has 2 points"
    );
    let unknown = path(json!([
        { "type": 1, "points": [{ "x": 0, "y": 0 }] },
        { "type": 6, "points": [{ "x": 1, "y": 1 }] },
    ]));
    assert_eq!(
        refused(&unknown, Some(&size), LIMIT),
        "an element has the unknown type 6"
    );
    let untyped = path(json!([{ "points": [{ "x": 0, "y": 0 }] }]));
    assert_eq!(
        refused(&untyped, Some(&size), LIMIT),
        "an element has no type"
    );
    let moves = path(json!([{ "type": 1, "points": [{ "x": 0, "y": 0 }] }]));
    assert_eq!(refused(&moves, Some(&size), LIMIT), "it is empty");
    assert_eq!(refused(&path(json!([])), Some(&size), LIMIT), "it is empty");
    let open = path(json!([{ "type": 2, "points": [{ "x": 1, "y": 1 }] }]));
    assert_eq!(
        refused(&open, Some(&size), LIMIT),
        "it does not start with a move"
    );
}

#[test]
fn draws_a_rounded_mask_with_a_quarter_circle_cubic_at_each_corner() {
    let rounded = path_source(json!({ "scalar_path_source": { "type": 0, "scalar": 10 } }));
    let at: tsd::GeometryArchive = message(
        "TSD.GeometryArchive",
        json!({ "position": { "x": 0, "y": 0 }, "size": { "width": 100, "height": 50 } }),
    );
    let paths = as_json(&mask_paths(Some(&rounded), Some(&at)).unwrap()[0].segments);
    // A quarter circle as one cubic: control points at 0.5523 of the radius from the corner's ends.
    assert_eq!(paths[0], json!(["M", 10, 0]));
    assert_eq!(paths[1], json!(["L", 90, 0]));
    assert_eq!(paths[2], json!(["C", 95.523, 0, 100, 4.477, 100, 10]));
    assert_eq!(paths.as_array().unwrap().len(), 10);
}
