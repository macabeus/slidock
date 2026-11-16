//! MicroType Express on fixtures that sfntly's encoder made from OFL fonts (see `tests/data/container/README.md`),
//! compared with what libeot decodes them into and with the fonts they were made from.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod container_support;

use container_support::*;
use read_fonts::{FontRef, TableProvider};
use slidock_fonts::container::{self, Format, Hint};
use slidock_fonts::read_sfnt;

const FONTS: [&str; 4] = ["barlow", "oswald", "robotomono", "roboto-hdmx"];

fn unwrap(name: &str) -> container::Unwrapped {
    let unwrapped = container::unwrap(&fixture(name), &Hint::default()).unwrap();
    assert_eq!(unwrapped.kind, Format::Eot);
    assert!(unwrapped.warnings.is_empty(), "{name}");
    unwrapped
}

/// The values a glyph program's leading push instructions push, and the instructions after them.
fn split_pushes(instructions: &[u8]) -> (Vec<i32>, &[u8]) {
    let mut values = Vec::new();
    let mut at = 0;
    while let Some(&op) = instructions.get(at) {
        let (count, words, start) = match op {
            0x40 => (instructions[at + 1] as usize, false, at + 2),
            0x41 => (instructions[at + 1] as usize, true, at + 2),
            0xb0..=0xb7 => ((op - 0xaf) as usize, false, at + 1),
            0xb8..=0xbf => ((op - 0xb7) as usize, true, at + 1),
            _ => break,
        };
        let size = if words { 2 } else { 1 };
        for i in 0..count {
            let at = start + size * i;
            values.push(if words {
                u16_at(instructions, at) as i16 as i32
            } else {
                instructions[at] as i32
            });
        }
        at = start + size * count;
    }
    (values, &instructions[at..])
}

/// The glyph with its instructions' leading pushes replaced by the values they push, so that glyphs whose push
/// instructions were regenerated compare equal, and without `OVERLAP_SIMPLE`, which the compact glyph format
/// cannot hold.
fn normalized(glyph: &[u8]) -> (GlyphModel, Vec<i32>) {
    match glyph_model(glyph) {
        GlyphModel::Simple {
            bbox,
            end_points,
            instructions,
            points,
        } => {
            let (values, rest) = split_pushes(&instructions);
            let model = GlyphModel::Simple {
                bbox,
                end_points,
                instructions: rest.to_vec(),
                points: points
                    .into_iter()
                    .map(|(x, y, on_curve, _)| (x, y, on_curve, false))
                    .collect(),
            };
            (model, values)
        }
        GlyphModel::Composite {
            bbox,
            components,
            instructions,
        } => {
            let (values, rest) = split_pushes(&instructions);
            let model = GlyphModel::Composite {
                bbox,
                components,
                instructions: rest.to_vec(),
            };
            (model, values)
        }
        GlyphModel::Empty => (GlyphModel::Empty, Vec::new()),
    }
}

#[test]
fn mtx_decodes_byte_for_byte_like_libeot() {
    for font in ["barlow", "oswald", "robotomono"] {
        let name = format!("{font}-mtx.eot");
        let decoded = unwrap(&name).sfnt;
        assert!(decoded == fixture(&format!("{name}.libeot.ttf")), "{name}");
    }
}

#[test]
fn mtx_hdmx_is_decoded_where_libeot_drops_it() {
    let name = "roboto-hdmx-mtx.eot";
    let decoded = unwrap(name).sfnt;
    let libeot = fixture(&format!("{name}.libeot.ttf"));
    let ours = tables(&decoded, 0);
    let theirs = tables(&libeot, 0);
    assert_eq!(
        ours.keys()
            .filter(|tag| **tag != *b"hdmx")
            .collect::<Vec<_>>(),
        theirs.keys().collect::<Vec<_>>()
    );
    for (tag, (data, _)) in &theirs {
        let mut ours = ours[tag].0.to_vec();
        let mut theirs = data.to_vec();
        if tag == b"head" {
            // The font checksum covers hdmx too.
            ours[8..12].fill(0);
            theirs[8..12].fill(0);
        }
        assert_eq!(ours, theirs, "{}", String::from_utf8_lossy(tag));
    }
    let source = fixture("roboto-hdmx.ttf");
    assert_eq!(ours[b"hdmx"].0, table(&source, 0, b"hdmx"));
}

#[test]
fn mtx_decodes_to_the_fonts_sfntly_encoded() {
    for font in FONTS {
        let name = format!("{font}-mtx.eot");
        let decoded = unwrap(&name).sfnt;
        let source = fixture(&format!("{font}.ttf"));
        assert_eq!(checksum(&decoded), 0xb1b0_afba, "{name}");
        let ours = tables(&decoded, 0);
        let theirs = tables(&source, 0);
        assert_eq!(
            ours.keys().collect::<Vec<_>>(),
            theirs.keys().collect::<Vec<_>>(),
            "{name}"
        );
        for (tag, (data, sum)) in &ours {
            let mut data = data.to_vec();
            if tag == b"head" {
                data[8..12].fill(0);
            }
            assert_eq!(checksum(&data), *sum, "{name}: {tag:?} checksum");
            if [b"glyf", b"loca", b"head"].contains(&tag) {
                continue;
            }
            assert_eq!(
                ours[tag].0,
                theirs[tag].0,
                "{name}: {}",
                String::from_utf8_lossy(tag)
            );
        }
        // head differs in checkSumAdjustment only.
        let (mut ours_head, mut source_head) =
            (ours[b"head"].0.to_vec(), theirs[b"head"].0.to_vec());
        ours_head[8..12].fill(0);
        source_head[8..12].fill(0);
        assert_eq!(ours_head, source_head, "{name}: head");
        let (ours_glyphs, source_glyphs) = (glyphs(&decoded, 0), glyphs(&source, 0));
        assert_eq!(ours_glyphs.len(), source_glyphs.len(), "{name}");
        let mut pushes = 0;
        for (gid, (ours, theirs)) in ours_glyphs.iter().zip(&source_glyphs).enumerate() {
            let (ours, ours_values) = normalized(ours);
            let (theirs, their_values) = normalized(theirs);
            assert_eq!(ours, theirs, "{name}: glyph {gid}");
            assert_eq!(ours_values, their_values, "{name}: glyph {gid} pushes");
            pushes += ours_values.len();
        }
        if font != "oswald" && font != "robotomono" {
            assert!(pushes > 0, "{name}: hinted glyphs");
        }
        let faces = read_sfnt(&decoded).unwrap();
        assert_eq!(faces.len(), 1);
        let face = FontRef::new(&decoded).unwrap();
        let (loca, glyf) = (face.loca(None).unwrap(), face.glyf().unwrap());
        for gid in 0..u32::from(face.maxp().unwrap().num_glyphs()) {
            loca.get_glyf(gid.into(), &glyf).unwrap();
        }
    }
}

#[test]
fn mtx_eots_carry_their_header() {
    let unwrapped = unwrap("barlow-mtx.eot");
    assert!(!unwrapped.subset);
    let names = unwrapped.header_names.unwrap();
    assert_eq!(names.family, "Barlow Condensed");
    assert_eq!(names.weight, 400);
    assert!(!names.italic);
}
