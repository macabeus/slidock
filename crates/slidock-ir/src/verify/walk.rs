//! Walking a presentation: containers in diagnostic order, elements depth-first in z-order, and the text bodies a
//! container renders.

use crate::ids::NodeId;
use crate::path::{IrPath, PathSeg};
use crate::source::{Container, Element, Presentation};
use crate::table::table_cells;
use crate::text::TextBody;

/// A container with its path from the root (`masters["k"]`, `layouts["k"]`, `slides[i]`).
#[derive(Clone, Debug)]
pub struct ContainerRef<'a> {
    pub container: Container<'a>,
    pub path: IrPath,
}

/// Masters, then layouts, then slides, each in presentation order: the order diagnostics follow.
pub fn containers_of(presentation: &Presentation) -> Vec<ContainerRef<'_>> {
    let masters = presentation
        .masters
        .iter()
        .map(|(key, master)| ContainerRef {
            container: Container::Master(master),
            path: IrPath::root().join(["masters".into(), PathSeg::from(key.to_string())]),
        });
    let layouts = presentation
        .layouts
        .iter()
        .map(|(key, layout)| ContainerRef {
            container: Container::Layout(layout),
            path: IrPath::root().join(["layouts".into(), PathSeg::from(key.to_string())]),
        });
    let slides = presentation
        .slides
        .iter()
        .enumerate()
        .map(|(i, slide)| ContainerRef {
            container: Container::Slide(slide),
            path: IrPath::root().join([PathSeg::from("slides"), PathSeg::Index(i)]),
        });
    masters.chain(layouts).chain(slides).collect()
}

/// An element with its path and nesting depth (1 for a container's own elements).
#[derive(Clone, Debug)]
pub struct ElementRef<'a> {
    pub element: &'a Element,
    pub path: IrPath,
    pub depth: usize,
}

/// Every element under `elements`, depth-first in z-order, through group children and Foreign fallbacks.
pub fn walk_elements<'a>(elements: &'a [Element], path: &IrPath) -> Vec<ElementRef<'a>> {
    let mut out = Vec::new();
    walk_into(elements, path, 1, &mut out);
    out
}

fn walk_into<'a>(
    elements: &'a [Element],
    path: &IrPath,
    depth: usize,
    out: &mut Vec<ElementRef<'a>>,
) {
    for (index, element) in elements.iter().enumerate() {
        let element_path = path.with(index);
        out.push(ElementRef {
            element,
            path: element_path.clone(),
            depth,
        });
        match element {
            Element::Group(group) => walk_into(
                &group.children,
                &element_path.with("children"),
                depth + 1,
                out,
            ),
            Element::Foreign(foreign) => {
                if let Some(children) = foreign.fallback.as_ref().and_then(|f| f.elements.as_ref())
                {
                    walk_into(
                        children,
                        &element_path.join(["fallback", "elements"]),
                        depth + 1,
                        out,
                    );
                }
            }
            _ => {}
        }
    }
}

/// Every element of a container.
pub fn container_elements<'a>(container: &ContainerRef<'a>) -> Vec<ElementRef<'a>> {
    walk_elements(
        container.container.elements(),
        &container.path.with("elements"),
    )
}

/// A text body an element draws, with its path from the root and the element's id.
#[derive(Clone, Debug)]
pub struct RenderedText<'a> {
    pub text: &'a TextBody,
    pub path: IrPath,
    pub node: &'a NodeId,
}

/// The text bodies an element draws itself (its children's are theirs), with their paths below the element: a
/// shape's `["text"]`, each table cell's `["rows", r, "cells", c, "text"]`.
pub fn element_texts(element: &Element) -> Vec<(&TextBody, Vec<PathSeg>)> {
    match element {
        Element::Shape(shape) => shape
            .text
            .iter()
            .map(|text| (text, vec!["text".into()]))
            .collect(),
        Element::Table(table) => table_cells(&table.rows)
            .map(|(cell, r, c)| {
                let path = vec![
                    "rows".into(),
                    PathSeg::Index(r),
                    "cells".into(),
                    PathSeg::Index(c),
                    "text".into(),
                ];
                (&cell.text, path)
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// The text bodies a container renders: its shapes' text and its table cells'.
pub fn rendered_text<'a>(container: &ContainerRef<'a>) -> Vec<RenderedText<'a>> {
    let mut out = Vec::new();
    for element in container_elements(container) {
        for (text, path) in element_texts(element.element) {
            out.push(RenderedText {
                text,
                path: element.path.join(path),
                node: element.element.id(),
            });
        }
    }
    out
}
