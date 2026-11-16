//! What lowering one container shares: where it is in the package, its theme's format scheme, how it reports, and the
//! features it saw in the source (the loss check's first input).

use std::rc::Rc;

use indexmap::IndexMap;
use slidock_core_api::Limits;
use slidock_core_api::limits::{CellBudget, ContainerBudget, container_budget};
use slidock_core_api::lowering::{ContainerReporter, IdScope};
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSink, DiagnosticSite};
use slidock_ir::features::FeatureId;
use slidock_ir::formats::IdPrefix;
use slidock_ir::ids::NodeId;
use slidock_ir::presentation::SlideSize;
use slidock_ir::text::Link;

use crate::geometry::NotANumber;
use crate::links::{LinkTargets, read_link};
use crate::media::MediaRegistry;
use crate::package::{Package, Rels};
use crate::report::Report;
use crate::table_style::TableStyles;
use crate::theme::FormatScheme;
use crate::xml::Element;

/// What every container of the presentation reads alike.
#[derive(Clone, Copy)]
pub(crate) struct Shared<'a> {
    pub pkg: &'a Package<'a>,
    /// Slide ids by part, for links between slides.
    pub slides: &'a IndexMap<String, NodeId>,
    pub table_styles: &'a TableStyles,
    pub media: &'a MediaRegistry,
    pub limits: &'a Limits,
    /// With native motion, the slide size the timing reader measures in; `None` on the parity path.
    pub motion: Option<&'a SlideSize>,
}

pub(crate) struct Lowering<'a> {
    pub pkg: &'a Package<'a>,
    pub part: String,
    pub container: NodeId,
    /// The ids of this container's elements; native ids repeat across containers, so each has its own scope.
    pub ids: IdScope,
    /// The container part's relationships.
    pub rels: Rc<Rels>,
    pub slides: &'a IndexMap<String, NodeId>,
    /// The format scheme of the container's theme, which style references index.
    pub formats: &'a FormatScheme,
    /// The presentation's table styles, which tables name by id.
    pub table_styles: &'a TableStyles,
    pub media: &'a MediaRegistry,
    pub limits: &'a Limits,
    /// What the container's tables may still hold.
    pub budget: ContainerBudget,
    pub reporter: ContainerReporter<'a>,
    /// The custom-geometry coordinates of this container that are not numbers, by element and field.
    pub not_numbers: Vec<ElementNaN>,
    pub motion: Option<&'a SlideSize>,
}

/// A coordinate that is not a number in the custom geometry `field` (`geometry`, a picture's `clip`) of `node`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ElementNaN {
    pub node: NodeId,
    pub field: &'static str,
    pub at: NotANumber,
}

impl<'a> Lowering<'a> {
    pub fn new(
        shared: Shared<'a>,
        part: &str,
        container: NodeId,
        rels: Rc<Rels>,
        formats: &'a FormatScheme,
        cells: &CellBudget,
        sink: &'a mut dyn DiagnosticSink,
    ) -> Self {
        Lowering {
            pkg: shared.pkg,
            part: part.to_owned(),
            ids: IdScope::new(IdPrefix::Pp),
            rels,
            slides: shared.slides,
            formats,
            table_styles: shared.table_styles,
            media: shared.media,
            limits: shared.limits,
            budget: container_budget(shared.limits, cells),
            reporter: ContainerReporter::new(sink, container.clone()),
            container,
            not_numbers: Vec::new(),
            motion: shared.motion,
        }
    }

    pub fn not_a_number(&mut self, node: &NodeId, field: &'static str, at: Vec<NotANumber>) {
        self.not_numbers.extend(at.into_iter().map(|at| ElementNaN {
            node: node.clone(),
            field,
            at,
        }));
    }

    pub fn seen(&mut self, feature: FeatureId) {
        self.reporter.seen.insert(feature);
    }

    /// A diagnostic on one of this container's elements.
    pub fn report_on(&mut self, node: &NodeId, code: DiagnosticCode, site: DiagnosticSite) {
        self.reporter.report(code, site.node(node.clone()));
    }

    pub fn link(&self, hlink: &Element) -> Option<Link> {
        read_link(
            hlink,
            &LinkTargets {
                rels: &self.rels,
                slides: self.slides,
            },
        )
    }
}

impl Report for Lowering<'_> {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        self.reporter.report(code, site);
    }

    fn dropped(&mut self, modifier: &str) {
        Report::dropped(&mut self.reporter, modifier);
    }
}
