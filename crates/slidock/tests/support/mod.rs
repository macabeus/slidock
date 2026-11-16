//! Agent-written presentations built in code, the way an agent writes IR: explicit colours and families, a master
//! and a layout when a test needs a chain, and media given as bytes.
#![allow(dead_code, reason = "each suite uses part of the support")]

use std::sync::Arc;

use serde_json::{Value, json};
use slidock::{
    Compiled, Compiler, Frontends, Input, MediaFiles, Options, VerifyMode, slidock_generator,
};
use slidock_io::BytesReader;

pub const NAME: &str = "deck.presentation-ir.json";

pub fn agent() -> Value {
    json!({ "kind": "agent" })
}

pub fn rgb(r: f64, g: f64, b: f64) -> Value {
    json!({ "kind": "rgb", "space": "srgb", "r": r, "g": g, "b": b, "a": 1 })
}

pub fn frame(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({ "x": x, "y": y, "w": w, "h": h, "rotation": 0, "flipH": false, "flipV": false })
}

/// A rectangle holding one paragraph of white Liberation Sans.
pub fn text_shape(id: &str, at: Value, text: &str) -> Value {
    json!({
        "id": id, "origin": agent(), "kind": "shape", "frame": at,
        "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
        "text": { "body": {}, "paragraphs": [{
            "level": 0, "props": { "align": "center" },
            "inlines": [{ "kind": "run", "text": text, "props": {
                "font": { "kind": "family", "family": "Liberation Sans" }, "size": 24, "color": rgb(1.0, 1.0, 1.0)
            } }]
        }] }
    })
}

/// A filled rectangle.
pub fn rect(id: &str, at: Value) -> Value {
    json!({
        "id": id, "origin": agent(), "kind": "shape", "frame": at,
        "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
        "fill": { "kind": "solid", "color": rgb(0.2, 0.4, 0.6) }
    })
}

pub fn picture(id: &str, at: Value, sha256: &str) -> Value {
    json!({ "id": id, "origin": agent(), "kind": "picture", "frame": at, "media": sha256 })
}

pub fn slide(id: &str, elements: Vec<Value>) -> Value {
    json!({
        "id": id, "origin": agent(), "hidden": false, "showLayout": true, "timeline": [],
        "background": { "kind": "solid", "color": rgb(0.12941, 0.12941, 0.12941) },
        "elements": elements
    })
}

/// `slide` on a layout.
pub fn slide_on(id: &str, layout: &str, elements: Vec<Value>) -> Value {
    let mut slide = slide(id, elements);
    slide["layout"] = json!(layout);
    slide
}

pub fn master(id: &str, elements: Vec<Value>) -> Value {
    json!({ "id": id, "origin": agent(), "name": "Master", "elements": elements })
}

pub fn layout(id: &str, master: &str, elements: Vec<Value>) -> Value {
    json!({ "id": id, "origin": agent(), "name": "Layout", "master": master, "showMaster": true,
            "elements": elements })
}

fn defaults() -> Value {
    let level = json!({
        "para": {
            "align": "left", "bullet": { "kind": "none" }, "indent": 0,
            "lineSpacing": { "basis": "size-1.2", "kind": "multiple", "value": 1 },
            "marginLeft": 0, "marginRight": 0, "rtl": false,
            "spaceAfter": { "kind": "pt", "value": 0 }, "spaceBefore": { "kind": "pt", "value": 0 }, "tabSize": 72
        },
        "run": {
            "baseline": 0, "caps": "none", "color": { "kind": "theme", "mods": [], "slot": "tx1" },
            "font": { "kind": "theme", "slot": "minor" }, "highlight": null, "italic": false, "letterSpacing": 0,
            "link": null, "scriptScale": 1, "size": 18, "strike": "none", "synthesizeBold": true,
            "underline": "none", "weight": 400
        }
    });
    let text: serde_json::Map<String, Value> =
        (0..9).map(|i| (i.to_string(), level.clone())).collect();
    json!({
        "body": { "anchor": "top", "autofit": { "kind": "none" }, "insets": { "b": 3.6, "l": 7.2, "r": 7.2, "t": 3.6 },
                  "overflow": "spread", "wrap": "square" },
        "text": text
    })
}

/// A 720 × 405 pt presentation of these slides, with no masters, layouts or media.
pub fn presentation(slides: Vec<Value>) -> Value {
    json!({
        "$schema": slidock::SCHEMA_URL,
        "size": { "w": 720, "h": 405 },
        "defaults": defaults(),
        "themes": {}, "masters": {}, "layouts": {}, "fonts": {}, "media": {},
        "phase": "source",
        "slides": slides
    })
}

/// One slide saying hello.
pub fn hello() -> Value {
    presentation(vec![slide(
        "ir-s1",
        vec![text_shape(
            "ir-s1.hello",
            frame(60.0, 160.0, 600.0, 80.0),
            "Hello",
        )],
    )])
}

/// A master with a band, a layout on it with a box, three slides: the first and last on the layout.
pub fn chain() -> Value {
    let mut p = presentation(vec![
        slide_on(
            "ir-s1",
            "ir-l",
            vec![text_shape(
                "ir-s1.title",
                frame(40.0, 40.0, 640.0, 60.0),
                "One",
            )],
        ),
        slide(
            "ir-s2",
            vec![rect("ir-s2.box", frame(10.0, 10.0, 50.0, 50.0))],
        ),
        slide_on("ir-s3", "ir-l", vec![]),
    ]);
    p["masters"] =
        json!({ "ir-m": master("ir-m", vec![rect("ir-m.band", frame(0.0, 380.0, 720.0, 25.0))]) });
    p["layouts"] = json!({ "ir-l": layout("ir-l", "ir-m", vec![rect("ir-l.box", frame(600.0, 20.0, 80.0, 80.0))]) });
    p
}

/// A 1 × 1 24-bit BMP holding one red pixel.
pub fn red_bmp() -> Vec<u8> {
    let mut bmp = vec![0u8; 58];
    bmp[..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&58u32.to_le_bytes());
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&1i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&1i32.to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
    bmp[54..57].copy_from_slice(&[0, 0, 255]);
    bmp
}

pub fn sha256(bytes: &[u8]) -> String {
    slidock_canon::sha256_hex(bytes)
}

/// Adds a media entry for `bytes` of type `mime`, `w` × `h` pixels; returns its hash.
pub fn add_media(presentation: &mut Value, bytes: &[u8], mime: &str, w: u64, h: u64) -> String {
    let sha = sha256(bytes);
    presentation["media"][&sha] = json!({
        "sha256": sha, "type": mime, "bytes": bytes.len(), "width": w, "height": h,
        "origin": { "kind": "source", "part": "media/item" }
    });
    sha
}

/// A slide showing a BMP, which plan-media transcodes to PNG.
pub fn with_bmp() -> (Value, Vec<u8>) {
    let bmp = red_bmp();
    let mut p = presentation(vec![]);
    let sha = add_media(&mut p, &bmp, "image/bmp", 1, 1);
    p["slides"] = json!([slide(
        "ir-s1",
        vec![picture("ir-s1.pic", frame(10.0, 10.0, 100.0, 100.0), &sha)]
    )]);
    (p, bmp)
}

pub fn bytes(presentation: &Value) -> Vec<u8> {
    serde_json::to_vec_pretty(presentation).unwrap()
}

/// The media set holding each of `files` under its hash.
pub fn media_files(files: &[(&[u8], &str)]) -> MediaFiles {
    let mut media = MediaFiles::new();
    for (bytes, ext) in files {
        assert!(media.insert_bytes(
            &format!("{}.{ext}", sha256(bytes)),
            Arc::<[u8]>::from(*bytes)
        ));
    }
    media
}

/// `presentation` compiled to HTML through the facade, verified after every pass.
pub fn compile_with(presentation: &Value, media: MediaFiles) -> Compiled {
    let compiler = Compiler::new(Frontends::default().with_ir_media(media)).unwrap();
    let reader = BytesReader::new(bytes(presentation));
    let options = Options {
        verify: VerifyMode::Each,
        generator: slidock_generator(),
        ..Options::default()
    };
    compiler
        .compile(&Input::new(NAME, &reader), &options)
        .unwrap()
}

pub fn compile(presentation: &Value) -> Compiled {
    compile_with(presentation, MediaFiles::new())
}

/// Codes and containers of diagnostics, `presentation` for none.
pub fn codes(diagnostics: &[slidock_ir::diagnostics::Diagnostic]) -> Vec<(String, String)> {
    diagnostics
        .iter()
        .map(|d| {
            (
                d.code.code().to_owned(),
                d.container
                    .as_ref()
                    .map_or("presentation".to_owned(), |c| c.as_str().to_owned()),
            )
        })
        .collect()
}
