//! slidock-canon against JavaScript itself: every file in `tests/data` was written by Node 24 (`generate.mjs`).

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

use slidock_canon::{
    Grid, Layout, format_grid, is_js_whitespace, js_round, js_trim, js_trim_end, js_trim_start,
    sha256_hex, to_fixed, to_js_string, utf16, value_to_canonical_json, write_json_string,
};

fn bits(hex: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(hex, 16).unwrap())
}

/// The tab-separated columns of each data line.
fn rows(text: &'static str) -> impl Iterator<Item = Vec<&'static str>> {
    text.lines().map(|line| line.split('\t').collect())
}

/// Runs `check` over every row, collecting the failures, and returns the row count.
fn check_all(
    text: &'static str,
    mut check: impl FnMut(&[&'static str]) -> Option<String>,
) -> usize {
    let mut failures = Vec::new();
    let mut count = 0;
    for row in rows(text) {
        count += 1;
        failures.extend(check(&row));
    }
    let shown: Vec<&String> = failures.iter().take(20).collect();
    assert!(
        failures.is_empty(),
        "{} of {count} failed: {shown:#?}",
        failures.len()
    );
    count
}

#[test]
fn number_to_string() {
    let count = check_all(include_str!("data/number-string.tsv"), |row| {
        let (x, expected) = (bits(row[0]), row[1]);
        let got = to_js_string(x);
        (got != expected).then(|| format!("String({x:e}) = {got}, expected {expected}"))
    });
    assert!(count > 10_000, "{count}");
}

#[test]
fn math_round() {
    let count = check_all(include_str!("data/math-round.tsv"), |row| {
        let (x, expected) = (bits(row[0]), bits(row[1]));
        let got = js_round(x);
        let same = got.to_bits() == expected.to_bits() || (got.is_nan() && expected.is_nan());
        (!same).then(|| format!("Math.round({x:e}) = {got:e}, expected {expected:e}"))
    });
    assert!(count > 10_000, "{count}");
}

#[test]
fn number_to_fixed() {
    let count = check_all(include_str!("data/to-fixed.tsv"), |row| {
        let (x, digits, expected) = (bits(row[0]), row[1].parse::<u8>().unwrap(), row[2]);
        let got = to_fixed(x, digits).unwrap();
        (got != expected).then(|| format!("({x:e}).toFixed({digits}) = {got}, expected {expected}"))
    });
    assert!(count > 10_000, "{count}");
}

#[test]
fn grid_printing() {
    let count = check_all(include_str!("data/grid.tsv"), |row| {
        let x = bits(row[0]);
        let fixed = format_grid(x, Grid::Thousandths).unwrap();
        let ratio = format_grid(x, Grid::HundredThousandths).unwrap();
        (fixed != row[1] || ratio != row[2])
            .then(|| format!("{x:e}: {fixed} / {ratio}, expected {} / {}", row[1], row[2]))
    });
    assert!(count > 10_000, "{count}");
}

#[test]
fn whitespace_is_javascripts() {
    let listed: Vec<u32> = include_str!("data/whitespace.txt")
        .lines()
        .map(|h| u32::from_str_radix(h, 16).unwrap())
        .collect();
    let ours: Vec<u32> = (0..=0x10_FFFF)
        .filter_map(char::from_u32)
        .filter(|c| is_js_whitespace(*c))
        .map(u32::from)
        .collect();
    assert_eq!(ours, listed);
    assert_eq!(listed.len(), 25);
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct StringCase {
    s: String,
    length: usize,
    trim: String,
    trim_start: String,
    trim_end: String,
    slices: Vec<(usize, usize, Option<String>)>,
}

#[test]
fn string_lengths_trims_and_slices() {
    let mut slices = 0;
    let count = check_all(include_str!("data/strings.jsonl"), |row| {
        let case: StringCase = serde_json::from_str(row[0]).unwrap();
        let mut problems = Vec::new();
        if utf16::len(&case.s) != case.length {
            problems.push("length");
        }
        if js_trim(&case.s) != case.trim
            || js_trim_start(&case.s) != case.trim_start
            || js_trim_end(&case.s) != case.trim_end
        {
            problems.push("trim");
        }
        for (start, end, expected) in &case.slices {
            slices += 1;
            if utf16::slice(&case.s, *start, *end) != expected.as_deref() {
                problems.push("slice");
            }
        }
        (!problems.is_empty()).then(|| format!("{:?}: {problems:?}", case.s))
    });
    assert!(count >= 3_000 && slices >= 12_000, "{count} {slices}");
}

#[test]
fn sorting_matches_array_sort() {
    let sorted: Vec<String> = serde_json::from_str(include_str!("data/sorted.json")).unwrap();
    let mut ours = sorted.clone();
    ours.reverse();
    ours.sort_by(|a, b| utf16::cmp(a, b));
    assert_eq!(ours, sorted);
    // The data includes pairs that code point order sorts the other way.
    let mut by_bytes = sorted.clone();
    by_bytes.sort();
    assert_ne!(by_bytes, sorted);
    assert!(sorted.len() >= 3_000);
}

#[test]
fn strings_escape_as_json_stringify() {
    let count = check_all(include_str!("data/json-strings.tsv"), |row| {
        let s: String = row[0]
            .split(',')
            .filter(|h| !h.is_empty())
            .map(|h| char::from_u32(u32::from_str_radix(h, 16).unwrap()).unwrap())
            .collect();
        let mut got = String::new();
        write_json_string(&s, &mut got);
        (got != row[1]).then(|| format!("{s:?}: {got}, expected {}", row[1]))
    });
    assert!(count > 10_000, "{count}");
}

#[derive(serde::Deserialize)]
struct CanonicalCase {
    input: String,
    compact: Option<String>,
    indented: Option<String>,
}

#[test]
fn canonical_json() {
    let mut refused = 0;
    let count = check_all(include_str!("data/canonical.jsonl"), |row| {
        let case: CanonicalCase = serde_json::from_str(row[0]).unwrap();
        let value: serde_json::Value = serde_json::from_str(&case.input).unwrap();
        let compact = value_to_canonical_json(&value, Layout::Compact).ok();
        let indented = value_to_canonical_json(&value, Layout::Indented).ok();
        refused += usize::from(case.compact.is_none());
        (compact != case.compact || indented != case.indented)
            .then(|| format!("{}: {compact:?}, expected {:?}", case.input, case.compact))
    });
    assert!(
        count >= 1_500 && refused > 0 && refused < count / 2,
        "{count} {refused}"
    );
}

#[test]
fn sha256_of_utf8() {
    let count = check_all(include_str!("data/sha256.tsv"), |row| {
        let s: String = serde_json::from_str(row[0]).unwrap();
        let got = sha256_hex(&s);
        (got != row[1]).then(|| format!("{s:?}: {got}"))
    });
    assert!(count >= 500, "{count}");
}
