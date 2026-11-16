//! resolve-inheritance: every value an element takes from the placeholders it inherits is copied onto it, nearest
//! ancestor first, so every rendered value sits on the element itself. Frames for every element, a table's sized by its
//! grid; geometry, fill, stroke, text-box properties (field-wise) and list styles (per level, field-wise) for shapes;
//! the shadow (`null` included) for every element that carries one, from the nearest ancestor that states one,
//! whatever its kind. Text bodies then take what is still unset from the presentation defaults; a table cell's takes
//! no autofit unless it states one, as its row grows instead. Text content stays the element's own.

use std::collections::BTreeMap;

use slidock_core_api::visit::walk_elements;
use slidock_core_api::{ContainerView, OwnedContainer, PassCtx, PassError, PassInfo, SlidePass};
use slidock_ir::ids::NodeId;
use slidock_ir::presentation::limits::INHERITS_CHAIN;
use slidock_ir::source::{Element, Shape};
use slidock_ir::table::grid_size;
use slidock_ir::text::{Autofit, PartialBodyProps, TextBody};
use slidock_ir::units::Size;
use slidock_ir::verify::Invariant;

use crate::container::{try_map_text_bodies, with_mapped_elements};
use crate::layers::{Layer, merge_list_styles, partial_body};

pub struct ResolveInheritance;

static INFO: PassInfo = PassInfo::new(
    "resolve-inheritance",
    1,
    &[Invariant::PlaceholdersBound],
    &[Invariant::InheritanceResolved],
);

impl SlidePass for ResolveInheritance {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        view: &ContainerView<'_>,
        _ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError> {
        let index = chain_index(view);
        let defaults = partial_body(&view.defaults.body);
        with_mapped_elements(view.container, |element| {
            resolve(element, &index, &defaults).map(Some)
        })
    }
}

/// Elements the view's chain holds, by id: the container's, its layout's and its master's, the first of an id
/// winning.
fn chain_index<'a>(view: &ContainerView<'a>) -> BTreeMap<&'a NodeId, &'a Element> {
    let mut index = BTreeMap::new();
    let chain = [
        Some(view.container.elements()),
        view.layout.map(|l| l.elements.as_slice()),
        view.master.map(|m| m.elements.as_slice()),
    ];
    for elements in chain.into_iter().flatten() {
        for element in walk_elements(elements) {
            index.entry(element.id()).or_insert(element);
        }
    }
    index
}

/// The elements `element` inherits from, nearest first; the verifier bounds the chain and rules out cycles, and
/// the walk stops at a repeat or past the bound regardless.
fn ancestors<'a>(element: &Element, index: &BTreeMap<&NodeId, &'a Element>) -> Vec<&'a Element> {
    let mut out: Vec<&Element> = Vec::new();
    let mut seen: Vec<&NodeId> = vec![element.id()];
    let mut next = element.inherits();
    while let Some(id) = next {
        if seen.contains(&id) || out.len() >= INHERITS_CHAIN {
            break;
        }
        let Some(ancestor) = index.get(id).copied() else {
            break;
        };
        out.push(ancestor);
        seen.push(ancestor.id());
        next = ancestor.inherits();
    }
    out
}

/// What a table cell's text box holds over the defaults: rows grow to fit their text, so a cell does not fit it.
fn cell_body() -> PartialBodyProps {
    PartialBodyProps {
        autofit: Some(Autofit::None {}),
        ..Default::default()
    }
}

fn inherit_text(text: &TextBody, shapes: &[&Shape], defaults: &PartialBodyProps) -> TextBody {
    // Farthest first, so the element's own values land last.
    let chain: Vec<&TextBody> = shapes
        .iter()
        .rev()
        .filter_map(|s| s.text.as_ref())
        .collect();
    let body = PartialBodyProps::layered(
        std::iter::once(Some(defaults))
            .chain(chain.iter().map(|t| Some(&t.body)))
            .chain(std::iter::once(Some(&text.body))),
    );
    let list_style = merge_list_styles(
        chain
            .iter()
            .map(|t| t.list_style.as_ref())
            .chain(std::iter::once(text.list_style.as_ref())),
    );
    let mut out = text.clone();
    out.body = body;
    if list_style.is_some() {
        out.list_style = list_style;
    }
    out
}

fn resolve(
    element: &Element,
    index: &BTreeMap<&NodeId, &Element>,
    defaults: &PartialBodyProps,
) -> Result<Element, PassError> {
    let chain = ancestors(element, index);
    let mut out = element.clone();
    if out.frame().is_none() {
        let inherited = chain.iter().find_map(|ancestor| ancestor.frame().clone());
        *out.frame_mut() = inherited;
    }
    // An explicit `null` is "no shadow"; only an absent shadow inherits.
    if let Some(shadow) = out.shadow_mut()
        && shadow.is_none()
    {
        *shadow = chain
            .iter()
            .find_map(|ancestor| ancestor.shadow().map(|stated| stated.cloned()));
    }
    match &mut out {
        Element::Table(table) => {
            // A table's box is its grid's, wherever its frame comes from.
            if let Some(frame) = table.frame.as_mut() {
                let (w, h) = grid_size(&table.columns, &table.rows);
                frame.w = Size::from_steps(w.steps());
                frame.h = Size::from_steps(h.steps());
            }
            let cell = cell_body();
            let table = Element::Table(table.clone());
            let mapped = try_map_text_bodies(&table, |text, _| {
                let mut text = text.clone();
                text.body =
                    PartialBodyProps::layered([Some(defaults), Some(&cell), Some(&text.body)]);
                Ok(text)
            })?;
            Ok(mapped.unwrap_or(table))
        }
        Element::Shape(shape) => {
            let shapes: Vec<&Shape> = chain
                .iter()
                .filter_map(|ancestor| match ancestor {
                    Element::Shape(shape) => Some(&**shape),
                    _ => None,
                })
                .collect();
            if shape.geometry.is_none() {
                shape.geometry = shapes.iter().find_map(|s| s.geometry.clone());
            }
            if shape.fill.is_none() {
                shape.fill = shapes.iter().find_map(|s| s.fill.clone());
            }
            // An explicit `null` is "no stroke"; only an absent stroke inherits.
            if shape.stroke.is_none() {
                shape.stroke = shapes.iter().find_map(|s| s.stroke.clone());
            }
            if let Some(text) = &shape.text {
                shape.text = Some(inherit_text(text, &shapes, defaults));
            }
            Ok(out)
        }
        _ => Ok(out),
    }
}
