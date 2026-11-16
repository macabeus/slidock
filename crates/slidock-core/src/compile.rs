//! `compile`: one input through its frontend and the passes. Every way an input can fail ends as diagnostics in the
//! result: a refusal (unreadable, over a limit), a frontend crash, a timeout, or a presentation-level failure.

use std::fmt;

use indexmap::IndexMap;
use slidock_core_api::{
    Analysis, Clock, ContainerOrder, Deadline, EmbeddedFont, Frontend, FrontendCtx, FrontendError,
    Input, Limits, LowerOptions, LowerResult, MediaPolicy, MediaStore, PassEnv, PresentationFacts,
    Provenance, Sink, SniffHints, sort_diagnostics,
};
use slidock_fonts::{FontCatalog, builtin_catalog};
use slidock_io::ZipArchive;
use slidock_io::zip::is_zip_head;
use slidock_ir::diagnostics::{
    DataValue, Diagnostic, DiagnosticCode, DiagnosticSite, Stage, codes, make_diagnostic,
};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::SourceFormat;
use slidock_ir::ids::SlideId;
use slidock_ir::{resolved, source};

use crate::foreign::foreign_diagnostics;
use crate::manager::{FailureDump, PassManager, PassSnapshot, PassTiming, RunOptions, VerifyMode};
use crate::registry::FrontendRegistry;
use crate::verify::verify_source_motion;

/// The first bytes every frontend sniffs.
pub const SNIFF_BYTES: usize = 4096;

/// Which frontend reads the input.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum FrontendChoice {
    /// The first registered frontend that sniffs the input.
    #[default]
    Auto,
    Format(SourceFormat),
}

/// How far a compile goes.
#[derive(Clone, Copy, Debug)]
pub enum Phase<'a> {
    /// Stop after lowering and verify-source.
    Source,
    /// Run these passes and verify-resolved.
    Resolved(&'a PassManager),
}

/// What reading an input takes: where its frontends, limits and clock come from.
#[derive(Clone, Copy)]
pub struct InputOptions<'a> {
    pub registry: &'a FrontendRegistry,
    pub frontend: FrontendChoice,
    pub limits: &'a Limits,
    /// For stage deadlines and pass timings; nothing times out without one.
    pub clock: Option<&'a dyn Clock>,
    /// The whole compile's ([`Limits::compile_ms`] from when it started), which every stage keeps within.
    pub deadline: Deadline<'a>,
}

impl fmt::Debug for InputOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("InputOptions")
            .field("registry", &self.registry)
            .field("frontend", &self.frontend)
            .field("clock", &self.clock.is_some())
            .finish_non_exhaustive()
    }
}

#[derive(Clone, Copy)]
pub struct CompileOptions<'a> {
    pub registry: &'a FrontendRegistry,
    pub frontend: FrontendChoice,
    /// Lower and compile only these slides (with their layout, master and theme chain).
    pub only: Option<&'a [SlideId]>,
    pub phase: Phase<'a>,
    pub verify: VerifyMode,
    pub limits: Limits,
    /// The fonts bind-fonts binds against.
    pub fonts: &'a FontCatalog,
    pub policy: MediaPolicy,
    /// For stage deadlines and pass timings in `stats`; timings stay out of the outputs.
    pub clock: Option<&'a dyn Clock>,
    /// Keeps the presentation after each pass in `trace`.
    pub trace: bool,
    /// Keep the native motion data the frontend lowers (the effect vocabulary, timing and match keys). Off, the
    /// parity path, it is left out once the presentation verifies ([`slidock_ir::effects::strip_native_motion`]).
    pub native_motion: bool,
}

impl fmt::Debug for CompileOptions<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompileOptions")
            .field("registry", &self.registry)
            .field("frontend", &self.frontend)
            .field("only", &self.only)
            .field("phase", &self.phase)
            .field("verify", &self.verify)
            .field("clock", &self.clock.is_some())
            .field("trace", &self.trace)
            .field("native_motion", &self.native_motion)
            .finish_non_exhaustive()
    }
}

impl<'a> CompileOptions<'a> {
    /// Auto-detected frontend, the whole presentation, default limits and policy, the built-in fonts, verified at
    /// the boundaries, native motion left out.
    pub fn new(registry: &'a FrontendRegistry, phase: Phase<'a>) -> Self {
        CompileOptions {
            registry,
            frontend: FrontendChoice::Auto,
            only: None,
            phase,
            verify: VerifyMode::Boundaries,
            limits: Limits::default(),
            fonts: builtin_catalog(),
            policy: MediaPolicy::default(),
            clock: None,
            trace: false,
            native_motion: false,
        }
    }

    /// The input options of a compile that starts now.
    pub fn input(&self) -> InputOptions<'_> {
        InputOptions {
            registry: self.registry,
            frontend: self.frontend,
            limits: &self.limits,
            clock: self.clock,
            deadline: Deadline::start(self.clock, self.limits.compile_ms),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stats {
    pub passes: Vec<PassTiming>,
}

pub struct CompileResult {
    pub provenance: Option<Provenance>,
    /// The verified source presentation: the frontend's output, failed containers marked.
    pub source: Option<source::Presentation>,
    pub presentation: Option<resolved::Presentation>,
    pub media: Option<Box<dyn MediaStore>>,
    /// The fonts the source embeds.
    pub fonts: Vec<EmbeddedFont>,
    /// What the frontend detected, per container and under `presentation`.
    pub features: IndexMap<String, Vec<FeatureId>>,
    /// In output order.
    pub diagnostics: Vec<Diagnostic>,
    /// What failed passes saw, for replay.
    pub failures: Vec<FailureDump>,
    /// Set when the input was refused as a whole.
    pub refusal: Option<Diagnostic>,
    pub stats: Option<Stats>,
    /// The presentation after each pass, in pass order; set only with `trace`.
    pub trace: Option<Vec<PassSnapshot>>,
}

impl fmt::Debug for CompileResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CompileResult")
            .field("provenance", &self.provenance)
            .field("source", &self.source.is_some())
            .field("presentation", &self.presentation.is_some())
            .field("diagnostics", &self.diagnostics)
            .field("refusal", &self.refusal)
            .finish_non_exhaustive()
    }
}

impl CompileResult {
    fn empty() -> Self {
        CompileResult {
            provenance: None,
            source: None,
            presentation: None,
            media: None,
            fonts: Vec::new(),
            features: IndexMap::new(),
            diagnostics: Vec::new(),
            failures: Vec::new(),
            refusal: None,
            stats: None,
            trace: None,
        }
    }
}

/// A diagnostic of the pipeline itself.
pub(crate) fn core(code: DiagnosticCode, data: &[(&'static str, DataValue)]) -> Diagnostic {
    let mut site = DiagnosticSite::new();
    for (key, value) in data {
        site = site.data(*key, value.clone());
    }
    make_diagnostic(code, Stage::Driver, "pipeline", site)
}

/// A frontend stage.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StageName {
    Analyze,
    Lower,
}

impl StageName {
    fn as_str(self) -> &'static str {
        match self {
            StageName::Analyze => "analyze",
            StageName::Lower => "lower",
        }
    }
}

/// Why reading an input stopped: a refusal of the whole input, or a stage that crashed or ran out of time.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Stop {
    pub diagnostic: Box<Diagnostic>,
    pub refused: bool,
}

impl Stop {
    fn refused(diagnostic: Diagnostic) -> Self {
        Stop {
            diagnostic: Box::new(diagnostic),
            refused: true,
        }
    }

    fn failed(diagnostic: Diagnostic) -> Self {
        Stop {
            diagnostic: Box::new(diagnostic),
            refused: false,
        }
    }
}

/// A stage that stopped at `deadline`: its own limit, or the compile's (IR0905).
pub fn stage_stopped(stage: &str, deadline: &Deadline<'_>, own: bool) -> Diagnostic {
    let limit = deadline.limit_ms();
    let reason = if own {
        format!("over its {limit} ms limit")
    } else {
        format!("over the compile's {limit} ms limit")
    };
    core(
        codes::IR0905,
        &[("stage", stage.into()), ("reason", reason.into())],
    )
}

/// Runs a frontend stage under its time limit and within the compile's; a refusal, crash or timeout becomes a
/// [`Stop`]. Limits are cooperative: a stage that finished late still failed its limit.
fn run_stage<'c, T>(
    stage: StageName,
    format: SourceFormat,
    limit_ms: u64,
    clock: Option<&'c dyn Clock>,
    compile: Deadline<'c>,
    run: impl FnOnce(Deadline<'c>) -> Result<T, FrontendError>,
) -> Result<T, Stop> {
    let own = Deadline::start(clock, limit_ms);
    let is_own = !compile.sooner_than(&own);
    let deadline = if is_own { own } else { compile };
    let stopped = || Stop::failed(stage_stopped(stage.as_str(), &deadline, is_own));
    match run(deadline) {
        Err(FrontendError::Refused(diagnostic)) => Err(Stop {
            diagnostic,
            refused: true,
        }),
        Err(_) if deadline.expired() => Err(stopped()),
        Err(FrontendError::Stopped) => Err(stopped()),
        Err(FrontendError::Crashed(error)) => Err(Stop::failed(core(
            codes::IR0906,
            &[
                ("format", format.as_str().into()),
                ("stage", stage.as_str().into()),
                ("error", error.into()),
            ],
        ))),
        Ok(_) if deadline.expired() => Err(stopped()),
        Ok(value) => Ok(value),
    }
}

/// The forced frontend, or the first that sniffs the input's head (and, for a zip, its entry names). A head that
/// cannot be read sniffs as empty; a broken zip sniffs on its head alone, and its frontend reports the damage.
fn choose_frontend<'r>(input: &Input<'_>, options: &InputOptions<'r>) -> Option<&'r dyn Frontend> {
    if let FrontendChoice::Format(format) = options.frontend {
        return options.registry.get(format);
    }
    let head = slidock_io::read_head(input.reader, SNIFF_BYTES)
        .map(|head| head.into_owned())
        .unwrap_or_default();
    let zip_names: Option<Vec<String>> = if is_zip_head(&head) {
        ZipArchive::open(input.reader, options.limits.zip)
            .ok()
            .map(|zip| {
                zip.entries()
                    .iter()
                    .map(|entry| entry.name.clone())
                    .collect()
            })
    } else {
        None
    };
    let hints = SniffHints {
        name: input.name,
        zip_names: zip_names.as_deref(),
    };
    options.registry.sniff(&head, &hints)
}

/// An input whose frontend read its metadata.
pub(crate) struct Opened<'r> {
    pub frontend: &'r dyn Frontend,
    pub analysis: Analysis,
}

/// The steps every use of an input shares: size limit, frontend choice and analyze, with the slide limit.
pub(crate) fn open<'r>(
    input: &Input<'_>,
    options: &InputOptions<'r>,
    collected: &mut Vec<Diagnostic>,
) -> Result<Opened<'r>, Stop> {
    let limits = options.limits;
    if input.size() > limits.input_bytes {
        return Err(Stop::refused(core(
            codes::IR0903,
            &[
                ("bytes", input.size().into()),
                ("limit", limits.input_bytes.into()),
            ],
        )));
    }
    let frontend = choose_frontend(input, options)
        .ok_or_else(|| Stop::refused(core(codes::IR0902, &[("name", input.name.into())])))?;
    let format = frontend.format();
    let analysis = run_stage(
        StageName::Analyze,
        format,
        limits.analyze_ms,
        options.clock,
        options.deadline,
        |deadline| {
            let mut sink = Sink::new(collected, Stage::Frontend, format.as_str());
            let mut ctx = FrontendCtx {
                limits,
                diag: &mut sink,
                deadline,
                spent: slidock_core_api::Output::default(),
            };
            frontend.analyze(input, &mut ctx)
        },
    )?;
    if let Some(refusal) = analysis.refusal.clone() {
        return Err(Stop::refused(refusal));
    }
    if analysis.slides.len() > limits.slides {
        return Err(Stop::refused(core(
            codes::IR0904,
            &[
                ("count", analysis.slides.len().into()),
                ("limit", limits.slides.into()),
            ],
        )));
    }
    Ok(Opened { frontend, analysis })
}

/// The `only` checks: every id is a slide of the show, and the media fit the budget (which media a presentation over
/// budget omits depends on every slide, so a partial compile would guess).
pub(crate) fn check_only(
    only: Option<&[SlideId]>,
    facts: &PresentationFacts,
    policy: &MediaPolicy,
) -> Result<(), Stop> {
    let Some(only) = only else { return Ok(()) };
    if let Some(unknown) = only
        .iter()
        .find(|id| !facts.slides.iter().any(|slide| &slide.id == *id))
    {
        return Err(Stop::refused(core(
            codes::IR0913,
            &[("id", unknown.as_str().into())],
        )));
    }
    if facts.media_bytes > policy.presentation_bytes {
        return Err(Stop::refused(core(
            codes::IR0912,
            &[
                ("mediaBytes", facts.media_bytes.into()),
                ("budget", policy.presentation_bytes.into()),
            ],
        )));
    }
    Ok(())
}

/// The lower stage.
pub(crate) fn lower(
    input: &Input<'_>,
    opened: &Opened<'_>,
    options: &InputOptions<'_>,
    request: &LowerOptions<'_>,
    collected: &mut Vec<Diagnostic>,
) -> Result<LowerResult, Stop> {
    let format = opened.frontend.format();
    let limits = options.limits;
    run_stage(
        StageName::Lower,
        format,
        limits.lower_ms,
        options.clock,
        options.deadline,
        |deadline| {
            let mut sink = Sink::new(collected, Stage::Frontend, format.as_str());
            let mut ctx = FrontendCtx {
                limits,
                diag: &mut sink,
                deadline,
                spent: slidock_core_api::Output::default(),
            };
            opened.frontend.lower(input, &mut ctx, request)
        },
    )
}

#[derive(Clone, Debug, PartialEq)]
pub struct AnalyzeResult {
    pub analysis: Option<Analysis>,
    pub diagnostics: Vec<Diagnostic>,
    pub refusal: Option<Diagnostic>,
}

/// A frontend's analysis of an input: metadata only, as a review step shows it before anything is lowered.
pub fn analyze(input: &Input<'_>, options: &InputOptions<'_>) -> AnalyzeResult {
    let mut collected = Vec::new();
    let opened = open(input, options, &mut collected);
    let order = ContainerOrder::default();
    match opened {
        Ok(opened) => AnalyzeResult {
            analysis: Some(opened.analysis),
            diagnostics: sort_diagnostics(collected, &order),
            refusal: None,
        },
        Err(stop) => {
            collected.push((*stop.diagnostic).clone());
            AnalyzeResult {
                analysis: None,
                diagnostics: sort_diagnostics(collected, &order),
                refusal: stop.refused.then_some(*stop.diagnostic),
            }
        }
    }
}

/// One input through its frontend and, in the resolved phase, the passes.
pub fn compile(input: &Input<'_>, options: &CompileOptions<'_>) -> CompileResult {
    let mut collected = Vec::new();
    let stopped = |mut collected: Vec<Diagnostic>, stop: Stop| {
        collected.push((*stop.diagnostic).clone());
        CompileResult {
            diagnostics: sort_diagnostics(collected, &ContainerOrder::default()),
            refusal: stop.refused.then_some(*stop.diagnostic),
            ..CompileResult::empty()
        }
    };
    let input_options = options.input();
    let opened = match open(input, &input_options, &mut collected) {
        Ok(opened) => opened,
        Err(stop) => return stopped(collected, stop),
    };
    let facts = opened.analysis.facts();
    if let Err(stop) = check_only(options.only, &facts, &options.policy) {
        return stopped(collected, stop);
    }
    let lower_options = LowerOptions {
        facts: &facts,
        only: options.only,
        native_motion: options.native_motion,
    };
    let lowered = match lower(
        input,
        &opened,
        &input_options,
        &lower_options,
        &mut collected,
    ) {
        Ok(lowered) => lowered,
        Err(stop) => return stopped(collected, stop),
    };
    let LowerResult {
        provenance,
        presentation: unverified,
        media,
        fonts,
        features,
    } = lowered;
    let mut result = CompileResult {
        provenance: Some(provenance),
        media: Some(media),
        fonts,
        features,
        ..CompileResult::empty()
    };

    let manager = match options.phase {
        Phase::Source => {
            // A frontend's presentation is walked only once it is verified: the IR frontend's is the input's JSON
            // as it came.
            let verified = verify_source_motion(&unverified, Some(&facts), options.native_motion);
            if let Some(source) = &verified.presentation {
                collected.extend(foreign_diagnostics(source));
            }
            collected.extend(verified.diagnostics);
            let order = ContainerOrder::of(verified.presentation.as_ref());
            result.diagnostics = sort_diagnostics(collected, &order);
            result.source = verified.presentation;
            return result;
        }
        Phase::Resolved(manager) => manager,
    };
    let env = PassEnv {
        limits: &options.limits,
        fonts: options.fonts,
        policy: &options.policy,
        facts: &facts,
        host_media: &[],
    };
    let run = RunOptions {
        verify: options.verify,
        env,
        clock: options.clock,
        deadline: input_options.deadline,
        visit_order: None,
        trace: options.trace,
    };
    let verified = verify_source_motion(&unverified, Some(&facts), options.native_motion);
    let ran = manager.run_verified(verified, &run);
    if let Some(source) = &ran.source {
        collected.extend(foreign_diagnostics(source));
    }
    collected.extend(ran.diagnostics);
    let order = match (&ran.presentation, &ran.source) {
        (Some(presentation), _) => ContainerOrder::of_resolved(presentation),
        (None, source) => ContainerOrder::of(source.as_ref()),
    };
    result.diagnostics = sort_diagnostics(collected, &order);
    result.source = ran.source;
    result.presentation = ran.presentation;
    result.failures = ran.failures;
    result.stats = ran.timings.map(|passes| Stats { passes });
    result.trace = ran.trace;
    result
}
