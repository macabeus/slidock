//! Oracle words with their boxes, from `pdftotext -bbox-layout` inside the renderer container (`renderer::docker`),
//! cached under `text/<key>/<pdf sha256>/` by a key that names poppler, the renderer platform and this module's
//! source; [`PageText::to_metrics`] hands them to the text metric (`slidock_bench_metrics::oracle_words`). Its output
//! is XHTML with a public DOCTYPE, so this module parses it as XML and refuses only an internal DTD subset.

use std::path::Path;
use std::time::Duration;

use quick_xml::events::Event;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use slidock_bench_metrics::{OracleBlock, OracleLine, OracleText, OracleWord};

use crate::cache::{read_json, write_json};
use crate::error::{BenchError, Result};
use crate::hash::{sha256, sha256_json};
use crate::oracle::pdf::OraclePdf;
use crate::renderer::docker::{PLATFORM_DIGEST, Renderer};

/// This module's source: an edited extractor reads only its own output.
const CODE: &str = include_str!("text_boxes.rs");

/// A number as JSON writes it: NaN as `null`, and `null` read back as NaN.
mod nan_null {
    use super::*;

    pub fn serialize<S: Serializer>(
        value: &f64,
        serializer: S,
    ) -> std::result::Result<S::Ok, S::Error> {
        if value.is_finite() {
            serializer.serialize_f64(*value)
        } else {
            serializer.serialize_none()
        }
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> std::result::Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or(f64::NAN))
    }
}

/// A word's box in points, y down from the page's top-left corner.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Word {
    pub text: String,
    #[serde(with = "nan_null")]
    pub x_min: f64,
    #[serde(with = "nan_null")]
    pub y_min: f64,
    #[serde(with = "nan_null")]
    pub x_max: f64,
    #[serde(with = "nan_null")]
    pub y_max: f64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Line {
    pub words: Vec<Word>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Block {
    pub lines: Vec<Line>,
}

/// One page of words.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct PageText {
    #[serde(with = "nan_null")]
    pub width: f64,
    #[serde(with = "nan_null")]
    pub height: f64,
    pub blocks: Vec<Block>,
}

impl PageText {
    /// The page as the metrics read it.
    pub fn to_metrics(&self) -> OracleText {
        OracleText {
            width: self.width,
            height: self.height,
            blocks: self
                .blocks
                .iter()
                .map(|block| OracleBlock {
                    lines: block
                        .lines
                        .iter()
                        .map(|line| OracleLine {
                            words: line
                                .words
                                .iter()
                                .map(|w| OracleWord {
                                    text: w.text.clone(),
                                    x_min: w.x_min,
                                    y_min: w.y_min,
                                    x_max: w.x_max,
                                    y_max: w.y_max,
                                })
                                .collect(),
                        })
                        .collect(),
                })
                .collect(),
        }
    }
}

fn number_attribute(tag: &quick_xml::events::BytesStart<'_>, name: &str) -> f64 {
    tag.attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| {
            a.normalized_value(quick_xml::XmlVersion::Implicit1_0)
                .ok()
                .map(|v| slidock_canon::js_number(&v))
        })
        .unwrap_or(f64::NAN)
}

/// The character an entity reference stands for: a character reference or one of XML's five.
fn entity(reference: &quick_xml::events::BytesRef<'_>) -> Result<String> {
    if let Some(c) = reference
        .resolve_char_ref()
        .map_err(|error| BenchError::failed(format!("pdftotext output: {error}")))?
    {
        return Ok(c.to_string());
    }
    let name = reference.xml10_content();
    Ok(match name.as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        other => {
            return Err(BenchError::failed(format!(
                "pdftotext output names the entity &{other};"
            )));
        }
    }
    .to_owned())
}

/// Parses one page of `pdftotext -bbox-layout` XHTML.
pub fn parse_bbox_layout(xhtml: &str) -> Result<PageText> {
    let mut reader = quick_xml::Reader::from_str(xhtml);
    let mut page = PageText::default();
    let mut word: Option<Word> = None;
    let invalid =
        |error: quick_xml::Error| BenchError::failed(format!("pdftotext output: {error}"));
    loop {
        let event = reader.read_event().map_err(invalid)?;
        match event {
            Event::DocType(doctype) => {
                if doctype.into_inner().contains('[') {
                    return Err(BenchError::failed(
                        "pdftotext output with an internal DTD subset",
                    ));
                }
            }
            Event::Start(tag) | Event::Empty(tag) => match tag.name().as_ref() {
                "page" => {
                    page.width = number_attribute(&tag, "width");
                    page.height = number_attribute(&tag, "height");
                }
                "block" => page.blocks.push(Block::default()),
                "line" => {
                    if let Some(block) = page.blocks.last_mut() {
                        block.lines.push(Line::default());
                    }
                }
                "word" => {
                    word = Some(Word {
                        text: String::new(),
                        x_min: number_attribute(&tag, "xMin"),
                        y_min: number_attribute(&tag, "yMin"),
                        x_max: number_attribute(&tag, "xMax"),
                        y_max: number_attribute(&tag, "yMax"),
                    });
                }
                _ => {}
            },
            Event::Text(text) => {
                if let Some(word) = word.as_mut() {
                    word.text.push_str(&text.xml10_content());
                }
            }
            Event::CData(text) => {
                if let Some(word) = word.as_mut() {
                    word.text.push_str(&text.into_inner());
                }
            }
            Event::GeneralRef(reference) => {
                if let Some(word) = word.as_mut() {
                    word.text.push_str(&entity(&reference)?);
                }
            }
            Event::End(tag) => {
                if tag.name().as_ref() == "word"
                    && let Some(done) = word.take()
                    && let Some(line) = page.blocks.last_mut().and_then(|b| b.lines.last_mut())
                {
                    line.words.push(done);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(page)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TextKey<'a> {
    poppler: &'a str,
    platform_digest: &'a str,
    code: String,
}

pub fn text_key(poppler: &str) -> Result<String> {
    sha256_json(&TextKey {
        poppler,
        platform_digest: PLATFORM_DIGEST,
        code: sha256(CODE),
    })
}

/// The words of one page (1-based), extracted once per key.
pub async fn page_text(
    renderer: &Renderer,
    pdf: &OraclePdf,
    page: u32,
    key: &str,
    no_cache: bool,
) -> Result<PageText> {
    let path = renderer
        .work
        .join("text")
        .join(key)
        .join(&pdf.sha256)
        .join(format!("{page}.json"));
    if !no_cache && let Some(text) = read_json::<PageText>(&path)? {
        return Ok(text);
    }
    let page_arg = page.to_string();
    let result = renderer
        .exec(
            &[
                "pdftotext",
                "-bbox-layout",
                "-f",
                &page_arg,
                "-l",
                &page_arg,
                &pdf.container_path,
                "-",
            ],
            Duration::from_secs(60),
        )
        .await?;
    if !result.ok() {
        return Err(BenchError::failed(format!(
            "pdftotext page {page} of {}: {}",
            pdf.sha256,
            result.stderr.trim()
        )));
    }
    let text = parse_bbox_layout(&result.stdout_text())?;
    write_json(Path::new(&path), &text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    const PAGE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE html PUBLIC "-//W3C//DTD XHTML 1.0 Transitional//EN" "http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd">
<html xmlns="http://www.w3.org/1999/xhtml">
<head><title></title></head>
<body>
<doc>
  <page width="960.000000" height="540.000000">
    <flow>
      <block xMin="10" yMin="20" xMax="100" yMax="40">
        <line xMin="10" yMin="20" xMax="100" yMax="40">
          <word xMin="10.500000" yMin="20.000000" xMax="50.000000" yMax="40.000000">Tom&amp;Jerry</word>
          <word xMin="55.000000" yMin="20.000000" xMax="100.000000" yMax="40.000000">&#233;t&#xE9;</word>
        </line>
      </block>
    </flow>
  </page>
</doc>
</body>
</html>
"#;

    #[test]
    fn reads_pages_blocks_lines_and_words() {
        let page = parse_bbox_layout(PAGE).unwrap();
        assert_eq!((page.width, page.height), (960.0, 540.0));
        let words = &page.blocks[0].lines[0].words;
        assert_eq!(words.len(), 2);
        assert_eq!(words[0].text, "Tom&Jerry");
        assert_eq!(words[0].x_min, 10.5);
        assert_eq!(words[1].text, "été");
    }

    #[test]
    fn refuses_an_internal_dtd_subset() {
        let hostile = PAGE.replace(
            "\"http://www.w3.org/TR/xhtml1/DTD/xhtml1-transitional.dtd\">",
            "\"x\" [<!ENTITY a \"b\">]>",
        );
        assert!(parse_bbox_layout(&hostile).is_err());
    }

    #[test]
    fn missing_sizes_survive_the_cache_as_null() {
        let page = PageText {
            width: f64::NAN,
            ..PageText::default()
        };
        let text = serde_json::to_string(&page).unwrap();
        assert!(text.contains("\"width\":null"), "{text}");
        let back: PageText = serde_json::from_str(&text).unwrap();
        assert!(back.width.is_nan());
    }
}
