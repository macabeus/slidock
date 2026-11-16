//! `quantise` against the IR formula evaluated by the engine (`tests/data/quantise.tsv`): ties, values next to
//! ties, magnitudes from 1e-9 to 1e30 (where only `toFixed` decides), non-finite values and random bit patterns.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod samples;

use samples::{canonical_bits, parse_bits};
use slidock_math::{Grid, grid_steps, quantise};

#[test]
fn matches_the_engine_bit_for_bit() {
    let mut count = 0;
    let mut misses = Vec::new();
    for line in include_str!("data/quantise.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let cols: Vec<f64> = line.split('\t').map(parse_bits).collect();
        let [x, pt, ratio] = cols[..] else {
            panic!("bad line {line}")
        };
        for (grid, expected) in [(Grid::Thousandths, pt), (Grid::HundredThousandths, ratio)] {
            let got = quantise(x, grid);
            if canonical_bits(got) != canonical_bits(expected) {
                misses.push(format!(
                    "quantise({x:e}, {grid:?}) = {got:e}, expected {expected:e}"
                ));
            }
        }
        count += 1;
    }
    assert_eq!(misses, Vec::<String>::new());
    assert!(count > 10_000, "{count}");
}

#[test]
fn steps_are_the_quantised_decimal() {
    let mut count = 0;
    for line in include_str!("data/quantise.tsv")
        .lines()
        .filter(|l| !l.starts_with('#'))
    {
        let x = parse_bits(line.split('\t').next().unwrap());
        for grid in [Grid::Thousandths, Grid::HundredThousandths] {
            if let Ok(steps) = grid_steps(x, grid) {
                assert_eq!(
                    quantise(x, grid),
                    steps as f64 / grid.steps_per_unit() as f64,
                    "{x:e} {grid:?}"
                );
                count += 1;
            }
        }
    }
    assert!(count > 10_000, "{count}");
}
