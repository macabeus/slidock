//! lower-geometry: everything a backend draws as frame-local values it prints directly, free of any source format's
//! conventions.
//! - Geometry becomes paths in points: presets through the ECMA guide interpreter, custom paths scaled from their
//!   own units to the frame, an absent geometry as the rect preset. The preset name stays as provenance.
//! - A stroke's line ends become extra `line-end` paths, and the line stops where an arrowhead begins.
//! - Source gradients become functions of the unrotated, unflipped box's fractions (`lower_gradient`); what they
//!   approximate is reported with the format's gradient code.
//! - A picture's clip lowers like a shape's geometry.
//! - A table cell's fill lowers to a form that holds at whatever height its rows grow to: an unscaled linear
//!   gradient keeps its angle, which holds in the box as drawn; the other forms are fractions of the declared cell,
//!   which stretch with it.
//! - Text gets its turn, for a shape's text and every table cell's: 180 under a vertical flip, unless the text flips
//!   with its frame (then the frame's flips draw it, and it takes none).

use indexmap::IndexMap;
use slidock_core_api::{ContainerView, OwnedContainer, PassCtx, PassError, PassInfo, SlidePass};
use slidock_geometry::path::{ShapePath, map_segment, scale_segment};
use slidock_geometry::{StrokeEnds, js_max, js_min, preset_paths, with_line_ends};
use slidock_ir::diagnostics::DiagnosticSite;
use slidock_ir::frame::Frame;
use slidock_ir::geometry::{
    FillRule, Geometry, Path, PathRole, PathsGeometry, PathsTag, PresetName, Segment,
};
use slidock_ir::ids::NodeId;
use slidock_ir::paint::{CellFill, Fill, LinearAngled, SourceStroke, StrokePaint};
use slidock_ir::source::{Element, Shape, Table};
use slidock_ir::text::{TextBody, TextRotation};
use slidock_ir::units::{Angle, Pt, Size};
use slidock_ir::verify::{Invariant, framed};

use crate::container::{background_mut, unit_error, with_mapped_elements};
use crate::lower_gradient::{Approximation, box_angle, lower_fill, odf_angled};

pub struct LowerGeometry;

static INFO: PassInfo = PassInfo::new(
    "lower-geometry",
    1,
    &[Invariant::ThemeResolved],
    &[Invariant::GeometryLowered],
);

impl SlidePass for LowerGeometry {
    fn info(&self) -> &PassInfo {
        &INFO
    }

    fn run(
        &self,
        view: &ContainerView<'_>,
        ctx: &mut PassCtx<'_>,
    ) -> Result<OwnedContainer, PassError> {
        let mut notes: Notes = Vec::new();
        let mut out = with_mapped_elements(view.container, |element| {
            lower_element(element, &mut notes).map(Some)
        })?;
        let box_frame = Frame {
            x: Pt::ZERO,
            y: Pt::ZERO,
            w: view.size.w,
            h: view.size.h,
            rotation: Angle::ZERO,
            flip_h: false,
            flip_v: false,
        };
        let background = background_mut(&mut out);
        if let Some(fill) = background.as_ref() {
            let lowered = lower_fill(fill, &box_frame, None)?;
            notes.extend(lowered.approximations.into_iter().map(|note| (None, note)));
            *background = Some(lowered.fill);
        }
        let container = view.container.id().clone();
        for (node, note) in notes {
            ctx.diag.report(
                note.code,
                DiagnosticSite::new()
                    .container(container.clone())
                    .node(node)
                    .feature(note.feature)
                    .data("reason", note.reason),
            );
        }
        Ok(out)
    }
}

/// What lowering approximated, by element (none for the background).
type Notes = Vec<(Option<NodeId>, Approximation)>;

/// The most a lowered coordinate reaches from the origin.
const LIMIT: f64 = 100_000.0;

fn pt(x: f64) -> Result<Pt, PassError> {
    Pt::quantise(js_min(LIMIT, js_max(-LIMIT, x))).map_err(|e| unit_error("A path point", e))
}

fn quantised(segment: slidock_geometry::path::Segment) -> Result<Segment, PassError> {
    let mut failure = None;
    let out = map_segment(segment, |x| {
        pt(x).unwrap_or_else(|error| {
            failure.get_or_insert(error);
            Pt::ZERO
        })
    });
    match failure {
        Some(error) => Err(error),
        None => Ok(out),
    }
}

/// A traced path and the fill rule it keeps.
struct Traced {
    path: ShapePath,
    fill_rule: FillRule,
}

/// A source geometry's paths in a `w` × `h` box, before quantisation.
fn trace(geometry: &Geometry, w: f64, h: f64) -> Vec<Traced> {
    match geometry {
        Geometry::Preset(preset) => preset_paths(preset.name, w, h, &preset.adjust)
            .into_iter()
            .map(|path| Traced {
                path,
                fill_rule: FillRule::Nonzero,
            })
            .collect(),
        Geometry::Custom(custom) => custom
            .paths
            .iter()
            .map(|path| {
                let (pw, ph) = (path.w.to_f64(), path.h.to_f64());
                let sx = if pw > 0.0 { w / pw } else { 1.0 };
                let sy = if ph > 0.0 { h / ph } else { 1.0 };
                let segments = path
                    .segments
                    .iter()
                    .map(|segment| scale_segment(map_segment(*segment, |v| v.to_f64()), sx, sy))
                    .collect();
                Traced {
                    path: ShapePath {
                        segments,
                        fill: path.fill,
                        stroke: path.stroke,
                    },
                    fill_rule: path.fill_rule,
                }
            })
            .collect(),
        Geometry::Paths(_) => Vec::new(),
    }
}

fn rect() -> Option<Geometry> {
    PresetName::parse("rect").map(|name| {
        Geometry::Preset(slidock_ir::geometry::PresetGeometry {
            kind: slidock_ir::geometry::PresetTag,
            name,
            adjust: IndexMap::new(),
        })
    })
}

fn lowered_path(
    path: &ShapePath,
    fill_rule: FillRule,
    role: Option<PathRole>,
) -> Result<Path, PassError> {
    Ok(Path {
        segments: path
            .segments
            .iter()
            .map(|segment| quantised(*segment))
            .collect::<Result<_, _>>()?,
        fill: path.fill,
        stroke: path.stroke,
        fill_rule,
        role,
    })
}

/// A geometry (a rectangle when absent) as paths in the frame, with the stroke's line ends.
fn lower_paths(
    geometry: Option<&Geometry>,
    frame: &Frame,
    stroke: Option<&SourceStroke>,
) -> Result<Geometry, PassError> {
    let fallback = rect();
    let Some(source) = geometry.or(fallback.as_ref()) else {
        return Err(PassError::failed("The rect preset is missing"));
    };
    if let Geometry::Paths(_) = source {
        return Ok(source.clone());
    }
    let traced = trace(source, frame.w.to_f64(), frame.h.to_f64());
    let drawn = stroke.filter(|stroke| !matches!(stroke.fill, StrokePaint::None));
    let shapes: Vec<ShapePath> = traced.iter().map(|t| t.path.clone()).collect();
    let (paths, ends) = match drawn {
        Some(stroke) => {
            let ends = StrokeEnds {
                width: stroke.width.to_f64(),
                head: stroke.head.as_ref(),
                tail: stroke.tail.as_ref(),
            };
            let out = with_line_ends(&shapes, ends);
            (out.paths, out.ends)
        }
        None => (shapes, Vec::new()),
    };
    let mut out = Vec::with_capacity(paths.len() + ends.len());
    for (path, traced) in paths.iter().zip(&traced) {
        out.push(lowered_path(path, traced.fill_rule, None)?);
    }
    for end in &ends {
        out.push(lowered_path(
            end,
            FillRule::Nonzero,
            Some(PathRole::LineEnd),
        )?);
    }
    Ok(Geometry::Paths(PathsGeometry {
        kind: PathsTag,
        paths: out,
        preset: match source {
            Geometry::Preset(preset) => Some(preset.name),
            _ => None,
        },
    }))
}

/// Text that flips with its frame takes no turn of its own. Other text stays readable (DrawingML): a vertical flip
/// turns it upside down, a horizontal one leaves it as is.
fn text_rotation(text: &TextBody, frame: &Frame) -> TextRotation {
    if frame.flip_v && !text.flips_with_frame {
        TextRotation::Half
    } else {
        TextRotation::Zero
    }
}

fn lower_shape(
    shape: &Shape,
    frame: &Frame,
    notes: &mut Vec<Approximation>,
) -> Result<Shape, PassError> {
    let mut out = shape.clone();
    let stroke = shape.stroke.as_ref().and_then(Option::as_ref);
    let geometry = lower_paths(shape.geometry.as_ref(), frame, stroke)?;
    if let Some(fill) = &shape.fill {
        let outline = match &geometry {
            Geometry::Paths(paths) => Some(paths.paths.as_slice()),
            _ => None,
        };
        let lowered = lower_fill(fill, frame, outline)?;
        notes.extend(lowered.approximations);
        out.fill = Some(lowered.fill);
    }
    out.geometry = Some(geometry);
    if let Some(text) = out.text.as_mut() {
        text.rotation = Some(text_rotation(text, frame));
    }
    Ok(out)
}

/// A cell's fill in a form free of the cell's height, which its text decides as it is drawn: a linear gradient by
/// its angle (unscaled DrawingML with neither outset nor tile, and ODF's smooth linear and axial ones), the rest as
/// [`lower_fill`] makes them against the declared cell size, as fractions that stretch with the cell.
fn lower_cell_fill(
    fill: &CellFill,
    cell: &Frame,
    notes: &mut Vec<Approximation>,
) -> Result<CellFill, PassError> {
    match fill {
        CellFill::Fill(Fill::LinearAngle(linear))
            if !linear.scaled && linear.outset.is_none() && linear.tile.is_none() =>
        {
            Ok(CellFill::Angled(LinearAngled {
                angle: box_angle(linear.angle, linear.rotate_with_shape, cell)?,
                stops: linear.stops.clone(),
            }))
        }
        CellFill::Fill(fill) => {
            if let Fill::Odf(odf) = fill
                && let Some(angled) = odf_angled(odf, cell, notes)?
            {
                return Ok(CellFill::Angled(angled));
            }
            let lowered = lower_fill(fill, cell, None)?;
            notes.extend(lowered.approximations);
            Ok(CellFill::Fill(lowered.fill))
        }
        CellFill::Angled(_) => Ok(fill.clone()),
    }
}

/// A cell's declared box: its spanned columns and rows, placed and turned as the table is.
fn cell_frame(
    table: &Table,
    frame: &Frame,
    row: usize,
    column: usize,
    rows: u32,
    columns: u32,
) -> Frame {
    let sum = |sizes: &mut dyn Iterator<Item = f64>| sizes.sum::<f64>();
    let w = sum(&mut table
        .columns
        .iter()
        .skip(column)
        .take(columns as usize)
        .map(|c| c.width.to_f64()));
    let h = sum(&mut table
        .rows
        .iter()
        .skip(row)
        .take(rows as usize)
        .map(|r| r.height.to_f64()));
    Frame {
        w: Size::quantise(w).unwrap_or(frame.w),
        h: Size::quantise(h).unwrap_or(frame.h),
        ..frame.clone()
    }
}

fn lower_table(
    table: &Table,
    frame: &Frame,
    notes: &mut Vec<Approximation>,
) -> Result<Table, PassError> {
    let mut out = table.clone();
    for (r, row) in out.rows.iter_mut().enumerate() {
        for (c, cell) in row.cells.iter_mut().enumerate() {
            let Some(cell) = cell else { continue };
            if let Some(fill) = &cell.fill {
                let spans = (cell.row_span.unwrap_or(1), cell.col_span.unwrap_or(1));
                let declared = cell_frame(table, frame, r, c, spans.0, spans.1);
                cell.fill = Some(lower_cell_fill(fill, &declared, notes)?);
            }
            cell.text.rotation = Some(text_rotation(&cell.text, frame));
        }
    }
    Ok(out)
}

fn lower_element(element: &Element, notes: &mut Notes) -> Result<Element, PassError> {
    let mut own = Vec::new();
    let out = match element {
        Element::Shape(shape) => Element::from(lower_shape(shape, framed(element)?, &mut own)?),
        Element::Table(table) => Element::from(lower_table(table, framed(element)?, &mut own)?),
        Element::Picture(picture) if picture.clip.is_some() => {
            let frame = framed(element)?;
            let mut out = picture.clone();
            out.clip = Some(lower_paths(picture.clip.as_ref(), frame, None)?);
            Element::Picture(out)
        }
        other => other.clone(),
    };
    notes.extend(
        own.into_iter()
            .map(|note| (Some(element.id().clone()), note)),
    );
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_points_to_the_coordinate_range_and_refuses_nan() {
        assert_eq!(pt(1e9).unwrap().to_f64(), 100_000.0);
        assert_eq!(pt(-1e9).unwrap().to_f64(), -100_000.0);
        assert!(pt(f64::NAN).is_err());
    }
}
