//! What lowering one container shares: the object graph, how the container reports, the features it saw in the
//! source (the loss check's input) and where media comes from.

use std::fmt;

use slidock_core_api::Limits;
use slidock_core_api::limits::{CellBudget, ContainerBudget, container_budget};
use slidock_core_api::lowering::ContainerReporter;
use slidock_io::SvgRefusal;
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSink, DiagnosticSite};
use slidock_ir::features::FeatureId;
use slidock_ir::ids::{NodeId, Sha256};

use crate::graph::ObjectGraph;
use crate::proto::tsp;

/// A data file's id in the package's media table (`TSP.PackageMetadata.datas`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataId(pub u64);

impl DataId {
    /// The data id a reference names.
    pub fn of(reference: Option<&tsp::DataReference>) -> Option<DataId> {
        reference.and_then(|r| r.identifier).map(DataId)
    }
}

impl fmt::Display for DataId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// How a data file becomes media.
pub trait MediaLookup {
    /// The media of a data id, whatever its type; `None` when the package lacks it.
    fn media(&mut self, data: Option<DataId>) -> Option<Sha256>;

    /// Whether a data id holds an image the output can show: a raster sniffed from its first bytes, or an SVG whose
    /// whole document passes `slidock_io::check_svg`.
    fn is_image(&mut self, data: Option<DataId>) -> bool;

    /// Whether a data id holds an image only a host may decode (`slidock_core_api::HOST_DECODED`: a HEIC photo).
    fn host_decodes(&mut self, _data: Option<DataId>) -> bool {
        false
    }

    /// Why a data id that holds an SVG is not shown; `None` for anything else.
    fn svg_refusal(&mut self, _data: Option<DataId>) -> Option<SvgRefusal> {
        None
    }
}

/// Which kind of container is lowering: template slides become layouts, whose placeholders are unprinted prototypes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layer {
    Slide,
    Layout,
}

/// The container-independent part of a [`Lowering`].
pub struct Setup<'a> {
    pub graph: &'a ObjectGraph,
    /// The container's id.
    pub container: NodeId,
    pub layer: Layer,
    pub limits: &'a Limits,
    pub media: &'a mut dyn MediaLookup,
    /// The presentation's width, which bounds how far a box that grows with its text may grow.
    pub slide_width: f64,
    /// The presentation's height, the box of a background's gradient.
    pub slide_height: f64,
    /// The 1-based show position written into slide-number fields; the runtime renumbers.
    pub slide_number: usize,
    /// The deck path ([`slidock_core_api::LowerOptions::native_motion`]): the native readings that change the IR's
    /// own fields (where a transition plays, a build's delivery) apply, and how the native motion data plays is
    /// reported (KN1404–KN1409).
    pub native_motion: bool,
}

pub struct Lowering<'a> {
    pub reporter: ContainerReporter<'a>,
    pub graph: &'a ObjectGraph,
    pub layer: Layer,
    pub limits: &'a Limits,
    /// What the container's tables may still hold.
    pub budget: ContainerBudget,
    pub media: &'a mut dyn MediaLookup,
    pub slide_width: f64,
    pub slide_height: f64,
    pub slide_number: usize,
    pub native_motion: bool,
}

impl fmt::Debug for Lowering<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Lowering")
            .field("container", self.reporter.container())
            .field("layer", &self.layer)
            .field("budget", &self.budget)
            .field("slide_width", &self.slide_width)
            .field("slide_number", &self.slide_number)
            .field("native_motion", &self.native_motion)
            .finish_non_exhaustive()
    }
}

impl<'a> Lowering<'a> {
    /// A container's lowering, its budget what the presentation has left when it starts (`cells`).
    pub fn new(setup: Setup<'a>, cells: &CellBudget, diag: &'a mut dyn DiagnosticSink) -> Self {
        Lowering {
            budget: container_budget(setup.limits, cells),
            reporter: ContainerReporter::new(diag, setup.container),
            graph: setup.graph,
            layer: setup.layer,
            limits: setup.limits,
            media: setup.media,
            slide_width: setup.slide_width,
            slide_height: setup.slide_height,
            slide_number: setup.slide_number,
            native_motion: setup.native_motion,
        }
    }

    pub fn container(&self) -> &NodeId {
        self.reporter.container()
    }

    /// A diagnostic on this container.
    pub fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        self.reporter.report(code, site);
    }

    /// A diagnostic on how this container's native motion data plays, reported on the deck path only.
    pub fn report_motion(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        if self.native_motion {
            self.reporter.report(code, site);
        }
    }

    /// Records a feature the source holds.
    pub fn see(&mut self, feature: FeatureId) {
        self.reporter.seen.insert(feature);
    }
}
