//! a11y: every slide's title and reading order, for navigation and assistive technology. The title is the text of its
//! first visible title placeholder, else of the text body with the largest text, a shape's or a table cell's (the
//! topmost element's on a tie, then the first in z-order), with whitespace collapsed and at most 200 characters (UTF-16
//! code units); a slide with neither has `""` (MID5801). A surrogate pair the cut would split is dropped whole, since a
//! Rust string cannot hold its lone high surrogate. The reading order is the title placeholders, then the other
//! placeholders by index, then everything else in z-order.

use std::cmp::Ordering;

use slidock_canon::{is_js_whitespace, js_trim, utf16};
use slidock_core_api::visit::walk_elements;
use slidock_core_api::{ContainerView, OwnedContainer, PassCtx, PassError, PassInfo, SlidePass};
use slidock_ir::diagnostics::{DiagnosticSite, codes};
use slidock_ir::ids::ElementId;
use slidock_ir::source::{Container, Element};
use slidock_ir::text::{Inline, TextBody};
use slidock_ir::verify::{Invariant, cascaded_run, element_texts, framed};

use crate::roles::is_title_role;

pub struct A11y;

static INFO: PassInfo = PassInfo::new(
    "a11y",
    1,
    &[Invariant::TextCascaded],
    &[Invariant::A11yDone],
);

const MAX_TITLE: usize = 200;

/// Runs of JS whitespace collapsed to one space, and the ends trimmed.
fn collapse(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut space = false;
    for c in text.chars() {
        if is_js_whitespace(c) {
            space = true;
            continue;
        }
        if space && !out.is_empty() {
            out.push(' ');
        }
        space = false;
        out.push(c);
    }
    out
}

/// The text a body shows, paragraphs and breaks as spaces, whitespace collapsed.
fn plain_text(text: &TextBody) -> String {
    let paragraphs: Vec<String> = text
        .paragraphs
        .iter()
        .map(|paragraph| {
            paragraph
                .inlines
                .iter()
                .map(|inline| match inline {
                    Inline::Run { text, .. } => text.as_str(),
                    Inline::Field { cached, .. } => cached.as_str(),
                    _ => " ",
                })
                .collect()
        })
        .collect();
    collapse(&paragraphs.join(" "))
}

/// The largest run size in a body's text, runs of only whitespace aside.
fn largest_size(text: &TextBody, node: &ElementId) -> Result<f64, PassError> {
    let mut largest: f64 = 0.0;
    for paragraph in &text.paragraphs {
        for inline in &paragraph.inlines {
            let Inline::Run { text, props, .. } = inline else {
                continue;
            };
            if js_trim(text).is_empty() {
                continue;
            }
            largest = largest.max(cascaded_run(Some(props), node)?.size.to_f64());
        }
    }
    Ok(largest)
}

/// A text body that could title its slide: what it shows, its largest size and the element drawing it.
struct Candidate<'a> {
    element: &'a Element,
    plain: String,
    size: f64,
}

fn shown(element: &Element) -> bool {
    element.hidden() != Some(true)
}

fn title_of(elements: &[Element]) -> Result<String, PassError> {
    let mut candidates: Vec<Candidate<'_>> = Vec::new();
    for element in walk_elements(elements).filter(|e| shown(e)) {
        for (text, _) in element_texts(element) {
            let plain = plain_text(text);
            if plain.is_empty() {
                continue;
            }
            candidates.push(Candidate {
                element,
                plain,
                size: largest_size(text, element.id())?,
            });
        }
    }
    let placeholder = candidates
        .iter()
        .find(|c| is_title_role(c.element.placeholder().map(|p| p.role)));
    let chosen = match placeholder {
        Some(chosen) => Some(chosen),
        None => {
            let mut best: Option<&Candidate<'_>> = None;
            for candidate in &candidates {
                best = Some(match best {
                    None => candidate,
                    Some(best) if candidate.size != best.size => {
                        if candidate.size > best.size {
                            candidate
                        } else {
                            best
                        }
                    }
                    Some(best) => {
                        if framed(candidate.element)?.y < framed(best.element)?.y {
                            candidate
                        } else {
                            best
                        }
                    }
                });
            }
            best
        }
    };
    Ok(chosen.map_or_else(String::new, |chosen| {
        js_trim(utf16::clip(&chosen.plain, MAX_TITLE)).to_owned()
    }))
}

/// Title placeholders, other placeholders by index, then the rest; each group in z-order.
fn reading_order_of(elements: &[Element]) -> Vec<ElementId> {
    let rank = |element: &Element| match element.placeholder() {
        Some(p) if is_title_role(Some(p.role)) => 0,
        Some(_) => 1,
        None => 2,
    };
    // A placeholder without an index sorts after every indexed one.
    let index = |element: &Element| element.placeholder().and_then(|p| p.index);
    let mut ranked: Vec<(usize, &Element)> = elements
        .iter()
        .enumerate()
        .filter(|(_, e)| shown(e))
        .collect();
    ranked.sort_by(|(za, a), (zb, b)| {
        rank(a)
            .cmp(&rank(b))
            .then_with(|| {
                if rank(a) != 1 {
                    return Ordering::Equal;
                }
                match (index(a), index(b)) {
                    (Some(x), Some(y)) => x.cmp(&y),
                    (Some(_), None) => Ordering::Less,
                    (None, Some(_)) => Ordering::Greater,
                    (None, None) => Ordering::Equal,
                }
            })
            .then_with(|| za.cmp(zb))
    });
    ranked.into_iter().map(|(_, e)| e.id().clone()).collect()
}

impl SlidePass for A11y {
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
        let title = title_of(&slide.elements)?;
        if title.is_empty() {
            ctx.diag.report(
                codes::MID5801,
                DiagnosticSite::new().container(slide.id.clone()),
            );
        }
        let mut out = slide.clone();
        out.title = Some(title);
        out.reading_order = Some(reading_order_of(&slide.elements));
        Ok(OwnedContainer::Slide(out))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapses_js_whitespace() {
        assert_eq!(collapse("  big \u{a0}\u{2003} x\n\ty  "), "big x y");
        assert_eq!(collapse("\u{feff}"), "");
        assert_eq!(collapse("a"), "a");
    }
}
