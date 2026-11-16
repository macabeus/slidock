//! The sfnt reader on fonts built in code: the fields it reads, the fallbacks, and every lie it refuses.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_fonts::sfnt::{Region, SfntError, SfntFace, TableTag, read_sfnt};
use slidock_ir::fonts::{FontMetrics, GlyphBounds, VMetrics, WinMetrics};
use support::*;

fn one(bytes: &[u8]) -> SfntFace {
    let mut faces = read_sfnt(bytes).unwrap();
    assert_eq!(faces.len(), 1);
    faces.remove(0)
}

fn with_table(spec: &FaceSpec, tag: &[u8; 4], bytes: Vec<u8>) -> Vec<u8> {
    let mut tables = face_tables(spec);
    tables.retain(|(t, _)| t != tag);
    tables.push((*tag, bytes));
    tables.sort_by_key(|a| a.0);
    assemble(&tables)
}

fn table_of(spec: &FaceSpec, tag: &[u8; 4]) -> Vec<u8> {
    face_tables(spec)
        .into_iter()
        .find(|(t, _)| t == tag)
        .unwrap()
        .1
}

fn without(spec: &FaceSpec, tag: &[u8; 4]) -> Vec<u8> {
    let mut tables = face_tables(spec);
    tables.retain(|(t, _)| t != tag);
    assemble(&tables)
}

#[test]
fn reads_names_weight_style_and_all_three_metric_tables() {
    let face = one(&sfnt(&FaceSpec {
        typographic_family: Some("Roboto Flex".into()),
        weight: Some(500),
        units_per_em: Some(2048),
        hhea: Some([1900, -500, 0]),
        typo: Some([1536, -512, 102]),
        win: Some([2146, 555]),
        use_typo_metrics: true,
        ..FaceSpec::new("Roboto")
    }));
    assert_eq!(
        face,
        SfntFace {
            family: "Roboto Flex".into(),
            subfamily: "Regular".into(),
            full_name: Some("Roboto Regular".into()),
            postscript_name: "Roboto-Regular".into(),
            weight: 500,
            width: 5,
            italic: false,
            italic_angle: 0.0,
            variable: false,
            use_typo_metrics: true,
            metrics: FontMetrics {
                units_per_em: 2048,
                hhea: VMetrics {
                    ascender: 1900,
                    descender: -500,
                    line_gap: 0
                },
                typo: VMetrics {
                    ascender: 1536,
                    descender: -512,
                    line_gap: 102
                },
                win: WinMetrics {
                    ascent: 2146,
                    descent: 555
                },
                bounds: Some(GlyphBounds { y_min: 0, y_max: 0 }),
                grave_top: None,
            },
            fixed_pitch: false,
        }
    );
}

#[test]
fn reads_the_glyph_bounds_and_the_top_of_a_graves_glyph_box() {
    let face = one(&sfnt(&FaceSpec {
        bounds: Some([-985, 2297]),
        grave_top: Some(1874),
        ..FaceSpec::new("Tall")
    }));
    assert_eq!(
        face.metrics.bounds,
        Some(GlyphBounds {
            y_min: -985,
            y_max: 2297
        })
    );
    assert_eq!(face.metrics.grave_top, Some(1874));
    assert_eq!(one(&sfnt(&FaceSpec::new("Plain"))).metrics.grave_top, None);
}

#[test]
fn reads_italics_variable_fonts_and_every_face_of_a_collection() {
    let faces = read_sfnt(&ttc(&[
        FaceSpec {
            subfamily: Some("Regular".into()),
            ..FaceSpec::new("Helvetica Neue")
        },
        FaceSpec {
            subfamily: Some("Bold Italic".into()),
            weight: Some(700),
            italic: true,
            variable: true,
            ..FaceSpec::new("Helvetica Neue")
        },
    ]))
    .unwrap();
    let summary: Vec<_> = faces
        .iter()
        .map(|f| {
            (
                f.postscript_name.as_str(),
                f.weight,
                f.italic,
                f.italic_angle,
                f.variable,
            )
        })
        .collect();
    assert_eq!(
        summary,
        [
            ("HelveticaNeue-Regular", 400, false, 0.0, false),
            ("HelveticaNeue-Bold Italic", 700, true, -12.0, true)
        ]
    );
}

#[test]
fn reads_each_collection_member_s_width_class() {
    let member = |subfamily: &str, weight: u16, width: Option<u16>| FaceSpec {
        subfamily: Some(subfamily.into()),
        weight: Some(weight),
        width,
        ..FaceSpec::new("Helvetica Neue")
    };
    let faces = read_sfnt(&ttc(&[
        member("Bold", 700, None),
        member("Condensed Bold", 700, Some(3)),
        member("Odd", 400, Some(0)),
        member("Wide", 400, Some(12)),
    ]))
    .unwrap();
    let widths: Vec<_> = faces.iter().map(|f| (f.weight, f.width)).collect();
    assert_eq!(widths, [(700, 5), (700, 3), (400, 0), (400, 12)]);
    // A font without an OS/2 table states no width: normal, as CSS reads an unstated stretch.
    let old = one(&sfnt(&FaceSpec {
        no_os2: true,
        ..FaceSpec::new("Old")
    }));
    assert_eq!(old.width, 5);
    // The width is not part of the serialized face.
    let printed = serde_json::to_value(&faces[1]).unwrap();
    assert!(printed.get("width").is_none());
}

#[test]
fn falls_back_to_hhea_metrics_without_an_os2_table() {
    let face = one(&sfnt(&FaceSpec {
        no_os2: true,
        hhea: Some([800, -200, 90]),
        ..FaceSpec::new("Old")
    }));
    assert_eq!(
        face.metrics.typo,
        VMetrics {
            ascender: 800,
            descender: -200,
            line_gap: 90
        }
    );
    assert_eq!(
        face.metrics.win,
        WinMetrics {
            ascent: 800,
            descent: 200
        }
    );
    assert_eq!(face.weight, 400);
    let inverted = one(&sfnt(&FaceSpec {
        no_os2: true,
        hhea: Some([-5, 7, 0]),
        ..FaceSpec::new("Odd")
    }));
    assert_eq!(
        inverted.metrics.win,
        WinMetrics {
            ascent: 0,
            descent: 0
        }
    );
}

#[test]
fn an_os2_table_shorter_than_78_bytes_counts_as_absent() {
    let spec = FaceSpec {
        weight: Some(700),
        use_typo_metrics: true,
        hhea: Some([900, -100, 10]),
        ..FaceSpec::new("Short")
    };
    let mut os2 = table_of(&spec, b"OS/2");
    os2.truncate(77);
    // Read as the OS/2 table slidock writes for a face without one: hhea spaces its lines.
    let face = one(&with_table(&spec, b"OS/2", os2.clone()));
    assert_eq!((face.weight, face.use_typo_metrics), (400, true));
    assert_eq!(face.metrics.typo.ascender, 900);
    os2.push(0);
    let face = one(&with_table(&spec, b"OS/2", os2));
    assert_eq!((face.weight, face.use_typo_metrics), (700, true));
}

#[test]
fn italic_comes_from_fs_selection_mac_style_or_the_italic_angle() {
    let spec = FaceSpec::new("Slant");
    let mut os2 = table_of(&spec, b"OS/2");
    os2[63] = 0x01;
    assert!(one(&with_table(&spec, b"OS/2", os2)).italic);
    let mut head = table_of(&spec, b"head");
    head[45] = 0x02;
    assert!(one(&with_table(&spec, b"head", head)).italic);
    let mut post = table_of(&spec, b"post");
    post[4..8].copy_from_slice(&(-65536i32 / 4).to_be_bytes());
    let face = one(&with_table(&spec, b"post", post.clone()));
    assert_eq!((face.italic, face.italic_angle), (true, -0.25));
    // A backslant is a slant too.
    let mut backslant = post.clone();
    backslant[4..8].copy_from_slice(&(65536i32 / 4).to_be_bytes());
    let face = one(&with_table(&spec, b"post", backslant));
    assert_eq!((face.italic, face.italic_angle), (true, 0.25));
    post.truncate(7);
    let face = one(&with_table(&spec, b"post", post));
    assert_eq!((face.italic, face.italic_angle), (false, 0.0));
    assert!(!one(&sfnt(&spec)).italic);
}

#[test]
fn a_subfamily_naming_a_slant_makes_a_face_without_style_bits_italic() {
    // HelveticaNeue.ttc's Medium Italic: no italic bit, an italicAngle of 0, the subfamily "Medium Italic".
    let named = |subfamily: &str, typographic: Option<&str>| {
        let spec = FaceSpec {
            subfamily: Some(subfamily.into()),
            weight: Some(500),
            ..FaceSpec::new("Helvetica Neue")
        };
        let mut faces = face_tables(&spec);
        if let Some(typographic) = typographic {
            let names = [
                windows_name(1, "Helvetica Neue"),
                windows_name(2, subfamily),
                windows_name(6, "HelveticaNeue-Face"),
                windows_name(17, typographic),
            ];
            faces.retain(|(tag, _)| tag != b"name");
            faces.push((*b"name", name_table(&names)));
            faces.sort_by_key(|a| a.0);
        }
        let face = one(&assemble(&faces));
        (face.italic, face.italic_angle)
    };
    assert_eq!(named("Medium Italic", None), (true, 0.0));
    assert_eq!(named("BoldOblique", None), (true, 0.0));
    // Name id 17 decides when the font has one.
    assert_eq!(named("Regular", Some("Medium Italic")), (true, 0.0));
    assert_eq!(named("Italic", Some("Medium")), (false, 0.0));
    // Negative control: an upright subfamily stays upright.
    assert_eq!(named("Medium", None), (false, 0.0));
    let printed = serde_json::to_value(one(&sfnt(&FaceSpec {
        subfamily: Some("Medium Italic".into()),
        ..FaceSpec::new("Helvetica Neue")
    })))
    .unwrap();
    assert_eq!(printed["italic"], true);
}

#[test]
fn a_face_is_fixed_pitch_when_its_ascii_characters_advance_by_one_width() {
    let face = |advances: Vec<u16>| {
        one(&sfnt(&FaceSpec {
            advances: Some(advances),
            ..FaceSpec::new("PragmataPro")
        }))
    };
    let fixed = face(vec![1229; 95]);
    assert!(fixed.fixed_pitch);
    // Ninety of the 95 printable characters suffice.
    assert!(face(vec![600; 90]).fixed_pitch);
    // The pitch is not part of the serialized face.
    assert!(
        serde_json::to_value(&fixed)
            .unwrap()
            .get("fixedPitch")
            .is_none()
    );
    // Negative controls: one other advance, too few characters mapped, no hmtx table, no cmap.
    let mut proportional = vec![1229; 95];
    proportional[40] = 1230;
    assert!(!face(proportional).fixed_pitch);
    assert!(!face(vec![600; 89]).fixed_pitch);
    let spec = FaceSpec {
        advances: Some(vec![600; 95]),
        ..FaceSpec::new("PragmataPro")
    };
    assert!(!one(&without(&spec, b"hmtx")).fixed_pitch);
    assert!(!one(&without(&spec, b"cmap")).fixed_pitch);
    assert!(!one(&sfnt(&FaceSpec::new("Plain"))).fixed_pitch);
    // Each face of a collection is judged on its own.
    let faces = read_sfnt(&ttc(&[
        FaceSpec {
            advances: Some(vec![500; 95]),
            ..FaceSpec::new("Mixed")
        },
        FaceSpec {
            advances: Some((0..95).map(|i| 400 + i).collect()),
            subfamily: Some("Bold".into()),
            ..FaceSpec::new("Mixed")
        },
    ]))
    .unwrap();
    assert_eq!(
        faces.iter().map(|f| f.fixed_pitch).collect::<Vec<_>>(),
        [true, false]
    );
}

#[test]
fn normalizes_the_weight_class() {
    for (class, weight) in [
        (5u16, 500u32),
        (0, 100),
        (1000, 900),
        (350, 400),
        (649, 600),
    ] {
        let face = one(&sfnt(&FaceSpec {
            weight: Some(class),
            ..FaceSpec::new("W")
        }));
        assert_eq!(face.weight, weight, "usWeightClass {class}");
    }
}

#[test]
fn ranks_names_windows_english_then_windows_then_unicode_then_mac() {
    let spec = FaceSpec::new("Unused");
    let utf16 = |s: &str| {
        s.encode_utf16()
            .flat_map(u16::to_be_bytes)
            .collect::<Vec<u8>>()
    };
    let names = |records: Vec<NameRecord>| {
        let face = one(&with_table(&spec, b"name", name_table(&records)));
        face.family
    };
    let mac = (1, 0, 0, 1, b"Mac".to_vec());
    let unicode = (0, 3, 0, 1, utf16("Unicode"));
    let windows = (3, 10, 0x407, 1, utf16("Windows"));
    let english = (3, 1, 0x409, 1, utf16("English"));
    let symbol = (3, 0, 0x409, 1, utf16("Symbol"));
    let other = (2, 0, 0, 1, b"Iso".to_vec());
    assert_eq!(
        names(vec![
            mac.clone(),
            unicode.clone(),
            windows.clone(),
            english.clone()
        ]),
        "English"
    );
    assert_eq!(
        names(vec![mac.clone(), unicode.clone(), windows.clone()]),
        "Windows"
    );
    assert_eq!(
        names(vec![mac.clone(), unicode.clone(), symbol.clone()]),
        "Unicode"
    );
    assert_eq!(
        names(vec![other.clone(), symbol.clone(), mac.clone()]),
        "Mac"
    );
    let second = (3, 1, 0x409, 1, utf16("Second"));
    assert_eq!(names(vec![english, second]), "English");
    let mac_latin1 = (1, 0, 0, 1, vec![b'C', 0xe9]);
    assert_eq!(names(vec![mac_latin1]), "C\u{e9}");
    let err = read_sfnt(&with_table(&spec, b"name", name_table(&[other, symbol]))).unwrap_err();
    assert_eq!(err, SfntError::NoFamilyName);
}

#[test]
fn prefers_typographic_names_and_falls_back_for_the_rest() {
    let spec = FaceSpec::new("Unused");
    let face = |records: &[NameRecord]| read_sfnt(&with_table(&spec, b"name", name_table(records)));
    let only_family = face(&[windows_name(1, "Open Sans\u{e9}")])
        .unwrap()
        .remove(0);
    assert_eq!(only_family.subfamily, "Regular");
    assert_eq!(only_family.full_name, None);
    assert_eq!(only_family.postscript_name, "OpenSans");
    let typographic = face(&[
        windows_name(1, "Open Sans Light"),
        windows_name(2, "Regular"),
        windows_name(16, "Open Sans"),
        windows_name(17, "Light"),
    ])
    .unwrap()
    .remove(0);
    assert_eq!(
        (typographic.family.as_str(), typographic.subfamily.as_str()),
        ("Open Sans", "Light")
    );
    let empty_typographic = face(&[windows_name(1, "Family"), windows_name(16, "")]);
    assert_eq!(empty_typographic.unwrap_err(), SfntError::NoFamilyName);
    // An empty typographic family hides id 1 even when id 6 names the face.
    let empty_typographic = face(&[
        windows_name(1, "Family"),
        windows_name(6, "Family-Regular"),
        windows_name(16, ""),
    ]);
    assert_eq!(empty_typographic.unwrap_err(), SfntError::NoFamilyName);
    let empty_postscript = face(&[windows_name(1, "Family"), windows_name(6, "")]);
    assert_eq!(empty_postscript.unwrap_err(), SfntError::NoFamilyName);
    let unprintable = face(&[windows_name(1, "\u{e9}\u{e9}")]);
    assert_eq!(unprintable.unwrap_err(), SfntError::NoFamilyName);
    assert_eq!(face(&[]).unwrap_err(), SfntError::NoFamilyName);
}

#[test]
fn name_records_must_lie_in_the_table_and_strings_in_the_file() {
    let spec = FaceSpec::new("Names");
    let mut name = name_table(&[windows_name(1, "Ok")]);
    name.truncate(17);
    let err = read_sfnt(&with_table(&spec, b"name", name)).unwrap_err();
    assert_eq!(err, SfntError::TruncatedTable(TableTag(*b"name")));

    // A string of a skipped platform past the end of the file still fails the face.
    let mut name = name_table(&[windows_name(1, "Ok"), (2, 0, 0, 9, vec![0; 4])]);
    let second_offset = 6 + 12 + 10;
    name[second_offset..second_offset + 2].copy_from_slice(&u16be(0xfff0));
    let err = read_sfnt(&with_table(&spec, b"name", name)).unwrap_err();
    assert_eq!(err, SfntError::OutsideFile(Region::NameString));
}

#[test]
fn a_name_string_may_run_past_its_table_while_inside_the_file() {
    let spec = FaceSpec::new("Unused");
    let mut tables = face_tables(&spec);
    let index = tables.iter().position(|(t, _)| t == b"name").unwrap();
    tables[index].1 = name_table(&[windows_name(1, "Spill")]);
    let mut bytes = assemble(&tables);
    let length_at = 12 + index * 16 + 12;
    let length = u32::from_be_bytes(bytes[length_at..length_at + 4].try_into().unwrap());
    bytes[length_at..length_at + 4].copy_from_slice(&u32be(length - 2));
    assert_eq!(one(&bytes).family, "Spill");
}

#[test]
fn requires_head_hhea_and_name_at_their_minimum_lengths() {
    let spec = FaceSpec::new("Need");
    for (tag, min) in [(b"head", 54), (b"hhea", 36), (b"name", 6)] {
        let err = read_sfnt(&without(&spec, tag)).unwrap_err();
        assert_eq!(err, SfntError::MissingTable(TableTag(*tag)));
        let mut short = table_of(&spec, tag);
        short.truncate(min - 1);
        let err = read_sfnt(&with_table(&spec, tag, short)).unwrap_err();
        assert_eq!(err, SfntError::TruncatedTable(TableTag(*tag)));
    }
    assert_eq!(
        read_sfnt(&without(&spec, b"head")).unwrap_err().to_string(),
        "The font has no head table"
    );
}

#[test]
fn units_per_em_must_be_between_16_and_16384() {
    for (units, ok) in [(15, false), (16, true), (16_384, true), (16_385, false)] {
        let result = read_sfnt(&sfnt(&FaceSpec {
            units_per_em: Some(units),
            ..FaceSpec::new("Em")
        }));
        match ok {
            true => assert_eq!(result.unwrap()[0].metrics.units_per_em, u32::from(units)),
            false => assert_eq!(result.unwrap_err(), SfntError::UnitsPerEm(units)),
        }
    }
}

#[test]
fn accepts_the_three_sfnt_versions_only() {
    let bytes = sfnt(&FaceSpec::new("V"));
    for (version, ok) in [
        (0x0001_0000u32, true),
        (0x4f54_544f, true),
        (0x7472_7565, true),
        (0x0002_0000, false),
    ] {
        let mut copy = bytes.clone();
        copy[..4].copy_from_slice(&u32be(version));
        match ok {
            true => assert_eq!(one(&copy).family, "V"),
            false => assert_eq!(read_sfnt(&copy).unwrap_err(), SfntError::NotSfnt),
        }
    }
    let err = read_sfnt(b"not a font at all").unwrap_err();
    assert_eq!(err, SfntError::NotSfnt);
    assert_eq!(err.to_string(), "Not a TrueType or OpenType font");
    assert_eq!(
        read_sfnt(b"ab").unwrap_err(),
        SfntError::OutsideFile(Region::FontData)
    );
}

#[test]
fn every_table_record_must_lie_inside_the_file() {
    let spec = FaceSpec::new("Cut");
    let bytes = sfnt(&spec);
    let err = read_sfnt(&bytes[..bytes.len() - 40]).unwrap_err();
    assert!(
        matches!(err, SfntError::OutsideFile(Region::Table(_))),
        "{err:?}"
    );
    // An unused table out of the file fails the face too.
    let mut tables = face_tables(&spec);
    tables.push((*b"zzzz", vec![0; 8]));
    let mut bytes = assemble(&tables);
    let record = 12 + (tables.len() - 1) * 16;
    bytes[record + 8..record + 12].copy_from_slice(&u32be(u32::MAX));
    let err = read_sfnt(&bytes).unwrap_err();
    assert_eq!(
        err,
        SfntError::OutsideFile(Region::Table(TableTag(*b"zzzz")))
    );
    assert_eq!(err.to_string(), "The zzzz table is outside the file");
}

#[test]
fn a_table_may_end_exactly_at_the_end_of_the_file() {
    let mut tables = face_tables(&FaceSpec::new("Edge"));
    tables.push((*b"zzzz", vec![0; 8]));
    let mut bytes = assemble(&tables);
    assert_eq!(one(&bytes).family, "Edge");
    let length_at = 12 + (tables.len() - 1) * 16 + 12;
    bytes[length_at..length_at + 4].copy_from_slice(&u32be(9));
    assert_eq!(
        read_sfnt(&bytes).unwrap_err(),
        SfntError::OutsideFile(Region::Table(TableTag(*b"zzzz")))
    );
}

#[test]
fn refuses_more_than_512_tables() {
    let mut bytes = sfnt(&FaceSpec::new("Many"));
    bytes[4..6].copy_from_slice(&u16be(513));
    assert_eq!(read_sfnt(&bytes).unwrap_err(), SfntError::TableCount(513));
}

#[test]
fn a_tag_listed_twice_names_its_last_record() {
    let spec = FaceSpec::new("Twice");
    let mut tables = face_tables(&spec);
    let mut heavy = table_of(&spec, b"OS/2");
    heavy[4..6].copy_from_slice(&u16be(800));
    tables.push((*b"OS/2", heavy));
    assert_eq!(one(&assemble(&tables)).weight, 800);
}

#[test]
fn collections_hold_1_to_256_faces() {
    let bytes = ttc(&[FaceSpec::new("A"), FaceSpec::new("B")]);
    let families: Vec<_> = read_sfnt(&bytes)
        .unwrap()
        .into_iter()
        .map(|f| f.family)
        .collect();
    assert_eq!(families, ["A", "B"]);
    for count in [0u32, 257] {
        let mut copy = bytes.clone();
        copy[8..12].copy_from_slice(&u32be(count));
        assert_eq!(read_sfnt(&copy).unwrap_err(), SfntError::FaceCount(count));
    }
    let mut copy = bytes.clone();
    copy[8..12].copy_from_slice(&u32be(3));
    assert!(read_sfnt(&copy).is_err());
    assert_eq!(
        read_sfnt(b"ttcf\0\x01\0\0").unwrap_err(),
        SfntError::OutsideFile(Region::FontData)
    );
}

fn grave_spec() -> FaceSpec {
    FaceSpec {
        grave_top: Some(1500),
        ..FaceSpec::new("Grave")
    }
}

fn grave_with_cmap(cmap: Vec<u8>) -> Result<Option<i32>, SfntError> {
    read_sfnt(&with_table(&grave_spec(), b"cmap", cmap)).map(|faces| faces[0].metrics.grave_top)
}

#[test]
fn finds_the_grave_through_unicode_subtables_only() {
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 1, format4_grave(1))])),
        Ok(Some(1500))
    );
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 10, format4_grave(1))])),
        Ok(Some(1500))
    );
    assert_eq!(
        grave_with_cmap(cmap_table(&[(0, 4, format4_grave(1))])),
        Ok(Some(1500))
    );
    assert_eq!(
        grave_with_cmap(cmap_table(&[(1, 0, format4_grave(1))])),
        Ok(None)
    );
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 0, format4_grave(1))])),
        Ok(None)
    );
    // A later Unicode subtable is tried when an earlier one does not map À.
    let mut other = format4_grave(1);
    other[14..16].copy_from_slice(&u16be(0x41));
    other[20..22].copy_from_slice(&u16be(0x41));
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 1, other), (0, 3, format4_grave(1))])),
        Ok(Some(1500))
    );
    assert_eq!(grave_with_cmap(vec![0, 0, 0, 0]), Ok(None));
    assert_eq!(grave_with_cmap(vec![0, 0, 0]), Ok(None));
}

#[test]
fn a_cmap_record_past_the_table_ends_the_search() {
    let mut truncated = cmap_table(&[(3, 1, vec![])]);
    truncated.truncate(10);
    assert_eq!(grave_with_cmap(truncated), Ok(None));
}

#[test]
fn a_subtable_outside_the_file_fails_the_face() {
    let mut cmap = cmap_table(&[(3, 1, format4_grave(1))]);
    cmap[8..12].copy_from_slice(&u32be(0x7fff_0000));
    assert_eq!(
        grave_with_cmap(cmap),
        Err(SfntError::OutsideFile(Region::FontData))
    );
}

fn format4_ranged(glyph_in_array: u16, delta: u16) -> Vec<u8> {
    // One segment U+00C0–U+00C0 reading its glyph from the glyph array, then the 0xFFFF segment.
    let mut sub = Vec::new();
    for field in [
        4u16,
        0,
        0,
        4,
        4,
        1,
        0,
        0xc0,
        0xffff,
        0,
        0xc0,
        0xffff,
        delta,
        1,
        4,
        0,
        glyph_in_array,
    ] {
        sub.extend(u16be(field));
    }
    sub
}

#[test]
fn format4_reads_the_glyph_array_and_a_zero_there_ends_the_search() {
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 1, format4_ranged(1, 0))])),
        Ok(Some(1500))
    );
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 1, format4_ranged(0xffff, 2))])),
        Ok(Some(1500))
    );
    let zero = cmap_table(&[(3, 1, format4_ranged(0, 1)), (0, 3, format4_grave(1))]);
    assert_eq!(grave_with_cmap(zero), Ok(None));
}

#[test]
fn format4_stops_at_the_first_segment_ending_at_or_after_the_code_point() {
    // Unsorted segments: U+00C1–U+00C5 comes first and starts past À, so the later segment that covers À is
    // never read.
    let mut sub = Vec::new();
    for field in [
        4u16,
        32,
        0,
        4,
        4,
        1,
        0,
        0xc5,
        0xffff,
        0,
        0xc1,
        0xc0,
        0,
        1u16.wrapping_sub(0xc0),
        0,
        0,
    ] {
        sub.extend(u16be(field));
    }
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 1, sub.clone())])),
        Ok(None)
    );
    // The next Unicode subtable is still tried.
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 1, sub), (0, 3, format4_grave(1))])),
        Ok(Some(1500))
    );
}

#[test]
fn format4_with_an_odd_segment_count_reads_at_the_offsets_it_implies() {
    // segCountX2 = 3 puts the start codes, deltas and range offsets at odd positions; the range offset read
    // there (0x4100) points outside the file.
    let mut sub = format4_grave(1);
    sub[6..8].copy_from_slice(&u16be(3));
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 1, sub)])),
        Err(SfntError::OutsideFile(Region::FontData))
    );
    // segCountX2 = 1 is half a segment, which is still read: start 0 at byte 17 and the range offset 0x4100 at
    // byte 19, which points outside the file. Rounding the count down would skip the subtable.
    let mut half = Vec::new();
    for field in [4u16, 24, 0, 1, 0, 0, 0, 0xc0, 0, 0x0041, 0, 0] {
        half.extend(u16be(field));
    }
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 1, half)])),
        Err(SfntError::OutsideFile(Region::FontData))
    );
}

fn format12(groups: &[(u32, u32, u32)]) -> Vec<u8> {
    let mut sub = Vec::new();
    sub.extend(u16be(12));
    sub.extend(u16be(0));
    sub.extend(u32be(16 + 12 * groups.len() as u32));
    sub.extend(u32be(0));
    sub.extend(u32be(groups.len() as u32));
    for &(first, last, glyph) in groups {
        sub.extend(u32be(first));
        sub.extend(u32be(last));
        sub.extend(u32be(glyph));
    }
    sub
}

#[test]
fn format12_maps_through_its_groups_and_refuses_huge_counts() {
    assert_eq!(
        grave_with_cmap(cmap_table(&[(
            3,
            10,
            format12(&[(0x41, 0x5a, 7), (0xbf, 0xc1, 0)])
        )])),
        Ok(Some(1500))
    );
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 10, format12(&[(0x41, 0x5a, 7)]))])),
        Ok(None)
    );
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 10, format12(&[(0xc0, 0xc0, 2)]))])),
        Ok(None)
    );
    let mut huge = format12(&[]);
    huge[12..16].copy_from_slice(&u32be((1 << 20) + 1));
    assert_eq!(
        grave_with_cmap(cmap_table(&[(3, 10, huge)])),
        Err(SfntError::CmapGroups((1 << 20) + 1))
    );
}

#[test]
fn the_glyph_box_needs_maxp_loca_glyf_and_a_10_byte_header() {
    let spec = grave_spec();
    for tag in [b"maxp", b"loca", b"glyf"] {
        assert_eq!(one(&without(&spec, tag)).metrics.grave_top, None, "{tag:?}");
    }
    let mut maxp = table_of(&spec, b"maxp");
    maxp[4..6].copy_from_slice(&u16be(1));
    assert_eq!(
        one(&with_table(&spec, b"maxp", maxp)).metrics.grave_top,
        None
    );
    let mut loca = table_of(&spec, b"loca");
    loca[4..6].copy_from_slice(&u16be(4));
    assert_eq!(
        one(&with_table(&spec, b"loca", loca)).metrics.grave_top,
        None
    );
    let mut loca = table_of(&spec, b"loca");
    loca.truncate(4);
    assert_eq!(
        one(&with_table(&spec, b"loca", loca)).metrics.grave_top,
        None
    );
    let mut glyf = table_of(&spec, b"glyf");
    glyf.truncate(8);
    assert_eq!(
        one(&with_table(&spec, b"glyf", glyf)).metrics.grave_top,
        None
    );
}

#[test]
fn reads_long_loca_offsets_when_head_says_so() {
    let spec = grave_spec();
    let mut head = table_of(&spec, b"head");
    head[50..52].copy_from_slice(&u16be(1));
    let mut loca = Vec::new();
    for offset in [0u32, 0, 10] {
        loca.extend(u32be(offset));
    }
    let mut tables = face_tables(&spec);
    for (tag, bytes) in &mut tables {
        match &*tag {
            b"head" => *bytes = head.clone(),
            b"loca" => *bytes = loca.clone(),
            _ => {}
        }
    }
    assert_eq!(one(&assemble(&tables)).metrics.grave_top, Some(1500));
    // Read as short offsets, the same loca puts the glyph nowhere.
    let mut short = tables.clone();
    for (tag, bytes) in &mut short {
        if tag == b"head" {
            *bytes = table_of(&spec, b"head");
        }
    }
    assert_eq!(one(&assemble(&short)).metrics.grave_top, None);
}

#[test]
fn refuses_other_files_and_never_panics_on_mutations() {
    let single = sfnt(&FaceSpec {
        grave_top: Some(1200),
        ..FaceSpec::new("Cut")
    });
    let collection = ttc(&[FaceSpec::new("A"), FaceSpec::new("B")]);
    let mut random = Seeded::new(0xf0);
    let mut parsed = 0;
    for i in 0..fuzz_iterations() {
        let input = mutate(if i % 2 == 1 { &single } else { &collection }, &mut random);
        if read_sfnt(&input).is_ok() {
            parsed += 1;
        }
    }
    assert!(parsed > 0, "no mutation parsed");
}
