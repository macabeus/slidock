//! Subsets keep the licence names (ids 13 and 14), and a subset whose face's licence reserves its names ships under
//! neutral ones, every other table as the plain subset's, so it draws the same.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use read_fonts::types::Tag;
use read_fonts::{FontRef, TableProvider};
use slidock_fonts::rename::{reserves_names, shipped_names};
use slidock_fonts::sfnt::read_details;
use slidock_fonts::subset::{SubsetOptions, subset_face};
use slidock_fonts::{
    CodepointSet, FontOrigin, Hint, ShipInput, ShipOptions, container, read_sfnt, ship_face,
};
use support::fixtures::*;
use support::*;

const OFL: &str = "This Font Software is licensed under the SIL Open Font License, Version 1.1.";
const OFL_URL: &str = "https://openfontlicense.org";

fn mac_name(id: u16, value: &str) -> NameRecord {
    (1, 0, 0, id, value.as_bytes().to_vec())
}

/// Oswald with its name table replaced by `names` (Windows records, plus a Mac family record).
fn named(names: &[(u16, &str)]) -> Vec<u8> {
    let mut records: Vec<NameRecord> = names
        .iter()
        .filter(|(id, _)| *id == 1)
        .map(|(id, value)| mac_name(*id, value))
        .collect();
    records.extend(names.iter().map(|(id, value)| windows_name(*id, value)));
    with_tables(OSWALD, &[(b"name", Some(name_table(&records)))])
}

fn reserved(copyright: &str) -> Vec<u8> {
    named(&[
        (0, copyright),
        (1, "Oswald"),
        (2, "Regular"),
        (3, "3.0;Oswald-Regular"),
        (4, "Oswald Regular"),
        (5, "Version 3.0"),
        (6, "Oswald-Regular"),
        (13, OFL),
        (14, OFL_URL),
    ])
}

fn wanted() -> CodepointSet {
    let mut set = CodepointSet::new();
    set.insert_text("Reserved names");
    set
}

/// Every name record of `font`: (platform, id, string).
fn records(font: &[u8]) -> Vec<(u16, u16, String)> {
    let font = FontRef::new(font).unwrap();
    let name = font.name().unwrap();
    name.name_record()
        .iter()
        .map(|record| {
            let text: String = record.string(name.string_data()).unwrap().chars().collect();
            (record.platform_id(), record.name_id().to_u16(), text)
        })
        .collect()
}

fn subset(font: &[u8]) -> Vec<u8> {
    subset_face(font, 0, &wanted(), &SubsetOptions::default()).unwrap()
}

/// The tables of `font` but `name`, with `head`'s checksum adjustment zeroed.
fn tables_but_name(font: &[u8]) -> Vec<(Tag, Vec<u8>)> {
    let font = FontRef::new(font).unwrap();
    font.table_directory()
        .table_records()
        .iter()
        .map(|record| record.tag())
        .filter(|tag| *tag != Tag::new(b"name"))
        .map(|tag| {
            let mut data = font.table_data(tag).unwrap().as_bytes().to_vec();
            if tag == Tag::new(b"head") {
                data[8..12].fill(0);
            }
            (tag, data)
        })
        .collect()
}

fn shipped(font: &[u8]) -> Vec<u8> {
    let details = read_details(font, 0).unwrap();
    shipped_names(subset(font), &details, Some("OFL-1.1")).unwrap()
}

#[test]
fn a_subset_keeps_the_licence_description_and_url() {
    let details = read_details(&subset(OSWALD), 0).unwrap();
    let full = read_details(OSWALD, 0).unwrap();
    assert!(full.licence.is_some() && full.licence_url.is_some());
    assert_eq!(details.licence, full.licence);
    assert_eq!(details.licence_url, full.licence_url);
}

#[test]
fn a_reserved_font_name_ships_under_a_neutral_name_and_draws_the_same() {
    let copyright =
        "Copyright 2016 The Oswald Project Authors, with Reserved Font Name \"Oswald\".";
    let font = reserved(copyright);
    let renamed = shipped(&font);
    let names = records(&renamed);
    for (platform, id, text) in &names {
        assert!(
            !text.contains("Oswald") || [0, 13].contains(id),
            "{platform} {id} {text}"
        );
    }
    let family = names
        .iter()
        .find(|(p, id, _)| (*p, *id) == (3, 1))
        .unwrap()
        .2
        .clone();
    assert!(family.starts_with("Subset "), "{family}");
    let expected = [
        (1, 1, family.clone()),
        (3, 0, copyright.to_owned()),
        (3, 1, family.clone()),
        (3, 2, "Regular".to_owned()),
        (3, 3, format!("{}-Regular", family.replace(' ', ""))),
        (3, 4, format!("{family} Regular")),
        (3, 5, "Version 3.0".to_owned()),
        (3, 6, format!("{}-Regular", family.replace(' ', ""))),
        (3, 13, OFL.to_owned()),
        (3, 14, OFL_URL.to_owned()),
    ];
    assert_eq!(names, expected);
    assert_eq!(tables_but_name(&renamed), tables_but_name(&subset(&font)));
    // Stable, and a face of another family gets another name.
    assert_eq!(shipped(&font), renamed);
    let other = named(&[
        (0, copyright),
        (1, "Other"),
        (2, "Regular"),
        (6, "Other-Regular"),
    ]);
    assert_ne!(records(&shipped(&other))[0].2, family);
}

#[test]
fn the_typographic_names_are_renamed_each_from_its_own() {
    let font = named(&[
        (
            0,
            "Copyright (c) 2010 Somebody with Reserved Font Name Lato.",
        ),
        (1, "Lato Medium"),
        (2, "Italic"),
        (4, "Lato Medium Italic"),
        (6, "Lato-MediumItalic"),
        (16, "Lato"),
        (17, "Medium Italic"),
    ]);
    let names = records(&shipped(&font));
    let get = |id: u16| {
        names
            .iter()
            .find(|(p, i, _)| (*p, *i) == (3, id))
            .unwrap()
            .2
            .clone()
    };
    let typographic = get(16);
    assert!(typographic.starts_with("Subset ") && get(1).starts_with("Subset "));
    assert_ne!(get(1), typographic);
    assert_eq!(get(4), format!("{typographic} Medium Italic"));
    assert_eq!(
        get(6),
        format!("{}-MediumItalic", typographic.replace(' ', ""))
    );
    assert_eq!(
        (get(2), get(17)),
        ("Italic".to_owned(), "Medium Italic".to_owned())
    );
    // Faces of one family keep sharing their families.
    let bold = named(&[
        (
            0,
            "Copyright (c) 2010 Somebody with Reserved Font Name Lato.",
        ),
        (1, "Lato Medium"),
        (2, "Regular"),
        (16, "Lato"),
        (17, "Medium"),
    ]);
    let bold_names = records(&shipped(&bold));
    for id in [1, 16] {
        let of = |names: &[(u16, u16, String)]| {
            names
                .iter()
                .find(|(p, i, _)| (*p, *i) == (3, id))
                .unwrap()
                .2
                .clone()
        };
        assert_eq!(of(&bold_names), of(&names));
    }
}

#[test]
fn names_no_licence_reserves_stay() {
    let font = reserved("Copyright 2016 The Oswald Project Authors.");
    assert_eq!(shipped(&font), subset(&font));
    assert_eq!(shipped(OSWALD), subset(OSWALD));
}

#[test]
fn the_vera_names_are_reserved_and_dejavu_is_not() {
    let details = read_details(OSWALD, 0).unwrap();
    let vera = Some("Bitstream-Vera");
    assert!(reserves_names(&details, vera, ["Bitstream Vera Sans"]));
    assert!(reserves_names(&details, vera, ["Vera Sans Mono"]));
    assert!(!reserves_names(&details, vera, ["DejaVu Sans"]));
    assert!(!reserves_names(
        &details,
        Some("OFL-1.1"),
        ["Bitstream Vera Sans"]
    ));
}

#[test]
fn liberations_reservation_wording_counts() {
    let font = reserved(
        "Digitized data copyright (c) 2010 Google Corporation with Reserved Font Arimo, Tinos and Cousine.",
    );
    assert!(
        records(&shipped(&font))
            .iter()
            .all(|(_, id, text)| *id == 0 || !text.contains("Oswald"))
    );
}

#[test]
fn the_shipped_file_is_renamed_and_its_rule_and_licence_entry_are_not() {
    let copyright =
        "Copyright 2016 The Oswald Project Authors, with Reserved Font Name \"Oswald\".";
    let font = reserved(copyright);
    let shipped = ship_face(
        &ShipInput {
            family: "Oswald",
            bytes: &font,
            index: 0,
            origin: FontOrigin::Google,
            codepoints: &wanted(),
            small_caps: false,
            source_licence: Some("OFL-1.1"),
        },
        &ShipOptions::default(),
    )
    .unwrap();
    assert!(shipped.subset && shipped.notices.is_empty());
    assert_eq!(shipped.rule.family, "Oswald");
    assert_eq!(shipped.licence.copyright.as_deref(), Some(copyright));
    let sfnt = container::unwrap(&shipped.rule.bytes, &Hint::default())
        .unwrap()
        .sfnt;
    let face = &read_sfnt(&sfnt).unwrap()[0];
    assert!(face.family.starts_with("Subset "), "{}", face.family);
    assert_eq!(shipped.face.family, face.family);
}
