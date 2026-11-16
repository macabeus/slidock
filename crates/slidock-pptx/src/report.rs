//! How the readers report: a code with its site, wherever they read (on an element, on its container, or on the
//! presentation), and the colour modifiers the IR drops (PX2107), which always land on the container or the
//! presentation, never on an element.

use slidock_core_api::lowering::ContainerReporter;
use slidock_ir::diagnostics::{DiagnosticCode, DiagnosticSink, DiagnosticSite, codes};
use slidock_ir::ids::NodeId;

pub(crate) trait Report {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite);

    /// A colour modifier the IR drops (PX2107).
    fn dropped(&mut self, modifier: &str);
}

/// The site of a dropped modifier.
pub(crate) fn dropped_site(modifier: &str) -> DiagnosticSite {
    DiagnosticSite::new().data("modifier", modifier)
}

/// Reports on the presentation.
pub(crate) struct OnPresentation<'s>(pub &'s mut dyn DiagnosticSink);

impl Report for OnPresentation<'_> {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        self.0.report(code, site);
    }

    fn dropped(&mut self, modifier: &str) {
        self.0.report(codes::PX2107, dropped_site(modifier));
    }
}

/// Reports on a container, through the sink before its lowering context exists.
pub(crate) struct OnContainer<'s> {
    pub sink: &'s mut dyn DiagnosticSink,
    pub container: NodeId,
}

impl Report for OnContainer<'_> {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        self.sink
            .report(code, site.container(self.container.clone()));
    }

    fn dropped(&mut self, modifier: &str) {
        self.report(codes::PX2107, dropped_site(modifier));
    }
}

impl Report for ContainerReporter<'_> {
    fn report(&mut self, code: DiagnosticCode, site: DiagnosticSite) {
        ContainerReporter::report(self, code, site);
    }

    fn dropped(&mut self, modifier: &str) {
        ContainerReporter::report(self, codes::PX2107, dropped_site(modifier));
    }
}
