//! Number printing against JavaScript. `vectors/numbers.json` holds what JavaScript printers of the same rules
//! (`formatFixed`, `formatRatio`, `formatSides`, `literalColor`) write for inputs that stress the grid: magnitudes from
//! 1e-6 to 1e5, half-grid points and their neighbouring doubles, and the products and sums the printers compute. It
//! was generated once by `~/.slidock/tools/html-number-vectors.ts` (Bun). A seeded fuzz then checks the grid printers
//! against the JavaScript expression they stand for, written with slidock-canon's JavaScript primitives, over far more
//! values than the file holds.
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

use serde_json::Value;
use slidock_canon::{js_round, to_fixed, to_js_string};
use slidock_html::css::{fixed, ratio, sides};
use slidock_html::literal_color;
use slidock_ir::color::{ColorSpace, RgbColor};
use slidock_ir::units::UnitRatio;

fn vectors() -> Value {
    serde_json::from_str(include_str!("vectors/numbers.json")).unwrap()
}

fn from_bits(hex: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(hex, 16).unwrap())
}

#[test]
fn prints_numbers_as_the_vectors_record() {
    let vectors = vectors();
    let numbers = vectors["numbers"].as_array().unwrap();
    assert!(numbers.len() > 3000);
    for vector in numbers {
        let x = from_bits(vector[0].as_str().unwrap());
        assert_eq!(
            fixed(x).unwrap(),
            vector[1].as_str().unwrap(),
            "fixed {x:e}"
        );
        assert_eq!(
            ratio(x).unwrap(),
            vector[2].as_str().unwrap(),
            "ratio {x:e}"
        );
    }
}

#[test]
fn writes_sides_as_the_vectors_record() {
    let vectors = vectors();
    let cases = vectors["sides"].as_array().unwrap();
    assert_eq!(cases.len(), 300);
    for case in cases {
        let side = |i: usize| case[i].as_f64().unwrap();
        assert_eq!(
            sides(side(0), side(1), side(2), side(3)).unwrap(),
            case[4].as_str().unwrap()
        );
    }
}

#[test]
fn prints_colours_as_the_vectors_record() {
    let vectors = vectors();
    let cases = vectors["colors"].as_array().unwrap();
    assert_eq!(cases.len(), 500);
    for case in cases {
        let steps: Vec<UnitRatio> = case[0]
            .as_array()
            .unwrap()
            .iter()
            .map(|steps| UnitRatio::from_steps(steps.as_i64().unwrap()))
            .collect();
        let color = |space| RgbColor::new(space, steps[0], steps[1], steps[2], steps[3]);
        assert_eq!(
            literal_color(&color(ColorSpace::Srgb)).unwrap(),
            case[1].as_str().unwrap()
        );
        assert_eq!(
            literal_color(&color(ColorSpace::P3)).unwrap(),
            case[2].as_str().unwrap()
        );
    }
}

/// `formatNumber(quantise(x, unit))`, as JavaScript evaluates it: `Number((Math.round(x / Q) * Q).toFixed(D))`,
/// `-0` as `0`, printed with `String`.
fn js_model(x: f64, quantum: f64, digits: u8) -> String {
    let rounded = js_round(x / quantum) * quantum;
    let normalised: f64 = to_fixed(rounded, digits).unwrap().parse().unwrap();
    to_js_string(if normalised == 0.0 { 0.0 } else { normalised })
}

struct XorShift(u64);

impl XorShift {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }
}

#[test]
fn fuzzes_the_grid_printers_against_the_javascript_expression() {
    let mut rng = XorShift(0x2545_f491_4f6c_dd1d);
    let mut checked = 0;
    for _ in 0..100_000 {
        let sign = if rng.unit() < 0.5 { -1.0 } else { 1.0 };
        // Magnitudes 2^-20 to 2^17, uniform in the exponent.
        let exponent = (rng.unit() * 37.0) as u64 + 1003;
        let random = sign * f64::from_bits((exponent << 52) | (rng.next() >> 12));
        // A half-grid point, as `k / 1000 + 0.0005` computes it, and the doubles beside it.
        let k = (rng.unit() * 1e8).floor() * sign;
        let half = k / 1000.0 + 0.0005;
        let ratio_half = k / 100_000.0 + 0.000005;
        for x in [
            random,
            half,
            f64::from_bits(half.to_bits() + 1),
            f64::from_bits(half.to_bits() - 1),
            ratio_half,
        ] {
            assert_eq!(fixed(x).unwrap(), js_model(x, 0.001, 3), "fixed {x:e}");
            assert_eq!(ratio(x).unwrap(), js_model(x, 0.00001, 5), "ratio {x:e}");
            checked += 1;
        }
    }
    assert_eq!(checked, 500_000);
}
