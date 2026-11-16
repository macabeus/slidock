//! bind-placeholders: each placeholder's `inherits` edge to the placeholder it takes unset values from. An explicit
//! target (Keynote) wins; otherwise a slide's placeholder matches its layout's by index, then by role class, and a
//! layout's matches its master's by role class. The edge is recomputed on every run.

use slidock_core_api::visit::walk_elements;
use slidock_core_api::{PassCtx, PassError, PassInfo, PresentationPass};
use slidock_ir::diagnostics::{DiagnosticSink, DiagnosticSite, codes};
use slidock_ir::elements::Placeholder;
use slidock_ir::ids::{ElementId, NodeId};
use slidock_ir::source::{Container, Element, Presentation};
use slidock_ir::verify::Invariant;

use crate::container::try_map_elements;
use crate::roles::{role_class, role_name};

pub struct BindPlaceholders;

static INFO: PassInfo = PassInfo::new(
    "bind-placeholders",
    1,
    &[Invariant::FramesCanonical],
    &[Invariant::PlaceholdersBound],
);

impl PresentationPass for BindPlaceholders {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        presentation: &Presentation,
        ctx: &mut PassCtx<'_>,
    ) -> Result<Presentation, PassError> {
        let mut out = presentation.clone();
        for (key, master) in &presentation.masters {
            let elements = bind(Container::Master(master), None, false, ctx.diag)?;
            if let (Some(elements), Some(slot)) = (elements, out.masters.get_mut(key)) {
                slot.elements = elements;
            }
        }
        for (key, layout) in &presentation.layouts {
            let master = layout
                .master
                .as_ref()
                .and_then(|id| presentation.masters.get(id))
                .map(Container::Master);
            let elements = bind(Container::Layout(layout), master, false, ctx.diag)?;
            if let (Some(elements), Some(slot)) = (elements, out.layouts.get_mut(key)) {
                slot.elements = elements;
            }
        }
        for (slide, slot) in presentation.slides.iter().zip(out.slides.iter_mut()) {
            let layout = slide
                .layout
                .as_ref()
                .and_then(|id| presentation.layouts.get(id))
                .map(Container::Layout);
            if let Some(elements) = bind(Container::Slide(slide), layout, true, ctx.diag)? {
                slot.elements = elements;
            }
        }
        Ok(out)
    }
}

/// The placeholders of a parent that is there and has not failed.
fn placeholders_of(parent: Option<Container<'_>>) -> Vec<(&Element, &Placeholder)> {
    let Some(parent) = parent.filter(|p| p.failed().is_none()) else {
        return Vec::new();
    };
    walk_elements(parent.elements())
        .filter_map(|element| element.placeholder().map(|p| (element, p)))
        .collect()
}

/// The parent placeholder an element binds to: by index (slides only), then by role class, exact role first.
fn find_match<'a>(
    placeholder: &Placeholder,
    candidates: &[(&'a Element, &Placeholder)],
    by_index: bool,
) -> Option<&'a ElementId> {
    if by_index
        && let Some(index) = placeholder.index
        && let Some((element, _)) = candidates.iter().find(|(_, p)| p.index == Some(index))
    {
        return Some(element.id());
    }
    let class = role_class(placeholder.role);
    let same_class: Vec<&(&Element, &Placeholder)> = candidates
        .iter()
        .filter(|(_, p)| role_class(p.role) == class)
        .collect();
    same_class
        .iter()
        .find(|(_, p)| p.role == placeholder.role)
        .or(same_class.first())
        .map(|(element, _)| element.id())
}

/// The container's elements with every placeholder's edge recomputed; `None` when nothing changed.
fn bind(
    container: Container<'_>,
    parent: Option<Container<'_>>,
    by_index: bool,
    diag: &mut dyn DiagnosticSink,
) -> Result<Option<Vec<Element>>, PassError> {
    let candidates = placeholders_of(parent);
    try_map_elements(container.elements(), |element| {
        let Some(placeholder) = element.placeholder() else {
            return Ok(None);
        };
        let target: Option<NodeId> = placeholder
            .target
            .clone()
            .or_else(|| find_match(placeholder, &candidates, by_index).cloned());
        if target.is_none()
            && let Some(parent) = parent.filter(|p| p.failed().is_none())
        {
            diag.report(
                codes::MID5101,
                DiagnosticSite::new()
                    .container(container.id().clone())
                    .node(element.id().clone())
                    .data("role", role_name(placeholder.role))
                    .data_opt("index", placeholder.index)
                    .data("parent", parent.id().as_str()),
            );
        }
        if element.inherits() == target.as_ref() {
            return Ok(None);
        }
        let mut out = element.clone();
        set_inherits(&mut out, target);
        Ok(Some(out))
    })
}

fn set_inherits(element: &mut Element, target: Option<ElementId>) {
    let slot = match element {
        Element::Shape(e) => &mut e.inherits,
        Element::Picture(e) => &mut e.inherits,
        Element::Media(e) => &mut e.inherits,
        Element::Table(e) => &mut e.inherits,
        Element::Group(e) => &mut e.inherits,
        Element::Foreign(e) => &mut e.inherits,
    };
    *slot = target;
}
