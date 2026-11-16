//! The top of one glyph's box: the code point through a Unicode `cmap` subtable, then the stored `glyf` header
//! through `maxp` and `loca`. Fonts without TrueType outlines have no such box.

use super::SfntError;
use super::reader::{Directory, Reader, Table};

/// More groups than any real format 12 subtable holds; a larger count is a lie.
const MAX_CMAP_GROUPS: u32 = 1 << 20;

/// The glyph a code point maps to in the first Unicode subtable (format 4 or 12) that maps it.
///
/// A format 4 segment that maps the code point to glyph 0 through its glyph array ends the search. The `cmap`
/// table itself was checked against the file; its subtables are read wherever their offsets point, and an offset
/// outside the file fails the face.
pub(super) fn glyph_of(
    reader: &Reader<'_>,
    cmap: Table,
    code_point: u32,
) -> Result<Option<u64>, SfntError> {
    let code_point = u64::from(code_point);
    let end = cmap.offset + cmap.length;
    let count = reader.u16(cmap.offset + 2)?;
    for i in 0..u64::from(count) {
        let record = cmap.offset + 4 + i * 8;
        if record + 8 > end {
            return Ok(None);
        }
        let platform = reader.u16(record)?;
        let encoding = reader.u16(record + 2)?;
        if !(platform == 0 || (platform == 3 && matches!(encoding, 1 | 10))) {
            continue;
        }
        let sub = cmap.offset + u64::from(reader.u32(record + 4)?);
        match reader.u16(sub)? {
            4 => {
                if let Some(glyph) = format4(reader, sub, code_point)? {
                    return Ok(glyph);
                }
            }
            12 => {
                if let Some(glyph) = format12(reader, sub, code_point)? {
                    return Ok(Some(glyph));
                }
            }
            _ => {}
        }
    }
    Ok(None)
}

/// `Some(answer)` when a segment covers the code point, `None` to try the next subtable.
fn format4(
    reader: &Reader<'_>,
    sub: u64,
    code_point: u64,
) -> Result<Option<Option<u64>>, SfntError> {
    let seg_count_x2 = u64::from(reader.u16(sub + 6)?);
    for k in 0..seg_count_x2.div_ceil(2) {
        let end_code = u64::from(reader.u16(sub + 14 + k * 2)?);
        if end_code < code_point {
            continue;
        }
        let start_at = sub + 16 + seg_count_x2 + k * 2;
        let start = u64::from(reader.u16(start_at)?);
        if start > code_point {
            break;
        }
        let delta = u64::from(reader.u16(start_at + seg_count_x2)?);
        let range_at = start_at + seg_count_x2 * 2;
        let range = u64::from(reader.u16(range_at)?);
        if range == 0 {
            return Ok(Some(Some((code_point + delta) & 0xffff)));
        }
        let glyph = u64::from(reader.u16(range_at + range + (code_point - start) * 2)?);
        return Ok(Some((glyph != 0).then_some((glyph + delta) & 0xffff)));
    }
    Ok(None)
}

fn format12(reader: &Reader<'_>, sub: u64, code_point: u64) -> Result<Option<u64>, SfntError> {
    let groups = reader.u32(sub + 12)?;
    if groups > MAX_CMAP_GROUPS {
        return Err(SfntError::CmapGroups(groups));
    }
    for k in 0..u64::from(groups) {
        let group = sub + 16 + k * 12;
        let first = u64::from(reader.u32(group)?);
        let last = u64::from(reader.u32(group + 4)?);
        if (first..=last).contains(&code_point) {
            return Ok(Some(u64::from(reader.u32(group + 8)?) + code_point - first));
        }
    }
    Ok(None)
}

/// The yMax of a TrueType glyph's header, or `None` when the font has no outlines there or the glyph draws
/// nothing (a header needs 10 bytes).
pub(super) fn glyph_top(
    reader: &Reader<'_>,
    tables: &Directory,
    long_offsets: bool,
    glyph: u64,
) -> Result<Option<i32>, SfntError> {
    let (Some(maxp), Some(loca), Some(glyf)) = (
        tables.get(b"maxp"),
        tables.get(b"loca"),
        tables.get(b"glyf"),
    ) else {
        return Ok(None);
    };
    if maxp.length < 6 || glyph >= u64::from(reader.u16(maxp.offset + 4)?) {
        return Ok(None);
    }
    let size = if long_offsets { 4 } else { 2 };
    if (glyph + 2) * size > loca.length {
        return Ok(None);
    }
    let at = |index: u64| -> Result<i64, SfntError> {
        Ok(if long_offsets {
            i64::from(reader.u32(loca.offset + index * 4)?)
        } else {
            i64::from(reader.u16(loca.offset + index * 2)?) * 2
        })
    };
    let (start, next) = (at(glyph)?, at(glyph + 1)?);
    let glyf_length = i64::try_from(glyf.length).unwrap_or(i64::MAX);
    if next - start < 10 || next > glyf_length {
        return Ok(None);
    }
    Ok(Some(i32::from(
        reader.i16(glyf.offset + start.unsigned_abs() + 8)?,
    )))
}
