//! normalize-frames: every frame in the canonical form, relative to its parent's box. A group's child space (OOXML
//! chOff/chExt) scales and offsets its children into the group's own box, composed down nested groups, and goes; a
//! table's grid scales with its frame, so its columns and rows still add up to it. A group's flips move down to its
//! children, mirrored in the group's box, so every element carries the flips it is drawn with: text that does not flip
//! with its frame then undoes exactly the flips of its own shape.

use slidock_core_api::{ContainerView, OwnedContainer, PassCtx, PassError, PassInfo, SlidePass};
use slidock_ir::frame::{ChildSpace, Frame};
use slidock_ir::source::{Element, Table};
use slidock_ir::units::{Angle, Pt, Size};
use slidock_ir::verify::Invariant;

use crate::container::{elements_mut, unit_error};

pub struct NormalizeFrames;

static INFO: PassInfo = PassInfo::new(
    "normalize-frames",
    1,
    &[Invariant::SourceValid],
    &[Invariant::FramesCanonical],
);

impl SlidePass for NormalizeFrames {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        view: &ContainerView<'_>,
        _ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError> {
        let mut out = view.unchanged();
        let elements = normalize(view.container.elements(), None)?;
        *elements_mut(&mut out) = elements;
        Ok(out)
    }
}

/// How a group's child coordinates map into its box: subtract the offset, then scale.
#[derive(Clone, Copy, Debug)]
struct Mapping {
    x: f64,
    y: f64,
    sx: f64,
    sy: f64,
}

fn pt(x: f64) -> Result<Pt, PassError> {
    Pt::quantise(x).map_err(|e| unit_error("A frame leaves the point grid", e))
}

fn size(x: f64) -> Result<Size, PassError> {
    Size::quantise(x).map_err(|e| unit_error("A frame leaves the point grid", e))
}

/// `angle` into [0, 360) on the degree grid; 359.9996 rounds up to 360, which is 0.
pub fn normal_angle(angle: f64) -> Result<Angle, PassError> {
    let turned = Angle::quantise(((angle % 360.0) + 360.0) % 360.0)
        .map_err(|e| unit_error("An angle leaves the degree grid", e))?;
    Ok(if turned.to_f64() == 360.0 {
        Angle::ZERO
    } else {
        turned
    })
}

fn map_frame(frame: &Frame, mapping: Option<Mapping>) -> Result<Frame, PassError> {
    let (x, y, w, h) = (
        frame.x.to_f64(),
        frame.y.to_f64(),
        frame.w.to_f64(),
        frame.h.to_f64(),
    );
    let (x, y, w, h) = match mapping {
        Some(m) => ((x - m.x) * m.sx, (y - m.y) * m.sy, w * m.sx, h * m.sy),
        None => (x, y, w, h),
    };
    Ok(Frame {
        x: pt(x)?,
        y: pt(y)?,
        w: size(w)?,
        h: size(h)?,
        rotation: normal_angle(frame.rotation.to_f64())?,
        flip_h: frame.flip_h,
        flip_v: frame.flip_v,
    })
}

/// Track lengths scaled so they still add up to `total`, the scaled length of them all: the grid lines scale and
/// land on the point grid, and each track spans two of them.
fn scaled_tracks(lengths: &[f64], scale: f64, total: f64) -> Result<Vec<Size>, PassError> {
    let mut at = 0.0;
    let mut line = 0.0;
    let last = lengths.len().saturating_sub(1);
    lengths
        .iter()
        .enumerate()
        .map(|(index, length)| {
            at += length;
            let next = if index == last {
                total
            } else {
                pt(at * scale)?.to_f64()
            };
            let track = size(next - line)?;
            line = next;
            Ok(track)
        })
        .collect()
}

fn scale_table(table: &mut Table, frame: &Frame, mapping: Mapping) -> Result<(), PassError> {
    let widths: Vec<f64> = table.columns.iter().map(|c| c.width.to_f64()).collect();
    let heights: Vec<f64> = table.rows.iter().map(|r| r.height.to_f64()).collect();
    let widths = scaled_tracks(&widths, mapping.sx, frame.w.to_f64())?;
    let heights = scaled_tracks(&heights, mapping.sy, frame.h.to_f64())?;
    for (column, width) in table.columns.iter_mut().zip(widths) {
        column.width = width;
    }
    for (row, height) in table.rows.iter_mut().zip(heights) {
        row.height = height;
    }
    Ok(())
}

/// A child space maps onto the group's box; an empty extent keeps its axis unscaled.
fn mapping_of(space: &ChildSpace, frame: Option<&Frame>) -> Mapping {
    let scale = |outer: Option<f64>, inner: f64| match outer {
        Some(outer) if inner > 0.0 => outer / inner,
        _ => 1.0,
    };
    Mapping {
        x: space.x.to_f64(),
        y: space.y.to_f64(),
        sx: scale(frame.map(|f| f.w.to_f64()), space.w.to_f64()),
        sy: scale(frame.map(|f| f.h.to_f64()), space.h.to_f64()),
    }
}

/// A child of a group that flips, drawn alike without the group's flips: its box mirrored in the group's `w` ×
/// `h`, its own flips toggled, and its rotation reversed (a mirror turns the other way).
fn mirror(element: &mut Element, group: &Frame) -> Result<(), PassError> {
    let Some(frame) = element.frame_mut().as_mut() else {
        return Ok(());
    };
    if group.flip_h {
        frame.x = pt(group.w.to_f64() - frame.x.to_f64() - frame.w.to_f64())?;
    }
    if group.flip_v {
        frame.y = pt(group.h.to_f64() - frame.y.to_f64() - frame.h.to_f64())?;
    }
    if group.flip_h != group.flip_v {
        frame.rotation = normal_angle(-frame.rotation.to_f64())?;
    }
    frame.flip_h = frame.flip_h != group.flip_h;
    frame.flip_v = frame.flip_v != group.flip_v;
    Ok(())
}

fn normalize(elements: &[Element], mapping: Option<Mapping>) -> Result<Vec<Element>, PassError> {
    elements
        .iter()
        .map(|element| normalize_element(element, mapping))
        .collect()
}

fn normalize_element(element: &Element, mapping: Option<Mapping>) -> Result<Element, PassError> {
    let mut out = element.clone();
    let frame = match element.frame() {
        Some(frame) => Some(map_frame(frame, mapping)?),
        None => None,
    };
    out.frame_mut().clone_from(&frame);
    match &mut out {
        Element::Group(group) => {
            let inner = group
                .child_space
                .take()
                .map(|space| mapping_of(&space, frame.as_ref()));
            group.children = normalize(&group.children, inner)?;
            if let Some(frame) = frame.as_ref().filter(|f| f.flip_h || f.flip_v) {
                for child in &mut group.children {
                    mirror(child, frame)?;
                }
                // Mirrored child groups pass their flips on in turn.
                group.children = normalize(&group.children, None)?;
                if let Some(own) = group.frame.as_mut() {
                    own.flip_h = false;
                    own.flip_v = false;
                }
            }
        }
        Element::Table(table) => {
            if let (Some(frame), Some(mapping)) = (frame.as_ref(), mapping) {
                scale_table(table, frame, mapping)?;
            }
        }
        Element::Foreign(foreign) => {
            if let Some(children) = foreign.fallback.as_mut().and_then(|f| f.elements.as_mut()) {
                *children = normalize(children, None)?;
            }
        }
        _ => {}
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalises_angles_onto_the_degree_grid() {
        let deg = |x: f64| normal_angle(x).unwrap().to_f64();
        assert_eq!(deg(-90.0), 270.0);
        assert_eq!(deg(720.5), 0.5);
        assert_eq!(deg(359.9996), 0.0);
        assert_eq!(deg(-0.0), 0.0);
        assert!(normal_angle(f64::NAN).is_err());
    }

    #[test]
    fn scaled_tracks_add_up_to_the_total() {
        let tracks = scaled_tracks(&[33.333, 100.0, 46.667], 1.5, 270.0).unwrap();
        let widths: Vec<f64> = tracks.iter().map(|t| t.to_f64()).collect();
        assert_eq!(widths, [50.0, 150.0, 70.0]);
    }
}
