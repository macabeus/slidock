//! What shipping a face needs beyond its metrics: the licence fields (`OS/2.fsType`, name ids 0, 13 and 14), the
//! `wght` axis of a variable face, the outline format, a face of a collection as a font file of its own, which
//! code points its `cmap` maps, and whether a browser's font sanitizer would load it.

use std::borrow::Cow;

use read_fonts::TableProvider;
use read_fonts::tables::cmap::PlatformId;
use read_fonts::types::Tag;
use skrifa::MetadataProvider;
use write_fonts::FontBuilder;

use super::names::read_names;
use super::reader::{Directory, Reader};
use super::{SfntError, TableTag, face_offset};
use crate::glyphs::CodepointSet;
use crate::limits::FontLimits;

/// The outlines a face draws with, which name the format of its raw file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outlines {
    /// `glyf`: a `.ttf`.
    TrueType,
    /// `CFF ` or `CFF2`: an `.otf`.
    Cff,
}

/// A variable face's `wght` axis, in user units.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WeightAxis {
    pub min: f64,
    pub default: f64,
    pub max: f64,
}

/// The licence and format facts of one face.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceDetails {
    /// `OS/2.fsType`; restricted embedding ([`super::UNSTATED_FS_TYPE`]) when the face has no `OS/2` table, which
    /// states no permissions: an unknown value fails closed.
    pub fs_type: u16,
    /// `OS/2.usWeightClass` as written, 400 without `OS/2`.
    pub weight_class: u16,
    pub weight_axis: Option<WeightAxis>,
    pub outlines: Outlines,
    /// `maxp.numGlyphs`.
    pub glyphs: u16,
    /// Name id 0.
    pub copyright: Option<String>,
    /// Name id 13.
    pub licence: Option<String>,
    /// Name id 14.
    pub licence_url: Option<String>,
}

fn non_empty(value: Option<&String>) -> Option<String> {
    value
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// Reads the shipping facts of face `index` of a font file or collection.
pub fn read_details(bytes: &[u8], index: u32) -> Result<FaceDetails, SfntError> {
    let reader = Reader::new(bytes);
    let tables = Directory::read(&reader, face_offset(&reader, index)?)?;
    let name = tables.required(b"name", 6)?;
    let maxp = tables.required(b"maxp", 6)?;
    let outlines = if tables.has(b"glyf") {
        Outlines::TrueType
    } else if tables.has(b"CFF ") || tables.has(b"CFF2") {
        Outlines::Cff
    } else {
        return Err(SfntError::MissingTable(TableTag(*b"glyf")));
    };
    let (fs_type, weight_class) = match tables.get(b"OS/2").filter(|os2| os2.length >= 10) {
        Some(os2) => (reader.u16(os2.offset + 8)?, reader.u16(os2.offset + 4)?),
        None => (super::UNSTATED_FS_TYPE, 400),
    };
    let names = read_names(&reader, name)?;
    Ok(FaceDetails {
        fs_type,
        weight_class,
        weight_axis: weight_axis(&reader, &tables)?,
        outlines,
        glyphs: reader.u16(maxp.offset + 4)?,
        copyright: non_empty(names.get(&0)),
        licence: non_empty(names.get(&13)),
        licence_url: non_empty(names.get(&14)),
    })
}

/// The `wght` record of the `fvar` axis array, if the face has one with a sane range.
fn weight_axis(reader: &Reader<'_>, tables: &Directory) -> Result<Option<WeightAxis>, SfntError> {
    let Some(fvar) = tables.get(b"fvar").filter(|fvar| fvar.length >= 16) else {
        return Ok(None);
    };
    let axes = fvar.offset + u64::from(reader.u16(fvar.offset + 4)?);
    let count = u64::from(reader.u16(fvar.offset + 8)?);
    let size = u64::from(reader.u16(fvar.offset + 10)?);
    if size < 20 {
        return Ok(None);
    }
    let fixed = |at: u64| -> Result<f64, SfntError> { Ok(f64::from(reader.i32(at)?) / 65536.0) };
    for i in 0..count {
        let record = axes + i * size;
        if record + 20 > fvar.offset + fvar.length {
            break;
        }
        if reader.tag(record)? != TableTag(*b"wght") {
            continue;
        }
        let axis = WeightAxis {
            min: fixed(record + 4)?,
            default: fixed(record + 8)?,
            max: fixed(record + 12)?,
        };
        let sane = axis.min <= axis.default && axis.default <= axis.max && axis.min >= 1.0;
        return Ok(sane.then_some(axis));
    }
    Ok(None)
}

/// Face `index` as a font file of its own: a single font as it is, a face of a collection rebuilt with its own
/// table directory (checksums recomputed).
pub fn face_file<'a>(
    bytes: &'a [u8],
    index: u32,
    limits: &FontLimits,
) -> Result<Cow<'a, [u8]>, SfntError> {
    if bytes.len() > limits.font_bytes {
        return Err(SfntError::TooLarge(bytes.len()));
    }
    let reader = Reader::new(bytes);
    let offset = face_offset(&reader, index)?;
    let tables = Directory::read(&reader, offset)?;
    if reader.tag(0)? != TableTag(*b"ttcf") {
        return Ok(Cow::Borrowed(bytes));
    }
    let mut builder = FontBuilder::new();
    for (tag, table) in tables.iter() {
        let data = reader.bytes(table.offset, table.length, super::Region::Table(tag))?;
        builder.add_raw(Tag::new(&tag.0), data);
    }
    Ok(Cow::Owned(builder.build()))
}

/// The code points of `wanted` that face `index` maps to a glyph other than `.notdef`.
pub fn covered(bytes: &[u8], index: u32, wanted: &CodepointSet) -> Result<CodepointSet, SfntError> {
    let font = read_fonts::FontRef::from_index(bytes, index)
        .map_err(|error| SfntError::Unreadable(error.to_string()))?;
    let charmap = font.charmap();
    Ok(wanted
        .iter()
        .filter(|&code_point| {
            charmap
                .map(code_point)
                .is_some_and(|glyph| glyph.to_u32() != 0)
        })
        .collect())
}

/// Of the 95 printable ASCII characters, how many a face must map for [`fixed_pitch`] to judge its pitch.
const PITCH_SAMPLE: usize = 90;

/// Whether face `index` advances every printable ASCII character it maps (at least 90 of the 95) by the same
/// width, as a monospace font does whatever its name says. A face whose `cmap` or `hmtx` cannot be read is not.
pub fn fixed_pitch(bytes: &[u8], index: u32) -> bool {
    let Ok(font) = read_fonts::FontRef::from_index(bytes, index) else {
        return false;
    };
    let Ok(hmtx) = font.hmtx() else {
        return false;
    };
    let charmap = font.charmap();
    let mut advances = (0x20..0x7f_u32)
        .filter_map(|code_point| charmap.map(code_point))
        .filter(|glyph| glyph.to_u32() != 0)
        .filter_map(|glyph| hmtx.advance(glyph));
    let Some(first) = advances.next() else {
        return false;
    };
    let mut count = 1;
    for advance in advances {
        if advance != first {
            return false;
        }
        count += 1;
    }
    count >= PITCH_SAMPLE
}

/// The tables OTS, the sanitizer Chrome and Firefox load web fonts through, refuses a font without.
const SANITIZER_TABLES: [&[u8; 4]; 8] = [
    b"maxp", b"head", b"OS/2", b"cmap", b"hhea", b"hmtx", b"name", b"post",
];

/// Why a browser's font sanitizer would refuse a face, so that the browser draws a fallback instead.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum SanitizerRefusal {
    #[error("The face cannot be read: {0}")]
    Unreadable(String),
    #[error("The face has no {0} table")]
    MissingTable(TableTag),
    #[error("The face has neither glyf and loca, CFF, nor CBDT and CBLC outlines")]
    NoOutlines,
    #[error("The face's cmap has no Unicode subtable a browser reads")]
    NoUnicodeCmap,
}

/// Why OTS would refuse face `index`, checked on what it requires before anything else: its required tables, an
/// outline format, and a `cmap` subtable it keeps (format 4 for Windows symbol or BMP and Unicode BMP, format 12
/// for Windows or Unicode full repertoire, format 13). `None` when the face passes these checks.
pub fn sanitizer_refusal(bytes: &[u8], index: u32) -> Option<SanitizerRefusal> {
    let font = match read_fonts::FontRef::from_index(bytes, index) {
        Ok(font) => font,
        Err(error) => return Some(SanitizerRefusal::Unreadable(error.to_string())),
    };
    let has = |tag: &[u8; 4]| font.table_data(Tag::new(tag)).is_some();
    if let Some(tag) = SANITIZER_TABLES.iter().find(|tag| !has(tag)) {
        return Some(SanitizerRefusal::MissingTable(TableTag(**tag)));
    }
    let outlines = (has(b"glyf") && has(b"loca"))
        || has(b"CFF ")
        || has(b"CFF2")
        || (has(b"CBDT") && has(b"CBLC"));
    if !outlines {
        return Some(SanitizerRefusal::NoOutlines);
    }
    let cmap = match font.cmap() {
        Ok(cmap) => cmap,
        Err(error) => return Some(SanitizerRefusal::Unreadable(error.to_string())),
    };
    let data = cmap.offset_data();
    let kept = cmap.encoding_records().iter().any(|record| {
        let format = usize::try_from(record.subtable_offset().to_u32())
            .ok()
            .and_then(|at| data.read_at::<u16>(at).ok());
        match (record.platform_id(), format) {
            (PlatformId::Unicode, Some(4)) => matches!(record.encoding_id(), 0 | 1 | 3),
            (PlatformId::Unicode, Some(12)) => matches!(record.encoding_id(), 3 | 4),
            (PlatformId::Windows, Some(4)) => matches!(record.encoding_id(), 0 | 1),
            (PlatformId::Windows, Some(12 | 13)) => record.encoding_id() == 10,
            _ => false,
        }
    });
    (!kept).then_some(SanitizerRefusal::NoUnicodeCmap)
}
