//! The output side on real fonts: subsetting, WOFF2 encoding, the fallback ladder, true `@font-face` descriptors
//! and byte stability. `tests/oracle.rs` (feature `font-oracle`) runs the same outputs through ots-sanitize and
//! fontTools.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_fonts::face_css::FontFormat;
use slidock_fonts::glyphs::ALWAYS_DRAWN;
use slidock_fonts::sfnt::{
    SanitizerRefusal, SfntError, TableTag, covered, face_file, read_details, sanitizer_refusal,
};
use slidock_fonts::subset::{SubsetError, SubsetOptions, subset_face};
use slidock_fonts::woff2_out::{EncodeError, encode_woff2};
use slidock_fonts::{
    CodepointSet, FontLimits, FontNotice, FontOrigin, ShipError, ShipInput, ShipOptions,
    ShippedFont, WeightRange, read_sfnt, ship_face,
};
use slidock_ir::fonts::FaceStyle;
use support::fixtures::*;
use support::*;

const TEXT: &str = "The quick brown fox jumps over the lazy dog. Ünïcödé — “fi” 0123";

fn text(text: &str) -> CodepointSet {
    let mut set = CodepointSet::new();
    set.insert_text(text);
    set
}

fn ship_with(family: &str, bytes: &[u8], index: u32, codepoints: &CodepointSet) -> ShippedFont {
    ship_face(
        &ShipInput {
            family,
            bytes,
            index,
            origin: FontOrigin::Google,
            codepoints,
            small_caps: false,
            source_licence: None,
        },
        &ShipOptions::default(),
    )
    .unwrap()
}

fn ship(bytes: &[u8]) -> ShippedFont {
    ship_with("Test", bytes, 0, &text(TEXT))
}

fn glyph_count(font: &[u8]) -> u16 {
    read_details(font, 0).unwrap().glyphs
}

fn all(set: &CodepointSet) -> CodepointSet {
    let mut set = set.clone();
    for point in ALWAYS_DRAWN {
        set.insert(point);
    }
    set
}

#[test]
fn subsets_keep_the_drawn_code_points_and_drop_the_rest() {
    let wanted = text("Hello");
    for (name, font) in truetype() {
        let subset = subset_face(font, 0, &wanted, &SubsetOptions::default()).unwrap();
        let kept = covered(&subset, 0, &all(&wanted)).unwrap();
        assert_eq!(
            kept,
            covered(font, 0, &all(&wanted)).unwrap(),
            "{name} keeps every drawn code point it maps"
        );
        let others = text("XYZ");
        assert!(!covered(font, 0, &others).unwrap().is_empty(), "{name}");
        assert!(
            covered(&subset, 0, &others).unwrap().is_empty(),
            "{name} drops what the deck does not draw"
        );
        assert!(subset.len() < font.len(), "{name}");
        assert!(glyph_count(&subset) < glyph_count(font), "{name}");
    }
}

#[test]
fn subsets_measure_as_the_full_face() {
    for (name, font) in truetype() {
        let full = &read_sfnt(font).unwrap()[0];
        let subset = subset_face(font, 0, &text("Hi"), &SubsetOptions::default()).unwrap();
        let shipped = &read_sfnt(&subset).unwrap()[0];
        assert_eq!(shipped.metrics, full.metrics, "{name}");
        assert_eq!(
            (
                &shipped.family,
                shipped.weight,
                shipped.italic,
                shipped.variable
            ),
            (&full.family, full.weight, full.italic, full.variable),
            "{name}"
        );
        assert_eq!(shipped.postscript_name, full.postscript_name, "{name}");
    }
}

#[test]
fn subsets_keep_the_layout_closure_and_small_caps_on_request() {
    let wanted = text("fi office");
    let mapped = covered(BARLOW_ITALIC, 0, &all(&wanted)).unwrap().len() as u16;
    let plain = subset_face(BARLOW_ITALIC, 0, &wanted, &SubsetOptions::default()).unwrap();
    // .notdef, one glyph per mapped code point, and the ligatures and alternates the default features reach.
    assert!(glyph_count(&plain) > mapped + 1);
    let small_caps = SubsetOptions {
        small_caps: true,
        ..SubsetOptions::default()
    };
    let with_smcp = subset_face(BARLOW_ITALIC, 0, &wanted, &small_caps).unwrap();
    assert!(glyph_count(&with_smcp) > glyph_count(&plain));
}

#[test]
fn a_cff_face_ships_whole() {
    let shipped = ship(OSWALD_CFF);
    assert_eq!(
        shipped.notices,
        [FontNotice::SubsetFailed(SubsetError::CffOutlines)]
    );
    assert_eq!(shipped.notices[0].code(), "MID5418");
    assert!(!shipped.subset);
    assert_eq!(shipped.rule.format, FontFormat::Woff2);
    assert_eq!(
        shipped.rule.bytes,
        encode_woff2(OSWALD_CFF, &FontLimits::default()).unwrap()
    );
}

#[test]
fn a_face_that_forbids_subsetting_ships_whole() {
    let forbidding = with_fs_type(OSWALD, 0x0100);
    let shipped = ship(&forbidding);
    assert_eq!(shipped.notices, [FontNotice::NoSubsetting]);
    assert!(!shipped.subset);
    assert_eq!(
        shipped.rule.bytes,
        encode_woff2(&forbidding, &FontLimits::default()).unwrap()
    );
    // The same face with the bit clear is subset.
    let allowed = ship(&with_fs_type(OSWALD, 0x0000));
    assert!(allowed.subset && allowed.notices.is_empty());
    assert!(allowed.rule.bytes.len() < shipped.rule.bytes.len());
}

#[test]
fn a_face_the_encoder_refuses_ships_as_its_raw_sfnt() {
    // Apple's `true` version tag: the WOFF2 encoder takes only 1.0 and `OTTO`. Not subset, so the tag survives.
    let mut apple = with_fs_type(OSWALD, 0x0100);
    apple[..4].copy_from_slice(b"true");
    let shipped = ship(&apple);
    assert!(matches!(
        shipped.notices.as_slice(),
        [
            FontNotice::NoSubsetting,
            FontNotice::EncodeFailed(EncodeError::Failed(_))
        ]
    ));
    assert_eq!(shipped.notices[1].code(), "MID5419");
    assert_eq!(shipped.rule.format, FontFormat::TrueType);
    assert_eq!(shipped.rule.bytes, apple);
    let css = shipped.rule.css();
    assert!(css.contains("src:url(data:font/ttf;base64,"), "{css}");
    assert!(css.contains(" format(\"truetype\")"), "{css}");
}

#[test]
fn unreadable_bytes_do_not_ship() {
    for bytes in [&b""[..], b"not a font at all", &OSWALD[..200]] {
        let error = ship_face(
            &ShipInput {
                family: "Test",
                bytes,
                index: 0,
                origin: FontOrigin::User,
                codepoints: &text("a"),
                small_caps: false,
                source_licence: None,
            },
            &ShipOptions::default(),
        )
        .unwrap_err();
        assert_eq!(error.code(), "MID5419");
        assert!(matches!(error, ShipError::Unreadable(_)));
    }
}

#[test]
fn a_collection_face_ships_as_a_font_of_its_own() {
    let ttc = collection(&[OSWALD, SILKSCREEN]);
    let face = face_file(&ttc, 1, &FontLimits::default()).unwrap();
    assert_eq!(read_sfnt(&face).unwrap(), read_sfnt(SILKSCREEN).unwrap());
    let wanted = text(TEXT);
    assert_eq!(
        ship_with("Test", &ttc, 1, &wanted),
        ship_with("Test", SILKSCREEN, 0, &wanted)
    );
    assert_eq!(
        ship_with("Test", &ttc, 0, &wanted),
        ship_with("Test", OSWALD, 0, &wanted)
    );
    assert!(face_file(&ttc, 2, &FontLimits::default()).is_err());
}

fn table_of(font: &[u8], tag: &[u8; 4]) -> Option<Vec<u8>> {
    let font = read_fonts::FontRef::new(font).unwrap();
    font.table_data(read_fonts::types::Tag::new(tag))
        .map(|data| data.as_bytes().to_vec())
}

/// A `morx` or `kerx` table with no chains or subtables: enough for its presence to count.
fn apple_table() -> Vec<u8> {
    [0, 2, 0, 0, 0, 0, 0, 0].to_vec()
}

mod shaper_tables {
    use super::*;

    #[test]
    fn a_face_kerned_by_the_legacy_kern_table_keeps_it_and_its_glyph_ids() {
        let font = kern_only(OSWALD);
        let wanted = text("AVA");
        let subset = subset_face(&font, 0, &wanted, &SubsetOptions::default()).unwrap();
        assert_eq!(table_of(&subset, b"kern"), table_of(&font, b"kern"));
        assert_eq!(glyph_of(&subset, 'A'), glyph_of(&font, 'A'));
        assert_eq!(glyph_of(&subset, 'V'), glyph_of(&font, 'V'));
        let shipped = ship(&font);
        assert!(shipped.subset && shipped.notices.is_empty());
        assert!(
            shipped.rule.bytes.len() < encode_woff2(&font, &FontLimits::default()).unwrap().len()
        );
        // Oswald kerns through GPOS: its `kern` feature makes a legacy table redundant, and glyph ids are compacted.
        let both = with_tables(OSWALD, &[(b"kern", table_of(&font, b"kern"))]);
        let subset = subset_face(&both, 0, &wanted, &SubsetOptions::default()).unwrap();
        assert_eq!(table_of(&subset, b"kern"), None);
        assert!(glyph_count(&subset) < glyph_count(&both));
    }

    #[test]
    fn a_face_shaped_by_apple_tables_ships_whole() {
        let morx_only = with_tables(OSWALD, &[(b"GSUB", None), (b"morx", Some(apple_table()))]);
        let shipped = ship(&morx_only);
        assert_eq!(
            shipped.notices,
            [FontNotice::SubsetFailed(SubsetError::AppleLayout(
                "morx".into()
            ))]
        );
        assert_eq!(shipped.notices[0].code(), "MID5418");
        assert!(!shipped.subset);
        // Beside GSUB, the shaper does not read morx: the face subsets.
        let with_gsub = with_tables(OSWALD, &[(b"morx", Some(apple_table()))]);
        assert!(ship(&with_gsub).subset);
        // kerx is read unless GSUB and a kerning GPOS stand beside it: Roboto Mono has no GPOS.
        let kerx = |font: &[u8]| with_tables(font, &[(b"kerx", Some(apple_table()))]);
        assert_eq!(
            ship(&kerx(ROBOTO_MONO)).notices,
            [FontNotice::SubsetFailed(SubsetError::AppleLayout(
                "kerx".into()
            ))]
        );
        assert!(ship(&kerx(OSWALD)).subset);
    }
}

#[test]
fn a_face_named_only_on_the_mac_platform_subsets() {
    let font = mac_names_only(OSWALD);
    let shipped = ship(&font);
    assert!(shipped.subset, "{:?}", shipped.notices);
    assert!(shipped.notices.is_empty());
    assert_eq!(shipped.face.family, read_sfnt(OSWALD).unwrap()[0].family);
}

#[test]
fn faces_browsers_would_refuse_do_not_ship() {
    for (name, font) in ALL {
        assert_eq!(sanitizer_refusal(font, 0), None, "{name}");
    }
    let refused = |font: &[u8]| {
        ship_face(
            &ShipInput {
                family: "Test",
                bytes: font,
                index: 0,
                origin: FontOrigin::Local,
                codepoints: &text("Hello"),
                small_caps: false,
                source_licence: None,
            },
            &ShipOptions::default(),
        )
        .unwrap_err()
    };
    let no_os2 = with_tables(OSWALD, &[(b"OS/2", None)]);
    let error = refused(&no_os2);
    assert_eq!(
        error,
        ShipError::Refused(SanitizerRefusal::MissingTable(TableTag(*b"OS/2")))
    );
    assert_eq!(error.code(), "MID5419");
    assert_eq!(
        refused(&mac_cmap_only(OSWALD)),
        ShipError::Refused(SanitizerRefusal::NoUnicodeCmap)
    );
    assert_eq!(
        refused(&with_tables(OSWALD, &[(b"post", None)])),
        ShipError::Refused(SanitizerRefusal::MissingTable(TableTag(*b"post")))
    );
}

#[test]
fn overlapping_tables_longer_than_the_file_are_refused() {
    // Every extra record covers the whole file: copying the tables would take 300 times its size.
    let bomb = with_overlapping_records(SILKSCREEN, 300, SILKSCREEN.len() as u32);
    assert_eq!(read_sfnt(&bomb).unwrap_err(), SfntError::TablesOverlap);
    assert!(matches!(
        ship_face(
            &ShipInput {
                family: "Bomb",
                bytes: &with_fs_type(&bomb, 0x0100),
                index: 0,
                origin: FontOrigin::User,
                codepoints: &text("a"),
                small_caps: false,
                source_licence: None,
            },
            &ShipOptions::default(),
        ),
        Err(ShipError::Unreadable(SfntError::TablesOverlap))
    ));
    let ttc = collection(&[&bomb]);
    assert_eq!(
        face_file(&ttc, 0, &FontLimits::default()).unwrap_err(),
        SfntError::TablesOverlap
    );
    assert!(encode_woff2(&bomb, &FontLimits::default()).is_err());
    // Records that overlap but together fit in the file still read.
    let (head, _) = table_range(SILKSCREEN, b"head").unwrap();
    let mut shared = with_overlapping_records(SILKSCREEN, 1, 54);
    let record = 12 + (u16::from_be_bytes([shared[4], shared[5]]) as usize - 1) * 16;
    shared[record + 8..record + 12].copy_from_slice(&u32be((head + 16) as u32));
    assert_eq!(read_sfnt(&shared).unwrap(), read_sfnt(SILKSCREEN).unwrap());
}

mod descriptors {
    use super::*;

    fn weight_and_style(shipped: &ShippedFont) -> (WeightRange, FaceStyle) {
        (
            shipped.rule.descriptor.weight,
            shipped.rule.descriptor.style,
        )
    }

    /// A face of the in-code builder that calls itself bold everywhere but in `usWeightClass`, with the `hmtx`
    /// table browsers require.
    fn labelled_bold(weight: u16) -> Vec<u8> {
        let face = sfnt(&FaceSpec {
            subfamily: Some("Bold".into()),
            postscript_name: Some("Brand-Bold".into()),
            weight: Some(weight),
            bold_bits: true,
            grave_top: Some(1400),
            ..FaceSpec::new("Brand")
        });
        with_tables(&face, &[(b"hmtx", Some(vec![0; 4]))])
    }

    #[test]
    fn a_face_labelled_bold_states_its_own_weight() {
        let shipped = ship_with("Brand", &labelled_bold(400), 0, &text("A"));
        assert_eq!(
            weight_and_style(&shipped),
            (WeightRange::point(400), FaceStyle::Normal)
        );
        assert!(
            shipped
                .rule
                .css()
                .contains(";font-weight:400;font-style:normal")
        );
        // The same face with a bold usWeightClass states 700: the descriptor reads the file.
        let bold = ship_with("Brand", &labelled_bold(700), 0, &text("A"));
        assert_eq!(bold.rule.descriptor.weight, WeightRange::point(700));
    }

    #[test]
    fn real_faces_state_their_weight_style_and_axis() {
        assert_eq!(
            weight_and_style(&ship(OSWALD)),
            (WeightRange::point(400), FaceStyle::Normal)
        );
        assert_eq!(
            weight_and_style(&ship(&with_u16(OSWALD, b"OS/2", 4, 700))),
            (WeightRange::point(700), FaceStyle::Normal)
        );
        assert_eq!(
            weight_and_style(&ship(BARLOW_ITALIC)),
            (WeightRange::point(400), FaceStyle::Italic)
        );
        assert_eq!(
            weight_and_style(&ship(OSWALD_VARIABLE)),
            (WeightRange::between(200, 700), FaceStyle::Normal)
        );
        // Noto Sans Hebrew's default instance is Thin (usWeightClass 100); its rule states the whole axis.
        let noto = ship(NOTO_HEBREW);
        assert_eq!(noto.rule.descriptor.weight, WeightRange::between(100, 900));
        assert!(noto.rule.css().contains(";font-weight:100 900;"));
    }

    #[test]
    fn unicode_range_lists_what_the_shipped_file_maps() {
        let wanted = text("Ab שלום");
        let latin = ship_with("Oswald", OSWALD, 0, &wanted);
        assert_eq!(
            latin
                .rule
                .unicode_range
                .as_ref()
                .unwrap()
                .unicode_range()
                .unwrap(),
            "U+20,U+2D,U+41,U+62,U+A0,U+C0,U+2010,U+2026"
        );
        let hebrew = ship_with("Noto Sans Hebrew", NOTO_HEBREW, 0, &wanted);
        let range = hebrew.rule.unicode_range.as_ref().unwrap();
        assert!(range.contains(0x5e9) && range.contains(u32::from('A')));
    }
}

#[test]
fn shipping_is_byte_stable() {
    for (name, font) in ALL {
        let first = ship(font);
        let second = ship(font);
        assert_eq!(first, second, "{name}");
        assert_eq!(first.rule.css(), second.rule.css(), "{name}");
    }
}

#[test]
fn every_fixture_ships_as_woff2() {
    for (name, font) in ALL {
        let shipped = ship(font);
        assert_eq!(shipped.rule.format, FontFormat::Woff2, "{name}");
        assert_eq!(&shipped.rule.bytes[..4], b"wOF2", "{name}");
        assert_eq!(shipped.subset, name.ends_with(".ttf"), "{name}");
    }
}

#[test]
fn limits_bound_every_step() {
    let tiny = FontLimits {
        font_bytes: 1000,
        ..FontLimits::default()
    };
    let options = SubsetOptions {
        small_caps: false,
        limits: tiny,
    };
    assert_eq!(
        subset_face(OSWALD, 0, &text("a"), &options),
        Err(SubsetError::TooLarge(OSWALD.len()))
    );
    assert_eq!(
        encode_woff2(OSWALD, &tiny),
        Err(EncodeError::TooLarge(OSWALD.len()))
    );
    let few = SubsetOptions {
        small_caps: false,
        limits: FontLimits {
            codepoints: 3,
            ..FontLimits::default()
        },
    };
    assert_eq!(
        subset_face(OSWALD, 0, &text("abcd"), &few),
        Err(SubsetError::TooManyCodepoints(4))
    );
    let shipped = ship_face(
        &ShipInput {
            family: "Test",
            bytes: OSWALD,
            index: 0,
            origin: FontOrigin::Google,
            codepoints: &text("abcd"),
            small_caps: false,
            source_licence: None,
        },
        &ShipOptions {
            limits: few.limits,
            ..ShipOptions::default()
        },
    )
    .unwrap();
    assert_eq!(
        shipped.notices,
        [FontNotice::SubsetFailed(SubsetError::TooManyCodepoints(4))]
    );
    assert!(
        ship_face(
            &ShipInput {
                family: "Test",
                bytes: OSWALD,
                index: 0,
                origin: FontOrigin::Google,
                codepoints: &text("a"),
                small_caps: false,
                source_licence: None,
            },
            &ShipOptions {
                limits: tiny,
                ..ShipOptions::default()
            },
        )
        .is_err()
    );
}

/// Mutated fixtures through every entry point: nothing panics, and nothing reaches a panic the steps recover
/// from natively (a wasm build would abort on it).
#[test]
fn mutated_fonts_never_panic() {
    let mut random = Seeded::new(0x5eed_f0a7);
    let wanted = text("Hello, world");
    let mut panicked = 0;
    let iterations = fuzz_iterations() / 4;
    let kern_only = kern_only(OSWALD);
    for i in 0..iterations {
        let sources: [&[u8]; 5] = [
            SILKSCREEN,
            OSWALD_MVAR,
            NOTO_HEBREW,
            BARLOW_ITALIC,
            &kern_only,
        ];
        let source = sources[i % 5];
        let mut bytes = mutate(source, &mut random);
        for _ in 0..random.below(4) {
            bytes = mutate(&bytes, &mut random);
        }
        let shipped = ship_face(
            &ShipInput {
                family: "Fuzz",
                bytes: &bytes,
                index: 0,
                origin: FontOrigin::User,
                codepoints: &wanted,
                small_caps: i % 3 == 0,
                source_licence: None,
            },
            &ShipOptions::default(),
        );
        if let Ok(shipped) = &shipped {
            let recovered = shipped
                .notices
                .iter()
                .filter(|notice| {
                    matches!(
                        notice,
                        FontNotice::SubsetFailed(SubsetError::Panicked)
                            | FontNotice::EncodeFailed(EncodeError::Panicked)
                    )
                })
                .count();
            if recovered > 0
                && let Ok(dir) = std::env::var("FUZZ_DUMP")
            {
                std::fs::write(format!("{dir}/panic-{i}.ttf"), &bytes).unwrap();
            }
            panicked += recovered;
            let _ = shipped.rule.css();
        }
        let _ = slidock_fonts::sfnt::metrics_at(&bytes, 0, 700, &FontLimits::default());
        let _ = read_details(&bytes, 0);
        let _ = covered(&bytes, 0, &wanted);
    }
    eprintln!("{iterations} mutated fonts, {panicked} subsetter or encoder panics recovered");
    // A wasm build cannot recover a panic: the checks before each step must refuse what reaches one.
    assert_eq!(panicked, 0);
}
