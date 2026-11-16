//! Magic Move match keys. Keynote stores no correspondence between
//! the objects of two slides, and a duplicated slide copies its drawables under new ids: it matches objects by
//! content. A slide that leaves by Magic Move, and the slide shown after it, give each top-level element a key
//! derived from its content:
//! - a picture or a medium: its media;
//! - a shape with text: the text (with Magic Move's object match, the text must be equal);
//! - any other shape: its geometry scaled to a unit box (the same shape at another size matches) and its fill;
//! - a group: its children's contents, in order.
//!
//! The player morphs the elements of the two slides that share a key, one to one; an unkeyed
//! element, or one whose key the other slide lacks, fades, as Keynote fades unmatched objects. Copies of one content
//! stacked in one place (the same frame) are one candidate, since any of them morphs alike: the top one (painted
//! last) is keyed and the copies under it fade. Copies of one content in `n` places (the boxes and arrows of a
//! diagram) are keyed by their place in painting order, `k` of `n`, so two slides that hold as many copies pair them
//! in that order, as Keynote moves them (the panel of boxes and arrows of a talk's Magic Moves); slides that hold
//! another count of copies pair none of them. Tables and foreign objects get no key. A key depends only on its
//! slide, so a slide lowered alone keys as in the whole presentation.

use slidock_ir::frame::Frame;
use slidock_ir::geometry::{Geometry, Seg};
use slidock_ir::source::Element;

use super::effects::MAGIC_MOVE;

/// The prefix of a Keynote match key.
const PREFIX: &str = "kn:";
/// Hex digits of the content hash a key keeps.
const HASH_DIGITS: usize = 24;
/// Group nesting a signature descends; a deeper group gets no key.
const MAX_DEPTH: usize = 32;

/// Whether a transition effect is Magic Move.
pub fn is_magic_move(effect: &str) -> bool {
    MAGIC_MOVE.contains(&effect)
}

fn text_of(element: &slidock_ir::source::Shape) -> Option<String> {
    let body = element.text.as_ref()?;
    let paragraphs: Vec<String> = body
        .paragraphs
        .iter()
        .map(|paragraph| {
            paragraph
                .inlines
                .iter()
                .filter_map(|inline| match inline {
                    slidock_ir::text::Inline::Run { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect()
        })
        .collect();
    let text = paragraphs.join("\n");
    (!text.trim().is_empty()).then_some(text)
}

/// A path's segments with each coordinate scaled by its box's size (kept as is along a side of size 0), and its
/// fill and stroke flags.
fn unit_path<C: Copy>(
    out: &mut String,
    segments: &[Seg<C>],
    size: (f64, f64),
    value: impl Fn(C) -> f64,
    flags: (bool, bool),
) {
    out.push('|');
    for segment in segments {
        out.push_str(segment.command());
        for (i, coord) in segment.coords().into_iter().enumerate() {
            let side = if i % 2 == 0 { size.0 } else { size.1 };
            let v = value(coord);
            let unit = if side > 0.0 { v / side } else { v };
            out.push(' ');
            out.push_str(&slidock_canon::to_js_string(slidock_math::quantise(
                unit,
                slidock_canon::Grid::HundredThousandths,
            )));
        }
    }
    out.push_str(if flags.0 { " f" } else { "" });
    out.push_str(if flags.1 { " s" } else { "" });
}

/// A geometry scaled to a unit box, so the same shape at another size keeps its signature: frame-local paths by
/// the frame, custom paths by their own box; a preset by its name and adjustments.
fn unit_geometry(geometry: Option<&Geometry>, frame: (f64, f64)) -> String {
    let mut out = String::new();
    match geometry {
        None => out.push_str("none"),
        Some(Geometry::Preset(preset)) => {
            out.push_str(&slidock_ir::canonical::to_json(preset).unwrap_or_default());
        }
        Some(Geometry::Custom(custom)) => {
            out.push_str("custom");
            for path in &custom.paths {
                unit_path(
                    &mut out,
                    &path.segments,
                    (path.w.to_f64(), path.h.to_f64()),
                    |c| c.to_f64(),
                    (path.fill, path.stroke),
                );
            }
        }
        Some(Geometry::Paths(paths)) => {
            out.push_str("paths");
            if let Some(preset) = &paths.preset {
                out.push_str(&slidock_ir::canonical::to_json(preset).unwrap_or_default());
            }
            for path in &paths.paths {
                unit_path(
                    &mut out,
                    &path.segments,
                    frame,
                    |c| c.to_f64(),
                    (path.fill, path.stroke),
                );
            }
        }
    }
    out
}

/// What an element shows, for matching; `None` for what never matches.
fn signature(element: &Element, depth: usize) -> Option<String> {
    if depth > MAX_DEPTH {
        return None;
    }
    match element {
        Element::Picture(picture) => Some(format!("picture {}", picture.media)),
        Element::Media(media) => media.media.as_ref().map(|sha| format!("media {sha}")),
        Element::Shape(shape) => Some(match text_of(shape) {
            Some(text) => format!("text {text}"),
            None => {
                let frame = shape
                    .frame
                    .as_ref()
                    .map_or((0.0, 0.0), |frame| (frame.w.to_f64(), frame.h.to_f64()));
                format!(
                    "shape {} {}",
                    unit_geometry(shape.geometry.as_ref(), frame),
                    slidock_ir::canonical::to_json(&shape.fill).unwrap_or_default()
                )
            }
        }),
        Element::Group(group) => {
            let children: Option<Vec<String>> = group
                .children
                .iter()
                .map(|child| signature(child, depth + 1))
                .collect();
            Some(format!("group [{}]", children?.join("\u{0}")))
        }
        Element::Table(_) | Element::Foreign(_) => None,
    }
}

/// The match key of a signature: a hash, so keys are short whatever the content.
fn key_of(signature: &str) -> String {
    let hash = slidock_canon::sha256_hex(signature.as_bytes());
    format!("{PREFIX}{}", hash.get(..HASH_DIGITS).unwrap_or(&hash))
}

/// Keys every top-level element: the top copy of each place its content takes on the slide (copies stacked in one
/// frame are one place), by its content alone when the content takes one place, else by its content and the place's
/// rank in painting order (`k` of `n`).
pub fn assign_keys(elements: &mut [Element]) {
    let candidates: Vec<Option<(String, Option<Frame>)>> = elements
        .iter()
        .map(|element| signature(element, 0).map(|signature| (signature, element.frame().clone())))
        .collect();
    // Each candidate that is the top copy of its place: no later candidate of its content in its frame.
    let top: Vec<bool> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            candidate.as_ref().is_some_and(|(signature, frame)| {
                !candidates
                    .iter()
                    .skip(index + 1)
                    .flatten()
                    .any(|(s, f)| s == signature && f == frame)
            })
        })
        .collect();
    let keys: Vec<Option<String>> = candidates
        .iter()
        .enumerate()
        .map(|(index, candidate)| {
            let (signature, _) = candidate.as_ref()?;
            if !top.get(index).copied().unwrap_or(false) {
                return None;
            }
            let places = |range: std::ops::Range<usize>| {
                range
                    .filter(|&i| {
                        top.get(i).copied().unwrap_or(false)
                            && candidates
                                .get(i)
                                .and_then(Option::as_ref)
                                .is_some_and(|(s, _)| s == signature)
                    })
                    .count()
            };
            let (rank, count) = (places(0..index), places(0..candidates.len()));
            Some(if count == 1 {
                key_of(signature)
            } else {
                key_of(&format!("{signature}\u{0}{rank}/{count}"))
            })
        })
        .collect();
    for (element, key) in elements.iter_mut().zip(keys) {
        *element.match_key_mut() = key;
    }
}

/// The keys two slides share: the elements Magic Move morphs.
pub fn shared_keys(from: &[Element], to: &[Element]) -> usize {
    let keys = |elements: &[Element]| -> std::collections::BTreeSet<String> {
        elements
            .iter()
            .filter_map(|element| element.match_key().map(str::to_owned))
            .collect()
    };
    keys(from).intersection(&keys(to)).count()
}
