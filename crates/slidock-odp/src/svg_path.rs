//! SVG path data (`svg:d`) and point lists (`draw:points`), as ODF's `draw:path`, `draw:polyline` and
//! `draw:polygon` write them in their `svg:viewBox` units, into the IR's path segments. Relative commands become
//! absolute, smooth curves get their reflected control point, quadratic curves stay quadratic and elliptical arcs
//! become cubics (at most a quarter turn each).

use std::f64::consts::PI;

use slidock_core_api::lowering::LowerError;
use slidock_geometry::js_max;
use slidock_ir::geometry::{PathUnitSegment, Seg};
use slidock_ir::units::PathUnit;
use slidock_math::{atan2, cos, sin, tan};

const UNIT_LIMIT: f64 = 1e9;

fn unit(x: f64) -> PathUnit {
    // `Math.max(-limit, Math.min(limit, x))`: NaN stays NaN.
    let clamped = slidock_geometry::js_max(-UNIT_LIMIT, slidock_geometry::js_min(UNIT_LIMIT, x));
    PathUnit::quantise(clamped).unwrap_or(PathUnit::ZERO)
}

/// The segments a shape may still hold.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Budget {
    pub left: i64,
}

impl Budget {
    pub(crate) fn spend(&mut self) -> Result<(), LowerError> {
        self.left -= 1;
        if self.left < 0 {
            return Err(LowerError::Limit(
                "a shape holds too many path segments".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Token {
    Command(u8),
    Number(f64),
}

/// The length of a number at the start of `bytes`: `[-+]?(\d+\.?\d*|\.\d+)`, with `([eE][-+]?\d+)?` when
/// `exponent`.
fn number_length(bytes: &[u8], exponent: bool) -> Option<usize> {
    let digits = |from: usize| {
        bytes.get(from..).map_or(0, |rest| {
            rest.iter().take_while(|b| b.is_ascii_digit()).count()
        })
    };
    let mut at = usize::from(matches!(bytes.first(), Some(b'-' | b'+')));
    let whole = digits(at);
    if whole > 0 {
        at += whole;
        if bytes.get(at) == Some(&b'.') {
            at += 1 + digits(at + 1);
        }
    } else if bytes.get(at) == Some(&b'.') && digits(at + 1) > 0 {
        at += 1 + digits(at + 1);
    } else {
        return None;
    }
    if exponent && matches!(bytes.get(at), Some(b'e' | b'E')) {
        let sign = usize::from(matches!(bytes.get(at + 1), Some(b'-' | b'+')));
        let count = digits(at + 1 + sign);
        if count > 0 {
            at += 1 + sign + count;
        }
    }
    Some(at)
}

/// Every command letter and number of the path data, in order; anything else is skipped.
fn tokens(d: &str) -> Vec<Token> {
    let bytes = d.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        let rest = bytes.get(at..).unwrap_or_default();
        let Some(&first) = rest.first() else { break };
        if b"MmLlHhVvCcSsQqTtAaZz".contains(&first) {
            out.push(Token::Command(first));
            at += 1;
        } else if let Some(length) = number_length(rest, true) {
            let text = d.get(at..at + length).unwrap_or_default();
            out.push(Token::Number(text.parse().unwrap_or(f64::NAN)));
            at += length;
        } else {
            at += d
                .get(at..)
                .and_then(|s| s.chars().next())
                .map_or(1, char::len_utf8);
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Point {
    x: f64,
    y: f64,
}

/// An elliptical arc from `from` as cubics (SVG 1.1 implementation notes, F.6.5 and F.6.6).
fn arc_cubics(
    from: Point,
    rx_in: f64,
    ry_in: f64,
    rotation_deg: f64,
    large_arc: bool,
    sweep: bool,
    to: Point,
) -> Vec<[f64; 6]> {
    let mut rx = rx_in.abs();
    let mut ry = ry_in.abs();
    if rx == 0.0 || ry == 0.0 || (from.x == to.x && from.y == to.y) {
        return vec![[from.x, from.y, to.x, to.y, to.x, to.y]];
    }
    let phi = (rotation_deg * PI) / 180.0;
    let (cphi, sphi) = (cos(phi), sin(phi));
    let dx = (from.x - to.x) / 2.0;
    let dy = (from.y - to.y) / 2.0;
    let x1 = cphi * dx + sphi * dy;
    let y1 = -sphi * dx + cphi * dy;
    let lambda = (x1 * x1) / (rx * rx) + (y1 * y1) / (ry * ry);
    if lambda > 1.0 {
        rx *= lambda.sqrt();
        ry *= lambda.sqrt();
    }
    let numerator = rx * rx * ry * ry - rx * rx * y1 * y1 - ry * ry * x1 * x1;
    let denominator = rx * rx * y1 * y1 + ry * ry * x1 * x1;
    let factor =
        (if large_arc == sweep { -1.0 } else { 1.0 }) * js_max(0.0, numerator / denominator).sqrt();
    let cx1 = (factor * rx * y1) / ry;
    let cy1 = (-factor * ry * x1) / rx;
    let cx = cphi * cx1 - sphi * cy1 + (from.x + to.x) / 2.0;
    let cy = sphi * cx1 + cphi * cy1 + (from.y + to.y) / 2.0;
    let theta = atan2((y1 - cy1) / ry, (x1 - cx1) / rx);
    let mut delta = atan2((-y1 - cy1) / ry, (-x1 - cx1) / rx) - theta;
    if sweep && delta < 0.0 {
        delta += 2.0 * PI;
    }
    if !sweep && delta > 0.0 {
        delta -= 2.0 * PI;
    }
    let pieces = js_max(1.0, (delta.abs() / (PI / 2.0) - 1e-9).ceil());
    let step = delta / pieces;
    let k = (4.0 / 3.0) * tan(step / 4.0);
    let point = |angle: f64| Point {
        x: cx + rx * cphi * cos(angle) - ry * sphi * sin(angle),
        y: cy + rx * sphi * cos(angle) + ry * cphi * sin(angle),
    };
    let tangent = |angle: f64| Point {
        x: -rx * cphi * sin(angle) - ry * sphi * cos(angle),
        y: -rx * sphi * sin(angle) + ry * cphi * cos(angle),
    };
    let mut out = Vec::new();
    let mut i = 0.0;
    while i < pieces {
        let a0 = theta + i * step;
        let a1 = a0 + step;
        let p0 = point(a0);
        let p1 = if i == pieces - 1.0 { to } else { point(a1) };
        let (t0, t1) = (tangent(a0), tangent(a1));
        out.push([
            p0.x + k * t0.x,
            p0.y + k * t0.y,
            p1.x - k * t1.x,
            p1.y - k * t1.y,
            p1.x,
            p1.y,
        ]);
        i += 1.0;
    }
    out
}

/// SVG path data as segments; parsing stops at the first malformed command, keeping what came before.
pub(crate) fn parse_path_data(
    d: &str,
    budget: &mut Budget,
) -> Result<Vec<PathUnitSegment>, LowerError> {
    let tokens = tokens(d);
    let mut segments: Vec<PathUnitSegment> = Vec::new();
    let mut push = |segment: PathUnitSegment, segments: &mut Vec<PathUnitSegment>| {
        budget.spend()?;
        segments.push(segment);
        Ok::<(), LowerError>(())
    };
    let mut at = 0;
    let mut current = Point { x: 0.0, y: 0.0 };
    let mut start = current;
    let mut last_control: Option<Point> = None;
    let mut last_quad: Option<Point> = None;
    let mut command: u8 = 0;
    let numbers = |at: &mut usize, count: usize| -> Option<Vec<f64>> {
        let values: Vec<f64> = tokens
            .get(*at..(*at + count).min(tokens.len()))
            .unwrap_or_default()
            .iter()
            .filter_map(|token| match token {
                Token::Number(n) => Some(*n),
                Token::Command(_) => None,
            })
            .collect();
        if values.len() < count {
            return None;
        }
        *at += count;
        Some(values)
    };
    while let Some(token) = tokens.get(at) {
        match token {
            Token::Command(letter) => {
                command = *letter;
                at += 1;
            }
            Token::Number(_) if command == 0 || command == b'Z' || command == b'z' => break,
            Token::Number(_) => {}
        }
        let relative = command.is_ascii_lowercase();
        let base = if relative {
            current
        } else {
            Point { x: 0.0, y: 0.0 }
        };
        let abs = |x: f64, y: f64| Point {
            x: base.x + x,
            y: base.y + y,
        };
        let upper = command.to_ascii_uppercase();
        let mut control: Option<Point> = None;
        let mut quad: Option<Point> = None;
        match upper {
            b'Z' => {
                push(Seg::Z, &mut segments)?;
                current = start;
            }
            b'M' | b'L' | b'T' => {
                let Some(n) = numbers(&mut at, 2) else { break };
                let (Some(&nx), Some(&ny)) = (n.first(), n.get(1)) else {
                    break;
                };
                let to = abs(nx, ny);
                if upper == b'T' {
                    let q = match last_quad {
                        Some(last) => Point {
                            x: 2.0 * current.x - last.x,
                            y: 2.0 * current.y - last.y,
                        },
                        None => current,
                    };
                    push(
                        Seg::Q(unit(q.x), unit(q.y), unit(to.x), unit(to.y)),
                        &mut segments,
                    )?;
                    quad = Some(q);
                } else if upper == b'M' {
                    push(Seg::M(unit(to.x), unit(to.y)), &mut segments)?;
                } else {
                    push(Seg::L(unit(to.x), unit(to.y)), &mut segments)?;
                }
                if upper == b'M' {
                    start = to;
                    // Further pairs after a move are lines.
                    command = if relative { b'l' } else { b'L' };
                }
                current = to;
            }
            b'H' | b'V' => {
                let Some(n) = numbers(&mut at, 1) else { break };
                let Some(&value) = n.first() else { break };
                current = if upper == b'H' {
                    Point {
                        x: (if relative { current.x } else { 0.0 }) + value,
                        y: current.y,
                    }
                } else {
                    Point {
                        x: current.x,
                        y: (if relative { current.y } else { 0.0 }) + value,
                    }
                };
                push(Seg::L(unit(current.x), unit(current.y)), &mut segments)?;
            }
            b'C' | b'S' => {
                let Some(n) = numbers(&mut at, if upper == b'C' { 6 } else { 4 }) else {
                    break;
                };
                let v = |i: usize| n.get(i).copied().unwrap_or(f64::NAN);
                let c1 = if upper == b'C' {
                    abs(v(0), v(1))
                } else {
                    match last_control {
                        Some(last) => Point {
                            x: 2.0 * current.x - last.x,
                            y: 2.0 * current.y - last.y,
                        },
                        None => current,
                    }
                };
                let (c2, to) = if upper == b'C' {
                    (abs(v(2), v(3)), abs(v(4), v(5)))
                } else {
                    (abs(v(0), v(1)), abs(v(2), v(3)))
                };
                push(
                    Seg::C(
                        unit(c1.x),
                        unit(c1.y),
                        unit(c2.x),
                        unit(c2.y),
                        unit(to.x),
                        unit(to.y),
                    ),
                    &mut segments,
                )?;
                control = Some(c2);
                current = to;
            }
            b'Q' => {
                let Some(n) = numbers(&mut at, 4) else { break };
                let v = |i: usize| n.get(i).copied().unwrap_or(f64::NAN);
                let q = abs(v(0), v(1));
                let to = abs(v(2), v(3));
                push(
                    Seg::Q(unit(q.x), unit(q.y), unit(to.x), unit(to.y)),
                    &mut segments,
                )?;
                quad = Some(q);
                current = to;
            }
            b'A' => {
                let Some(n) = numbers(&mut at, 7) else { break };
                let v = |i: usize| n.get(i).copied().unwrap_or(f64::NAN);
                let to = abs(v(5), v(6));
                for [a, b, c, d, e, f] in
                    arc_cubics(current, v(0), v(1), v(2), v(3) != 0.0, v(4) != 0.0, to)
                {
                    push(
                        Seg::C(unit(a), unit(b), unit(c), unit(d), unit(e), unit(f)),
                        &mut segments,
                    )?;
                }
                current = to;
            }
            _ => break,
        }
        last_control = control;
        last_quad = quad;
    }
    Ok(segments)
}

/// A `draw:points` list (`x,y x,y …`) as a polyline, closed for a polygon.
pub(crate) fn parse_points(
    points: &str,
    close: bool,
    budget: &mut Budget,
) -> Result<Vec<PathUnitSegment>, LowerError> {
    let bytes = points.as_bytes();
    let mut numbers = Vec::new();
    let mut at = 0;
    while at < bytes.len() {
        match number_length(bytes.get(at..).unwrap_or_default(), false) {
            Some(length) => {
                let text = points.get(at..at + length).unwrap_or_default();
                numbers.push(text.parse::<f64>().unwrap_or(f64::NAN));
                at += length;
            }
            None => {
                at += points
                    .get(at..)
                    .and_then(|s| s.chars().next())
                    .map_or(1, char::len_utf8);
            }
        }
    }
    let mut segments = Vec::new();
    for (i, pair) in numbers.as_chunks::<2>().0.iter().enumerate() {
        budget.spend()?;
        let [x, y] = *pair;
        segments.push(if i == 0 {
            Seg::M(unit(x), unit(y))
        } else {
            Seg::L(unit(x), unit(y))
        });
    }
    if close && !segments.is_empty() {
        segments.push(Seg::Z);
    }
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn steps(segment: &PathUnitSegment) -> (String, Vec<f64>) {
        (
            segment.command().to_owned(),
            segment.coords().iter().map(|c| c.to_f64()).collect(),
        )
    }

    #[test]
    fn reads_relative_commands_smooth_curves_and_arcs_as_cubics() {
        let mut budget = Budget { left: 100 };
        let segments = parse_path_data(
            "m10 10 h10 v10 s5 5 10 0 q5 5 10 0 t10 0 A5 5 0 0 1 60 20 z",
            &mut budget,
        )
        .unwrap();
        let commands: Vec<&str> = segments.iter().map(Seg::command).collect();
        // A half turn of arc is two quarter-turn cubics.
        assert_eq!(commands, ["M", "L", "L", "C", "Q", "Q", "C", "C", "Z"]);
        assert_eq!(steps(&segments[0]).1, [10.0, 10.0]);
        assert_eq!(steps(&segments[2]).1, [20.0, 20.0]);
        assert_eq!(steps(&segments[3]).1, [20.0, 20.0, 25.0, 25.0, 30.0, 20.0]);
        assert_eq!(steps(&segments[5]).1, [45.0, 15.0, 50.0, 20.0]);
        assert_eq!(steps(&segments[7]).1[4..], [60.0, 20.0]);
        let error = parse_path_data("M0 0 L1 1 L2 2", &mut Budget { left: 2 }).unwrap_err();
        assert!(error.to_string().contains("too many path segments"));
    }

    #[test]
    fn tokenizes_as_the_svg_grammar_reads() {
        assert_eq!(
            tokens("M1e2-3.5.5Z"),
            [
                Token::Command(b'M'),
                Token::Number(100.0),
                Token::Number(-3.5),
                Token::Number(0.5),
                Token::Command(b'Z')
            ]
        );
        assert_eq!(tokens("1e"), [Token::Number(1.0)]);
        assert_eq!(tokens("é-x2"), [Token::Number(2.0)]);
    }

    #[test]
    fn reads_point_lists() {
        let segments =
            parse_points("100,1100 600,100 1100", true, &mut Budget { left: 10 }).unwrap();
        let commands: Vec<&str> = segments.iter().map(Seg::command).collect();
        assert_eq!(commands, ["M", "L", "Z"]);
    }
}
