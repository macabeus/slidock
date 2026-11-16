//! The port against fdlibm 5.3 itself: the JDK's StrictMath is a strict (unfused) Java port of it, so every result
//! must be bit-identical (`tests/data/Jdk.java`). The one exception is hypot with a subnormal smaller argument,
//! where the port has FreeBSD's fix; `tests/fdlibm.rs` checks those against correctly rounded results.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod samples;

use samples::{canonical_bits, parse_bits, subnormal_hypot, sweeps};

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

fn fixed_branch(name: &str, args: &[f64]) -> bool {
    name == "hypot" && subnormal_hypot(args[0], args[1])
}

#[test]
fn every_vector_is_bit_identical() {
    let (mut checked, mut fixed, mut fix_matters) = (0, 0, 0);
    let mut misses = Vec::new();
    for line in include_str!("data/fdlibm-jdk.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let cols: Vec<&str> = line.split('\t').collect();
        let (result, args) = cols[1..].split_last().unwrap();
        let args: Vec<f64> = args.iter().map(|h| parse_bits(h)).collect();
        let (expected, got) = (parse_bits(result), call(cols[0], &args));
        let same = canonical_bits(got) == canonical_bits(expected);
        if fixed_branch(cols[0], &args) {
            fixed += 1;
            fix_matters += usize::from(!same);
        } else {
            checked += 1;
            if !same {
                misses.push(format!("{}{args:?}: {got:e}, fdlibm {expected:e}", cols[0]));
            }
        }
    }
    assert_eq!(misses, Vec::<String>::new());
    assert!(checked > 18_000, "{checked}");
    // The left-out branch is exercised, and FreeBSD's fix is visible there.
    assert!(fixed > 100 && fix_matters > 0, "{fixed} {fix_matters}");
}

#[test]
fn every_sweep_is_bit_identical() {
    let rows = sweeps(include_str!("data/fdlibm-sweep-jdk.tsv"));
    for row in &rows {
        let digest =
            row.digest_of(|args| (!fixed_branch(&row.name, args)).then(|| call(&row.name, args)));
        assert_eq!(
            digest.as_deref(),
            Some(row.digest.as_str()),
            "{} ({} samples)",
            row.key,
            row.count
        );
    }
    for name in [
        "sin", "cos", "tan", "asin", "acos", "atan", "exp", "log", "cbrt", "atan2", "hypot",
    ] {
        assert!(
            rows.iter().any(|row| row.name == name),
            "no sweep for {name}"
        );
    }
}

/// The functions that reach a helper's branches: with their argument as the helper's (`highWord` rows), and
/// through the reduction, where the reduced argument has a tail (`reducedHighWord` rows).
const CALLERS: [(&str, &[&str], &[&str]); 4] = [
    ("kernel_sin", &["sin"], &["sin", "cos"]),
    ("kernel_cos", &["cos"], &["sin", "cos"]),
    ("kernel_tan", &["tan"], &["tan"]),
    ("rem_pio2", &["sin", "cos", "tan"], &[]),
];

/// `(distribution, function, constant)` for every comparison of a high word (`ix`, `hx`, `iy`, `hy`, `ha`, `hb`)
/// with a hex constant in the port; a helper's comparisons belong to its `CALLERS`.
fn high_word_branches() -> Vec<(&'static str, &'static str, u32)> {
    let source = include_str!("../src/fdlibm/mod.rs");
    let source = source.split("#[cfg(test)]").next().unwrap();
    let mut branches = Vec::new();
    let mut callers: Vec<(&str, &str)> = Vec::new();
    for line in source.lines() {
        if let Some(rest) = line.strip_prefix("pub fn ") {
            callers = vec![("highWord", rest.split('(').next().unwrap())];
        } else if let Some(rest) = line.strip_prefix("fn ") {
            let helper = rest.split(['(', '<']).next().unwrap();
            callers = match CALLERS.iter().find(|(name, ..)| *name == helper) {
                Some((_, direct, reduced)) => (direct.iter().map(|f| ("highWord", *f)))
                    .chain(reduced.iter().map(|f| ("reducedHighWord", *f)))
                    .collect(),
                None => vec![("highWord", helper)],
            };
        }
        let tokens: Vec<&str> = line
            .split_whitespace()
            .map(|t| t.trim_matches(['(', ')', '{']))
            .collect();
        for i in 0..tokens.len().saturating_sub(2) {
            let (word, op, constant) = (tokens[i], tokens[i + 1], tokens[i + 2]);
            let operand = i == 0 || !matches!(tokens[i - 1], "-" | "+");
            if let Some(hex) = constant.strip_prefix("0x")
                && operand
                && ["ix", "hx", "iy", "hy", "ha", "hb"].contains(&word)
                && ["<", "<=", ">", ">=", "==", "!="].contains(&op)
            {
                let value = u32::from_str_radix(&hex.replace('_', ""), 16).unwrap();
                branches.extend(callers.iter().map(|&(dist, caller)| (dist, caller, value)));
            }
        }
    }
    branches
}

/// Every high-word branch is swept at its constant and at both neighbours, so an off-by-one in a comparison
/// changes a sweep digest.
#[test]
fn every_high_word_branch_has_a_boundary_sweep() {
    let rows = sweeps(include_str!("data/fdlibm-sweep-jdk.tsv"));
    // (distribution, function, magnitude of the lowest high word, count) of every high-word range.
    let ranges: Vec<(&str, &str, u32, u32)> = rows
        .iter()
        .flat_map(|row| {
            let cols: Vec<&str> = row.key.split('\t').collect();
            let words: Vec<u32> = match cols[1] {
                "highWord" | "reducedHighWord" => cols[2]
                    .split(',')
                    .map(|w| u32::from_str_radix(w, 16).unwrap())
                    .collect(),
                _ => Vec::new(),
            };
            // reducedHighWord's first word is its count of multiples of π/2.
            let skip = usize::from(cols[1] == "reducedHighWord");
            words[skip.min(words.len())..]
                .chunks(2)
                .map(|w| (cols[1], row.name.as_str(), w[0] & 0x7fff_ffff, w[1]))
                .collect::<Vec<_>>()
        })
        .collect();
    let branches = high_word_branches();
    assert!(branches.len() > 50, "{}", branches.len());
    let missing: Vec<String> = branches
        .iter()
        .filter(|(dist, name, c)| {
            !ranges
                .iter()
                .any(|(d, n, lo, count)| d == dist && n == name && *lo < *c && c + 1 < lo + count)
        })
        .map(|(dist, name, c)| format!("{name} {dist} {c:08x}"))
        .collect();
    assert_eq!(missing, Vec::<String>::new());
}
