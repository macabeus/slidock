//! Keynote silently substitutes a missing font, so an export can show the wrong face. An oracle passes only when the
//! export embeds every PostScript name the presentation requests.

use std::collections::BTreeSet;
use std::time::Duration;

use serde_json::Value;

use crate::error::{BenchError, Result};
use crate::oracle::pdf::OraclePdf;
use crate::renderer::docker::Renderer;

/// PostScript names of the fonts a PDF embeds, minus subset prefixes (`AAAAAB+HelveticaNeue` → `HelveticaNeue`).
pub fn parse_pdffonts(output: &str) -> Vec<String> {
    let lines: Vec<&str> = output.split('\n').collect();
    let after_rule = lines
        .iter()
        .position(|line| line.starts_with("---"))
        .map_or(0, |rule| rule + 1);
    let mut names = BTreeSet::new();
    for line in lines.iter().skip(after_rule) {
        let name = line.split(char::is_whitespace).next().unwrap_or("");
        if name.is_empty() || name == "[none]" {
            continue;
        }
        let bytes = name.as_bytes();
        let subset = bytes.len() > 7
            && bytes.get(6) == Some(&b'+')
            && bytes.iter().take(6).all(u8::is_ascii_uppercase);
        names.insert(if subset {
            name.get(7..).unwrap_or(name).to_owned()
        } else {
            name.to_owned()
        });
    }
    names.into_iter().collect()
}

pub async fn embedded_fonts(renderer: &Renderer, pdf: &OraclePdf) -> Result<Vec<String>> {
    let result = renderer
        .exec(&["pdffonts", &pdf.container_path], Duration::from_secs(60))
        .await?;
    if !result.ok() {
        return Err(BenchError::failed(format!(
            "pdffonts failed on {}: {}",
            pdf.sha256,
            result.stderr.trim()
        )));
    }
    Ok(parse_pdffonts(&result.stdout_text()))
}

fn drawn_by(container: Option<&Value>) -> Vec<String> {
    container
        .and_then(|c| c.pointer("/ext/keynote/fonts"))
        .and_then(Value::as_array)
        .map(|fonts| {
            fonts
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// The PostScript names an export of the presentation must embed, from its source-phase IR: those the printed text
/// of the shown slides, and of the layouts they show, draws (the Keynote frontend records them per container). An
/// export leaves out skipped slides and a layout's placeholders, so fonts used only there stay unchecked. A Keynote
/// presentation (one with `ext.keynote`) always carries the record, so one that draws no text expects no fonts; any
/// other presentation falls back to every name it requests.
pub fn exported_font_names(presentation: &Value) -> Vec<String> {
    let empty = Vec::new();
    let slides = presentation
        .get("slides")
        .and_then(Value::as_array)
        .unwrap_or(&empty);
    let layouts = presentation.get("layouts").and_then(Value::as_object);
    let layout_values = layouts.into_iter().flat_map(|l| l.values());
    let recorded = presentation.pointer("/ext/keynote").is_some()
        || slides
            .iter()
            .chain(layout_values)
            .any(|container| !drawn_by(Some(container)).is_empty());
    if !recorded {
        return presentation
            .get("fonts")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|fonts| fonts.values())
            .filter_map(|entry| {
                entry
                    .pointer("/request/postscriptNames")
                    .and_then(Value::as_array)
            })
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect();
    }
    let mut names = BTreeSet::new();
    for slide in slides {
        if slide.get("hidden").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        names.extend(drawn_by(Some(slide)));
        let shows_layout = slide.get("showLayout").and_then(Value::as_bool) != Some(false);
        if let (true, Some(layout)) = (shows_layout, slide.get("layout").and_then(Value::as_str)) {
            names.extend(drawn_by(layouts.and_then(|l| l.get(layout))));
        }
    }
    names.into_iter().collect()
}

/// The requested names missing from the embedded set; empty when the oracle shows every requested face.
pub fn missing_fonts(requested: &[String], embedded: &[String]) -> Vec<String> {
    let present: BTreeSet<&String> = embedded.iter().collect();
    let missing: BTreeSet<&String> = requested
        .iter()
        .filter(|name| !present.contains(name))
        .collect();
    missing.into_iter().cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pdffonts_without_subset_prefixes() {
        let output = "name                                 type              encoding         emb sub uni object ID\n\
                      ------------------------------------ ----------------- ---------------- --- --- --- ---------\n\
                      ABCDEF+HelveticaNeue                 CID TrueType      Identity-H       yes yes yes      5  0\n\
                      HelveticaNeue-Bold                   TrueType          WinAnsi          yes no  no       7  0\n\
                      [none]                               Type 3            Custom           yes no  no       9  0\n\
                      ABCDEF+HelveticaNeue                 CID TrueType      Identity-H       yes yes yes     11  0\n";
        assert_eq!(
            parse_pdffonts(output),
            ["HelveticaNeue", "HelveticaNeue-Bold"]
        );
    }

    #[test]
    fn proposes_the_fonts_of_shown_slides_and_their_layouts() {
        let ir = serde_json::json!({
            "slides": [
                {"id": "s1", "layout": "l1", "ext": {"keynote": {"fonts": ["A"]}}},
                {"id": "s2", "hidden": true, "ext": {"keynote": {"fonts": ["Hidden"]}}},
                {"id": "s3", "layout": "l2", "showLayout": false}
            ],
            "layouts": {
                "l1": {"ext": {"keynote": {"fonts": ["B"]}}},
                "l2": {"ext": {"keynote": {"fonts": ["NotShown"]}}}
            },
            "fonts": {"x": {"request": {"postscriptNames": ["Requested"]}}}
        });
        assert_eq!(exported_font_names(&ir), ["A", "B"]);
        let unrecorded = serde_json::json!({
            "slides": [{"id": "s1"}],
            "layouts": {},
            "fonts": {"x": {"request": {"postscriptNames": ["P", "Q"]}}}
        });
        assert_eq!(exported_font_names(&unrecorded), ["P", "Q"]);
        let textless_keynote = serde_json::json!({
            "ext": {"keynote": {"writer": []}},
            "slides": [{"id": "s1", "layout": "l1"}],
            "layouts": {"l1": {}},
            "fonts": {"x": {"request": {"postscriptNames": ["ThemeOnly"]}}}
        });
        assert!(exported_font_names(&textless_keynote).is_empty());
        assert_eq!(
            missing_fonts(&["A".into(), "B".into(), "A".into()], &["A".into()]),
            ["B"]
        );
    }
}
