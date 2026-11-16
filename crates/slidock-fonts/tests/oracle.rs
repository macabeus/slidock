//! Differential checks of the shipped files against Chromium's sanitizer and fontTools (feature `font-oracle`).
//! `SLIDOCK_FONT_ORACLE_PYTHON` names a Python with `fonttools[woff]` and `opentype-sanitizer` installed (a venv
//! outside the repo); the test fails when it is missing. For every fixture, and two fonts derived from Oswald:
//!
//! - ots-sanitize accepts the subset sfnt and the shipped WOFF2;
//! - fontTools decompiles every table of the subset;
//! - fontTools decodes the WOFF2 to exactly the subset's table data;
//! - the subset's cmap maps exactly the code points `unicode-range` states.
#![cfg(feature = "font-oracle")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::path::PathBuf;
use std::process::Command;

use slidock_fonts::subset::{SubsetOptions, subset_face};
use slidock_fonts::{CodepointSet, FontOrigin, ShipInput, ShipOptions, ship_face};
use support::fixtures::*;
use support::{kern_only, mac_names_only};

const TEXT: &str = "Sphinx of black quartz, judge my vow! 0–9 “fi” ŝ Ünïcödé שלום עולם (a) [b]";

const CHECK: &str = r#"
import io, json, sys
import ots
from fontTools.ttLib import TTFont

def tables(data):
    font = TTFont(io.BytesIO(data))
    return {tag: font.reader[tag] for tag in font.reader.keys()}

report = {}
for name in sys.argv[2:]:
    base = sys.argv[1] + "/" + name
    entry = {}
    for suffix in ("sfnt", "woff2"):
        run = ots.sanitize(base + "." + suffix, capture_output=True)
        entry["ots_" + suffix] = run.returncode == 0 or (run.stdout + run.stderr).decode(errors="replace")[-300:]
    sfnt = open(base + ".sfnt", "rb").read()
    woff2 = open(base + ".woff2", "rb").read()
    font = TTFont(io.BytesIO(sfnt))
    for tag in font.keys():
        font[tag]
    entry["decompiles"] = True
    entry["same_tables"] = tables(woff2) == tables(sfnt)
    cmap = font.getBestCmap() or {}
    entry["cmap"] = sorted(cmap)
    report[name] = entry
print(json.dumps(report))
"#;

#[test]
fn shipped_files_pass_ots_and_round_trip_through_fonttools() {
    let python = std::env::var("SLIDOCK_FONT_ORACLE_PYTHON")
        .expect("SLIDOCK_FONT_ORACLE_PYTHON names a Python with fontTools and opentype-sanitizer");
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("font-oracle");
    std::fs::create_dir_all(&dir).unwrap();
    let mut wanted = CodepointSet::new();
    wanted.insert_text(TEXT);
    let mut names = Vec::new();
    let mut ranges = Vec::new();
    // Derived in code: kept `kern` with unchanged glyph ids, and Macintosh-only names.
    let derived = [
        ("Oswald-kern-only.ttf", kern_only(OSWALD)),
        ("Oswald-mac-names.ttf", mac_names_only(OSWALD)),
    ];
    let fonts = ALL
        .iter()
        .map(|(name, font)| (*name, *font))
        .chain(derived.iter().map(|(name, font)| (*name, font.as_slice())));
    for (name, font) in fonts {
        let shipped = ship_face(
            &ShipInput {
                family: "Oracle",
                bytes: font,
                index: 0,
                origin: FontOrigin::Google,
                codepoints: &wanted,
                small_caps: false,
                source_licence: None,
            },
            &ShipOptions::default(),
        )
        .unwrap();
        let sfnt = if shipped.subset {
            subset_face(font, 0, &wanted, &SubsetOptions::default()).unwrap()
        } else {
            font.to_vec()
        };
        std::fs::write(dir.join(format!("{name}.sfnt")), &sfnt).unwrap();
        std::fs::write(dir.join(format!("{name}.woff2")), &shipped.rule.bytes).unwrap();
        names.push(name);
        ranges.push((shipped.subset, shipped.rule.unicode_range.clone().unwrap()));
    }
    let script = dir.join("check.py");
    std::fs::write(&script, CHECK).unwrap();
    let output = Command::new(&python)
        .arg("-I")
        .arg(&script)
        .arg(&dir)
        .args(&names)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let report: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    for (name, (subset, range)) in names.iter().zip(ranges) {
        let entry = &report[*name];
        assert_eq!(entry["ots_sfnt"], true, "{name}: {entry}");
        assert_eq!(entry["ots_woff2"], true, "{name}: {entry}");
        assert_eq!(entry["decompiles"], true, "{name}");
        assert_eq!(entry["same_tables"], true, "{name}");
        if subset {
            let cmap: Vec<u32> = entry["cmap"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_u64().unwrap() as u32)
                .collect();
            assert_eq!(cmap, range.iter().collect::<Vec<_>>(), "{name}");
        }
    }
    assert!(names.len() >= 5);
}
