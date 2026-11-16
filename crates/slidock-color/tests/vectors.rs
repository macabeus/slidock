//! The committed colour vectors: grid colours under random modifier lists, with their expected results. They were
//! generated once from seeded inputs (`$SLIDOCK_PRIVATE/tools/color-geometry-vectors.ts`). Numbers are `String(n)`.

#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

use serde_json::Value;
use slidock_color::apply_color_mods;
use slidock_ir::color::{ColorMod, ColorSpace, RgbColor};
use slidock_ir::units::{Ratio, UnitRatio};

fn num(value: &Value) -> f64 {
    value.as_str().unwrap().parse().unwrap()
}

fn components(value: &Value) -> Vec<f64> {
    value.as_array().unwrap().iter().map(num).collect()
}

#[test]
fn matches_the_committed_vectors() {
    let mut misses = Vec::new();
    let mut count = 0;
    for line in include_str!("data/mods.jsonl").lines() {
        let case: Value = serde_json::from_str(line).unwrap();
        let space: ColorSpace = serde_json::from_value(case[0].clone()).unwrap();
        let c = components(&case[1]);
        let unit = |x: f64| UnitRatio::exact(x).unwrap();
        let color = RgbColor::new(space, unit(c[0]), unit(c[1]), unit(c[2]), unit(c[3]));
        let mods: Vec<ColorMod> = case[2]
            .as_array()
            .unwrap()
            .iter()
            .map(|pair| ColorMod {
                op: serde_json::from_value(pair[0].clone()).unwrap(),
                value: Ratio::exact(num(&pair[1])).unwrap(),
            })
            .collect();
        let out = apply_color_mods(&color, &mods).unwrap();
        let got = [out.r, out.g, out.b, out.a].map(UnitRatio::to_f64);
        if got.as_slice() != components(&case[3]) || out.space != space {
            misses.push(format!("{line}: {got:?}"));
        }
        count += 1;
    }
    assert_eq!(misses, Vec::<String>::new());
    assert!(count >= 1000, "{count}");
}
