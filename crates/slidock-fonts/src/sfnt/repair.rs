//! Faces browsers' font sanitizer refuses that slidock can make loadable without changing how they draw.
//!
//! OTS, which Chrome and Firefox load web fonts through, requires an `OS/2` table; old Apple TrueType fonts
//! (Courier.ttc, with its AAT tables) have none. A face refused for that alone gets an `OS/2` version 4 table
//! written from its other tables ([`implied_os2`]): the weight from `head.macStyle` or a weight in its subfamily
//! name, the slant from `head.macStyle`, `post.italicAngle` or its subfamily, the typo metrics from `hhea` (and
//! `USE_TYPO_METRICS`, so every platform spaces lines by `hhea`), the win metrics from `hhea` widened to the glyph
//! box (`head.yMin`/`yMax`), the Unicode ranges, first and last characters and code pages from the `cmap`, and the
//! average advance from `hmtx`. The face's `fsType` is unknown: unknown fails closed, so the table states
//! restricted embedding (0x0002). The repaired face must then pass [`sanitizer_refusal`]; a face it still fails
//! keeps its refusal. The tables OTS drops (AAT `morx`, `feat`, `prop`, …) need no repair: the browser draws
//! without them, as it draws every font whose AAT tables it does not read.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use read_fonts::TableProvider;
use read_fonts::types::Tag;
use skrifa::MetadataProvider;
use slidock_ir::fonts::{GlyphBounds, VMetrics, WinMetrics};
use write_fonts::FontBuilder;

use super::glyph::{glyph_of, glyph_top};
use super::names::read_names;
use super::reader::{Directory, Reader};
use super::{SfntError, TableTag, face_offset, names_a_slant};
use crate::bind::NORMAL_WIDTH;
use crate::limits::FontLimits;
use crate::names::weight_of_name;
use crate::sfnt::details::{SanitizerRefusal, sanitizer_refusal};

/// What slidock wrote into a face so that browsers load it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FaceRepair {
    /// The face has no `OS/2` table, which browsers require: one was written from its other tables, stating
    /// restricted embedding.
    Os2,
}

impl FaceRepair {
    /// The table slidock wrote.
    pub const fn table(self) -> &'static str {
        match self {
            FaceRepair::Os2 => "OS/2",
        }
    }
}

impl fmt::Display for FaceRepair {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FaceRepair::Os2 => f.write_str("it has no OS/2 table, which browsers require"),
        }
    }
}

/// A font file with faces repaired.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Repaired {
    pub bytes: Vec<u8>,
    /// Per face, what was repaired.
    pub faces: Vec<Option<FaceRepair>>,
}

/// The `fsType` a face that states none is given: restricted license embedding.
pub const UNSTATED_FS_TYPE: u16 = 0x0002;

const BOLD: u16 = 0x01;
const ITALIC: u16 = 0x02;

/// `OS/2.fsSelection` bits.
const SELECTION_ITALIC: u16 = 0x0001;
const SELECTION_BOLD: u16 = 0x0020;
const SELECTION_REGULAR: u16 = 0x0040;
const SELECTION_USE_TYPO_METRICS: u16 = 0x0080;

/// The length of an `OS/2` version 4 table.
const OS2_V4_LENGTH: usize = 96;

/// What a face without `OS/2` implies for the fields readers take from that table.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct ImpliedOs2 {
    pub(super) weight_class: u16,
    pub(super) width_class: u16,
    pub(super) fs_selection: u16,
    pub(super) typo: VMetrics,
    pub(super) win: WinMetrics,
}

/// The `OS/2` fields a face without the table implies: weight 700 when `head.macStyle` says bold, else a weight
/// named in its subfamily (`Light`, `Black`), else 400; normal width; italic when `macStyle`, `post.italicAngle`
/// or the subfamily says so; typo metrics equal to `hhea`, which `USE_TYPO_METRICS` makes the line spacing; win
/// metrics covering both `hhea` and the glyph box.
pub(super) fn implied_os2(
    mac_style: u16,
    italic_angle: f64,
    subfamily: &str,
    hhea: &VMetrics,
    bounds: &GlyphBounds,
) -> ImpliedOs2 {
    let bold = mac_style & BOLD != 0;
    let named = || {
        subfamily
            .split(|c: char| c.is_whitespace() || c == '-')
            .filter_map(weight_of_name)
            .find(|&weight| weight != 400)
    };
    let weight_class = if bold {
        700
    } else {
        named().and_then(|w| u16::try_from(w).ok()).unwrap_or(400)
    };
    let italic = mac_style & ITALIC != 0 || italic_angle != 0.0 || names_a_slant(subfamily);
    let mut fs_selection = SELECTION_USE_TYPO_METRICS;
    if italic {
        fs_selection |= SELECTION_ITALIC;
    }
    if bold {
        fs_selection |= SELECTION_BOLD;
    }
    if !italic && !bold {
        fs_selection |= SELECTION_REGULAR;
    }
    ImpliedOs2 {
        weight_class,
        width_class: NORMAL_WIDTH,
        fs_selection,
        typo: hhea.clone(),
        win: WinMetrics {
            ascent: hhea.ascender.max(bounds.y_max).max(0),
            descent: (-hhea.descender).max(-bounds.y_min).max(0),
        },
    }
}

/// The implied `OS/2` fields of the face whose directory is at `offset`, read from its `head`, `hhea`, `post`
/// and `name` tables.
pub(super) fn implied_of(reader: &Reader<'_>, tables: &Directory) -> Result<ImpliedOs2, SfntError> {
    let head = tables.required(b"head", 54)?;
    let hhea = tables.required(b"hhea", 36)?;
    let name = tables.required(b"name", 6)?;
    let italic_angle = match tables.get(b"post").filter(|post| post.length >= 8) {
        Some(post) => f64::from(reader.i32(post.offset + 4)?) / 65536.0,
        None => 0.0,
    };
    let names = read_names(reader, name)?;
    let subfamily = names
        .get(&17)
        .or_else(|| names.get(&2))
        .map_or("Regular", String::as_str);
    Ok(implied_os2(
        reader.u16(head.offset + 44)?,
        italic_angle,
        subfamily,
        &VMetrics {
            ascender: reader.i16(hhea.offset + 4)?.into(),
            descender: reader.i16(hhea.offset + 6)?.into(),
            line_gap: reader.i16(hhea.offset + 8)?.into(),
        },
        &GlyphBounds {
            y_min: reader.i16(head.offset + 38)?.into(),
            y_max: reader.i16(head.offset + 42)?.into(),
        },
    ))
}

/// The OpenType Unicode ranges (`ulUnicodeRange1`–`4`) as `(first, last, bit)`, sorted by first code point (the
/// OS/2 specification's table). Bit 57 is set for any code point beyond the BMP.
const UNICODE_RANGES: &[(u32, u32, u8)] = &include!("unicode_ranges.in");

const NON_PLANE_0: u8 = 57;

fn unicode_range_bit(code_point: u32) -> Option<u8> {
    if code_point > 0xffff {
        return Some(NON_PLANE_0);
    }
    let at = UNICODE_RANGES.partition_point(|&(first, _, _)| first <= code_point);
    let &(first, last, bit) = UNICODE_RANGES.get(at.checked_sub(1)?)?;
    (first..=last).contains(&code_point).then_some(bit)
}

/// The code pages a character set covers (`ulCodePageRange1`–`2`), by the characters FontForge and fontTools test
/// for each.
fn code_pages(mapped: &BTreeSet<u32>) -> u64 {
    let has = |ch: char| mapped.contains(&u32::from(ch));
    let ascii = (0x20..0x7e).all(|point| mapped.contains(&point));
    let lineart = has('┤');
    let root = has('√');
    let mut bits = 0u64;
    let mut set = |bit: u32, on: bool| {
        if on {
            bits |= 1 << bit;
        }
    };
    set(0, ascii && has('Þ'));
    set(1, ascii && has('Ľ'));
    set(58, ascii && has('Ľ') && lineart);
    set(2, has('Б'));
    set(57, has('Б') && has('Ѕ') && lineart);
    set(49, has('Б') && has('╜') && lineart);
    set(3, has('Ά'));
    set(48, has('Ά') && lineart && has('½'));
    set(60, has('Ά') && lineart && root);
    set(4, ascii && has('İ'));
    set(56, ascii && has('İ') && lineart);
    set(5, has('א'));
    set(53, has('א') && lineart && root);
    set(6, has('ر'));
    set(51, has('ر') && root);
    set(61, has('ر') && lineart);
    set(7, ascii && has('ŗ'));
    set(59, ascii && has('ŗ') && lineart);
    set(8, ascii && has('₫'));
    set(16, has('ๅ'));
    set(17, has('エ'));
    set(18, has('ㄅ'));
    set(19, has('ㄱ'));
    set(20, has('央'));
    set(21, has('곴'));
    set(30, ascii && has('♥'));
    set(54, ascii && has('þ') && lineart);
    set(62, ascii && has('╚'));
    set(63, ascii && has('╚'));
    let nordic = ascii && lineart && root;
    set(50, nordic && has('Å'));
    set(52, nordic && has('é'));
    set(55, nordic && has('õ'));
    set(29, ascii && has('‰') && has('∑'));
    bits
}

fn clamp_i16(value: i64) -> i16 {
    i16::try_from(value.clamp(i64::from(i16::MIN), i64::from(i16::MAX))).unwrap_or(0)
}

fn clamp_u16(value: i64) -> u16 {
    u16::try_from(value.clamp(0, i64::from(u16::MAX))).unwrap_or(0)
}

/// Appends `values` big-endian.
fn put_u16s(out: &mut Vec<u8>, values: &[u16]) {
    for value in values {
        out.extend_from_slice(&value.to_be_bytes());
    }
}

/// `units × ratio`, rounded half up.
fn scaled(units: u16, ratio: f64) -> i16 {
    clamp_i16((f64::from(units) * ratio + 0.5).floor() as i64)
}

/// An `OS/2` version 4 table for face `index`, which has none, written from its other tables.
pub fn synthesize_os2(bytes: &[u8], index: u32) -> Result<Vec<u8>, SfntError> {
    let reader = Reader::new(bytes);
    let tables = Directory::read(&reader, face_offset(&reader, index)?)?;
    let implied = implied_of(&reader, &tables)?;
    let font = read_fonts::FontRef::from_index(bytes, index)
        .map_err(|error| SfntError::Unreadable(error.to_string()))?;
    let unreadable = |error: read_fonts::ReadError| SfntError::Unreadable(error.to_string());
    let head = font.head().map_err(unreadable)?;
    let upm = head.units_per_em();
    let glyphs = font.maxp().map_err(unreadable)?.num_glyphs();

    let hmtx = font.hmtx().map_err(unreadable)?;
    let metrics = hmtx.h_metrics();
    let last = metrics.last().map_or(0, |metric| metric.advance());
    let (mut sum, mut count) = (0u64, 0u64);
    for glyph in 0..usize::from(glyphs) {
        let advance = metrics.get(glyph).map_or(last, |metric| metric.advance());
        if advance != 0 {
            sum += u64::from(advance);
            count += 1;
        }
    }
    let average = (sum + count / 2)
        .checked_div(count)
        .and_then(|average| i64::try_from(average).ok())
        .unwrap_or(0);

    let mapped: BTreeSet<u32> = font
        .charmap()
        .mappings()
        .filter(|(_, glyph)| glyph.to_u32() != 0)
        .map(|(code_point, _)| code_point)
        .collect();
    let mut ranges = [0u32; 4];
    for &code_point in &mapped {
        if let Some(bit) = unicode_range_bit(code_point)
            && let Some(word) = ranges.get_mut(usize::from(bit / 32))
        {
            *word |= 1 << (bit % 32);
        }
    }
    let pages = code_pages(&mapped);
    let first = mapped.first().copied().unwrap_or(0);
    let last_char = mapped.last().copied().unwrap_or(0);

    let top_of = |ch: char| -> Result<Option<i32>, SfntError> {
        let Some(cmap) = tables.get(b"cmap").filter(|cmap| cmap.length >= 4) else {
            return Ok(None);
        };
        let long_offsets = reader.i16(tables.required(b"head", 54)?.offset + 50)? == 1;
        match glyph_of(&reader, cmap, u32::from(ch))? {
            Some(glyph) => glyph_top(&reader, &tables, long_offsets, glyph),
            None => Ok(None),
        }
    };
    let x_height = top_of('x')?.map(i64::from);
    let cap_height = top_of('H')?.map(i64::from);
    let underline = font
        .post()
        .map(|post| post.underline_thickness().to_i16())
        .unwrap_or(0);
    let strikeout_size = if underline > 0 {
        underline
    } else {
        scaled(upm, 0.05)
    };

    let strikeout_position = match x_height {
        Some(height) => clamp_i16((height * 3 + 2) / 5),
        None => scaled(upm, 0.22),
    };
    // The fields in the order the OpenType specification lays out version 4, each big-endian.
    let mut os2 = Vec::with_capacity(OS2_V4_LENGTH);
    let signed = |value: i16| u16::from_ne_bytes(value.to_ne_bytes());
    put_u16s(
        &mut os2,
        &[
            4,
            signed(clamp_i16(average)),
            implied.weight_class,
            implied.width_class,
            UNSTATED_FS_TYPE,
        ],
    );
    put_u16s(
        &mut os2,
        &[
            scaled(upm, 0.65),
            scaled(upm, 0.6),
            0,
            scaled(upm, 0.075),
            scaled(upm, 0.65),
            scaled(upm, 0.6),
            0,
            scaled(upm, 0.35),
            strikeout_size,
            strikeout_position,
            0,
        ]
        .map(signed),
    );
    os2.extend_from_slice(&[0; 10]);
    for range in ranges {
        os2.extend_from_slice(&range.to_be_bytes());
    }
    os2.extend_from_slice(b"NONE");
    put_u16s(
        &mut os2,
        &[
            implied.fs_selection,
            clamp_u16(i64::from(first)),
            clamp_u16(i64::from(last_char)),
            signed(clamp_i16(i64::from(implied.typo.ascender))),
            signed(clamp_i16(i64::from(implied.typo.descender))),
            signed(clamp_i16(i64::from(implied.typo.line_gap))),
            clamp_u16(i64::from(implied.win.ascent)),
            clamp_u16(i64::from(implied.win.descent)),
        ],
    );
    os2.extend_from_slice(&pages.to_be_bytes()[4..]);
    os2.extend_from_slice(&pages.to_be_bytes()[..4]);
    put_u16s(
        &mut os2,
        &[
            signed(clamp_i16(x_height.unwrap_or(0))),
            signed(clamp_i16(cap_height.unwrap_or(0))),
            0,
            0x20,
            0,
        ],
    );
    Ok(os2)
}

/// One face's tables as they go out: each a range of the input, or bytes written here.
enum TableData<'a> {
    Input { offset: u64, data: &'a [u8] },
    Written(Vec<u8>),
}

impl TableData<'_> {
    fn bytes(&self) -> &[u8] {
        match self {
            TableData::Input { data, .. } => data,
            TableData::Written(data) => data,
        }
    }
}

struct FaceOut<'a> {
    version: u32,
    tables: Vec<(TableTag, TableData<'a>)>,
}

fn face_tables<'a>(reader: &Reader<'a>, offset: u64) -> Result<FaceOut<'a>, SfntError> {
    let tables = Directory::read(reader, offset)?;
    let mut out = Vec::new();
    for (tag, table) in tables.iter() {
        let data = reader.bytes(table.offset, table.length, super::Region::Table(tag))?;
        out.push((
            tag,
            TableData::Input {
                offset: table.offset,
                data,
            },
        ));
    }
    Ok(FaceOut {
        version: reader.u32(offset)?,
        tables: out,
    })
}

/// A single font of `face`'s tables.
fn single(face: &FaceOut<'_>) -> Vec<u8> {
    let mut builder = FontBuilder::new();
    for (tag, data) in &face.tables {
        builder.add_raw(Tag::new(&tag.0), data.bytes().to_vec());
    }
    builder.build()
}

fn pad4(out: &mut Vec<u8>) {
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// A collection of `faces`, each with its own table directory; a table of the input that several faces share is
/// written once.
fn collection(faces: &[FaceOut<'_>]) -> Result<Vec<u8>, SfntError> {
    let too_large = || SfntError::TooLarge(usize::MAX);
    let to_u32 = |value: usize| u32::try_from(value).map_err(|_| too_large());
    let header = 12 + 4 * faces.len();
    let directories: usize = faces.iter().map(|face| 12 + 16 * face.tables.len()).sum();
    let mut data = Vec::new();
    let mut shared: BTreeMap<(u64, usize), usize> = BTreeMap::new();
    let mut placed: Vec<Vec<(TableTag, u32, usize, usize)>> = Vec::new();
    for face in faces {
        let mut records = Vec::new();
        for (tag, table) in &face.tables {
            let bytes = table.bytes();
            let at = match table {
                TableData::Input {
                    offset,
                    data: input,
                } => match shared.get(&(*offset, input.len())) {
                    Some(&at) => at,
                    None => {
                        let at = header + directories + data.len();
                        shared.insert((*offset, input.len()), at);
                        data.extend_from_slice(bytes);
                        pad4(&mut data);
                        at
                    }
                },
                TableData::Written(_) => {
                    let at = header + directories + data.len();
                    data.extend_from_slice(bytes);
                    pad4(&mut data);
                    at
                }
            };
            let checksum = read_fonts::tables::compute_checksum(bytes);
            records.push((*tag, checksum, at, bytes.len()));
        }
        records.sort_by_key(|record| record.0);
        placed.push(records);
    }
    let mut out = Vec::with_capacity(header + directories + data.len());
    out.extend_from_slice(b"ttcf");
    out.extend_from_slice(&0x0001_0000u32.to_be_bytes());
    out.extend_from_slice(&to_u32(faces.len())?.to_be_bytes());
    let mut directory_at = header;
    for face in faces {
        out.extend_from_slice(&to_u32(directory_at)?.to_be_bytes());
        directory_at += 12 + 16 * face.tables.len();
    }
    for (face, records) in faces.iter().zip(&placed) {
        let count = u16::try_from(records.len()).map_err(|_| too_large())?;
        let selector = if count == 0 { 0 } else { count.ilog2() };
        let range = 16u16.checked_shl(selector).unwrap_or(0);
        out.extend_from_slice(&face.version.to_be_bytes());
        for field in [
            count,
            range,
            u16::try_from(selector).unwrap_or(0),
            (count * 16).saturating_sub(range),
        ] {
            out.extend_from_slice(&field.to_be_bytes());
        }
        for (tag, checksum, at, length) in records {
            out.extend_from_slice(&tag.0);
            out.extend_from_slice(&checksum.to_be_bytes());
            out.extend_from_slice(&to_u32(*at)?.to_be_bytes());
            out.extend_from_slice(&to_u32(*length)?.to_be_bytes());
        }
    }
    out.extend_from_slice(&data);
    Ok(out)
}

/// The file with each face that browsers' font sanitizer refuses only for want of an `OS/2` table given one
/// ([`synthesize_os2`]), when the repaired face passes [`sanitizer_refusal`]. `None` when no face is repaired: the
/// file is used as it is. A file over `limits.font_bytes` is left alone.
pub fn repair(bytes: &[u8], limits: &FontLimits) -> Option<Repaired> {
    if bytes.len() > limits.font_bytes {
        return None;
    }
    let reader = Reader::new(bytes);
    let collection_count = match reader.tag(0).ok()? {
        TableTag(tag) if &tag == b"ttcf" => Some(reader.u32(8).ok()?),
        _ => None,
    };
    let count = collection_count.unwrap_or(1);
    let mut faces = Vec::new();
    let mut repairs = Vec::new();
    for index in 0..count {
        let mut face = face_tables(&reader, face_offset(&reader, index).ok()?).ok()?;
        let missing_os2 = sanitizer_refusal(bytes, index)
            == Some(SanitizerRefusal::MissingTable(TableTag(*b"OS/2")));
        let mut repair = None;
        if missing_os2 && let Ok(os2) = synthesize_os2(bytes, index) {
            face.tables
                .push((TableTag(*b"OS/2"), TableData::Written(os2)));
            face.tables.sort_by_key(|(tag, _)| *tag);
            if sanitizer_refusal(&single(&face), 0).is_none() {
                repair = Some(FaceRepair::Os2);
            } else {
                face.tables.retain(|(tag, _)| tag.0 != *b"OS/2");
            }
        }
        faces.push(face);
        repairs.push(repair);
    }
    if repairs.iter().all(Option::is_none) {
        return None;
    }
    let out = match (collection_count, faces.first()) {
        (None, Some(face)) => single(face),
        _ => collection(&faces).ok()?,
    };
    (out.len() <= limits.font_bytes).then_some(Repaired {
        bytes: out,
        faces: repairs,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unicode_range_bits_follow_the_specification_table() {
        assert_eq!(unicode_range_bit(u32::from('A')), Some(0));
        assert_eq!(unicode_range_bit(0xe9), Some(1));
        assert_eq!(unicode_range_bit(0x0416), Some(9));
        assert_eq!(unicode_range_bit(0x1d00), Some(4));
        assert_eq!(unicode_range_bit(0x1f600), Some(NON_PLANE_0));
        // Negative control: a code point in no block of the table.
        assert_eq!(unicode_range_bit(0x0870), None);
        let mut previous = 0;
        for &(first, last, bit) in UNICODE_RANGES {
            assert!(first >= previous && first <= last && bit < 128);
            previous = last + 1;
        }
    }

    #[test]
    fn code_pages_need_ascii_for_the_latin_ones() {
        let ascii: BTreeSet<u32> = (0x20..0x7f).collect();
        let mut latin1 = ascii.clone();
        latin1.insert(u32::from('Þ'));
        assert_eq!(code_pages(&latin1) & 1, 1);
        // Negative control: Þ without ASCII is no Latin 1 code page.
        assert_eq!(code_pages(&[u32::from('Þ')].into_iter().collect()) & 1, 0);
        let mut mac = ascii;
        mac.extend([u32::from('‰'), u32::from('∑')]);
        assert_eq!(code_pages(&mac) >> 29 & 1, 1);
    }

    #[test]
    fn implied_fields_read_the_style_bits_names_and_glyph_box() {
        let hhea = VMetrics {
            ascender: 1544,
            descender: -504,
            line_gap: 0,
        };
        let bounds = GlyphBounds {
            y_min: -838,
            y_max: 2231,
        };
        let bold = implied_os2(BOLD, 0.0, "Bold", &hhea, &bounds);
        assert_eq!(bold.weight_class, 700);
        assert_eq!(bold.fs_selection, 0x80 | 0x20);
        assert_eq!(bold.typo, hhea);
        assert_eq!(
            bold.win,
            WinMetrics {
                ascent: 2231,
                descent: 838
            }
        );
        let oblique = implied_os2(ITALIC, -12.0, "Oblique", &hhea, &bounds);
        assert_eq!(
            (oblique.weight_class, oblique.fs_selection),
            (400, 0x80 | 0x01)
        );
        let light = implied_os2(0, 0.0, "Light", &hhea, &bounds);
        assert_eq!((light.weight_class, light.fs_selection), (300, 0x80 | 0x40));
        // Negative control: a regular face with a slanted name but clear bits is italic by its name.
        let named = implied_os2(0, 0.0, "Italic", &hhea, &bounds);
        assert_eq!(named.fs_selection & 0x01, 0x01);
        // The win metrics never shrink below hhea.
        let small = implied_os2(
            0,
            0.0,
            "Regular",
            &hhea,
            &GlyphBounds {
                y_min: -10,
                y_max: 10,
            },
        );
        assert_eq!(
            small.win,
            WinMetrics {
                ascent: 1544,
                descent: 504
            }
        );
    }
}
