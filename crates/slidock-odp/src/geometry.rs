//! The outline of each kind of drawable, in the IR's source terms: custom shapes as the preset their type names,
//! rectangles (rounded by their corner radius), ellipses and their sections, cuts and arcs as presets, polylines,
//! polygons and paths as custom paths in their `svg:viewBox` units, lines and connectors as a straight line across
//! their box with the flips that point it from its start to its end.

use indexmap::IndexMap;
use slidock_canon::{js_round, utf16};
use slidock_core_api::lowering::LowerError;
use slidock_ir::frame::Frame;
use slidock_ir::geometry::{
    CustomGeometry, CustomPath, CustomTag, FillRule, Geometry, PathUnitSegment, PresetGeometry,
    PresetName, PresetTag, Seg,
};
use slidock_ir::units::{Angle, PathUnit};

use crate::custom_shape_map::map_custom_shape;
use crate::enhanced::enhanced_geometry;
use crate::graphic::split_list;
use crate::names::{QName, qn};
use crate::odf::{Element, Ns};
use crate::svg_path::{Budget, parse_path_data, parse_points};
use crate::transform::parse_transform;
use crate::units::{angle_deg, length_pt, plain_number, pt, raw_length, size, to_pt};

/// A drawable's outline as read.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum ReadGeometry {
    Geometry {
        geometry: Geometry,
        flip_h: bool,
        flip_v: bool,
        dropped_modifiers: bool,
    },
    /// A custom shape type outside the preset map, or a path missing its view box or points.
    Unknown(String),
    /// A custom shape outside the preset map whose enhanced geometry is not read, and why.
    Unreadable { kind: String, reason: String },
}

fn preset_geometry(name: PresetName, adjust: IndexMap<String, f64>) -> Geometry {
    Geometry::Preset(PresetGeometry {
        kind: PresetTag,
        name,
        adjust,
    })
}

fn preset(name: &str, adjust: &[(&str, f64)]) -> ReadGeometry {
    match PresetName::parse(name) {
        Some(name) => ReadGeometry::Geometry {
            geometry: preset_geometry(
                name,
                adjust.iter().map(|(k, v)| ((*k).to_owned(), *v)).collect(),
            ),
            flip_h: false,
            flip_v: false,
            dropped_modifiers: false,
        },
        None => ReadGeometry::Unknown(name.to_owned()),
    }
}

/// A custom shape's outline with its mirroring: the preset its type names, else (a LibreOffice shape outside the
/// preset map, or one drawn by hand) its own enhanced path and equations, when they are readable.
pub(crate) fn custom_shape_geometry(
    shape: &Element,
    budget: &mut Budget,
) -> Result<ReadGeometry, LowerError> {
    let enhanced = shape.child(Ns::Draw, "enhanced-geometry");
    let kind = enhanced
        .and_then(|e| e.attr(qn!("draw:type")))
        .unwrap_or("non-primitive");
    let flag = |name: QName| enhanced.and_then(|e| e.bool_attr(name)).unwrap_or(false);
    let (flip_h, flip_v) = (
        flag(qn!("draw:mirror-horizontal")),
        flag(qn!("draw:mirror-vertical")),
    );
    if let Some(mapped) =
        map_custom_shape(kind, enhanced.and_then(|e| e.attr(qn!("draw:modifiers"))))
    {
        return Ok(ReadGeometry::Geometry {
            geometry: preset_geometry(mapped.preset, mapped.adjust),
            flip_h,
            flip_v,
            dropped_modifiers: mapped.dropped_modifiers,
        });
    }
    let kind = utf16::clip(kind, 64).to_owned();
    let Some(enhanced) = enhanced.filter(|e| e.attr(qn!("draw:enhanced-path")).is_some()) else {
        return Ok(ReadGeometry::Unknown(kind));
    };
    let frame = (
        length_pt(shape.attr(qn!("svg:width"))).unwrap_or(0.0),
        length_pt(shape.attr(qn!("svg:height"))).unwrap_or(0.0),
    );
    Ok(match enhanced_geometry(enhanced, frame, budget)? {
        Ok(geometry) => ReadGeometry::Geometry {
            geometry,
            flip_h,
            flip_v,
            dropped_modifiers: false,
        },
        Err(reason) => ReadGeometry::Unreadable { kind, reason },
    })
}

/// A rectangle; a corner radius makes it a rounded one, its radius a share of the shorter side.
pub(crate) fn rect_geometry(element: &Element, frame: Option<&Frame>) -> ReadGeometry {
    let radius = length_pt(element.attr(qn!("draw:corner-radius"))).unwrap_or(0.0);
    let side = frame.map_or(0.0, |frame| frame.w.to_f64().min(frame.h.to_f64()));
    if radius <= 0.0 || side <= 0.0 {
        return preset("rect", &[]);
    }
    preset(
        "roundRect",
        &[("adj", js_round(((radius / side) * 100_000.0).min(50_000.0)))],
    )
}

/// An ellipse, or the section, cut or arc between two angles. ODF's angles turn counter-clockwise from 3 o'clock;
/// DrawingML's pie, chord and arc run clockwise, so the sweep from start to end is the clockwise one from −end to
/// −start, in 60000ths of a degree.
pub(crate) fn ellipse_geometry(element: &Element) -> ReadGeometry {
    let name = match element.attr(qn!("draw:kind")).unwrap_or("full") {
        "section" => "pie",
        "cut" => "chord",
        "arc" => "arc",
        _ => return preset("ellipse", &[]),
    };
    let clockwise = |value: Option<&str>, fallback: f64| {
        let degrees = angle_deg(value, 1.0).unwrap_or(fallback);
        js_round(((((360.0 - degrees) % 360.0) + 360.0) % 360.0) * 60_000.0)
    };
    preset(
        name,
        &[
            (
                "adj1",
                clockwise(element.attr(qn!("draw:end-angle")), 360.0),
            ),
            (
                "adj2",
                clockwise(element.attr(qn!("draw:start-angle")), 0.0),
            ),
        ],
    )
}

/// `svg:viewBox` as its origin and size.
fn view_box(element: &Element) -> Option<[f64; 4]> {
    let text = slidock_canon::js_trim(element.attr(qn!("svg:viewBox")).unwrap_or(""));
    let parts: Vec<f64> = split_list(text)
        .into_iter()
        .map(|part| plain_number(Some(part)))
        .collect::<Option<Vec<f64>>>()?;
    let [x, y, w, h] = <[f64; 4]>::try_from(parts).ok()?;
    (w >= 0.0 && h >= 0.0).then_some([x, y, w, h])
}

/// Segments moved so the view box starts at the origin.
fn shifted(segments: Vec<PathUnitSegment>, dx: f64, dy: f64) -> Vec<PathUnitSegment> {
    if dx == 0.0 && dy == 0.0 {
        return segments;
    }
    let x = |c: PathUnit| path_unit(c.to_f64() - dx);
    let y = |c: PathUnit| path_unit(c.to_f64() - dy);
    segments
        .into_iter()
        .map(|segment| match segment {
            Seg::Z => Seg::Z,
            Seg::M(a, b) => Seg::M(x(a), y(b)),
            Seg::L(a, b) => Seg::L(x(a), y(b)),
            Seg::Q(a, b, c, d) => Seg::Q(x(a), y(b), x(c), y(d)),
            Seg::C(a, b, c, d, e, f) => Seg::C(x(a), y(b), x(c), y(d), x(e), y(f)),
        })
        .collect()
}

fn path_unit(x: f64) -> PathUnit {
    PathUnit::quantise(x).unwrap_or(PathUnit::ZERO)
}

/// A polyline, polygon or path in its view box's units.
pub(crate) fn path_geometry(
    element: &Element,
    budget: &mut Budget,
) -> Result<ReadGeometry, LowerError> {
    let Some([x, y, w, h]) = view_box(element) else {
        return Ok(ReadGeometry::Unknown(format!(
            "{} without a view box",
            element.name
        )));
    };
    let local = element.local.as_str();
    let segments = if local == "path" {
        parse_path_data(element.attr(qn!("svg:d")).unwrap_or(""), budget)?
    } else {
        parse_points(
            element.attr(qn!("draw:points")).unwrap_or(""),
            local == "polygon",
            budget,
        )?
    };
    if segments.is_empty() {
        return Ok(ReadGeometry::Unknown(format!(
            "{} without points",
            element.name
        )));
    }
    let path = CustomPath {
        w: path_unit(w),
        h: path_unit(h),
        fill: local != "polyline",
        stroke: true,
        fill_rule: FillRule::Nonzero,
        segments: shifted(segments, x, y),
    };
    Ok(ReadGeometry::Geometry {
        geometry: Geometry::Custom(CustomGeometry {
            kind: CustomTag,
            paths: vec![path],
        }),
        flip_h: false,
        flip_v: false,
        dropped_modifiers: false,
    })
}

/// A line or connector from (`svg:x1`, `svg:y1`) to (`svg:x2`, `svg:y2`): its box and a straight line across it,
/// flipped so it runs from the start to the end; a transform moves both ends.
pub(crate) fn line_geometry(element: &Element, name: &str) -> (Frame, ReadGeometry) {
    let coordinate = |name: QName| raw_length(element.attr(name)).unwrap_or(0.0);
    let (mut x1, mut y1, mut x2, mut y2) = (
        coordinate(qn!("svg:x1")),
        coordinate(qn!("svg:y1")),
        coordinate(qn!("svg:x2")),
        coordinate(qn!("svg:y2")),
    );
    if let Some(m) = parse_transform(element.attr(qn!("draw:transform")).unwrap_or("")) {
        (x1, y1, x2, y2) = (
            m.a * x1 + m.c * y1 + m.e,
            m.b * x1 + m.d * y1 + m.f,
            m.a * x2 + m.c * y2 + m.e,
            m.b * x2 + m.d * y2 + m.f,
        );
    }
    let frame = Frame {
        x: pt(to_pt(x1.min(x2))),
        y: pt(to_pt(y1.min(y2))),
        w: size(to_pt((x2 - x1).abs())),
        h: size(to_pt((y2 - y1).abs())),
        rotation: Angle::ZERO,
        flip_h: x2 < x1,
        flip_v: y2 < y1,
    };
    (frame, preset(name, &[]))
}
