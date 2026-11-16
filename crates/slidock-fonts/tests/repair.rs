//! Repairing faces browsers' font sanitizer refuses: a face lacking only `OS/2` gets one written from its other
//! tables, measures as before, passes the sanitizer check, ships, and states restricted embedding.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use read_fonts::TableProvider;
use slidock_fonts::sfnt::{
    FaceRepair, SanitizerRefusal, TableTag, UNSTATED_FS_TYPE, read_details, repair,
    sanitizer_refusal,
};
use slidock_fonts::{
    CodepointSet, Decision, FontLimits, FontOrigin, FsType, LicencePolicy, ShipInput, ShipOptions,
    Target, Toggle, decide, read_sfnt, ship_face,
};
use support::fixtures::*;
use support::*;

/// Oswald without its `OS/2` table: an otherwise sound TrueType face, as old Apple fonts are.
fn no_os2(font: &[u8]) -> Vec<u8> {
    with_tables(font, &[(b"OS/2", None)])
}

/// `font` with `head.macStyle` set.
fn with_mac_style(font: &[u8], mac_style: u16) -> Vec<u8> {
    with_u16(font, b"head", 44, mac_style)
}

fn os2_of(font: &[u8]) -> read_fonts::tables::os2::Os2<'_> {
    read_fonts::FontRef::new(font).unwrap().os2().unwrap()
}

#[test]
fn a_face_lacking_only_os2_gets_one_that_browsers_accept() {
    let broken = no_os2(OSWALD);
    assert_eq!(
        sanitizer_refusal(&broken, 0),
        Some(SanitizerRefusal::MissingTable(TableTag(*b"OS/2")))
    );
    let repaired = repair(&broken, &FontLimits::default()).expect("repaired");
    assert_eq!(repaired.faces, [Some(FaceRepair::Os2)]);
    assert_eq!(sanitizer_refusal(&repaired.bytes, 0), None);

    let os2 = os2_of(&repaired.bytes);
    assert_eq!(os2.version(), 4);
    assert_eq!(os2.offset_data().len(), 96, "the length of version 4");
    assert_eq!(os2.fs_type(), UNSTATED_FS_TYPE);
    assert_eq!(os2.us_weight_class(), 400);
    assert_eq!(os2.us_width_class(), 5);
    // Regular and USE_TYPO_METRICS.
    assert_eq!(os2.fs_selection().bits(), 0x40 | 0x80);
    // Basic Latin, Latin-1 Supplement and Latin Extended-A are mapped.
    assert_eq!(os2.ul_unicode_range_1() & 0b111, 0b111);
    assert_eq!(os2.ul_code_page_range_1().unwrap() & 1, 1, "Latin 1");
    let font = read_fonts::FontRef::new(OSWALD).unwrap();
    let original = font.os2().unwrap();
    assert_eq!(os2.us_first_char_index(), original.us_first_char_index());
    assert_eq!(os2.x_avg_char_width(), {
        let hmtx = font.hmtx().unwrap();
        let glyphs = font.maxp().unwrap().num_glyphs();
        let advances: Vec<u64> = (0..glyphs)
            .map(|glyph| u64::from(hmtx.advance(glyph.into()).unwrap()))
            .filter(|&advance| advance != 0)
            .collect();
        let count = advances.len() as u64;
        ((advances.iter().sum::<u64>() + count / 2) / count) as i16
    });
    assert_eq!(os2.sx_height(), original.sx_height());
    assert_eq!(os2.s_cap_height(), original.s_cap_height());
}

#[test]
fn a_repaired_face_measures_as_it_read_without_os2_and_spaces_lines_by_hhea() {
    let broken = no_os2(OSWALD);
    let repaired = repair(&broken, &FontLimits::default()).unwrap();
    let before = &read_sfnt(&broken).unwrap()[0];
    let after = &read_sfnt(&repaired.bytes).unwrap()[0];
    assert_eq!(after, before);
    let hhea = &read_sfnt(OSWALD).unwrap()[0].metrics.hhea;
    assert_eq!(&after.metrics.hhea, hhea, "hhea is kept");
    assert_eq!(&after.metrics.typo, hhea, "the typo metrics are hhea's");
    assert!(after.use_typo_metrics);
    let head = read_fonts::FontRef::new(OSWALD).unwrap().head().unwrap();
    assert_eq!(
        after.metrics.win.ascent,
        hhea.ascender.max(i32::from(head.y_max()))
    );
    assert_eq!(
        after.metrics.win.descent,
        (-hhea.descender).max(-i32::from(head.y_min()))
    );
    // Negative control: Oswald's own OS/2 measures otherwise, so the comparison above can fail.
    assert_ne!(&read_sfnt(OSWALD).unwrap()[0].metrics, &after.metrics);
}

#[test]
fn the_style_comes_from_mac_style_and_the_post_angle() {
    let bold = repair(
        &no_os2(&with_mac_style(OSWALD, 0x01)),
        &FontLimits::default(),
    )
    .unwrap();
    let os2 = os2_of(&bold.bytes);
    assert_eq!(os2.us_weight_class(), 700);
    assert_eq!(os2.fs_selection().bits(), 0x20 | 0x80);
    assert_eq!(read_sfnt(&bold.bytes).unwrap()[0].weight, 700);
    let italic = repair(
        &no_os2(&with_mac_style(OSWALD, 0x02)),
        &FontLimits::default(),
    )
    .unwrap();
    assert_eq!(os2_of(&italic.bytes).fs_selection().bits(), 0x01 | 0x80);
    assert!(read_sfnt(&italic.bytes).unwrap()[0].italic);
    // A bold face without OS/2 reads as bold whether repaired or not.
    assert_eq!(
        read_sfnt(&no_os2(&with_mac_style(OSWALD, 0x01))).unwrap()[0].weight,
        700
    );
}

#[test]
fn a_face_without_os2_states_restricted_embedding() {
    let broken = no_os2(OSWALD);
    let repaired = repair(&broken, &FontLimits::default()).unwrap();
    for font in [&broken, &repaired.bytes] {
        let fs_type = FsType(read_details(font, 0).unwrap().fs_type);
        assert!(fs_type.restricted());
        let decision = |target, toggle| {
            decide(
                FontOrigin::Local,
                fs_type,
                &LicencePolicy::default(),
                target,
                toggle,
            )
        };
        assert_eq!(decision(Target::Preview, Toggle::Default), Decision::Ship);
        assert_eq!(
            decision(Target::Download, Toggle::Default),
            Decision::Exclude(slidock_fonts::policy::Exclusion::Restricted)
        );
        assert_eq!(
            decision(Target::Download, Toggle::Include),
            Decision::ShipWithNotice
        );
    }
    // Negative control: Oswald states installable embedding.
    assert!(!FsType(read_details(OSWALD, 0).unwrap().fs_type).restricted());
}

#[test]
fn a_repaired_face_ships() {
    let repaired = repair(&no_os2(OSWALD), &FontLimits::default()).unwrap();
    let mut codepoints = CodepointSet::new();
    codepoints.insert_text("Hello");
    let shipped = ship_face(
        &ShipInput {
            family: "Oswald",
            bytes: &repaired.bytes,
            index: 0,
            origin: FontOrigin::Local,
            codepoints: &codepoints,
            small_caps: false,
            source_licence: None,
        },
        &ShipOptions::default(),
    )
    .unwrap();
    assert!(shipped.subset);
    assert_eq!(shipped.licence.fs_type, FsType(UNSTATED_FS_TYPE));
    let sfnt = slidock_fonts::container::unwrap(&shipped.rule.bytes, &Default::default())
        .unwrap()
        .sfnt;
    assert_eq!(sanitizer_refusal(&sfnt, 0), None);
}

#[test]
fn only_a_face_the_sanitizer_refuses_for_os2_alone_is_repaired() {
    let limits = FontLimits::default();
    // A sound face needs nothing.
    assert_eq!(repair(OSWALD, &limits), None);
    // Without post as well, the repaired face would still be refused.
    let no_post = with_tables(OSWALD, &[(b"OS/2", None), (b"post", None)]);
    assert_eq!(repair(&no_post, &limits), None);
    // A Mac-only cmap is refused after the OS/2 check: writing OS/2 cannot fix it.
    let mac_only = no_os2(&mac_cmap_only(OSWALD));
    assert_eq!(
        sanitizer_refusal(&mac_only, 0),
        Some(SanitizerRefusal::MissingTable(TableTag(*b"OS/2")))
    );
    assert_eq!(repair(&mac_only, &limits), None);
    // A face with no outlines is no font a browser draws.
    let bare = sfnt(&FaceSpec {
        no_os2: true,
        advances: Some(vec![500; 4]),
        ..FaceSpec::new("Bare")
    });
    assert_eq!(repair(&bare, &limits), None);
    // A file over the limit is left alone.
    let small = FontLimits {
        font_bytes: 1024,
        ..limits
    };
    assert_eq!(repair(&no_os2(OSWALD), &small), None);
}

#[test]
fn a_collection_keeps_its_faces_and_repairs_those_that_need_it() {
    let broken = no_os2(OSWALD);
    let file = collection(&[&broken, ROBOTO_MONO]);
    let repaired = repair(&file, &FontLimits::default()).unwrap();
    assert_eq!(repaired.faces, [Some(FaceRepair::Os2), None]);
    assert_eq!(&repaired.bytes[..4], b"ttcf");
    let faces = read_sfnt(&repaired.bytes).unwrap();
    assert_eq!(faces.len(), 2);
    assert_eq!(faces[0], read_sfnt(&broken).unwrap()[0]);
    assert_eq!(faces[1], read_sfnt(ROBOTO_MONO).unwrap()[0]);
    assert_eq!(sanitizer_refusal(&repaired.bytes, 0), None);
    assert_eq!(sanitizer_refusal(&repaired.bytes, 1), None);
    let roboto = read_fonts::FontRef::from_index(&repaired.bytes, 1).unwrap();
    assert_eq!(
        roboto.os2().unwrap().offset_data().as_bytes(),
        read_fonts::FontRef::new(ROBOTO_MONO)
            .unwrap()
            .os2()
            .unwrap()
            .offset_data()
            .as_bytes(),
        "the sound face's own OS/2 is kept"
    );
}

#[test]
fn shared_tables_of_a_collection_are_written_once() {
    let broken = no_os2(OSWALD);
    let shared = shared_collection(&broken, 2);
    let offsets = |file: &[u8], index| -> Vec<u32> {
        read_fonts::FontRef::from_index(file, index)
            .unwrap()
            .table_directory()
            .table_records()
            .iter()
            .map(|record| record.offset())
            .collect()
    };
    assert_eq!(offsets(&shared, 0), offsets(&shared, 1));
    let repaired = repair(&shared, &FontLimits::default()).unwrap();
    assert_eq!(
        repaired.faces,
        [Some(FaceRepair::Os2), Some(FaceRepair::Os2)]
    );
    let (first, second) = (offsets(&repaired.bytes, 0), offsets(&repaired.bytes, 1));
    // Every table but each face's own OS/2 is the one copy.
    let differing = first.iter().zip(&second).filter(|(a, b)| a != b).count();
    assert_eq!(differing, 1);
    assert!(repaired.bytes.len() < shared.len() + 1024);
}

/// A collection of `count` faces whose directories all point at one copy of `font`'s tables.
fn shared_collection(font: &[u8], count: usize) -> Vec<u8> {
    let source = read_fonts::FontRef::new(font).unwrap();
    let records = source.table_directory().table_records();
    let directory = 12 + 16 * records.len();
    let start = 12 + 4 * count + directory * count;
    let mut data = Vec::new();
    let mut placed = Vec::new();
    for record in records {
        let bytes = source.table_data(record.tag()).unwrap();
        placed.push((record.tag(), start + data.len(), bytes.len()));
        data.extend_from_slice(bytes.as_bytes());
        data.resize(data.len().next_multiple_of(4), 0);
    }
    let mut out = Vec::new();
    out.extend(b"ttcf");
    out.extend(u32be(0x0001_0000));
    out.extend(u32be(count as u32));
    for face in 0..count {
        out.extend(u32be((12 + 4 * count + directory * face) as u32));
    }
    for _ in 0..count {
        out.extend(u32be(0x0001_0000));
        out.extend(u16be(records.len() as u16));
        out.extend([0; 6]);
        for (tag, at, length) in &placed {
            out.extend(tag.to_be_bytes());
            out.extend(u32be(0));
            out.extend(u32be(*at as u32));
            out.extend(u32be(*length as u32));
        }
    }
    out.extend(data);
    out
}
