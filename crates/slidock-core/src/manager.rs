//! The pass manager: verify-source, the passes in order, then verify-resolved. Each pass declares the invariants it
//! requires and provides; construction checks that order, and under [`VerifyMode::Each`] every invariant provided so
//! far is checked after every pass, so a violation names the pass and the container that broke it. A pass error or a
//! violation fails only its container; only presentation-level problems fail the compile.
//!
//! The output budget ([`slidock_core_api::Limits::output`]) is charged as the passes expand the presentation: after
//! each pass, containers in the order the pass visits them (masters, layouts, then slides in show order), a
//! container whose output would pass what the presentation has left is left out (IR0914), failed as a pass failure
//! fails it, and the next container is charged. A slide pass charges each output as it makes it, so what it holds is
//! within the budget and one container.

use std::ops::Range;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use slidock_canon::{js_round, utf16};
use slidock_core_api::failure::{Causes, FailOptions, FailureCause, describe_error};
use slidock_core_api::{
    Clock, Deadline, Output, OwnedContainer, Pass, PassEnv, PassError, PassInfo, PresentationFacts,
    PresentationPass, Scope, Sink, SlidePass, Unverified, fail_containers, view_of,
    with_dependents,
};
use slidock_ir::diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticSite, Stage, codes, make_diagnostic,
};
use slidock_ir::ids::NodeId;
use slidock_ir::source::{Container, Presentation};
use slidock_ir::verify::{Invariant, InvariantError, VerifyOptions, verify_resolved};
use slidock_ir::{parse, resolved};
use thiserror::Error;

use crate::verify::{
    VERIFY_RESOLVED, VerifiedSource, invariant_of, invariant_violations, presentation_failed,
    verify_source_presentation,
};

/// When the invariants are checked: after every pass, or only at the two ends. `Boundaries` runs everything again
/// under `Each` when anything fails, so both modes return the same presentation and diagnostics.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum VerifyMode {
    Each,
    #[default]
    Boundaries,
}

/// A pass list the manager refuses.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum ConfigError {
    #[error("Pass {0} appears twice")]
    DuplicatePass(String),
    #[error("Pass {pass} requires {invariant}, which no earlier pass provides")]
    Unprovided { pass: String, invariant: Invariant },
    #[error("verify-resolved requires {}, which no pass provides", names(.0))]
    Unresolved(Vec<Invariant>),
}

fn names(invariants: &[Invariant]) -> String {
    invariants
        .iter()
        .map(|invariant| invariant.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

mod invariant_names {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use slidock_ir::verify::Invariant;

    pub fn serialize<S: Serializer>(
        invariants: &[Invariant],
        serializer: S,
    ) -> Result<S::Ok, S::Error> {
        let names: Vec<&str> = invariants
            .iter()
            .map(|invariant| invariant.as_str())
            .collect();
        names.serialize(serializer)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(
        deserializer: D,
    ) -> Result<Vec<Invariant>, D::Error> {
        Vec::<String>::deserialize(deserializer)?
            .iter()
            .map(|name| {
                Invariant::parse(name).ok_or_else(|| {
                    serde::de::Error::custom(format_args!("unknown invariant {name}"))
                })
            })
            .collect()
    }
}

/// What `replay` re-runs: one pass on the input it failed on.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FailureDump {
    pub pass: String,
    pub version: u32,
    pub scope: Scope,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub container: Option<NodeId>,
    /// The invariants provided through this pass, which replay checks.
    #[serde(with = "invariant_names")]
    pub invariants: Vec<Invariant>,
    /// The container's view for a slide-scope pass, the presentation for a presentation-scope one.
    pub view: Value,
    /// The presentation facts the pass's context held.
    pub facts: PresentationFacts,
}

/// How long a pass took, and per container for a slide-scope pass, in milliseconds on a 1 µs grid.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PassTiming {
    pub pass: String,
    pub ms: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub containers: Option<IndexMap<NodeId, f64>>,
}

/// The presentation a pass handed on: the next pass's input, failed containers marked.
#[derive(Clone, Debug, PartialEq)]
pub struct PassSnapshot {
    pub pass: String,
    pub presentation: Presentation,
}

/// The order containers are visited in; tests permute it to prove the output is independent of it.
pub type VisitOrder<'a> = &'a dyn Fn(&mut Vec<Container<'_>>);

#[derive(Clone, Copy)]
pub struct RunOptions<'a> {
    pub verify: VerifyMode,
    pub env: PassEnv<'a>,
    /// For timings, taken only when set.
    pub clock: Option<&'a dyn Clock>,
    /// The compile's; the passes stop (the presentation fails as a whole) once it passes, checked before each pass
    /// and each container.
    pub deadline: Deadline<'a>,
    pub visit_order: Option<VisitOrder<'a>>,
    /// Keeps the presentation after each pass in `trace`.
    pub trace: bool,
}

impl std::fmt::Debug for RunOptions<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RunOptions")
            .field("verify", &self.verify)
            .field("env", &self.env)
            .field("clock", &self.clock.is_some())
            .field("deadline", &self.deadline)
            .field("visit_order", &self.visit_order.is_some())
            .field("trace", &self.trace)
            .finish()
    }
}

impl<'a> RunOptions<'a> {
    pub fn new(verify: VerifyMode, env: PassEnv<'a>) -> Self {
        RunOptions {
            verify,
            env,
            clock: None,
            deadline: Deadline::never(),
            visit_order: None,
            trace: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunResult {
    pub presentation: Option<resolved::Presentation>,
    /// The verified source presentation the passes started from, failed containers included.
    pub source: Option<Presentation>,
    pub diagnostics: Vec<Diagnostic>,
    pub failures: Vec<FailureDump>,
    pub timings: Option<Vec<PassTiming>>,
    pub trace: Option<Vec<PassSnapshot>>,
}

/// Where a run of the passes stands: the presentation handed on (none once it failed as a whole), the invariants
/// provided so far, and what the passes reported.
#[derive(Clone, Debug)]
pub(crate) struct Progress {
    presentation: Option<Presentation>,
    provided: Vec<Invariant>,
    diagnostics: Vec<Diagnostic>,
    failures: Vec<FailureDump>,
    troubled: bool,
    timings: Option<Vec<PassTiming>>,
    trace: Option<Vec<PassSnapshot>>,
}

impl Progress {
    pub(crate) fn new(source: &Presentation, options: &RunOptions<'_>) -> Self {
        Progress {
            presentation: Some(source.clone()),
            provided: vec![Invariant::SourceValid],
            diagnostics: Vec::new(),
            failures: Vec::new(),
            troubled: false,
            timings: options.clock.map(|_| Vec::new()),
            trace: options.trace.then(Vec::new),
        }
    }

    /// The presentation handed on so far.
    pub(crate) fn presentation(&self) -> Option<&Presentation> {
        self.presentation.as_ref()
    }

    pub(crate) fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }
}

/// What one pass, or one check, produced.
struct Step {
    presentation: Option<Presentation>,
    diagnostics: Vec<Diagnostic>,
    failures: Vec<FailureDump>,
    troubled: bool,
}

/// A violation as the checks found it.
struct Found {
    invariant: Invariant,
    rule: DiagnosticCode,
    node: Option<NodeId>,
    detail: String,
}

impl From<InvariantError> for Found {
    fn from(error: InvariantError) -> Self {
        Found {
            detail: error.to_string(),
            invariant: error.invariant,
            rule: error.rule,
            node: error.node,
        }
    }
}

/// Milliseconds on a 1 µs grid, so timings print without e-notation.
fn round_ms(ms: f64) -> f64 {
    js_round(ms * 1000.0) / 1000.0
}

fn to_value<T: Serialize + ?Sized>(value: &T) -> Value {
    serde_json::to_value(value).unwrap_or(Value::Null)
}

/// The SHA-256 of what a failed pass saw, computed only on failure, so a report names the exact input to replay.
fn input_sha(info: &PassInfo, view: &Value) -> String {
    let keyed = serde_json::json!({ "pass": info.name, "version": info.version, "view": view });
    slidock_canon::sha256_canonical(&keyed).unwrap_or_default()
}

fn violation(info: &PassInfo, container: &NodeId, view: &Value, found: Found) -> Diagnostic {
    let site = DiagnosticSite::new()
        .container(container.clone())
        .node(found.node)
        .data("pass", info.name.as_ref())
        .data("invariant", found.invariant.as_str())
        .data("rule", found.rule.code())
        .data("detail", found.detail)
        .data("input", input_sha(info, view));
    make_diagnostic(codes::IR0909, Stage::Pass, &info.name, site)
}

/// `presentation` with each container a pass made in place of its input; the others (failed or left out) cloned as
/// they were.
fn replace_containers(
    presentation: &Presentation,
    mut outputs: IndexMap<NodeId, OwnedContainer>,
) -> Presentation {
    let Presentation {
        schema,
        size,
        defaults,
        themes,
        ext,
        phase,
        masters,
        layouts,
        slides,
        fonts,
        media,
    } = presentation;
    Presentation {
        schema: *schema,
        size: size.clone(),
        defaults: defaults.clone(),
        themes: themes.clone(),
        ext: ext.clone(),
        phase: *phase,
        masters: masters
            .iter()
            .map(|(key, master)| match outputs.shift_remove(key) {
                Some(OwnedContainer::Master(out)) => (key.clone(), out),
                _ => (key.clone(), master.clone()),
            })
            .collect(),
        layouts: layouts
            .iter()
            .map(|(key, layout)| match outputs.shift_remove(key) {
                Some(OwnedContainer::Layout(out)) => (key.clone(), out),
                _ => (key.clone(), layout.clone()),
            })
            .collect(),
        slides: slides
            .iter()
            .map(|slide| match outputs.shift_remove(&slide.id) {
                Some(OwnedContainer::Slide(out)) => out,
                _ => slide.clone(),
            })
            .collect(),
        fonts: fonts.clone(),
        media: media.clone(),
    }
}

/// The passes stopped at the compile's deadline: the presentation fails as a whole.
fn passes_stopped(deadline: &Deadline<'_>) -> Diagnostic {
    crate::compile::stage_stopped("passes", deadline, false)
}

/// A container left out of the output budget after `pass`.
fn over_budget(pass: &str, container: &NodeId, what: &str, limit: u64) -> FailureCause {
    let site = DiagnosticSite::new()
        .container(container.clone())
        .data("pass", pass)
        .data("what", what)
        .data("limit", limit);
    FailureCause::of_pass(
        make_diagnostic(codes::IR0914, Stage::Pass, pass, site),
        pass,
    )
}

/// Charges every standing container to the output budget in container order, and fails those left out with the
/// containers that show them; returns their diagnostics.
fn charge_output(presentation: &mut Presentation, budget: &Output, pass: &str) -> Vec<Diagnostic> {
    let mut spent = Output::default();
    let mut causes = Causes::new();
    for container in presentation.containers() {
        if container.failed().is_some() {
            continue;
        }
        let with = spent.plus(&Output::of(container));
        match with.over(budget) {
            None => spent = with,
            Some((what, limit)) => {
                causes.insert(
                    container.id().clone(),
                    over_budget(pass, container.id(), what, limit),
                );
            }
        }
    }
    if causes.is_empty() {
        return Vec::new();
    }
    let (all, dependents) = with_dependents(presentation, &causes);
    fail_containers(presentation, &all, &FailOptions::default());
    let mut out: Vec<Diagnostic> = causes.into_values().map(|cause| cause.diagnostic).collect();
    out.extend(dependents);
    out
}

fn threw(info: &PassInfo, container: &NodeId, view: &Value, error: &str) -> Diagnostic {
    let site = DiagnosticSite::new()
        .container(container.clone())
        .data("pass", info.name.as_ref())
        .data("error", error)
        .data("input", input_sha(info, view));
    make_diagnostic(codes::IR0908, Stage::Pass, &info.name, site)
}

fn dump(
    info: &PassInfo,
    scope: Scope,
    provided: &[Invariant],
    view: Value,
    facts: &PresentationFacts,
    container: Option<NodeId>,
) -> FailureDump {
    let mut invariants: Vec<Invariant> = Vec::new();
    for invariant in provided.iter().chain(info.provides.iter()) {
        if !invariants.contains(invariant) {
            invariants.push(*invariant);
        }
    }
    FailureDump {
        pass: info.name.to_string(),
        version: info.version,
        scope,
        container,
        invariants,
        view,
        facts: facts.clone(),
    }
}

fn same_shape(before: &Presentation, after: &Presentation) -> bool {
    before.masters.keys().eq(after.masters.keys())
        && before.layouts.keys().eq(after.layouts.keys())
        && before.slides.len() == after.slides.len()
        && before
            .slides
            .iter()
            .zip(&after.slides)
            .all(|(a, b)| a.id == b.id)
}

fn is_failed(presentation: &Presentation, id: &NodeId) -> bool {
    presentation
        .containers()
        .find(|c| c.id() == id)
        .is_some_and(|c| c.failed().is_some())
}

fn verify_options(facts: &PresentationFacts) -> VerifyOptions {
    VerifyOptions {
        linkable_slides: facts.linkable(),
    }
}

pub struct PassManager {
    passes: Vec<Pass>,
}

impl std::fmt::Debug for PassManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.passes.iter().map(Pass::name))
            .finish()
    }
}

impl PassManager {
    /// Checks the order: each pass's requirements are provided before it, names are unique, and together the
    /// passes provide every resolved invariant.
    pub fn new(passes: Vec<Pass>) -> Result<Self, ConfigError> {
        let mut provided = vec![Invariant::SourceValid];
        let mut seen: Vec<&str> = Vec::new();
        for pass in &passes {
            let info = pass.info();
            if seen.contains(&info.name.as_ref()) {
                return Err(ConfigError::DuplicatePass(info.name.to_string()));
            }
            seen.push(&info.name);
            if let Some(invariant) = info.requires.iter().find(|i| !provided.contains(i)) {
                return Err(ConfigError::Unprovided {
                    pass: info.name.to_string(),
                    invariant: *invariant,
                });
            }
            provided.extend(info.provides.iter().copied());
        }
        let missing: Vec<Invariant> = Invariant::RESOLVED
            .into_iter()
            .filter(|invariant| !provided.contains(invariant))
            .collect();
        if !missing.is_empty() {
            return Err(ConfigError::Unresolved(missing));
        }
        Ok(PassManager { passes })
    }

    pub fn passes(&self) -> &[Pass] {
        &self.passes
    }

    /// verify-source → the passes → verify-resolved.
    pub fn run(&self, input: &Unverified, options: &RunOptions<'_>) -> RunResult {
        self.run_verified(
            verify_source_presentation(input, Some(options.env.facts)),
            options,
        )
    }

    /// The passes → verify-resolved, over what verify-source made of the input.
    pub fn run_verified(&self, verified: VerifiedSource, options: &RunOptions<'_>) -> RunResult {
        let Some(source) = verified.presentation else {
            return RunResult {
                presentation: None,
                source: None,
                diagnostics: verified.diagnostics,
                failures: Vec::new(),
                timings: None,
                trace: None,
            };
        };
        let mut progress = Progress::new(&source, options);
        self.advance(&mut progress, 0..self.passes.len(), options.verify, options);
        let mut result = self.conclude(progress, &source, options);
        let mut diagnostics = verified.diagnostics;
        diagnostics.append(&mut result.diagnostics);
        result.diagnostics = diagnostics;
        result
    }

    /// The passes from `progress` on, verify-resolved, and under `Boundaries` the run again under `Each` from
    /// `source` when anything failed.
    pub(crate) fn conclude(
        &self,
        progress: Progress,
        source: &Presentation,
        options: &RunOptions<'_>,
    ) -> RunResult {
        let finished = self.verify_end(progress, options);
        let result = if options.verify == VerifyMode::Boundaries && finished.1 {
            let mut again = Progress::new(source, options);
            self.advance(&mut again, 0..self.passes.len(), VerifyMode::Each, options);
            self.verify_end(again, options).0
        } else {
            finished.0
        };
        RunResult {
            source: Some(source.clone()),
            ..result
        }
    }

    /// Runs the passes in `range` over `progress`, stopping when the presentation fails as a whole.
    pub(crate) fn advance(
        &self,
        progress: &mut Progress,
        range: Range<usize>,
        mode: VerifyMode,
        options: &RunOptions<'_>,
    ) {
        let each = mode == VerifyMode::Each;
        let passes = self.passes.get(range).unwrap_or(&[]);
        for pass in passes {
            if options.deadline.expired() {
                if progress.presentation.take().is_some() {
                    progress.diagnostics.push(passes_stopped(&options.deadline));
                }
                return;
            }
            let Some(presentation) = progress.presentation.take() else {
                return;
            };
            let info = pass.info();
            let started = options.clock.map(Clock::now_ms);
            let step = match pass {
                Pass::Presentation(pass) => self.presentation_pass(
                    pass.as_ref(),
                    &presentation,
                    &progress.provided,
                    options,
                ),
                Pass::Slide(pass) => self.slide_pass(
                    pass.as_ref(),
                    &presentation,
                    &progress.provided,
                    options,
                    &mut progress.timings,
                ),
            };
            progress.diagnostics.extend(step.diagnostics);
            progress.failures.extend(step.failures);
            progress.troubled |= step.troubled;
            let Some(mut next) = step.presentation else {
                return;
            };
            if let (Some(started), Some(clock), Pass::Presentation(_)) =
                (started, options.clock, pass)
                && let Some(timings) = progress.timings.as_mut()
            {
                timings.push(PassTiming {
                    pass: info.name.to_string(),
                    ms: round_ms(clock.now_ms() - started),
                    containers: None,
                });
            }
            progress.provided.extend(info.provides.iter().copied());
            if each {
                let checked =
                    self.check_after(pass, &presentation, next, &progress.provided, options);
                progress.troubled |= !checked.diagnostics.is_empty();
                progress.diagnostics.extend(checked.diagnostics);
                progress.failures.extend(checked.failures);
                let Some(checked) = checked.presentation else {
                    return;
                };
                next = checked;
            }
            if let Some(trace) = progress.trace.as_mut() {
                trace.push(PassSnapshot {
                    pass: info.name.to_string(),
                    presentation: next.clone(),
                });
            }
            progress.presentation = Some(next);
        }
    }

    fn presentation_pass(
        &self,
        pass: &dyn PresentationPass,
        presentation: &Presentation,
        provided: &[Invariant],
        options: &RunOptions<'_>,
    ) -> Step {
        let info = pass.info();
        let mut diagnostics = Vec::new();
        let result = {
            let mut sink = Sink::new(&mut diagnostics, Stage::Pass, &info.name);
            let mut ctx = options.env.ctx(&mut sink);
            pass.run(presentation, &mut ctx)
        };
        let out = match result {
            Ok(out) => out,
            Err(error) => {
                let reason = match &error {
                    PassError::Invariant(error) => {
                        format!("{} ({}): {error}", error.invariant, error.rule.code())
                    }
                    PassError::Failed(message) => describe_error(message),
                };
                let view = to_value(presentation);
                let failure = dump(
                    info,
                    Scope::Presentation,
                    provided,
                    view,
                    options.env.facts,
                    None,
                );
                return Step {
                    presentation: None,
                    diagnostics: vec![presentation_failed(Stage::Pass, &info.name, &reason)],
                    failures: vec![failure],
                    troubled: true,
                };
            }
        };
        if !same_shape(presentation, &out) {
            let reason = "a presentation-scope pass added, removed or reordered containers";
            return Step {
                presentation: None,
                diagnostics: vec![presentation_failed(Stage::Pass, &info.name, reason)],
                failures: Vec::new(),
                troubled: true,
            };
        }
        // Failed containers are out of every pass's reach.
        let mut restored = out;
        for (master, before) in restored
            .masters
            .values_mut()
            .zip(presentation.masters.values())
        {
            if before.failed.is_some() {
                master.clone_from(before);
            }
        }
        for (layout, before) in restored
            .layouts
            .values_mut()
            .zip(presentation.layouts.values())
        {
            if before.failed.is_some() {
                layout.clone_from(before);
            }
        }
        for (slide, before) in restored.slides.iter_mut().zip(&presentation.slides) {
            if before.failed.is_some() {
                slide.clone_from(before);
            }
        }
        diagnostics.extend(charge_output(
            &mut restored,
            &options.env.limits.output,
            &info.name,
        ));
        Step {
            presentation: Some(restored),
            diagnostics,
            failures: Vec::new(),
            troubled: false,
        }
    }

    fn slide_pass(
        &self,
        pass: &dyn SlidePass,
        presentation: &Presentation,
        provided: &[Invariant],
        options: &RunOptions<'_>,
        timings: &mut Option<Vec<PassTiming>>,
    ) -> Step {
        let info = pass.info();
        let facts = options.env.facts;
        let mut diagnostics = Vec::new();
        let mut failures = Vec::new();
        let mut causes = Causes::new();
        let mut declined = Causes::new();
        let budget = &options.env.limits.output;
        let mut spent = Output::default();
        let mut outputs: IndexMap<NodeId, OwnedContainer> = IndexMap::new();
        let mut container_ms: Vec<(NodeId, f64)> = Vec::new();
        let mut total = 0.0;
        let mut refs: Vec<Container<'_>> = presentation.containers().collect();
        if let Some(order) = options.visit_order {
            order(&mut refs);
        }
        for container in refs {
            if options.deadline.expired() {
                return Step {
                    presentation: None,
                    diagnostics: vec![passes_stopped(&options.deadline)],
                    failures,
                    troubled: false,
                };
            }
            if container.failed().is_some() {
                continue;
            }
            let id = container.id();
            let view = view_of(presentation, container, facts);
            let started = options.clock.map(Clock::now_ms);
            let mut own = Vec::new();
            let result = {
                let mut sink = Sink::new(&mut own, Stage::Pass, &info.name);
                let mut ctx = options.env.ctx(&mut sink);
                pass.run(&view, &mut ctx)
            };
            let failure = match result {
                Ok(out) if out.id() == id && out.kind() == container.kind() => {
                    let with = spent.plus(&Output::of(out.as_ref()));
                    match with.over(budget) {
                        None => {
                            spent = with;
                            outputs.insert(id.clone(), out);
                        }
                        Some((what, limit)) => {
                            declined.insert(id.clone(), over_budget(&info.name, id, what, limit));
                        }
                    }
                    None
                }
                Ok(_) => Some(threw(
                    info,
                    id,
                    &to_value(&view),
                    "returned a different container",
                )),
                Err(PassError::Invariant(error)) => {
                    Some(violation(info, id, &to_value(&view), error.into()))
                }
                Err(PassError::Failed(message)) => {
                    Some(threw(info, id, &to_value(&view), &describe_error(&message)))
                }
            };
            if let (Some(started), Some(clock)) = (started, options.clock) {
                let ms = clock.now_ms() - started;
                container_ms.push((id.clone(), round_ms(ms)));
                total += ms;
            }
            match failure {
                Some(diagnostic) => {
                    causes.insert(
                        id.clone(),
                        FailureCause::of_pass(diagnostic, info.name.as_ref()),
                    );
                    failures.push(dump(
                        info,
                        Scope::Slide,
                        provided,
                        to_value(&view),
                        facts,
                        Some(id.clone()),
                    ));
                }
                None if declined.contains_key(id) => {}
                None => diagnostics.append(&mut own),
            }
        }
        if let Some(timings) = timings.as_mut() {
            container_ms.sort_by(|(a, _), (b, _)| utf16::cmp(a.as_str(), b.as_str()));
            timings.push(PassTiming {
                pass: info.name.to_string(),
                ms: round_ms(total),
                containers: Some(container_ms.into_iter().collect()),
            });
        }

        let mut replaced = replace_containers(presentation, outputs);
        // A failed container has no output, so it fails from its input form.
        let troubled = !causes.is_empty();
        causes.extend(declined);
        let (all, dependents) = with_dependents(&replaced, &causes);
        fail_containers(&mut replaced, &all, &FailOptions::default());
        diagnostics.extend(causes.into_values().map(|cause| cause.diagnostic));
        diagnostics.extend(dependents);
        Step {
            presentation: Some(replaced),
            diagnostics,
            failures,
            troubled,
        }
    }

    /// Under `Each`: the schema and every invariant provided so far, over the containers still standing.
    fn check_after(
        &self,
        pass: &Pass,
        before: &Presentation,
        after: Presentation,
        provided: &[Invariant],
        options: &RunOptions<'_>,
    ) -> Step {
        let info = pass.info();
        let facts = options.env.facts;
        let emitter = format!("verify:{}", info.name);
        let linkable = facts.linkable();
        let mut diagnostics = Vec::new();
        let mut failures = Vec::new();
        let mut presentation = after;
        loop {
            let violations = invariant_violations(&presentation, provided, &emitter, &linkable);
            if violations.is_empty() {
                return Step {
                    presentation: Some(presentation),
                    diagnostics,
                    failures,
                    troubled: false,
                };
            }
            if let Some((invariant, diagnostic)) =
                violations.iter().find(|(_, d)| d.container.is_none())
            {
                let reason = format!(
                    "{invariant} ({}): {}",
                    diagnostic.code.code(),
                    diagnostic.message
                );
                diagnostics.push(presentation_failed(Stage::Pass, &info.name, &reason));
                return Step {
                    presentation: None,
                    diagnostics,
                    failures,
                    troubled: true,
                };
            }
            let mut causes = Causes::new();
            for (invariant, diagnostic) in violations {
                let Some(id) = diagnostic.container.clone() else {
                    continue;
                };
                if causes.contains_key(&id) {
                    continue;
                }
                if is_failed(&presentation, &id) {
                    // Failing it again would change nothing: the check would loop.
                    let reason = format!("{id} cannot be represented even as a failed container");
                    diagnostics.push(presentation_failed(Stage::Pass, &info.name, &reason));
                    return Step {
                        presentation: None,
                        diagnostics,
                        failures,
                        troubled: true,
                    };
                }
                let reference = before.containers().find(|c| c.id() == &id);
                let view = match (pass.scope(), reference) {
                    (Scope::Slide, Some(container)) => to_value(&view_of(before, container, facts)),
                    _ => to_value(before),
                };
                let found = Found {
                    invariant,
                    rule: diagnostic.code,
                    node: diagnostic.node.clone(),
                    detail: diagnostic.message.clone(),
                };
                let failure = violation(info, &id, &view, found);
                causes.insert(
                    id.clone(),
                    FailureCause::of_pass(failure, info.name.as_ref()),
                );
                failures.push(dump(info, pass.scope(), provided, view, facts, Some(id)));
            }
            let (all, dependents) = with_dependents(&presentation, &causes);
            diagnostics.extend(causes.into_values().map(|cause| cause.diagnostic));
            diagnostics.extend(dependents);
            fail_containers(&mut presentation, &all, &FailOptions::default());
        }
    }

    /// verify-resolved with isolation; violations here slipped past every pass's checks. Returns the result and
    /// whether anything failed along the way.
    fn verify_end(&self, progress: Progress, options: &RunOptions<'_>) -> (RunResult, bool) {
        let Progress {
            presentation,
            diagnostics: mut collected,
            failures,
            mut troubled,
            timings,
            trace,
            ..
        } = progress;
        let mut resolved = None;
        if let Some(mut current) = presentation {
            let verify = verify_options(options.env.facts);
            loop {
                let errors = match verify_resolved(&current, &verify) {
                    Ok(done) => {
                        resolved = Some(done);
                        break;
                    }
                    Err(errors) => errors,
                };
                troubled = true;
                if let Some(level) = errors.iter().find(|d| d.container.is_none()) {
                    collected.push(presentation_failed(
                        Stage::Verify,
                        VERIFY_RESOLVED,
                        &level.message,
                    ));
                    break;
                }
                let mut causes = Causes::new();
                let mut unrepresentable = None;
                for error in &errors {
                    let Some(id) = &error.container else { continue };
                    if causes.contains_key(id) {
                        continue;
                    }
                    if is_failed(&current, id) {
                        unrepresentable = Some(id.clone());
                        break;
                    }
                    let site = DiagnosticSite::new()
                        .container(id.clone())
                        .node(error.node.clone())
                        .data("pass", VERIFY_RESOLVED)
                        .data(
                            "invariant",
                            invariant_of(error, &Invariant::RESOLVED).as_str(),
                        )
                        .data("rule", error.code.code())
                        .data("detail", error.message.as_str());
                    let diagnostic =
                        make_diagnostic(codes::IR0909, Stage::Verify, VERIFY_RESOLVED, site);
                    causes.insert(
                        id.clone(),
                        FailureCause::of_pass(diagnostic, VERIFY_RESOLVED),
                    );
                }
                if let Some(id) = unrepresentable {
                    let reason = format!("{id} cannot be represented even as a failed container");
                    collected.push(presentation_failed(Stage::Verify, VERIFY_RESOLVED, &reason));
                    break;
                }
                let (all, dependents) = with_dependents(&current, &causes);
                collected.extend(causes.into_values().map(|cause| cause.diagnostic));
                collected.extend(dependents);
                fail_containers(&mut current, &all, &FailOptions::default());
            }
        }
        let result = RunResult {
            presentation: resolved,
            source: None,
            diagnostics: collected,
            failures,
            timings,
            trace,
        };
        (result, troubled)
    }
}

/// Why a dump cannot be replayed.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum ReplayError {
    #[error("The dump is for {} pass {}, not {} pass {}", .dump.0.as_str(), .dump.1, .pass.0.as_str(), .pass.1)]
    WrongPass {
        dump: (Scope, String),
        pass: (Scope, String),
    },
    #[error("The dump does not read as a pass input: {0}")]
    Unreadable(String),
}

/// A presentation holding just a view's chain and its container, so a replayed container can be verified alone.
fn presentation_of_view(view: &Value) -> Result<(Presentation, NodeId), ReplayError> {
    let unreadable = |what: &str| ReplayError::Unreadable(what.to_owned());
    let field = |key: &str| view.get(key).cloned();
    let container = field("container").ok_or_else(|| unreadable("no container"))?;
    let id = container
        .get("id")
        .and_then(Value::as_str)
        .and_then(|id| NodeId::new(id).ok())
        .ok_or_else(|| unreadable("no container id"))?;
    let keyed = |value: Option<Value>| {
        let mut record = serde_json::Map::new();
        if let Some(value) = value
            && let Some(id) = value.get("id").and_then(Value::as_str)
        {
            record.insert(id.to_owned(), value);
        }
        record
    };
    let mut masters = keyed(field("master"));
    let mut layouts = keyed(field("layout"));
    let mut slides = Vec::new();
    match view.get("kind").and_then(Value::as_str) {
        Some("master") => {
            masters.insert(id.to_string(), container);
        }
        Some("layout") => {
            layouts.insert(id.to_string(), container);
        }
        Some("slide") => slides.push(container),
        _ => return Err(unreadable("an unknown container kind")),
    }
    let presentation = serde_json::json!({
        "phase": "source",
        "size": field("size"),
        "defaults": field("defaults"),
        "themes": keyed(field("theme")),
        "masters": masters,
        "layouts": layouts,
        "slides": slides,
        "fonts": field("fonts"),
        "media": field("media"),
    });
    let presentation = parse::from_value(presentation)
        .map_err(|error| ReplayError::Unreadable(error.to_string()))?;
    Ok((presentation, id))
}

/// Replaces the container `id` of `presentation` by `out`.
fn replace(presentation: &mut Presentation, out: OwnedContainer) {
    match out {
        OwnedContainer::Master(out) => {
            if let Some(slot) = presentation.masters.get_mut(&out.id) {
                *slot = out;
            }
        }
        OwnedContainer::Layout(out) => {
            if let Some(slot) = presentation.layouts.get_mut(&out.id) {
                *slot = out;
            }
        }
        OwnedContainer::Slide(out) => {
            if let Some(slot) = presentation.slides.iter_mut().find(|s| s.id == out.id) {
                *slot = out;
            }
        }
    }
}

/// Re-runs a dumped failure with the invariant checks on: the diagnostics it still produces, empty once fixed.
pub fn replay_failure(
    dump: &FailureDump,
    pass: &Pass,
    env: &PassEnv<'_>,
) -> Result<Vec<Diagnostic>, ReplayError> {
    let info = pass.info();
    if dump.pass != info.name || dump.scope != pass.scope() {
        return Err(ReplayError::WrongPass {
            dump: (dump.scope, dump.pass.clone()),
            pass: (pass.scope(), info.name.to_string()),
        });
    }
    let facts = &dump.facts;
    let mut diagnostics = Vec::new();
    let (input, id) = match pass {
        Pass::Presentation(_) => {
            let presentation: Presentation = parse::from_value(dump.view.clone())
                .map_err(|error| ReplayError::Unreadable(error.to_string()))?;
            (presentation, None)
        }
        Pass::Slide(_) => {
            let (presentation, id) = presentation_of_view(&dump.view)?;
            (presentation, Some(id))
        }
    };
    let ran = {
        let mut sink = Sink::new(&mut diagnostics, Stage::Pass, &info.name);
        let mut ctx = env.ctx(&mut sink);
        match pass {
            Pass::Presentation(pass) => pass.run(&input, &mut ctx),
            Pass::Slide(pass) => {
                let container = input
                    .containers()
                    .find(|c| Some(c.id()) == id.as_ref())
                    .ok_or_else(|| ReplayError::Unreadable("no container".to_owned()))?;
                let view = view_of(&input, container, facts);
                pass.run(&view, &mut ctx).map(|out| {
                    let mut presentation = input.clone();
                    replace(&mut presentation, out);
                    presentation
                })
            }
        }
    };
    let presentation = match ran {
        Ok(presentation) => presentation,
        Err(error) => {
            let message = match &error {
                PassError::Invariant(error) => error.to_string(),
                PassError::Failed(message) => message.clone(),
            };
            let site = DiagnosticSite::new()
                .container(dump.container.clone())
                .data("pass", info.name.as_ref())
                .data("error", describe_error(&message));
            diagnostics.push(make_diagnostic(
                codes::IR0908,
                Stage::Pass,
                &info.name,
                site,
            ));
            return Ok(diagnostics);
        }
    };
    let emitter = format!("verify:{}", info.name);
    let violations =
        invariant_violations(&presentation, &dump.invariants, &emitter, &facts.linkable());
    diagnostics.extend(violations.into_iter().map(|(invariant, diagnostic)| {
        let site = DiagnosticSite::new()
            .container(diagnostic.container.clone())
            .node(diagnostic.node.clone())
            .data("pass", info.name.as_ref())
            .data("invariant", invariant.as_str())
            .data("rule", diagnostic.code.code())
            .data("detail", diagnostic.message);
        make_diagnostic(codes::IR0909, Stage::Pass, &info.name, site)
    }));
    Ok(diagnostics)
}
