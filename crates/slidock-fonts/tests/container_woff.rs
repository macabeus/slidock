//! WOFF and WOFF2 decoding on fixtures made by fontTools and the reference encoder from five OFL fonts (see
//! `tests/data/container/README.md`), compared with what fontTools and the reference decoder make of them.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod container_support;

use container_support::*;
use slidock_fonts::container::{self, Format, Hint};
use slidock_fonts::read_sfnt;

const FONTS: [&str; 6] = [
    "barlow",
    "fira",
    "oswald",
    "pacifico",
    "roboto",
    "robotomono",
];

fn unwrap(bytes: &[u8]) -> container::Unwrapped {
    container::unwrap(bytes, &Hint::default()).unwrap()
}

/// Every face of `ours` has the tables of `oracle`, byte-equal except `glyf`, `loca`, and `head`'s
/// `checkSumAdjustment` and lossless-modification flag; glyphs are compared one by one as [`GlyphModel`]s (fontTools re-encodes them). Each
/// record's checksum is right.
fn assert_tables_match(ours: &[u8], oracle: &[u8], label: &str) {
    let faces = face_offsets(ours);
    assert_eq!(faces.len(), face_offsets(oracle).len(), "{label}: faces");
    for (ours_face, oracle_face) in faces.into_iter().zip(face_offsets(oracle)) {
        let ours_tables = tables(ours, ours_face);
        let oracle_tables = tables(oracle, oracle_face);
        assert_eq!(
            ours_tables.keys().collect::<Vec<_>>(),
            oracle_tables.keys().collect::<Vec<_>>(),
            "{label}: tags"
        );
        for (tag, (data, record_checksum)) in &ours_tables {
            let name = String::from_utf8_lossy(tag);
            let mut expected_checksum = checksum(data);
            if tag == b"head" {
                expected_checksum = expected_checksum.wrapping_sub(u32_at(data, 8));
            }
            assert_eq!(
                *record_checksum, expected_checksum,
                "{label}: {name} checksum"
            );
            let oracle_data = oracle_tables[tag].0;
            match tag {
                b"glyf" | b"loca" => {}
                b"head" => {
                    // checkSumAdjustment is the layout's; flags bit 11 says a WOFF2 encoder transformed the font.
                    let without = |head: &[u8]| {
                        let mut head = head.to_vec();
                        head[8..12].fill(0);
                        head[16] &= !0x08;
                        head
                    };
                    assert_eq!(without(data), without(oracle_data), "{label}: head");
                }
                _ => assert!(*data == oracle_data, "{label}: {name} differs"),
            }
        }
        if ours_tables.contains_key(b"glyf") {
            let ours_glyphs: Vec<GlyphModel> = glyphs(ours, ours_face)
                .iter()
                .map(|g| glyph_model(g))
                .collect();
            let oracle_glyphs: Vec<GlyphModel> = glyphs(oracle, oracle_face)
                .iter()
                .map(|g| glyph_model(g))
                .collect();
            assert!(!ours_glyphs.is_empty());
            assert_eq!(ours_glyphs, oracle_glyphs, "{label}: glyphs");
        }
    }
}

/// The font's checksum, with every `head.checkSumAdjustment`, is 0xB1B0AFBA.
fn assert_font_checksum(sfnt: &[u8]) {
    if &sfnt[..4] != b"ttcf" {
        assert_eq!(checksum(sfnt), 0xb1b0_afba);
    }
}

#[test]
fn woff2_decodes_like_fonttools_table_by_table() {
    let mut cases: Vec<String> = FONTS.iter().map(|f| format!("{f}.woff2")).collect();
    cases.extend(["barlow-hmtx.woff2", "barlow-null.woff2", "barlow-cff.woff2"].map(String::from));
    for case in &cases {
        let decoded = unwrap(&fixture(case));
        assert_eq!(decoded.kind, Format::Woff2);
        assert!(decoded.warnings.is_empty());
        assert_tables_match(&decoded.sfnt, &fixture(&format!("{case}.ft.ttf")), case);
        assert_font_checksum(&decoded.sfnt);
        read_sfnt(&decoded.sfnt).unwrap();
    }
}

#[test]
fn woff2_decodes_byte_for_byte_like_the_reference_decoder() {
    let mut cases: Vec<String> = FONTS.iter().map(|f| format!("{f}.woff2")).collect();
    cases.extend(
        [
            "barlow-hmtx.woff2",
            "barlow-null.woff2",
            "barlow-cff.woff2",
            "pair.woff2",
        ]
        .map(String::from),
    );
    for case in &cases {
        let decoded = unwrap(&fixture(case)).sfnt;
        assert!(decoded == fixture(&format!("{case}.g.ttf")), "{case}");
    }
}

#[test]
fn woff2_collections_decode_to_both_faces() {
    let decoded = unwrap(&fixture("pair.woff2")).sfnt;
    let faces = read_sfnt(&decoded).unwrap();
    assert_eq!(faces.len(), 2);
    assert_eq!(faces[0].family, "Barlow Condensed");
    assert_eq!(faces[1].family, "Fira Sans");
    assert_tables_match(&decoded, &fixture("pair.ttc"), "pair.woff2");
}

#[test]
fn woff_decodes_to_the_original_tables() {
    for font in FONTS
        .iter()
        .map(|f| f.to_string())
        .chain(["barlow-cff".into()])
    {
        let source = if font == "barlow-cff" {
            "barlow-cff.otf".into()
        } else {
            format!("{font}.ttf")
        };
        let decoded = unwrap(&fixture(&format!("{font}.woff")));
        assert_eq!(decoded.kind, Format::Woff);
        assert!(
            decoded.warnings.is_empty(),
            "{font}: {:?}",
            decoded.warnings
        );
        let original = fixture(&source);
        assert_tables_match(&decoded.sfnt, &original, &font);
        // WOFF keeps glyf and loca as they were.
        if font != "barlow-cff" {
            assert_eq!(
                table(&decoded.sfnt, 0, b"glyf"),
                table(&original, 0, b"glyf")
            );
            assert_eq!(
                table(&decoded.sfnt, 0, b"loca"),
                table(&original, 0, b"loca")
            );
        }
        assert_font_checksum(&decoded.sfnt);
    }
}
