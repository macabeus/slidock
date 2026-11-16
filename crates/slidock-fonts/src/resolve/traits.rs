//! What resolution reads from a face beyond [`SfntFace`](crate::SfntFace): its legacy family (name id 1, the
//! family Local Font Access reports and older files name a weight in) and the range of its `wght` axis.

use read_fonts::tables::name::Name;
use read_fonts::{FontData, FontRead, FontRef, TableProvider};

use super::local::LocalFontMeta;
use crate::bind::WeightRange;

const LEGACY_FAMILY: u16 = 1;
const SUBFAMILY: u16 = 2;
const FULL_NAME: u16 = 4;
const POSTSCRIPT_NAME: u16 = 6;
const ENGLISH_US: u16 = 0x409;

/// A face's legacy family and `wght` range, as far as they can be read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FaceTraits {
    pub legacy_family: Option<String>,
    /// `fvar`'s `wght` axis, clamped to 1–1000.
    pub wght: Option<WeightRange>,
}

/// The traits of face `index` of a font file (0 for a single font); nothing for what cannot be read.
pub fn face_traits(bytes: &[u8], index: u32) -> FaceTraits {
    let Ok(font) = FontRef::from_index(bytes, index) else {
        return FaceTraits::default();
    };
    FaceTraits {
        legacy_family: legacy_family(&font),
        wght: wght_range(&font),
    }
}

/// Name id 1.
fn legacy_family(font: &FontRef<'_>) -> Option<String> {
    name_string(&font.name().ok()?, LEGACY_FAMILY)
}

/// A name id's string, ranked Windows English, any Windows, Unicode, then Mac Roman (as the sfnt reader ranks names),
/// trimmed; `None` when absent or empty.
fn name_string(name: &Name<'_>, id: u16) -> Option<String> {
    let data = name.string_data();
    let rank = |platform: u16, language: u16| match (platform, language) {
        (3, ENGLISH_US) => Some(0),
        (3, _) => Some(1),
        (0, _) => Some(2),
        (1, 0) => Some(3),
        _ => None,
    };
    name.name_record()
        .iter()
        .filter(|record| record.name_id().to_u16() == id)
        .filter_map(|record| Some((rank(record.platform_id(), record.language_id())?, record)))
        .min_by_key(|(rank, _)| *rank)
        .and_then(|(_, record)| record.string(data).ok())
        .map(|string| string.chars().collect::<String>())
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty())
}

/// The metadata Local Font Access reports for a face, from its `name` table alone (a system font scan reads only
/// that table of each file): `None` without a PostScript name or a family.
pub fn local_meta_of_name_table(bytes: &[u8]) -> Option<LocalFontMeta> {
    let name = Name::read(FontData::new(bytes)).ok()?;
    let postscript_name = name_string(&name, POSTSCRIPT_NAME)?;
    let family = name_string(&name, LEGACY_FAMILY)?;
    Some(LocalFontMeta {
        full_name: name_string(&name, FULL_NAME).unwrap_or_else(|| family.clone()),
        style: name_string(&name, SUBFAMILY).unwrap_or_default(),
        postscript_name,
        family,
    })
}

fn wght_range(font: &FontRef<'_>) -> Option<WeightRange> {
    let fvar = font.fvar().ok()?;
    let axes = fvar.axes().ok()?;
    let axis = axes
        .iter()
        .find(|axis| axis.axis_tag() == read_fonts::types::Tag::new(b"wght"))?;
    let clamp = |value: f64| value.round().clamp(1.0, 1000.0) as u32;
    let (min, max) = (axis.min_value().to_f64(), axis.max_value().to_f64());
    (min.is_finite() && max.is_finite()).then(|| WeightRange::between(clamp(min), clamp(max)))
}

/// How many of `points` face `index` of a font file has no glyph for, through its best Unicode `cmap` subtable;
/// every one of them when it has none that can be read.
pub fn missing_code_points(bytes: &[u8], index: u32, points: impl IntoIterator<Item = u32>) -> u32 {
    let subtable = FontRef::from_index(bytes, index)
        .ok()
        .and_then(|font| font.cmap().ok())
        .and_then(|cmap| cmap.best_subtable())
        .map(|(_, _, subtable)| subtable);
    let missing = points.into_iter().filter(|&point| {
        subtable
            .as_ref()
            .and_then(|subtable| subtable.map_codepoint(point))
            .is_none_or(|glyph| glyph.to_u32() == 0)
    });
    u32::try_from(missing.count()).unwrap_or(u32::MAX)
}
