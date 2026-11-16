//! A page's motion: its transition (the drawing-page style's `smil:*` and `presentation:transition-*`) and its build
//! animations (the `anim:*` timing tree). How far they lower is the frontend's [`MotionMode`]: the parity path detects
//! both, reports them (OD9402, OD9401) and lowers none, so the static render is the slide fully built; native motion
//! reads them into the effect vocabulary ([`crate::transition`], [`crate::timing`]) and detects what it lowers (the
//! slide's transition and timeline), or reports a timing tree none of whose effects lowered (OD9401).

use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::features::FeatureId;
use slidock_ir::presentation::SlideSize;
use slidock_ir::source::Element as IrElement;
use slidock_ir::timeline::{TimelineStep, Transition};

use crate::context::Lowering;
use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::styles::{PropertySet, get};
use crate::{timing, transition};

/// How far a page's motion lowers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum MotionMode {
    /// Detected and reported, not lowered: the parity path.
    #[default]
    Detect,
    /// Read into the effect vocabulary.
    Native,
}

/// What a page's motion lowers to.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Motion {
    pub transition: Option<Transition>,
    pub timeline: Vec<TimelineStep>,
}

/// Whether a page sets a transition: an effect, or an automatic advance.
fn has_transition(props: &PropertySet) -> bool {
    let style = get(props, qn!("presentation:transition-style"));
    get(props, qn!("smil:type")).is_some()
        || style.is_some_and(|style| style != "none")
        || get(props, qn!("presentation:transition-type")) == Some("automatic")
}

/// Whether a page's timing tree animates anything: an effect aimed at an element, or a preset effect class.
fn animates(element: &Element) -> bool {
    element.kids().any(|node| {
        (node.in_namespace(Ns::Anim)
            && (node.attr(qn!("smil:targetElement")).is_some()
                || node.attr(qn!("presentation:preset-class")).is_some()))
            || animates(node)
    })
}

/// What a page's motion reads: the page, its drawing-page properties, the elements it lowered and the slide's size.
pub(crate) struct MotionSource<'p, 'a> {
    pub page: &'a Element,
    /// The page's path, which its drawables' origins extend.
    pub path: &'p str,
    pub props: &'p PropertySet,
    pub elements: &'p [IrElement],
    pub size: &'p SlideSize,
}

/// The page's motion, from its drawing-page properties and its timing tree.
pub(crate) fn read<'a>(
    source: &MotionSource<'_, 'a>,
    mode: MotionMode,
    ctx: &Lowering<'a, '_>,
) -> Motion {
    let page = source.page;
    let transitions = has_transition(source.props);
    let animated = page
        .kids()
        .any(|node| node.in_namespace(Ns::Anim) && animates(node));
    if transitions {
        ctx.see(FeatureId::SlideTransition);
    }
    match mode {
        MotionMode::Detect => {
            if animated {
                ctx.see(FeatureId::Animation);
            }
            if transitions {
                ctx.report(
                    codes::OD9402,
                    DiagnosticSite::new().feature(FeatureId::SlideTransition),
                );
            }
            if animated {
                ctx.report(
                    codes::OD9401,
                    DiagnosticSite::new().feature(FeatureId::Animation),
                );
            }
            Motion::default()
        }
        MotionMode::Native => {
            let transition = transition::read(page, source.props, ctx);
            let timeline = if animated {
                timing::read(page, source.path, source.elements, source.size, ctx)
            } else {
                Vec::new()
            };
            if !timeline.is_empty() {
                ctx.see(FeatureId::SlideTimeline);
            } else if animated {
                // Every effect was left out, each with its own diagnostic: the slide shows fully built.
                ctx.see(FeatureId::Animation);
                ctx.report(
                    codes::OD9401,
                    DiagnosticSite::new().feature(FeatureId::Animation),
                );
            }
            Motion {
                transition,
                timeline,
            }
        }
    }
}
