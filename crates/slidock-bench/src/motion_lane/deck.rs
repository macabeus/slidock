//! A bundled deck's motion, read back from its `index.html` (the deck data, `slidock-deck@1`), and the timeline and
//! transitions of the compile's IR (`presentation-ir.json` of `--emit ir --native-motion`, the deck path's motion) to
//! compare it with.
//! Values are kept as written: the lane checks what the player makes of them, it does not repair them.

use std::path::Path;

use serde::Deserialize;
use serde_json::Value;
use slidock_bench_schema::Sha256;

use crate::error::{BenchError, Result};

#[derive(Clone, Copy, Debug, PartialEq, Deserialize)]
pub struct DeckSize {
    pub w: f64,
    pub h: f64,
}

/// What a step builds: an element, a paragraph range of it, or its background.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Target {
    pub element: String,
    #[serde(default)]
    pub paragraphs: Option<(u32, u32)>,
    #[serde(default)]
    pub background: Option<bool>,
}

impl Target {
    pub fn is_background(&self) -> bool {
        self.background == Some(true)
    }

    /// `<element>`, `<element> ¶<first>–<last>` or `<element> background`.
    pub fn label(&self) -> String {
        if self.is_background() {
            return format!("{} background", self.element);
        }
        match self.paragraphs {
            Some((first, last)) => format!("{} ¶{first}–{last}", self.element),
            None => self.element.clone(),
        }
    }
}

/// One build step as the deck data carries it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckStep {
    pub target: Target,
    pub trigger: String,
    pub kind: String,
    #[serde(default)]
    pub delivery: Option<String>,
    #[serde(default)]
    pub delay_ms: Value,
    #[serde(default)]
    pub duration_ms: Value,
    #[serde(default)]
    pub native: String,
    pub fallback: String,
    /// The vocabulary effect; absent when the compiler named none.
    #[serde(default)]
    pub effect: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub offset_ms: Option<Value>,
    #[serde(default)]
    pub repeat: Option<Value>,
    #[serde(default)]
    pub auto_reverse: Option<bool>,
    #[serde(default)]
    pub stagger: Option<Value>,
    #[serde(default)]
    pub after: Option<Value>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckAdvance {
    #[serde(default)]
    pub on_click: Option<bool>,
    #[serde(default)]
    pub after_ms: Option<Value>,
}

/// The transition into a slide.
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckTransition {
    #[serde(default)]
    pub native: String,
    pub fallback: String,
    #[serde(default)]
    pub duration_ms: Value,
    #[serde(default)]
    pub effect: Option<String>,
    #[serde(default)]
    pub params: Option<Value>,
    #[serde(default)]
    pub advance: Option<DeckAdvance>,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeckSlide {
    pub id: String,
    pub hidden: bool,
    #[serde(default)]
    pub timeline: Vec<DeckStep>,
    #[serde(default)]
    pub transition: Option<DeckTransition>,
    #[serde(default)]
    pub match_keys: Option<Value>,
}

impl DeckSlide {
    /// Whether the motion lane plays it: a timeline or a transition.
    pub fn has_motion(&self) -> bool {
        !self.timeline.is_empty() || self.transition.is_some()
    }

    /// Its click groups: the `click` steps.
    pub fn clicks(&self) -> u32 {
        self.timeline
            .iter()
            .filter(|step| step.trigger == "click")
            .count() as u32
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct DeckData {
    size: DeckSize,
    slides: Vec<DeckSlide>,
}

/// A bundled deck's motion.
#[derive(Clone, Debug, PartialEq)]
pub struct MotionDeck {
    pub size: DeckSize,
    pub slides: Vec<DeckSlide>,
    /// The sha256 of its `index.html`.
    pub index: Sha256,
}

impl MotionDeck {
    /// The indexes of the slides a presenting player shows, in order.
    pub fn shown(&self) -> Vec<usize> {
        self.slides
            .iter()
            .enumerate()
            .filter(|(_, slide)| !slide.hidden)
            .map(|(index, _)| index)
            .collect()
    }
}

/// The text between `open` and the next `close`.
pub fn between<'a>(text: &'a str, open: &str, close: &str) -> Option<&'a str> {
    let start = text.find(open)? + open.len();
    let end = text.get(start..)?.find(close)? + start;
    text.get(start..end)
}

/// The deck data of an `index.html`.
pub fn parse_deck(html: &str) -> std::result::Result<(DeckSize, Vec<DeckSlide>), String> {
    let json = between(
        html,
        "<script type=\"application/json\" id=\"sd-deck\">",
        "</script>",
    )
    .ok_or("no deck data")?;
    let data: DeckData =
        serde_json::from_str(json).map_err(|error| format!("deck data: {error}"))?;
    Ok((data.size, data.slides))
}

/// Reads a bundled deck folder's motion.
pub fn read_deck(dir: &Path) -> Result<MotionDeck> {
    let path = dir.join("index.html");
    let bytes = std::fs::read(&path).map_err(BenchError::io(&path))?;
    let html = String::from_utf8_lossy(&bytes);
    let (size, slides) = parse_deck(&html)
        .map_err(|why| BenchError::failed(format!("{}: {why}", path.display())))?;
    Ok(MotionDeck {
        size,
        slides,
        index: Sha256::of(&bytes),
    })
}

/// A step of the IR's timeline (`slide.timeline`).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrStep {
    pub target: Target,
    pub trigger: String,
    pub kind: String,
    #[serde(default)]
    pub delivery: Option<String>,
    #[serde(default)]
    pub delay_ms: Value,
    #[serde(default)]
    pub duration_ms: Value,
    pub effect: IrEffect,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct IrEffect {
    pub id: String,
    pub fallback: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IrTransition {
    #[serde(default)]
    pub duration_ms: Value,
    pub effect: IrEffect,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct IrSlide {
    pub id: String,
    /// Skipped in a show.
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub timeline: Vec<IrStep>,
    #[serde(default)]
    pub transition: Option<IrTransition>,
}

#[derive(Deserialize)]
struct IrDocument {
    slides: Vec<IrSlide>,
}

/// The slides of a `presentation-ir.json`.
pub fn read_ir(path: &Path) -> Result<Vec<IrSlide>> {
    let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
    let document: IrDocument = serde_json::from_str(&text)
        .map_err(|error| BenchError::failed(format!("{}: {error}", path.display())))?;
    Ok(document.slides)
}

/// A number as JSON writes it, or `None`.
pub fn number(value: &Value) -> Option<f64> {
    value.as_f64()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_deck_data_reads_with_its_motion() {
        let html = r#"<html><head><script type="application/json" id="sd-deck">{"format":"slidock-deck@1","size":{"w":960,"h":540},"slides":[{"id":"a","hidden":false,"timeline":[{"target":{"element":"a.2","paragraphs":[0,1]},"trigger":"click","kind":"entrance","delivery":"byParagraph","delayMs":0,"durationMs":500,"native":"pptx:entr.2/4","fallback":"dissolve","effect":"flyIn","params":{"dir":"u"}}],"transition":{"native":"pptx:fade","fallback":"dissolve","durationMs":700,"advance":null,"effect":"fade","params":{"through":"none"}}},{"id":"b","hidden":true,"timeline":[],"transition":null}]}</script></head></html>"#;
        let (size, slides) = parse_deck(html).unwrap();
        assert_eq!(size.w, 960.0);
        assert_eq!(slides.len(), 2);
        let step = &slides[0].timeline[0];
        assert_eq!(step.effect.as_deref(), Some("flyIn"));
        assert_eq!(step.target.paragraphs, Some((0, 1)));
        assert_eq!(step.target.label(), "a.2 ¶0–1");
        assert_eq!(number(&step.duration_ms), Some(500.0));
        assert!(slides[0].has_motion());
        assert!(!slides[1].has_motion());
        assert_eq!(slides[0].clicks(), 1);
        assert!(parse_deck("<html></html>").is_err());
    }
}
