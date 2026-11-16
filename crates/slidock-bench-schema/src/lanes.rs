//! The lanes beyond `static`: the host check, the fonts lane's
//! `fonts@1` metrics and source attribution, the motion lane's `motion@1` frames and timeline fidelity, and the
//! shell lane's zip checks. Row records carry their lane's section; pair- and slide-level sections sit at the top
//! of the results file.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use crate::de::{nullable, optional};
use crate::ids::{Frontend, Sha256, SlideId};
use crate::results::{FrontendSummary, Outcome};

/// Whether two images are the same PNG.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum FrameMatch {
    /// The sha256s are equal.
    Identical,
    /// Paused frames only: they differ as a frame composited mid-animation rasterizes: no channel by more than
    /// [`ANTIALIAS_CHANNEL`] beyond [`ANTIALIAS_PIXELS`] pixels (the player's own effect tests' bound), or, where the
    /// motion lane proved every animated node at its end equal to itself once settled, close enough that the static
    /// lane's metrics class it `match` (a glyph re-rasterized at a subpixel offset).
    Antialiased,
    Differs,
}

/// How far a channel of a paused, composited frame may stray from the still render.
pub const ANTIALIAS_CHANNEL: u8 = 96;
/// How many pixels may stray further: glyph edges.
pub const ANTIALIAS_PIXELS: u32 = 256;

impl FrameMatch {
    /// The frames show the same thing.
    pub const fn passes(self) -> bool {
        matches!(self, FrameMatch::Identical | FrameMatch::Antialiased)
    }
}

/// Host lane: the slide mounted in a shadow root on the player's stage against the standalone `--emit documents`
/// render of the same row. The gate requires `identical` on every row.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct HostCheck {
    pub status: FrameMatch,
    pub standalone: Sha256,
    pub mounted: Sha256,
    /// Render-vs-render SSIM at metric size, when they differ.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<f64>", optional)]
    pub ssim: Option<f64>,
}

/// Where a face's file came from, in resolution order (then the bundled substitutes).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum FontSource {
    /// Embedded in the source file (`.fntdata`, ODP `Fonts/`).
    Embedded,
    /// The viewer's fonts (in the bench, the pinned set's `host` entries).
    Local,
    /// The Google Fonts provider (in the bench, the CLI's recorded `css2` answers in
    /// `bench/fonts/fonts-lane.lock.json`).
    Google,
    /// Files the user dropped.
    Dropped,
    /// A bundled substitute (Liberation, Carlito, DejaVu, Caladea).
    Substitute,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum FontStyle {
    Normal,
    Italic,
    Oblique,
}

/// A file the deck ships: one face of `<style id="sd-fonts">`, decoded from its `data:` WOFF2.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ShippedFace {
    /// The `@font-face` family.
    pub family: String,
    pub style: FontStyle,
    /// The `font-weight` descriptor: one weight, or `<min> <max>` for a variable face.
    pub weight: String,
    pub source: FontSource,
    pub postscript_name: String,
    /// The sha256 of the file it was subset from, when the bench provided that file (Google mirror, substitutes).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<Sha256>", optional)]
    pub original_sha256: Option<Sha256>,
    /// The sha256 of the shipped WOFF2.
    pub woff2_sha256: Sha256,
    /// The shipped WOFF2's size.
    pub bytes: u64,
}

/// A face the deck's text draws (the deck data's `fonts`), and where its file came from.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RequestedFace {
    /// The family the slides name.
    pub family: String,
    pub style: FontStyle,
    /// The weight drawn.
    pub weight: u16,
    /// `null` when no file ships for it: another face of its family, a substitute or the viewer draws it.
    #[serde(with = "nullable")]
    #[ts(as = "Option<FontSource>")]
    pub source: Option<FontSource>,
}

/// Glyphs counted by how they were drawn. `exact + substitute + fallback = glyphs`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct GlyphCounts {
    pub glyphs: u32,
    /// Drawn by a shipped face of the requested design: a file of the requested family from any source.
    pub exact: u32,
    /// Drawn by a shipped bundled face standing in for another family (a substitute or a generic's family).
    pub substitute: u32,
    /// Drawn by a font the deck does not ship: the renderer's own fallback.
    pub fallback: u32,
    /// Drawn by another font than the first shipped family of the text's stack: code points the bound face lacks
    /// (MID5415's measure).
    pub coverage_gaps: u32,
    /// Shipped glyphs by the source of the face that drew them.
    pub sources: BTreeMap<FontSource, u32>,
}

impl GlyphCounts {
    /// Adds another count into this one.
    pub fn add(&mut self, other: &GlyphCounts) {
        self.glyphs = self.glyphs.saturating_add(other.glyphs);
        self.exact = self.exact.saturating_add(other.exact);
        self.substitute = self.substitute.saturating_add(other.substitute);
        self.fallback = self.fallback.saturating_add(other.fallback);
        self.coverage_gaps = self.coverage_gaps.saturating_add(other.coverage_gaps);
        for (source, glyphs) in &other.sources {
            let entry = self.sources.entry(*source).or_default();
            *entry = entry.saturating_add(*glyphs);
        }
    }
}

/// Fonts lane, per pair: what the compile shipped and what the pair's rows drew with it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontsPairReport {
    pub faces: Vec<ShippedFace>,
    pub requested: Vec<RequestedFace>,
    /// The shipped WOFF2 bytes, all faces.
    pub font_bytes: u64,
    /// The bytes of `<style id="sd-fonts">` in `index.html`: the faces as the deck carries them, base64 and rules.
    pub css_bytes: u64,
    /// Per family the slides name: the glyphs of the pair's rows.
    pub families: BTreeMap<String, GlyphCounts>,
    /// The compile's font diagnostics (MID54xx), by code.
    pub notices: BTreeMap<String, u32>,
}

/// How a lane's render of a row compares with its static render.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum Equivalence {
    /// The render sha256s are equal.
    Identical,
    /// The render differs and the class moved up.
    DriftBetter,
    /// The render differs and the class stayed.
    DriftSameClass,
    /// The class moved down.
    Regressed,
}

/// The fonts lane's render (injected WOFF2, an empty fontconfig) against the static lane's (system fonts).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontsEquivalence {
    pub status: Equivalence,
    pub static_render: Sha256,
    pub static_outcome: Outcome,
    /// Render-vs-render SSIM at metric size, when they differ.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<f64>", optional)]
    pub ssim: Option<f64>,
}

/// `fonts@1`, per row, from `CSS.getPlatformFontsForNode` over every text node the mounted slide draws.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontsRowMetrics {
    /// Every text node's glyphs.
    pub total: GlyphCounts,
    /// `exact / glyphs`, `null` on a row that draws no glyph.
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub exact_share: Option<f64>,
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub substitute_share: Option<f64>,
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub fallback_share: Option<f64>,
    /// The renderer's fonts that drew glyphs (their family names).
    pub fallback_fonts: Vec<String>,
    /// Per family the slides name.
    pub families: BTreeMap<String, GlyphCounts>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<FontsEquivalence>", optional)]
    pub equivalence: Option<FontsEquivalence>,
}

/// How the player played an effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum EffectPlay {
    Native,
    /// The deck names no effect the player plays (the compiler gave none, or the player cannot play it): the IR's
    /// fallback played.
    Fallback,
    /// Neither played.
    Unsupported,
}

/// An effect a row shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct EffectRef {
    /// The native id (`pptx:entr.2/4`, `keynote:apple:move in`).
    pub id: String,
    /// The IR's canonical fallback.
    pub fallback: String,
    pub played: EffectPlay,
}

/// A frame that must equal another frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct FrameCheck {
    pub status: FrameMatch,
    pub expected: Sha256,
    pub actual: Sha256,
}

/// The stage count the IR implies against the stages oracle's pages for the slide.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StageCount {
    pub expected: u32,
    pub oracle: u32,
}

/// What a motion row shows over time, as PNG sha256s: a stage row's group before it plays, halfway and at its end
/// (every animation paused there); a transition row's first frame, its own frame and its last frame.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct FrameStrip {
    pub start: Sha256,
    pub mid: Sha256,
    pub end: Sha256,
}

/// A build unit whose visibility is not the one the timeline gives at the stage.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RevealMiss {
    /// The element's `data-el`.
    pub element: String,
    /// The paragraph (`data-p`) of a paragraph build.
    #[serde(with = "nullable")]
    #[ts(as = "Option<u32>")]
    pub paragraph: Option<u32>,
    /// Whether the timeline shows it at the stage.
    pub expected: bool,
    /// Whether it shows; `null` when the slide has no such unit.
    #[serde(with = "nullable")]
    #[ts(as = "Option<bool>")]
    pub actual: Option<bool>,
}

/// What shows at a stage, against the timeline: an entrance target shows from its step on, an exit target and a
/// step's `after: hide` target hide from their step on.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct RevealCheck {
    /// The units checked.
    pub units: u32,
    pub wrong: Vec<RevealMiss>,
}

/// `motion@1`, per stage or transition row.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionRowMetrics {
    /// Transition rows: the effect into the slide.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<EffectRef>", optional)]
    pub effect: Option<EffectRef>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<FrameStrip>", optional)]
    pub frames: Option<FrameStrip>,
    /// SSIM of the row's frame against its reference: a stage's end frame against its still render where they
    /// differ; a transition or probe frame against the reference model's.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<f64>", optional)]
    pub ssim: Option<f64>,
    /// Mean ΔE00 of the same pair.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<f64>", optional)]
    pub delta_e: Option<f64>,
    /// Stage rows: the frame with every animation of the stage paused at its end, against the player's still render
    /// of the stage (exit targets hidden, holds and dims applied).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<FrameCheck>", optional)]
    pub end_state: Option<FrameCheck>,
    /// Stage rows: the frame once the player has settled the stage, against the same still render.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<FrameCheck>", optional)]
    pub settled: Option<FrameCheck>,
    /// Stage rows: the stage's first frame equals the state before it (the previous stage's still render, or the
    /// transition's last frame for stage 0), and each unit shows at it as before the stage, so nothing a step builds
    /// shows before its step.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<bool>", optional)]
    pub start_state: Option<bool>,
    /// Stage rows: what shows once the stage settled, against the timeline.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<RevealCheck>", optional)]
    pub reveal: Option<RevealCheck>,
    /// Stage rows: no unit appeared or disappeared since the previous stage but by a step of this one.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<bool>", optional)]
    pub monotonic: Option<bool>,
    /// Transition rows: the frame differs from both the outgoing and the incoming slide.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<bool>", optional)]
    pub moving: Option<bool>,
    /// Each check the row failed, `<check>: <detail>`: a frame check (`end-state`, `start-state`, `settled`,
    /// `reveal`, `monotonic`), a timeline field of a step of the stage (`moves`, `direction`, …), or the transition's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<String>>")]
    pub failures: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<StageCount>", optional)]
    pub stage_count: Option<StageCount>,
    /// A `motion-mismatch` row whose every failed check a review accepts (`bench/motion/fallback-allow.json`'s
    /// `checks`): the reasons. The row keeps its class; the lane does not fail on it.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub accepted: Option<String>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct EffectCounts {
    pub native: u32,
    pub fallback: u32,
    pub unsupported: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct StepCounts {
    pub expected: u32,
    pub played: u32,
}

/// Targets the player split for word, character or paragraph delivery.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct DeliveryCounts {
    pub paragraphs: u32,
    pub words: u32,
    pub chars: u32,
}

/// When a step starts inside its click group and how long one iteration lasts, in ms.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepTiming {
    pub start_ms: f64,
    pub duration_ms: f64,
}

/// One step of a slide's timeline: what the deck says, and what the player played.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepPlay {
    /// Its index in the slide's timeline.
    pub index: u32,
    /// The click group it belongs to (0: the automatic steps at the start).
    pub group: u32,
    pub trigger: String,
    pub kind: String,
    /// `<element>`, `<element> ¶<first>–<last>` or `<element> background`.
    pub target: String,
    pub native: String,
    /// The vocabulary effect the deck names; `null` when it names none.
    #[serde(with = "nullable")]
    #[ts(as = "Option<String>")]
    pub effect: Option<String>,
    /// Its params as the deck writes them (JSON), when it has any.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub params: Option<String>,
    /// `byParagraph`, `byWord` or `byChar`; absent when built whole.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub delivery: Option<String>,
    pub fallback: String,
    pub played: EffectPlay,
    pub expected: StepTiming,
    /// `null` when the player animated nothing of it.
    #[serde(with = "nullable")]
    #[ts(as = "Option<StepTiming>")]
    pub actual: Option<StepTiming>,
    /// The group the player played it in.
    #[serde(with = "nullable")]
    #[ts(as = "Option<u32>")]
    pub played_in: Option<u32>,
    /// The units it animated (elements, paragraphs, words or characters).
    pub units: u32,
}

/// The transition into a slide: what the deck says, and what the player played.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TransitionPlay {
    pub effect: EffectRef,
    /// The vocabulary effect the deck names; `null` when it names none.
    #[serde(with = "nullable")]
    #[ts(as = "Option<String>")]
    pub name: Option<String>,
    /// Its params as the deck writes them (JSON), when it has any.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub params: Option<String>,
    pub expected_ms: f64,
    /// `null` when the player animated nothing (a cut).
    #[serde(with = "nullable")]
    #[ts(as = "Option<f64>")]
    pub actual_ms: Option<f64>,
    /// Its first frame against the outgoing slide; `null` for the first slide shown.
    #[serde(with = "nullable")]
    #[ts(as = "Option<FrameCheck>")]
    pub from: Option<FrameCheck>,
    /// Its last frame against the incoming slide before its first group.
    #[serde(with = "nullable")]
    #[ts(as = "Option<FrameCheck>")]
    pub to: Option<FrameCheck>,
}

/// What a timeline check compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum TimelineField {
    /// The step played in another click group.
    Group,
    /// The steps started in another order.
    Order,
    Start,
    Duration,
    /// The step played its fallback where the deck names an effect the player plays, or the other way round.
    Effect,
    /// The step animated nothing.
    Missing,
    /// An animation no step explains.
    Extra,
    /// An animated property outside the player's allow-list.
    Property,
    /// The deck's timeline or transition disagrees with the compile's IR.
    Ir,
    Transition,
    /// The player reported a move that failed or a step it dropped.
    Player,
    /// The step or transition does not move between its ends (a jump cut, or keyframes that change nothing).
    Moves,
    /// Its content moves another way than its `dir`.
    Direction,
}

/// One difference between what the deck says and what played.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TimelineMismatch {
    /// The step's index; `null` for the slide or its transition.
    #[serde(with = "nullable")]
    #[ts(as = "Option<u32>")]
    pub step: Option<u32>,
    pub field: TimelineField,
    pub expected: String,
    pub actual: String,
}

/// Motion lane, per slide with a timeline or a transition: what the player played against the deck's timeline.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TimelineFidelity {
    pub steps: StepCounts,
    /// The played order equals the IR's.
    pub order: bool,
    pub durations_exact: bool,
    pub delays_exact: bool,
    pub delivery: DeliveryCounts,
    /// Per native effect id.
    pub effects: BTreeMap<String, EffectCounts>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<TransitionPlay>", optional)]
    pub transition: Option<TransitionPlay>,
    /// Each step, in timeline order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<StepPlay>>")]
    pub plays: Vec<StepPlay>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<TimelineMismatch>>")]
    pub mismatches: Vec<TimelineMismatch>,
    /// The deck's timeline and transition agree with the compile's IR; absent when the IR holds neither for the
    /// slide (a frontend whose parity path leaves motion out).
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<bool>", optional)]
    pub ir: Option<bool>,
    /// The player's diagnostics on the slide, by code.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    #[ts(optional, as = "Option<BTreeMap<String, u32>>")]
    pub diagnostics: BTreeMap<String, u32>,
}

/// How a step or transition played against what its deck, or its probe expectation, says.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, TS)]
#[serde(rename_all = "kebab-case")]
pub enum MotionVerdict {
    /// The expected effect played natively, and its timeline and frames check out.
    Native,
    /// Its fallback played where nothing more is expected: the deck (or the probe) names no effect for it.
    Declared,
    /// `bench/motion/fallback-allow.json` accepts its fallback (or another effect) where an effect is expected.
    Allowed,
    /// It played as expected (natively, its declared fallback, or nothing where nothing animates) but failed a check
    /// `bench/motion/fallback-allow.json` accepts: a defect a review tracks.
    Accepted,
    /// A probe step the IR leaves out, and nothing played.
    Dropped,
    /// Anything else: another effect, an unaccepted fallback, nothing, or a failed check.
    Mismatched,
}

/// One item of a deck's motion: a slide's transition or one of its steps.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct MotionItem {
    pub slide: SlideId,
    /// The probe's slide key (`tr-push-l`); absent for a corpus deck.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub key: Option<String>,
    /// `transition`, or `step <index>`.
    pub item: String,
    pub native: String,
    /// The effect expected (the probe's, else the deck's); `null` when none is.
    #[serde(with = "nullable")]
    #[ts(as = "Option<String>")]
    pub expected: Option<String>,
    /// What played: the effect, `fallback:<name>`, or `nothing`.
    pub played: String,
    pub verdict: MotionVerdict,
    /// Why it is mismatched, or the allowlist's reason.
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<String>", optional)]
    pub why: Option<String>,
}

/// Motion lane, per deck: its items by verdict.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionPairReport {
    /// Slides with a timeline or a transition.
    pub slides: u32,
    pub steps: u32,
    pub transitions: u32,
    pub native: u32,
    /// Fallbacks played where nothing more is expected, and accepted ones.
    pub fallback_allowed: u32,
    /// Items that played as expected with a failed check a review accepts.
    pub accepted: u32,
    /// Probe items the IR leaves out.
    pub dropped: u32,
    pub mismatched: u32,
    /// Whether the items are the probe's expectations (`probes/out/*.expect.json`) rather than the deck's own.
    pub probe: bool,
    /// Every item that is not native.
    pub items: Vec<MotionItem>,
}

/// Shell lane, per row: the unzipped deck's frame at the row's position against the motion run's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ShellCheck {
    pub status: FrameMatch,
    pub frame: Sha256,
    pub expected: Sha256,
}

/// Shell lane, per pair: the zip's file list and what the deck asked of the network.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ShellPairCheck {
    pub files: u32,
    /// Referenced media, font and player files the zip lacks.
    pub missing: Vec<String>,
    /// Absolute or external URLs left in the deck.
    pub external: Vec<String>,
    /// Requests the deck made beyond its own files (`file:`, `data:`, `blob:`).
    pub requests: Vec<String>,
    /// The zip's `index.html` against the deck the motion run scored.
    pub deck: FrameMatch,
    pub boot: ShellBoot,
}

/// Shell lane, per pair: the unzipped deck opened as a user opens it, booted by its own script under its own Content
/// Security Policy with nothing of the lane's in the way.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct ShellBoot {
    /// The deck's own boot script mounted its player on the page.
    pub mounted: bool,
    /// Content Security Policy violations and uncaught errors the page reported.
    pub errors: Vec<String>,
    /// The first shown position as the booted deck shows it, against the motion run's settled frame there: identical
    /// when every pixel outside the player's own chrome (`chrome`) is; antialiased, for a deck whose size is not whole
    /// pixels (its stage is centred a fraction of a pixel off, and resampled), when outside the chrome no channel
    /// differs by more than [`ANTIALIAS_CHANNEL`] beyond [`ANTIALIAS_PIXELS`] pixels; `null` when that slide shows media
    /// whose frame depends on time (a video, an animated GIF), which only the walk compares.
    #[serde(with = "nullable")]
    #[ts(as = "Option<ShellCheck>")]
    pub first: Option<ShellCheck>,
    /// The boxes of the player's chrome around and beside its stage (x, y, width, height), left out of `first`: the
    /// pixels the stage does not wholly cover, and the slide counter, which shows for a while after each move.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[ts(optional, as = "Option<Vec<[f64; 4]>>")]
    pub chrome: Vec<[f64; 4]>,
}

impl ShellBoot {
    /// The deck booted itself, reported nothing, and shows what was scored.
    pub fn passes(&self) -> bool {
        self.mounted
            && self.errors.is_empty()
            && self
                .first
                .as_ref()
                .is_none_or(|first| first.status.passes())
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct FrameSummary {
    pub rows: u32,
    pub identical: u32,
    pub differs: u32,
    /// Paused frames that differ only by antialiasing.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[ts(optional, as = "Option<u32>")]
    pub antialiased: u32,
}

fn is_zero(value: &u32) -> bool {
    *value == 0
}

/// Fonts lane, per frontend: rows by class, their glyphs, and what the pairs shipped.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FontsSummary {
    pub rows: u32,
    /// Rows classed `fonts-complete`.
    pub complete: u32,
    /// The rows' glyphs.
    pub total: GlyphCounts,
    pub faces: u32,
    /// Requested faces no file ships for.
    pub unresolved: u32,
    pub font_bytes: u64,
}

/// Rows by motion class.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct MotionClasses {
    pub rows: u32,
    #[serde(rename = "motion-match")]
    #[ts(rename = "motion-match")]
    pub motion_match: u32,
    #[serde(rename = "motion-close")]
    #[ts(rename = "motion-close")]
    pub motion_close: u32,
    #[serde(rename = "motion-mismatch")]
    #[ts(rename = "motion-mismatch")]
    pub motion_mismatch: u32,
    #[serde(rename = "motion-fallback")]
    #[ts(rename = "motion-fallback")]
    pub motion_fallback: u32,
}

/// Slides whose timeline played as the deck says.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct TimelineSummary {
    pub slides: u32,
    pub exact: u32,
}

/// Items (transitions and steps) by verdict.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct VerdictCounts {
    pub native: u32,
    pub fallback_allowed: u32,
    pub accepted: u32,
    pub dropped: u32,
    pub mismatched: u32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MotionSummary {
    /// Stage rows scored against a stages oracle, by `classifier@2`.
    pub stages: FrontendSummary,
    /// Stage rows no oracle scores, by motion class: the invariants alone.
    pub stage_checks: MotionClasses,
    pub transitions: MotionClasses,
    pub effects: EffectCounts,
    pub end_state: FrameSummary,
    pub timelines: TimelineSummary,
    pub items: VerdictCounts,
}

/// Per frontend, the summary of the file's lane.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize, TS)]
#[serde(deny_unknown_fields)]
pub struct LaneSummaries {
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<Frontend, FrameSummary>>", optional)]
    pub host: Option<BTreeMap<Frontend, FrameSummary>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<Frontend, FontsSummary>>", optional)]
    pub fonts: Option<BTreeMap<Frontend, FontsSummary>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<Frontend, MotionSummary>>", optional)]
    pub motion: Option<BTreeMap<Frontend, MotionSummary>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "optional")]
    #[ts(as = "Option<BTreeMap<Frontend, FrameSummary>>", optional)]
    pub shell: Option<BTreeMap<Frontend, FrameSummary>>,
}
