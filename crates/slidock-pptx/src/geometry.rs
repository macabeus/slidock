//! Placement and outline: `a:xfrm` (offset, extent, rotation, flips; a group's child space) and a shape's geometry, a
//! preset with its adjustments or a custom path list. Custom guides evaluate here, through the same interpreter as the
//! presets (`slidock-geometry`), into path coordinates; arcs become cubics. lower-geometry scales the paths to the
//! frame.

use indexmap::IndexMap;
use slidock_canon::js_trim;
use slidock_core_api::lowering::LowerError;
use slidock_geometry::{FormulaError, Guides, Point, arc_to};
use slidock_ir::frame::{ChildSpace, Frame};
use slidock_ir::geometry::{
    CustomGeometry, CustomPath, CustomTag, FillRule, Geometry, MAX_PATH_SEGMENTS, PathUnitSegment,
    PresetGeometry, PresetName, PresetTag, Seg,
};
use slidock_ir::units::PathUnit;

use crate::units::{angle, emu_to_pt, emu_to_size};
use crate::xml::{El, Element, Ns};

/// An `a:xfrm` (or `p:xfrm`) as a frame; `None` when it lacks the offset or the extent.
pub(crate) fn read_frame(xfrm: Option<&Element>) -> Option<Frame> {
    let off = xfrm.child(Ns::A, "off")?;
    let ext = xfrm.child(Ns::A, "ext")?;
    Some(Frame {
        x: emu_to_pt(off.int_attr("x").unwrap_or(0)),
        y: emu_to_pt(off.int_attr("y").unwrap_or(0)),
        w: emu_to_size(ext.int_attr("cx").unwrap_or(0)),
        h: emu_to_size(ext.int_attr("cy").unwrap_or(0)),
        rotation: angle(xfrm.int_attr("rot").unwrap_or(0)),
        flip_h: xfrm.bool_attr("flipH").unwrap_or(false),
        flip_v: xfrm.bool_attr("flipV").unwrap_or(false),
    })
}

/// A group's child space (`a:chOff`, `a:chExt`), else the group's own box.
pub(crate) fn read_child_space(
    xfrm: Option<&Element>,
    frame: Option<&Frame>,
) -> Option<ChildSpace> {
    let (Some(off), Some(ext)) = (xfrm.child(Ns::A, "chOff"), xfrm.child(Ns::A, "chExt")) else {
        return frame.map(|frame| ChildSpace {
            x: frame.x,
            y: frame.y,
            w: frame.w,
            h: frame.h,
        });
    };
    Some(ChildSpace {
        x: emu_to_pt(off.int_attr("x").unwrap_or(0)),
        y: emu_to_pt(off.int_attr("y").unwrap_or(0)),
        w: emu_to_size(ext.int_attr("cx").unwrap_or(0)),
        h: emu_to_size(ext.int_attr("cy").unwrap_or(0)),
    })
}

/// `a:avLst`/`a:gdLst` guides as (name, formula) pairs.
fn guide_list<'a>(list: impl El<'a>) -> Vec<(&'a str, &'a str)> {
    list.children(Ns::A, "gd")
        .into_iter()
        .filter_map(|gd| {
            let name = gd.attr("name").filter(|name| !name.is_empty())?;
            let formula = gd.attr("fmla").filter(|formula| !formula.is_empty())?;
            Some((name, formula))
        })
        .collect()
}

fn is_js_space(c: char) -> bool {
    slidock_canon::is_js_whitespace(c)
}

/// `/^val\s+(-?\d+(?:\.\d+)?)$/` on a trimmed formula: the literal value.
fn literal_value(formula: &str) -> Option<f64> {
    let rest = js_trim(formula).strip_prefix("val")?;
    let value = rest.trim_start_matches(is_js_space);
    if value.len() == rest.len() {
        return None;
    }
    let digits = value.strip_prefix('-').unwrap_or(value);
    let (whole, fraction) = match digits.split_once('.') {
        Some((whole, fraction)) => (whole, Some(fraction)),
        None => (digits, None),
    };
    let decimal = |part: &str| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit());
    if !decimal(whole) || fraction.is_some_and(|fraction| !decimal(fraction)) {
        return None;
    }
    value.parse().ok()
}

fn is_guide_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    matches!(bytes.first(), Some(b) if b.is_ascii_alphabetic())
        && bytes.len() <= 32
        && bytes.iter().all(u8::is_ascii_alphanumeric)
}

/// A preset's adjustments: only literal values (`val N`) override the preset's defaults.
fn adjustments(av_lst: Option<&Element>) -> IndexMap<String, f64> {
    let mut out = IndexMap::new();
    for (name, formula) in guide_list(av_lst) {
        if let Some(value) = literal_value(formula).filter(|_| is_guide_name(name)) {
            out.insert(name.to_owned(), value.clamp(-1e9, 1e9));
        }
    }
    out
}

const PATH_UNIT_LIMIT: f64 = 1e9;

/// A path coordinate on the IR's grid, clamped to its range; `None` for NaN, which has no IR value.
fn unit(x: f64) -> Option<PathUnit> {
    let clamped = x.clamp(-PATH_UNIT_LIMIT, PATH_UNIT_LIMIT);
    let steps = slidock_canon::grid_steps(clamped, PathUnit::GRID).ok()?;
    Some(PathUnit::from_steps(steps))
}

/// A custom path coordinate that is not a number (`sqrt` of a negative guide, `0 * Infinity`): where it sits in the
/// geometry's JSON, `paths[path].segments[segment][index]`. See [`crate::not_numbers`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct NotANumber {
    pub path: usize,
    pub segment: usize,
    pub index: usize,
}

/// A segment on the IR's grid; a NaN coordinate reads as 0 and is recorded in `nan`.
fn on_grid(segment: Seg<f64>, at: (usize, usize), nan: &mut Vec<NotANumber>) -> PathUnitSegment {
    let mut coordinate = |index: usize, x: f64| {
        unit(x).unwrap_or_else(|| {
            nan.push(NotANumber {
                path: at.0,
                segment: at.1,
                index: index + 1,
            });
            PathUnit::from_steps(0)
        })
    };
    match segment {
        Seg::M(x, y) => Seg::M(coordinate(0, x), coordinate(1, y)),
        Seg::L(x, y) => Seg::L(coordinate(0, x), coordinate(1, y)),
        Seg::Q(a, b, c, d) => Seg::Q(
            coordinate(0, a),
            coordinate(1, b),
            coordinate(2, c),
            coordinate(3, d),
        ),
        Seg::C(a, b, c, d, e, f) => Seg::C(
            coordinate(0, a),
            coordinate(1, b),
            coordinate(2, c),
            coordinate(3, d),
            coordinate(4, e),
            coordinate(5, f),
        ),
        Seg::Z => Seg::Z,
    }
}

/// Why a custom geometry stops: a formula it cannot evaluate or a required attribute it lacks (the shape is unknown),
/// or a limit (its container fails).
enum PathError {
    Formula(FormulaError),
    Missing(&'static str),
    Limit(LowerError),
}

impl From<FormulaError> for PathError {
    fn from(error: FormulaError) -> Self {
        PathError::Formula(error)
    }
}

/// One `a:path` of a custom geometry (the `index`th), evaluated against the guides, in the path's own units.
fn custom_path(
    path: &Element,
    index: usize,
    guides: &Guides,
    size: (i64, i64),
    budget: &mut i64,
    nan: &mut Vec<NotANumber>,
) -> Result<CustomPath, PathError> {
    let w = path.int_attr("w").unwrap_or(size.0);
    let h = path.int_attr("h").unwrap_or(size.1);
    // Every coordinate and arc measure is a required attribute: one that is missing is not read as 0.
    let value = |element: &Element, name: &'static str| -> Result<f64, PathError> {
        let token = element.attr(name).ok_or(PathError::Missing(name))?;
        Ok(guides.value_of(token)?)
    };
    let point = |element: &Element| -> Result<Point, PathError> {
        let pt = element.child(Ns::A, "pt").map_or(element, |rc| &**rc);
        Ok(Point::new(value(pt, "x")?, value(pt, "y")?))
    };
    let mut segments: Vec<PathUnitSegment> = Vec::new();
    let mut push = |segments: &mut Vec<PathUnitSegment>, segment: Seg<f64>| {
        *budget -= 1;
        if *budget < 0 || segments.len() >= MAX_PATH_SEGMENTS {
            return Err(PathError::Limit(LowerError::Limit(
                "a shape holds too many path segments".to_owned(),
            )));
        }
        segments.push(on_grid(segment, (index, segments.len()), nan));
        Ok(())
    };
    let mut current = Point::default();
    let mut start = current;
    for command in path.kids() {
        match &*command.local {
            "moveTo" => {
                current = point(command)?;
                start = current;
                push(&mut segments, Seg::M(current.x, current.y))?;
            }
            "lnTo" => {
                current = point(command)?;
                push(&mut segments, Seg::L(current.x, current.y))?;
            }
            "quadBezTo" => {
                let points = command
                    .children(Ns::A, "pt")
                    .map(|pt| point(pt))
                    .collect::<Result<Vec<_>, _>>()?;
                let [c, end, ..] = points.as_slice() else {
                    continue;
                };
                push(&mut segments, Seg::Q(c.x, c.y, end.x, end.y))?;
                current = *end;
            }
            "cubicBezTo" => {
                let points = command
                    .children(Ns::A, "pt")
                    .map(|pt| point(pt))
                    .collect::<Result<Vec<_>, _>>()?;
                let [c1, c2, end, ..] = points.as_slice() else {
                    continue;
                };
                push(&mut segments, Seg::C(c1.x, c1.y, c2.x, c2.y, end.x, end.y))?;
                current = *end;
            }
            "arcTo" => {
                let arc = arc_to(
                    current,
                    value(command, "wR")?,
                    value(command, "hR")?,
                    value(command, "stAng")?,
                    value(command, "swAng")?,
                );
                for [x1, y1, x2, y2, x, y] in arc.cubics {
                    push(&mut segments, Seg::C(x1, y1, x2, y2, x, y))?;
                }
                current = arc.end;
            }
            "close" => {
                push(&mut segments, Seg::Z)?;
                current = start;
            }
            _ => {}
        }
    }
    let size = |extent: i64| unit((extent as f64).max(0.0)).unwrap_or(PathUnit::from_steps(0));
    Ok(CustomPath {
        w: size(w),
        h: size(h),
        fill: path.attr("fill").unwrap_or("norm") != "none",
        stroke: Some(path).bool_attr("stroke").unwrap_or(true),
        fill_rule: FillRule::Nonzero,
        segments,
    })
}

/// The geometry a shape's properties read as.
pub(crate) enum ReadGeometry {
    /// A preset, or a custom geometry with the coordinates that are not numbers (see [`NotANumber`]).
    Geometry(Geometry, Vec<NotANumber>),
    None,
    /// A preset name outside the standard's list, or a custom geometry that fails to evaluate.
    Unknown(String),
}

/// The geometry of a shape's properties. `size` is the shape's extent in EMUs, which custom guides measure against
/// (`w`, `h`, `ss`); `segments` caps the path segments of one shape.
pub(crate) fn read_geometry(
    sp_pr: Option<&Element>,
    size: (i64, i64),
    segments: usize,
) -> Result<ReadGeometry, LowerError> {
    if let Some(preset) = sp_pr.child(Ns::A, "prstGeom") {
        let name = preset.attr("prst").unwrap_or("");
        let Some(parsed) = PresetName::parse(name) else {
            return Ok(ReadGeometry::Unknown(name.to_owned()));
        };
        let preset = Geometry::Preset(PresetGeometry {
            kind: PresetTag,
            name: parsed,
            adjust: adjustments(preset.child(Ns::A, "avLst").map(|rc| &**rc)),
        });
        return Ok(ReadGeometry::Geometry(preset, Vec::new()));
    }
    let Some(custom) = sp_pr.child(Ns::A, "custGeom") else {
        return Ok(ReadGeometry::None);
    };
    let mut nan = Vec::new();
    let mut read = || -> Result<Vec<CustomPath>, PathError> {
        let no_overrides = IndexMap::new();
        let guides = Guides::builtin(size.0 as f64, size.1 as f64)
            .with_guides(&guide_list(custom.child(Ns::A, "avLst")), &no_overrides)?
            .with_guides(&guide_list(custom.child(Ns::A, "gdLst")), &no_overrides)?;
        let mut budget = i64::try_from(segments).unwrap_or(i64::MAX);
        custom
            .child(Ns::A, "pathLst")
            .children(Ns::A, "path")
            .into_iter()
            .take(256)
            .enumerate()
            .map(|(index, path)| custom_path(path, index, &guides, size, &mut budget, &mut nan))
            .collect()
    };
    match read() {
        Ok(paths) if paths.is_empty() => Ok(ReadGeometry::None),
        Ok(paths) => Ok(ReadGeometry::Geometry(
            Geometry::Custom(CustomGeometry {
                kind: CustomTag,
                paths,
            }),
            nan,
        )),
        Err(PathError::Formula(error)) => Ok(ReadGeometry::Unknown(error.to_string())),
        Err(PathError::Missing(name)) => Ok(ReadGeometry::Unknown(format!(
            "a path point or arc has no {name}"
        ))),
        Err(PathError::Limit(error)) => Err(error),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_only_literal_adjustments() {
        assert_eq!(literal_value(" val 30000 "), Some(30_000.0));
        assert_eq!(literal_value("val\t-12.5"), Some(-12.5));
        for formula in [
            "val30000", "val 1.", "val .5", "val +1", "*/ w 1 2", "val 1e3", "val 1 2",
        ] {
            assert_eq!(literal_value(formula), None, "{formula}");
        }
        assert!(is_guide_name("adj1") && !is_guide_name("1adj") && !is_guide_name("a_b"));
    }
}
