//! normalize-timeline: a slide's build steps as the runtime plays them, in order. A step whose target is off the slide
//! goes; a paragraph range is kept inside its target's text, clamped to the paragraphs there are, and removed when none
//! of it exists, so the step plays on the whole target. Each change warns MID5701. Each kept step then passes through
//! [`NormalizeTimeline::check_effect`] with the element it plays on, which keeps every effect as the frontend wrote it;
//! native-effect validation belongs there.

use std::collections::BTreeMap;

use slidock_core_api::visit::walk_elements;
use slidock_core_api::{ContainerView, OwnedContainer, PassCtx, PassError, PassInfo, SlidePass};
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::ids::NodeId;
use slidock_ir::source::{Container, Element};
use slidock_ir::timeline::TimelineStep;
use slidock_ir::verify::Invariant;

pub struct NormalizeTimeline;

static INFO: PassInfo = PassInfo::new(
    "normalize-timeline",
    1,
    &[Invariant::SourceValid],
    &[Invariant::TimelineNormalized],
);

/// What became of a step: kept (perhaps changed), and why it changed.
struct Normalized {
    step: Option<TimelineStep>,
    reason: Option<&'static str>,
}

fn normalize_step(step: &TimelineStep, target: Option<&Element>) -> Normalized {
    let Some(target) = target else {
        return Normalized {
            step: None,
            reason: Some("is not on the slide"),
        };
    };
    let Some((start, end)) = step.target.paragraphs else {
        return Normalized {
            step: Some(step.clone()),
            reason: None,
        };
    };
    let count = match target {
        Element::Shape(shape) => shape.text.as_ref().map_or(0, |t| t.paragraphs.len()),
        _ => 0,
    } as i64;
    let first = i64::from(start.min(end));
    let last = i64::from(start.max(end)).min(count - 1);
    let mut kept = step.clone();
    if first > last {
        kept.target.paragraphs = None;
        return Normalized {
            step: Some(kept),
            reason: Some("has none of those paragraphs"),
        };
    }
    if first == i64::from(start) && last == i64::from(end) {
        return Normalized {
            step: Some(kept),
            reason: None,
        };
    }
    // Both ends lie in 0..=start.max(end), so they fit the range's type.
    let clamp = |x: i64| u32::try_from(x).unwrap_or(u32::MAX);
    kept.target.paragraphs = Some((clamp(first), clamp(last)));
    Normalized {
        step: Some(kept),
        reason: Some("has fewer paragraphs"),
    }
}

impl NormalizeTimeline {
    /// A kept step's effect as the runtime plays it, checked against the element it plays on; `site` locates the
    /// step for what a check reports. The frontend's effect, unchanged.
    fn check_effect(
        &self,
        step: TimelineStep,
        _target: &Element,
        _site: DiagnosticSite,
        _ctx: &mut PassCtx<'_>,
    ) -> Result<TimelineStep, PassError> {
        Ok(step)
    }
}

impl SlidePass for NormalizeTimeline {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        view: &ContainerView<'_>,
        ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError> {
        let Container::Slide(slide) = view.container else {
            return Ok(view.unchanged());
        };
        let targets: BTreeMap<&NodeId, &Element> = walk_elements(&slide.elements)
            .map(|element| (element.id(), element))
            .collect();
        let mut timeline = Vec::with_capacity(slide.timeline.len());
        for (index, step) in slide.timeline.iter().enumerate() {
            let target = &step.target.element;
            let element = targets.get(target).copied();
            let site = || {
                DiagnosticSite::new()
                    .container(slide.id.clone())
                    .node(element.map(|_| target.clone()))
                    .data("step", index)
                    .data("target", target.as_str())
            };
            let Normalized { step: kept, reason } = normalize_step(step, element);
            if let Some(reason) = reason {
                ctx.diag
                    .report(codes::MID5701, site().data("reason", reason));
            }
            if let (Some(kept), Some(element)) = (kept, element) {
                timeline.push(self.check_effect(kept, element, site(), ctx)?);
            }
        }
        let mut out = slide.clone();
        out.timeline = timeline;
        Ok(OwnedContainer::Slide(out))
    }
}
