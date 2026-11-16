//! What lowering one container shares: where it is in the package, the styles its part sees, how it reports, and the
//! features it saw in the source (the loss check's first input).

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};

use slidock_core_api::Limits;
use slidock_core_api::limits::ContainerBudget;
use slidock_core_api::lowering::{IdScope, LowerError, TakenId};
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSink, DiagnosticSite};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::{IdPart, NodeId};

use crate::ids::{id_scope, take};
use crate::media::MediaRegistry;
use crate::package::OdpPackage;
use crate::styles::StyleScope;

/// A page's date declaration.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct DateTimeDecl {
    pub fixed: bool,
    pub text: String,
}

/// What a page's fields show: its footer and header text and its date (`presentation:*-decl`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct PageDeclarations {
    pub footer: Option<String>,
    pub header: Option<String>,
    pub date_time: Option<DateTimeDecl>,
}

/// What every view of one container shares: its element ids, its drawable count, its table budget, the features
/// it saw and the diagnostics it reported.
pub(crate) struct ContainerState {
    pub container: NodeId,
    ids: RefCell<IdScope>,
    index: Cell<i64>,
    pub budget: RefCell<ContainerBudget>,
    pub seen: RefCell<BTreeSet<FeatureId>>,
    reports: RefCell<Vec<(DiagnosticCode, DiagnosticSite)>>,
}

impl ContainerState {
    pub(crate) fn new(container: NodeId, budget: ContainerBudget) -> Self {
        ContainerState {
            container,
            ids: RefCell::new(id_scope()),
            index: Cell::new(0),
            budget: RefCell::new(budget),
            seen: RefCell::new(BTreeSet::new()),
            reports: RefCell::new(Vec::new()),
        }
    }

    /// Hands the diagnostics reported so far to the sink, each on this container.
    pub(crate) fn flush(&self, sink: &mut dyn DiagnosticSink) {
        for (code, site) in self.reports.borrow_mut().drain(..) {
            sink.report(code, site.container(self.container.clone()));
        }
    }
}

/// Lowering one container, through the styles of one part.
#[derive(Clone, Copy)]
pub(crate) struct Lowering<'a, 'd> {
    pub pkg: &'d OdpPackage<'a>,
    /// The part the container is in: `content.xml` for slides, `styles.xml` for master pages.
    pub part: &'static str,
    /// The id body element ids extend: `p3`, `m.TITLE`.
    pub key: &'d str,
    /// The styles the container's part sees.
    pub styles: &'d StyleScope<'a>,
    /// The master page the container is or shows: its outline styles serve deeper outline levels.
    pub master_name: Option<&'a str>,
    pub declarations: Option<&'d PageDeclarations>,
    /// Slide ids by page name, for links between slides.
    pub pages: &'d BTreeMap<&'a str, NodeId>,
    pub media: &'d RefCell<MediaRegistry>,
    pub limits: &'d Limits,
    pub state: &'d ContainerState,
}

impl<'a, 'd> Lowering<'a, 'd> {
    pub(crate) fn container(&self) -> &'d NodeId {
        &self.state.container
    }

    /// A diagnostic on this container.
    pub(crate) fn report(&self, code: DiagnosticCode, site: DiagnosticSite) {
        self.state.reports.borrow_mut().push((code, site));
    }

    /// Records a feature the source holds.
    pub(crate) fn see(&self, feature: FeatureId) {
        self.state.seen.borrow_mut().insert(feature);
    }

    /// The next drawable's 1-based document-order index in the container, which ids fall back on.
    pub(crate) fn next_index(&self) -> i64 {
        let next = self.state.index.get() + 1;
        self.state.index.set(next);
        next
    }

    /// An element id in this container.
    pub(crate) fn take_id(&self, parts: &[IdPart<'_>]) -> Result<TakenId, LowerError> {
        take(&mut self.state.ids.borrow_mut(), parts)
    }

    /// The same container read through another part's styles: a master element materialised on a slide.
    pub(crate) fn through_styles(&self, styles: &'d StyleScope<'a>, part: &'static str) -> Self {
        Lowering {
            styles,
            part,
            ..*self
        }
    }
}
