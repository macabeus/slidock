//! Diagnostics for Foreign nodes, emitted here so every frontend gets them.

use slidock_core_api::visit::walk_elements;
use slidock_ir::diagnostics::{Diagnostic, DiagnosticSite, Stage, codes, make_diagnostic};
use slidock_ir::elements::ForeignType;
use slidock_ir::source::{Element, Presentation};

/// The wire name of a Foreign type (`table`, `3d`).
fn type_name(foreign_type: ForeignType) -> String {
    serde_json::to_value(foreign_type)
        .ok()
        .and_then(|value| value.as_str().map(str::to_owned))
        .unwrap_or_default()
}

/// One diagnostic per Foreign node: IR0910 (decline) when the node has no fallback, IR0911 (lossy) when its fallback
/// prints instead.
pub fn foreign_diagnostics(presentation: &Presentation) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for container in presentation.containers() {
        for element in walk_elements(container.elements()) {
            let Element::Foreign(foreign) = element else {
                continue;
            };
            let fallback = foreign.fallback.as_ref().is_some_and(|fallback| {
                fallback.media.is_some()
                    || fallback.elements.as_ref().is_some_and(|e| !e.is_empty())
            });
            let code = if fallback {
                codes::IR0911
            } else {
                codes::IR0910
            };
            let site = DiagnosticSite::new()
                .container(container.id().clone())
                .node(foreign.id.clone())
                .origin(foreign.origin.clone())
                .feature(foreign.foreign_type.feature())
                .data("type", type_name(foreign.foreign_type));
            out.push(make_diagnostic(code, Stage::Frontend, "foreign", site));
        }
    }
    out
}
