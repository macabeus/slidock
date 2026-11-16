//! Placement: `svg:x`/`svg:y`/`svg:width`/`svg:height` and `draw:transform`. A transform list applies to the shape's
//! box in the order it is written (LibreOffice writes `rotate(a) translate(x y)`: rotate the box about its own origin,
//! then move it there), and `svg:x`/`svg:y` move the result. `rotate(a)` takes radians, counter-clockwise on the page.
//! The composed matrix decomposes into the IR's frame: the box's size, its clockwise rotation about its centre and a
//! vertical flip when the matrix mirrors; a shear is dropped, and the caller reports it.

use std::f64::consts::PI;

use slidock_ir::frame::Frame;
use slidock_ir::units::Angle;
use slidock_math::{Grid, atan2, cos, hypot, quantise, sin};

use crate::graphic::split_list;
use crate::names::qn;
use crate::odf::Element;
use crate::units::{is_space, length_pt, plain_number, pt, raw_length, size, to_pt};

/// `x' = a·x + c·y + e`, `y' = b·x + d·y + f`, in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Matrix {
    pub a: f64,
    pub b: f64,
    pub c: f64,
    pub d: f64,
    pub e: f64,
    pub f: f64,
}

const IDENTITY: Matrix = Matrix::new([1.0, 0.0, 0.0, 1.0, 0.0, 0.0]);

impl Matrix {
    pub(crate) const fn new([a, b, c, d, e, f]: [f64; 6]) -> Self {
        Matrix { a, b, c, d, e, f }
    }

    /// `second ∘ self`: the point goes through `self`, then `second`.
    pub(crate) fn then(self, second: Matrix) -> Matrix {
        let Matrix {
            a: a1,
            b: b1,
            c: c1,
            d: d1,
            e: e1,
            f: f1,
        } = self;
        let Matrix {
            a: a2,
            b: b2,
            c: c2,
            d: d2,
            e: e2,
            f: f2,
        } = second;
        Matrix::new([
            a2 * a1 + c2 * b1,
            b2 * a1 + d2 * b1,
            a2 * c1 + c2 * d1,
            b2 * c1 + d2 * d1,
            a2 * e1 + c2 * f1 + e2,
            b2 * e1 + d2 * f1 + f2,
        ])
    }

    fn is_finite(&self) -> bool {
        [self.a, self.b, self.c, self.d, self.e, self.f]
            .iter()
            .all(|x| x.is_finite())
    }
}

/// One `name(args)` of a transform list (`([a-zA-Z]+)\s*\(([^)]*)\)`), found from `from`: where it starts and
/// ends, its name and its arguments.
fn next_operation(value: &str, from: usize) -> Option<(usize, usize, &str, &str)> {
    let bytes = value.as_bytes();
    let mut at = from;
    while at < bytes.len() {
        if !bytes.get(at).is_some_and(u8::is_ascii_alphabetic) {
            at += 1;
            continue;
        }
        let start = at;
        while bytes.get(at).is_some_and(u8::is_ascii_alphabetic) {
            at += 1;
        }
        let name = value.get(start..at)?;
        let rest = value.get(at..)?;
        let after_space = rest.trim_start_matches(is_space);
        if let Some(inner) = after_space.strip_prefix('(')
            && let Some(close) = inner.find(')')
        {
            let open = value.len() - inner.len();
            let args = inner.get(..close)?;
            return Some((start, open + close + 1, name, args));
        }
    }
    None
}

/// A `draw:transform` list as one matrix; `None` when it is unparsable.
pub(crate) fn parse_transform(value: &str) -> Option<Matrix> {
    let mut matrix = IDENTITY;
    let mut consumed = 0;
    while let Some((start, end, name, inner)) = next_operation(value, consumed) {
        if !slidock_canon::js_trim(value.get(consumed..start)?).is_empty() {
            return None;
        }
        consumed = end;
        let args: Vec<&str> = split_list(slidock_canon::js_trim(inner))
            .into_iter()
            .filter(|arg| !arg.is_empty())
            .collect();
        let arg = |i: usize| args.get(i).copied();
        let number = |i: usize| plain_number(arg(i));
        let length = |i: usize| {
            raw_length(arg(i)).or_else(|| (plain_number(arg(i)) == Some(0.0)).then_some(0.0))
        };
        let step = match name {
            "rotate" => number(0).map(|angle| {
                Matrix::new([cos(angle), -sin(angle), sin(angle), cos(angle), 0.0, 0.0])
            }),
            "translate" => {
                let x = length(0);
                let y = if args.len() > 1 { length(1) } else { Some(0.0) };
                x.zip(y)
                    .map(|(x, y)| Matrix::new([1.0, 0.0, 0.0, 1.0, x, y]))
            }
            "scale" => {
                let x = number(0);
                let y = if args.len() > 1 { number(1) } else { x };
                x.zip(y)
                    .map(|(x, y)| Matrix::new([x, 0.0, 0.0, y, 0.0, 0.0]))
            }
            "skewX" => number(0)
                .map(|angle| Matrix::new([1.0, 0.0, sin(angle) / cos(angle), 1.0, 0.0, 0.0])),
            "skewY" => number(0)
                .map(|angle| Matrix::new([1.0, sin(angle) / cos(angle), 0.0, 1.0, 0.0, 0.0])),
            "matrix" => match [
                number(0),
                number(1),
                number(2),
                number(3),
                length(4),
                length(5),
            ] {
                [Some(a), Some(b), Some(c), Some(d), Some(e), Some(f)] => {
                    Some(Matrix::new([a, b, c, d, e, f]))
                }
                _ => None,
            },
            _ => None,
        };
        let step = step.filter(Matrix::is_finite)?;
        matrix = matrix.then(step);
    }
    slidock_canon::js_trim(value.get(consumed..)?)
        .is_empty()
        .then_some(matrix)
}

fn degrees(radians: f64) -> f64 {
    let turned = quantise(
        ((((radians * 180.0) / PI) % 360.0) + 360.0) % 360.0,
        Grid::Thousandths,
    );
    if turned >= 360.0 { 0.0 } else { turned }
}

/// A frame, whether a shear was dropped to make it, and whether an unparsable transform was ignored.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Placed {
    pub frame: Frame,
    pub sheared: bool,
    pub unreadable: bool,
}

/// The frame of a `w` × `h` box moved by `matrix`.
pub(crate) fn frame_of(w: f64, h: f64, matrix: Matrix) -> Placed {
    let Matrix { a, b, c, d, e, f } = matrix;
    let scale_x = hypot(a, b);
    let scale_y = hypot(c, d);
    let width = w * scale_x;
    let height = h * scale_y;
    let cx = a * (w / 2.0) + c * (h / 2.0) + e;
    let cy = b * (w / 2.0) + d * (h / 2.0) + f;
    let sheared = (a * c + b * d).abs() > 1e-9 * (scale_x * scale_y).max(1.0);
    Placed {
        frame: Frame {
            x: pt(to_pt(cx - width / 2.0)),
            y: pt(to_pt(cy - height / 2.0)),
            w: size(to_pt(width.max(0.0))),
            h: size(to_pt(height.max(0.0))),
            rotation: Angle::quantise(degrees(atan2(b, a))).unwrap_or(Angle::ZERO),
            flip_h: false,
            flip_v: a * d - b * c < 0.0,
        },
        sheared,
        unreadable: false,
    }
}

/// The placement attributes of a drawable, as a frame; `None` when its size is missing.
pub(crate) fn placement(element: &Element) -> Option<Placed> {
    let w = length_pt(element.attr(qn!("svg:width")))?;
    let h = length_pt(element.attr(qn!("svg:height")))?;
    let x = raw_length(element.attr(qn!("svg:x"))).unwrap_or(0.0);
    let y = raw_length(element.attr(qn!("svg:y"))).unwrap_or(0.0);
    let transform = match element.attr(qn!("draw:transform")) {
        None => Some(IDENTITY),
        Some(written) => parse_transform(written),
    };
    let moved = transform
        .unwrap_or(IDENTITY)
        .then(Matrix::new([1.0, 0.0, 0.0, 1.0, x, y]));
    let mut placed = frame_of(w.max(0.0), h.max(0.0), moved);
    placed.unreadable = transform.is_none();
    Some(placed)
}

/// The axis-aligned box a frame covers on its parent, rotation included.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Bounds {
    pub x0: f64,
    pub y0: f64,
    pub x1: f64,
    pub y1: f64,
}

pub(crate) fn bounds(frame: &Frame) -> Bounds {
    let angle = (frame.rotation.to_f64() * PI) / 180.0;
    let (c, s) = (cos(angle).abs(), sin(angle).abs());
    let (w, h) = (frame.w.to_f64(), frame.h.to_f64());
    let half_w = (w * c + h * s) / 2.0;
    let half_h = (w * s + h * c) / 2.0;
    let cx = frame.x.to_f64() + w / 2.0;
    let cy = frame.y.to_f64() + h / 2.0;
    Bounds {
        x0: cx - half_w,
        y0: cy - half_h,
        x1: cx + half_w,
        y1: cy + half_h,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame_values(frame: &Frame) -> [f64; 5] {
        [
            frame.x.to_f64(),
            frame.y.to_f64(),
            frame.w.to_f64(),
            frame.h.to_f64(),
            frame.rotation.to_f64(),
        ]
    }

    fn element(attrs: &[(&str, &str)]) -> Element {
        Element {
            attrs: attrs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
            ..Element::default()
        }
    }

    #[test]
    fn applies_a_transform_list_in_order_then_the_position() {
        // LibreOffice's rotated shape: rotate the box about its origin, then move that corner to the translate.
        let placed = placement(&element(&[
            ("svg:width", "2.053cm"),
            ("svg:height", "3.473cm"),
            (
                "draw:transform",
                "rotate (-0.392350015848326) translate (3.931cm 4.887cm)",
            ),
        ]))
        .unwrap();
        assert_eq!(
            frame_values(&placed.frame),
            [90.398, 145.915, 58.195, 98.447, 22.48]
        );
        assert!(!placed.frame.flip_h && !placed.frame.flip_v);
        let plain = placement(&element(&[
            ("svg:x", "1in"),
            ("svg:y", "2in"),
            ("svg:width", "1cm"),
            ("svg:height", "1cm"),
        ]))
        .unwrap();
        assert_eq!(frame_values(&plain.frame)[..2], [72.0, 144.0]);
        assert!(placement(&element(&[("svg:x", "1in")])).is_none());
    }

    #[test]
    fn finds_mirroring_and_shear_in_a_matrix() {
        let mirrored = frame_of(10.0, 10.0, Matrix::new([1.0, 0.0, 0.0, -1.0, 0.0, 10.0]));
        assert!(mirrored.frame.flip_v);
        assert_eq!(frame_values(&mirrored.frame)[..2], [0.0, 0.0]);
        assert!(frame_of(10.0, 10.0, Matrix::new([1.0, 0.0, 0.5, 1.0, 0.0, 0.0])).sheared);
        assert!(parse_transform("scale (2) matrix (1 0 0 1 1cm 0)").is_some());
        assert!(parse_transform("rotate (1) bogus").is_none());
        assert!(parse_transform("rotate ()").is_none());
        assert!(parse_transform("x rotate (1)").is_none());
        assert!(parse_transform("rotate(1)rotate (2)").is_some());
        assert!(parse_transform("").is_some());
    }
}
