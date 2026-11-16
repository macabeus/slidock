//! The embedded fonts of the corpus deck `pptx-talk` (`--features golden`), read from
//! `$SLIDOCK_PRIVATE/corpus` (default `~/.slidock/corpus`) as the golden files pin it: 20 EOT 2.2 parts, MicroType
//! Express compressed. Each decodes to the very bytes libeot
//! (umanwizard/libeot at 0407abddc581d32e9871ee41535183ee1d924d85, `eot2ttf`) writes for it, recorded here as SHA-256.
//! Skips when the golden files are absent.
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod container_support;

#[path = "../../../tests/golden/support.rs"]
mod golden;

use std::path::PathBuf;

use container_support::*;
use read_fonts::{FontRef, TableProvider};
use sha2::{Digest, Sha256};
use slidock_fonts::container::eot;
use slidock_fonts::container::{Format, Hint, sniff, unwrap};
use slidock_fonts::read_sfnt;
use slidock_io::{BytesReader, ZipArchive, ZipLimits};

const DECK: &str = "corpus:pptx-talk.pptx";

/// SHA-256 of libeot's output for each part.
const LIBEOT: [(&str, &str); 20] = [
    (
        "BarlowCondensed-bold",
        "68b51aa106921827be531ee34b9b5b987a37493da4db3dcbdec2c0c12080baff",
    ),
    (
        "BarlowCondensed-boldItalic",
        "9dbd2ef11f64d024cb1ff26a3e4fbc04773b21a42d89a56decd5a6e974cc34a9",
    ),
    (
        "BarlowCondensed-italic",
        "527d439f3599b2f7cbacba4ea28fd89569fa74577e91aa91d08df7471aa95bca",
    ),
    (
        "BarlowCondensed-regular",
        "d34072dd151e8fdb373203b23d9b71c47c20248bf0f03f9435b9fd40109f74ad",
    ),
    (
        "Oswald-bold",
        "2116d053e5cc51aaa18eab91aa2c7bcd68195eb3bdb9c0cd4a0ab021d409c025",
    ),
    (
        "Oswald-regular",
        "c7d2a09c76331203208159e360e7cf5715846c41377567bb06397481ed8559fd",
    ),
    (
        "OswaldMedium-bold",
        "2116d053e5cc51aaa18eab91aa2c7bcd68195eb3bdb9c0cd4a0ab021d409c025",
    ),
    (
        "OswaldMedium-regular",
        "c7d2a09c76331203208159e360e7cf5715846c41377567bb06397481ed8559fd",
    ),
    (
        "Roboto-bold",
        "29170e89a51feb4db6e5d3f6cd4ae60682f4dd40da31a3df6b830d6f350d1ff5",
    ),
    (
        "Roboto-boldItalic",
        "a7b0027ebb4404ac1ca00db395faf84d1cff0df16f47b97a3e0c6f6eaf13492f",
    ),
    (
        "Roboto-italic",
        "9650647370ca466368262d9b0c62dee56098819c1539bba9965c81df6a3d8cab",
    ),
    (
        "Roboto-regular",
        "8a0f696f8736af03a1950a6b3d15acf38ce8325b9e5d7e1d96a662db79645af0",
    ),
    (
        "RobotoMedium-bold",
        "29170e89a51feb4db6e5d3f6cd4ae60682f4dd40da31a3df6b830d6f350d1ff5",
    ),
    (
        "RobotoMedium-boldItalic",
        "a7b0027ebb4404ac1ca00db395faf84d1cff0df16f47b97a3e0c6f6eaf13492f",
    ),
    (
        "RobotoMedium-italic",
        "9650647370ca466368262d9b0c62dee56098819c1539bba9965c81df6a3d8cab",
    ),
    (
        "RobotoMedium-regular",
        "8a0f696f8736af03a1950a6b3d15acf38ce8325b9e5d7e1d96a662db79645af0",
    ),
    (
        "RobotoMono-bold",
        "df631ba619a4152125e5ffc3482500cba9b259c8ab462a17f8473cdddf996cb4",
    ),
    (
        "RobotoMono-boldItalic",
        "18ee4fdf92454eac6003ba64f2fd5a0e879c5d7c5be960718cf41177698b8cc8",
    ),
    (
        "RobotoMono-italic",
        "d04e1f2e88c8f76b9ece8d4ef264a4288a1b057571ce877b72c1c133b6cbab8b",
    ),
    (
        "RobotoMono-regular",
        "53dee5e01fd990f6016ee48597f9bc419737f1bea3f3064abb1a067605a10451",
    ),
];

fn private() -> PathBuf {
    std::env::var_os("SLIDOCK_PRIVATE").map_or_else(
        || PathBuf::from(std::env::var("HOME").unwrap()).join(".slidock"),
        PathBuf::from,
    )
}

/// The pinned corpus deck, or `None` when the golden files are absent.
fn deck() -> Option<Vec<u8>> {
    let golden = golden::Golden::open("slidock-fonts embedded fonts")?;
    Some(std::fs::read(golden.input(DECK)).unwrap())
}

/// The tables that differ from libeot's output, when it is kept at
/// `$SLIDOCK_PRIVATE/oracles/libeot-0407abd/pptx-talk/<stem>.ttf`.
fn table_diff(ours: &[u8], stem: &str) -> String {
    let path = private()
        .join("oracles/libeot-0407abd/pptx-talk")
        .join(format!("{stem}.ttf"));
    let Ok(oracle) = std::fs::read(&path) else {
        return format!("(no libeot output at {})", path.display());
    };
    let (ours, oracle) = (tables(ours, 0), tables(&oracle, 0));
    let tags: std::collections::BTreeSet<_> = ours.keys().chain(oracle.keys()).collect();
    tags.into_iter()
        .filter(|tag| ours.get(*tag).map(|t| t.0) != oracle.get(*tag).map(|t| t.0))
        .map(|tag| String::from_utf8_lossy(tag).into_owned())
        .collect::<Vec<_>>()
        .join(", ")
}

#[test]
fn the_corpus_fonts_are_mtx_compressed_eot_2_2_that_decode_like_libeot() {
    let Some(deck) = deck() else {
        return;
    };
    let zip = ZipArchive::open(BytesReader::new(deck), ZipLimits::default()).unwrap();
    let parts: Vec<_> = zip
        .entries()
        .iter()
        .filter(|e| e.name.starts_with("ppt/fonts/"))
        .cloned()
        .collect();
    assert_eq!(parts.len(), 20);
    let mut decoded = 0;
    for part in parts {
        let bytes = zip.read(&part).unwrap();
        let hint = Hint {
            content_type: Some("application/x-fontdata"),
            part_name: Some(&part.name),
        };
        assert_eq!(sniff(&bytes, &hint), Some(Format::Eot), "{}", part.name);
        let parsed = eot::parse(&bytes).unwrap();
        let header = &parsed.header;
        assert_eq!(header.version, 0x0002_0002, "{}", part.name);
        assert_eq!(header.flags, eot::FLAG_COMPRESSED, "{}", part.name);
        assert_eq!(header.fs_type, 0, "{}", part.name);
        assert!(parsed.warnings.is_empty(), "{}", part.name);
        assert_eq!(parsed.data.first(), Some(&3), "{}: MTX version", part.name);
        let stem = part
            .name
            .trim_start_matches("ppt/fonts/")
            .trim_end_matches(".fntdata");
        let (file_family, slot) = stem.split_once('-').unwrap();
        let expected_family = match file_family {
            "BarlowCondensed" if slot.starts_with("bold") => "Barlow Condensed Black",
            "BarlowCondensed" => "Barlow Condensed",
            "Oswald" | "OswaldMedium" => "Oswald",
            "Roboto" | "RobotoMedium" => "Roboto",
            "RobotoMono" => "Roboto Mono",
            other => panic!("unexpected part {other}"),
        };
        assert_eq!(header.family_name, expected_family, "{}", part.name);
        assert_eq!(header.italic, slot.ends_with("talic"), "{}", part.name);
        let weight = match (file_family, slot.starts_with("bold")) {
            ("BarlowCondensed", true) => 900,
            (_, true) => 700,
            (_, false) => 400,
        };
        assert_eq!(header.weight, weight, "{}", part.name);

        let unwrapped = unwrap(&bytes, &hint).unwrap();
        assert!(unwrapped.warnings.is_empty(), "{}", part.name);
        let sfnt = &unwrapped.sfnt;
        let expected = LIBEOT.iter().find(|(name, _)| *name == stem).unwrap().1;
        let digest = hex::encode(Sha256::digest(sfnt));
        assert!(
            digest == expected,
            "{stem}: not libeot's bytes; tables that differ: {}",
            table_diff(sfnt, stem)
        );
        // A valid font: checksums, every table record, every glyph.
        assert_eq!(checksum(sfnt), 0xb1b0_afba, "{stem}");
        for (tag, (data, sum)) in tables(sfnt, 0) {
            let mut data = data.to_vec();
            if tag == *b"head" {
                data[8..12].fill(0);
            }
            assert_eq!(checksum(&data), sum, "{stem}: {tag:?}");
        }
        assert_eq!(read_sfnt(sfnt).unwrap().len(), 1, "{stem}");
        let font = FontRef::new(sfnt).unwrap();
        let (loca, glyf) = (font.loca(None).unwrap(), font.glyf().unwrap());
        let glyph_count = font.maxp().unwrap().num_glyphs();
        assert_eq!(glyphs(sfnt, 0).len(), usize::from(glyph_count), "{stem}");
        for gid in 0..u32::from(glyph_count) {
            loca.get_glyf(gid.into(), &glyf)
                .unwrap_or_else(|e| panic!("{stem}: glyph {gid}: {e}"));
        }
        for glyph in glyphs(sfnt, 0) {
            glyph_model(&glyph);
        }
        decoded += 1;
    }
    assert_eq!(decoded, 20);
}
