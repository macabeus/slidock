//! Compiles `data/builtin-metrics.json` and `data/floor-metrics.json` into static Rust data
//! (`$OUT_DIR/builtin_metrics.rs`, `$OUT_DIR/floor_metrics.rs`), so the catalogs need no JSON parser at run time and a
//! malformed table fails the build.

use std::error::Error;
use std::fmt::Write as _;
use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    families: Vec<Family>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Family {
    family: String,
    faces: Vec<Face>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Face {
    weight: u32,
    #[serde(default = "normal_width")]
    width: u16,
    style: String,
    postscript_name: String,
    variable: bool,
    metrics: Metrics,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Metrics {
    units_per_em: u32,
    hhea: VMetrics,
    typo: VMetrics,
    win: WinMetrics,
    bounds: Option<Bounds>,
    grave_top: Option<i32>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct VMetrics {
    ascender: i32,
    descender: i32,
    line_gap: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WinMetrics {
    ascent: i32,
    descent: i32,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct Bounds {
    y_min: i32,
    y_max: i32,
}

/// `OS/2.usWidthClass` of a face of normal width, when a face states none.
fn normal_width() -> u16 {
    5
}

fn vmetrics(m: &VMetrics) -> String {
    format!(
        "VMetrics {{ ascender: {}, descender: {}, line_gap: {} }}",
        m.ascender, m.descender, m.line_gap
    )
}

fn face(out: &mut String, family: &str, face: &Face) -> Result<(), Box<dyn Error>> {
    let style = match face.style.as_str() {
        "normal" => "Normal",
        "italic" => "Italic",
        other => return Err(format!("{family}: unknown style {other:?}").into()),
    };
    let m = &face.metrics;
    let bounds = m.bounds.as_ref().map_or_else(
        || "None".to_owned(),
        |b| {
            format!(
                "Some(GlyphBounds {{ y_min: {}, y_max: {} }})",
                b.y_min, b.y_max
            )
        },
    );
    let grave_top = m
        .grave_top
        .map_or_else(|| "None".to_owned(), |top| format!("Some({top})"));
    writeln!(
        out,
        "BuiltinFace {{ weight: {}, width: {}, style: FaceStyle::{style}, postscript_name: {:?}, variable: {}, \
         metrics: FontMetrics {{ units_per_em: {}, hhea: {}, typo: {}, \
         win: WinMetrics {{ ascent: {}, descent: {} }}, bounds: {bounds}, grave_top: {grave_top} }} }},",
        face.weight,
        face.width,
        face.postscript_name,
        face.variable,
        m.units_per_em,
        vmetrics(&m.hhea),
        vmetrics(&m.typo),
        m.win.ascent,
        m.win.descent,
    )?;
    Ok(())
}

/// Compiles `data/<name>-metrics.json` into `$OUT_DIR/<name>_metrics.rs`.
fn compile(name: &str) -> Result<(), Box<dyn Error>> {
    let source = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?)
        .join(format!("data/{name}-metrics.json"));
    println!("cargo::rerun-if-changed={}", source.display());
    let table: Table = serde_json::from_str(&std::fs::read_to_string(&source)?)?;
    let mut out = String::from("&[\n");
    for family in &table.families {
        writeln!(
            out,
            "BuiltinFamily {{ family: {:?}, faces: &[",
            family.family
        )?;
        for f in &family.faces {
            face(&mut out, &family.family, f)?;
        }
        out.push_str("] },\n");
    }
    out.push_str("]\n");
    let target = PathBuf::from(std::env::var("OUT_DIR")?).join(format!("{name}_metrics.rs"));
    std::fs::write(target, out)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    compile("builtin")?;
    compile("floor")
}
