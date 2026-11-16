//! Walks a bundled deck with its own player (`motion.js`), slide by slide and click by click, and records what
//! played: every animation of each batch the player made, frames of each batch paused at chosen times with what each
//! animated node showed then (its box, the computed value of each property animated on it, whether it shows), the
//! frame once each stage settled with what the same nodes show then, the player's still render of the same stage, and
//! which build units showed. Frames are PNGs stored by sha256; the walk keeps only their hashes.
//!
//! A stage's batch is paused at its start, middle and end, and each step of it at its own start, quarter, middle,
//! three quarters and end, so the checks see every step move ([`super::check`]); a step's sets ([`model::switches`])
//! also just before and just after each switch (at most [`moved::SWITCHES_SEEN`] per step), so they see it switch
//! there.
//!
//! An endless repeat loops at the times the walk pauses, and the batch's end is a click that cuts it ([`pauses`]): it
//! holds where its round ends, so the stage's end frame and settled frame show the cut, as the still render does,
//! not the end of its first round.
//!
//! A walk covers a run of shown slides on its own page: it cuts to the slide before the run (nothing plays), then
//! moves on, so each slide of the run is entered with its transition as a presenter enters it.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::Deserialize;
use serde_json::{Value, json};
use slidock_bench_schema::Sha256;

use super::check::{covers, interval, moved, owners};
use super::deck::MotionDeck;
use super::model::{self, Unit, step_units};
use crate::cache::write_atomic;
use crate::renderer::session::{Browser, MotionPage, MotionPageRequest, RenderError, Router};

/// The page side of the walk.
pub const MOTION_JS: &str = include_str!("motion.js");

/// The per-mille of a transition the lane screenshots: its ends, and the three rows.
pub const TRANSITION_PM: [u16; 5] = [0, 250, 500, 750, 1000];
/// How many more screenshots a paused frame takes, at most, to hold still.
const STILL_TRIES: usize = 6;

/// The per-mille of a stage's batch the lane screenshots: start, middle, end.
pub const STAGE_PM: [u16; 3] = [0, 500, 1000];
/// Where in its own span the lane pauses each step: start, quarters, end.
pub const STEP_PM: [u16; 5] = [0, 250, 500, 750, 1000];
/// Times closer than this (ms) are one pause.
pub const SAME_MS: f64 = 0.5;
/// How long after a set's begin the lane pauses to see it switched.
pub const SWITCH_MS: f64 = 1.0;

/// One animation the player created, as the page describes it.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct Animation {
    /// The target node, numbered in the page (distinct targets count the delivered units).
    pub node: u64,
    /// The `data-el` of the target or its nearest ancestor that has one.
    pub element: Option<String>,
    /// The `data-p` of the target or its nearest ancestor that has one.
    pub paragraph: Option<u32>,
    /// The target is a word or character the player split out.
    pub split: bool,
    /// The slide whose layer the target is in.
    pub layer: Option<String>,
    /// `incoming`, `current`, `clone-in`, `clone-out`, `stage` or `other`.
    pub role: String,
    /// The target is a slide layer itself.
    pub whole: bool,
    pub keys: Vec<String>,
    pub delay: f64,
    pub duration: f64,
    /// `-1` for an endless repeat.
    pub iterations: f64,
    /// Its playback direction (`alternate` plays each round forward then back).
    #[serde(default)]
    pub direction: Option<String>,
    pub easing: Option<String>,
    pub fill: Option<String>,
    /// Its end time; `None` for an endless repeat.
    pub end: Option<f64>,
}

impl Animation {
    /// An endless repeat: it plays until a click cuts it.
    pub fn is_endless(&self) -> bool {
        self.end.is_none()
    }

    /// When it ends: its end, or, for an endless repeat, the end of its round (forward then back when it alternates),
    /// where a cut leaves it.
    pub fn round_end(&self) -> f64 {
        self.end.unwrap_or_else(|| {
            let back = matches!(
                self.direction.as_deref(),
                Some("alternate" | "alternate-reverse")
            );
            self.delay + self.duration * if back { 2.0 } else { 1.0 }
        })
    }

    /// The zero-length `opacity: 0` hold the player adds after an exit (or a step that hides its target after).
    pub fn is_hide(&self) -> bool {
        self.duration == 0.0 && self.fill.as_deref() == Some("forwards") && self.keys == ["opacity"]
    }
}

/// What the page measured of an animated node at one time.
#[derive(Clone, Debug, PartialEq, Deserialize)]
pub struct NodeProbe {
    pub node: u64,
    /// Its border box on the page with transforms applied: x, y, width, height (CSS pixels, one per frame pixel).
    pub rect: [f64; 4],
    /// The computed value of each property the player's animations on it animate.
    pub pose: BTreeMap<String, String>,
    /// Connected, visible, and not transparent up to the page.
    pub shown: bool,
    /// Still in the document (a word or character the player split out is gone once the stage settles).
    pub connected: bool,
}

impl NodeProbe {
    pub fn center(&self) -> (f64, f64) {
        let [x, y, w, h] = self.rect;
        (x + w / 2.0, y + h / 2.0)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct PageProbe {
    nodes: Vec<NodeProbe>,
}

/// A batch paused at one time: the frame, and what each animated node showed.
#[derive(Clone, Debug, PartialEq)]
pub struct Sample {
    /// The batch's time; −1 for the instant before it.
    pub at_ms: f64,
    pub frame: Sha256,
    pub nodes: Vec<NodeProbe>,
}

impl Sample {
    pub fn node(&self, node: u64) -> Option<&NodeProbe> {
        self.nodes.iter().find(|probe| probe.node == node)
    }
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageBatch {
    animations: Vec<Animation>,
    duration_ms: f64,
    /// The batch moves the slides of a transition (the incoming slide is marked while it runs).
    transition: bool,
}

/// A diagnostic the player reported.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct PlayerDiagnostic {
    pub code: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct Position {
    slide: usize,
    step: u32,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
struct Report {
    batch: Option<PageBatch>,
    position: Position,
    diagnostics: Vec<PlayerDiagnostic>,
}

/// A batch of animations the player made at once, and frames of it paused at per-mille of its length and at each of
/// its steps' [`STEP_PM`].
#[derive(Clone, Debug, PartialEq)]
pub struct Batch {
    pub animations: Vec<Animation>,
    /// When it ends, one round of an endless repeat counted ([`batch_length`]).
    pub duration_ms: f64,
    pub frames: Vec<(u16, Sha256)>,
    /// Every pause, by time.
    pub samples: Vec<Sample>,
}

impl Batch {
    pub fn frame(&self, pm: u16) -> Option<&Sha256> {
        self.frames
            .iter()
            .find(|(at, _)| *at == pm)
            .map(|(_, sha)| sha)
    }

    /// The pause at `at_ms` (any time before the batch is the instant before it).
    pub fn sample(&self, at_ms: f64) -> Option<&Sample> {
        self.samples
            .iter()
            .find(|s| (s.at_ms - at_ms).abs() <= SAME_MS || (s.at_ms < 0.0 && at_ms < 0.0))
    }

    /// Its pause at per-mille `pm` of its length.
    pub fn sample_pm(&self, pm: u16) -> Option<&Sample> {
        self.sample(pm_ms(self.duration_ms, pm))
    }
}

/// The batch time of per-mille `pm` of `span` ms from `start`: the start itself is the instant before it, where
/// zero-length switches have not flipped yet.
pub fn at(start: f64, span: f64, pm: u16) -> f64 {
    if pm == 0 && start <= 0.0 {
        -1.0
    } else {
        start + span * f64::from(pm) / 1000.0
    }
}

fn pm_ms(duration: f64, pm: u16) -> f64 {
    at(0.0, duration, pm)
}

/// A batch's length: its last end (`page`, the page's reading of its finite animations), counting one round of each
/// endless repeat, as the timeline counts it.
pub fn batch_length(page: f64, animations: &[Animation]) -> f64 {
    animations
        .iter()
        .map(Animation::round_end)
        .filter(|end| end.is_finite())
        .fold(page.max(0.0), f64::max)
}

/// The pauses of a batch at `times` (ascending), each with whether the walk cuts the batch's endless repeats first: a
/// batch that has one is cut at its end (`duration`), as the click that ends it, and every pause from there on sees
/// the cut.
pub fn pauses(times: &[f64], duration: f64, animations: &[Animation]) -> Vec<(f64, bool)> {
    let endless = animations.iter().any(Animation::is_endless);
    times
        .iter()
        .map(|ms| (*ms, endless && *ms >= duration - SAME_MS))
        .collect()
}

/// Where a stage's batch pauses: its [`STAGE_PM`], and each step of group `group` at its [`STEP_PM`] (a step's span is
/// the first round of its animations: [`interval`]).
pub fn stage_times(
    steps: &[super::deck::DeckStep],
    group: u32,
    animations: &[Animation],
    duration: f64,
) -> Vec<f64> {
    let mut times: Vec<f64> = STAGE_PM.iter().map(|pm| pm_ms(duration, *pm)).collect();
    let groups = model::groups(steps);
    let current: Vec<&Animation> = animations
        .iter()
        .filter(|a| !a.whole && a.role == "current")
        .collect();
    let owners = owners(steps, group, &current);
    for (index, (step, g)) in steps.iter().zip(&groups).enumerate() {
        if *g != group {
            continue;
        }
        let own: Vec<&Animation> = current
            .iter()
            .copied()
            .filter(|a| covers(step, a))
            .collect();
        // A set is seen just before and just after its begin, on the animations the checks give its step.
        if let Some(Ok(switches)) = model::switches(step, true) {
            let attributed: Vec<&Animation> = current
                .iter()
                .zip(&owners)
                .filter(|(_, owner)| **owner == Some(index))
                .map(|(a, _)| *a)
                .collect();
            if let Ok(timed) = moved::timed_switches(&switches, &attributed, &|_| String::new()) {
                times.extend(moved::switch_pauses(&timed));
            }
        }
        if let Some((start, end)) = interval(&own)
            && end > start
        {
            times.extend(STEP_PM.iter().map(|pm| at(start, end - start, *pm)));
        }
    }
    times.sort_by(f64::total_cmp);
    times.dedup_by(|b, a| (*b - *a).abs() <= SAME_MS);
    times
}

/// A stage: the click group that played (none for a stage with nothing to play), and the slide once settled.
#[derive(Clone, Debug, PartialEq)]
pub struct StageSeen {
    pub batches: Vec<Batch>,
    pub settled: Sha256,
    /// Once settled, what every node the stage's batches (and stage 0's transition) animated shows.
    pub settled_nodes: Vec<NodeProbe>,
    /// The player's still render of the slide at this step.
    pub still: Sha256,
    /// Per unit of [`units`], whether it shows once settled; `None` when the slide has no such unit.
    pub visible: Vec<Option<bool>>,
    /// The words the settled stage draws, in slide points, when the walk measures them (a deck with a stages
    /// oracle).
    pub words: Option<Vec<StageWord>>,
}

/// A word a settled stage draws in one text node: its text and box in slide points, and what joining it with its
/// neighbours needs ([`super::stages::frame_words`]).
#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StageWord {
    pub text: String,
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    /// It starts its text node.
    pub join_before: bool,
    /// It ends its text node.
    pub join_after: bool,
    /// Its element's `data-el`.
    pub el: Option<String>,
    /// Its text box (`[data-sd-text]`), by its order in the slide.
    pub text_box: Option<u32>,
    /// Its font size in slide points.
    pub size: f64,
}

/// One slide entered and stepped through.
#[derive(Clone, Debug, PartialEq)]
pub struct SlideSeen {
    pub index: usize,
    /// The frame just before the move into the slide; `None` for the first slide shown.
    pub before: Option<Sha256>,
    pub transition: Option<Batch>,
    pub stages: Vec<StageSeen>,
    pub diagnostics: Vec<PlayerDiagnostic>,
    /// Why the walk stopped on it.
    pub failed: Option<String>,
}

/// The units a slide's steps build, in a stable order: what the walk measures.
pub fn units(deck: &MotionDeck, index: usize) -> Vec<Unit> {
    let mut units: Vec<Unit> = deck
        .slides
        .get(index)
        .map(|slide| slide.timeline.iter().flat_map(step_units).collect())
        .unwrap_or_default();
    units.sort();
    units.dedup();
    units
}

/// Where a walk keeps its frames: `<dir>/<sha256>.png`.
pub struct FrameStore {
    pub dir: PathBuf,
}

impl FrameStore {
    pub fn path(&self, sha: &Sha256) -> PathBuf {
        self.dir.join(format!("{sha}.png"))
    }

    fn keep(&self, png: &[u8]) -> std::result::Result<Sha256, RenderError> {
        let sha = Sha256::of(png);
        let path = self.path(&sha);
        if !path.is_file() {
            write_atomic(&path, png).map_err(|error| RenderError::Crash(error.to_string()))?;
        }
        Ok(sha)
    }
}

fn parse<T: for<'de> Deserialize<'de>>(
    value: Value,
    what: &str,
) -> std::result::Result<T, RenderError> {
    serde_json::from_value(value).map_err(|error| RenderError::Crash(format!("{what}: {error}")))
}

/// A page and the deck it walks.
struct Walk<'a> {
    page: &'a MotionPage,
    deck: &'a MotionDeck,
    store: &'a FrameStore,
    words: bool,
}

impl Walk<'_> {
    async fn call(&self, expression: &str) -> std::result::Result<Value, RenderError> {
        let trace = std::env::var_os("SLIDOCK_BENCH_TRACE").is_some();
        if trace {
            eprintln!("call {} {expression}", self.page.id());
        }
        let value = self
            .page
            .call(&format!("window.__sdMotion.{expression}"))
            .await;
        if trace {
            eprintln!("done {} {expression}", self.page.id());
        }
        value
    }

    async fn report(&self, expression: &str) -> std::result::Result<Report, RenderError> {
        let mut report: Report = parse(self.call(expression).await?, expression)?;
        if let Some(batch) = report.batch.as_mut() {
            batch.duration_ms = batch_length(batch.duration_ms, &batch.animations);
        }
        Ok(report)
    }

    /// A screenshot once the frame holds still: a layer composited for an animation, or a large image, may show
    /// unrastered tiles for a few frames, so the frame is taken again until two in a row agree.
    async fn shot(&self) -> std::result::Result<Sha256, RenderError> {
        let mut last = self.page.shot().await?;
        for _ in 0..STILL_TRIES {
            self.call("frames(2)").await?;
            let png = self.page.shot().await?;
            if png == last {
                break;
            }
            last = png;
        }
        self.store.keep(&last)
    }

    /// Screenshots the paused batch at each time (`times`, which hold each of `marks`), with what its nodes show, then
    /// lets it go: the move's next report.
    async fn play(
        &self,
        batch: PageBatch,
        marks: &[u16],
        times: &[f64],
        diagnostics: &mut Vec<PlayerDiagnostic>,
    ) -> std::result::Result<(Batch, Report), RenderError> {
        let mut samples: Vec<Sample> = Vec::with_capacity(times.len());
        for (ms, cut) in pauses(times, batch.duration_ms, &batch.animations) {
            let probe: PageProbe = parse(
                self.call(&format!("seek({}, {cut})", json!(ms))).await?,
                "seek",
            )?;
            samples.push(Sample {
                at_ms: ms,
                frame: self.shot().await?,
                nodes: probe.nodes,
            });
        }
        let mut next = self.report("finish()").await?;
        diagnostics.append(&mut next.diagnostics);
        let mut played = Batch {
            animations: batch.animations,
            duration_ms: batch.duration_ms,
            frames: Vec::new(),
            samples,
        };
        played.frames = marks
            .iter()
            .filter_map(|pm| Some((*pm, played.sample_pm(*pm)?.frame.clone())))
            .collect();
        Ok((played, next))
    }

    /// What each of `nodes` shows now.
    async fn probe(
        &self,
        nodes: &BTreeSet<u64>,
    ) -> std::result::Result<Vec<NodeProbe>, RenderError> {
        let ids: Vec<u64> = nodes.iter().copied().collect();
        parse(self.call(&format!("probe({})", json!(ids))).await?, "probe")
    }

    /// Plays a stage's batches, then records the settled stage. `entered` holds the nodes stage 0's transition
    /// animated.
    async fn stage(
        &self,
        index: usize,
        step: u32,
        mut report: Report,
        entered: &[Animation],
        diagnostics: &mut Vec<PlayerDiagnostic>,
    ) -> std::result::Result<StageSeen, RenderError> {
        let units = Value::Array(
            units(self.deck, index)
                .into_iter()
                .map(|unit| json!([unit.element, unit.paragraph]))
                .collect(),
        );
        let timeline = self
            .deck
            .slides
            .get(index)
            .map(|slide| slide.timeline.as_slice())
            .unwrap_or_default();
        let mut batches: Vec<Batch> = Vec::new();
        while let Some(batch) = report.batch.take() {
            let times = stage_times(timeline, step, &batch.animations, batch.duration_ms);
            let (played, next) = self.play(batch, &STAGE_PM, &times, diagnostics).await?;
            batches.push(played);
            report = next;
        }
        let settled = self.shot().await?;
        let words = if self.words {
            Some(parse(
                self.call(&format!("words({})", json!(self.deck.size.w)))
                    .await?,
                "words",
            )?)
        } else {
            None
        };
        let nodes: BTreeSet<u64> = batches
            .iter()
            .flat_map(|b| b.animations.iter())
            .chain(entered)
            .map(|a| a.node)
            .collect();
        let settled_nodes = self.probe(&nodes).await?;
        let visible: Vec<Option<bool>> =
            parse(self.call(&format!("visible({units})")).await?, "visible")?;
        self.call(&format!("still({index}, {step})")).await?;
        let still = self.shot().await?;
        self.call("still(null, 0)").await?;
        Ok(StageSeen {
            batches,
            settled,
            settled_nodes,
            still,
            visible,
            words,
        })
    }

    /// Enters slide `index` by the move whose first report is `report`, and steps through it.
    async fn slide(
        &self,
        index: usize,
        before: Option<Sha256>,
        mut report: Report,
    ) -> std::result::Result<SlideSeen, RenderError> {
        let mut diagnostics = std::mem::take(&mut report.diagnostics);
        if report.position.slide != index {
            return Err(RenderError::Crash(format!(
                "the player moved to slide {} instead of {index}",
                report.position.slide
            )));
        }
        let mut transition = None;
        if report.batch.as_ref().is_some_and(|batch| batch.transition)
            && let Some(batch) = report.batch.take()
        {
            let times: Vec<f64> = TRANSITION_PM
                .iter()
                .map(|pm| pm_ms(batch.duration_ms, *pm))
                .collect();
            let (played, next) = self
                .play(batch, &TRANSITION_PM, &times, &mut diagnostics)
                .await?;
            transition = Some(played);
            report = next;
            diagnostics.append(&mut report.diagnostics);
        }
        let clicks = self
            .deck
            .slides
            .get(index)
            .map_or(0, |slide| slide.clicks());
        let entered: Vec<Animation> = transition
            .as_ref()
            .map(|t: &Batch| t.animations.clone())
            .unwrap_or_default();
        let mut stages = vec![
            self.stage(index, 0, report, &entered, &mut diagnostics)
                .await?,
        ];
        for step in 1..=clicks {
            let mut next = self.report("next()").await?;
            diagnostics.append(&mut next.diagnostics);
            if next.position.slide != index || next.position.step != step {
                return Err(RenderError::Crash(format!(
                    "a click on slide {index} at step {} reached slide {} step {}",
                    step - 1,
                    next.position.slide,
                    next.position.step
                )));
            }
            stages.push(self.stage(index, step, next, &[], &mut diagnostics).await?);
        }
        Ok(SlideSeen {
            index,
            before,
            transition,
            stages,
            diagnostics,
            failed: None,
        })
    }
}

/// What a walk of a run of slides needs.
pub struct WalkRequest {
    pub url: String,
    pub router: Option<Arc<dyn Router>>,
    pub deck: Arc<MotionDeck>,
    /// Shown slides, in order, consecutive among the shown ones.
    pub slides: Vec<usize>,
    pub store: Arc<FrameStore>,
    /// Measures the words of every settled stage (for a stages oracle's text score).
    pub words: bool,
}

/// Walks a run of slides. A slide the walk cannot finish is returned failed, and the walk stops there.
pub async fn walk(browser: &Browser, request: &WalkRequest) -> Vec<SlideSeen> {
    let deck = &request.deck;
    let size = (deck.size.w * 4.0 / 3.0, deck.size.h * 4.0 / 3.0);
    let failed = |index: usize, why: String| SlideSeen {
        index,
        before: None,
        transition: None,
        stages: Vec::new(),
        diagnostics: Vec::new(),
        failed: Some(why),
    };
    let page = match browser
        .open_motion_page(&MotionPageRequest {
            url: request.url.clone(),
            size,
            init_script: MOTION_JS.to_owned(),
            router: request.router.clone(),
        })
        .await
    {
        Ok(page) => page,
        Err(error) => {
            return request
                .slides
                .iter()
                .map(|index| failed(*index, error.to_string()))
                .collect();
        }
    };
    let walk = Walk {
        page: &page,
        deck,
        store: &request.store,
        words: request.words,
    };
    let mut seen = Vec::new();
    let result: std::result::Result<(), (usize, RenderError)> = async {
        let first = request.slides.first().copied().unwrap_or_default();
        let booted = walk.call("boot()").await.map_err(|e| (first, e))?;
        if std::env::var_os("SLIDOCK_BENCH_TRACE").is_some() {
            eprintln!("boot {booted}");
        }
        let shown = deck.shown();
        let mut before: Option<Sha256> = None;
        for (n, index) in request.slides.iter().enumerate() {
            let report = if n == 0 && shown.first() == Some(index) {
                walk.report("pending()").await
            } else {
                if n == 0 {
                    let previous = shown
                        .iter()
                        .rev()
                        .find(|i| *i < index)
                        .copied()
                        .unwrap_or_default();
                    let clicks = deck.slides.get(previous).map_or(0, |s| s.clicks());
                    walk.report(&format!("cut({previous}, {clicks})"))
                        .await
                        .map_err(|e| (*index, e))?;
                    before = Some(walk.shot().await.map_err(|e| (*index, e))?);
                }
                walk.report("next()").await
            }
            .map_err(|e| (*index, e))?;
            if std::env::var_os("SLIDOCK_BENCH_TRACE").is_some() {
                eprintln!("walk {} slide {index}", request.url);
            }
            let slide = walk
                .slide(*index, before.clone(), report)
                .await
                .map_err(|e| (*index, e))?;
            before = slide.stages.last().map(|stage| stage.settled.clone());
            seen.push(slide);
        }
        Ok(())
    }
    .await;
    if let Err((index, error)) = result {
        seen.push(failed(index, error.to_string()));
    }
    page.close(browser).await;
    seen
}

/// Splits the shown slides into runs of at most `size`.
pub fn runs(shown: &[usize], size: usize) -> Vec<Vec<usize>> {
    shown.chunks(size.max(1)).map(<[usize]>::to_vec).collect()
}

/// The frame folder of a run.
pub fn frames_dir(run_dir: &Path) -> PathBuf {
    run_dir.join("frames")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_scripts_keep_the_120_column_style() {
        // Prettier wraps the code but not the comments.
        for (name, script) in [
            ("motion.js", MOTION_JS),
            ("shell.js", super::super::shell::SHELL_JS),
        ] {
            let long: Vec<usize> = script
                .lines()
                .enumerate()
                .filter(|(_, line)| line.chars().count() > 120)
                .map(|(i, _)| i + 1)
                .collect();
            assert_eq!(long, Vec::<usize>::new(), "{name}: lines over 120 columns");
        }
    }

    #[test]
    fn runs_cover_every_shown_slide_once() {
        let shown = [0, 1, 3, 4, 5, 9];
        let runs = runs(&shown, 4);
        assert_eq!(runs, vec![vec![0, 1, 3, 4], vec![5, 9]]);
        assert_eq!(super::runs(&[], 4), Vec::<Vec<usize>>::new());
    }

    fn timed(delay: f64, duration: f64, endless: bool, direction: &str) -> Animation {
        Animation {
            node: 0,
            element: None,
            paragraph: None,
            split: false,
            layer: None,
            role: "current".into(),
            whole: false,
            keys: vec!["transform".into()],
            delay,
            duration,
            iterations: if endless { -1.0 } else { 1.0 },
            direction: Some(direction.into()),
            easing: None,
            fill: Some("both".into()),
            end: (!endless).then_some(delay + duration),
        }
    }

    #[test]
    fn a_batch_counts_one_round_of_an_endless_repeat() {
        // The page reads only finite ends: a batch of one endless repeat reads 0.
        assert_eq!(
            batch_length(0.0, &[timed(100.0, 500.0, true, "normal")]),
            600.0
        );
        // One that plays back and forth: its round is forward then back.
        assert_eq!(
            batch_length(0.0, &[timed(0.0, 500.0, true, "alternate")]),
            1000.0
        );
        // A longer finite animation ends the batch.
        let both = [
            timed(0.0, 500.0, true, "normal"),
            timed(0.0, 800.0, false, "normal"),
        ];
        assert_eq!(batch_length(800.0, &both), 800.0);
        assert_eq!(
            batch_length(300.0, &[timed(0.0, 300.0, false, "normal")]),
            300.0
        );
    }

    #[test]
    fn an_endless_repeat_is_cut_at_the_batch_end_not_at_the_end_of_its_first_round() {
        let loop_ = [timed(0.0, 500.0, true, "normal")];
        let duration = batch_length(0.0, &loop_);
        let times = [-1.0, 125.0, 250.0, 375.0, 500.0];
        assert_eq!(
            pauses(&times, duration, &loop_),
            vec![
                (-1.0, false),
                (125.0, false),
                (250.0, false),
                (375.0, false),
                (500.0, true)
            ]
        );
        // Beside a longer step it loops on (its first round's end is a pause like any) until the batch ends.
        let both = [
            timed(0.0, 500.0, true, "normal"),
            timed(0.0, 800.0, false, "normal"),
        ];
        let duration = batch_length(800.0, &both);
        assert_eq!(
            pauses(&[500.0, 600.0, 800.0], duration, &both),
            vec![(500.0, false), (600.0, false), (800.0, true)]
        );
        // A batch with no endless repeat is never cut.
        let finite = [timed(0.0, 500.0, false, "normal")];
        assert!(pauses(&times, 500.0, &finite).iter().all(|(_, cut)| !cut));
    }

    #[test]
    fn a_hide_is_a_zero_length_forward_opacity() {
        let animation = |duration: f64, fill: &str, keys: &[&str]| Animation {
            node: 0,
            element: None,
            paragraph: None,
            split: false,
            layer: None,
            role: "current".into(),
            whole: false,
            keys: keys.iter().map(|k| (*k).to_owned()).collect(),
            delay: 500.0,
            duration,
            iterations: 1.0,
            direction: None,
            easing: None,
            fill: Some(fill.into()),
            end: Some(500.0 + duration),
        };
        assert!(animation(0.0, "forwards", &["opacity"]).is_hide());
        assert!(!animation(0.0, "both", &["opacity"]).is_hide());
        assert!(!animation(500.0, "forwards", &["opacity"]).is_hide());
        assert!(!animation(0.0, "forwards", &["opacity", "transform"]).is_hide());
    }
}
