//! Embedded fonts are handed on as the package holds them: one font per part, with the family and the faces the
//! presentation declares for it, a STORED part as a range of the input and a DEFLATE part kept compressed until read.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use slidock_core_api::MediaSource;
use slidock_core_api::frontend::FontContainer;
use slidock_ir::fonts::{FaceRequest, FaceStyle};
use support::*;

const FONTS: &str = concat!(
    "<p:embeddedFont><p:font typeface=\"Lato\"/><p:regular r:id=\"rIdF1\"/><p:bold r:id=\"rIdF2\"/>",
    "<p:italic r:id=\"rIdMissing\"/></p:embeddedFont>",
    "<p:embeddedFont><p:font typeface=\"Twice\"/><p:regular r:id=\"rIdF3\"/><p:bold r:id=\"rIdF3\"/></p:embeddedFont>",
    "<p:embeddedFont><p:font typeface=\"Hidden\"/><p:regular r:id=\"rIdF4\"/></p:embeddedFont>",
);

const RELS: &str = concat!(
    "<Relationship Id=\"rIdF1\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/font\" Target=\"fonts/Lato-regular.fntdata\"/>",
    "<Relationship Id=\"rIdF2\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/font\" Target=\"fonts/Lato-bold.ttf\"/>",
    "<Relationship Id=\"rIdF3\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/font\" Target=\"fonts/Twice.fntdata\"/>",
    "<Relationship Id=\"rIdF4\" Type=\"http://schemas.openxmlformats.org/officeDocument/2006/relationships/font\" Target=\"fonts/{00000000-0000-0000-0000-000000000000}.odttf\"/>",
);

fn part(name: &str) -> Vec<u8> {
    format!("{name} font bytes ").repeat(40).into_bytes()
}

/// A presentation embedding fonts, every entry STORED (`level` 0) or DEFLATE.
fn deck(level: u8) -> Vec<u8> {
    let bytes = pptx(PptxSpec {
        embedded_fonts: Some(FONTS.to_owned()),
        slides: vec![slide("")],
        parts: vec![
            ("ppt/fonts/Lato-regular.fntdata".into(), part("regular")),
            ("ppt/fonts/Lato-bold.ttf".into(), part("bold")),
            ("ppt/fonts/Twice.fntdata".into(), part("twice")),
            (
                "ppt/fonts/{00000000-0000-0000-0000-000000000000}.odttf".into(),
                part("odttf"),
            ),
        ],
        ..PptxSpec::default()
    });
    let mut files = unzip(&bytes);
    let rels = String::from_utf8(files["ppt/_rels/presentation.xml.rels"].clone()).unwrap();
    let rels = rels.replace("</Relationships>", &format!("{RELS}</Relationships>"));
    files.insert("ppt/_rels/presentation.xml.rels".into(), rels.into_bytes());
    zip(&files, level)
}

fn bytes_of(source: &MediaSource, input: &[u8]) -> Vec<u8> {
    match source {
        MediaSource::InputRange { offset, len } => {
            input[*offset as usize..(*offset + *len) as usize].to_vec()
        }
        MediaSource::Bytes(bytes) => bytes.to_vec(),
        MediaSource::Reader(reader) => reader
            .read_at(0, reader.len() as usize)
            .unwrap()
            .into_owned(),
    }
}

#[test]
fn hands_on_each_part_once_with_its_declared_faces() {
    for level in [0, 6] {
        let input = deck(level);
        let fonts = lower(&input).lowered.fonts;
        let summary: Vec<(&str, Vec<FaceRequest>, FontContainer)> = fonts
            .iter()
            .map(|font| (font.family.as_str(), font.faces.clone(), font.container))
            .collect();
        let regular = FaceRequest {
            weight: 400,
            style: FaceStyle::Normal,
        };
        let bold = FaceRequest {
            weight: 700,
            style: FaceStyle::Normal,
        };
        assert_eq!(
            summary,
            vec![
                ("Lato", vec![regular.clone()], FontContainer::Eot),
                ("Lato", vec![bold.clone()], FontContainer::Sfnt),
                ("Twice", vec![regular, bold], FontContainer::Eot),
            ],
            "level {level}"
        );
        for (font, name) in fonts.iter().zip(["regular", "bold", "twice"]) {
            match (&font.source, level) {
                (MediaSource::InputRange { .. }, 0) | (MediaSource::Reader(_), 6) => {}
                (other, _) => panic!("level {level}: {other:?}"),
            }
            assert_eq!(bytes_of(&font.source, &input), part(name), "level {level}");
        }
    }
}
