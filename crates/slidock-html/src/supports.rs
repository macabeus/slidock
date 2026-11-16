//! What the HTML backend prints, as static data for the loss check, and the diagnostics that name what it leaves
//! out. Features a Foreign node carries are named by the core's IR0910/IR0911 instead, tables included.

use std::collections::BTreeSet;

use slidock_ir::diagnostics::Stage;
use slidock_ir::diagnostics::{
    Diagnostic, DiagnosticCode, DiagnosticSite, Severity, codes, make_diagnostic,
};
use slidock_ir::elements::ForeignType;
use slidock_ir::features::{FeatureId, census_container, census_presentation};
use slidock_ir::ids::NodeId;
use slidock_ir::resolved::Presentation;

use crate::context::EMITTER;
use crate::error::{HtmlError, Result};

/// What the backend does not print. A vector picture is printed: an SVG that passed the allow-list is copied, and
/// plan-media omits every other vector type (MID5601), which names the loss before the backend.
const NOT_PRINTED: &[FeatureId] = &[
    // Nothing to show in a document.
    FeatureId::ElementAudio,
    // Carried by Foreign nodes, which print their fallback or a marker.
    FeatureId::ElementChart,
    FeatureId::ElementSmartart,
    FeatureId::ElementOle,
    FeatureId::ElementMath,
    FeatureId::ElementOther,
    // Approximated or dropped by frontends, before the IR.
    FeatureId::FillPattern,
    FeatureId::FillTile,
    FeatureId::StrokeGradient,
    FeatureId::EffectGlow,
    FeatureId::EffectReflection,
    FeatureId::EffectSoftEdge,
    FeatureId::Effect3d,
    FeatureId::ImageRecolor,
    FeatureId::FrameShear,
    FeatureId::TextBulletImage,
    FeatureId::TextColumns,
    FeatureId::TextVertical,
    FeatureId::Animation,
    FeatureId::Transition,
    // Bindings render text by family name alone.
    FeatureId::FontEmbedded,
];

/// The features this backend prints.
pub fn supported_features() -> Vec<FeatureId> {
    FeatureId::ALL
        .iter()
        .copied()
        .filter(|feature| !NOT_PRINTED.contains(feature))
        .collect()
}

const FOREIGN_TYPES: [ForeignType; 9] = [
    ForeignType::Table,
    ForeignType::Chart,
    ForeignType::Smartart,
    ForeignType::Ole,
    ForeignType::Math,
    ForeignType::Ink,
    ForeignType::ThreeD,
    ForeignType::Zoom,
    ForeignType::Unknown,
];

fn is_foreign(feature: FeatureId) -> bool {
    FOREIGN_TYPES
        .iter()
        .any(|foreign| foreign.feature() == feature)
}

/// One code per severity floor, since a code has one severity (floors are decline, lossy or info).
fn unsupported_code(feature: FeatureId) -> Option<DiagnosticCode> {
    match feature.floor() {
        Severity::Decline => Some(codes::HTML6002),
        Severity::Lossy => Some(codes::HTML6003),
        Severity::Info => Some(codes::HTML6004),
        _ => None,
    }
}

fn unsupported(features: &[FeatureId], container: Option<&NodeId>, out: &mut Vec<Diagnostic>) {
    for &feature in features {
        if !NOT_PRINTED.contains(&feature) || is_foreign(feature) {
            continue;
        }
        let Some(code) = unsupported_code(feature) else {
            continue;
        };
        let site = DiagnosticSite::new()
            .container(container.cloned())
            .feature(feature)
            .data("feature", feature.as_str());
        out.push(make_diagnostic(code, Stage::Backend, EMITTER, site));
    }
}

/// A diagnostic for every feature a container (or the presentation) holds that this backend leaves out.
pub fn unsupported_diagnostics(presentation: &Presentation) -> Result<Vec<Diagnostic>> {
    let source = presentation
        .to_source()
        .map_err(|error| HtmlError::Ir(error.to_string()))?;
    let mut out = Vec::new();
    let census = census_presentation(&source);
    unsupported(
        census.get("presentation").map_or(&[], Vec::as_slice),
        None,
        &mut out,
    );
    let failed: BTreeSet<&NodeId> = presentation
        .masters
        .values()
        .filter(|master| master.body.failed().is_some())
        .map(|master| &master.id)
        .chain(
            presentation
                .layouts
                .values()
                .filter(|layout| layout.body.failed().is_some())
                .map(|layout| &layout.id),
        )
        .chain(
            presentation
                .slides
                .iter()
                .filter(|slide| slide.body.failed().is_some())
                .map(|slide| &slide.id),
        )
        .collect();
    for container in source.containers() {
        if !failed.contains(container.id()) {
            unsupported(
                &census_container(container, &source),
                Some(container.id()),
                &mut out,
            );
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn leaves_out_what_it_cannot_print_and_foreign_content_the_core_names() {
        let supported = supported_features();
        for feature in [FeatureId::ElementAudio, FeatureId::FontEmbedded] {
            assert!(!supported.contains(&feature), "{feature:?}");
        }
        for feature in [
            FeatureId::ImageVector,
            FeatureId::ElementVideo,
            FeatureId::ElementTable,
            FeatureId::TextLink,
            FeatureId::FillRadial,
            FeatureId::SlideTimeline,
            FeatureId::EffectShadow,
            FeatureId::TextShadow,
        ] {
            assert!(supported.contains(&feature), "{feature:?}");
        }
    }
}
