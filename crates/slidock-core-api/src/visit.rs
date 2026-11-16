//! Walking and mapping a presentation's containers and elements.

use indexmap::{IndexMap, IndexSet};
use slidock_ir::ids::{NodeId, Sha256, SlideId};
use slidock_ir::paint::{CellFill, Fill};
use slidock_ir::source::{Element, Presentation};
use slidock_ir::table::table_cells;
use slidock_ir::text::TextBody;

/// The containers each slide shows behind its own elements: its layout unless it hides it, and that layout's master
/// unless the layout hides it. What a viewer sees of a slide comes from these and the slide itself.
pub fn slide_shows(presentation: &Presentation) -> IndexMap<SlideId, Vec<NodeId>> {
    presentation
        .slides
        .iter()
        .map(|slide| {
            let layout = slide
                .layout
                .as_ref()
                .filter(|_| slide.show_layout)
                .and_then(|id| presentation.layouts.get(id));
            let mut shown = Vec::new();
            if let Some(layout) = layout {
                shown.push(layout.id.clone());
                if let Some(master) = layout.master.as_ref().filter(|_| layout.show_master)
                    && presentation.masters.contains_key(master)
                {
                    shown.push(master.clone());
                }
            }
            (slide.id.clone(), shown)
        })
        .collect()
}

/// Every element under `elements`, depth-first in z-order, through group children and Foreign fallbacks.
pub fn walk_elements(elements: &[Element]) -> WalkElements<'_> {
    WalkElements {
        stack: vec![elements.iter()],
    }
}

/// The iterator [`walk_elements`] returns.
#[derive(Clone, Debug)]
pub struct WalkElements<'a> {
    stack: Vec<std::slice::Iter<'a, Element>>,
}

impl<'a> Iterator for WalkElements<'a> {
    type Item = &'a Element;

    fn next(&mut self) -> Option<&'a Element> {
        loop {
            let top = self.stack.last_mut()?;
            let Some(element) = top.next() else {
                self.stack.pop();
                continue;
            };
            match element {
                Element::Group(group) => self.stack.push(group.children.iter()),
                Element::Foreign(foreign) => {
                    if let Some(children) =
                        foreign.fallback.as_ref().and_then(|f| f.elements.as_ref())
                    {
                        self.stack.push(children.iter());
                    }
                }
                _ => {}
            }
            return Some(element);
        }
    }
}

/// The media hashes an element shows itself: its picture or video, poster, image fills (a table's cells' too) and
/// Foreign fallback.
pub fn element_media(element: &Element, out: &mut Vec<Sha256>) {
    match element {
        Element::Picture(picture) => out.push(picture.media),
        Element::Media(media) => out.extend(media.media.iter().chain(media.poster.iter())),
        Element::Shape(shape) => {
            if let Some(Fill::Image { media, .. }) = &shape.fill {
                out.push(*media);
            }
        }
        Element::Table(table) => {
            for (cell, _, _) in table_cells(&table.rows) {
                if let Some(CellFill::Fill(Fill::Image { media, .. })) = &cell.fill {
                    out.push(*media);
                }
            }
        }
        Element::Foreign(foreign) => {
            out.extend(foreign.fallback.as_ref().and_then(|f| f.media));
        }
        Element::Group(_) => {}
    }
}

/// The media each container that did not fail shows (its elements', through groups and fallbacks, and its
/// background's), each once, in presentation order.
pub fn container_media(presentation: &Presentation) -> Vec<(NodeId, IndexSet<Sha256>)> {
    let mut out = Vec::new();
    for container in presentation.containers() {
        if container.failed().is_some() {
            continue;
        }
        let mut shown = Vec::new();
        for element in walk_elements(container.elements()) {
            element_media(element, &mut shown);
        }
        if let Some(Fill::Image { media, .. }) = container.background() {
            shown.push(*media);
        }
        out.push((container.id().clone(), shown.into_iter().collect()));
    }
    out
}

/// Maps every element bottom-up: `f` receives each element with its children (and Foreign fallback elements)
/// already mapped, and returns its replacement, or `None` to keep it. Returns `None` when nothing changed, so a pass
/// that changes nothing hands on its input.
pub fn map_elements(
    elements: &[Element],
    f: &mut impl FnMut(&Element) -> Option<Element>,
) -> Option<Vec<Element>> {
    let mut out: Option<Vec<Element>> = None;
    for (index, element) in elements.iter().enumerate() {
        let rebuilt = match element {
            Element::Group(group) => map_elements(&group.children, f).map(|children| {
                let mut group = group.clone();
                group.children = children;
                Element::Group(group)
            }),
            Element::Foreign(foreign) => foreign
                .fallback
                .as_ref()
                .and_then(|fallback| fallback.elements.as_ref())
                .and_then(|children| map_elements(children, f))
                .map(|children| {
                    let mut foreign = foreign.clone();
                    if let Some(fallback) = foreign.fallback.as_mut() {
                        fallback.elements = Some(children);
                    }
                    Element::Foreign(foreign)
                }),
            _ => None,
        };
        let mapped = match rebuilt {
            Some(next) => Some(f(&next).unwrap_or(next)),
            None => f(element),
        };
        match (mapped, out.as_mut()) {
            (Some(mapped), Some(out)) => out.push(mapped),
            (Some(mapped), None) => {
                let mut copy: Vec<Element> = elements.iter().take(index).cloned().collect();
                copy.push(mapped);
                out = Some(copy);
            }
            (None, Some(out)) => out.push(element.clone()),
            (None, None) => {}
        }
    }
    out
}

/// Where a text body sits on its element: a shape's own text, or a table cell's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum TextSite {
    Shape,
    Cell,
}

/// `element` with each text body it draws itself mapped by `f` (`None` keeps a body); `None` when no body changed.
pub fn map_text_bodies(
    element: &Element,
    f: &mut impl FnMut(&TextBody, TextSite) -> Option<TextBody>,
) -> Option<Element> {
    match element {
        Element::Shape(shape) => {
            let text = f(shape.text.as_ref()?, TextSite::Shape)?;
            let mut shape = shape.clone();
            shape.text = Some(text);
            Some(Element::Shape(shape))
        }
        Element::Table(table) => {
            let mut changed: Option<Element> = None;
            for (r, row) in table.rows.iter().enumerate() {
                for (c, cell) in row.cells.iter().enumerate() {
                    let Some(cell) = cell else { continue };
                    let Some(text) = f(&cell.text, TextSite::Cell) else {
                        continue;
                    };
                    let copy = changed.get_or_insert_with(|| Element::Table(table.clone()));
                    if let Element::Table(copy) = copy
                        && let Some(Some(slot)) =
                            copy.rows.get_mut(r).and_then(|row| row.cells.get_mut(c))
                    {
                        slot.text = text;
                    }
                }
            }
            changed
        }
        _ => None,
    }
}
