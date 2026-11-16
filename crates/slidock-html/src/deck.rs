//! The deck JSON, `slidock-deck@1` (`docs/format.md` §4): the manifest the player reads from
//! `<script id="sd-deck">`. It holds the slides in show order with their numbering, a11y titles and speaker notes,
//! each slide's build steps and transition as data, the media files and font faces the deck ships, and a summary of
//! the compile's diagnostics. The slide markup is not part of it: each slide is an inert template beside it.
//!
//! Build steps and transitions carry what the IR carries: the native effect id as provenance and its canonical
//! fallback, with the trigger, kind, delivery and timing. The common effect vocabulary joins them, and the slides'
//! match keys join the slides, as optional fields under the same format version (a reader ignores what it does not
//! know, so no version bump); the parity path's IR holds none, and its deck JSON is that of the IR before them.
//! A colour or font step plays as [`crate::paint_motion`] resolves it: its colour shifts computed from its target's
//! paint, or its fallback when the target has no single paint to expose.
//!
//! A reader ignores the fields it does not know, so `slidock-deck@1` grows by new optional fields. A new value of
//! an existing field (an enum variant, a meaning) makes a new format version: a reader refuses the versions it does
//! not know. The effect vocabulary is the exception: an `effect` a reader does not know reads as none, and the step
//! or transition plays its canonical fallback, so the vocabulary grows within a version.
//!
//! The JSON is canonical (keys in UTF-16 order, JavaScript number printing), so the same compile writes the same
//! bytes on every host. The TypeScript declarations in `packages/types` are generated from these types.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use slidock_canon::{Layout, sha256_hex, to_canonical_json, utf16};
use slidock_format::paths::{layout_path, slide_path};
use slidock_ir::diagnostics::{Diagnostic, Severity};
use slidock_ir::effects::{BuildEffect, Easing, StepTiming, TransitionEffect};
use slidock_ir::fonts::{FaceStyle, GenericFamily, font_key};
use slidock_ir::ids::Sha256;
use slidock_ir::media::PlanAction;
use slidock_ir::resolved::{Element, Presentation, Slide};
use slidock_ir::text::{Inline, TextBody};
use slidock_ir::timeline::{
    Delivery, StepFallback, StepKind, TimelineStep, Transition, TransitionFallback, Trigger,
};
use ts_rs::TS;

use crate::Documents;
use crate::error::{HtmlError, Result};
use crate::manifest::{Provenance, presentation_title, slide_title};
use crate::paint_motion;

/// The format version every deck JSON names, so a player knows the data it reads (zips outlive players).
pub const DECK_FORMAT: &str = "slidock-deck@1";

/// The format version: `"slidock-deck@1"`, the only one a reader of this crate accepts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub enum DeckFormat {
    #[default]
    #[serde(rename = "slidock-deck@1")]
    V1,
}

impl DeckFormat {
    pub const fn as_str(self) -> &'static str {
        match self {
            DeckFormat::V1 => DECK_FORMAT,
        }
    }
}

/// The player a deck ships with: its version and the SHA-256 of the script embedded beside the deck JSON, and of the
/// presenter view's script when the page ships one.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct PlayerStamp {
    pub version: String,
    /// Lowercase hex.
    pub sha256: String,
    /// The presenter view's script (of the same version), lowercase hex; `null` when the page has no presenter view.
    pub presenter: Option<String>,
}

/// A deck: what `<script type="application/json" id="sd-deck">` holds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct Deck {
    pub format: DeckFormat,
    /// The player embedded beside the deck JSON. A bundle records the player it embeds; `null` in a deck no bundle
    /// wrote (the web app's, which its own player shows).
    pub player: Option<PlayerStamp>,
    /// The SHA-256 of the deck's content (see [`deck_id`]): the same compile has the same id, whatever player it
    /// ships with. Presenter windows key their channel on it.
    pub id: String,
    /// The input's name without its extension, as the document set's manifest titles it.
    pub title: String,
    /// The BCP 47 language of the deck's text; `null` while no frontend reads one.
    pub lang: Option<String>,
    pub size: DeckSize,
    pub source: DeckSource,
    /// Show order, hidden slides included.
    pub slides: Vec<DeckSlide>,
    /// The files under `media/`, by path.
    pub media: Vec<DeckMedia>,
    /// The faces the deck's text draws, by family, weight and style; slides list theirs by index.
    pub fonts: Vec<DeckFont>,
    pub diagnostics: DiagnosticSummary,
}

/// The slide size in points.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct DeckSize {
    pub w: f64,
    pub h: f64,
}

/// The input the deck was compiled from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct DeckSource {
    /// `pptx`, `odp`, `keynote` or `ir`.
    pub format: String,
    pub name: String,
    /// Lowercase hex.
    pub sha256: String,
    pub bytes: u64,
}

/// One slide. Its markup is the template `<template data-sd-slide="<id>">`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "deck/")]
pub struct DeckSlide {
    pub id: String,
    /// The layout composed under it, if any.
    pub layout: Option<String>,
    pub hidden: bool,
    /// The 1-based number among the shown slides; `null` for a hidden slide.
    pub number: Option<u32>,
    /// The title for navigation and assistive technology; `""` when the slide has none.
    pub title: String,
    /// Speaker notes as plain paragraphs: line breaks as `\n`, tabs as `\t`, fields as their values, trailing blank
    /// paragraphs and trailing whitespace removed.
    pub notes: Vec<String>,
    /// The code of the diagnostic that failed the slide; its template then holds the failed slide's marker.
    pub failed: Option<String>,
    /// The build steps, in order.
    pub timeline: Vec<DeckStep>,
    /// The transition into this slide.
    pub transition: Option<DeckTransition>,
    /// The Magic Move / Morph match keys of the slide's own elements (group children and foreign fallbacks
    /// included), by element id (`data-el`): a player morphs the elements of two slides that share a key. Absent
    /// when the slide has none.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[ts(as = "Option<BTreeMap<String, String>>", optional)]
    pub match_keys: BTreeMap<String, String>,
    /// The faces the slide draws, its layout's included, as ascending indexes into the deck's `fonts`: what a
    /// player loads before it shows the slide.
    pub fonts: Vec<u32>,
}

fn is_false(value: &bool) -> bool {
    !*value
}

/// What a step builds: an element (its `data-el`), optionally a range of its paragraphs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct StepTarget {
    pub element: String,
    /// First and last paragraph index, inclusive.
    pub paragraphs: Option<(u32, u32)>,
    /// The element's shape without its text (PPTX `animBg`); absent when the step builds the whole element.
    #[serde(default, skip_serializing_if = "is_false")]
    #[ts(as = "Option<bool>", optional)]
    pub background: bool,
}

/// One build step.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "deck/")]
pub struct DeckStep {
    pub target: StepTarget,
    pub trigger: Trigger,
    pub kind: StepKind,
    pub delivery: Delivery,
    pub delay_ms: u32,
    pub duration_ms: u32,
    /// The source's own effect, `<format>:<name>` (`pptx:fade`, `pptx:entr.2/4`): provenance, not something a
    /// player decodes.
    pub native: String,
    /// What every player can play.
    pub fallback: StepFallback,
    /// The effect in the common vocabulary (`effect` and `params`), when the source's maps to one: a player plays
    /// it, or `fallback` when it does not know it.
    #[serde(flatten, default)]
    pub effect: DeckBuildEffect,
    /// Timing beyond the delay and the duration (`offsetMs`, `easing`, `stagger`, `repeat`, `autoReverse`, `after`).
    #[serde(flatten, default)]
    pub timing: StepTiming,
}

/// A step's effect in the common vocabulary, flattened into the step: a build effect's `effect` and `params`, or
/// neither. A reader that does not know the effect reads neither and plays the step's fallback.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "deck/")]
pub enum DeckBuildEffect {
    Effect(BuildEffect),
    #[ts(type = "{ effect?: never, params?: never }")]
    None {},
}

impl Default for DeckTransitionEffect {
    fn default() -> Self {
        DeckTransitionEffect::None {}
    }
}

impl Default for DeckBuildEffect {
    fn default() -> Self {
        DeckBuildEffect::None {}
    }
}

/// A transition's effect in the common vocabulary, flattened into the transition, or neither field.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(untagged)]
#[ts(export_to = "deck/")]
pub enum DeckTransitionEffect {
    Effect(TransitionEffect),
    #[ts(type = "{ effect?: never, params?: never }")]
    None {},
}

/// How a slide advances: on a click, and after a delay from the end of its transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "deck/")]
pub struct DeckAdvance {
    pub on_click: bool,
    pub after_ms: Option<u32>,
}

/// The transition into a slide.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "deck/")]
pub struct DeckTransition {
    /// The source's own effect, `<format>:<name>`.
    pub native: String,
    pub fallback: TransitionFallback,
    pub duration_ms: u32,
    pub advance: Option<DeckAdvance>,
    /// The effect in the common vocabulary (`effect` and `params`), when the source's maps to one.
    #[serde(flatten, default)]
    pub effect: DeckTransitionEffect,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub easing: Option<Easing>,
}

/// A file under `media/`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct DeckMedia {
    /// `media/<sha256>.<ext>`, as the slide markup references it.
    pub path: String,
    /// The type of the file as written.
    pub mime: String,
    /// The SHA-256 of the source item, lowercase hex: the file's name.
    pub sha256: String,
    /// The file's size: the source item's for a copy, `null` for a transcode (known once written).
    pub bytes: Option<u64>,
}

/// Where a face's file comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase")]
#[ts(export_to = "deck/")]
pub enum FontSource {
    /// Embedded in the source file.
    Embedded,
    /// Installed on the computer that compiled the deck (Local Font Access).
    Local,
    /// Google Fonts.
    Google,
    /// A file the user added.
    User,
    /// One of slidock's substitute families.
    Bundled,
    /// No file ships: the viewer's installed font, or its substitute, draws it.
    System,
    /// No file ships: an emoji family, which the viewer's emoji font draws.
    Emoji,
}

/// A face the deck's text draws.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct DeckFont {
    /// The family the slides name.
    pub family: String,
    /// The weight drawn, 100 to 900.
    pub weight: u32,
    pub style: FaceStyle,
    pub source: FontSource,
    /// The copyright and licence of the shipped file; `null` when no file ships.
    pub licence: Option<String>,
}

/// How many diagnostics of one code the compile reported.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct CodeCount {
    pub code: String,
    pub severity: Severity,
    pub count: u32,
}

/// The compile's diagnostics, counted.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct DiagnosticSummary {
    pub total: u32,
    pub error: u32,
    pub decline: u32,
    pub lossy: u32,
    pub warning: u32,
    pub info: u32,
    /// By code.
    pub codes: Vec<CodeCount>,
}

impl DiagnosticSummary {
    pub fn of(diagnostics: &[Diagnostic]) -> DiagnosticSummary {
        let mut summary = DiagnosticSummary::default();
        let mut codes: BTreeMap<&str, (Severity, u32)> = BTreeMap::new();
        for diagnostic in diagnostics {
            summary.total = summary.total.saturating_add(1);
            let count = match diagnostic.severity {
                Severity::Error => &mut summary.error,
                Severity::Decline => &mut summary.decline,
                Severity::Lossy => &mut summary.lossy,
                Severity::Warning => &mut summary.warning,
                Severity::Info => &mut summary.info,
            };
            *count = count.saturating_add(1);
            let entry = codes
                .entry(diagnostic.code.code())
                .or_insert((diagnostic.severity, 0));
            entry.1 = entry.1.saturating_add(1);
        }
        summary.codes = codes
            .into_iter()
            .map(|(code, (severity, count))| CodeCount {
                code: code.to_owned(),
                severity,
                count,
            })
            .collect();
        summary
    }
}

/// One diagnostic as the app shows it: what happened and where.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export_to = "deck/")]
pub struct DeckDiagnostic {
    pub code: String,
    pub severity: Severity,
    pub message: String,
    /// The feature it is about (`text.bullet.auto`).
    pub feature: Option<String>,
    /// The slide, layout or master it belongs to; `null` for the presentation.
    pub container: Option<String>,
    pub node: Option<String>,
}

impl From<&Diagnostic> for DeckDiagnostic {
    fn from(diagnostic: &Diagnostic) -> Self {
        DeckDiagnostic {
            code: diagnostic.code.code().to_owned(),
            severity: diagnostic.severity,
            message: diagnostic.message.clone(),
            feature: diagnostic
                .feature
                .map(|feature| feature.as_str().to_owned()),
            container: diagnostic.container.as_ref().map(ToString::to_string),
            node: diagnostic.node.as_ref().map(ToString::to_string),
        }
    }
}

/// A font file the host added to the compile: where it came from and its licence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFileInfo {
    pub sha256: Sha256,
    pub source: FontSource,
    pub licence: Option<String>,
}

/// What a deck is built from.
#[derive(Clone, Copy, Debug)]
pub struct DeckInput<'a> {
    /// The presentation as printed.
    pub presentation: &'a Presentation,
    /// Its document set.
    pub documents: &'a Documents,
    /// Every diagnostic of the compile, in output order.
    pub diagnostics: &'a [Diagnostic],
    pub source: &'a Provenance,
    /// The font files the host added.
    pub fonts: &'a [FontFileInfo],
}

/// Speaker notes as plain paragraphs: runs and fields as their text, breaks as `\n`, tabs as `\t`, trailing blank
/// paragraphs and trailing whitespace removed. Joined by `\n` they are [`crate::notes_text`].
pub fn notes_paragraphs(notes: Option<&TextBody>) -> Vec<String> {
    let Some(notes) = notes else {
        return Vec::new();
    };
    let mut paragraphs: Vec<String> = notes
        .paragraphs
        .iter()
        .map(|paragraph| {
            paragraph
                .inlines
                .iter()
                .map(|inline| match inline {
                    Inline::Run { text, .. } => text.as_str(),
                    Inline::Field { cached, .. } => cached.as_str(),
                    Inline::Break { .. } => "\n",
                    Inline::Tab {} => "\t",
                })
                .collect()
        })
        .collect();
    while let Some(last) = paragraphs.last_mut() {
        let trimmed = slidock_canon::js_trim_end(last).len();
        if trimmed == 0 {
            paragraphs.pop();
        } else {
            last.truncate(trimmed);
            break;
        }
    }
    paragraphs
}

fn step(step: &TimelineStep) -> DeckStep {
    let native = step.effect.native.as_ref();
    DeckStep {
        target: StepTarget {
            element: step.target.element.to_string(),
            paragraphs: step.target.paragraphs,
            background: step.target.background,
        },
        trigger: step.trigger,
        kind: step.kind,
        delivery: step.delivery,
        delay_ms: step.delay_ms.0,
        duration_ms: step.duration_ms.0,
        native: native.map_or_else(
            || step.effect.id.to_string(),
            |native| native.id.to_string(),
        ),
        fallback: step.effect.fallback,
        effect: native.map_or(DeckBuildEffect::None {}, |native| {
            DeckBuildEffect::Effect(native.effect.clone())
        }),
        timing: step.timing.as_deref().cloned().unwrap_or_default(),
    }
}

fn transition(transition: &Transition) -> DeckTransition {
    let native = transition.effect.native.as_ref();
    DeckTransition {
        native: native.map_or_else(
            || transition.effect.id.to_string(),
            |native| native.id.to_string(),
        ),
        fallback: transition.effect.fallback,
        duration_ms: transition.duration_ms.0,
        advance: transition.advance.as_ref().map(|advance| DeckAdvance {
            on_click: advance.on_click,
            after_ms: advance.after_ms.map(|ms| ms.0),
        }),
        effect: native.map_or(DeckTransitionEffect::None {}, |native| {
            DeckTransitionEffect::Effect(native.effect)
        }),
        easing: transition.easing,
    }
}

/// The faces a slide's template draws, as indexes into `fonts`: what its document's text draws, and its layout
/// document's when the slide shows its layout, each at the weight its binding prints.
fn slide_fonts(
    presentation: &Presentation,
    documents: &Documents,
    slide: &Slide,
    fonts: &[DeckFont],
) -> Vec<u32> {
    let layout = slide
        .layout
        .as_ref()
        .filter(|_| slide.show_layout)
        .map(|layout| layout_path(layout.as_str()));
    let drawn = [Some(slide_path(slide.id.as_str())), layout]
        .into_iter()
        .flatten()
        .filter_map(|path| documents.faces.get(&path))
        .flatten();
    let mut indexes = BTreeSet::new();
    for face in drawn {
        let Some(entry) = presentation.fonts.get(&font_key(&face.family)) else {
            continue;
        };
        let weight = entry.binding.drawn_weight(face.weight);
        let style = if face.italic {
            FaceStyle::Italic
        } else {
            FaceStyle::Normal
        };
        let index = fonts.iter().position(|font| {
            font.family == entry.request.family && font.weight == weight && font.style == style
        });
        if let Some(index) = index.and_then(|index| u32::try_from(index).ok()) {
            indexes.insert(index);
        }
    }
    indexes.into_iter().collect()
}

/// The match keys of a slide's own elements, by element id.
fn match_keys(slide: &Slide) -> BTreeMap<String, String> {
    let mut keys = BTreeMap::new();
    let mut elements: Vec<&Element> = slide.body.elements().iter().collect();
    while let Some(element) = elements.pop() {
        if let Some(key) = element.match_key() {
            keys.insert(element.id().to_string(), key.to_owned());
        }
        match element {
            Element::Group(group) => elements.extend(group.children.iter()),
            Element::Foreign(foreign) => {
                if let Some(fallback) = foreign
                    .fallback
                    .as_ref()
                    .and_then(|fallback| fallback.elements.as_ref())
                {
                    elements.extend(fallback.iter());
                }
            }
            _ => {}
        }
    }
    keys
}

fn slide(
    input: &DeckInput<'_>,
    slide: &Slide,
    number: Option<u32>,
    fonts: &[DeckFont],
) -> DeckSlide {
    DeckSlide {
        id: slide.id.to_string(),
        layout: slide.layout.as_ref().map(ToString::to_string),
        hidden: slide.hidden,
        number,
        title: slide_title(slide),
        notes: notes_paragraphs(slide.notes.as_ref()),
        failed: slide
            .body
            .failed()
            .map(|failed| failed.code.as_str().to_owned()),
        timeline: paint_motion::played_steps(slide, input.presentation)
            .iter()
            .map(|played| step(&played.step))
            .collect(),
        transition: slide.transition.as_ref().map(transition),
        match_keys: match_keys(slide),
        fonts: slide_fonts(input.presentation, input.documents, slide, fonts),
    }
}

fn slides(input: &DeckInput<'_>, fonts: &[DeckFont]) -> Vec<DeckSlide> {
    let mut shown: u32 = 0;
    input
        .presentation
        .slides
        .iter()
        .map(|entry| {
            let number = (!entry.hidden).then(|| {
                shown = shown.saturating_add(1);
                shown
            });
            slide(input, entry, number, fonts)
        })
        .collect()
}

fn media(presentation: &Presentation, documents: &Documents) -> Vec<DeckMedia> {
    documents
        .media
        .iter()
        .map(|(path, plan)| DeckMedia {
            path: path.clone(),
            mime: plan.mime.as_str().to_owned(),
            sha256: plan.sha256.to_hex(),
            bytes: match presentation.media.get(&plan.sha256) {
                Some(entry) if entry.plan.action == PlanAction::Copy => Some(entry.bytes),
                _ => None,
            },
        })
        .collect()
}

fn fonts(presentation: &Presentation, files: &[FontFileInfo]) -> Vec<DeckFont> {
    let mut faces: Vec<DeckFont> = Vec::new();
    for entry in presentation.fonts.values() {
        let binding = &entry.binding;
        let file = binding
            .file
            .as_ref()
            .and_then(|file| files.iter().find(|info| info.sha256 == file.sha256));
        for face in &entry.request.faces {
            faces.push(DeckFont {
                family: entry.request.family.clone(),
                weight: binding.drawn_weight(face.weight),
                style: face.style,
                source: match file {
                    Some(file) => file.source,
                    None if binding.generic == GenericFamily::Emoji => FontSource::Emoji,
                    None => FontSource::System,
                },
                licence: file.and_then(|file| file.licence.clone()),
            });
        }
    }
    faces.sort_by(|a, b| {
        utf16::cmp(&a.family, &b.family)
            .then(a.weight.cmp(&b.weight))
            .then(style_rank(a.style).cmp(&style_rank(b.style)))
    });
    faces.dedup_by(|a, b| a.family == b.family && a.weight == b.weight && a.style == b.style);
    faces
}

fn style_rank(style: FaceStyle) -> u8 {
    match style {
        FaceStyle::Normal => 0,
        FaceStyle::Italic => 1,
    }
}

#[derive(Serialize)]
struct IdContent<'a> {
    deck: &'a Deck,
    /// Each document's SHA-256, by path.
    documents: BTreeMap<&'a str, String>,
}

/// The content hash a deck's `id` holds: the SHA-256 of the canonical JSON of the deck (its `id` empty, its player
/// stamp `null`) together with the SHA-256 of every document of its set.
pub fn deck_id(deck: &Deck, documents: &Documents) -> Result<String> {
    let content = Deck {
        id: String::new(),
        player: None,
        ..deck.clone()
    };
    let content = IdContent {
        deck: &content,
        documents: documents
            .text
            .iter()
            .map(|(path, text)| (path.as_str(), sha256_hex(text)))
            .collect(),
    };
    Ok(slidock_canon::sha256_canonical(&content)?)
}

/// The deck JSON of a printed presentation, with no player stamp.
pub fn build_deck(input: &DeckInput<'_>) -> Result<Deck> {
    let presentation = input.presentation;
    let fonts = fonts(presentation, input.fonts);
    let mut deck = Deck {
        format: DeckFormat::V1,
        player: None,
        id: String::new(),
        title: presentation_title(&input.source.name),
        lang: None,
        size: DeckSize {
            w: presentation.size.w.to_f64(),
            h: presentation.size.h.to_f64(),
        },
        source: DeckSource {
            format: input.source.format.clone(),
            name: input.source.name.clone(),
            sha256: input.source.sha256.clone(),
            bytes: input.source.bytes,
        },
        slides: slides(input, &fonts),
        media: media(presentation, input.documents),
        fonts,
        diagnostics: DiagnosticSummary::of(input.diagnostics),
    };
    deck.id = deck_id(&deck, input.documents)?;
    Ok(deck)
}

impl Deck {
    /// The deck as compact canonical JSON.
    pub fn to_json(&self) -> Result<String> {
        Ok(to_canonical_json(self, Layout::Compact)?)
    }

    /// [`Deck::to_json`] made safe inside `<script type="application/json">`: every `<` is written `\u003c`, so no
    /// `</script>` or `<!--` can end the element. The text parses to the same value.
    pub fn to_script_json(&self) -> Result<String> {
        Ok(self.to_json()?.replace('<', "\\u003c"))
    }

    /// Reads a deck JSON, refusing any other format version and ignoring the fields it does not know.
    pub fn from_json(text: &str) -> Result<Deck> {
        serde_json::from_str(text).map_err(|error| HtmlError::Deck(error.to_string()))
    }
}
