//! DrawingML shape guides (ECMA-376 Part 1, §20.1.9.11): each guide is one of seventeen operators over numbers and
//! earlier guides, evaluated in order. Lengths are in the shape's units; angles in 60000ths of a degree. Presets
//! evaluate pre-parsed formulas ([`crate::presets`]); custom geometry evaluates its formula strings here, with the same
//! operators.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use indexmap::IndexMap;
use slidock_canon::is_js_whitespace;
use slidock_math::{atan2, cos, sin, tan};

use crate::builtins::{BUILTIN_NAMES, SS_DIVISORS, WH_DIVISORS};
use crate::literal::parse_literal;
use crate::op::Op;

const DEG: f64 = 60_000.0;

fn to_radians(angle: f64) -> f64 {
    (angle / DEG) * (PI / 180.0)
}

fn to_angle(radians: f64) -> f64 {
    (radians * 180.0 * DEG) / PI
}

/// `Math.max(x, y)`: NaN wins, and +0 is above −0.
pub fn js_max(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x == y {
        if x.is_sign_positive() { x } else { y }
    } else if x > y {
        x
    } else {
        y
    }
}

/// `Math.min(x, y)`: NaN wins, and −0 is below +0.
pub fn js_min(x: f64, y: f64) -> f64 {
    if x.is_nan() || y.is_nan() {
        f64::NAN
    } else if x == y {
        if x.is_sign_negative() { x } else { y }
    } else if x < y {
        x
    } else {
        y
    }
}

/// A formula that cannot be evaluated. The messages are the frontends' diagnostic details.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum FormulaError {
    #[error("Unknown guide {0}")]
    UnknownGuide(String),
    #[error("Unknown operator {0}")]
    UnknownOperator(String),
    #[error("{op} takes {arity} arguments: {formula}")]
    Arity {
        op: &'static str,
        arity: usize,
        formula: String,
    },
}

impl Op {
    /// The operator over its arguments (missing ones are 0).
    pub fn apply(self, x: f64, y: f64, z: f64) -> f64 {
        match self {
            Op::MulDiv => {
                if z == 0.0 {
                    0.0
                } else {
                    (x * y) / z
                }
            }
            Op::AddSub => x + y - z,
            Op::AddDiv => {
                if z == 0.0 {
                    0.0
                } else {
                    (x + y) / z
                }
            }
            Op::IfElse => {
                if x > 0.0 {
                    y
                } else {
                    z
                }
            }
            Op::Abs => x.abs(),
            Op::At2 => to_angle(atan2(y, x)),
            Op::Cat2 => x * cos(atan2(z, y)),
            Op::Cos => x * cos(to_radians(y)),
            Op::Max => js_max(x, y),
            Op::Min => js_min(x, y),
            Op::Mod => (x * x + y * y + z * z).sqrt(),
            Op::Pin => {
                if y < x {
                    x
                } else if y > z {
                    z
                } else {
                    y
                }
            }
            Op::Sat2 => x * sin(atan2(z, y)),
            Op::Sin => x * sin(to_radians(y)),
            Op::Sqrt => x.sqrt(),
            Op::Tan => x * tan(to_radians(y)),
            Op::Val => x,
        }
    }
}

/// The values of a `w` × `h` shape's built-in guides, in [`BUILTIN_NAMES`] order.
pub fn builtin_values(w: f64, h: f64) -> [f64; 41] {
    let ss = js_min(w, h);
    let ls = js_max(w, h);
    let mut out = [0.0; 41];
    let head = [
        w,
        h,
        0.0,
        0.0,
        w,
        h,
        w / 2.0,
        h / 2.0,
        ss,
        ls,
        10_800_000.0,
        5_400_000.0,
        2_700_000.0,
        16_200_000.0,
        8_100_000.0,
        13_500_000.0,
        18_900_000.0,
    ];
    let values = head
        .into_iter()
        .chain(WH_DIVISORS.iter().map(|&n| w / f64::from(n)))
        .chain(WH_DIVISORS.iter().map(|&n| h / f64::from(n)))
        .chain(SS_DIVISORS.iter().map(|&n| ss / f64::from(n)));
    for (slot, value) in out.iter_mut().zip(values) {
        *slot = value;
    }
    out
}

/// Guide values by name.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Guides(BTreeMap<String, f64>);

impl Guides {
    /// The built-in guides of a `w` × `h` shape.
    pub fn builtin(w: f64, h: f64) -> Guides {
        Guides(
            BUILTIN_NAMES
                .iter()
                .zip(builtin_values(w, h))
                .map(|(name, value)| ((*name).to_owned(), value))
                .collect(),
        )
    }

    pub fn get(&self, name: &str) -> Option<f64> {
        self.0.get(name).copied()
    }

    pub fn set(&mut self, name: &str, value: f64) {
        self.0.insert(name.to_owned(), value);
    }

    /// A guide name or a number, as a value.
    pub fn value_of(&self, token: &str) -> Result<f64, FormulaError> {
        self.get(token)
            .or_else(|| parse_literal(token))
            .ok_or_else(|| FormulaError::UnknownGuide(token.to_owned()))
    }

    /// One formula (`+- w 0 adj`, operator first) against these guides.
    pub fn evaluate(&self, formula: &str) -> Result<f64, FormulaError> {
        let mut tokens = formula
            .split(is_js_whitespace)
            .filter(|token| !token.is_empty());
        let name = tokens.next().unwrap_or("");
        let op = Op::parse(name).ok_or_else(|| FormulaError::UnknownOperator(name.to_owned()))?;
        let arity = op.arity();
        // Extra arguments are ignored: the standard's own circular arrows write `+- xH 0 dxB 0` for xH − dxB.
        let args: Vec<&str> = tokens.take(arity).collect();
        if args.len() < arity {
            return Err(FormulaError::Arity {
                op: op.name(),
                arity,
                formula: formula.to_owned(),
            });
        }
        let mut values = [0.0; 3];
        for (slot, token) in values.iter_mut().zip(args) {
            *slot = self.value_of(token)?;
        }
        let [x, y, z] = values;
        Ok(op.apply(x, y, z))
    }

    /// These guides, then `guides` in order, each over the ones before it; a name in `overrides` takes the
    /// override's value instead of its formula.
    pub fn with_guides<N: AsRef<str>, F: AsRef<str>>(
        mut self,
        guides: &[(N, F)],
        overrides: &IndexMap<String, f64>,
    ) -> Result<Guides, FormulaError> {
        for (name, formula) in guides {
            let name = name.as_ref();
            let value = match overrides.get(name) {
                Some(&value) => value,
                None => self.evaluate(formula.as_ref())?,
            };
            self.set(name, value);
        }
        Ok(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn js_min_and_max_order_zeros_and_propagate_nan() {
        assert!(js_max(0.0, -0.0).is_sign_positive());
        assert!(js_max(-0.0, 0.0).is_sign_positive());
        assert!(js_min(0.0, -0.0).is_sign_negative());
        assert!(js_min(-0.0, 0.0).is_sign_negative());
        assert!(js_max(f64::NAN, 1.0).is_nan());
        assert!(js_min(1.0, f64::NAN).is_nan());
        assert_eq!(js_max(1.0, 2.0), 2.0);
        assert_eq!(js_min(1.0, 2.0), 1.0);
    }

    #[test]
    fn literals_are_plain_decimals() {
        assert_eq!(parse_literal("42"), Some(42.0));
        assert_eq!(parse_literal("-8333"), Some(-8333.0));
        assert_eq!(parse_literal("0.5"), Some(0.5));
        assert_eq!(parse_literal("-0").map(f64::is_sign_negative), Some(true));
        for token in ["", "-", "1.", ".5", "+1", "1e3", "0x10", "1.2.3", "١"] {
            assert_eq!(parse_literal(token), None, "{token}");
        }
    }

    #[test]
    fn builtin_values_follow_the_name_order() {
        let values = builtin_values(200.0, 100.0);
        let named = |name: &str| {
            BUILTIN_NAMES
                .iter()
                .position(|n| *n == name)
                .and_then(|i| values.get(i).copied())
        };
        assert_eq!(named("ss"), Some(100.0));
        assert_eq!(named("ls"), Some(200.0));
        assert_eq!(named("wd32"), Some(6.25));
        assert_eq!(named("hd10"), Some(10.0));
        assert_eq!(named("ssd16"), Some(6.25));
        assert_eq!(named("3cd4"), Some(16_200_000.0));
    }
}
