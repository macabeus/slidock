//! The loss check: every feature a frontend detected reaches the output, or a diagnostic of the stage that lost it
//! names it at the feature's floor or above. Three checks per container (masters and layouts included):
//! 1. detected − census(source IR): the frontend dropped it;
//! 2. census(source IR) − census(resolved IR): a pass dropped it;
//! 3. census(resolved IR) − what the backend prints: the backend leaves it out (or the core's IR0910/IR0911 name a
//!    Foreign node's content, which no backend prints, whatever it prints of that feature elsewhere).
//!
//! An unaccounted loss is IR0901, an error: a broken compiler contract, which fails its container.

use indexmap::IndexMap;
use serde::Serialize;
use slidock_core_api::visit::walk_elements;
use slidock_ir::diagnostics::{Diagnostic, DiagnosticSite, Stage, codes, make_diagnostic};
use slidock_ir::elements::ForeignType;
use slidock_ir::features::{FeatureId, census_container};
use slidock_ir::ids::NodeId;
use slidock_ir::source::{Container, Element, Presentation};

/// What the loss check compares.
#[derive(Clone, Copy, Debug)]
pub struct LossCheckInput<'a> {
    /// What the frontend detected, per container id and under `presentation`.
    pub detected: &'a IndexMap<String, Vec<FeatureId>>,
    /// The verified source presentation.
    pub source: &'a Presentation,
    /// The resolved presentation, in the source phase's types.
    pub presentation: &'a Presentation,
    /// Every diagnostic so far: frontend, verifier, passes and backend.
    pub diagnostics: &'a [Diagnostic],
    /// What the backend prints.
    pub supported: &'a [FeatureId],
}

/// Which stage a check holds to account.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Check {
    Frontend,
    Pass,
    Backend,
}

impl Check {
    fn as_str(self) -> &'static str {
        match self {
            Check::Frontend => "frontend",
            Check::Pass => "pass",
            Check::Backend => "backend",
        }
    }

    fn stage(self) -> Stage {
        match self {
            Check::Frontend => Stage::Frontend,
            Check::Pass => Stage::Pass,
            Check::Backend => Stage::Backend,
        }
    }
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

fn is_foreign_feature(feature: FeatureId) -> bool {
    FOREIGN_TYPES.iter().any(|t| t.feature() == feature)
}

/// Whether `d` accounts for losing `feature` on `container` at this check.
fn accounts(d: &Diagnostic, feature: FeatureId, container: Option<&NodeId>, check: Check) -> bool {
    if d.feature != Some(feature) {
        return false;
    }
    if d.container.is_some() && d.container.as_ref() != container {
        return false;
    }
    // A Foreign node's content is named by the core in one place, whichever check finds it missing.
    if (d.code == codes::IR0910 || d.code == codes::IR0911) && is_foreign_feature(feature) {
        return check == Check::Backend;
    }
    d.stage == check.stage() && d.severity.at_least(feature.floor())
}

fn minus(a: &[FeatureId], b: &[FeatureId]) -> Vec<FeatureId> {
    a.iter().copied().filter(|f| !b.contains(f)).collect()
}

/// The features a container's Foreign nodes carry: their content prints as a fallback or a marker at most.
fn foreign_carried(container: Container<'_>) -> Vec<FeatureId> {
    walk_elements(container.elements())
        .filter_map(|element| match element {
            Element::Foreign(foreign) => Some(foreign.foreign_type.feature()),
            _ => None,
        })
        .collect()
}

/// The presentation-level features: what the census records under `presentation`.
fn presentation_level(presentation: &Presentation) -> Vec<FeatureId> {
    slidock_ir::features::census_presentation(presentation)
        .shift_remove("presentation")
        .unwrap_or_default()
}

/// The containers by id: masters, layouts, slides.
fn containers_of(presentation: &Presentation) -> IndexMap<&NodeId, Container<'_>> {
    presentation.containers().map(|c| (c.id(), c)).collect()
}

/// IR0901 for every loss no diagnostic accounts for; its container is absent for a presentation-level feature.
pub fn loss_check(input: &LossCheckInput<'_>) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut report = |features: Vec<FeatureId>, container: Option<&NodeId>, check: Check| {
        for feature in features {
            if input
                .diagnostics
                .iter()
                .any(|d| accounts(d, feature, container, check))
            {
                continue;
            }
            let site = DiagnosticSite::new()
                .container(container.cloned())
                .feature(feature)
                .data("feature", feature.as_str())
                .data("check", check.as_str());
            out.push(make_diagnostic(
                codes::IR0901,
                Stage::Driver,
                "loss-check",
                site,
            ));
        }
    };
    let empty = Vec::new();
    let detected = |key: &str| input.detected.get(key).unwrap_or(&empty);

    let source_level = presentation_level(input.source);
    let resolved_level = presentation_level(input.presentation);
    report(
        minus(detected("presentation"), &source_level),
        None,
        Check::Frontend,
    );
    report(minus(&source_level, &resolved_level), None, Check::Pass);
    report(
        minus(&resolved_level, input.supported),
        None,
        Check::Backend,
    );

    let sources = containers_of(input.source);
    for (id, resolved) in containers_of(input.presentation) {
        let Some(original) = sources.get(id) else {
            continue;
        };
        if original.failed().is_some() || resolved.failed().is_some() {
            continue;
        }
        let before = census_container(*original, input.source);
        let after = census_container(resolved, input.presentation);
        report(
            minus(detected(id.as_str()), &before),
            Some(id),
            Check::Frontend,
        );
        report(minus(&before, &after), Some(id), Check::Pass);
        let printed = minus(input.supported, &foreign_carried(resolved));
        report(minus(&after, &printed), Some(id), Check::Backend);
    }
    out
}
