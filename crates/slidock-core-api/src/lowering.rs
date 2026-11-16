//! What every frontend shares while it lowers containers: ids unique in a namespace, the errors that fail one
//! container, the isolation that keeps that failure to its container, and how a container reports and collects the
//! features of its source. Frontends keep only their codes and their own part reading.

use std::collections::BTreeSet;

use slidock_ir::diagnostics::{
    DataValue, Diagnostic, DiagnosticCode, DiagnosticData, DiagnosticSink, DiagnosticSite,
};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::IdPrefix;
use slidock_ir::ids::{IdError, IdPart, NodeId, node_id};
use slidock_ir::presentation::Failed;
use thiserror::Error;

use crate::deadline::Stopped;
use crate::failure::{Failable, describe_error};
use crate::frontend::{FrontendCtx, FrontendError};
use crate::limits::Output;

/// Hands out unique ids in one namespace: a repeated native id gets `.<n>`, and the caller is told.
#[derive(Clone, Debug)]
pub struct IdScope {
    prefix: IdPrefix,
    used: BTreeSet<NodeId>,
}

/// An id [`IdScope::take`] handed out, and whether its native id was given out before.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TakenId {
    pub id: NodeId,
    pub duplicate: bool,
}

impl IdScope {
    pub fn new(prefix: IdPrefix) -> Self {
        IdScope {
            prefix,
            used: BTreeSet::new(),
        }
    }

    /// The id for `parts`, and whether it collided with one given out before.
    pub fn take(&mut self, parts: &[IdPart<'_>]) -> Result<TakenId, IdError> {
        let base = node_id(self.prefix, parts)?;
        if self.used.insert(base.clone()) {
            return Ok(TakenId {
                id: base,
                duplicate: false,
            });
        }
        let mut suffixed: Vec<IdPart<'_>> = parts.to_vec();
        suffixed.push(IdPart::Int(0));
        for n in 2..=i64::MAX {
            if let Some(last) = suffixed.last_mut() {
                *last = IdPart::Int(n);
            }
            let id = node_id(self.prefix, &suffixed)?;
            if self.used.insert(id.clone()) {
                return Ok(TakenId {
                    id,
                    duplicate: true,
                });
            }
        }
        // Every suffix up to i64::MAX is taken: more ids than memory holds.
        Err(IdError::NoParts)
    }
}

/// How lowering one container reports, and the features it saw in the source (the loss check's input).
pub struct ContainerReporter<'a> {
    sink: &'a mut dyn DiagnosticSink,
    container: NodeId,
    /// Features seen in the source of this container.
    pub seen: BTreeSet<FeatureId>,
}

impl std::fmt::Debug for ContainerReporter<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ContainerReporter")
            .field("container", &self.container)
            .field("seen", &self.seen)
            .finish_non_exhaustive()
    }
}

impl<'a> ContainerReporter<'a> {
    pub fn new(sink: &'a mut dyn DiagnosticSink, container: NodeId) -> Self {
        ContainerReporter {
            sink,
            container,
            seen: BTreeSet::new(),
        }
    }

    pub fn container(&self) -> &NodeId {
        &self.container
    }

    /// A diagnostic on this container; the site's container is set to it.
    pub fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        let site = site.container(self.container.clone());
        self.sink.report(code, site);
    }

    /// Features as a container records them: each once, sorted.
    pub fn features(&self) -> Vec<FeatureId> {
        self.seen.iter().copied().collect()
    }
}

/// Features as a container records them: each once, sorted by name.
pub fn sorted_features(seen: impl IntoIterator<Item = FeatureId>) -> Vec<FeatureId> {
    seen.into_iter()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// A frontend's codes for a failed container: a part it cannot read, a limit, anything else; and what every
/// failure of the container records besides its reason.
#[derive(Clone, Debug, PartialEq)]
pub struct FailureCodes {
    pub part: Option<DiagnosticCode>,
    pub limit: Option<DiagnosticCode>,
    pub other: DiagnosticCode,
    pub data: DiagnosticData,
}

impl FailureCodes {
    pub fn new(other: DiagnosticCode) -> Self {
        FailureCodes {
            part: None,
            limit: None,
            other,
            data: DiagnosticData::new(),
        }
    }

    pub fn part(mut self, code: DiagnosticCode) -> Self {
        self.part = Some(code);
        self
    }

    pub fn limit(mut self, code: DiagnosticCode) -> Self {
        self.limit = Some(code);
        self
    }

    pub fn data(mut self, key: impl Into<String>, value: impl Into<DataValue>) -> Self {
        self.data.insert(key.into(), value.into());
        self
    }
}

/// Why lowering one container stopped.
#[derive(Clone, Debug, PartialEq, Error)]
pub enum LowerError {
    /// The whole input is refused; ends the lowering.
    #[error("{}", .0.message)]
    Refused(Box<Diagnostic>),
    /// The deadline passed; ends the lowering.
    #[error("stopped: over its time limit")]
    Stopped,
    /// A frontend limit (elements, group depth, text, path segments); the container fails alone.
    #[error("{0}")]
    Limit(String),
    /// A part the container needs is missing or cannot be read; the container fails with its reason.
    #[error("{part}: {reason}")]
    Part { part: String, reason: String },
    /// A failure that names its own code and data.
    #[error("{}", .code.code())]
    Coded {
        code: DiagnosticCode,
        data: DiagnosticData,
    },
    /// Anything else; the container fails with the frontend's other code.
    #[error("{0}")]
    Other(String),
}

impl From<Stopped> for LowerError {
    fn from(_: Stopped) -> Self {
        LowerError::Stopped
    }
}

impl From<FrontendError> for LowerError {
    fn from(error: FrontendError) -> Self {
        match error {
            FrontendError::Refused(diagnostic) => LowerError::Refused(diagnostic),
            FrontendError::Stopped => LowerError::Stopped,
            FrontendError::Crashed(message) => LowerError::Other(message),
        }
    }
}

fn failure_of(error: &LowerError, codes: &FailureCodes) -> (DiagnosticCode, DiagnosticData) {
    let with_reason = |code: DiagnosticCode, extra: &[(&str, &str)]| {
        let mut data = codes.data.clone();
        for (key, value) in extra {
            data.insert((*key).to_owned(), DataValue::Str(describe_error(value)));
        }
        (code, data)
    };
    match error {
        LowerError::Coded { code, data } => (*code, data.clone()),
        LowerError::Part { part, reason } if codes.part.is_some() => {
            let mut data = codes.data.clone();
            data.insert("part".to_owned(), part.as_str().into());
            data.insert("reason".to_owned(), reason.as_str().into());
            (codes.part.unwrap_or(codes.other), data)
        }
        LowerError::Limit(reason) => {
            with_reason(codes.limit.unwrap_or(codes.other), &[("reason", reason)])
        }
        other => with_reason(codes.other, &[("reason", &other.to_string())]),
    }
}

/// Lowers one container in isolation. A refusal and a passed deadline end the whole lowering; any other error fails
/// the container alone with the frontend's code for it (a [`LowerError::Coded`] names its own): it keeps its place,
/// id and links, and loses its elements. A container whose output would pass what [`Limits::output`] has left fails
/// too (IR0914), so a presentation over the budget lowers only what fits, in the order its containers lower.
///
/// [`Limits::output`]: crate::Limits::output
pub fn isolate_container<C: Failable>(
    skeleton: &C,
    ctx: &mut FrontendCtx<'_>,
    codes: &FailureCodes,
    lower: impl FnOnce(&mut FrontendCtx<'_>) -> Result<C, LowerError>,
) -> Result<C, FrontendError> {
    ctx.deadline.check()?;
    let error = match lower(ctx) {
        Ok(container) if container.failed().is_some() => return Ok(container),
        Ok(container) => {
            let with = ctx.spent.plus(&Output::of(container.container()));
            let Some((what, limit)) = with.over(&ctx.limits.output) else {
                ctx.spent = with;
                return Ok(container);
            };
            let site = DiagnosticSite::new()
                .container(skeleton.id().clone())
                .data("pass", "lowering")
                .data("what", what)
                .data("limit", limit);
            let code = slidock_ir::diagnostics::codes::IR0914;
            ctx.diag.report(code, site);
            return Ok(skeleton.failed_skeleton(Failed::new(code, None), false));
        }
        Err(LowerError::Refused(diagnostic)) => return Err(FrontendError::Refused(diagnostic)),
        Err(LowerError::Stopped) => return Err(FrontendError::Stopped),
        Err(_) if ctx.deadline.expired() => return Err(FrontendError::Stopped),
        Err(error) => error,
    };
    let (code, data) = failure_of(&error, codes);
    let mut site = DiagnosticSite::new().container(skeleton.id().clone());
    for (key, value) in data {
        site = site.data(key, value);
    }
    ctx.diag.report(code, site);
    Ok(skeleton.failed_skeleton(Failed::new(code, None), false))
}
