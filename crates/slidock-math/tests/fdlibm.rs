//! The port against its references: the engine's results and correctly rounded values (`tests/data/fdlibm.tsv`),
//! IEEE special values, and the private correctly rounded references (`golden`). Bit equality with fdlibm is checked by
//! `tests/jdk.rs`; `src/fdlibm/v8_model.rs` accounts for the engine's last-bit differences.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod samples;

#[cfg(feature = "golden")]
#[path = "../../../tests/golden/support.rs"]
mod golden;

use samples::{canonical_bits, parse_bits, ulps};
use std::collections::BTreeMap;

fn call(name: &str, args: &[f64]) -> f64 {
    match (name, args) {
        ("sin", [x]) => slidock_math::sin(*x),
        ("cos", [x]) => slidock_math::cos(*x),
        ("tan", [x]) => slidock_math::tan(*x),
        ("asin", [x]) => slidock_math::asin(*x),
        ("acos", [x]) => slidock_math::acos(*x),
        ("atan", [x]) => slidock_math::atan(*x),
        ("exp", [x]) => slidock_math::exp(*x),
        ("log", [x]) => slidock_math::log(*x),
        ("cbrt", [x]) => slidock_math::cbrt(*x),
        ("atan2", [y, x]) => slidock_math::atan2(*y, *x),
        ("hypot", [x, y]) => slidock_math::hypot(*x, *y),
        _ => panic!("unknown function {name}/{}", args.len()),
    }
}

/// Vectors whose reference is the correctly rounded result rather than the engine's.
const CORRECTLY_ROUNDED: [&str; 2] = ["cbrt", "hypot"];

#[derive(Default)]
struct Tally {
    count: usize,
    exact: usize,
}

#[test]
fn every_vector_is_within_one_ulp() {
    let mut tallies: BTreeMap<&str, Tally> = BTreeMap::new();
    let mut misses = Vec::new();
    for line in include_str!("data/fdlibm.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let cols: Vec<&str> = line.split('\t').collect();
        let (result, args) = cols[2..].split_last().unwrap();
        let args: Vec<f64> = args.iter().map(|h| parse_bits(h)).collect();
        let (expected, got) = (parse_bits(result), call(cols[0], &args));
        let tally = tallies.entry(cols[0]).or_default();
        tally.count += 1;
        if canonical_bits(got) == canonical_bits(expected) {
            tally.exact += 1;
        } else if ulps(got, expected) > 1 {
            misses.push(format!(
                "{}{args:?}: {got:e}, expected {expected:e}",
                cols[0]
            ));
        }
    }
    assert_eq!(misses, Vec::<String>::new());
    assert_eq!(tallies.len(), 11);
    for (name, tally) in &tallies {
        assert!(tally.count >= 900, "{name}: {} vectors", tally.count);
        // fdlibm's errors are below one ulp: most results are the correctly rounded ones.
        if CORRECTLY_ROUNDED.contains(name) {
            let share = tally.exact as f64 / tally.count as f64;
            assert!(share > 0.9, "{name}: {share} correctly rounded");
        }
    }
}

fn same(a: f64, b: f64) -> bool {
    canonical_bits(a) == canonical_bits(b)
}

#[test]
fn ieee_special_values() {
    let (inf, nan) = (f64::INFINITY, f64::NAN);
    for f in [
        slidock_math::sin,
        slidock_math::tan,
        slidock_math::asin,
        slidock_math::atan,
        slidock_math::cbrt,
    ] {
        assert!(same(f(-0.0), -0.0) && same(f(0.0), 0.0));
        assert!(f(nan).is_nan());
    }
    for f in [slidock_math::sin, slidock_math::cos, slidock_math::tan] {
        assert!(f(inf).is_nan() && f(-inf).is_nan());
    }
    assert!(same(slidock_math::cos(-0.0), 1.0));
    assert!(slidock_math::asin(1.0000000000000002).is_nan() && slidock_math::acos(-1.5).is_nan());
    assert!(
        same(slidock_math::acos(1.0), 0.0) && same(slidock_math::acos(-1.0), std::f64::consts::PI)
    );
    assert!(same(slidock_math::atan(inf), std::f64::consts::FRAC_PI_2));
    assert!(same(slidock_math::exp(inf), inf) && same(slidock_math::exp(-inf), 0.0));
    assert!(same(slidock_math::exp(710.0), inf) && same(slidock_math::exp(-746.0), 0.0));
    assert!(same(slidock_math::log(0.0), -inf) && same(slidock_math::log(-0.0), -inf));
    assert!(slidock_math::log(-1.0).is_nan() && same(slidock_math::log(inf), inf));
    assert!(same(slidock_math::log(1.0), 0.0));
    assert!(same(slidock_math::cbrt(-8.0), -2.0) && same(slidock_math::cbrt(-inf), -inf));
    assert!(same(slidock_math::atan2(0.0, -0.0), std::f64::consts::PI));
    assert!(same(slidock_math::atan2(-0.0, -0.0), -std::f64::consts::PI));
    assert!(same(slidock_math::atan2(-0.0, 0.0), -0.0));
    assert!(same(
        slidock_math::atan2(inf, -inf),
        3.0 * std::f64::consts::FRAC_PI_4
    ));
    assert!(same(slidock_math::hypot(inf, nan), inf) && same(slidock_math::hypot(nan, -inf), inf));
    assert!(slidock_math::hypot(nan, 1.0).is_nan());
    assert!(
        same(slidock_math::hypot(3.0, -4.0), 5.0) && same(slidock_math::hypot(-0.0, -0.0), 0.0)
    );
}

/// Where fdlibm 5.3 and the engine part ways, the port keeps fdlibm.
#[test]
fn keeps_fdlibm_where_the_engine_differs() {
    // V8 special-cases exp(1) to Math.E; fdlibm is one ulp above.
    assert_eq!(
        slidock_math::exp(1.0).to_bits(),
        std::f64::consts::E.to_bits() + 1
    );
    // FreeBSD (and V8) return ±π/2 for |y/x| > 2^60 with x < 0; fdlibm computes π - (π/2 - π_lo).
    assert_eq!(
        slidock_math::atan2(1.0, -1e-300).to_bits(),
        std::f64::consts::FRAC_PI_2.to_bits() + 1
    );
}

/// The correctly rounded references (`bc` at 420 digits) in the private fixtures (`golden`): every result within one
/// ulp, most exact, and every result's bits as recorded (`slidock-math/fdlibm-references.tsv`).
#[cfg(feature = "golden")]
#[test]
fn within_one_ulp_of_the_correctly_rounded_references_and_as_recorded() {
    let Some(golden) = golden::Golden::open("slidock-math fdlibm") else {
        return;
    };
    let path = golden.fixture("fdlibm-references.json");
    let references: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    let mut exact = 0;
    let mut misses = Vec::new();
    let mut recorded = String::new();
    for reference in &references {
        let name = reference["fn"].as_str().unwrap();
        let args: Vec<f64> = reference["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|arg| arg[0].as_f64().unwrap() * samples::pow2(arg[1].as_i64().unwrap() as i32))
            .collect();
        let expected = parse_bits(reference["bits"].as_str().unwrap());
        let got = call(name, &args);
        let bits: Vec<String> = args
            .iter()
            .map(|a| format!("{:016x}", a.to_bits()))
            .collect();
        recorded.push_str(&format!(
            "{name}\t{}\t{:016x}\n",
            bits.join(","),
            got.to_bits()
        ));
        match ulps(got, expected) {
            0 => exact += 1,
            1 => {}
            d => misses.push(format!("{name}{args:?}: {d} ulps")),
        }
    }
    assert_eq!(misses, Vec::<String>::new());
    assert!(references.len() > 100, "{}", references.len());
    assert!(
        exact as f64 / references.len() as f64 > 0.95,
        "{exact}/{}",
        references.len()
    );
    let problem = golden.record("slidock-math/fdlibm-references.tsv", recorded.as_bytes());
    assert!(problem.is_none(), "{}", problem.unwrap_or_default());
}
