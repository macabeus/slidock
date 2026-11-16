//! What a timing tree's `smil:targetElement` names: a drawable (its `xml:id`/`draw:id`), or one of
//! its paragraphs (`text:p xml:id`), which the IR addresses by index in the drawable's flattened paragraph list.
//! The paragraphs are numbered in the order the text reader lowers them: paragraphs and headings, list items'
//! paragraphs and nested lists in document order. A drawable without an id of its own (Google Slides' frames) is
//! found by its path, which its lowered element's origin holds; its paragraphs can still be targeted.

use std::collections::BTreeMap;

use slidock_canon::utf16;
use slidock_ir::frame::Frame;
use slidock_ir::ids::{ElementId, Sha256};
use slidock_ir::origin::Origin;
use slidock_ir::source::Element as IrElement;

use crate::ids::native_id;
use crate::names::qn;
use crate::odf::{Element, Ns};
use crate::shapes::keyed_drawables;

/// A drawable a step can target.
#[derive(Clone, Debug)]
pub(crate) struct Shape {
    pub id: ElementId,
    pub frame: Option<Frame>,
    pub paragraphs: u32,
    /// The medium a media element plays.
    pub media: Option<Sha256>,
}

/// What a target resolves to.
#[derive(Clone, Debug)]
pub(crate) struct Target<'t> {
    pub shape: &'t Shape,
    pub paragraph: Option<u32>,
}

/// The targets of one page.
#[derive(Debug, Default)]
pub(crate) struct Targets<'a> {
    /// By native id, or `#` and the path for a drawable without one (an id never holds `#`).
    shapes: BTreeMap<String, Shape>,
    paragraphs: BTreeMap<&'a str, (String, u32)>,
}

/// The ids of a drawable's paragraphs, in the text reader's order.
fn paragraph_ids(shape: &Element) -> Vec<Option<&str>> {
    fn list<'e>(list: &'e Element, out: &mut Vec<Option<&'e str>>) {
        for item in list.kids() {
            if !item.is(Ns::Text, "list-item") && !item.is(Ns::Text, "list-header") {
                continue;
            }
            blocks(item, out);
        }
    }
    fn blocks<'e>(holder: &'e Element, out: &mut Vec<Option<&'e str>>) {
        for node in holder.kids() {
            if node.is(Ns::Text, "p") || node.is(Ns::Text, "h") {
                out.push(node.attr(qn!("xml:id")).filter(|id| !id.is_empty()));
            } else if node.is(Ns::Text, "list") {
                list(node, out);
            }
        }
    }
    let holder = shape.child(Ns::Draw, "text-box").unwrap_or(shape);
    let mut out = Vec::new();
    blocks(holder, &mut out);
    out
}

/// What a lowered element gives a target: its id, frame and medium.
type Lowered<'e> = (&'e ElementId, Option<&'e Frame>, Option<Sha256>);

/// Every lowered element by its native id, and by its path when it has none; group children included.
fn by_origin(elements: &[IrElement]) -> (BTreeMap<&str, Lowered<'_>>, BTreeMap<&str, Lowered<'_>>) {
    let mut native = BTreeMap::new();
    let mut paths = BTreeMap::new();
    let mut stack: Vec<&IrElement> = elements.iter().rev().collect();
    while let Some(element) = stack.pop() {
        if let Origin::Source { object, path, .. } = element.origin() {
            let media = match element {
                IrElement::Media(media) => media.media,
                _ => None,
            };
            let lowered = (element.id(), element.frame().as_ref(), media);
            match (object, path) {
                (Some(object), _) => {
                    native.entry(object.as_str()).or_insert(lowered);
                }
                (None, Some(path)) => {
                    paths.entry(path.as_str()).or_insert(lowered);
                }
                (None, None) => {}
            }
        }
        if let IrElement::Group(group) = element {
            stack.extend(group.children.iter().rev());
        }
    }
    (native, paths)
}

impl<'a> Targets<'a> {
    /// The targets of a page at `path`: its drawables that lowered, by native id, or by path.
    pub(crate) fn of(page: &'a Element, path: &str, elements: &[IrElement]) -> Targets<'a> {
        let (lowered, by_path) = by_origin(elements);
        let mut targets = Targets::default();
        for keyed in keyed_drawables(page, path) {
            let (key, found) = match native_id(keyed.element) {
                Some(native) => (native.to_owned(), lowered.get(native)),
                None => {
                    let path = utf16::clip(&keyed.path, 512);
                    (format!("#{path}"), by_path.get(path))
                }
            };
            let Some((id, frame, media)) = found else {
                continue;
            };
            let ids = paragraph_ids(keyed.element);
            for (index, paragraph) in ids.iter().enumerate() {
                if let (Some(paragraph), Ok(index)) = (paragraph, u32::try_from(index)) {
                    targets
                        .paragraphs
                        .entry(paragraph)
                        .or_insert((key.clone(), index));
                }
            }
            targets.shapes.entry(key).or_insert(Shape {
                id: (*id).clone(),
                frame: frame.cloned(),
                paragraphs: u32::try_from(ids.len()).unwrap_or(u32::MAX),
                media: *media,
            });
        }
        targets
    }

    pub(crate) fn resolve(&self, name: &str) -> Option<Target<'_>> {
        if let Some(shape) = self.shapes.get(name) {
            return Some(Target {
                shape,
                paragraph: None,
            });
        }
        let (shape, index) = self.paragraphs.get(name)?;
        Some(Target {
            shape: self.shapes.get(shape.as_str())?,
            paragraph: Some(*index),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::odf::parse;

    #[test]
    fn numbers_paragraphs_as_the_text_reader_does() {
        let shape = parse(concat!(
            r#"<draw:rect xmlns:draw="urn:oasis:names:tc:opendocument:xmlns:drawing:1.0" "#,
            r#"xmlns:text="urn:oasis:names:tc:opendocument:xmlns:text:1.0">"#,
            r#"<text:list><text:list-item><text:p xml:id="p0">a</text:p></text:list-item>"#,
            r#"<text:list-item><text:p xml:id="p1"/></text:list-item>"#,
            r#"<text:list-item><text:p xml:id="p2">b</text:p><text:list><text:list-item>"#,
            r#"<text:p xml:id="p3">c</text:p></text:list-item></text:list></text:list-item></text:list>"#,
            r#"<text:h>h</text:h><text:p xml:id="p5">d</text:p></draw:rect>"#
        ));
        assert_eq!(
            paragraph_ids(&shape),
            [
                Some("p0"),
                Some("p1"),
                Some("p2"),
                Some("p3"),
                None,
                Some("p5")
            ]
        );
    }
}
