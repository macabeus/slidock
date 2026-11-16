//! The bench's pinned font set (`--features golden`, tests/golden; skips when the golden files are absent), read from
//! `$SLIDOCK_PRIVATE/bench/cache/fonts` (files named by their SHA-256, each pinned by the golden manifest): every file
//! reads as recorded (`slidock-fonts/read-sfnt.json`), field for field; `bench/fonts/fonts.lock.json`, the bench's font
//! lock, lists the set, and its `url` sources, the open-licensed release files, regenerate the built-in table.
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

#[path = "../../../tests/golden/support.rs"]
mod golden;

use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::sync::OnceLock;

use golden::Golden;

use read_fonts::TableProvider;
use serde_json::Value;
use slidock_canon::{Layout, sha256_hex, to_canonical_json};
use slidock_fonts::catalog::builtin::metrics_table;
use slidock_fonts::catalog::substitutes::family_generic;
use slidock_fonts::names::generic_of;
use slidock_fonts::{CatalogFace, FamilyKey, FontCatalog, builtin_catalog, read_sfnt};
use slidock_ir::fonts::{FaceStyle, GenericFamily};
use slidock_ir::ids::Sha256;

const BUILTIN_TABLE: &str = include_str!("../data/builtin-metrics.json");

/// The golden files, opened once; `None` when they are absent.
fn golden() -> Option<&'static Golden> {
    static GOLDEN: OnceLock<Option<Golden>> = OnceLock::new();
    GOLDEN
        .get_or_init(|| Golden::open("slidock-fonts"))
        .as_ref()
}

fn read_json(path: &Path) -> Value {
    let text = fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    serde_json::from_str(&text).unwrap()
}

/// The font files of the set in name order, with their bytes; each file is named by its SHA-256.
fn font_files() -> Vec<(String, Vec<u8>)> {
    let dir = golden().unwrap().fonts();
    let mut names: Vec<String> = fs::read_dir(&dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .filter(|name| name.ends_with(".ttf") || name.ends_with(".ttc") || name.ends_with(".otf"))
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let bytes = fs::read(dir.join(&name)).unwrap();
            assert_eq!(
                sha256_hex(&bytes),
                stem(&name),
                "{name} does not hold the pinned bytes"
            );
            (name, bytes)
        })
        .collect()
}

fn stem(name: &str) -> &str {
    name.split('.').next().unwrap()
}

/// The SHA-256 of every file of the lock, and of its release files.
fn locked_files() -> (BTreeSet<String>, BTreeSet<String>) {
    let lock = read_json(&golden::repo().join("bench/fonts/fonts.lock.json"));
    let fonts = lock["fonts"].as_array().unwrap();
    let sha = |font: &Value| font["sha256"].as_str().unwrap().to_owned();
    let all = fonts.iter().map(sha).collect();
    let release = fonts
        .iter()
        .filter(|font| font["source"]["kind"] == "url")
        .map(sha)
        .collect();
    (all, release)
}

#[test]
fn reads_every_file_of_the_set_as_recorded_field_for_field() {
    let Some(golden) = golden() else {
        return;
    };
    let files = font_files();
    assert_eq!(files.len(), 58);
    let mut faces = 0;
    let mut read = serde_json::Map::new();
    for (name, bytes) in &files {
        let ours = read_sfnt(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
        faces += ours.len();
        read.insert(name.clone(), serde_json::to_value(&ours).unwrap());
    }
    assert_eq!(faces, 76);
    let printed = to_canonical_json(&Value::Object(read), Layout::Indented).unwrap();
    let problem = golden.record("slidock-fonts/read-sfnt.json", printed.as_bytes());
    assert!(problem.is_none(), "{}", problem.unwrap_or_default());
}

#[test]
fn the_lock_lists_the_set_and_38_release_files() {
    if golden().is_none() {
        return;
    }
    let (all, release) = locked_files();
    let files: BTreeSet<String> = font_files()
        .iter()
        .map(|(name, _)| stem(name).to_owned())
        .collect();
    assert_eq!(all, files);
    assert_eq!(release.len(), 38);
}

#[test]
fn regenerates_the_builtin_table_from_the_release_files() {
    if golden().is_none() {
        return;
    }
    let (_, release) = locked_files();
    let entries = font_files()
        .into_iter()
        .filter(|(name, _)| release.contains(stem(name)))
        .flat_map(|(_, bytes)| {
            let sha256 = Sha256::of(&bytes);
            read_sfnt(&bytes).unwrap().into_iter().map(move |face| {
                (
                    face.family.clone(),
                    CatalogFace::from_sfnt(&face, Some(sha256)),
                )
            })
        });
    let catalog = FontCatalog::build(entries);
    let printed = to_canonical_json(&metrics_table(&catalog), Layout::Indented).unwrap();
    assert_eq!(printed, BUILTIN_TABLE);
    // The built-in faces carry no file, so their families take the generic their names suggest: the one their
    // release files' pitch gives.
    for (key, family) in catalog.families() {
        assert_eq!(
            builtin_catalog().get(key).unwrap().generic,
            family.generic,
            "{}",
            family.family
        );
    }
    let monospace: Vec<&str> = catalog
        .families()
        .filter(|(_, family)| family.faces.iter().any(|face| face.monospace))
        .map(|(_, family)| family.family.as_str())
        .collect();
    assert_eq!(monospace, ["Liberation Mono", "Roboto Mono"]);
}

/// HelveticaNeue.ttc of the set (Apple's, read from the reference Mac).
const HELVETICA_NEUE: &str = "f589d70dc741853a336fcb0d11aa1505d1646898fba2340a0f5999ea920406b3";

/// The catalog of one file's faces, as `--fonts` builds it.
fn file_catalog(bytes: &[u8]) -> FontCatalog {
    let sha256 = Sha256::of(bytes);
    FontCatalog::build(read_sfnt(bytes).unwrap().iter().map(|face| {
        (
            face.family.clone(),
            CatalogFace::from_sfnt(face, Some(sha256)),
        )
    }))
}

#[test]
fn a_collection_binds_its_members_of_normal_width() {
    if golden().is_none() {
        return;
    }
    let files = font_files();
    let (_, bytes) = files
        .iter()
        .find(|(name, _)| stem(name) == HELVETICA_NEUE)
        .unwrap();
    // The file holds Condensed Bold (700) after Bold, and Condensed Black, the only 900.
    let members: Vec<(String, u32, u16)> = read_sfnt(bytes)
        .unwrap()
        .into_iter()
        .map(|face| (face.postscript_name, face.weight, face.width))
        .collect();
    assert_eq!(members.len(), 14);
    let bold = members.iter().position(|m| m.0 == "HelveticaNeue-Bold");
    let condensed = members
        .iter()
        .position(|m| m.0 == "HelveticaNeue-CondensedBold");
    assert!(bold < condensed);
    assert_eq!(
        members[condensed.unwrap()],
        ("HelveticaNeue-CondensedBold".into(), 700, 3)
    );
    assert!(members.contains(&("HelveticaNeue-CondensedBlack".into(), 900, 3)));
    let catalog = file_catalog(bytes);
    let family = catalog.get(&FamilyKey::of("Helvetica Neue")).unwrap();
    let drawn = |weight: u32, style: FaceStyle| {
        family
            .css_match(weight, style)
            .unwrap()
            .postscript_name
            .as_str()
    };
    assert_eq!(drawn(700, FaceStyle::Normal), "HelveticaNeue-Bold");
    assert_eq!(drawn(900, FaceStyle::Normal), "HelveticaNeue-Bold");
    assert_eq!(drawn(400, FaceStyle::Normal), "HelveticaNeue");
    assert_eq!(drawn(700, FaceStyle::Italic), "HelveticaNeue-BoldItalic");
    // Medium Italic sets none of the italic bits and an italicAngle of 0; its subfamily name files it as italic.
    let medium_italic = read_sfnt(bytes)
        .unwrap()
        .into_iter()
        .find(|face| face.postscript_name == "HelveticaNeue-MediumItalic")
        .unwrap();
    assert_eq!(
        (
            medium_italic.italic,
            medium_italic.italic_angle,
            medium_italic.subfamily.as_str()
        ),
        (true, 0.0, "Medium Italic")
    );
    assert_eq!(drawn(500, FaceStyle::Normal), "HelveticaNeue-Medium");
    assert_eq!(drawn(500, FaceStyle::Italic), "HelveticaNeue-MediumItalic");
    // Twelve members are of normal width, each in a slot of its own.
    assert_eq!(family.faces.len(), 12);
    assert!(family.faces.iter().all(|face| face.width == 5));
}

/// Whether every printable ASCII character the face maps advances by the same width (at least 90 of the 95).
fn fixed_advance(bytes: &[u8], index: u32) -> bool {
    let font = read_fonts::FontRef::from_index(bytes, index).unwrap();
    let cmap = font.cmap().unwrap();
    let metrics = font.hmtx().unwrap();
    let advances: Vec<u16> = (0x20..0x7f_u32)
        .filter_map(|c| cmap.map_codepoint(c))
        .filter_map(|glyph| metrics.advance(glyph))
        .collect();
    let widths: BTreeSet<u16> = advances.iter().copied().collect();
    advances.len() >= 90 && widths.len() == 1
}

#[test]
fn every_face_of_the_set_that_advances_by_one_width_is_monospace() {
    if golden().is_none() {
        return;
    }
    let (mut fixed_families, mut monospace) = (BTreeSet::new(), BTreeSet::new());
    for (name, bytes) in font_files() {
        let catalog = file_catalog(&bytes);
        let faces = read_sfnt(&bytes).unwrap();
        for (index, face) in faces.iter().enumerate() {
            let fixed = fixed_advance(&bytes, u32::try_from(index).unwrap());
            assert_eq!(face.fixed_pitch, fixed, "{name} {}", face.postscript_name);
            if fixed {
                fixed_families.insert(face.family.clone());
            }
        }
        for (_, family) in catalog.families() {
            let fixed = faces.iter().any(|face| {
                face.fixed_pitch && FamilyKey::of(&face.family) == FamilyKey::of(&family.family)
            });
            let expected = if fixed {
                GenericFamily::Monospace
            } else {
                family_generic(&family.family)
            };
            assert_eq!(family.generic, expected, "{name} {}", family.family);
            if family.generic == GenericFamily::Monospace {
                monospace.insert(family.family.clone());
            }
        }
    }
    let expected = [
        "Courier New",
        "Liberation Mono",
        "Monaco",
        "Monoid",
        "Roboto Mono",
    ];
    assert_eq!(fixed_families.into_iter().collect::<Vec<_>>(), expected);
    // Each of them is monospace by its name as well.
    for family in expected {
        assert_eq!(generic_of(family), GenericFamily::Monospace, "{family}");
    }
    assert_eq!(monospace.into_iter().collect::<Vec<_>>(), expected);
}
