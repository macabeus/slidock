//! Why a row scored as it did: per element (keyed by `data-el`, the IR id, with the
//! element's origin from the run's IR) the missing and extra words, moved, resized, restyled and reflowed text,
//! substituted fonts and image regions scored on their own; the heatmap and the side-by-side picture; and the printed
//! report. Positions are in ‰ of the slide's long side.

use std::collections::{BTreeMap, BTreeSet};

use serde_json::Value;
use slidock_bench_metrics::{
    Alignment, HostLibm, Libm, Rgba, SsimMap, TextWord, delta_e00, js_round, line_breaks, median,
    pair_measures, region_lab, ssim,
};
use slidock_bench_schema::api::{
    ElementFinding, FontFinding, ImageFinding, MissingLine, Moved, Reflowed, Resized, Restyled,
};
use slidock_bench_schema::results::RowRecord;

use crate::error::{BenchError, Result};
use crate::renderer::session::Measure;

/// `moved` above this distance (‰ of the long side).
pub const MOVED_PM: f64 = 1.4;
/// `resized` when the median height ratio is further than this from 1.
pub const RESIZED: f64 = 0.05;
/// `restyled` per word above this ΔE00.
pub const RESTYLED_DELTA_E: f64 = 10.0;

/// `Math.round(v · 10^d) / 10^d + 0`, for `digits` 1, 2 or 4.
fn round(value: f64, digits: u8) -> f64 {
    let scale = match digits {
        1 => 10.0,
        2 => 100.0,
        _ => 10_000.0,
    };
    js_round(value * scale) / scale + 0.0
}

/// `Math.hypot` as JavaScriptCore computes it.
fn hypot(x: f64, y: f64) -> f64 {
    <HostLibm as Libm>::hypot(x, y)
}

/// An element of the IR: its kind, name and origin line.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct IrElement {
    pub kind: String,
    pub name: Option<String>,
    pub origin: String,
}

/// One line for an origin: the source part and path, or how the node came to be.
pub fn describe_origin(origin: Option<&Value>) -> String {
    let Some(origin) = origin else {
        return String::new();
    };
    let text = |key: &str| origin.get(key).and_then(Value::as_str).unwrap_or("");
    match text("kind") {
        "source" => [text("part"), text("path")]
            .into_iter()
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join(" "),
        "synth" => format!("made by {}", text("pass")),
        "agent" => "agent-written".to_owned(),
        _ => format!(
            "fused from {}",
            origin
                .get("origins")
                .and_then(Value::as_array)
                .map_or(0, Vec::len)
        ),
    }
}

/// Visits every element of the IR (slides, layouts, masters), through groups' children and foreign fallbacks.
pub fn for_each_element(ir: &Value, visit: &mut dyn FnMut(&Value)) {
    fn walk(value: &Value, visit: &mut dyn FnMut(&Value)) {
        let Some(items) = value.as_array() else {
            return;
        };
        for element in items {
            visit(element);
            for key in ["children", "elements"] {
                if let Some(children) = element.get(key) {
                    walk(children, visit);
                }
            }
            match element.get("fallback") {
                Some(fallback @ Value::Array(_)) => walk(fallback, visit),
                Some(fallback @ Value::Object(_)) => {
                    walk(&Value::Array(vec![fallback.clone()]), visit);
                }
                _ => {}
            }
        }
    }
    if let Some(slides) = ir.get("slides").and_then(Value::as_array) {
        for slide in slides {
            if let Some(elements) = slide.get("elements") {
                walk(elements, visit);
            }
        }
    }
    for key in ["layouts", "masters"] {
        for container in ir.get(key).and_then(Value::as_object).into_iter().flatten() {
            if let Some(elements) = container.1.get("elements") {
                walk(elements, visit);
            }
        }
    }
}

/// Every element of the IR by id, the first of each id.
pub fn element_index(ir: &Value) -> BTreeMap<String, IrElement> {
    let mut out = BTreeMap::new();
    for_each_element(ir, &mut |element| {
        if let Some(id) = element.get("id").and_then(Value::as_str) {
            out.entry(id.to_owned()).or_insert_with(|| IrElement {
                kind: element
                    .get("kind")
                    .and_then(Value::as_str)
                    .unwrap_or("unknown")
                    .to_owned(),
                name: element
                    .get("name")
                    .and_then(Value::as_str)
                    .filter(|n| !n.is_empty())
                    .map(str::to_owned),
                origin: describe_origin(element.get("origin")),
            });
        }
    });
    out
}

/// Families bound to something other than an installed face: family → (binding family, binding kind).
pub fn substituted_families(ir: &Value) -> BTreeMap<String, (String, String)> {
    let mut out = BTreeMap::new();
    for entry in ir
        .get("fonts")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let entry = entry.1;
        let Some(binding) = entry.get("binding") else {
            continue;
        };
        let kind = binding.get("kind").and_then(Value::as_str).unwrap_or("");
        if kind.is_empty() || kind == "installed" {
            continue;
        }
        let family = entry
            .get("request")
            .and_then(|r| r.get("family"))
            .and_then(Value::as_str)
            .unwrap_or("");
        let bound = binding.get("family").and_then(Value::as_str).unwrap_or("");
        out.insert(family.to_owned(), (bound.to_owned(), kind.to_owned()));
    }
    out
}

/// What the findings are computed from.
pub struct ExplainInput<'a> {
    pub ir: &'a Value,
    pub measure: &'a Measure,
    pub oracle: &'a [TextWord],
    /// The oracle line of each oracle word.
    pub oracle_lines: &'a [usize],
    pub dom: &'a [TextWord],
    pub alignment: &'a Alignment,
    /// The slide size in points.
    pub slide: (f64, f64),
    /// Both rasters at the metric size.
    pub oracle_metric: &'a Rgba,
    pub render_metric: &'a Rgba,
}

fn badness(entry: &ElementFinding) -> f64 {
    f64::from(entry.missing) * 4.0
        + f64::from(entry.extra) * 2.0
        + entry.moved.map_or(0.0, |m| hypot(m.dx_pm, m.dy_pm))
        + if entry.reflowed.is_some() { 5.0 } else { 0.0 }
        + if entry.resized.is_some() { 3.0 } else { 0.0 }
        + entry.restyled.map_or(0.0, |r| f64::from(r.words))
        + entry.image.map_or(0.0, |i| (1.0 - i.ssim) * 20.0)
        + entry.fonts.len() as f64
}

/// A region of an image, clamped, at least one pixel.
pub fn crop(image: &Rgba, x0: i64, y0: i64, x1: i64, y1: i64) -> Result<Rgba> {
    let (w, h) = (image.width() as i64, image.height() as i64);
    let left = x0.max(0);
    let top = y0.max(0);
    let width = (x1.min(w) - left).max(1);
    let height = (y1.min(h) - top).max(1);
    let mut data = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        let from = (((top + y) * w + left) * 4) as usize;
        let to = from + (width * 4) as usize;
        match image.data().get(from..to) {
            Some(row) => data.extend_from_slice(row),
            None => data.extend(std::iter::repeat_n(0u8, (width * 4) as usize)),
        }
    }
    Rgba::new(width as usize, height as usize, data).map_err(|e| BenchError::failed(e.to_string()))
}

/// The per-element findings, worst first, and the missing words grouped by oracle line.
pub fn explain_elements(
    input: &ExplainInput<'_>,
) -> Result<(Vec<ElementFinding>, Vec<MissingLine>)> {
    let measure = input.measure;
    let (sw, sh) = input.slide;
    let long_side = sw.max(sh);
    let pm = |value: f64| (value / long_side) * 1000.0;
    let index = element_index(input.ir);
    let families = substituted_families(input.ir);
    let dom_lines = line_breaks(input.dom, |a, b| {
        let (first, second) = (measure.words.get(a), measure.words.get(b));
        first.map(|w| (&w.el, w.text_box)) == second.map(|w| (&w.el, w.text_box))
    });
    let scale = input.oracle_metric.width() as f64 / sw;
    let mut by_element: BTreeMap<String, ElementFinding> = BTreeMap::new();
    let finding = |by_element: &mut BTreeMap<String, ElementFinding>, el: &str| {
        if !by_element.contains_key(el) {
            let element = index.get(el).cloned().unwrap_or_else(|| IrElement {
                kind: "unknown".into(),
                ..IrElement::default()
            });
            by_element.insert(
                el.to_owned(),
                ElementFinding {
                    el: el.to_owned(),
                    kind: element.kind,
                    name: element.name,
                    origin: element.origin,
                    words: 0,
                    moved: None,
                    resized: None,
                    restyled: None,
                    reflowed: None,
                    fonts: Vec::new(),
                    image: None,
                    missing: 0,
                    extra: 0,
                },
            );
        }
    };
    // Pairs by element, in first-seen order as a JavaScript Map keeps them.
    let mut order: Vec<String> = Vec::new();
    let mut pairs_by_element: BTreeMap<String, Vec<(usize, usize)>> = BTreeMap::new();
    for &(i, j) in &input.alignment.pairs {
        let el = measure
            .words
            .get(j)
            .and_then(|w| w.el.clone())
            .unwrap_or_else(|| "unattributed".to_owned());
        if !pairs_by_element.contains_key(&el) {
            order.push(el.clone());
        }
        pairs_by_element.entry(el).or_default().push((i, j));
    }
    for el in &order {
        let Some(pairs) = pairs_by_element.get(el) else {
            continue;
        };
        finding(&mut by_element, el);
        let measures = pair_measures::<HostLibm>(input.oracle, input.dom, pairs, long_side)
            .map_err(|e| BenchError::failed(e.to_string()))?;
        let dx = median(&measures.iter().map(|m| m.dx_pm).collect::<Vec<_>>());
        let dy = median(&measures.iter().map(|m| m.dy_pm).collect::<Vec<_>>());
        let ratio = median(&measures.iter().map(|m| m.height_ratio).collect::<Vec<_>>());
        let deltas: Vec<f64> = pairs
            .iter()
            .map(|&(i, j)| {
                let region = |word: &TextWord, image: &Rgba| {
                    region_lab::<HostLibm>(
                        image,
                        word.rect.x * scale,
                        word.rect.y * scale,
                        (word.rect.x + word.rect.w) * scale,
                        (word.rect.y + word.rect.h) * scale,
                    )
                };
                let a = input
                    .oracle
                    .get(i)
                    .and_then(|w| region(w, input.oracle_metric));
                let b = input
                    .dom
                    .get(j)
                    .and_then(|w| region(w, input.render_metric));
                match (a, b) {
                    (Some(a), Some(b)) => delta_e00::<HostLibm>(a, b),
                    _ => 0.0,
                }
            })
            .collect();
        let oracle_lines: BTreeSet<usize> = pairs
            .iter()
            .filter_map(|&(i, _)| input.oracle_lines.get(i).copied())
            .collect();
        let render_lines: BTreeSet<usize> = pairs
            .iter()
            .filter_map(|&(_, j)| dom_lines.get(j).copied())
            .collect();
        if let Some(entry) = by_element.get_mut(el) {
            entry.words = pairs.len() as u32;
            if hypot(dx, dy) > MOVED_PM {
                entry.moved = Some(Moved {
                    dx_pm: round(dx, 1),
                    dy_pm: round(dy, 1),
                });
            }
            if (ratio - 1.0).abs() > RESIZED {
                entry.resized = Some(Resized {
                    height_ratio: round(ratio, 2),
                });
            }
            let restyled: Vec<f64> = deltas
                .iter()
                .copied()
                .filter(|d| *d > RESTYLED_DELTA_E)
                .collect();
            if !restyled.is_empty() {
                entry.restyled = Some(Restyled {
                    words: restyled.len() as u32,
                    max_delta_e: round(
                        restyled.iter().copied().fold(f64::NEG_INFINITY, f64::max),
                        1,
                    ),
                });
            }
            if oracle_lines.len() != render_lines.len() {
                entry.reflowed = Some(Reflowed {
                    oracle_lines: oracle_lines.len() as u32,
                    render_lines: render_lines.len() as u32,
                });
            }
        }
    }
    for word in &measure.words {
        let (Some(el), Some((binding, kind))) = (&word.el, families.get(&word.family)) else {
            continue;
        };
        finding(&mut by_element, el);
        if let Some(entry) = by_element.get_mut(el)
            && !entry.fonts.iter().any(|f| f.family == word.family)
        {
            entry.fonts.push(FontFinding {
                family: word.family.clone(),
                binding: binding.clone(),
                kind: kind.clone(),
            });
        }
    }
    for &j in &input.alignment.extra {
        if let Some(el) = measure.words.get(j).and_then(|w| w.el.clone()) {
            finding(&mut by_element, &el);
            if let Some(entry) = by_element.get_mut(&el) {
                entry.extra += 1;
            }
        }
    }
    for image in &measure.images {
        let Some(el) = &image.el else { continue };
        let [x0, y0, x1, y1] = [
            image.x * scale,
            image.y * scale,
            (image.x + image.w) * scale,
            (image.y + image.h) * scale,
        ]
        .map(|v| js_round(v) as i64);
        if x1 - x0 < 8 || y1 - y0 < 8 {
            continue;
        }
        let a = crop(input.oracle_metric, x0, y0, x1, y1)?;
        let b = crop(input.render_metric, x0, y0, x1, y1)?;
        let structure = ssim(&a, &b).map_err(|e| BenchError::failed(e.to_string()))?;
        let (fx0, fy0, fx1, fy1) = (x0 as f64, y0 as f64, x1 as f64, y1 as f64);
        let la = region_lab::<HostLibm>(input.oracle_metric, fx0, fy0, fx1, fy1);
        let lb = region_lab::<HostLibm>(input.render_metric, fx0, fy0, fx1, fy1);
        finding(&mut by_element, el);
        if let Some(entry) = by_element.get_mut(el) {
            entry.image = Some(ImageFinding {
                ssim: round(structure.mean, 4),
                delta_e: match (la, lb) {
                    (Some(a), Some(b)) => round(delta_e00::<HostLibm>(a, b), 1),
                    _ => 0.0,
                },
            });
        }
    }

    // Missing words, grouped by oracle line (in first-seen order) and attributed to the nearest element.
    let mut line_order: Vec<usize> = Vec::new();
    let mut missing_by_line: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
    for &i in &input.alignment.missing {
        let line = input.oracle_lines.get(i).copied().unwrap_or(usize::MAX);
        if !missing_by_line.contains_key(&line) {
            line_order.push(line);
        }
        missing_by_line.entry(line).or_default().push(i);
    }
    let mut missing = Vec::new();
    for line in line_order {
        let Some(words) = missing_by_line.get(&line) else {
            continue;
        };
        let Some(first) = words.first().and_then(|&i| input.oracle.get(i)) else {
            continue;
        };
        let (cx, cy) = (
            first.rect.x + first.rect.w / 2.0,
            first.rect.y + first.rect.h / 2.0,
        );
        let mut best: Option<&str> = None;
        let mut best_distance = f64::INFINITY;
        for b in &measure.elements {
            let dx = (b.x - cx).max(0.0).max(cx - (b.x + b.w));
            let dy = (b.y - cy).max(0.0).max(cy - (b.y + b.h));
            let distance = hypot(dx, dy);
            if distance < best_distance {
                best = Some(&b.el);
                best_distance = distance;
            }
        }
        if let Some(el) = best {
            finding(&mut by_element, el);
            if let Some(entry) = by_element.get_mut(el) {
                entry.missing += words.len() as u32;
            }
        }
        missing.push(MissingLine {
            text: words
                .iter()
                .filter_map(|&i| input.oracle.get(i).map(|w| w.text.as_str()))
                .collect::<Vec<_>>()
                .join(" "),
            x_pm: round(pm(first.rect.x), 1),
            y_pm: round(pm(first.rect.y), 1),
            el: best.map(str::to_owned),
        });
    }

    let mut elements: Vec<ElementFinding> = by_element
        .into_values()
        .filter(|entry| badness(entry) > 0.0)
        .collect();
    elements.sort_by(|a, b| {
        badness(b)
            .partial_cmp(&badness(a))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| slidock_canon::utf16::cmp(&a.el, &b.el))
    });
    Ok((elements, missing))
}

const WINDOW: usize = 8;
const STRIDE: usize = 4;
const GAP: usize = 8;

/// Each pixel takes the worst (1 − SSIM) of the windows covering it, drawn as red over the faded `base`.
pub fn heatmap(base: &Rgba, map: &SsimMap) -> Result<Rgba> {
    let (w, h) = (base.width(), base.height());
    let mut worst = vec![0f32; w * h];
    for row in 0..map.rows {
        for column in 0..map.columns {
            let value = map
                .values
                .get(row * map.columns + column)
                .copied()
                .unwrap_or(1.0);
            let bad = (1.0 - value).clamp(0.0, 1.0);
            for y in row * STRIDE..(row * STRIDE + WINDOW).min(h) {
                for x in column * STRIDE..(column * STRIDE + WINDOW).min(w) {
                    if let Some(p) = worst.get_mut(y * w + x) {
                        *p = p.max(bad);
                    }
                }
            }
        }
    }
    let mut data = vec![0u8; w * h * 4];
    for (p, bad) in worst.iter().enumerate() {
        let i = p * 4;
        let px = |k: usize| f64::from(base.data().get(i + k).copied().unwrap_or(255));
        let grey = 0.2126 * px(0) + 0.7152 * px(1) + 0.0722 * px(2);
        let faded = 255.0 - (255.0 - grey) * 0.25;
        let bad = f64::from(*bad);
        if let Some(out) = data.get_mut(i..i + 4) {
            out.copy_from_slice(&[
                js_round(faded + (255.0 - faded) * bad) as u8,
                js_round(faded * (1.0 - bad)) as u8,
                js_round(faded * (1.0 - bad)) as u8,
                255,
            ]);
        }
    }
    Rgba::new(w, h, data).map_err(|e| BenchError::failed(e.to_string()))
}

/// Images left to right with 8 px white gaps.
pub fn side_by_side(images: &[&Rgba]) -> Result<Rgba> {
    let width =
        images.iter().map(|i| i.width()).sum::<usize>() + GAP * images.len().saturating_sub(1);
    let height = images.iter().map(|i| i.height()).max().unwrap_or(1);
    let mut out = Rgba::blank(width, height, 255)
        .map_err(|e| BenchError::failed(e.to_string()))?
        .into_data();
    let mut left = 0;
    for image in images {
        let row_bytes = image.width() * 4;
        for y in 0..image.height() {
            let from = y * row_bytes;
            let to = (y * width + left) * 4;
            if let (Some(src), Some(dst)) = (
                image.data().get(from..from + row_bytes),
                out.get_mut(to..to + row_bytes),
            ) {
                dst.copy_from_slice(src);
            }
        }
        left += image.width() + GAP;
    }
    Rgba::new(width, height, out).map_err(|e| BenchError::failed(e.to_string()))
}

/// Encodes an image as an RGBA PNG.
pub fn encode_png(image: &Rgba) -> Result<Vec<u8>> {
    let mut png = Vec::new();
    {
        let mut encoder = png::Encoder::new(&mut png, image.width() as u32, image.height() as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| BenchError::failed(format!("png: {e}")))?;
        writer
            .write_image_data(image.data())
            .map_err(|e| BenchError::failed(format!("png: {e}")))?;
    }
    Ok(png)
}

fn signed(value: f64) -> String {
    let sign = if value >= 0.0 { '+' } else { '−' };
    format!("{sign}{}", slidock_canon::to_js_string(value.abs()))
}

/// The printed report: the row's scores, the diagnostics that apply, each element worst first, the missing oracle
/// lines and the extra rendered words.
pub fn explain_text(
    row: &str,
    record: &RowRecord,
    diagnostics: &[(String, u32)],
    elements: &[ElementFinding],
    missing: &[MissingLine],
) -> String {
    let n = slidock_canon::to_js_string;
    let mut lines = Vec::new();
    let outcome = record
        .outcome
        .classifier()
        .and_then(|o| serde_json::to_value(o).ok())
        .and_then(|v| v.as_str().map(str::to_owned))
        .unwrap_or_default();
    let head = match (&record.scores, &record.declined, record.reason) {
        (Some(s), _, _) => format!(
            "ssim {} (p1 {})  ΔE {}  text recall {}{}",
            n(s.ssim.mean),
            n(s.ssim.p1),
            n(s.color.mean),
            n(s.text.recall),
            s.text
                .p95_pm
                .map_or_else(String::new, |p| format!(" p95 {}‰", n(p)))
        ),
        (None, Some(d), _) => format!(
            "{}{}",
            d.code,
            d.via
                .as_ref()
                .map_or_else(String::new, |v| format!(" via {v}"))
        ),
        (None, None, Some(reason)) => serde_json::to_value(reason)
            .ok()
            .and_then(|v| v.as_str().map(str::to_owned))
            .unwrap_or_default(),
        _ => String::new(),
    };
    lines.push(format!("{row}  {outcome}  {head}"));
    if !diagnostics.is_empty() {
        let parts: Vec<String> = diagnostics
            .iter()
            .map(|(code, count)| {
                let name =
                    slidock_ir::diagnostics::DiagnosticCode::lookup(code).map_or("", |c| c.name());
                format!("{code}×{count} {name}").trim_end().to_owned()
            })
            .collect();
        lines.push(format!("diagnostics: {}", parts.join("; ")));
    }
    lines.push("elements (worst first):".to_owned());
    for entry in elements {
        let name = entry
            .name
            .as_ref()
            .map_or_else(String::new, |n| format!("  \"{n}\""));
        let origin = if entry.origin.is_empty() {
            String::new()
        } else {
            format!("  {}", entry.origin)
        };
        lines.push(format!("  {}  {}{name}{origin}", entry.el, entry.kind));
        if let Some(m) = entry.moved {
            lines.push(format!(
                "    moved     Δ({}‰, {}‰) over {} words",
                signed(m.dx_pm),
                signed(m.dy_pm),
                entry.words
            ));
        }
        if let Some(r) = entry.resized {
            lines.push(format!("    resized   word height ×{}", n(r.height_ratio)));
        }
        if let Some(r) = entry.restyled {
            lines.push(format!(
                "    restyled  {} words, ΔE up to {}",
                r.words,
                n(r.max_delta_e)
            ));
        }
        if let Some(r) = entry.reflowed {
            lines.push(format!(
                "    reflowed  oracle {} lines → {} lines",
                r.oracle_lines, r.render_lines
            ));
        }
        for font in &entry.fonts {
            lines.push(format!(
                "    font      {} → {} {}",
                font.family, font.kind, font.binding
            ));
        }
        if let Some(i) = entry.image {
            lines.push(format!(
                "    image     region ssim {}, ΔE {}",
                n(i.ssim),
                n(i.delta_e)
            ));
        }
        if entry.missing > 0 {
            lines.push(format!(
                "    missing   {} oracle words near it",
                entry.missing
            ));
        }
        if entry.extra > 0 {
            lines.push(format!("    extra     {} rendered words", entry.extra));
        }
    }
    for line in missing {
        let near = line
            .el
            .as_ref()
            .map_or_else(|| " unattributed".to_owned(), |el| format!(" near {el}"));
        lines.push(format!(
            "missing (oracle only): \"{}\" line at ({}‰, {}‰){near}",
            line.text,
            n(line.x_pm),
            n(line.y_pm)
        ));
    }
    for entry in elements.iter().filter(|e| e.extra > 0) {
        lines.push(format!(
            "extra (render only): {} {}, {} words",
            entry.el, entry.kind, entry.extra
        ));
    }
    format!("{}\n", lines.join("\n"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn origins_read_as_one_line() {
        assert_eq!(
            describe_origin(Some(
                &json!({"kind": "source", "part": "ppt/slides/slide1.xml", "path": "/p:sld"})
            )),
            "ppt/slides/slide1.xml /p:sld"
        );
        assert_eq!(
            describe_origin(Some(&json!({"kind": "synth", "pass": "layout"}))),
            "made by layout"
        );
        assert_eq!(
            describe_origin(Some(&json!({"kind": "fused", "origins": [1, 2]}))),
            "fused from 2"
        );
        assert_eq!(describe_origin(None), "");
    }

    #[test]
    fn the_index_walks_groups_and_layouts() {
        let ir = json!({
            "slides": [{"id": "s", "elements": [{"id": "g", "kind": "group", "children": [{"id": "t", "kind": "text", "name": "Title"}]}]}],
            "layouts": {"l": {"elements": [{"id": "p", "kind": "picture"}]}},
            "masters": {}
        });
        let index = element_index(&ir);
        assert_eq!(index.keys().collect::<Vec<_>>(), ["g", "p", "t"]);
        assert_eq!(index["t"].name.as_deref(), Some("Title"));
    }

    #[test]
    fn the_heatmap_reddens_low_ssim_windows() {
        let base = Rgba::blank(8, 8, 255).unwrap();
        let good = SsimMap {
            columns: 1,
            rows: 1,
            values: vec![1.0],
        };
        let bad = SsimMap {
            columns: 1,
            rows: 1,
            values: vec![0.0],
        };
        assert_eq!(
            &heatmap(&base, &good).unwrap().data()[..4],
            &[255, 255, 255, 255]
        );
        assert_eq!(
            &heatmap(&base, &bad).unwrap().data()[..4],
            &[255, 0, 0, 255]
        );
    }

    #[test]
    fn side_by_side_leaves_white_gaps() {
        let black = Rgba::blank(2, 2, 0).unwrap();
        let joined = side_by_side(&[&black, &black]).unwrap();
        assert_eq!((joined.width(), joined.height()), (12, 2));
        assert_eq!(&joined.data()[8..12], &[255, 255, 255, 255]);
        assert_eq!(&joined.data()[40..44], &[0, 0, 0, 255]);
    }
}
