//! Shipped faces are the same bytes in every build: these SHA-256 values were recorded from a native build, and the
//! test runs natively and as wasm32 (`wasm-bindgen-test`, under node):
//!
//! ```text
//! CARGO_TARGET_WASM32_UNKNOWN_UNKNOWN_RUNNER=wasm-bindgen-test-runner \
//!     cargo test -p slidock-fonts --target wasm32-unknown-unknown --test wasm_stable
//! ```
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_canon::sha256_hex;
use slidock_fonts::sfnt::metrics_at;
use slidock_fonts::{CodepointSet, FontLimits, FontOrigin, ShipInput, ShipOptions, ship_face};
use support::fixtures::*;

const TEXT: &str = "Fonts ship as the same bytes everywhere — “fi” ŝ 0–9 שלום";

/// Each fixture's `@font-face` rule (descriptors and the base64 WOFF2), by SHA-256.
const RULES: [(&str, &str); 8] = [
    (
        "Oswald-Regular.ttf",
        "edfd85416046792fff6a34a5b37f142cb850bb701a35a7831f92d383ac774edc",
    ),
    (
        "Oswald-Regular.otf",
        "2bf247227b3718a9328ac66c4741a3e2d39ca183c9c591ec85426d01ca6e4bcd",
    ),
    (
        "Oswald-Variable.ttf",
        "094d7a19a432f7e835b6578f8ac192daae91414556e78421743afb59efa1f87a",
    ),
    (
        "Oswald-Variable-MVAR.ttf",
        "2c7f8574b1d63a1e5edd5228255070d27b0bf3a0a6dfd07d7dd22b18ee6cdb0f",
    ),
    (
        "BarlowCondensed-Italic.ttf",
        "0498da062004c87465892db8cdc49c038f91dafc58a4fd65037ca935bd6108fd",
    ),
    (
        "RobotoMono-Regular.ttf",
        "66db3d13ef8903012b732527c4006941dfc69d3842cacab5e502efe176296b50",
    ),
    (
        "Silkscreen-Regular.ttf",
        "ed53e343d9356debdd38107076c1508637c15b6d150b67a8d54e2bd742a8e497",
    ),
    (
        "NotoSansHebrew-Variable.ttf",
        "6284d0ecb52abc11c54f25b2e377909978524f9f0c7f0f02ed123f09d5b69854",
    ),
];

fn rule_sha(font: &[u8]) -> String {
    let mut wanted = CodepointSet::new();
    wanted.insert_text(TEXT);
    let shipped = ship_face(
        &ShipInput {
            family: "Stable",
            bytes: font,
            index: 0,
            origin: FontOrigin::Google,
            codepoints: &wanted,
            small_caps: true,
            source_licence: None,
        },
        &ShipOptions::default(),
    )
    .unwrap();
    sha256_hex(shipped.rule.css().as_bytes())
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn shipped_rules_match_the_recorded_bytes() {
    for ((name, font), (recorded_name, recorded)) in ALL.iter().zip(RULES) {
        assert_eq!(*name, recorded_name);
        assert_eq!(rule_sha(font), recorded, "{name}");
    }
}

#[cfg_attr(not(target_arch = "wasm32"), test)]
#[cfg_attr(target_arch = "wasm32", wasm_bindgen_test::wasm_bindgen_test)]
fn instance_metrics_match_the_recorded_values() {
    let metrics = metrics_at(OSWALD_MVAR, 0, 650, &FontLimits::default()).unwrap();
    assert_eq!(
        format!("{metrics:?}"),
        "FontMetrics { units_per_em: 1000, hhea: VMetrics { ascender: 1247, descender: -325, line_gap: 22 }, \
         typo: VMetrics { ascender: 1247, descender: -325, line_gap: 22 }, win: WinMetrics { ascent: 1396, \
         descent: 404 }, bounds: Some(GlyphBounds { y_min: -198, y_max: 1073 }), grave_top: Some(1073) }"
    );
}
