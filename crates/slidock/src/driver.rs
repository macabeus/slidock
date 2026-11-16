//! The one module that joins the compiler, the backend and the lint: compile, print, check that every loss is reported,
//! lint the file set as a post-condition. A loss or a lint problem fails the container it belongs to (and the
//! containers that show it), which the backend prints again as a failed document; only a problem no container owns
//! fails the presentation.

use indexmap::IndexMap;
use slidock_canon::JsonError;
use slidock_core_api::{Causes, FailOptions, FailureCause, MediaStore, Provenance};
use slidock_core_api::{fail_containers, with_dependents};
use slidock_format::Problem;
use slidock_format::paths::{layout_path, slide_path};
use slidock_html::{EmitOptions, Emitted, HtmlError, emit_documents, supported_features};
use slidock_ir::diagnostics::{
    DataValue, Diagnostic, DiagnosticCode, DiagnosticSite, Stage, codes, make_diagnostic,
};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::NodeId;
use slidock_ir::media::{MediaPlan, PlanAction};
use slidock_ir::parse::StructureError;
use slidock_ir::{resolved, source};
use thiserror::Error;

use crate::loss::{LossCheckInput, loss_check};

/// What stops printing: a presentation the backend cannot print, or one that does not survive failing its
/// containers. Both are compiler bugs, never the input's fault.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum DriverError {
    #[error(transparent)]
    Html(#[from] HtmlError),
    #[error("the resolved presentation does not read back: {0}")]
    Ir(#[from] StructureError),
    #[error(transparent)]
    Json(#[from] JsonError),
}

/// Prints a resolved presentation as a document set; the backend by default, any printer in tests.
pub type Emit<'a> =
    dyn FnMut(&resolved::Presentation, &EmitOptions) -> Result<Emitted, HtmlError> + 'a;

/// What printing a compiled presentation reads.
#[derive(Clone, Copy)]
pub struct PrintInput<'a> {
    pub presentation: &'a resolved::Presentation,
    pub source: &'a source::Presentation,
    pub emit: &'a EmitOptions,
    /// What the frontend detected, per container and under `presentation`.
    pub features: &'a IndexMap<String, Vec<FeatureId>>,
    /// The compile's diagnostics, which account for losses.
    pub diagnostics: &'a [Diagnostic],
    /// The bytes the frontend holds; media without bytes print as omitted.
    pub media: &'a dyn MediaStore,
}

impl std::fmt::Debug for PrintInput<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrintInput")
            .field("emit", self.emit)
            .field("diagnostics", &self.diagnostics.len())
            .finish_non_exhaustive()
    }
}

/// A printed presentation.
#[derive(Clone, Debug, PartialEq)]
pub struct Printed {
    /// The presentation as printed: containers the driver failed are marked.
    pub presentation: resolved::Presentation,
    /// The document set; `None` when the presentation as a whole failed.
    pub documents: Option<slidock_html::Documents>,
    /// The loss check's and the lint's diagnostics, and those of the containers failed with them.
    pub driver: Vec<Diagnostic>,
    /// The backend's diagnostics for the documents printed; none when the presentation failed.
    pub backend: Vec<Diagnostic>,
}

impl Printed {
    /// The driver's and the backend's diagnostics.
    pub fn diagnostics(&self) -> impl Iterator<Item = &Diagnostic> {
        self.driver.iter().chain(&self.backend)
    }

    /// The backend's report of the documents printed: the media they reference and the backend's diagnostics, as
    /// indented canonical JSON.
    pub fn documents_report(&self) -> Option<Result<String, HtmlError>> {
        let documents = self.documents.as_ref()?;
        let emitted = Emitted {
            files: documents.clone(),
            diagnostics: self.backend.clone(),
        };
        Some(emitted.report())
    }
}

fn driver_diagnostic(
    code: DiagnosticCode,
    container: Option<NodeId>,
    data: &[(&'static str, DataValue)],
) -> Diagnostic {
    let mut site = DiagnosticSite::new().container(container);
    for (key, value) in data {
        site = site.data(*key, value.clone());
    }
    make_diagnostic(code, Stage::Driver, "driver", site)
}

/// The provenance a document set records.
pub fn html_provenance(provenance: &Provenance) -> slidock_html::Provenance {
    slidock_html::Provenance {
        format: provenance.format.as_str().to_owned(),
        name: provenance.name.clone(),
        sha256: provenance.sha256.to_hex(),
        bytes: provenance.bytes,
    }
}

/// Media the store cannot open (a missing or mismatched file of an agent-written presentation, already reported by
/// its frontend) prints as omitted, so every document references only files the writer can write.
pub fn with_available_media(
    presentation: &resolved::Presentation,
    store: &dyn MediaStore,
) -> resolved::Presentation {
    let mut out = presentation.clone();
    for (sha256, entry) in &mut out.media {
        if entry.plan.action == PlanAction::Omit || store.open(sha256).is_some() {
            continue;
        }
        entry.plan = MediaPlan {
            action: PlanAction::Omit,
            output: None,
            reason: Some("unavailable".to_owned()),
        };
    }
    out
}

/// Fails the containers `causes` names and those that show them; returns the presentation and the dependents'
/// diagnostics.
fn fail(
    presentation: &resolved::Presentation,
    causes: &Causes,
) -> Result<(resolved::Presentation, Vec<Diagnostic>), DriverError> {
    let mut source = presentation.to_source()?;
    let (all, dependents) = with_dependents(&source, causes);
    fail_containers(&mut source, &all, &FailOptions::default());
    Ok((resolved::Presentation::from_source(&source)?, dependents))
}

/// The container each document belongs to, by path.
fn document_owners(presentation: &resolved::Presentation) -> IndexMap<String, NodeId> {
    presentation
        .layouts
        .keys()
        .map(|id| (layout_path(id.as_str()), id.clone()))
        .chain(
            presentation
                .slides
                .iter()
                .map(|slide| (slide_path(slide.id.as_str()), slide.id.clone())),
        )
        .collect()
}

fn lint_errors(emitted: &Emitted) -> Vec<Problem> {
    emitted
        .files
        .lint()
        .into_iter()
        .filter(Problem::is_error)
        .collect()
}

/// The presentation failed as a whole, in `pass`.
fn presentation_failed(pass: &str, reason: &str) -> Diagnostic {
    driver_diagnostic(
        codes::IR0907,
        None,
        &[("pass", pass.into()), ("reason", reason.into())],
    )
}

/// Prints, loss-checks and lints a compiled presentation with the HTML backend; see the module comment.
pub fn print_presentation(input: &PrintInput<'_>) -> Result<Printed, DriverError> {
    print_presentation_with(input, &mut emit_documents)
}

/// [`print_presentation`] with any printer.
pub fn print_presentation_with(
    input: &PrintInput<'_>,
    emit: &mut Emit<'_>,
) -> Result<Printed, DriverError> {
    let mut presentation = with_available_media(input.presentation, input.media);
    let mut driver: Vec<Diagnostic> = Vec::new();
    let mut emitted = emit(&presentation, input.emit)?;
    let stopped = |presentation, driver| Printed {
        presentation,
        documents: None,
        driver,
        backend: Vec::new(),
    };

    let accounted: Vec<Diagnostic> = input
        .diagnostics
        .iter()
        .chain(&emitted.diagnostics)
        .cloned()
        .collect();
    let losses = loss_check(&LossCheckInput {
        detected: input.features,
        source: input.source,
        presentation: &presentation.to_source()?,
        diagnostics: &accounted,
        supported: &supported_features(),
    });
    driver.extend(losses.iter().cloned());
    if losses.iter().any(|loss| loss.container.is_none()) {
        driver.push(presentation_failed(
            "loss-check",
            "a presentation-level feature was lost",
        ));
        return Ok(stopped(presentation, driver));
    }
    if !losses.is_empty() {
        let causes: Causes = losses
            .into_iter()
            .filter_map(|loss| Some((loss.container.clone()?, FailureCause::new(loss))))
            .collect();
        let (failed, dependents) = fail(&presentation, &causes)?;
        presentation = failed;
        driver.extend(dependents);
        emitted = emit(&presentation, input.emit)?;
    }

    let problems = lint_errors(&emitted);
    if !problems.is_empty() {
        let owners = document_owners(&presentation);
        let lint_diagnostics: Vec<Diagnostic> = problems
            .iter()
            .map(|problem| {
                driver_diagnostic(
                    codes::HTML6900,
                    owners.get(&problem.file).cloned(),
                    &[
                        ("file", problem.file.as_str().into()),
                        ("rule", problem.code.name().into()),
                        ("problem", problem.message.as_str().into()),
                    ],
                )
            })
            .collect();
        driver.extend(lint_diagnostics.iter().cloned());
        if let Some(unowned) = problems.iter().find(|p| !owners.contains_key(&p.file)) {
            driver.push(presentation_failed("lint", &unowned.message));
            return Ok(stopped(presentation, driver));
        }
        let mut causes = Causes::new();
        for diagnostic in lint_diagnostics {
            if let Some(owner) = diagnostic.container.clone() {
                causes
                    .entry(owner)
                    .or_insert_with(|| FailureCause::new(diagnostic));
            }
        }
        let (failed, dependents) = fail(&presentation, &causes)?;
        presentation = failed;
        driver.extend(dependents);
        emitted = emit(&presentation, input.emit)?;
        if let Some(remaining) = lint_errors(&emitted).first() {
            // The failed-document printer itself breaks the format.
            driver.push(presentation_failed("lint", &remaining.message));
            return Ok(stopped(presentation, driver));
        }
    }
    Ok(Printed {
        presentation,
        documents: Some(emitted.files),
        driver,
        backend: emitted.diagnostics,
    })
}
