//! The few tables of a TrueType/OpenType font (or of each face of a collection) that line metrics and font binding
//! need: `head`, `hhea`, `OS/2`, `name`, `post`, the presence of `fvar`, and the box of one glyph (À, through
//! `cmap`, `maxp`, `loca` and `glyf`).
//!
//! Font files come from user folders and presentations, so every offset is checked and a lie is an [`SfntError`].
//! Fields are read at their fixed offsets through `read-fonts`' [`FontData`](read_fonts::FontData) rather than
//! its typed tables, whose version-dependent length rules, sorted-directory lookups and lazily checked subtables
//! would accept and refuse other fonts than this reader does.

mod details;
mod glyph;
mod instance;
mod names;
mod reader;
mod repair;

use std::fmt;

use serde::Serialize;
use slidock_ir::fonts::{FontMetrics, GlyphBounds, VMetrics, WinMetrics};

use reader::{Directory, Reader};

pub use details::{
    FaceDetails, Outlines, SanitizerRefusal, WeightAxis, covered, face_file, fixed_pitch,
    read_details, sanitizer_refusal,
};
pub use instance::metrics_at;
pub use repair::{FaceRepair, Repaired, UNSTATED_FS_TYPE, repair, synthesize_os2};

const MAX_FACES: u32 = 256;

/// The code point whose glyph top CoreText's leading rule compares with the ascent.
const A_GRAVE: u32 = 0xc0;

/// One face of a font file.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SfntFace {
    /// Name id 16 (typographic family), else id 1.
    pub family: String,
    /// Name id 17 (typographic subfamily), else id 2, else `Regular`.
    pub subfamily: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    /// Name id 6, else the family without spaces and other characters outside `!`–`~`.
    pub postscript_name: String,
    /// `usWeightClass` on the 100–900 scale.
    pub weight: u32,
    /// `usWidthClass` as the file states it (1–9, 5 normal; see [`crate::bind::stretch_rank`]), or 5 for a font
    /// without an `OS/2` table, which states no width. Left out of the serialized face, which holds the fields of
    /// the reference reader.
    #[serde(skip)]
    pub width: u16,
    /// `OS/2.fsSelection` bit 0, `head.macStyle` bit 1 or a nonzero `post.italicAngle`; or, when none is set, a
    /// subfamily naming the face Italic or Oblique ([`names_a_slant`]).
    pub italic: bool,
    /// `post.italicAngle` in degrees.
    pub italic_angle: f64,
    /// An `fvar` table: the file is a variable font, read at its default instance.
    pub variable: bool,
    /// `OS/2.fsSelection` bit 7: the typo metrics define line spacing.
    pub use_typo_metrics: bool,
    pub metrics: FontMetrics,
    /// Every printable ASCII character the face maps advances by the same width ([`fixed_pitch`]). Left out of
    /// the serialized face.
    #[serde(skip)]
    pub fixed_pitch: bool,
}

/// Whether a subfamily name calls the face slanted: `Italic` or `Oblique` anywhere in it, in any case
/// (`Medium Italic`, `BoldOblique`), as fontconfig reads a face whose style bits are clear.
pub fn names_a_slant(subfamily: &str) -> bool {
    let name = subfamily.to_ascii_lowercase();
    name.contains("italic") || name.contains("oblique")
}

/// A four-byte table tag, printed as Latin-1 characters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct TableTag(pub [u8; 4]);

impl fmt::Display for TableTag {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0
            .iter()
            .try_for_each(|&byte| write!(f, "{}", char::from(byte)))
    }
}

/// The part of a font a read fell outside of.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Region {
    FontData,
    Table(TableTag),
    NameString,
}

impl fmt::Display for Region {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Region::FontData => f.write_str("font data"),
            Region::Table(tag) => write!(f, "{tag} table"),
            Region::NameString => f.write_str("name string"),
        }
    }
}

/// Why a file is not a font this reader can use.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SfntError {
    #[error("The {0} is outside the file")]
    OutsideFile(Region),
    #[error("Not a TrueType or OpenType font")]
    NotSfnt,
    #[error("The font declares {0} tables")]
    TableCount(u16),
    #[error("The font's tables together are longer than the file")]
    TablesOverlap,
    #[error("The font has no {0} table")]
    MissingTable(TableTag),
    #[error("The {0} table is truncated")]
    TruncatedTable(TableTag),
    #[error("unitsPerEm {0} is out of range")]
    UnitsPerEm(u16),
    #[error("The cmap declares {0} groups")]
    CmapGroups(u32),
    #[error("The font has no family name")]
    NoFamilyName,
    #[error("The collection declares {0} faces")]
    FaceCount(u32),
    #[error("The file has no face {0}")]
    FaceIndex(u32),
    #[error("The font file is {0} bytes, over the limit")]
    TooLarge(usize),
    #[error("The font cannot be read: {0}")]
    Unreadable(String),
}

/// Every face of a TrueType/OpenType file or collection.
pub fn read_sfnt(bytes: &[u8]) -> Result<Vec<SfntFace>, SfntError> {
    let reader = Reader::new(bytes);
    let pitched = |mut face: SfntFace, index: u32| {
        face.fixed_pitch = fixed_pitch(bytes, index);
        face
    };
    if reader.tag(0)? != TableTag(*b"ttcf") {
        return Ok(vec![pitched(read_face(&reader, 0)?, 0)]);
    }
    let count = reader.u32(8)?;
    if !(1..=MAX_FACES).contains(&count) {
        return Err(SfntError::FaceCount(count));
    }
    (0..count)
        .map(|i| {
            let face = read_face(&reader, u64::from(reader.u32(12 + u64::from(i) * 4)?))?;
            Ok(pitched(face, i))
        })
        .collect()
}

/// Where face `index` of a file starts: 0 for a single font, its directory's offset in a collection.
fn face_offset(reader: &Reader<'_>, index: u32) -> Result<u64, SfntError> {
    if reader.tag(0)? != TableTag(*b"ttcf") {
        return if index == 0 {
            Ok(0)
        } else {
            Err(SfntError::FaceIndex(index))
        };
    }
    let count = reader.u32(8)?;
    if !(1..=MAX_FACES).contains(&count) {
        return Err(SfntError::FaceCount(count));
    }
    if index >= count {
        return Err(SfntError::FaceIndex(index));
    }
    Ok(u64::from(reader.u32(12 + u64::from(index) * 4)?))
}

/// `usWeightClass` on the 100–900 scale, rounded to a hundred; some old fonts write 1–9.
fn normal_weight(weight_class: u16) -> u32 {
    let weight = u32::from(weight_class);
    let weight = if (1..10).contains(&weight) {
        weight * 100
    } else {
        weight
    };
    ((weight + 50) / 100 * 100).clamp(100, 900)
}

/// The family with every character outside `!`–`~` removed.
fn postscript_of(family: &str) -> String {
    family.chars().filter(|c| ('!'..='~').contains(c)).collect()
}

fn read_face(reader: &Reader<'_>, offset: u64) -> Result<SfntFace, SfntError> {
    let tables = Directory::read(reader, offset)?;
    let head = tables.required(b"head", 54)?;
    let hhea = tables.required(b"hhea", 36)?;
    let name = tables.required(b"name", 6)?;
    let units_per_em = reader.u16(head.offset + 18)?;
    if !(16..=16_384).contains(&units_per_em) {
        return Err(SfntError::UnitsPerEm(units_per_em));
    }
    let mac_italic = reader.u16(head.offset + 44)? & 0x02 != 0;
    let hhea_metrics = VMetrics {
        ascender: reader.i16(hhea.offset + 4)?.into(),
        descender: reader.i16(hhea.offset + 6)?.into(),
        line_gap: reader.i16(hhea.offset + 8)?.into(),
    };

    // A face without OS/2 reads as the table slidock writes for it would ([`repair`]), so the face measures the same
    // whether it is repaired or not.
    let os2 = tables.get(b"OS/2").filter(|os2| os2.length >= 78);
    let (weight_class, width, fs_selection, typo, win) = match os2 {
        Some(os2) => (
            reader.u16(os2.offset + 4)?,
            reader.u16(os2.offset + 6)?,
            reader.u16(os2.offset + 62)?,
            VMetrics {
                ascender: reader.i16(os2.offset + 68)?.into(),
                descender: reader.i16(os2.offset + 70)?.into(),
                line_gap: reader.i16(os2.offset + 72)?.into(),
            },
            WinMetrics {
                ascent: reader.u16(os2.offset + 74)?.into(),
                descent: reader.u16(os2.offset + 76)?.into(),
            },
        ),
        None => {
            let implied = repair::implied_of(reader, &tables)?;
            (
                implied.weight_class,
                implied.width_class,
                implied.fs_selection,
                implied.typo,
                implied.win,
            )
        }
    };

    let bounds = GlyphBounds {
        y_min: reader.i16(head.offset + 38)?.into(),
        y_max: reader.i16(head.offset + 42)?.into(),
    };
    let grave = match tables.get(b"cmap").filter(|cmap| cmap.length >= 4) {
        Some(cmap) => glyph::glyph_of(reader, cmap, A_GRAVE)?,
        None => None,
    };
    let grave_top = match grave {
        Some(glyph) => {
            let long_offsets = reader.i16(head.offset + 50)? == 1;
            glyph::glyph_top(reader, &tables, long_offsets, glyph)?
        }
        None => None,
    };

    let italic_angle = match tables.get(b"post").filter(|post| post.length >= 8) {
        Some(post) => f64::from(reader.i32(post.offset + 4)?) / 65536.0,
        None => 0.0,
    };
    let names = names::read_names(reader, name)?;
    let family = names
        .get(&16)
        .or_else(|| names.get(&1))
        .filter(|family| !family.is_empty())
        .ok_or(SfntError::NoFamilyName)?;
    let postscript_name = names
        .get(&6)
        .cloned()
        .unwrap_or_else(|| postscript_of(family));
    if postscript_name.is_empty() {
        return Err(SfntError::NoFamilyName);
    }

    let subfamily = names
        .get(&17)
        .or_else(|| names.get(&2))
        .map_or_else(|| "Regular".to_owned(), Clone::clone);
    let italic =
        fs_selection & 0x01 != 0 || mac_italic || italic_angle != 0.0 || names_a_slant(&subfamily);

    Ok(SfntFace {
        family: family.clone(),
        subfamily,
        full_name: names.get(&4).cloned(),
        postscript_name,
        weight: normal_weight(weight_class),
        width,
        italic,
        italic_angle,
        variable: tables.has(b"fvar"),
        use_typo_metrics: fs_selection & 0x80 != 0,
        metrics: FontMetrics {
            units_per_em: units_per_em.into(),
            hhea: hhea_metrics,
            typo,
            win,
            bounds: Some(bounds),
            grave_top,
        },
        fixed_pitch: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn weights_scale_round_and_clamp() {
        let cases = [
            (0, 100),
            (1, 100),
            (5, 500),
            (9, 900),
            (10, 100),
            (149, 100),
            (150, 200),
            (350, 400),
            (449, 400),
            (450, 500),
            (950, 900),
            (u16::MAX, 900),
        ];
        for (class, weight) in cases {
            assert_eq!(normal_weight(class), weight, "usWeightClass {class}");
        }
    }

    #[test]
    fn a_subfamily_names_a_slant_with_italic_or_oblique() {
        for name in [
            "Italic",
            "Medium Italic",
            "BoldItalic",
            "ITALIC",
            "Oblique",
            "Light Oblique",
        ] {
            assert!(names_a_slant(name), "{name}");
        }
        // Negative controls: upright names, and a slant spelt another way.
        for name in [
            "Regular",
            "Medium",
            "Bold Condensed",
            "Slanted",
            "Kursiv",
            "Ital",
        ] {
            assert!(!names_a_slant(name), "{name}");
        }
    }

    #[test]
    fn postscript_fallback_keeps_printable_ascii() {
        assert_eq!(postscript_of("Helvetica Neue"), "HelveticaNeue");
        assert_eq!(postscript_of("Ärial\u{7f}~!"), "rial~!");
    }

    #[test]
    fn error_messages_name_the_region() {
        assert_eq!(
            SfntError::OutsideFile(Region::Table(TableTag(*b"OS/2"))).to_string(),
            "The OS/2 table is outside the file"
        );
        assert_eq!(
            SfntError::TruncatedTable(TableTag(*b"name")).to_string(),
            "The name table is truncated"
        );
    }
}
