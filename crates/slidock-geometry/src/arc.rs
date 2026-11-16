//! DrawingML's `arcTo` as cubic Béziers. The arc continues the path from its current point along an ellipse with radii
//! `wR` and `hR`; `stAng` and `swAng` are visual angles (the direction from the centre), clockwise in a y-down space,
//! in 60000ths of a degree. Each quarter turn or less becomes one cubic. A sweep past a whole turn draws one turn and
//! the rest, so it ends where the full sweep does and costs at most eight cubics.

use std::f64::consts::PI;

use slidock_math::{atan2, cos, sin, tan};

/// A point in a path's space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Point {
    pub x: f64,
    pub y: f64,
}

impl Point {
    pub const fn new(x: f64, y: f64) -> Point {
        Point { x, y }
    }
}

/// A cubic's controls and end: `[x1, y1, x2, y2, x, y]`.
pub type Cubic = [f64; 6];

/// An arc's cubics and the point it ends at.
#[derive(Clone, Debug, PartialEq)]
pub struct Arc {
    pub cubics: Vec<Cubic>,
    pub end: Point,
}

const RADIANS_PER_UNIT: f64 = PI / (180.0 * 60_000.0);
const QUARTER: f64 = PI / 2.0;
const TURN: f64 = 2.0 * PI;

/// The parametric angle of the point of an ellipse seen at visual angle `angle` from its centre.
fn parametric(angle: f64, w_r: f64, h_r: f64) -> f64 {
    atan2(w_r * sin(angle), h_r * cos(angle))
}

/// The cubics of an arc from `from`. A zero radius or sweep, or a value that is not finite, draws nothing.
pub fn arc_to(from: Point, w_r: f64, h_r: f64, st_ang: f64, sw_ang: f64) -> Arc {
    if w_r == 0.0
        || h_r == 0.0
        || sw_ang == 0.0
        || ![w_r, h_r, st_ang, sw_ang].iter().all(|v| v.is_finite())
    {
        return Arc {
            cubics: Vec::new(),
            end: from,
        };
    }
    let start = st_ang * RADIANS_PER_UNIT;
    let sweep = sw_ang * RADIANS_PER_UNIT;
    let t0 = parametric(start, w_r, h_r);
    // The parametric sweep keeps the visual sweep's direction and at most one whole turn: more redraws the same
    // outline.
    let whole = (sweep / TURN).trunc();
    let turns = whole.clamp(-1.0, 1.0);
    let remainder = sweep - whole * TURN;
    let rest = if remainder.abs() < 1e-9 {
        0.0
    } else {
        remainder
    };
    let mut span = if rest == 0.0 {
        0.0
    } else {
        parametric(start + rest, w_r, h_r) - t0
    };
    if rest > 0.0 && span <= 0.0 {
        span += TURN;
    }
    if rest < 0.0 && span >= 0.0 {
        span -= TURN;
    }
    span += turns * TURN;

    let cx = from.x - w_r * cos(t0);
    let cy = from.y - h_r * sin(t0);
    let at = |t: f64| Point::new(cx + w_r * cos(t), cy + h_r * sin(t));
    let pieces = ((span.abs() / QUARTER - 1e-9).ceil()).max(1.0) as u32;
    let step = span / f64::from(pieces);
    let k = (4.0 / 3.0) * tan(step / 4.0);
    let mut cubics = Vec::with_capacity(pieces as usize);
    let mut a = t0;
    for _ in 0..pieces {
        let b = a + step;
        let p0 = at(a);
        let p3 = at(b);
        cubics.push([
            p0.x - k * w_r * sin(a),
            p0.y + k * h_r * cos(a),
            p3.x + k * w_r * sin(b),
            p3.y - k * h_r * cos(b),
            p3.x,
            p3.y,
        ]);
        a = b;
    }
    Arc {
        cubics,
        end: at(t0 + span),
    }
}
