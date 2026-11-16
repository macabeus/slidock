//! Keynote's `all stages` exports as the motion lane's stage oracles.
//!
//! **Pages.** Keynote 14.5 prints, per shown slide in order (a skipped slide prints nothing), one page of the slide
//! before its first click, one more page per click, and one page before that when the slide's timeline opens with
//! builds that start by themselves (Keynote prints the slide as it appears, then once those builds ran). A build that
//! starts with or after another prints on that build's page. So a slide of K click groups takes K + 1 pages, plus a
//! lead page when its first step is not a click; position k (`#s<k>`) is page `first + lead + k`. A total that
//! differs is `oracle-page-count`, and the deck is not scored.
//!
//! **Identity.** The export is pinned by its page rasters (`stages.pageHashes` under the raster key, as the static
//! oracles are), since Keynote writes its creation date into the PDF and a re-export never has the same bytes; a
//! manifest that records only the file's sha256 pins that. Pages that differ from the pinned ones are
//! `oracle-identity`, and the deck is not scored.
//!
//! **Scores.** Each position's settled frame against its page with the static lane's metrics: SSIM and ΔE00 at the
//! metric size, the text recall of the page's words against the words the settled stage draws (measured by the walk
//! and joined as the static lane joins them, [`frame_words`]; no ink test: a frame has no text-less twin), and
//! `classifier@2`'s scored class. A deck with an export draws the faces the static lane renders Keynote's decks in:
//! the lane compiles it with the pinned font set as its last font source ([`super::lane_deck_arguments`]), since the
//! deck path would otherwise ship its bundled substitutes for every Apple and corpus face it has no file of, and a
//! class would be the faces'. The lane counts every position's class, and apart each slide's built position (its
//! last), the state the static lane renders. Classes are reported, not checked: the motion is held by two checks, each
//! comparing where Keynote's pages changed with where the frames changed, on a coarse grid (cells whose mean luma
//! moves): most of each side's changed cells must lie within [`REVEAL_REACH`] cells of the other side's (a build
//! drawn in another face's metrics lands a little elsewhere, a build of another element or none at all does not).
//!
//! - **reveal**: against the position before (the lead page and the opening frame for position 0): what this stage
//!   adds;
//! - **remaining**: against the slide's last position, for every position before it: what is still to come, so
//!   position 0 holds the slide's start state even without a lead page.
//!
//! On a slide with builds every position has at least one of them; a position that passes the ones it has is `match`,
//! one that fails either is `mismatch`, with the check it failed, and one with neither (a neighbour's frame or page
//! missing) is `unchecked`, counted apart. The positions of slides with no builds are the static lane's rows: only
//! their built classes are counted.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use slidock_bench_metrics::{
    HostLibm, Rgba, ScoredClass, Scores, Size, SsimSummary, TextWord, align_words,
    color_difference, decode_png, luma, metric_pair, oracle_words, pixel_score, quantise,
    scored_class, ssim, text_score,
};
use slidock_bench_schema::{Sha256, SlideId};

use super::deck::{IrSlide, MotionDeck};
use super::drive::StageWord;

/// The stage scores of a motion run, per pair: `runs/<key>/stages.json`.
pub const STAGES_FILE: &str = "stages.json";

/// Grid of the reveal check, per side of the long edge.
const REVEAL_CELLS: usize = 48;
/// A cell's mean luma (0–255) must move by more than this to count as changed.
const REVEAL_LUMA: f32 = 6.0;
/// How far (in cells) a changed cell may lie from the other side's.
pub const REVEAL_REACH: usize = 2;
/// The share of each side's changed cells the other side must reach.
const REVEAL_COVER: f64 = 0.5;

/// A shown slide's pages in an all-stages export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SlidePages {
    /// The slide's index in the deck.
    pub index: usize,
    pub slide: String,
    /// Its first page, 1-based.
    pub first: u32,
    /// Whether its first page is the slide before its opening automatic builds.
    pub lead: bool,
    /// Its positions: click groups + 1.
    pub positions: u32,
}

impl SlidePages {
    pub fn pages(&self) -> u32 {
        self.positions + u32::from(self.lead)
    }

    /// The page of position `k`.
    pub fn page(&self, k: u32) -> u32 {
        self.first + u32::from(self.lead) + k
    }
}

/// Shown slides' pages in show order, from each one's index, id, step triggers and whether it is skipped; and the
/// total page count they imply. A slide has a lead page when its timeline opens with a step that starts by itself.
fn plan<'a>(
    slides: impl IntoIterator<Item = (usize, &'a str, bool, Vec<&'a str>)>,
) -> (Vec<SlidePages>, u32) {
    let mut next = 1u32;
    let mut out = Vec::new();
    for (index, id, hidden, triggers) in slides {
        if hidden {
            continue;
        }
        let clicks = triggers.iter().filter(|t| **t == "click").count() as u32;
        let pages = SlidePages {
            index,
            slide: id.to_owned(),
            first: next,
            lead: triggers.first().is_some_and(|t| *t != "click"),
            positions: clicks.saturating_add(1),
        };
        next = next.saturating_add(pages.pages());
        out.push(pages);
    }
    (out, next - 1)
}

/// Every shown slide's pages, in show order, and the total page count they imply.
pub fn stage_pages(deck: &MotionDeck) -> (Vec<SlidePages>, u32) {
    plan(deck.slides.iter().enumerate().map(|(index, slide)| {
        let triggers = slide.timeline.iter().map(|s| s.trigger.as_str()).collect();
        (index, slide.id.as_str(), slide.hidden, triggers)
    }))
}

/// The same pages from a deck's IR (`--emit ir --native-motion`): what `oracle --stages --record` records.
pub fn ir_stage_pages(slides: &[IrSlide]) -> (Vec<SlidePages>, u32) {
    plan(slides.iter().enumerate().map(|(index, slide)| {
        let triggers = slide.timeline.iter().map(|s| s.trigger.as_str()).collect();
        (index, slide.id.as_str(), slide.hidden, triggers)
    }))
}

/// The pages each shown slide prints (`pagesPerRow`), by slide id.
pub fn pages_per_row(plan: &[SlidePages]) -> BTreeMap<String, u32> {
    plan.iter()
        .map(|pages| (pages.slide.clone(), pages.pages()))
        .collect()
}

/// How the pages a deck's timelines give each shown slide answer the manifest's `pagesPerRow` and the export.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Rows {
    /// The record is these rows.
    Recorded,
    /// Nothing is recorded yet.
    Unrecorded,
    /// They add up to another count than the export's pages (`oracle-page-count`).
    PageCount(String),
    /// The record differs (`stage-count`): which rows, and how.
    Differs(String),
}

/// `1 page`, `2 pages`.
fn page_count(n: u32) -> String {
    if n == 1 {
        "1 page".to_owned()
    } else {
        format!("{n} pages")
    }
}

/// Checks `rows` (the pages each shown slide prints by the deck's timelines) against the export's page count and the
/// manifest's record.
pub fn check_rows(
    recorded: Option<&BTreeMap<SlideId, u32>>,
    rows: &BTreeMap<String, u32>,
    pages: u32,
) -> Rows {
    let total: u64 = rows.values().map(|n| u64::from(*n)).sum();
    if total != u64::from(pages) {
        return Rows::PageCount(format!(
            "the export has {pages} pages, the deck's timelines imply {total}"
        ));
    }
    let Some(recorded) = recorded else {
        return Rows::Unrecorded;
    };
    let mut differences = Vec::new();
    for (slide, n) in rows {
        match recorded.iter().find(|(id, _)| id.as_str() == slide) {
            Some((_, m)) if m == n => {}
            Some((_, m)) => differences.push(format!(
                "{slide} prints {}, its timeline gives {n}",
                page_count(*m)
            )),
            None => differences.push(format!("{slide} is not recorded")),
        }
    }
    for slide in recorded.keys() {
        if !rows.contains_key(slide.as_str()) {
            differences.push(format!("{slide} is recorded, not shown"));
        }
    }
    if differences.is_empty() {
        return Rows::Recorded;
    }
    let more = differences.len().saturating_sub(8);
    differences.truncate(8);
    if more > 0 {
        differences.push(format!("{more} more"));
    }
    Rows::Differs(differences.join("; "))
}

/// Where two consecutive images of one source changed: the reveal grid's columns and its cells.
pub fn changed_cells(before: &Rgba, after: &Rgba) -> Option<Grid> {
    if before.width() != after.width() || before.height() != after.height() {
        return None;
    }
    let (width, height) = (before.width(), before.height());
    let long = width.max(height).max(1);
    let cols = (REVEAL_CELLS * width).div_ceil(long).max(1);
    let rows = (REVEAL_CELLS * height).div_ceil(long).max(1);
    let means = |image: &Rgba| -> Vec<f32> {
        let values = luma(image);
        let mut sums = vec![0f64; cols * rows];
        let mut counts = vec![0u32; cols * rows];
        for (i, value) in values.iter().enumerate() {
            let (x, y) = (i % width, i / width);
            let cell = (y * rows / height) * cols + x * cols / width;
            if let (Some(sum), Some(count)) = (sums.get_mut(cell), counts.get_mut(cell)) {
                *sum += f64::from(*value);
                *count += 1;
            }
        }
        sums.iter()
            .zip(&counts)
            .map(|(sum, count)| (*sum / f64::from((*count).max(1))) as f32)
            .collect()
    };
    let (a, b) = (means(before), means(after));
    Some(Grid {
        cols,
        cells: a
            .iter()
            .zip(&b)
            .map(|(x, y)| (x - y).abs() > REVEAL_LUMA)
            .collect(),
    })
}

/// Changed cells of the reveal grid, row by row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Grid {
    pub cols: usize,
    pub cells: Vec<bool>,
}

impl Grid {
    fn count(&self) -> u32 {
        self.cells.iter().filter(|c| **c).count() as u32
    }

    /// Whether a changed cell lies within [`REVEAL_REACH`] cells of cell `at`.
    fn reaches(&self, at: usize) -> bool {
        let cols = self.cols.max(1);
        let rows = self.cells.len() / cols;
        let (x, y) = (at % cols, at / cols);
        let span =
            |c: usize, n: usize| c.saturating_sub(REVEAL_REACH)..(c + REVEAL_REACH + 1).min(n);
        span(y, rows).any(|yy| {
            span(x, cols).any(|xx| self.cells.get(yy * cols + xx).copied().unwrap_or(false))
        })
    }

    /// The share of this grid's changed cells the other reaches (1 when none changed).
    fn covered_by(&self, other: &Grid) -> f64 {
        let changed: Vec<usize> = (0..self.cells.len()).filter(|i| self.cells[*i]).collect();
        if changed.is_empty() {
            return 1.0;
        }
        let reached = changed.iter().filter(|i| other.reaches(**i)).count();
        reached as f64 / changed.len() as f64
    }
}

/// The reveal check of one stage.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Reveal {
    /// Cells Keynote's page changed, and cells the frame changed.
    pub keynote: u32,
    pub played: u32,
    /// The share of Keynote's changed cells the frame's reach, and of the frame's Keynote's.
    pub keynote_covered: f64,
    pub played_covered: f64,
    pub agrees: bool,
}

/// Compares two change grids of the same size (each source resampled to the metric size first, so they align).
pub fn reveal(keynote: &Grid, played: &Grid) -> Reveal {
    let keynote_covered = keynote.covered_by(played);
    let played_covered = played.covered_by(keynote);
    Reveal {
        keynote: keynote.count(),
        played: played.count(),
        keynote_covered: quantise(keynote_covered),
        played_covered: quantise(played_covered),
        agrees: keynote_covered >= REVEAL_COVER && played_covered >= REVEAL_COVER,
    }
}

/// One position scored against its page.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StageScore {
    pub page: u32,
    pub ssim: f64,
    pub ssim_p1: f64,
    pub color: f64,
    /// The page's words found in the frame's; `None` when the page has none.
    pub text_recall: Option<f64>,
    pub class: &'static str,
    /// Whether this is the slide's built position (its last).
    pub built: bool,
    /// The settled frame scored (`frames/<sha256>.png` of the run).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub frame: Option<Sha256>,
    /// Against the stage before (the lead page for position 0); `None` for position 0 of a slide with no lead page.
    pub reveal: Option<Reveal>,
    /// Against the slide's last position; `None` for the last position.
    pub remaining: Option<Reveal>,
    /// The stage oracle's verdict on a slide with builds: `match`, `mismatch`, or `unchecked` when neither check ran;
    /// `None` on a slide with none.
    pub stage: Option<&'static str>,
    /// The checks a `mismatch` failed.
    pub failed: Vec<String>,
}

fn class_name(class: ScoredClass) -> &'static str {
    match class {
        ScoredClass::Match => "match",
        ScoredClass::Close => "close",
        ScoredClass::Mismatch => "mismatch",
    }
}

/// The scores of a frame against a page (both decoded), with the page's words in slide points and the frame's.
pub fn score_stage(
    page: &Rgba,
    frame: &Rgba,
    page_words: &[TextWord],
    frame_words: &[TextWord],
    slide: Size,
) -> Option<Scores> {
    let (oracle, render) = metric_pair(page, frame).ok()?;
    let structure = ssim(&oracle, &render).ok()?;
    let long_side = slide.w.max(slide.h);
    let alignment = align_words::<HostLibm>(page_words, frame_words, long_side, None).ok()?;
    Some(Scores {
        ssim: SsimSummary {
            mean: structure.mean,
            p1: structure.p1,
        },
        color: color_difference::<HostLibm>(&oracle, &render).ok()?,
        pixel: pixel_score(&oracle, &render).ok()?,
        text: text_score::<HostLibm>(page_words, frame_words, &alignment, long_side).ok()?,
    })
}

/// The frame's words as the metrics read them. Text in adjacent nodes of one text box with no whitespace between (a
/// run split by its styles, a bullet label and its run) reads as one word where the glyphs touch, as the static lane's
/// `measure.js` joins it and as the page's text extraction reads it: the same element and text box, the first node
/// ending and the next starting there, on one line (centres within half the taller's height), and a gap of at most
/// 0.15 of the larger font size (and no more than 0.5 pt of overlap).
pub fn frame_words(words: &[StageWord]) -> Vec<TextWord> {
    // Each word with the node word it began with, whose element, text box and size the join compares.
    let mut out: Vec<(TextWord, &StageWord)> = Vec::new();
    let mut previous: Option<&StageWord> = None;
    for word in words {
        let joined = match (out.last_mut(), previous) {
            (Some((last, first)), Some(previous)) => {
                let rect = last.rect;
                let gap = word.x - (rect.x + rect.w);
                let same_line = (word.y + word.h / 2.0 - (rect.y + rect.h / 2.0)).abs()
                    <= word.h.max(rect.h) / 2.0;
                let joins = first.el == word.el
                    && first.text_box == word.text_box
                    && previous.join_after
                    && word.join_before
                    && same_line
                    && gap >= -0.5
                    && gap <= 0.15 * first.size.max(word.size);
                if joins {
                    let right = (rect.x + rect.w).max(word.x + word.w);
                    let bottom = (rect.y + rect.h).max(word.y + word.h);
                    last.text.push_str(&word.text);
                    last.rect.x = rect.x.min(word.x);
                    last.rect.y = rect.y.min(word.y);
                    last.rect.w = right - last.rect.x;
                    last.rect.h = bottom - last.rect.y;
                }
                joins
            }
            _ => false,
        };
        if !joined {
            out.push((
                TextWord {
                    text: word.text.clone(),
                    rect: slidock_bench_metrics::Rect {
                        x: word.x,
                        y: word.y,
                        w: word.w,
                        h: word.h,
                    },
                },
                word,
            ));
        }
        previous = Some(word);
    }
    out.into_iter().map(|(word, _)| word).collect()
}

/// How an export answers the manifest's pin.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Identity {
    /// Its page rasters are the pinned ones.
    Pages,
    /// The manifest pins only the file, and this is it.
    File,
    /// The manifest pins nothing.
    Unpinned,
    /// It is not the pinned export: why.
    Differs(String),
}

/// Whether an export (its file's sha256, and its page rasters' under the raster key `key`) is the one `recorded`
/// pins. Page rasters pin it when recorded under this key; a file sha256 pins it only when no page rasters are.
pub fn identity(
    recorded: Option<&slidock_bench_schema::corpus::StagesOracle>,
    key: &str,
    hashes: &[slidock_bench_schema::Sha256],
    file_sha: &str,
) -> Identity {
    let Some(recorded) = recorded else {
        return Identity::Unpinned;
    };
    if let Some(pages) = recorded.page_hashes.as_ref().and_then(|h| h.get(key)) {
        if pages.as_slice() == hashes {
            return Identity::Pages;
        }
        let differing: Vec<String> = (0..pages.len().max(hashes.len()))
            .filter(|&i| pages.get(i) != hashes.get(i))
            .map(|i| (i + 1).to_string())
            .take(8)
            .collect();
        return Identity::Differs(format!(
            "pages {} differ from the pinned rasters ({} pinned, {} exported)",
            differing.join(", "),
            pages.len(),
            hashes.len()
        ));
    }
    if recorded.page_hashes.as_ref().is_some_and(|h| !h.is_empty()) {
        return Identity::Differs(format!("no page rasters are pinned under raster key {key}"));
    }
    match &recorded.sha256 {
        Some(sha) if sha.as_str() == file_sha => Identity::File,
        Some(sha) => Identity::Differs(format!(
            "the export's sha256 {file_sha} is not the recorded {sha}, and no page rasters are pinned"
        )),
        None => Identity::Unpinned,
    }
}

/// A page of the export: its raster file and its words.
pub struct PageFile {
    pub raster: PathBuf,
    pub words: Vec<TextWord>,
}

/// A page of the export: its raster and its words.
pub struct StagePage {
    pub raster: Rgba,
    pub words: Vec<TextWord>,
}

/// What a deck's positions show.
pub struct PlayedSlide<'a> {
    pub pages: &'a SlidePages,
    /// The settled frame of each position, decoded.
    pub frames: Vec<Option<Rgba>>,
    /// The words of each settled position.
    pub words: Vec<Option<Vec<TextWord>>>,
    /// The frame before the opening builds play (the slide as it appears), for the lead page.
    pub opening: Option<Rgba>,
}

/// Where each side changed between two of its images, all at the page's metric size, compared.
fn compare_changes(base: &Rgba, keynote: (&Rgba, &Rgba), played: (&Rgba, &Rgba)) -> Option<Reveal> {
    let metric = |image: &Rgba| metric_pair(base, image).ok().map(|(_, resized)| resized);
    let changes = |(from, to): (&Rgba, &Rgba)| {
        metric(from)
            .zip(metric(to))
            .and_then(|(a, b)| changed_cells(&a, &b))
    };
    changes(keynote)
        .zip(changes(played))
        .map(|(a, b)| reveal(&a, &b))
}

/// The stage scores of one slide; `page` gives a page of the export, decoded when asked for.
pub fn score_slide(
    played: &PlayedSlide<'_>,
    slide_size: Size,
    page: &dyn Fn(u32) -> Option<StagePage>,
) -> Vec<(u32, Option<StageScore>)> {
    let pages: Vec<Option<StagePage>> = (0..played.pages.positions)
        .map(|k| page(played.pages.page(k)))
        .collect();
    let lead = played
        .pages
        .lead
        .then(|| page(played.pages.first))
        .flatten();
    let last = played.pages.positions.saturating_sub(1) as usize;
    let page_at = |k: usize| pages.get(k).and_then(Option::as_ref);
    let frame_at = |k: usize| played.frames.get(k).and_then(Option::as_ref);
    let mut out = Vec::new();
    for k in 0..played.pages.positions {
        let at = k as usize;
        let (Some(oracle), Some(frame)) = (page_at(at), frame_at(at)) else {
            out.push((k, None));
            continue;
        };
        let empty = Vec::new();
        let words = played
            .words
            .get(at)
            .and_then(Option::as_ref)
            .unwrap_or(&empty);
        let Some(scores) = score_stage(&oracle.raster, frame, &oracle.words, words, slide_size)
        else {
            out.push((k, None));
            continue;
        };
        let base = &oracle.raster;
        // The stage before on each side.
        let (page_before, frame_before) = if at == 0 {
            (lead.as_ref(), played.opening.as_ref())
        } else {
            (page_at(at - 1), frame_at(at - 1))
        };
        let reveal = page_before
            .zip(frame_before)
            .and_then(|(before, frame_before)| {
                compare_changes(base, (&before.raster, base), (frame_before, frame))
            });
        let remaining = if at < last {
            page_at(last)
                .zip(frame_at(last))
                .and_then(|(built, frame_built)| {
                    compare_changes(base, (base, &built.raster), (frame, frame_built))
                })
        } else {
            None
        };
        out.push((
            k,
            Some(StageScore {
                page: played.pages.page(k),
                ssim: quantise(scores.ssim.mean),
                ssim_p1: quantise(scores.ssim.p1),
                color: quantise(scores.color.mean),
                text_recall: (scores.text.oracle_words > 0).then(|| quantise(scores.text.recall)),
                class: class_name(scored_class(&scores)),
                built: at == last,
                frame: None,
                reveal,
                remaining,
                stage: None,
                failed: Vec::new(),
            }),
        ));
    }
    if played.pages.positions > 1 || played.pages.lead {
        judge(&mut out);
    }
    out
}

/// The stage verdicts of a slide with builds: each check a position has agrees.
fn judge(scores: &mut [(u32, Option<StageScore>)]) {
    for (_, score) in scores.iter_mut() {
        let Some(score) = score else { continue };
        for (name, check) in [("reveal", &score.reveal), ("remaining", &score.remaining)] {
            if check.as_ref().is_some_and(|r| !r.agrees) {
                score.failed.push(name.to_owned());
            }
        }
        score.stage = Some(if !score.failed.is_empty() {
            "mismatch"
        } else if score.reveal.is_none() && score.remaining.is_none() {
            "unchecked"
        } else {
            "match"
        });
    }
}

/// A deck's stage oracle, scored.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairStages {
    /// The export's file name and sha256.
    pub export: String,
    pub sha256: String,
    pub pages: u32,
    /// The page count the deck's timelines imply.
    pub expected: u32,
    /// The pages each shown slide prints by the deck's timelines, by slide id: what the manifest's `pagesPerRow`
    /// records.
    pub pages_per_row: BTreeMap<String, u32>,
    /// Whether the manifest records `pagesPerRow` (they then agree, or the status is `stage-count`).
    pub rows_recorded: bool,
    /// `scored`, `oracle-page-count`, `oracle-identity`, `stage-count`, or why the deck is not scored.
    pub status: String,
    /// What pinned the export: `pages` (its page rasters), `file` (its sha256) or `unpinned`.
    pub pinned: String,
    /// The page rasters the manifest pins under the export's raster key.
    pub pinned_pages: u32,
    /// Stage verdicts of the positions of slides with builds.
    #[serde(rename = "match")]
    pub matched: u32,
    pub mismatch: u32,
    /// Positions of slides with builds where neither check ran.
    pub unchecked: u32,
    /// `classifier@2`'s scored class of every position, against its page.
    pub classes: BTreeMap<&'static str, u32>,
    /// The same of each slide's built position (its last): the state the static lane renders.
    pub built_classes: BTreeMap<&'static str, u32>,
    /// Positions with no score (a frame or a page missing).
    pub unscored: u32,
    /// Stages whose reveal agrees, of those with one.
    pub reveals: u32,
    pub reveals_agree: u32,
    /// Stages whose remaining check agrees, of those with one.
    pub remainings: u32,
    pub remainings_agree: u32,
    /// The rows whose stage verdict is `mismatch`.
    pub mismatched: Vec<String>,
    /// Per row id.
    pub rows: BTreeMap<String, StageScore>,
}

impl PairStages {
    pub fn add(&mut self, row: String, score: Option<StageScore>) {
        let Some(score) = score else {
            self.unscored += 1;
            return;
        };
        *self.classes.entry(score.class).or_default() += 1;
        if score.built {
            *self.built_classes.entry(score.class).or_default() += 1;
        }
        if let Some(reveal) = &score.reveal {
            self.reveals += 1;
            self.reveals_agree += u32::from(reveal.agrees);
        }
        if let Some(remaining) = &score.remaining {
            self.remainings += 1;
            self.remainings_agree += u32::from(remaining.agrees);
        }
        match score.stage {
            Some("match") => self.matched += 1,
            Some("unchecked") => self.unchecked += 1,
            Some(_) => {
                self.mismatch += 1;
                self.mismatched.push(row.clone());
            }
            None => {}
        }
        self.rows.insert(row, score);
    }

    /// Whether the stage oracle fails the lane: a stage it does not match, or an export it cannot use (not the pinned
    /// one, pages that do not add up, or rows that are not the recorded ones). A missing export is only pending.
    pub fn fails(&self) -> bool {
        self.mismatch > 0
            || self.status.starts_with("oracle-")
            || self.status.starts_with("stage-count")
    }

    /// How the export is pinned, as the lane prints it.
    pub fn pinned_text(&self) -> String {
        match self.pinned.as_str() {
            "pages" => format!("pinned by its {} page rasters", self.pinned_pages),
            "file" => "pinned by its sha256 only".to_owned(),
            _ => "not pinned".to_owned(),
        }
    }

    /// The line the lane prints.
    pub fn line(&self, pair: &str) -> String {
        if self.status != "scored" {
            return format!("stages    {pair}: {}", self.status);
        }
        let class = |name: &str| self.classes.get(name).copied().unwrap_or(0);
        let built = |name: &str| self.built_classes.get(name).copied().unwrap_or(0);
        format!(
            "stages    {pair}: {} pages ({}, {}; pagesPerRow {}); stages match {}, mismatch {}, unchecked {}; reveal agrees {}/{}, remaining agrees {}/{}; every position by class: match {}, close {}, mismatch {}; built positions: match {}, close {}, mismatch {}; unscored {}",
            self.pages,
            self.export,
            self.pinned_text(),
            if self.rows_recorded {
                "as recorded"
            } else {
                "not recorded"
            },
            self.matched,
            self.mismatch,
            self.unchecked,
            self.reveals_agree,
            self.reveals,
            self.remainings_agree,
            self.remainings,
            class("match"),
            class("close"),
            class("mismatch"),
            built("match"),
            built("close"),
            built("mismatch"),
            self.unscored,
        )
    }
}

/// The all-stages export of a source in `dir/<sha>/`: the one PDF there (the newest Keynote's when several).
pub fn export_of(dir: &Path, source_sha: &str) -> Option<PathBuf> {
    let folder = dir.join(source_sha);
    let mut pdfs: Vec<PathBuf> = std::fs::read_dir(&folder)
        .ok()?
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "pdf"))
        .collect();
    pdfs.sort();
    pdfs.pop()
}

/// The page words in slide points.
pub fn page_words(
    text: &slidock_bench_metrics::OracleText,
    page: Size,
    slide: Size,
) -> Vec<TextWord> {
    oracle_words(text, page, slide)
}

/// Decodes a PNG file.
pub fn decode_file(path: &Path) -> Option<Rgba> {
    decode_png(&std::fs::read(path).ok()?).ok()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion_lane::deck::{DeckSlide, DeckStep, Target};

    fn step(trigger: &str) -> DeckStep {
        DeckStep {
            target: Target {
                element: "e".into(),
                paragraphs: None,
                background: None,
            },
            trigger: trigger.into(),
            kind: "entrance".into(),
            delivery: None,
            delay_ms: 0.into(),
            duration_ms: 500.into(),
            native: String::new(),
            fallback: "fade".into(),
            effect: None,
            params: None,
            offset_ms: None,
            repeat: None,
            auto_reverse: None,
            stagger: None,
            after: None,
        }
    }

    fn slide(id: &str, hidden: bool, triggers: &[&str]) -> DeckSlide {
        DeckSlide {
            id: id.into(),
            hidden,
            timeline: triggers.iter().map(|t| step(t)).collect(),
            transition: None,
            match_keys: None,
        }
    }

    fn deck(slides: Vec<DeckSlide>) -> MotionDeck {
        MotionDeck {
            size: super::super::deck::DeckSize { w: 960.0, h: 540.0 },
            slides,
            index: slidock_bench_schema::Sha256::of(b""),
        }
    }

    #[test]
    fn pages_follow_clicks_with_a_lead_page_for_opening_automatic_builds() {
        let deck = deck(vec![
            // No builds: one page.
            slide("a", false, &[]),
            // Opening automatic build, then a click whose chain goes on by itself: lead + 2.
            slide("b", false, &["afterPrevious", "click", "afterPrevious"]),
            // Skipped: nothing.
            slide("c", true, &["click"]),
            // Two clicks, one with a build alongside it: 3 pages.
            slide("d", false, &["click", "withPrevious", "click"]),
            // A movie that starts by itself: lead + 1.
            slide("e", false, &["afterPrevious"]),
        ]);
        let (pages, total) = stage_pages(&deck);
        let summary: Vec<(&str, u32, bool, u32)> = pages
            .iter()
            .map(|p| (p.slide.as_str(), p.first, p.lead, p.positions))
            .collect();
        assert_eq!(
            summary,
            [
                ("a", 1, false, 1),
                ("b", 2, true, 2),
                ("d", 5, false, 3),
                ("e", 8, true, 1)
            ]
        );
        assert_eq!(total, 9);
        // Position k of a slide with a lead page skips it.
        assert_eq!(pages[1].page(0), 3);
        assert_eq!(pages[1].page(1), 4);
        assert_eq!(pages[2].page(2), 7);
        // The IR of the same deck gives the same pages: what `oracle --stages --record` records.
        let ir: Vec<IrSlide> = deck
            .slides
            .iter()
            .map(|slide| {
                serde_json::from_value(serde_json::json!({
                    "id": slide.id,
                    "hidden": slide.hidden,
                    "timeline": slide.timeline.iter().map(|s| serde_json::json!({
                        "target": {"element": "e"}, "trigger": s.trigger, "kind": "entrance",
                        "effect": {"id": "keynote:apple:dissolve", "fallback": "dissolve"}
                    })).collect::<Vec<_>>()
                }))
                .unwrap()
            })
            .collect();
        assert_eq!(ir_stage_pages(&ir), (pages.clone(), total));
        assert_eq!(
            pages_per_row(&pages),
            BTreeMap::from([
                ("a".to_owned(), 1),
                ("b".to_owned(), 3),
                ("d".to_owned(), 3),
                ("e".to_owned(), 2)
            ])
        );
    }

    #[test]
    fn rows_add_up_to_the_export_and_match_their_record() {
        let rows = BTreeMap::from([("kn-1".to_owned(), 1), ("kn-2".to_owned(), 3)]);
        let id = |s: &str| SlideId::new(s.to_owned()).unwrap();
        let recorded = BTreeMap::from([(id("kn-1"), 1), (id("kn-2"), 3)]);
        assert_eq!(check_rows(Some(&recorded), &rows, 4), Rows::Recorded);
        assert_eq!(check_rows(None, &rows, 4), Rows::Unrecorded);
        // The sum is the export's page count first.
        assert_eq!(
            check_rows(Some(&recorded), &rows, 5),
            Rows::PageCount("the export has 5 pages, the deck's timelines imply 4".into())
        );
        // A click moved from one slide to another keeps the sum, not the rows.
        let moved = BTreeMap::from([(id("kn-1"), 2), (id("kn-2"), 2)]);
        assert_eq!(
            check_rows(Some(&moved), &rows, 4),
            Rows::Differs(
                "kn-1 prints 2 pages, its timeline gives 1; kn-2 prints 2 pages, its timeline gives 3".into()
            )
        );
        let short = BTreeMap::from([(id("kn-1"), 2), (id("kn-2"), 1)]);
        assert_eq!(
            check_rows(Some(&short), &rows, 4),
            Rows::Differs(
                "kn-1 prints 2 pages, its timeline gives 1; kn-2 prints 1 page, its timeline gives 3"
                    .into()
            )
        );
        // A slide shown now that the record skipped, and one recorded that is no longer shown.
        let other = BTreeMap::from([(id("kn-1"), 1), (id("kn-3"), 3)]);
        assert_eq!(
            check_rows(Some(&other), &rows, 4),
            Rows::Differs("kn-2 is not recorded; kn-3 is recorded, not shown".into())
        );
    }

    #[test]
    fn a_stage_oracle_fails_on_a_mismatch_or_an_export_it_cannot_use() {
        let mut report = PairStages {
            status: "scored".into(),
            pinned: "pages".into(),
            pinned_pages: 255,
            ..PairStages::default()
        };
        assert!(!report.fails());
        assert_eq!(report.pinned_text(), "pinned by its 255 page rasters");
        report.mismatch = 1;
        assert!(report.fails());
        report.mismatch = 0;
        for status in [
            "oracle-identity: x",
            "oracle-page-count: x",
            "stage-count: x",
        ] {
            report.status = status.into();
            assert!(report.fails(), "{status}");
        }
        report.status = "pending: no all-stages export".into();
        assert!(!report.fails());
        report.pinned = "file".into();
        assert_eq!(report.pinned_text(), "pinned by its sha256 only");
    }

    fn image(width: usize, height: usize, dark: &[(usize, usize)]) -> Rgba {
        let mut data = vec![255u8; width * height * 4];
        for (x, y) in dark {
            let at = (y * width + x) * 4;
            data[at..at + 3].copy_from_slice(&[0, 0, 0]);
        }
        Rgba::new(width, height, data).unwrap()
    }

    #[test]
    fn a_reveal_agrees_where_both_sides_changed_in_the_same_place() {
        let blank = image(96, 54, &[]);
        let left: Vec<(usize, usize)> =
            (0..10).flat_map(|x| (0..10).map(move |y| (x, y))).collect();
        let right: Vec<(usize, usize)> = (80..90)
            .flat_map(|x| (0..10).map(move |y| (x, y)))
            .collect();
        let keynote = changed_cells(&blank, &image(96, 54, &left)).unwrap();
        let same = changed_cells(&blank, &image(96, 54, &left)).unwrap();
        let elsewhere = changed_cells(&blank, &image(96, 54, &right)).unwrap();
        assert!(reveal(&keynote, &same).agrees);
        let wrong = reveal(&keynote, &elsewhere);
        assert!(!wrong.agrees);
        assert_eq!((wrong.keynote_covered, wrong.played_covered), (0.0, 0.0));
        // The same build drawn a little lower and to the right (another face's metrics) still agrees.
        let nudged: Vec<(usize, usize)> =
            (4..14).flat_map(|x| (3..13).map(move |y| (x, y))).collect();
        assert!(
            reveal(
                &keynote,
                &changed_cells(&blank, &image(96, 54, &nudged)).unwrap()
            )
            .agrees
        );
        // Nothing changed on either side agrees; one side changing alone does not.
        let none = changed_cells(&blank, &blank).unwrap();
        assert!(reveal(&none, &none).agrees);
        assert!(!reveal(&keynote, &none).agrees);
        assert!(changed_cells(&blank, &image(10, 10, &[])).is_none());
    }

    fn check(agrees: bool) -> Reveal {
        Reveal {
            keynote: 1,
            played: 1,
            keynote_covered: if agrees { 1.0 } else { 0.0 },
            played_covered: if agrees { 1.0 } else { 0.0 },
            agrees,
        }
    }

    fn scored(reveal: Option<bool>, remaining: Option<bool>) -> Option<StageScore> {
        Some(StageScore {
            page: 1,
            ssim: 1.0,
            ssim_p1: 1.0,
            color: 0.0,
            text_recall: None,
            class: "mismatch",
            built: false,
            frame: None,
            reveal: reveal.map(check),
            remaining: remaining.map(check),
            stage: None,
            failed: Vec::new(),
        })
    }

    #[test]
    fn a_stage_matches_when_each_check_it_has_agrees() {
        let mut scores = vec![
            // Position 0 of a slide with no lead page: what is still to come.
            (0, scored(None, Some(true))),
            (1, scored(Some(true), Some(false))),
            (2, scored(Some(false), Some(true))),
            // A neighbour's frame missing: neither check ran.
            (3, scored(None, None)),
            // The slide built: only its reveal.
            (4, scored(Some(true), None)),
        ];
        judge(&mut scores);
        let verdicts: Vec<(Option<&str>, Vec<String>)> = scores
            .iter()
            .map(|(_, s)| {
                let s = s.as_ref().unwrap();
                (s.stage, s.failed.clone())
            })
            .collect();
        assert_eq!(verdicts[0], (Some("match"), vec![]));
        assert_eq!(
            verdicts[1],
            (Some("mismatch"), vec!["remaining".to_owned()])
        );
        assert_eq!(verdicts[2], (Some("mismatch"), vec!["reveal".to_owned()]));
        assert_eq!(verdicts[3], (Some("unchecked"), vec![]));
        assert_eq!(verdicts[4].0, Some("match"));
        let mut pair = PairStages::default();
        for (k, score) in scores {
            pair.add(format!("p:s:keynote#s{k}"), score);
        }
        assert_eq!((pair.matched, pair.mismatch, pair.unchecked), (2, 2, 1));
        assert_eq!(pair.mismatched, ["p:s:keynote#s1", "p:s:keynote#s2"]);
        assert_eq!((pair.reveals, pair.reveals_agree), (3, 2));
        assert_eq!((pair.remainings, pair.remainings_agree), (3, 2));
    }

    #[test]
    fn a_start_state_that_shows_a_build_too_early_fails_at_position_0() {
        let (w, h) = (192, 108);
        let box_at: Vec<(usize, usize)> = (10..50)
            .flat_map(|x| (10..50).map(move |y| (x, y)))
            .collect();
        let blank = image(w, h, &[]);
        let built = image(w, h, &box_at);
        // One click builds the box in: two positions, no lead page.
        let pages = SlidePages {
            index: 0,
            slide: "s".into(),
            first: 1,
            lead: false,
            positions: 2,
        };
        let keynote = [blank.clone(), built.clone()];
        let page = |n: u32| {
            keynote.get(n as usize - 1).map(|raster| StagePage {
                raster: raster.clone(),
                words: Vec::new(),
            })
        };
        let size = Size { w: 960.0, h: 540.0 };
        let verdicts = |frames: Vec<Rgba>| -> Vec<(Option<&'static str>, Vec<String>)> {
            let played = PlayedSlide {
                pages: &pages,
                frames: frames.into_iter().map(Some).collect(),
                words: vec![None, None],
                opening: None,
            };
            score_slide(&played, size, &page)
                .into_iter()
                .map(|(_, s)| {
                    let s = s.unwrap();
                    (s.stage, s.failed)
                })
                .collect()
        };
        assert_eq!(
            verdicts(vec![blank.clone(), built.clone()]),
            [(Some("match"), vec![]), (Some("match"), vec![])]
        );
        // The box already shows before the click: position 0 has nothing left to build where Keynote does.
        assert_eq!(
            verdicts(vec![built.clone(), built]),
            [
                (Some("mismatch"), vec!["remaining".to_owned()]),
                (Some("mismatch"), vec!["reveal".to_owned()])
            ]
        );
    }

    #[test]
    fn every_position_and_each_built_one_count_their_class() {
        let class = |class: &'static str, last: bool| {
            let mut score = scored(None, None).unwrap();
            score.class = class;
            score.built = last;
            Some(score)
        };
        let mut pair = PairStages {
            status: "scored".into(),
            ..PairStages::default()
        };
        // A slide with no builds: its one position is built.
        pair.add("p:a:keynote#s0".into(), class("mismatch", true));
        // A slide of three positions, built close.
        for (k, c, last) in [
            (0, "match", false),
            (1, "mismatch", false),
            (2, "close", true),
        ] {
            pair.add(format!("p:b:keynote#s{k}"), class(c, last));
        }
        pair.add("p:b:keynote#s3".into(), None);
        assert_eq!(
            pair.classes,
            BTreeMap::from([("close", 1), ("match", 1), ("mismatch", 2)])
        );
        assert_eq!(
            pair.built_classes,
            BTreeMap::from([("close", 1), ("mismatch", 1)])
        );
        assert!(pair.line("p:keynote").ends_with(
            "every position by class: match 1, close 1, mismatch 2; built positions: match 0, close 1, mismatch 1; unscored 1"
        ));
    }

    #[test]
    fn a_word_split_across_nodes_reads_as_one() {
        // `Object.is(0,` drawn as four runs of one text box at 90 pt, then `-0)` after a space.
        let node = |text: &str, x: f64, w: f64, before: bool, after: bool| StageWord {
            text: text.into(),
            x,
            y: 100.0,
            w,
            h: 120.0,
            join_before: before,
            join_after: after,
            el: Some("kn-1".into()),
            text_box: Some(0),
            size: 90.0,
        };
        let line = [
            node("Object", 10.0, 324.0, true, true),
            node(".", 334.0, 54.0, true, true),
            node("is", 388.0, 108.0, true, true),
            node("(0,", 496.0, 162.0, true, false),
            node("-0)", 712.0, 162.0, false, true),
        ];
        let texts = |words: &[StageWord]| -> Vec<String> {
            frame_words(words).into_iter().map(|w| w.text).collect()
        };
        assert_eq!(texts(&line), ["Object.is(0,", "-0)"]);
        let joined = &frame_words(&line)[0].rect;
        assert_eq!((joined.x, joined.w), (10.0, 648.0));
        // Not joined: another element, another text box, a gap wider than 0.15 em, another line, or whitespace.
        for apart in [
            StageWord {
                el: Some("kn-2".into()),
                ..node(".", 334.0, 54.0, true, true)
            },
            StageWord {
                text_box: Some(1),
                ..node(".", 334.0, 54.0, true, true)
            },
            node(".", 334.0 + 14.0, 54.0, true, true),
            StageWord {
                y: 220.0,
                ..node(".", 334.0, 54.0, true, true)
            },
            node(".", 334.0, 54.0, false, true),
        ] {
            let words = [node("Object", 10.0, 324.0, true, true), apart];
            assert_eq!(texts(&words), ["Object", "."]);
        }
        // Nor a word followed by whitespace in its own node.
        let words = [
            node("Object", 10.0, 324.0, true, false),
            node(".", 334.0, 54.0, true, true),
        ];
        assert_eq!(texts(&words), ["Object", "."]);
    }

    #[test]
    fn score_slide_marks_the_last_position_built() {
        let (w, h) = (192, 108);
        let box_at: Vec<(usize, usize)> = (10..50)
            .flat_map(|x| (10..50).map(move |y| (x, y)))
            .collect();
        let blank = image(w, h, &[]);
        let built = image(w, h, &box_at);
        let pages = SlidePages {
            index: 0,
            slide: "s".into(),
            first: 1,
            lead: false,
            positions: 2,
        };
        let keynote = [blank.clone(), built.clone()];
        let page = |n: u32| {
            keynote.get(n as usize - 1).map(|raster| StagePage {
                raster: raster.clone(),
                words: Vec::new(),
            })
        };
        let played = PlayedSlide {
            pages: &pages,
            frames: vec![Some(blank), Some(built)],
            words: vec![None, None],
            opening: None,
        };
        let scores: Vec<(u32, bool)> = score_slide(&played, Size { w: 960.0, h: 540.0 }, &page)
            .into_iter()
            .map(|(k, s)| (k, s.unwrap().built))
            .collect();
        assert_eq!(scores, [(0, false), (1, true)]);
    }

    fn pin(
        sha: Option<&str>,
        pages: Option<(&str, &[slidock_bench_schema::Sha256])>,
    ) -> slidock_bench_schema::corpus::StagesOracle {
        serde_json::from_value(serde_json::json!({
            "kind": "keynote-stages",
            "exporter": {"bundleId": "com.apple.iWork.Keynote", "version": "14.5", "build": "7045.0.17"},
        }))
        .map(|mut oracle: slidock_bench_schema::corpus::StagesOracle| {
            oracle.sha256 = sha.map(|s| slidock_bench_schema::Sha256::new(s.to_owned()).unwrap());
            oracle.page_hashes = pages.map(|(key, hashes)| {
                std::iter::once((key.to_owned(), hashes.to_vec())).collect()
            });
            oracle
        })
        .unwrap()
    }

    #[test]
    fn page_rasters_pin_an_export_whatever_its_file_bytes() {
        let a = slidock_bench_schema::Sha256::of(b"page 1");
        let b = slidock_bench_schema::Sha256::of(b"page 2");
        let file = slidock_bench_schema::Sha256::of(b"export")
            .as_str()
            .to_owned();
        let other = slidock_bench_schema::Sha256::of(b"re-export")
            .as_str()
            .to_owned();
        let pages = [a.clone(), b.clone()];
        let pinned = pin(Some(&file), Some(("k", &pages)));
        // A re-export (another creation date) with the same rasters is the pinned oracle.
        assert_eq!(
            identity(Some(&pinned), "k", &pages, &other),
            Identity::Pages
        );
        // Rasters that moved are not, whatever the file.
        let moved = [a.clone(), a.clone()];
        assert!(matches!(
            identity(Some(&pinned), "k", &moved, &file),
            Identity::Differs(why) if why.starts_with("pages 2 differ")
        ));
        assert!(matches!(
            identity(Some(&pinned), "k", &pages[..1], &file),
            Identity::Differs(_)
        ));
        // Rasters pinned under another raster key (another poppler, another size) do not pin these.
        assert!(matches!(
            identity(Some(&pinned), "other", &pages, &file),
            Identity::Differs(_)
        ));
        // With no rasters pinned, the file's sha256 pins it.
        let by_file = pin(Some(&file), None);
        assert_eq!(identity(Some(&by_file), "k", &pages, &file), Identity::File);
        assert!(matches!(
            identity(Some(&by_file), "k", &pages, &other),
            Identity::Differs(_)
        ));
        assert_eq!(identity(None, "k", &pages, &file), Identity::Unpinned);
        assert_eq!(
            identity(Some(&pin(None, None)), "k", &pages, &file),
            Identity::Unpinned
        );
    }
}
