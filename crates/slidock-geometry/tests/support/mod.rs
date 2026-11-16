//! Reading the committed vectors (`tests/data/*.jsonl`) and printing paths the way the generator does. The vectors
//! were generated once from seeded synthetic inputs (`$SLIDOCK_PRIVATE/tools/color-geometry-vectors.ts`); the line
//! ends' digests that Keynote's rule moved (an end on a curve or past its last segment, a path end), and those where
//! two ends' bodies overlap (each end placed on the whole path), were re-recorded from slidock on the same inputs.
//! Numbers are `String(n)` with `-0` kept; paths are hashed as text, `fs` flags then ` Op,x,y…` per segment, paths
//! joined by `|`, and the digest is the first 16 hex digits of the SHA-256.

#![allow(dead_code, reason = "each test file uses some of the helpers")]
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use indexmap::IndexMap;
use serde_json::Value;
use slidock_geometry::{Segment, ShapePath};
use slidock_ir::geometry::Seg;

pub const PRESETS: &str = include_str!("../data/presets.jsonl");
pub const FORMULAS: &str = include_str!("../data/formulas.jsonl");
pub const BUILTINS: &str = include_str!("../data/builtins.jsonl");
pub const ARCS: &str = include_str!("../data/arcs.jsonl");
pub const LINE_ENDS: &str = include_str!("../data/line-ends.jsonl");

pub fn rows(text: &str) -> Vec<Value> {
    text.lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

/// A number as the generator prints it: `String(n)`, or `-0`.
pub fn num(text: &Value) -> f64 {
    let text = text.as_str().unwrap();
    match text {
        "-0" => -0.0,
        "NaN" => f64::NAN,
        "Infinity" => f64::INFINITY,
        "-Infinity" => f64::NEG_INFINITY,
        _ => text.parse().unwrap(),
    }
}

/// `String(n)`, with `-0` kept.
pub fn js(n: f64) -> String {
    if n == 0.0 && n.is_sign_negative() {
        "-0".to_owned()
    } else {
        slidock_canon::to_js_string(n)
    }
}

/// Bit equality, with every NaN equal.
pub fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits() || (a.is_nan() && b.is_nan())
}

pub fn paths_text(paths: &[ShapePath]) -> String {
    paths
        .iter()
        .map(|path| {
            let mut text = format!("{}{}", u8::from(path.fill), u8::from(path.stroke));
            for segment in &path.segments {
                text.push(' ');
                text.push_str(segment.command());
                for x in segment.coords() {
                    text.push(',');
                    text.push_str(&js(x));
                }
            }
            text
        })
        .collect::<Vec<_>>()
        .join("|")
}

pub fn digest(text: &str) -> String {
    slidock_canon::sha256_hex(text)[..16].to_owned()
}

/// An adjustment record: `[[name, value], …]`.
pub fn adjust(value: &Value) -> IndexMap<String, f64> {
    value
        .as_array()
        .unwrap()
        .iter()
        .map(|pair| (pair[0].as_str().unwrap().to_owned(), num(&pair[1])))
        .collect()
}

/// A segment written as JSON: `["M", 0, 0]`, with NaN as `null` (`JSON.stringify`).
pub fn segment(value: &Value) -> Segment {
    let items = value.as_array().unwrap();
    let x: Vec<f64> = items[1..]
        .iter()
        .map(|v| {
            if v.is_null() {
                f64::NAN
            } else {
                v.as_f64().unwrap()
            }
        })
        .collect();
    match items[0].as_str().unwrap() {
        "M" => Seg::M(x[0], x[1]),
        "L" => Seg::L(x[0], x[1]),
        "Q" => Seg::Q(x[0], x[1], x[2], x[3]),
        "C" => Seg::C(x[0], x[1], x[2], x[3], x[4], x[5]),
        "Z" => Seg::Z,
        other => panic!("unknown command {other}"),
    }
}

/// Segments rounded to 1/1000, for readable expectations.
pub fn rounded(segments: &[Segment]) -> Vec<Seg<f64>> {
    let round = |x: f64| slidock_canon::js_round(x * 1000.0) / 1000.0 + 0.0;
    segments
        .iter()
        .map(|segment| slidock_geometry::map_segment(*segment, round))
        .collect()
}
