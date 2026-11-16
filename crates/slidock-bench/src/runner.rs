//! The work of a run, shared by `run`, `baseline` and `parity reference`: compile one (presentation, frontend) pair in a
//! child process, then render, score and classify each of its rows through a pool of browser contexts. Every row is
//! isolated: whatever goes wrong becomes that row's class, and the other rows carry on. Records are keyed by row id, so
//! the pool's finishing order never reaches the results.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use futures::{FutureExt, StreamExt};
use slidock_bench_metrics::{
    Classification, CompileFailure, Decline, HostLibm, Rect, Region, RenderErrorReason, Ring,
    RowFacts, Size, TextWord, classify, decode_png, gradient_regions, metric_pair, oracle_words,
    quantise_scores, score_render, shadow_rings, unscored_class,
};
use slidock_bench_schema::corpus::{LossKind, Presentation, RowRef, Source};
use slidock_bench_schema::results::{self, OraclePage, RowRecord};
use slidock_bench_schema::{RowId, Sha256};
use slidock_format::paths::{file_id, layout_path, slide_path};
use slidock_format::{ProblemCode, lint_presentation};

use crate::cache::{remove, write_atomic, write_json};
use crate::compile::{
    CompileReport, CompileSettings, CompileStatus, PairCompile, compile_pair, read_pair_compile,
};
use crate::corpus::Corpus;
use crate::error::{BenchError, Result};
use crate::gradient::{
    BACKGROUND_EL, IrGradients, gradient_metrics, gradient_targets, is_approximated,
    read_ir_gradients,
};
use crate::hash::sha256;
use crate::oracle::resolve::{OracleResolver, PageError, ResolvedOracle};
use crate::renderer::route::{PresentationRouter, composed_documents, read_compiled, slide_url};
use crate::renderer::session::{Browser, Measure, RenderError, RenderRequest};
use crate::results::row_paths;
use crate::shadow::{IrShadows, is_not_drawn, read_ir_shadows, shadow_metrics, shadow_targets};

/// A pair's compile, or none when the corpus check failed first.
#[derive(Clone, Debug)]
pub enum PairState {
    Skipped,
    Compiled(PairCompile),
}

impl PairState {
    fn compile(&self) -> Option<&PairCompile> {
        match self {
            PairState::Skipped => None,
            PairState::Compiled(compile) => Some(compile),
        }
    }
}

/// One diagnostic that applies to a slide, and the container it comes through.
struct Applying<'a> {
    code: &'a str,
    severity: &'a str,
    feature: Option<&'a str>,
    count: u32,
    via: Option<&'a str>,
    /// The element the diagnostic names.
    node: Option<&'a str>,
}

/// Diagnostics that apply to a slide: its own, those of the layout and master it shows, and presentation-level ones.
fn applying_diagnostics<'a>(report: Option<&'a CompileReport>, slide: &str) -> Vec<Applying<'a>> {
    let Some(report) = report else {
        return Vec::new();
    };
    let shown: BTreeSet<&str> = report
        .shows
        .as_ref()
        .and_then(|shows| shows.get(slide))
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    report
        .diagnostics
        .iter()
        .filter_map(|d| {
            let via = match d.container.as_deref() {
                Some(container) if container == slide => None,
                None => Some("presentation"),
                Some(container) if shown.contains(container) => Some(container),
                Some(_) => return None,
            };
            Some(Applying {
                code: &d.code,
                severity: &d.severity,
                feature: d.feature.as_deref(),
                count: d.count.unwrap_or(1),
                via,
                node: d.node.as_deref(),
            })
        })
        .collect()
}

fn diagnostic_counts(applying: &[Applying<'_>]) -> BTreeMap<String, u32> {
    let mut counts = BTreeMap::new();
    for d in applying {
        *counts.entry(d.code.to_owned()).or_default() += d.count;
    }
    counts
}

fn declines_of(applying: &[Applying<'_>]) -> Vec<Decline> {
    applying
        .iter()
        .filter(|d| d.severity == "decline")
        .map(|d| Decline {
            code: d.code.to_owned(),
            feature: d.feature.map(str::to_owned),
            via: d.via.map(str::to_owned),
        })
        .collect()
}

/// Which known losses of the manifest touch a row: through its diagnostics or the features its containers hold.
fn known_loss_kinds(
    presentation: &Presentation,
    row: &RowRef,
    report: Option<&CompileReport>,
) -> Vec<LossKind> {
    let losses = presentation
        .known_loss
        .as_ref()
        .and_then(|losses| losses.get(&row.id.frontend()))
        .map_or(&[][..], Vec::as_slice);
    if losses.is_empty() {
        return Vec::new();
    }
    let slide = row.id.slide().as_str();
    let mut features: BTreeSet<String> = applying_diagnostics(report, slide)
        .iter()
        .filter_map(|d| d.feature.map(str::to_owned))
        .collect();
    let shows = report
        .and_then(|r| r.shows.as_ref())
        .and_then(|s| s.get(slide))
        .cloned()
        .unwrap_or_default();
    for container in std::iter::once(slide.to_owned()).chain(shows) {
        if let Some(own) = report.and_then(|r| r.features.get(&container)) {
            features.extend(own.iter().cloned());
        }
    }
    let kinds: BTreeSet<LossKind> = losses
        .iter()
        .filter(|loss| features.contains(loss.feature.as_str()))
        .map(|loss| loss.kind)
        .collect();
    kinds.into_iter().collect()
}

/// A compiled folder as the rows read it, built once per folder: the file set, every slide's composed document,
/// the lint problems that would run code, and the element shadows and gradients of its IR.
struct PairDocuments {
    documents: std::result::Result<BTreeMap<String, String>, String>,
    runs_code: Vec<(String, ProblemCode)>,
    shadows: IrShadows,
    gradients: IrGradients,
}

/// Lint problems that would run code in the renderer. The bench lints the output itself: the compiler is under
/// test.
const RUNS_CODE: [ProblemCode; 3] = [
    ProblemCode::Script,
    ProblemCode::EventHandler,
    ProblemCode::JavascriptUrl,
];

fn pair_documents(dir: &Path) -> PairDocuments {
    let files = match read_compiled(dir) {
        Ok(files) => files,
        Err(error) => {
            return PairDocuments {
                documents: Err(error.to_string()),
                runs_code: Vec::new(),
                shadows: IrShadows::default(),
                gradients: IrGradients::default(),
            };
        }
    };
    let runs_code = lint_presentation(&files)
        .into_iter()
        .filter(|p| RUNS_CODE.contains(&p.code))
        .map(|p| (p.file, p.code))
        .collect();
    PairDocuments {
        documents: composed_documents(&files).map_err(|e| e.to_string()),
        runs_code,
        shadows: read_ir_shadows(dir),
        gradients: read_ir_gradients(dir),
    }
}

/// The diagnostics that apply to a row's slide, as the UI lists them.
pub fn applied_diagnostics(
    report: Option<&CompileReport>,
    slide: &str,
) -> Vec<slidock_bench_schema::api::AppliedDiagnostic> {
    let Some(report) = report else {
        return Vec::new();
    };
    let shown: BTreeSet<&str> = report
        .shows
        .as_ref()
        .and_then(|shows| shows.get(slide))
        .into_iter()
        .flatten()
        .map(String::as_str)
        .collect();
    report
        .diagnostics
        .iter()
        .filter_map(|d| {
            let via = match d.container.as_deref() {
                Some(container) if container == slide => None,
                None => Some("presentation".to_owned()),
                Some(container) if shown.contains(container) => Some(container.to_owned()),
                Some(_) => return None,
            };
            Some(slidock_bench_schema::api::AppliedDiagnostic {
                code: d.code.clone(),
                severity: d.severity.clone(),
                message: d.message.clone().unwrap_or_default(),
                feature: d.feature.clone(),
                container: d.container.clone(),
                count: d.count,
                via,
            })
        })
        .collect()
}

/// The elements the shadow-not-drawn diagnostics that apply to a row's slide name.
pub fn row_not_drawn_shadows(report: Option<&CompileReport>, slide: &str) -> Vec<String> {
    not_drawn_nodes(&applying_diagnostics(report, slide))
        .into_iter()
        .map(str::to_owned)
        .collect()
}

/// The elements the gradient-approximated diagnostics that apply to a row's slide name ([`BACKGROUND_EL`] for one
/// of the slide or a shown container that names none).
pub fn row_approximated_gradients(report: Option<&CompileReport>, slide: &str) -> Vec<String> {
    approximated_nodes(&applying_diagnostics(report, slide))
        .into_iter()
        .map(str::to_owned)
        .collect()
}

fn approximated_nodes<'a>(applying: &[Applying<'a>]) -> Vec<&'a str> {
    applying
        .iter()
        .filter(|d| is_approximated(d.code))
        .filter_map(|d| match (d.node, d.via) {
            (Some(node), _) => Some(node),
            (None, Some("presentation")) => None,
            (None, _) => Some(BACKGROUND_EL),
        })
        .collect()
}

fn not_drawn_nodes<'a>(applying: &[Applying<'a>]) -> Vec<&'a str> {
    applying
        .iter()
        .filter(|d| is_not_drawn(d.code))
        .filter_map(|d| d.node)
        .collect()
}

/// Counts of the diagnostics that apply to a row's slide, by code.
pub fn row_diagnostic_counts(report: Option<&CompileReport>, slide: &str) -> BTreeMap<String, u32> {
    diagnostic_counts(&applying_diagnostics(report, slide))
}

/// What every row of a run shares.
pub struct RowContext {
    /// Renders rows; without one, rows that need scoring become render errors.
    pub browser: Option<Arc<Browser>>,
    pub gif_frames: PathBuf,
    folders: Mutex<BTreeMap<PathBuf, Arc<PairDocuments>>>,
    pub log: fn(&str),
}

impl RowContext {
    pub fn new(browser: Option<Arc<Browser>>, gif_frames: PathBuf, log: fn(&str)) -> RowContext {
        RowContext {
            browser,
            gif_frames,
            folders: Mutex::new(BTreeMap::new()),
            log,
        }
    }

    async fn documents(&self, dir: &Path) -> Arc<PairDocuments> {
        let known = self
            .folders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(dir)
            .cloned();
        if let Some(known) = known {
            return known;
        }
        let owned = dir.to_path_buf();
        let built = Arc::new(
            tokio::task::spawn_blocking(move || pair_documents(&owned))
                .await
                .unwrap_or_else(|error| PairDocuments {
                    documents: Err(error.to_string()),
                    runs_code: Vec::new(),
                    shadows: IrShadows::default(),
                    gradients: IrGradients::default(),
                }),
        );
        self.folders
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .entry(dir.to_path_buf())
            .or_insert(built)
            .clone()
    }
}

/// One row to classify.
pub struct RowInput<'a> {
    pub row: &'a RowRef,
    pub presentation: &'a Presentation,
    pub state: &'a PairState,
    /// A problem that makes every row of the pair a render error (corpus hash, oracle).
    pub pair_error: Option<RenderErrorReason>,
    pub oracle: Option<Arc<ResolvedOracle>>,
    pub content: Sha256,
}

/// A row's record, and its render and measure when it was rendered.
pub struct ScoredRow {
    pub record: RowRecord,
    pub render: Option<Vec<u8>>,
    pub measure: Option<Measure>,
    /// The unquantised scores the class was decided from.
    pub raw: Option<slidock_bench_metrics::Scores>,
}

fn reason_of(text: &str) -> Option<results::Reason> {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).ok()
}

fn outcome_of(outcome: slidock_bench_metrics::Outcome) -> results::Outcome {
    match outcome {
        slidock_bench_metrics::Outcome::Match => results::Outcome::Match,
        slidock_bench_metrics::Outcome::Close => results::Outcome::Close,
        slidock_bench_metrics::Outcome::Mismatch => results::Outcome::Mismatch,
        slidock_bench_metrics::Outcome::Declined => results::Outcome::Declined,
        slidock_bench_metrics::Outcome::RenderError => results::Outcome::RenderError,
        slidock_bench_metrics::Outcome::Failed => results::Outcome::Failed,
    }
}

fn record_scores(scores: &slidock_bench_metrics::Scores) -> results::Scores {
    let q = quantise_scores(scores);
    results::Scores {
        ssim: results::SsimScore {
            mean: q.ssim.mean,
            p1: q.ssim.p1,
        },
        color: results::ColorScore {
            mean: q.color.mean,
            p99: q.color.p99,
        },
        pixel: q.pixel,
        text: results::TextScore {
            oracle_words: q.text.oracle_words as u32,
            dom_words: q.text.dom_words as u32,
            matched: q.text.matched as u32,
            recall: q.text.recall,
            precision: q.text.precision,
            median_pm: q.text.median_pm,
            p95_pm: q.text.p95_pm,
            height_ratio_median: q.text.height_ratio_median,
        },
    }
}

/// The record of a row before its class is decided.
fn base_record(input: &RowInput<'_>) -> RowRecord {
    let id = &input.row.id;
    RowRecord {
        presentation: id.presentation().clone(),
        slide: id.slide().clone(),
        frontend: id.frontend(),
        oracle: OraclePage {
            page: input.row.page,
        },
        outcome: results::Outcome::RenderError.into(),
        reason: None,
        code: None,
        data: None,
        declined: None,
        scores: None,
        diagnostics: BTreeMap::new(),
        html: None,
        render: None,
        content: input.content.clone(),
        reference: input.row.reference.then_some(slidock_bench_schema::True),
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

/// The record of a row whose scoring failed, built from the row alone so building it is safe.
pub fn crashed_record(input: &RowInput<'_>) -> RowRecord {
    RowRecord {
        reason: Some(results::Reason::RenderCrash),
        ..base_record(input)
    }
}

/// What a render and its scoring produced.
struct Rendered {
    scores: slidock_bench_metrics::Scores,
    /// `shadow@1`'s rings, worst first.
    rings: Vec<Ring>,
    /// `gradient@1`'s regions, worst first.
    regions: Vec<Region>,
    png: Vec<u8>,
    measure: Measure,
    html: String,
}

/// What a row's render is scored with besides its oracle.
struct ScoreInputs<'a> {
    documents: &'a BTreeMap<String, String>,
    shadows: &'a IrShadows,
    /// The elements the row's shadow-not-drawn diagnostics name.
    not_drawn: &'a [&'a str],
    gradients: &'a IrGradients,
    /// The elements the row's gradient-approximated diagnostics name.
    approximated: &'a [&'a str],
}

async fn render_and_score(
    input: &RowInput<'_>,
    context: &RowContext,
    browser: &Browser,
    compile: &PairCompile,
    inputs: &ScoreInputs<'_>,
) -> std::result::Result<Rendered, RenderErrorReason> {
    let documents = inputs.documents;
    let log = |why: &str| (context.log)(&format!("{}: {why}", input.row.id));
    let slide = input.row.id.slide().as_str();
    let html = documents
        .get(&file_id(slide))
        .cloned()
        .ok_or(RenderErrorReason::RenderCrash)?;
    let (w, h) = compile.size.ok_or(RenderErrorReason::RenderCrash)?;
    let oracle = input
        .oracle
        .as_ref()
        .ok_or(RenderErrorReason::RenderCrash)?;
    let page = oracle
        .page(input.row.page)
        .await
        .map_err(|error| match error {
            PageError::Identity(why) => {
                log(&why);
                RenderErrorReason::OracleIdentity
            }
            PageError::Failed(why) => {
                log(&why);
                RenderErrorReason::RenderCrash
            }
        })?;
    let request = RenderRequest {
        url: slide_url(&file_id(slide)),
        size: (w, h),
        px_per_pt: f64::from(slidock_bench_metrics::RASTER_LONG_SIDE_PX) / w.max(h),
        router: Arc::new(PresentationRouter {
            dir: compile.dir.clone(),
            documents: documents.clone(),
            gif_frames: context.gif_frames.clone(),
        }),
    };
    let shot = browser.render(&request).await.map_err(|error| {
        log(&error.to_string());
        match error {
            RenderError::Timeout => RenderErrorReason::RenderTimeout,
            RenderError::Crash(_) => RenderErrorReason::RenderCrash,
        }
    })?;
    let raster = page.raster.path.clone();
    let measure = shot.measure.clone();
    let text = page.text.to_metrics();
    let page_size = Size {
        w: page.size.w,
        h: page.size.h,
    };
    let png = shot.png.clone();
    let textless = shot.textless;
    let targets = shadow_targets(inputs.shadows, inputs.not_drawn, &shot.measure);
    let gradients = gradient_targets(
        inputs.gradients,
        inputs.approximated,
        &shot.measure,
        slide,
        Size { w, h },
    );
    let scored = tokio::task::spawn_blocking(move || -> std::result::Result<_, String> {
        let oracle = decode_png(&std::fs::read(&raster).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        let image = decode_png(&png).map_err(|e| e.to_string())?;
        let textless = decode_png(&textless).map_err(|e| e.to_string())?;
        let slide_size = Size { w, h };
        let oracle_text = oracle_words(&text, page_size, slide_size);
        let dom: Vec<TextWord> = measure
            .words
            .iter()
            .map(|word| TextWord {
                text: word.text.clone(),
                rect: Rect {
                    x: word.x,
                    y: word.y,
                    w: word.w,
                    h: word.h,
                },
            })
            .collect();
        let scores =
            score_render::<HostLibm>(&oracle, &image, &textless, &oracle_text, &dom, slide_size)
                .map_err(|e| e.to_string())?;
        if targets.is_empty() && gradients.is_empty() {
            return Ok((scores, Vec::new(), Vec::new()));
        }
        let (oracle, render) = metric_pair(&oracle, &image).map_err(|e| e.to_string())?;
        let rings =
            shadow_rings(&oracle, &render, &targets, slide_size).map_err(|e| e.to_string())?;
        let regions = gradient_regions(&oracle, &render, &gradients, slide_size)
            .map_err(|e| e.to_string())?;
        Ok((scores, rings, regions))
    })
    .await
    .map_err(|_| RenderErrorReason::RenderCrash)?
    .map_err(|why| {
        log(&why);
        RenderErrorReason::RenderCrash
    })?;
    let (scores, rings, regions) = scored;
    Ok(Rendered {
        scores,
        rings,
        regions,
        png: shot.png,
        measure: shot.measure,
        html,
    })
}

/// Classifies one row, rendering and scoring it while its class is still open.
pub async fn score_row(input: &RowInput<'_>, context: &RowContext) -> ScoredRow {
    let compile = input.state.compile();
    let report = compile.and_then(|c| c.report.as_ref());
    let slide_id = input.row.id.slide().as_str();
    let applying = applying_diagnostics(report, slide_id);
    let known = known_loss_kinds(input.presentation, input.row, report);
    let mut facts = RowFacts {
        declines: declines_of(&applying),
        render_error: input.pair_error,
        ..RowFacts::default()
    };
    if let Some(compile) = compile {
        match compile.status {
            CompileStatus::Crash => facts.compile = Some(CompileFailure::Crash),
            CompileStatus::Timeout => facts.compile = Some(CompileFailure::Timeout),
            CompileStatus::Memory => facts.compile = Some(CompileFailure::Memory),
            CompileStatus::Refused => {
                facts.compile = Some(CompileFailure::Refused);
                facts.refusal_code = report.and_then(|r| {
                    r.diagnostics
                        .iter()
                        .find(|d| d.severity == "error" && d.container.is_none())
                        .map(|d| d.code.clone())
                });
            }
            CompileStatus::Ok | CompileStatus::Errors => {}
        }
    }
    let slide = compile.and_then(|c| c.slides.get(slide_id));
    if facts.compile.is_none() && compile.is_some() && slide.is_none() {
        facts.missing = true;
    }
    facts.failed_code = slide.and_then(|s| s.failed.clone());

    let mut rendered = None;
    if let (None, Some(compile), Some(slide)) = (unscored_class(&facts), compile, slide) {
        let folder = context.documents(&compile.dir).await;
        let shown = [
            Some(slide_path(&slide.id)),
            slide.layout.as_deref().map(layout_path),
        ];
        facts.lint_code = folder
            .runs_code
            .iter()
            .find(|(file, _)| shown.iter().flatten().any(|path| path == file))
            .map(|(_, code)| code.name().to_owned());
        if unscored_class(&facts).is_none()
            && let Some(browser) = &context.browser
        {
            let not_drawn = not_drawn_nodes(&applying);
            let approximated = approximated_nodes(&applying);
            let outcome = match &folder.documents {
                Ok(documents) => {
                    let inputs = ScoreInputs {
                        documents,
                        shadows: &folder.shadows,
                        not_drawn: &not_drawn,
                        gradients: &folder.gradients,
                        approximated: &approximated,
                    };
                    render_and_score(input, context, browser, compile, &inputs).await
                }
                Err(why) => {
                    (context.log)(&format!("{}: {why}", input.row.id));
                    Err(RenderErrorReason::RenderCrash)
                }
            };
            match outcome {
                Ok(done) => {
                    facts.scores = Some(done.scores);
                    rendered = Some(done);
                }
                Err(reason) => facts.render_error = Some(reason),
            }
        }
    }

    let mut record = base_record(input);
    let decided = classify(&facts);
    record.outcome = outcome_of(decided.outcome()).into();
    match decided {
        Classification::Failed {
            reason,
            code,
            memory,
        } => {
            record.reason = reason_of(reason.as_str());
            record.code = code.filter(|c| !c.is_empty());
            if memory {
                record.data = Some(BTreeMap::from([("reason".to_owned(), "memory".to_owned())]));
            }
        }
        Classification::Declined(decline) => {
            record.declined = Some(results::Decline {
                code: decline.code,
                feature: decline.feature,
                via: decline.via,
            });
        }
        Classification::RenderError(reason) => record.reason = reason_of(reason.as_str()),
        Classification::Scored(_) => {}
    }
    if let Some(done) = &rendered {
        record.scores = Some(record_scores(&done.scores));
        record.shadow = shadow_metrics(&done.rings);
        record.gradient = gradient_metrics(&done.regions);
        record.html = Sha256::new(sha256(&done.html)).ok();
        record.render = Sha256::new(sha256(&done.png)).ok();
    }
    record.diagnostics = diagnostic_counts(&applying);
    if !known.is_empty() {
        record.known_loss = Some(known);
    }
    ScoredRow {
        record,
        render: rendered.as_ref().map(|r| r.png.clone()),
        raw: rendered.as_ref().map(|r| r.scores),
        measure: rendered.map(|r| r.measure),
    }
}

/// Writes a row's record, render and measure into the run folder.
pub fn save_row(run_dir: &Path, row: &RowId, scored: &ScoredRow) -> Result<()> {
    let paths = row_paths(run_dir, row);
    write_json(&paths.record, &scored.record)?;
    match &scored.render {
        Some(png) => write_atomic(&paths.render, png)?,
        None => remove(&paths.render)?,
    }
    match &scored.measure {
        Some(measure) => write_json(&paths.measure, &measure.rounded())?,
        None => remove(&paths.measure)?,
    }
    Ok(())
}

/// One pair's rows to score.
pub struct PairJob<'a> {
    pub presentation: &'a Presentation,
    pub source: &'a Source,
    /// The rows to score, in order.
    pub rows: Vec<RowRef>,
    /// Compile these slides only (`run --only`).
    pub only: Option<Vec<String>>,
}

/// How a run finds and builds what it scores.
pub struct ExecuteOptions<'a> {
    pub run_dir: &'a Path,
    pub corpus: &'a Corpus,
    pub settings: &'a CompileSettings,
    pub oracle: &'a OracleResolver,
    pub content: Sha256,
    pub concurrency: usize,
    /// Reads the compiled output already in the run folder instead of compiling.
    pub reuse_compiled: bool,
}

/// What a run produced: the row records, and the raw scores of the rows it scored.
#[derive(Default)]
pub struct Executed {
    pub records: BTreeMap<RowId, RowRecord>,
    pub raw: BTreeMap<RowId, slidock_bench_metrics::Scores>,
}

/// Compiles and scores every job.
pub async fn execute_pairs(
    jobs: &[PairJob<'_>],
    options: &ExecuteOptions<'_>,
    context: &RowContext,
) -> Result<Executed> {
    let mut executed = Executed::default();
    let mut verified: BTreeMap<String, bool> = BTreeMap::new();
    for job in jobs {
        let presentation = job.presentation;
        let source = job.source;
        let frontend = source.frontend.as_str();
        let id = presentation.id.as_str();
        let ok = match verified.get(id) {
            Some(ok) => *ok,
            None => {
                let ok = options.corpus.verify(presentation)?.is_empty();
                verified.insert(id.to_owned(), ok);
                ok
            }
        };
        let mut pair_error = None;
        let mut state = PairState::Skipped;
        let mut oracle = None;
        if !ok {
            pair_error = Some(RenderErrorReason::CorpusHashMismatch);
        } else {
            let compile = if options.reuse_compiled {
                read_pair_compile(options.run_dir, id, frontend)
            } else {
                let file = options.corpus.source_file(source)?.ok_or_else(|| {
                    BenchError::failed(format!("{id}: {} is not in the corpus", source.file))
                })?;
                compile_pair(
                    options.settings,
                    options.run_dir,
                    id,
                    frontend,
                    &file,
                    job.only.as_deref(),
                )
                .await?
            };
            state = PairState::Compiled(compile);
            match options.oracle.resolve(options.corpus, presentation).await? {
                Err(reason) => pair_error = Some(reason),
                Ok(resolved) => {
                    let expected = source
                        .rows
                        .as_ref()
                        .map(Vec::len)
                        .or(source.row_count.flatten().map(|n| n as usize))
                        .unwrap_or(job.rows.len());
                    if resolved.page_count() != expected {
                        pair_error = Some(RenderErrorReason::OraclePageCount);
                    } else {
                        oracle = Some(resolved);
                    }
                }
            }
        }
        let status = match &state {
            PairState::Skipped => "skipped".to_owned(),
            PairState::Compiled(c) => {
                let problem = c
                    .problem
                    .as_ref()
                    .map(|p| format!(" ({p})"))
                    .unwrap_or_default();
                format!("{:?}{problem}", c.status).to_lowercase()
            }
        };
        let error = pair_error.map(|e| format!(", {e}")).unwrap_or_default();
        (context.log)(&format!("{id}:{frontend} compile {status}{error}"));
        let state = Arc::new(state);
        let inputs: Vec<RowInput<'_>> = job
            .rows
            .iter()
            .map(|row| RowInput {
                row,
                presentation,
                state: &state,
                pair_error,
                oracle: oracle.clone(),
                content: options.content.clone(),
            })
            .collect();
        let scored: Vec<(RowId, Result<ScoredRow>)> = futures::stream::iter(inputs.iter())
            .map(|input| async move {
                let scored = std::panic::AssertUnwindSafe(score_row(input, context))
                    .catch_unwind()
                    .await
                    .unwrap_or_else(|_| {
                        (context.log)(&format!("{}: scoring panicked", input.row.id));
                        ScoredRow {
                            record: crashed_record(input),
                            render: None,
                            measure: None,
                            raw: None,
                        }
                    });
                let saved = save_row(options.run_dir, &input.row.id, &scored).map(|()| scored);
                (input.row.id.clone(), saved)
            })
            .buffer_unordered(options.concurrency.max(1))
            .collect()
            .await;
        for (row, scored) in scored {
            let scored = scored?;
            if let Some(raw) = scored.raw {
                executed.raw.insert(row.clone(), raw);
            }
            executed.records.insert(row, scored.record);
        }
    }
    Ok(executed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compile::ReportDiagnostic;

    fn diagnostic(code: &str, severity: &str, container: Option<&str>) -> ReportDiagnostic {
        ReportDiagnostic {
            code: code.into(),
            severity: severity.into(),
            container: container.map(str::to_owned),
            feature: None,
            count: None,
            message: None,
            node: None,
        }
    }

    #[test]
    fn undrawn_shadows_are_the_nodes_of_the_applying_shadow_codes() {
        let named = |code: &str, container: &str, node: Option<&str>| ReportDiagnostic {
            node: node.map(str::to_owned),
            ..diagnostic(code, "lossy", Some(container))
        };
        let report = CompileReport {
            diagnostics: vec![
                named("KN1304", "s1", Some("a")),
                named("PX2111", "l1", Some("b")),
                named("OD9104", "other", Some("c")),
                named("PX2101", "s1", Some("d")),
                named("KN1304", "s1", None),
            ],
            features: BTreeMap::new(),
            shows: Some(BTreeMap::from([("s1".to_owned(), vec!["l1".to_owned()])])),
        };
        assert_eq!(row_not_drawn_shadows(Some(&report), "s1"), ["a", "b"]);
    }

    #[test]
    fn approximated_gradients_are_the_nodes_of_the_applying_gradient_codes() {
        let named = |code: &str, container: Option<&str>, node: Option<&str>| ReportDiagnostic {
            node: node.map(str::to_owned),
            ..diagnostic(code, "lossy", container)
        };
        let report = CompileReport {
            diagnostics: vec![
                named("KN1301", Some("s1"), Some("a")),
                named("PX2106", Some("l1"), Some("b")),
                named("OD9103", Some("other"), Some("c")),
                named("KN1304", Some("s1"), Some("d")),
                // A background's: the slide's or a shown container's names `bg`; the presentation's names nothing.
                named("OD9103", Some("l1"), None),
                named("KN1301", None, None),
            ],
            features: BTreeMap::new(),
            shows: Some(BTreeMap::from([("s1".to_owned(), vec!["l1".to_owned()])])),
        };
        assert_eq!(
            row_approximated_gradients(Some(&report), "s1"),
            ["a", "b", "bg"]
        );
    }

    #[test]
    fn diagnostics_apply_through_the_shown_containers() {
        let report = CompileReport {
            diagnostics: vec![
                diagnostic("A1", "info", Some("s1")),
                diagnostic("B2", "decline", Some("l1")),
                diagnostic("C3", "info", None),
                diagnostic("D4", "decline", Some("other")),
                ReportDiagnostic {
                    count: Some(3),
                    ..diagnostic("A1", "info", Some("s1"))
                },
            ],
            features: BTreeMap::new(),
            shows: Some(BTreeMap::from([("s1".to_owned(), vec!["l1".to_owned()])])),
        };
        let applying = applying_diagnostics(Some(&report), "s1");
        let counts = diagnostic_counts(&applying);
        assert_eq!(
            counts,
            BTreeMap::from([("A1".into(), 4), ("B2".into(), 1), ("C3".into(), 1)])
        );
        let declines = declines_of(&applying);
        assert_eq!(declines.len(), 1);
        assert_eq!(declines[0].via.as_deref(), Some("l1"));
        let presentation = applying.iter().find(|d| d.code == "C3").unwrap();
        assert_eq!(presentation.via, Some("presentation"));
    }
}
