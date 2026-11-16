//! What `slidock compile --emit report` writes: provenance, diagnostics, the features the frontend detected, which
//! containers each slide shows, and pass timings when the host gave a clock.

use indexmap::IndexMap;
use serde::Serialize;
use slidock_core::CompileResult;
use slidock_core_api::Provenance;
use slidock_core_api::visit::slide_shows;
use slidock_ir::diagnostics::Diagnostic;
use slidock_ir::features::FeatureId;
use slidock_ir::ids::{NodeId, SlideId};
use slidock_ir::{resolved, source};

use crate::driver::DriverError;

/// The presentation a report describes: the source in the source phase, the printed or resolved one after it.
#[derive(Clone, Copy, Debug)]
pub enum Described<'a> {
    Source(&'a source::Presentation),
    Resolved(&'a resolved::Presentation),
}

#[derive(Serialize)]
struct Stats<'a> {
    passes: &'a [slidock_core::PassTiming],
}

#[derive(Serialize)]
struct Report<'a> {
    /// Null when the compile stopped before the frontend lowered the input.
    provenance: Option<&'a Provenance>,
    diagnostics: &'a [Diagnostic],
    features: &'a IndexMap<String, Vec<FeatureId>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    shows: Option<IndexMap<SlideId, Vec<NodeId>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    stats: Option<Stats<'a>>,
}

/// The compile report as indented canonical JSON. `diagnostics` are every diagnostic, in output order.
pub fn compile_report(
    result: &CompileResult,
    presentation: Option<Described<'_>>,
    diagnostics: &[Diagnostic],
) -> Result<String, DriverError> {
    let shows = match presentation {
        Some(Described::Source(source)) => Some(slide_shows(source)),
        Some(Described::Resolved(resolved)) => Some(slide_shows(&resolved.to_source()?)),
        None => None,
    };
    let report = Report {
        provenance: result.provenance.as_ref(),
        diagnostics,
        features: &result.features,
        shows,
        stats: result.stats.as_ref().map(|stats| Stats {
            passes: &stats.passes,
        }),
    };
    Ok(slidock_canon::to_canonical_json(
        &report,
        slidock_canon::Layout::Indented,
    )?)
}
