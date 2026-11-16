//! The motion lane: every deck of the corpus and every probe deck of
//! `bench/motion/probes.json`, bundled as the product bundles it (the deck path, with its native motion), played by
//! its own player in the renderer with JavaScript on and effects in full ([`drive`]).
//!
//! - **Frames.** The player's Web Animations are paused as they are created and seeked: no wall clock decides a
//!   frame. Each transition is captured at 0, 250, 500, 750 and 1000 ‰ of its length, each stage's batch at its
//!   start, middle and end and each of its steps at its own start, quarters and end, with what every animated node
//!   shows at each; then the stage settles and the player's still render of the same position is taken.
//! - **Invariants** ([`check`]): a stage's end frame and settled frame equal the still render, its animated nodes end
//!   as they settle; its first frame is the state before it, every unit showing as before; what shows is what the
//!   timeline gives ([`model`]), and nothing changes but by a step of the stage; every step and transition that
//!   animates moves between its ends, the way its `dir` says; a transition starts on the outgoing slide and ends on
//!   the incoming one, and a Magic Move's keys pair the slides it moves between.
//! - **Timeline** ([`check`]): the animations attributed to each step against the deck data (group, order, start,
//!   duration, effect or fallback), the transition's length, the player's allow-list of animated properties, and
//!   the deck's timeline against the IR the deck path compiles (`--emit ir --native-motion`, which must compile)
//!   where the IR carries one.
//! - **Verdicts** ([`verdict`]): each transition and step is native, a declared or accepted fallback, accepted with a
//!   reviewed defect, dropped, or mismatched; a probe deck is held to its expectations, params included. The lane
//!   passes when nothing is mismatched, no row is an unaccepted `motion-mismatch`, nothing failed to play or compile,
//!   and no entry of `bench/motion/fallback-allow.json` is stale.
//!
//! Rows: per slide with a timeline or a transition, `#s<k>` for each position k (the stage's invariants, a motion
//! class where no stages oracle scores it) and `#t@250|500|750` for its transition. A motion run is its own run
//! folder: `bundles/<p>.<fe>/`, `ir/<p>.<fe>/`, `frames/<sha256>.png` and `results.json`.

pub mod check;
pub mod deck;
pub mod drive;
pub mod model;
pub mod movies;
pub mod shell;
pub mod stages;
pub mod verdict;

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use serde_json::Value;
use slidock_bench_metrics::{
    CLASSIFIER_VERSION, COLOR_VERSION, PIXEL_VERSION, SSIM_VERSION, TEXT_VERSION,
};
use slidock_bench_schema::corpus::CorpusManifest;
use slidock_bench_schema::lanes::{
    ANTIALIAS_CHANNEL, ANTIALIAS_PIXELS, EffectCounts, EffectPlay, FrameMatch, LaneSummaries,
    MotionClasses, MotionPairReport, MotionSummary, MotionVerdict, TimelineFidelity, TimelineField,
    TimelineMismatch, VerdictCounts,
};
use slidock_bench_schema::results::{
    MotionOutcome, OraclePage, Outcome, Reason, Results, ResultsSchema, RowOutcome, RowRecord,
};
use slidock_bench_schema::stamp::{FontsStamp, MetricsStamp, Selection as StampSelection, Stamp};
use slidock_bench_schema::{
    Frontend, Lane, PairKey, PresentationId, RowId, RowStep, Sha256, SlideId, Tier,
};

use crate::cache::remove;
use crate::compile::{COMPILE_TIMEOUT, CompileStatus, compiler_binary, run_compiler_with};
use crate::corpus::{Corpus, Selection, load_manifest, refuse_holdout, select_pairs};
use crate::error::{BenchError, Exit, Result};
use crate::fonts::{check_font_set, load_font_lock};
use crate::fonts_lane::lock::{LANE_LOCK, LaneFonts, prepare};
use crate::fonts_lane::{Running, deck_arguments, lane_env};
use crate::hash::{FileHashes, sha256_file};
use crate::images::{Change, change, displaced_specks, metrics_match, render_vs_render, specks};
use crate::renderer::native::{NativeBrowser, default_binary};
use crate::renderer::route::{DeckRouter, deck_url};
use crate::renderer::session::Browser;
use crate::results::{results_path, summary, write_results};
use crate::setup::Context;
use crate::stamp::{compiler_stamp, deps_stamp, harness_hash, host_stamp};

use self::check::{Frames, Item, ItemRef, Problem, SlideCheck, SlideInput, check_slide};
use self::deck::{IrSlide, MotionDeck, read_deck, read_ir};
use self::drive::{FrameStore, SlideSeen, WalkRequest, frames_dir, runs, walk};
use self::verdict::{
    Allowlist, Expectation, FALLBACK_ALLOW, PROBES, SlideItems, Verdicts, expectations,
    load_allowlist, load_probes, params_problems,
};

/// `motion@1`.
pub const MOTION_VERSION: &str = "motion@1";
/// Shown slides one page walks.
const RUN_SLIDES: usize = 24;

/// Where the lane renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererChoice {
    /// The renderer container (the static lane's, with the pinned font set).
    Container,
    /// The same Chromium build on the host.
    Native(Option<PathBuf>),
}

/// What the motion lane runs.
pub struct MotionRequest {
    pub tiers: Vec<Tier>,
    pub presentations: Option<Vec<PresentationId>>,
    pub frontends: Option<Vec<Frontend>>,
    pub concurrency: usize,
    pub renderer: RendererChoice,
}

/// A deck the lane plays.
pub struct LanePair {
    pub pair: PairKey,
    pub file: PathBuf,
    pub digest: Sha256,
    /// A probe deck's expectations.
    pub expected: Option<Vec<Expectation>>,
    /// A Keynote deck's all-stages export, when one is on disk.
    pub stages: Option<PathBuf>,
    /// A Keynote deck's movie export, when one is on disk.
    pub movie: Option<PathBuf>,
    /// A deck of the corpus manifest (one whose stages the lane scores), not a probe or a private deck.
    pub corpus: bool,
}

impl LanePair {
    /// Whether Keynote's own frames score the deck: an all-stages or a movie export.
    pub fn scored_by_keynote(&self) -> bool {
        self.stages.is_some() || self.movie.is_some()
    }
}

/// A private deck the probe tier plays when it is on disk: a file under `$SLIDOCK_PRIVATE`, pinned by sha256, never
/// committed. Without the file the lane skips it and says so.
pub struct PrivateDeck {
    pub id: &'static str,
    pub frontend: Frontend,
    /// Its path under `$SLIDOCK_PRIVATE`.
    pub path: &'static str,
    pub sha256: &'static str,
}

/// The private decks: Keynote's own motion paths (65 builds, slide 65's custom path among them), which no committed
/// deck holds.
pub const PRIVATE_DECKS: [PrivateDeck; 1] = [PrivateDeck {
    id: "build-subset",
    frontend: Frontend::Keynote,
    path: "probes-incoming/keynote/build-subset.key",
    sha256: "4a2ecbb67214900a7ecde2658f6870cfe1b0d80a15002ef0ec7f3682a6fc6f09",
}];

/// `$SLIDOCK_PRIVATE`, or `~/.slidock`, from the variables `lookup` reads (an empty value counts as unset).
pub fn private_root(lookup: impl Fn(&str) -> Option<String>) -> Option<PathBuf> {
    let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
    var("SLIDOCK_PRIVATE")
        .map(PathBuf::from)
        .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".slidock")))
}

/// The private decks a selection names that are on disk, each with its file; one that is not on disk is left out with
/// a line saying so, and one whose sha256 differs is refused.
pub fn private_pairs(
    decks: &[PrivateDeck],
    root: Option<&Path>,
    presentations: Option<&Vec<PresentationId>>,
    frontends: Option<&Vec<Frontend>>,
) -> Result<Vec<LanePair>> {
    let mut pairs = Vec::new();
    for deck in decks {
        let id = PresentationId::new(deck.id).map_err(|e| BenchError::failed(e.to_string()))?;
        if !presentations.is_none_or(|p| p.contains(&id))
            || !frontends.is_none_or(|f| f.contains(&deck.frontend))
        {
            continue;
        }
        let pair = PairKey::new(id, deck.frontend);
        let Some(file) = root
            .map(|root| root.join(deck.path))
            .filter(|f| f.is_file())
        else {
            eprintln!(
                "skipped    {pair}: the private deck $SLIDOCK_PRIVATE/{} is not on disk",
                deck.path
            );
            continue;
        };
        let actual = sha256_file(&file)?;
        if actual != deck.sha256 {
            return Err(BenchError::refused(format!(
                "{}: sha256 {actual}, the lane pins {}",
                file.display(),
                deck.sha256
            )));
        }
        pairs.push(LanePair {
            pair,
            file,
            digest: slidock_bench_schema::sha256_json(&(deck.path, deck.sha256))?,
            expected: None,
            stages: None,
            movie: None,
            corpus: false,
        });
    }
    Ok(pairs)
}

/// The corpus pairs and probe decks a request selects.
pub fn lane_pairs(
    ctx: &Context,
    manifest: &CorpusManifest,
    tiers: &[Tier],
    presentations: Option<&Vec<PresentationId>>,
    frontends: Option<&Vec<Frontend>>,
) -> Result<Vec<LanePair>> {
    let selection = Selection {
        tiers: tiers.to_vec(),
        presentations: presentations.cloned(),
        frontends: frontends.cloned(),
    };
    let mut pairs = Vec::new();
    let mut corpus: Option<Corpus> = None;
    for (presentation, source) in select_pairs(manifest, &selection) {
        let corpus = match &mut corpus {
            Some(corpus) => corpus,
            None => corpus.insert(Corpus::new(
                ctx.env.require_corpus_dir()?,
                FileHashes::new(&ctx.cache.hashes, true),
                manifest,
            )),
        };
        let pair = presentation.pair(source.frontend);
        let file = corpus.source_file(source)?.ok_or_else(|| {
            BenchError::failed(format!("{pair}: {} is not in the corpus", source.file))
        })?;
        let Some((_, digest)) = presentation.corpus_stamp(source.frontend)? else {
            continue;
        };
        let keynote_sha = (source.frontend == Frontend::Keynote)
            .then_some(source.sha256.as_ref())
            .flatten();
        let stages =
            keynote_sha.and_then(|sha| stages::export_of(&ctx.env.stages_dir, sha.as_str()));
        let movie = keynote_sha.and_then(|sha| movies::movie_of(&ctx.env.movies_dir, sha.as_str()));
        pairs.push(LanePair {
            pair,
            file,
            digest,
            expected: None,
            stages,
            movie,
            corpus: true,
        });
    }
    if tiers.contains(&Tier::Probe) {
        let repo = &ctx.env.repo_root;
        for probe in load_probes(&repo.join(PROBES))?.probes {
            let named = presentations.is_none_or(|p| p.contains(&probe.id))
                && frontends.is_none_or(|f| f.contains(&probe.frontend));
            if !named {
                continue;
            }
            let file = repo.join(&probe.file);
            let actual = sha256_file(&file)?;
            if actual != probe.sha256.as_str() {
                return Err(BenchError::refused(format!(
                    "{}: sha256 {actual}, {PROBES} says {}",
                    probe.file, probe.sha256
                )));
            }
            let expect_path = repo.join(&probe.expect);
            let text =
                std::fs::read_to_string(&expect_path).map_err(BenchError::io(&expect_path))?;
            let expect: Value = serde_json::from_str(&text)
                .map_err(|e| BenchError::failed(format!("{}: {e}", expect_path.display())))?;
            let expected = expectations(&expect, &probe.deck)
                .map_err(|why| BenchError::failed(format!("{}: {why}", expect_path.display())))?;
            pairs.push(LanePair {
                pair: PairKey::new(probe.id.clone(), probe.frontend),
                file,
                digest: slidock_bench_schema::sha256_json(&(
                    &probe.file,
                    &probe.sha256,
                    &probe.expect,
                ))?,
                expected: Some(expected),
                stages: None,
                movie: None,
                corpus: false,
            });
        }
        let root = private_root(|name| std::env::var(name).ok());
        pairs.extend(private_pairs(
            &PRIVATE_DECKS,
            root.as_deref(),
            presentations,
            frontends,
        )?);
    }
    Ok(pairs)
}

/// Whether the deck's timeline and transition agree with the IR's for one slide: `None` when the IR holds neither.
pub fn compare_ir(
    slide: &deck::DeckSlide,
    ir: Option<&IrSlide>,
) -> (Option<bool>, Vec<TimelineMismatch>) {
    let Some(ir) = ir else {
        return (None, Vec::new());
    };
    if ir.timeline.is_empty() && ir.transition.is_none() {
        return (None, Vec::new());
    }
    let mut out = Vec::new();
    let mut differ = |step: Option<u32>, expected: String, actual: String| {
        out.push(TimelineMismatch {
            step,
            field: TimelineField::Ir,
            expected,
            actual,
        });
    };
    if !ir.timeline.is_empty() {
        if ir.timeline.len() != slide.timeline.len() {
            differ(
                None,
                format!("{} steps", ir.timeline.len()),
                format!("{} steps", slide.timeline.len()),
            );
        }
        for (index, (want, have)) in ir.timeline.iter().zip(&slide.timeline).enumerate() {
            // The structure compares; the effect itself is held to what the player plays.
            let a = step_text(
                &want.target,
                &want.trigger,
                &want.kind,
                want.delivery.as_deref(),
                &want.delay_ms,
                &want.duration_ms,
                &want.effect.fallback,
            );
            let b = step_text(
                &have.target,
                &have.trigger,
                &have.kind,
                have.delivery.as_deref(),
                &have.delay_ms,
                &have.duration_ms,
                &have.fallback,
            );
            if a != b {
                differ(Some(index as u32), a, b);
            }
        }
    }
    if let Some(want) = &ir.transition {
        let a = format!("{} over {} ms", want.effect.fallback, want.duration_ms);
        let b = slide.transition.as_ref().map_or_else(
            || "none".to_owned(),
            |have| format!("{} over {} ms", have.fallback, have.duration_ms),
        );
        if a != b {
            differ(None, a, b);
        }
    }
    (Some(out.is_empty()), out)
}

/// A step's structure as the IR and the deck both carry it.
fn step_text(
    target: &deck::Target,
    trigger: &str,
    kind: &str,
    delivery: Option<&str>,
    delay: &Value,
    duration: &Value,
    fallback: &str,
) -> String {
    format!(
        "{} {trigger} {kind} {} delay {delay} duration {duration} fallback {fallback}",
        target.label(),
        delivery.unwrap_or("all")
    )
}

/// One row's record.
fn record(id: &RowId, page: u32, content: &Sha256) -> RowRecord {
    RowRecord {
        presentation: id.presentation().clone(),
        slide: id.slide().clone(),
        frontend: id.frontend(),
        oracle: OraclePage { page: page.max(1) },
        outcome: Outcome::Failed.into(),
        reason: None,
        code: None,
        data: None,
        declined: None,
        scores: None,
        diagnostics: BTreeMap::new(),
        html: None,
        render: None,
        content: content.clone(),
        reference: None,
        known_loss: None,
        stage: None,
        transition_pm: None,
        host: None,
        fonts: None,
        motion: None,
        shadow: None,
        gradient: None,
        shell: None,
    }
}

/// Per frontend: rows by class, end states, timelines and verdicts.
pub fn motion_summary(
    rows: &BTreeMap<RowId, RowRecord>,
    timelines: &BTreeMap<RowId, TimelineFidelity>,
    pairs: &BTreeMap<PairKey, MotionPairReport>,
) -> BTreeMap<Frontend, MotionSummary> {
    let mut out: BTreeMap<Frontend, MotionSummary> = BTreeMap::new();
    let count = |classes: &mut MotionClasses, outcome: MotionOutcome| {
        classes.rows += 1;
        match outcome {
            MotionOutcome::MotionMatch => classes.motion_match += 1,
            MotionOutcome::MotionClose => classes.motion_close += 1,
            MotionOutcome::MotionMismatch => classes.motion_mismatch += 1,
            MotionOutcome::MotionFallback => classes.motion_fallback += 1,
        }
    };
    let mut scored: BTreeMap<Frontend, Vec<&RowRecord>> = BTreeMap::new();
    for row in rows.values() {
        let entry = out.entry(row.frontend).or_default();
        match (row.id().step(), row.outcome) {
            (RowStep::Transition(_), RowOutcome::Motion(outcome)) => {
                count(&mut entry.transitions, outcome)
            }
            (RowStep::Stage(_), RowOutcome::Motion(outcome)) => {
                count(&mut entry.stage_checks, outcome)
            }
            (RowStep::Stage(_), RowOutcome::Classifier(_)) => {
                scored.entry(row.frontend).or_default().push(row)
            }
            _ => {}
        }
        if let Some(end) = row.motion.as_ref().and_then(|m| m.end_state.as_ref()) {
            entry.end_state.rows += 1;
            match end.status {
                FrameMatch::Identical => entry.end_state.identical += 1,
                FrameMatch::Antialiased => entry.end_state.antialiased += 1,
                FrameMatch::Differs => entry.end_state.differs += 1,
            }
        }
    }
    for (frontend, rows) in scored {
        out.entry(frontend).or_default().stages = crate::results::summarise(&rows);
    }
    for (id, fidelity) in timelines {
        let entry = out.entry(id.frontend()).or_default();
        entry.timelines.slides += 1;
        if fidelity.mismatches.is_empty() {
            entry.timelines.exact += 1;
        }
        for counts in fidelity.effects.values() {
            entry.effects.native += counts.native;
            entry.effects.fallback += counts.fallback;
            entry.effects.unsupported += counts.unsupported;
        }
        if let Some(transition) = &fidelity.transition {
            let counts = EffectCounts {
                native: u32::from(transition.effect.played == EffectPlay::Native),
                fallback: u32::from(transition.effect.played == EffectPlay::Fallback),
                unsupported: u32::from(transition.effect.played == EffectPlay::Unsupported),
            };
            entry.effects.native += counts.native;
            entry.effects.fallback += counts.fallback;
            entry.effects.unsupported += counts.unsupported;
        }
    }
    for (pair, report) in pairs {
        let entry = out.entry(pair.frontend()).or_default();
        let items = &mut entry.items;
        *items = VerdictCounts {
            native: items.native + report.native,
            fallback_allowed: items.fallback_allowed + report.fallback_allowed,
            accepted: items.accepted + report.accepted,
            dropped: items.dropped + report.dropped,
            mismatched: items.mismatched + report.mismatched,
        };
    }
    out
}

/// What a pair's play produced.
struct PairPlayed {
    deck: Option<Arc<MotionDeck>>,
    /// The IR's slides, or why the IR did not compile.
    ir: std::result::Result<Vec<IrSlide>, String>,
    seen: BTreeMap<usize, SlideSeen>,
    failure: Option<(Reason, String)>,
}

/// Compiles a pair's deck ([`lane_deck_arguments`]) and its IR ([`ir_arguments`], the motion the deck path lowers:
/// each format's own placement and delivery, which the parity IR prints in its parity form); the IR must compile and
/// read, or the pair fails: its timelines are what the deck's are compared with.
async fn compile_pair(
    binary: &Path,
    lane_pair: &LanePair,
    pinned: Option<&Path>,
    dir: &Path,
    env: &BTreeMap<String, String>,
) -> Result<(
    std::result::Result<Arc<MotionDeck>, (Reason, String)>,
    std::result::Result<Vec<IrSlide>, String>,
)> {
    let pair = &lane_pair.pair;
    let name = format!("{}.{}", pair.presentation(), pair.frontend());
    let bundle = dir.join("bundles").join(&name);
    remove(&bundle)?;
    if let Some(parent) = bundle.parent() {
        std::fs::create_dir_all(parent).map_err(BenchError::io(parent))?;
    }
    let compiled = run_compiler_with(
        binary,
        &lane_deck_arguments(lane_pair, pinned, &bundle),
        COMPILE_TIMEOUT,
        true,
        env.clone(),
    )
    .await?;
    let deck = match compiled.status {
        CompileStatus::Ok | CompileStatus::Errors => read_deck(&bundle)
            .map(Arc::new)
            .map_err(|e| (Reason::CompileCrash, e.to_string())),
        CompileStatus::Timeout => Err((Reason::CompileTimeout, "the deck did not bundle".into())),
        CompileStatus::Refused => Err((Reason::CompileRefused, "the deck did not bundle".into())),
        _ => Err((Reason::CompileCrash, "the deck did not bundle".into())),
    };
    let ir_dir = dir.join("ir").join(&name);
    remove(&ir_dir)?;
    let arguments = ir_arguments(&lane_pair.file, pair.frontend(), &ir_dir);
    let outcome = run_compiler_with(binary, &arguments, COMPILE_TIMEOUT, true, env.clone()).await?;
    let ir = ir_of(outcome.status, &ir_dir.join("presentation-ir.json"));
    Ok((deck, ir))
}

/// The static lane's synced font set (`<cache>/fonts`), when it holds exactly the files `fonts.lock.json` pins.
pub(crate) struct PinnedSet {
    pub dir: PathBuf,
    /// The set id `fonts.lock.json` pins.
    pub font_set_id: String,
}

pub(crate) fn pinned_set(ctx: &Context) -> Option<PinnedSet> {
    let lock = load_font_lock(&ctx.env.fonts_lock_path).ok()?;
    let (font_set_id, _) = check_font_set(&lock, &ctx.cache, false).ok()?;
    Some(PinnedSet {
        dir: ctx.cache.fonts.clone(),
        font_set_id,
    })
}

/// A motion or shell run's `corpus` stamp: each pair's digest, joined with the pinned set's id for a deck that draws
/// it ([`lane_deck_arguments`]: a deck with an all-stages export, on a cache that holds the set). A run on a synced
/// cache and one on an unsynced cache never share a run key, and two runs stay comparable on every pair that drew
/// the same fonts, whatever else they selected; `fonts.fontSetId` is the lane's set.
pub(crate) fn corpus_stamp(
    pairs: &[LanePair],
    pinned: Option<&PinnedSet>,
) -> Result<BTreeMap<PairKey, Sha256>> {
    pairs
        .iter()
        .map(|p| {
            let digest = match (p.scored_by_keynote(), pinned) {
                (true, Some(set)) => slidock_bench_schema::sha256_json(&(
                    p.digest.as_str(),
                    "pinned",
                    set.font_set_id.as_str(),
                ))?,
                _ => p.digest.clone(),
            };
            Ok((p.pair.clone(), digest))
        })
        .collect()
}

/// A motion or shell run's fonts stamp: the lane's set and lock.
pub(crate) fn fonts_stamp(fonts: &LaneFonts) -> Result<FontsStamp> {
    Ok(FontsStamp {
        font_set_id: Sha256::new(fonts.font_set_id.clone())?,
        lock_sha256: Sha256::new(fonts.lock_sha256.clone())?,
        drift: None,
    })
}

/// The CLI arguments of a lane pair's deck: the deck path, fonts on, no network (the fonts lane's sources). A deck
/// with an all-stages or a movie export also takes the static lane's pinned set (`pinned`) as its last font source,
/// so its frames draw the faces the static lane renders the deck in (Keynote's Apple and corpus faces, which the deck
/// path otherwise draws in its bundled substitutes) and are scored against Keynote's like with like.
pub fn lane_deck_arguments(lane_pair: &LanePair, pinned: Option<&Path>, out: &Path) -> Vec<String> {
    let mut arguments = deck_arguments(&lane_pair.file, lane_pair.pair.frontend(), out);
    if let (true, Some(pinned)) = (lane_pair.scored_by_keynote(), pinned) {
        arguments.push("--fonts".into());
        arguments.push(pinned.to_string_lossy().into_owned());
    }
    arguments
}

/// The CLI arguments of a deck's IR with the motion its deck plays (`--emit ir --native-motion`). It compiles with
/// the CLI's built-in fonts only: the lane reads its timelines, which no face changes, and a folder of the cache
/// (the static lane's synced set) would make it depend on a `fonts sync` the lane does not need.
pub fn ir_arguments(file: &Path, frontend: Frontend, out: &Path) -> Vec<String> {
    vec![
        "compile".into(),
        file.to_string_lossy().into_owned(),
        "--frontend".into(),
        frontend.to_string(),
        "--emit".into(),
        "ir".into(),
        "--native-motion".into(),
        "--out".into(),
        out.to_string_lossy().into_owned(),
    ]
}

/// The IR a `--emit ir --native-motion` compile left, or why there is none: a compile that did not finish, or a file that does not
/// read.
fn ir_of(status: CompileStatus, path: &Path) -> std::result::Result<Vec<IrSlide>, String> {
    match status {
        CompileStatus::Ok | CompileStatus::Errors => {
            read_ir(path).map_err(|e| format!("the IR does not read: {e}"))
        }
        status => Err(format!("the IR did not compile ({status:?})")),
    }
}

/// The Keynote corpus pairs' stages oracles: each all-stages export on disk, its pages mapped to the deck's positions
/// ([`stages::stage_pages`]), rasterised and read in the renderer container (poppler), checked against the manifest's
/// pin ([`stages::identity`]), and each position's settled frame scored against its page. A pair with no export, or
/// whose deck did not draw the pinned set (`pinned`: not synced), is reported pending.
#[allow(clippy::too_many_arguments)]
async fn score_stages(
    ctx: &Context,
    manifest: &CorpusManifest,
    running: &Running,
    lane_pair: &LanePair,
    pinned: bool,
    deck: &MotionDeck,
    seen: &BTreeMap<usize, SlideSeen>,
    positions: &BTreeMap<RowId, Sha256>,
    store: &FrameStore,
) -> Result<stages::PairStages> {
    let mut report = stages::PairStages::default();
    let pair = &lane_pair.pair;
    let Some(file) = &lane_pair.stages else {
        report.status = format!(
            "pending: no all-stages export in {}/<sourceSha>/ (the attended `oracle --stages` step)",
            ctx.env.stages_dir.display()
        );
        return Ok(report);
    };
    if !pinned {
        report.status = "pending: the stages are scored in the static lane's pinned font set, which the cache does not hold; run `slidock-bench fonts sync`".to_owned();
        return Ok(report);
    }
    report.export = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sha = sha256_file(file)?;
    report.sha256 = sha.clone();
    let recorded = manifest
        .presentations
        .iter()
        .find(|p| &p.id == pair.presentation())
        .and_then(|p| p.stages.as_ref());
    // A container the lane started for the purpose is released after; the lane's own renderer stops with the lane.
    let (renderer, release) = match running {
        Running::Container(renderer) => (renderer.clone(), false),
        Running::Native(_) => match ctx.own_renderer().await {
            Ok(renderer) => (renderer, true),
            Err(error) => {
                report.status = format!(
                    "pending: the renderer container rasterises the export ({})",
                    without_list(&error.to_string())
                );
                return Ok(report);
            }
        },
    };
    let outcome = score_stages_with(
        &renderer,
        lane_pair,
        deck,
        seen,
        positions,
        store,
        (&sha, recorded),
        &mut report,
    )
    .await;
    let outcome = if release {
        crate::renderer::docker::released(outcome, renderer.release().await)
    } else {
        outcome
    };
    outcome?;
    if report.status == "scored" && (report.pinned != "pages" || !report.rows_recorded) {
        eprintln!(
            "stages    {pair}: the manifest records no {}; `oracle --stages --record` records them",
            match (report.pinned == "pages", report.rows_recorded) {
                (false, false) => "page rasters and no pagesPerRow",
                (false, true) => "page rasters",
                _ => "pagesPerRow",
            }
        );
    }
    Ok(report)
}

/// An error without the list it names in parentheses (`(missing: …)`, every file of a font set): the lane's line says
/// what to do, not each file.
fn without_list(text: &str) -> String {
    match (text.find(" (missing: "), text.rfind(')')) {
        (Some(open), Some(close)) if close > open => format!(
            "{}{}",
            text.get(..open).unwrap_or_default(),
            text.get(close + 1..).unwrap_or_default()
        ),
        _ => text.to_owned(),
    }
}

#[allow(clippy::too_many_arguments)]
async fn score_stages_with(
    renderer: &crate::renderer::docker::Renderer,
    lane_pair: &LanePair,
    deck: &MotionDeck,
    seen: &BTreeMap<usize, SlideSeen>,
    positions: &BTreeMap<RowId, Sha256>,
    store: &FrameStore,
    (sha, recorded): (&str, Option<&slidock_bench_schema::corpus::StagesOracle>),
    report: &mut stages::PairStages,
) -> Result<()> {
    use crate::oracle::pdf::{oracle_copy, page_sizes};
    use crate::oracle::resolve::{PresentationOracle, oracle_page, pages_key};
    let pair = &lane_pair.pair;
    let Some(file) = &lane_pair.stages else {
        return Ok(());
    };
    let (_, poppler) = renderer.facts().await?;
    let pdf = oracle_copy(&renderer.work, file, sha)?;
    let sizes = page_sizes(renderer, &pdf).await?;
    let oracle = PresentationOracle { pdf, pages: sizes };
    let (plan, expected) = stages::stage_pages(deck);
    report.pages = oracle.pages.len() as u32;
    report.expected = expected;
    report.pages_per_row = stages::pages_per_row(&plan);
    let recorded_rows = recorded.and_then(|r| r.pages_per_row.as_ref());
    match stages::check_rows(recorded_rows, &report.pages_per_row, report.pages) {
        stages::Rows::PageCount(why) => {
            report.status = format!("oracle-page-count: {why}");
            return Ok(());
        }
        stages::Rows::Differs(why) => {
            report.status = format!("stage-count: {why}");
            return Ok(());
        }
        stages::Rows::Recorded => report.rows_recorded = true,
        stages::Rows::Unrecorded => {}
    }
    let slide_size = slidock_bench_metrics::Size {
        w: deck.size.w,
        h: deck.size.h,
    };
    // Every page once, rasterised and read in the container; the rasters stay on disk until a slide asks for them.
    let mut pages: BTreeMap<u32, stages::PageFile> = BTreeMap::new();
    let mut hashes = Vec::new();
    let numbers: Vec<u32> = (1..=report.pages).collect();
    let fetched: Vec<(u32, Result<crate::oracle::resolve::OraclePage>)> =
        futures::stream::iter(numbers)
            .map(|n| {
                let oracle = &oracle;
                let poppler = &poppler;
                async move { (n, oracle_page(renderer, oracle, n, poppler, false).await) }
            })
            .buffered(4)
            .collect()
            .await;
    for (n, page) in fetched {
        let page = page?;
        hashes.push(Sha256::new(page.raster.sha256.clone())?);
        let size = slidock_bench_metrics::Size {
            w: page.size.w,
            h: page.size.h,
        };
        let words = stages::page_words(&page.text.to_metrics(), size, slide_size);
        pages.insert(
            n,
            stages::PageFile {
                raster: page.raster.path,
                words,
            },
        );
    }
    let key = pages_key(&oracle.pages, &poppler)?;
    report.pinned_pages = recorded
        .and_then(|r| r.page_hashes.as_ref())
        .and_then(|h| h.get(&key))
        .map_or(0, |pages| pages.len() as u32);
    report.pinned = match stages::identity(recorded, &key, &hashes, sha) {
        stages::Identity::Pages => "pages",
        stages::Identity::File => "file",
        stages::Identity::Unpinned => "unpinned",
        stages::Identity::Differs(why) => {
            report.status = format!("oracle-identity: {why}");
            return Ok(());
        }
    }
    .to_owned();
    report.status = "scored".to_owned();
    for slide_pages in &plan {
        let Some(slide_seen) = seen.get(&slide_pages.index) else {
            continue;
        };
        let Ok(slide_id) = SlideId::new(slide_pages.slide.clone()) else {
            continue;
        };
        let static_id = RowId::new(pair.presentation().clone(), slide_id, pair.frontend());
        let decode = |sha: &Sha256| stages::decode_file(&store.path(sha));
        let frames: Vec<Option<slidock_bench_metrics::Rgba>> = (0..slide_pages.positions)
            .map(|k| {
                positions
                    .get(&static_id.with_step(RowStep::Stage(k)))
                    .and_then(decode)
            })
            .collect();
        let words = (0..slide_pages.positions)
            .map(|k| {
                slide_seen
                    .stages
                    .get(k as usize)
                    .and_then(|stage| stage.words.as_deref())
                    .map(stages::frame_words)
            })
            .collect();
        let opening = slide_seen
            .stages
            .first()
            .and_then(|stage| stage.batches.first())
            .and_then(|batch| batch.frame(0))
            .and_then(decode);
        let played = stages::PlayedSlide {
            pages: slide_pages,
            frames,
            words,
            opening,
        };
        let take = |n: u32| {
            let page = pages.get(&n)?;
            Some(stages::StagePage {
                raster: stages::decode_file(&page.raster)?,
                words: page.words.clone(),
            })
        };
        for (k, mut score) in stages::score_slide(&played, slide_size, &take) {
            let row = static_id.with_step(RowStep::Stage(k));
            if let Some(score) = &mut score {
                score.frame = positions.get(&row).cloned();
            }
            report.add(row.to_string(), score);
        }
    }
    Ok(())
}

/// A Keynote corpus pair's movie oracle ([`movies`]): the record's spans, the movie's frames at each transition row's
/// time decoded and checked against the record's pins, and each row's frame scored against its movie frame. The score
/// goes on the row (`ssim`, `deltaE`), and its class when worse than the row's. A pair with no record, no decoder or
/// no pinned font set is pending.
#[allow(clippy::too_many_arguments)]
async fn score_movie(
    lane_pair: &LanePair,
    pair_played: &PairPlayed,
    recorded: Option<&movies::MovieOracle>,
    decoder: Option<&movies::Decoder>,
    pinned: bool,
    store: &FrameStore,
    records: &mut BTreeMap<RowId, RowRecord>,
) -> Result<movies::PairMovie> {
    let mut report = movies::PairMovie::default();
    let Some(file) = &lane_pair.movie else {
        return Ok(report);
    };
    report.movie = file
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let Some(recorded) = recorded else {
        report.status = "pending: no movie record; `oracle --movie --record` records it".to_owned();
        return Ok(report);
    };
    let Some(decoder) = decoder else {
        report.status = "pending: no movie decoder (ffmpeg, or SLIDOCK_FFMPEG)".to_owned();
        return Ok(report);
    };
    if !pinned {
        report.status = "pending: the movie is scored in the static lane's pinned font set, which the cache does not hold; run `slidock-bench fonts sync`".to_owned();
        return Ok(report);
    }
    let sha = sha256_file(file)?;
    report.sha256 = sha.clone();
    report.decoder = decoder.key();
    let pair = &lane_pair.pair;
    // Each transition row of the pair with its player frame and its movie frame.
    // Each transition row of the pair: its player frame, its movie frame, and the transition's ends in both.
    struct Row {
        id: RowId,
        render: Sha256,
        pm: u32,
        frame: u32,
        off: f64,
        ends: Option<((Sha256, u32), (Sha256, u32))>,
    }
    let rows: Vec<Row> = records
        .iter()
        .filter(|(id, _)| {
            id.presentation() == pair.presentation() && id.frontend() == pair.frontend()
        })
        .filter_map(|(id, row)| {
            let RowStep::Transition(pm) = id.step() else {
                return None;
            };
            let span = recorded.transitions.get(id.slide().as_str())?;
            let (frame, off) = span.frame(u32::from(pm));
            let ends = row
                .motion
                .as_ref()
                .and_then(|m| m.frames.as_ref())
                .map(|strip| {
                    (
                        (strip.start.clone(), span.start),
                        (strip.end.clone(), span.start + span.frames),
                    )
                });
            Some(Row {
                id: id.clone(),
                render: row.render.clone()?,
                pm: u32::from(pm),
                frame,
                off,
                ends,
            })
        })
        .collect();
    let mut indices: Vec<u32> = rows.iter().map(|row| row.frame).collect();
    indices.extend(
        rows.iter()
            .flat_map(|row| row.ends.iter().flat_map(|(a, b)| [a.1, b.1])),
    );
    let frames = decoder
        .frames(file, (recorded.width, recorded.height), &indices)
        .await?;
    // The pinned frames: each row's, and each transition's ends (`#t@0`, `#t@1000`).
    let mut wanted: BTreeMap<String, u32> = BTreeMap::new();
    for row in &rows {
        wanted.insert(row.id.to_string(), row.frame);
        if let Some(((_, first), (_, last))) = &row.ends {
            let slide = row.id.with_step(RowStep::Transition(0)).to_string();
            wanted.insert(slide, *first);
            wanted.insert(
                row.id.with_step(RowStep::Transition(1000)).to_string(),
                *last,
            );
        }
    }
    let pins = recorded.frame_hashes.get(&report.decoder);
    let mut differing = Vec::new();
    for (row, frame) in &wanted {
        let hash = frames.get(frame).map(movies::frame_hash);
        if let (Some(pin), Some(hash)) = (pins.and_then(|p| p.get(row)), &hash)
            && pin != hash
        {
            differing.push(row.clone());
        }
    }
    report.pinned = if sha == recorded.sha256.as_str() {
        "file"
    } else if pins.is_some() && differing.is_empty() {
        "frames"
    } else {
        "unpinned"
    }
    .to_owned();
    if !differing.is_empty() || report.pinned == "unpinned" && sha != recorded.sha256.as_str() {
        differing.truncate(8);
        report.status = format!(
            "oracle-identity: the movie's sha256 {sha} is not the recorded {}{}",
            recorded.sha256,
            if differing.is_empty() {
                ", and no frames are pinned under this decoder".to_owned()
            } else {
                format!(
                    ", and frames of {} differ from the pinned",
                    differing.join(", ")
                )
            }
        );
        return Ok(report);
    }
    if pair_played.deck.is_none() {
        report.status = "pending: the deck did not bundle".to_owned();
        return Ok(report);
    }
    report.status = "scored".to_owned();
    let scored = |movie_frame: u32, render: &Sha256| {
        let movie = frames.get(&movie_frame)?;
        let played = stages::decode_file(&store.path(render))?;
        movies::score(movie, &played)
    };
    for row in rows {
        let Some((ssim_mean, ssim_p1, delta_e)) = scored(row.frame, &row.render) else {
            continue;
        };
        let ends = row
            .ends
            .as_ref()
            .and_then(|((a, first), (b, last))| Some((scored(*first, a)?, scored(*last, b)?)));
        let score = movies::MovieScore::new(
            (row.frame, row.off),
            (ssim_mean, ssim_p1, delta_e),
            ends.map(|(first, last)| movies::Ceiling::between(first, last, row.pm)),
            row.render,
        );
        if let Some(record) = records.get_mut(&row.id) {
            let metrics = record.motion.get_or_insert_with(Default::default);
            metrics.ssim = Some(ssim_mean);
            metrics.delta_e = Some(delta_e);
            if let (RowOutcome::Motion(outcome), Some(why)) = (record.outcome, score.failure()) {
                metrics.failures.push(why);
                record.outcome = movies::worse(outcome, MotionOutcome::MotionMismatch).into();
            }
        }
        report.rows.insert(row.id.to_string(), score);
    }
    Ok(report)
}

/// Decoded frames by sha256, the most recent few kept: the motion checks compare each pause with several others.
struct DecodeCache<'a> {
    store: &'a FrameStore,
    kept: std::sync::Mutex<Vec<(Sha256, Arc<slidock_bench_metrics::Rgba>)>>,
}

impl<'a> DecodeCache<'a> {
    const KEEP: usize = 24;

    fn new(store: &'a FrameStore) -> Self {
        DecodeCache {
            store,
            kept: std::sync::Mutex::new(Vec::new()),
        }
    }

    fn get(&self, sha: &Sha256) -> Option<Arc<slidock_bench_metrics::Rgba>> {
        let mut kept = self.kept.lock().ok()?;
        if let Some(at) = kept.iter().position(|(k, _)| k == sha) {
            let entry = kept.remove(at);
            let image = entry.1.clone();
            kept.push(entry);
            return Some(image);
        }
        let bytes = std::fs::read(self.store.path(sha)).ok()?;
        let image = Arc::new(slidock_bench_metrics::decode_png(&bytes).ok()?);
        if kept.len() >= Self::KEEP {
            kept.remove(0);
        }
        kept.push((sha.clone(), image.clone()));
        Some(image)
    }
}

/// Runs the motion lane and writes its run.
pub async fn motion_lane(ctx: &Context, request: &MotionRequest) -> Result<Exit> {
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    refuse_holdout(
        &manifest,
        &request.tiers,
        request.presentations.as_deref().unwrap_or_default(),
        &[],
        "run --lane motion",
    )?;
    let pairs = lane_pairs(
        ctx,
        &manifest,
        &request.tiers,
        request.presentations.as_ref(),
        request.frontends.as_ref(),
    )?;
    if pairs.is_empty() {
        return Err(BenchError::refused("the selection names no deck"));
    }
    let repo = &ctx.env.repo_root;
    let allow = load_allowlist(&repo.join(FALLBACK_ALLOW))?;
    let fonts = prepare(&repo.join(LANE_LOCK), &ctx.cache).await?;
    let binary = compiler_binary(repo, &ctx.env.target_dir, ctx.env.compiler.as_deref()).await?;
    let running = start_renderer(ctx, &request.renderer, &fonts.root).await?;
    let outcome = run_lane(
        ctx, request, &manifest, &pairs, &allow, &fonts, &binary, &running,
    )
    .await;
    running.stop().await?;
    outcome
}

/// Starts the renderer a lane asks for: a container of the lane's own (or the shared one, reused), or the native
/// headless shell.
pub(crate) async fn start_renderer(
    ctx: &Context,
    choice: &RendererChoice,
    profile_root: &Path,
) -> Result<Running> {
    Ok(match choice {
        RendererChoice::Container => Running::Container(ctx.own_renderer().await?),
        RendererChoice::Native(binary) => {
            let binary = binary.clone().or_else(default_binary).ok_or_else(|| {
                BenchError::refused("no headless shell: set SLIDOCK_BENCH_CHROMIUM")
            })?;
            let profile = profile_root.join("motion-profile");
            Running::Native(NativeBrowser::start(&ctx.env.bench_dir, &binary, &profile).await?)
        }
    })
}

#[allow(clippy::too_many_arguments)]
async fn run_lane(
    ctx: &Context,
    request: &MotionRequest,
    manifest: &CorpusManifest,
    pairs: &[LanePair],
    allow: &Allowlist,
    fonts: &crate::fonts_lane::lock::LaneFonts,
    binary: &Path,
    running: &Running,
) -> Result<Exit> {
    let browser = Arc::new(Browser::connect(running.ws_endpoint()).await?);
    let repo = &ctx.env.repo_root;
    let mut tiers = request.tiers.clone();
    tiers.sort();
    tiers.dedup();
    let compiler = compiler_stamp(repo).await?;
    let content = compiler.content.clone();
    let pinned = pinned_set(ctx);
    let stamp = Stamp {
        compiler,
        deps: deps_stamp(repo).await?,
        host: host_stamp(),
        renderer: running.stamp(&browser).await?,
        fonts: fonts_stamp(fonts)?,
        metrics: MetricsStamp {
            ssim: SSIM_VERSION.to_owned(),
            color: COLOR_VERSION.to_owned(),
            pixel: PIXEL_VERSION.to_owned(),
            text: TEXT_VERSION.to_owned(),
            shadow: None,
            gradient: None,
            motion: Some(MOTION_VERSION.to_owned()),
            fonts: None,
        },
        classifier: CLASSIFIER_VERSION.to_owned(),
        harness: harness_hash(repo, None).await?,
        corpus: corpus_stamp(pairs, pinned.as_ref())?,
        selection: StampSelection { tiers },
        lane: Lane::Motion,
        partial: request.presentations.is_some() || request.frontends.is_some(),
        determinism: None,
        parity: None,
        perf: None,
    };
    let dir = ctx.cache.runs.join(stamp.run_key()?.as_str());
    std::fs::create_dir_all(&dir).map_err(BenchError::io(&dir))?;
    let store = Arc::new(FrameStore {
        dir: frames_dir(&dir),
    });
    let env = lane_env(&fonts.cli_cache);
    // Compile every pair, then walk every run of slides of every deck, `concurrency` pages at a time.
    let mut played: Vec<PairPlayed> = Vec::new();
    let mut jobs: Vec<(usize, WalkRequest)> = Vec::new();
    for (n, lane_pair) in pairs.iter().enumerate() {
        let (deck, ir) = compile_pair(
            binary,
            lane_pair,
            pinned.as_ref().map(|set| set.dir.as_path()),
            &dir,
            &env,
        )
        .await?;
        let name = format!(
            "{}.{}",
            lane_pair.pair.presentation(),
            lane_pair.pair.frontend()
        );
        match deck {
            Ok(deck) => {
                let router = Arc::new(DeckRouter {
                    dir: dir.join("bundles").join(&name),
                    gif_frames: ctx.cache.gif_frames.clone(),
                });
                for slides in runs(&deck.shown(), RUN_SLIDES) {
                    jobs.push((
                        n,
                        WalkRequest {
                            url: deck_url(),
                            router: Some(router.clone()),
                            deck: deck.clone(),
                            slides,
                            store: store.clone(),
                            words: lane_pair.stages.is_some(),
                        },
                    ));
                }
                eprintln!(
                    "{}: {} slides, {} shown",
                    lane_pair.pair,
                    deck.slides.len(),
                    deck.shown().len()
                );
                played.push(PairPlayed {
                    deck: Some(deck),
                    ir,
                    seen: BTreeMap::new(),
                    failure: None,
                });
            }
            Err(failure) => {
                eprintln!("{}: {}", lane_pair.pair, failure.1);
                played.push(PairPlayed {
                    deck: None,
                    ir,
                    seen: BTreeMap::new(),
                    failure: Some(failure),
                });
            }
        }
    }
    let walked: Vec<(usize, Vec<SlideSeen>)> = futures::stream::iter(jobs.iter())
        .map(|(n, job)| {
            let browser = browser.clone();
            async move { (*n, walk(&browser, job).await) }
        })
        .buffer_unordered(request.concurrency.max(1))
        .collect()
        .await;
    for (n, slides) in walked {
        if let Some(pair) = played.get_mut(n) {
            for slide in slides {
                pair.seen.insert(slide.index, slide);
            }
        }
    }
    let scores = |a: &Sha256, b: &Sha256| -> Option<(f64, f64)> {
        let a = std::fs::read(store.path(a)).ok()?;
        let b = std::fs::read(store.path(b)).ok()?;
        render_vs_render(&a, &b).ok()
    };
    let antialiased = |a: &Sha256, b: &Sha256| -> bool {
        let (Ok(a), Ok(b)) = (std::fs::read(store.path(a)), std::fs::read(store.path(b))) else {
            return false;
        };
        specks(&a, &b, ANTIALIAS_CHANNEL)
            .ok()
            .flatten()
            .is_some_and(|n| n <= ANTIALIAS_PIXELS)
    };
    let matches = |a: &Sha256, b: &Sha256| -> bool {
        let (Ok(a), Ok(b)) = (std::fs::read(store.path(a)), std::fs::read(store.path(b))) else {
            return false;
        };
        metrics_match(&a, &b).unwrap_or(false)
    };
    let decoded = DecodeCache::new(&store);
    let changed =
        |a: &Sha256, b: &Sha256, region: Option<[f64; 4]>, tolerance: u8| -> Option<Change> {
            let (a, b) = (decoded.get(a)?, decoded.get(b)?);
            change(&a, &b, region, tolerance)
        };
    let displaced = |a: &Sha256, b: &Sha256| -> bool {
        let (Some(a), Some(b)) = (decoded.get(a), decoded.get(b)) else {
            return false;
        };
        displaced_specks(&a, &b, ANTIALIAS_CHANNEL).is_some_and(|n| n <= ANTIALIAS_PIXELS)
    };
    let frames = Frames {
        scores: &scores,
        antialiased: &antialiased,
        matches: &matches,
        change: &changed,
        displaced: &displaced,
    };
    let mut records: BTreeMap<RowId, RowRecord> = BTreeMap::new();
    let mut timelines: BTreeMap<RowId, TimelineFidelity> = BTreeMap::new();
    let mut positions: BTreeMap<RowId, Sha256> = BTreeMap::new();
    let mut motion_pairs: BTreeMap<PairKey, MotionPairReport> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    let mut stale: Vec<String> = Vec::new();
    // Each failed check no review accepts: what an accepted check must name to accept it.
    let mut problems: Vec<String> = Vec::new();
    for (lane_pair, pair_played) in pairs.iter().zip(&played) {
        let pair = &lane_pair.pair;
        let Some(deck) = &pair_played.deck else {
            let (_, why) = pair_played
                .failure
                .clone()
                .unwrap_or((Reason::CompileCrash, String::new()));
            failures.push(format!("{pair}: {why}"));
            continue;
        };
        let shown = deck.shown();
        let first = shown.first().copied();
        let ir_slides: &[IrSlide] = match &pair_played.ir {
            Ok(slides) => slides,
            Err(why) => {
                failures.push(format!("{pair}: {why}"));
                &[]
            }
        };
        let mut items: Vec<(usize, SlideId, Vec<Item>)> = Vec::new();
        let mut verdicts = Verdicts {
            pair,
            deck,
            allow,
            used: BTreeSet::new(),
            used_checks: BTreeSet::new(),
        };
        for (page, index) in (1u32..).zip(&shown) {
            let Some(slide) = deck.slides.get(*index) else {
                continue;
            };
            let Ok(slide_id) = SlideId::new(slide.id.clone()) else {
                continue;
            };
            let static_id = RowId::new(
                pair.presentation().clone(),
                slide_id.clone(),
                pair.frontend(),
            );
            let Some(seen) = pair_played.seen.get(index) else {
                failures.push(format!("{static_id}: not walked"));
                continue;
            };
            if let Some(why) = &seen.failed {
                failures.push(format!("{static_id}: {why}"));
                if slide.has_motion() {
                    let mut row = record(&static_id.with_step(RowStep::Stage(0)), page, &content);
                    row.stage = Some(0);
                    row.outcome = Outcome::RenderError.into();
                    row.reason = Some(Reason::RenderCrash);
                    records.insert(row.id(), row);
                }
                continue;
            }
            let SlideCheck {
                mut fidelity,
                rows,
                positions: settled,
                items: mut slide_items,
            } = check_slide(&SlideInput {
                slide,
                seen,
                first: Some(*index) == first,
                previous: shown
                    .iter()
                    .rev()
                    .find(|i| *i < index)
                    .and_then(|i| deck.slides.get(*i)),
                next: shown
                    .iter()
                    .find(|i| *i > index)
                    .and_then(|i| deck.slides.get(*i)),
                frames: &frames,
            });
            for (k, sha) in settled {
                positions.insert(static_id.with_step(RowStep::Stage(k)), sha);
            }
            if !slide.has_motion() {
                continue;
            }
            let (ir, ir_mismatches) =
                compare_ir(slide, ir_slides.iter().find(|s| s.id == slide.id));
            fidelity.ir = ir;
            for mismatch in &ir_mismatches {
                for item in slide_items.iter_mut() {
                    let hit = match (mismatch.step, item.item) {
                        (Some(step), ItemRef::Step(i)) => step == i,
                        (None, _) => true,
                        _ => false,
                    };
                    if hit {
                        item.problems.push(Problem::new(
                            "ir",
                            format!(
                                "the deck says {}, the IR says {}",
                                mismatch.actual, mismatch.expected
                            ),
                        ));
                    }
                }
            }
            fidelity.mismatches.extend(ir_mismatches);
            let name = lane_pair
                .expected
                .as_ref()
                .and_then(|e| e.iter().find(|x| x.index == *index))
                .map_or_else(|| slide.id.clone(), |x| x.key.clone());
            if let Some(expected) = &lane_pair.expected {
                params_problems(expected, *index, &mut slide_items);
            }
            verdicts.accept(&name, &mut slide_items);
            for item in &slide_items {
                for problem in &item.problems {
                    problems.push(format!(
                        "problem    {pair} {name} {} [{}] {}: {}",
                        item.item.label(),
                        item.native,
                        problem.check,
                        problem.detail
                    ));
                }
            }
            let groups = model::groups(&slide.timeline);
            for (step, outcome, mut metrics, render, failing) in rows {
                // A failing row is accepted when a review accepts each check it failed for an item it plays.
                let plays = |item: &Item| match (item.item, step) {
                    (ItemRef::Step(i), RowStep::Stage(k)) => groups.get(i as usize) == Some(&k),
                    (ItemRef::Transition, RowStep::Transition(_)) => true,
                    _ => false,
                };
                let accepted: Vec<&Problem> = slide_items
                    .iter()
                    .filter(|item| plays(item))
                    .flat_map(|item| item.accepted.iter())
                    .collect();
                if outcome == MotionOutcome::MotionMismatch
                    && !failing.is_empty()
                    && failing
                        .iter()
                        .all(|check| accepted.iter().any(|a| &a.check == check))
                {
                    let mut reasons: Vec<&str> = accepted.iter().map(|a| a.text.as_str()).collect();
                    reasons.sort_unstable();
                    reasons.dedup();
                    metrics.accepted = Some(reasons.join("; "));
                }
                let id = static_id.with_step(step);
                let mut row = record(&id, page, &content);
                match step {
                    RowStep::Stage(k) => row.stage = Some(k),
                    RowStep::Transition(pm) => row.transition_pm = Some(pm),
                    RowStep::Static => {}
                }
                row.outcome = outcome.into();
                row.render = Some(render);
                row.motion = Some(metrics);
                records.insert(id, row);
            }
            timelines.insert(static_id.clone(), fidelity);
            items.push((*index, slide_id, slide_items));
        }
        let slides: Vec<SlideItems<'_>> = items
            .iter()
            .map(|(index, id, items)| SlideItems {
                index: *index,
                id,
                items,
            })
            .collect();
        let mut report = match &lane_pair.expected {
            Some(expected) => verdicts.probe(&slides, expected),
            None => verdicts.corpus(&slides),
        };
        report.slides = slides.len() as u32;
        for (slide, item, what) in verdicts.stale() {
            stale.push(format!(
                "stale      {pair} {slide} {item}: allowlisted ({what}), and no item needs it now; remove its entry"
            ));
        }
        motion_pairs.insert(pair.clone(), report);
    }
    // The stages oracles of the Keynote corpus decks.
    let mut stage_reports: BTreeMap<String, stages::PairStages> = BTreeMap::new();
    let mut stage_lines = Vec::new();
    for (lane_pair, pair_played) in pairs.iter().zip(&played) {
        if lane_pair.pair.frontend() != Frontend::Keynote || !lane_pair.corpus {
            continue;
        }
        let Some(deck) = &pair_played.deck else {
            continue;
        };
        let report = score_stages(
            ctx,
            manifest,
            running,
            lane_pair,
            pinned.is_some(),
            deck,
            &pair_played.seen,
            &positions,
            &store,
        )
        .await?;
        stage_lines.push(report.line(&lane_pair.pair.to_string()));
        for row in &report.mismatched {
            if let Some(score) = report.rows.get(row) {
                stage_lines.push(format!(
                    "stage      {row} (page {}): {}",
                    score.page,
                    score.failed.join("; ")
                ));
            }
        }
        stage_reports.insert(lane_pair.pair.to_string(), report);
    }
    crate::cache::write_json(&dir.join(stages::STAGES_FILE), &stage_reports)?;
    // The movie oracles of the Keynote corpus decks: each transition row against Keynote's frame at its time.
    let mut movie_reports: BTreeMap<String, movies::PairMovie> = BTreeMap::new();
    let movie_pairs: Vec<(&LanePair, &PairPlayed)> = pairs
        .iter()
        .zip(&played)
        .filter(|(lane_pair, _)| lane_pair.movie.is_some() && lane_pair.corpus)
        .collect();
    if !movie_pairs.is_empty() {
        let records_file = movies::record_path(&ctx.env.manifest_path);
        let movie_records = movies::load_records(&records_file)?;
        let decoder = movies::Decoder::find().await;
        for (lane_pair, pair_played) in movie_pairs {
            let report = score_movie(
                lane_pair,
                pair_played,
                movie_records
                    .movies
                    .get(lane_pair.pair.presentation().as_str()),
                decoder.as_ref().ok(),
                pinned.is_some(),
                &store,
                &mut records,
            )
            .await?;
            stage_lines.push(report.line(&lane_pair.pair.to_string()));
            movie_reports.insert(lane_pair.pair.to_string(), report);
        }
    }
    crate::cache::write_json(&dir.join(movies::MOVIES_FILE), &movie_reports)?;
    let lanes = motion_summary(&records, &timelines, &motion_pairs);
    let classified: BTreeMap<RowId, RowRecord> = records
        .iter()
        .filter(|(_, row)| row.outcome.classifier().is_some())
        .map(|(id, row)| (id.clone(), row.clone()))
        .collect();
    let mut results_summary = summary(&classified);
    results_summary.lanes = Some(LaneSummaries {
        motion: Some(lanes.clone()),
        ..LaneSummaries::default()
    });
    let results = Results {
        schema: ResultsSchema::Value,
        stamp,
        summary: results_summary,
        rows: records,
        holdout: None,
        timelines: Some(timelines),
        font_pairs: None,
        shell_pairs: None,
        motion_pairs: Some(motion_pairs),
        positions: Some(positions),
    };
    let path = results_path(&dir);
    write_results(&path, &results)?;
    print_report(&results);
    for line in &stage_lines {
        println!("{line}");
    }
    for line in problems.iter().chain(&stale) {
        println!("{line}");
    }
    for failure in &failures {
        println!("failed     {failure}");
    }
    println!("results: {}", path.display());
    let mismatched_rows = results
        .rows
        .values()
        .filter(|row| {
            let accepted = row.motion.as_ref().is_some_and(|m| m.accepted.is_some());
            matches!(
                row.outcome,
                RowOutcome::Motion(MotionOutcome::MotionMismatch)
                    | RowOutcome::Classifier(Outcome::RenderError | Outcome::Failed)
            ) && !accepted
        })
        .count();
    let mismatched_items: u32 = results
        .motion_pairs
        .iter()
        .flat_map(|p| p.values())
        .map(|r| r.mismatched)
        .sum();
    let stages_failed = stage_reports.values().any(stages::PairStages::fails)
        || movie_reports.values().any(movies::PairMovie::fails);
    Ok(Exit::check(
        failures.is_empty()
            && stale.is_empty()
            && mismatched_rows == 0
            && mismatched_items == 0
            && !stages_failed,
    ))
}

/// One line per deck, then per frontend.
pub fn print_report(results: &Results) {
    for (pair, report) in results.motion_pairs.iter().flatten() {
        println!(
            "{pair}: {} slides, {} transitions, {} steps{}: native {}, fallback-allowed {}, accepted {}, dropped {}, mismatched {}",
            report.slides,
            report.transitions,
            report.steps,
            if report.probe {
                " (probe expectations)"
            } else {
                ""
            },
            report.native,
            report.fallback_allowed,
            report.accepted,
            report.dropped,
            report.mismatched
        );
        for item in report
            .items
            .iter()
            .filter(|i| i.verdict == MotionVerdict::Mismatched)
        {
            println!(
                "mismatch   {pair} {} {} {}: {}",
                item.slide,
                item.key.as_deref().unwrap_or(""),
                item.item,
                item.why.as_deref().unwrap_or("")
            );
        }
    }
    for (id, fidelity) in results.timelines.iter().flatten() {
        for mismatch in fidelity
            .mismatches
            .iter()
            .filter(|m| !matches!(m.field, TimelineField::Ir))
        {
            println!(
                "timeline   {id} {}{:?}: expected {}, played {}",
                mismatch
                    .step
                    .map(|s| format!("step {s} "))
                    .unwrap_or_default(),
                mismatch.field,
                mismatch.expected,
                mismatch.actual
            );
        }
    }
    for (id, row) in &results.rows {
        if row.outcome == RowOutcome::Motion(MotionOutcome::MotionMismatch) {
            let failures = row
                .motion
                .as_ref()
                .map(|m| m.failures.join(" | "))
                .unwrap_or_default();
            match row.motion.as_ref().and_then(|m| m.accepted.as_deref()) {
                Some(reason) => println!("accepted   {id}: {reason}"),
                None => println!("row        {id}: motion-mismatch: {failures}"),
            }
        }
    }
    for (frontend, s) in results
        .summary
        .lanes
        .iter()
        .flat_map(|l| l.motion.iter().flatten())
    {
        println!(
            "{frontend}: stages {} (match {}, close {}, fallback {}, mismatch {}); transition frames {} (match {}, close {}, fallback {}, mismatch {}); end states {}/{} identical, {} antialiased; timelines {}/{} exact",
            s.stage_checks.rows,
            s.stage_checks.motion_match,
            s.stage_checks.motion_close,
            s.stage_checks.motion_fallback,
            s.stage_checks.motion_mismatch,
            s.transitions.rows,
            s.transitions.motion_match,
            s.transitions.motion_close,
            s.transitions.motion_fallback,
            s.transitions.motion_mismatch,
            s.end_state.identical,
            s.end_state.rows,
            s.end_state.antialiased,
            s.timelines.exact,
            s.timelines.slides
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion_lane::deck::{
        DeckSlide, DeckStep, DeckTransition, IrEffect, IrStep, IrTransition, Target,
    };
    use serde_json::json;

    fn target(element: &str) -> Target {
        Target {
            element: element.into(),
            paragraphs: None,
            background: None,
        }
    }

    fn deck_step(element: &str, trigger: &str, native: &str) -> DeckStep {
        DeckStep {
            target: target(element),
            trigger: trigger.into(),
            kind: "entrance".into(),
            delivery: Some("all".into()),
            delay_ms: json!(0),
            duration_ms: json!(500),
            native: native.into(),
            fallback: "dissolve".into(),
            effect: Some("fade".into()),
            params: None,
            offset_ms: None,
            repeat: None,
            auto_reverse: None,
            stagger: None,
            after: None,
        }
    }

    fn ir_step(element: &str, trigger: &str) -> IrStep {
        IrStep {
            target: target(element),
            trigger: trigger.into(),
            kind: "entrance".into(),
            delivery: Some("all".into()),
            delay_ms: json!(0),
            duration_ms: json!(500),
            effect: IrEffect {
                id: "keynote:apple:dissolve".into(),
                fallback: "dissolve".into(),
            },
        }
    }

    #[test]
    fn an_ir_that_did_not_compile_or_read_is_a_failure_not_an_empty_one() {
        let dir = std::env::temp_dir().join(format!("slidock-bench-ir-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("presentation-ir.json");
        assert!(
            ir_of(CompileStatus::Crash, &path)
                .unwrap_err()
                .contains("did not compile")
        );
        assert!(ir_of(CompileStatus::Timeout, &path).is_err());
        assert!(
            ir_of(CompileStatus::Ok, &path)
                .unwrap_err()
                .contains("does not read")
        );
        std::fs::write(&path, "{\"slides\": [{\"id\": \"kn-1\"}]}").unwrap();
        assert_eq!(ir_of(CompileStatus::Errors, &path).unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_pending_stage_oracle_says_what_to_run_without_each_missing_file() {
        assert_eq!(
            without_list(
                "refused: font-set-mismatch: the cache's fonts differ from fonts.lock.json (missing: a.ttf, b (1).ttf); run `slidock-bench fonts sync`"
            ),
            "refused: font-set-mismatch: the cache's fonts differ from fonts.lock.json; run `slidock-bench fonts sync`"
        );
        assert_eq!(without_list("docker: not found"), "docker: not found");
    }

    #[test]
    fn the_ir_compiles_with_the_native_motion_and_no_folder_of_the_cache() {
        let arguments = ir_arguments(
            Path::new("/corpus/deck.key"),
            Frontend::Keynote,
            Path::new("/run/ir/deck.keynote"),
        );
        assert_eq!(
            arguments,
            [
                "compile",
                "/corpus/deck.key",
                "--frontend",
                "keynote",
                "--emit",
                "ir",
                "--native-motion",
                "--out",
                "/run/ir/deck.keynote"
            ]
        );
        // A fresh cache has no synced font set: the IR names no font folder, so it compiles all the same.
        assert!(!arguments.iter().any(|a| a == "--fonts"));
    }

    #[test]
    fn a_private_deck_plays_when_it_is_on_disk_and_as_pinned() {
        let dir = tempfile::tempdir().unwrap();
        let decks = [PrivateDeck {
            id: "build-subset",
            frontend: Frontend::Keynote,
            path: "probes-incoming/keynote/deck.key",
            // sha256 of "deck".
            sha256: "d830325906c3d540ae219e6aba0f243d52cd708feea68355a3f63f76aff8da33",
        }];
        // Not on disk, or no private folder at all: skipped.
        assert!(
            private_pairs(&decks, Some(dir.path()), None, None)
                .unwrap()
                .is_empty()
        );
        assert!(private_pairs(&decks, None, None, None).unwrap().is_empty());
        let file = dir.path().join("probes-incoming/keynote/deck.key");
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "deck").unwrap();
        let pairs = private_pairs(&decks, Some(dir.path()), None, None).unwrap();
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0].pair.to_string(), "build-subset:keynote");
        assert_eq!(pairs[0].file, file);
        assert!(pairs[0].expected.is_none() && pairs[0].stages.is_none() && !pairs[0].corpus);
        // A selection that names other decks or formats leaves it out.
        let other = vec![PresentationId::new("keynote-talk").unwrap()];
        assert!(
            private_pairs(&decks, Some(dir.path()), Some(&other), None)
                .unwrap()
                .is_empty()
        );
        assert!(
            private_pairs(&decks, Some(dir.path()), None, Some(&vec![Frontend::Pptx]))
                .unwrap()
                .is_empty()
        );
        // Another file at its path is refused.
        std::fs::write(&file, "other").unwrap();
        assert!(private_pairs(&decks, Some(dir.path()), None, None).is_err());
    }

    #[test]
    fn the_private_root_is_slidock_private_or_the_home_folder() {
        let root = |vars: &[(&str, &str)]| {
            private_root(|name| {
                vars.iter()
                    .find(|(n, _)| *n == name)
                    .map(|(_, v)| (*v).to_owned())
            })
        };
        assert_eq!(
            root(&[("SLIDOCK_PRIVATE", "/p"), ("HOME", "/h")]),
            Some(PathBuf::from("/p"))
        );
        assert_eq!(
            root(&[("SLIDOCK_PRIVATE", ""), ("HOME", "/h")]),
            Some(PathBuf::from("/h/.slidock"))
        );
        assert_eq!(root(&[]), None);
        assert_eq!(
            PRIVATE_DECKS[0].path,
            "probes-incoming/keynote/build-subset.key"
        );
    }

    #[test]
    fn a_deck_with_a_stages_export_draws_the_pinned_set() {
        let pair = |stages: Option<&str>| LanePair {
            pair: PairKey::new(
                PresentationId::new("keynote-talk").unwrap(),
                Frontend::Keynote,
            ),
            file: PathBuf::from("/corpus/keynote-talk.key"),
            digest: Sha256::of("deck"),
            expected: None,
            stages: stages.map(PathBuf::from),
            movie: None,
            corpus: true,
        };
        let out = Path::new("/run/bundles/keynote-talk.keynote");
        let set = Path::new("/cache/fonts");
        let fonts = |arguments: Vec<String>| -> Vec<String> {
            arguments
                .windows(2)
                .filter(|w| w[0] == "--fonts")
                .map(|w| w[1].clone())
                .collect()
        };
        let with_export = pair(Some("/stages/abc/all-stages.pdf"));
        let arguments = lane_deck_arguments(&with_export, Some(set), out);
        assert_eq!(
            arguments[..6],
            deck_arguments(&with_export.file, Frontend::Keynote, out)[..]
        );
        assert_eq!(fonts(arguments), ["/cache/fonts"]);
        // Without an export the deck draws the deck path's own sources; with no synced set it draws them too, and its
        // stages are pending.
        assert!(fonts(lane_deck_arguments(&pair(None), Some(set), out)).is_empty());
        assert!(fonts(lane_deck_arguments(&with_export, None, out)).is_empty());
    }

    #[test]
    fn the_run_key_says_whether_the_decks_draw_the_pinned_set() {
        use crate::fonts::{FontEntry, FontLock, FontSource, font_cache_name, font_set_id};
        let dir = tempfile::tempdir().unwrap();
        let private = dir.path().join("private").to_string_lossy().into_owned();
        let mut env = crate::env::BenchEnv::from_vars(dir.path().to_path_buf(), |name| {
            (name == "SLIDOCK_PRIVATE").then(|| private.clone())
        })
        .unwrap();
        let entry = |file: &str| FontEntry {
            family: "F".into(),
            file: file.into(),
            sha256: crate::hash::sha256(file),
            licence: "test".into(),
            source: FontSource::Host { paths: Vec::new() },
        };
        let lock = FontLock {
            version: 1,
            fonts: vec![entry("A.ttf"), entry("B.otf")],
        };
        env.fonts_lock_path = dir.path().join("fonts.lock.json");
        std::fs::write(&env.fonts_lock_path, serde_json::to_string(&lock).unwrap()).unwrap();
        let ctx = Context::new(env);
        let lane = LaneFonts {
            root: dir.path().join("fonts-lane"),
            cli_cache: dir.path().join("cli-cache"),
            fallback: dir.path().join("fallback"),
            font_set_id: font_set_id([crate::hash::sha256("lane").as_str()]).unwrap(),
            lock_sha256: crate::hash::sha256("lane lock"),
            provided: Vec::new(),
        };
        let pair = |id: &str, frontend: Frontend, stages: Option<&str>| LanePair {
            pair: PairKey::new(PresentationId::new(id).unwrap(), frontend),
            file: PathBuf::from(format!("/corpus/{id}")),
            digest: Sha256::of(id),
            expected: None,
            stages: stages.map(PathBuf::from),
            movie: None,
            corpus: true,
        };
        let keynote = pair(
            "keynote-talk",
            Frontend::Keynote,
            Some("/stages/abc/all-stages.pdf"),
        );
        let pptx = pair("google-export", Frontend::Pptx, None);
        let full = [pptx, keynote];
        let stamp = |pairs: &[LanePair]| {
            let mut stamp = crate::test_support::results(Vec::new()).stamp;
            stamp.lane = Lane::Motion;
            stamp.fonts = fonts_stamp(&lane).unwrap();
            stamp.corpus = corpus_stamp(pairs, pinned_set(&ctx).as_ref()).unwrap();
            stamp
        };

        // Not synced: every deck draws the lane's set alone, and stamps its own digest.
        assert!(pinned_set(&ctx).is_none());
        let unsynced = stamp(&full);
        assert_eq!(unsynced.fonts.font_set_id.as_str(), lane.font_set_id);
        assert!(full.iter().all(|p| unsynced.corpus[&p.pair] == p.digest));
        // Synced (every file the lock pins, under its hash): the deck with an export draws it, and the key moves.
        std::fs::create_dir_all(&ctx.cache.fonts).unwrap();
        for entry in &lock.fonts {
            std::fs::write(ctx.cache.fonts.join(font_cache_name(entry)), "").unwrap();
        }
        let set = pinned_set(&ctx).unwrap();
        assert_eq!(set.dir, ctx.cache.fonts);
        assert_eq!(
            set.font_set_id,
            font_set_id(lock.fonts.iter().map(|e| e.sha256.as_str())).unwrap()
        );
        let synced = stamp(&full);
        assert_ne!(synced.run_key().unwrap(), unsynced.run_key().unwrap());
        assert_eq!(synced.fonts, unsynced.fonts);
        assert_ne!(synced.corpus[&full[1].pair], full[1].digest);
        // A deck with no export draws the lane's set either way.
        assert_eq!(synced.corpus[&full[0].pair], full[0].digest);
        // The two runs are incomparable on the deck that drew other fonts, and only on it.
        assert_eq!(
            synced.incomparable(&unsynced, &[full[0].pair.clone(), full[1].pair.clone()]),
            ["corpus[keynote-talk:keynote]"]
        );
        // A run that selects fewer decks compares with the full run on the decks they share.
        let partial = stamp(&full[..1]);
        assert_eq!(partial.fonts, synced.fonts);
        assert!(
            partial
                .incomparable(&synced, &[full[0].pair.clone()])
                .is_empty()
        );
        // A partly synced set is not the pinned one.
        std::fs::remove_file(ctx.cache.fonts.join(font_cache_name(&lock.fonts[1]))).unwrap();
        assert_eq!(stamp(&full).run_key().unwrap(), unsynced.run_key().unwrap());
    }

    #[test]
    fn the_deck_timeline_agrees_with_the_ir_in_structure() {
        let slide = DeckSlide {
            id: "kn-1".into(),
            hidden: false,
            timeline: vec![
                deck_step("a", "click", "keynote:apple:dissolve"),
                deck_step("b", "afterPrevious", "keynote:apple:move in"),
            ],
            transition: Some(DeckTransition {
                native: "keynote:apple:magic move".into(),
                fallback: "dissolve".into(),
                duration_ms: json!(700),
                effect: Some("morph".into()),
                params: None,
                advance: None,
            }),
            match_keys: None,
        };
        let mut ir = IrSlide {
            id: "kn-1".into(),
            hidden: false,
            timeline: vec![ir_step("a", "click"), ir_step("b", "afterPrevious")],
            transition: Some(IrTransition {
                duration_ms: json!(700),
                effect: IrEffect {
                    id: "keynote:apple:magic-move-implied-motion-path".into(),
                    fallback: "dissolve".into(),
                },
            }),
        };
        // Native ids are the deck's own provenance: only the structure compares.
        assert_eq!(compare_ir(&slide, Some(&ir)), (Some(true), Vec::new()));
        // A swapped pair of steps disagrees.
        ir.timeline.swap(0, 1);
        let (agrees, mismatches) = compare_ir(&slide, Some(&ir));
        assert_eq!(agrees, Some(false));
        assert_eq!(
            mismatches.iter().map(|m| m.step).collect::<Vec<_>>(),
            [Some(0), Some(1)]
        );
        // An IR without motion for the slide says nothing.
        assert_eq!(compare_ir(&slide, None), (None, Vec::new()));
        let empty = IrSlide {
            id: "kn-1".into(),
            hidden: false,
            timeline: Vec::new(),
            transition: None,
        };
        assert_eq!(compare_ir(&slide, Some(&empty)), (None, Vec::new()));
    }
}
