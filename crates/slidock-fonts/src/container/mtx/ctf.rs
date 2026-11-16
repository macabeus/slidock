//! Compact Table Format (W3C MTX submission §4–§7): block 1 is a TrueType font whose `glyf`, `cvt `, `hdmx` and
//! `VDMX` tables are re-encoded and whose `loca` is empty; block 2 holds every glyph's initial push values and block 3
//! the rest of its instructions.
//!
//! The rebuilt `glyf` and `loca` are laid out as libeot writes them: no flag repeats, a glyph's first coordinates
//! always written, push instructions regenerated in runs of one type, each glyph padded to two bytes.

use super::MtxError;
use crate::container::assemble::Table;
use crate::container::cursor::Cursor;
use crate::container::glyf::{self, PointEncoding, SimpleGlyph};
use crate::container::triplet::{self, Point, TripletError};
use crate::container::{
    ContainerError, ContainerLimits, ContainerWarning, MtxStream, Part, check_output,
};
use crate::sfnt::TableTag;

const HOP3: u8 = 0xfb;
const HOP4: u8 = 0xfc;
/// A simple glyph whose bounding box follows instead of being computed.
const EXPLICIT_BBOX: i16 = 0x7fff;
const NPUSHB: u8 = 0x40;
const NPUSHW: u8 = 0x41;
const PUSHB: u8 = 0xb0;
const PUSHW: u8 = 0xb8;
const ARGS_ARE_WORDS: u16 = 0x0001;
const HAVE_SCALE: u16 = 0x0008;
const MORE_COMPONENTS: u16 = 0x0020;
const HAVE_XY_SCALE: u16 = 0x0040;
const HAVE_2X2: u16 = 0x0080;
const HAVE_INSTRUCTIONS: u16 = 0x0100;

/// One record of block 1's table directory.
struct Record {
    tag: [u8; 4],
    offset: u32,
    length: u32,
}

/// Block 1's table data for `record`.
fn table_data<'a>(block: &'a [u8], record: &Record) -> Result<&'a [u8], ContainerError> {
    let start = record.offset as usize;
    start
        .checked_add(record.length as usize)
        .and_then(|end| block.get(start..end))
        .ok_or(MtxError::TableBounds(TableTag(record.tag)).into())
}

fn records(block: &[u8], limits: &ContainerLimits) -> Result<Vec<Record>, ContainerError> {
    let mut cursor = Cursor::new(block, Part::TableDirectory);
    cursor.u32_be()?;
    let count = cursor.u16_be()?;
    if count > limits.max_tables {
        return Err(ContainerError::TableCount {
            count: u32::from(count),
            limit: limits.max_tables,
        });
    }
    cursor.skip(6)?;
    let mut records = Vec::with_capacity(usize::from(count));
    for _ in 0..count {
        let tag = cursor.tag()?;
        cursor.u32_be()?;
        records.push(Record {
            tag,
            offset: cursor.u32_be()?,
            length: cursor.u32_be()?,
        });
    }
    Ok(records)
}

/// The data of the first table tagged `tag`.
fn find<'a>(
    block: &'a [u8],
    records: &[Record],
    tag: &[u8; 4],
) -> Result<Option<&'a [u8]>, ContainerError> {
    records
        .iter()
        .find(|record| record.tag == *tag)
        .map(|record| table_data(block, record))
        .transpose()
}

fn required<'a>(
    block: &'a [u8],
    records: &[Record],
    tag: &[u8; 4],
) -> Result<&'a [u8], ContainerError> {
    find(block, records, tag)?.ok_or(MtxError::MissingTable(TableTag(*tag)).into())
}

fn be_u16(data: &[u8], at: usize, tag: &[u8; 4]) -> Result<u16, ContainerError> {
    data.get(at..at + 2)
        .and_then(|bytes| bytes.try_into().ok())
        .map(u16::from_be_bytes)
        .ok_or(ContainerError::ShortTable(TableTag(*tag)))
}

/// The font's tables in block 1's directory order, decoded; `loca` is added after the others when the directory
/// lacks it. An `hdmx` or `VDMX` table that does not decode is dropped with a warning.
pub(super) fn tables(
    blocks: [&[u8]; 3],
    limits: &ContainerLimits,
) -> Result<(Vec<Table>, Vec<ContainerWarning>), ContainerError> {
    let [block, push, code] = blocks;
    let records = records(block, limits)?;
    let head = required(block, &records, b"head")?;
    let maxp = required(block, &records, b"maxp")?;
    let long_loca = be_u16(head, 50, b"head")? != 0;
    let glyph_count = be_u16(maxp, 4, b"maxp")?;
    if glyph_count > limits.max_glyphs {
        return Err(ContainerError::GlyphCount {
            count: u32::from(glyph_count),
            limit: limits.max_glyphs,
        });
    }
    let glyf = find(block, &records, b"glyf")?;
    let mut rebuilt = match glyf {
        Some(glyf) => Some(glyf_and_loca(
            glyf,
            push,
            code,
            glyph_count,
            long_loca,
            limits,
        )?),
        None => None,
    };
    if rebuilt.is_none() && records.iter().any(|record| record.tag == *b"loca") {
        return Err(MtxError::MissingTable(TableTag(*b"glyf")).into());
    }

    let mut tables = Vec::with_capacity(records.len() + 1);
    let mut warnings = Vec::new();
    let mut has_loca = false;
    // Records may all point at the same bytes, so the copies are budgeted as they are made.
    let mut size = rebuilt
        .as_ref()
        .map_or(0, |(glyf, loca)| glyf.len() + loca.len());
    for record in &records {
        let data = match &record.tag {
            b"glyf" => rebuilt
                .as_mut()
                .map(|(glyf, _)| std::mem::take(glyf))
                .unwrap_or_default(),
            b"loca" => {
                has_loca = true;
                rebuilt
                    .as_mut()
                    .map(|(_, loca)| std::mem::take(loca))
                    .unwrap_or_default()
            }
            b"cvt " => cvt(table_data(block, record)?)?,
            b"hdmx" => {
                let metrics = Metrics::read(block, &records, glyph_count);
                let data = table_data(block, record)?;
                match metrics.and_then(|metrics| hdmx(data, &metrics, limits)) {
                    Ok(data) => data,
                    Err(_) => {
                        warnings.push(ContainerWarning::DroppedTable(TableTag(record.tag)));
                        continue;
                    }
                }
            }
            b"VDMX" => match vdmx(table_data(block, record)?, limits) {
                Ok(data) => data,
                Err(_) => {
                    warnings.push(ContainerWarning::DroppedTable(TableTag(record.tag)));
                    continue;
                }
            },
            _ => {
                let data = table_data(block, record)?;
                check_output((size + data.len()) as u64, limits)?;
                data.to_vec()
            }
        };
        size += data.len();
        check_output(size as u64, limits)?;
        tables.push(Table {
            tag: record.tag,
            data,
        });
    }
    if let (false, Some((_, loca))) = (has_loca, rebuilt) {
        tables.push(Table {
            tag: *b"loca",
            data: loca,
        });
    }
    Ok((tables, warnings))
}

/// The `cvt ` table: a count, then each value as a 1-3 byte difference from the previous one (§5.2).
fn cvt(data: &[u8]) -> Result<Vec<u8>, ContainerError> {
    const WORD: u8 = 238;
    const NEGATIVE: u8 = 239;
    const POSITIVE: u8 = 248;
    const STEP: i16 = 238;
    let mut cursor = Cursor::new(data, Part::Table(TableTag(*b"cvt ")));
    let count = cursor.u16_be()?;
    // Every value takes at least one byte.
    if usize::from(count) > cursor.remaining() {
        return Err(ContainerError::Truncated(Part::Table(TableTag(*b"cvt "))));
    }
    let mut out = Vec::with_capacity(2 * usize::from(count));
    let mut value: i16 = 0;
    for _ in 0..count {
        let code = cursor.u8()?;
        let delta = match code {
            POSITIVE.. => STEP * i16::from(code - POSITIVE + 1) + i16::from(cursor.u8()?),
            NEGATIVE.. => -(STEP * i16::from(code - NEGATIVE) + i16::from(cursor.u8()?)),
            WORD => cursor.i16_be()?,
            _ => i16::from(code),
        };
        value = value.wrapping_add(delta);
        out.extend_from_slice(&value.to_be_bytes());
    }
    Ok(out)
}

/// The three glyph streams.
struct Streams<'a> {
    glyphs: Cursor<'a>,
    push: Cursor<'a>,
    code: Cursor<'a>,
}

fn glyf_and_loca(
    glyf: &[u8],
    push: &[u8],
    code: &[u8],
    glyph_count: u16,
    long_loca: bool,
    limits: &ContainerLimits,
) -> Result<(Vec<u8>, Vec<u8>), ContainerError> {
    let mut streams = Streams {
        glyphs: Cursor::new(glyf, Part::Mtx(MtxStream::Glyphs)),
        push: Cursor::new(push, Part::Mtx(MtxStream::PushData)),
        code: Cursor::new(code, Part::Mtx(MtxStream::Instructions)),
    };
    let mut out = Vec::new();
    let mut offsets = Vec::with_capacity(usize::from(glyph_count) + 1);
    offsets.push(0);
    for _ in 0..glyph_count {
        glyph(&mut streams, &mut out)?;
        if out.len() % 2 != 0 {
            out.push(0);
        }
        check_output(out.len() as u64, limits)?;
        offsets.push(u32::try_from(out.len()).map_err(|_| ContainerError::LocaOverflow)?);
    }
    let loca = glyf::loca(&offsets, long_loca)?;
    Ok((out, loca))
}

fn glyph(streams: &mut Streams<'_>, out: &mut Vec<u8>) -> Result<(), ContainerError> {
    let mut contours = streams.glyphs.i16_be()?;
    if contours < 0 {
        return composite(streams, out);
    }
    let mut bbox = None;
    if contours == EXPLICIT_BBOX {
        contours = streams.glyphs.i16_be()?;
        bbox = Some([
            streams.glyphs.i16_be()?,
            streams.glyphs.i16_be()?,
            streams.glyphs.i16_be()?,
            streams.glyphs.i16_be()?,
        ]);
    }
    match contours {
        0 => Ok(()),
        1.. => simple(streams, out, contours.unsigned_abs(), bbox),
        _ => Err(ContainerError::GlyphCoordinate),
    }
}

fn simple(
    streams: &mut Streams<'_>,
    out: &mut Vec<u8>,
    contours: u16,
    bbox: Option<[i16; 4]>,
) -> Result<(), ContainerError> {
    // The first count is the first contour's last point; the others are point counts.
    let mut contour_points = Vec::with_capacity(usize::from(contours));
    let mut total: u32 = 0;
    for i in 0..contours {
        let count = u32::from(streams.glyphs.u255()?) + u32::from(i == 0);
        total += count;
        contour_points.push(u16::try_from(count).map_err(|_| ContainerError::GlyphCoordinate)?);
    }
    let total = usize::try_from(total).map_err(|_| ContainerError::GlyphCoordinate)?;
    let flags = streams.glyphs.bytes(total)?;
    let mut points: Vec<Point> = Vec::new();
    let read = triplet::decode_into(flags, streams.glyphs.rest(), &mut points).map_err(
        |error| match error {
            TripletError::Truncated => ContainerError::Truncated(Part::Mtx(MtxStream::Glyphs)),
            TripletError::Overflow => ContainerError::GlyphCoordinate,
        },
    )?;
    streams.glyphs.skip(read)?;
    let instructions = instructions(streams)?;
    glyf::write_simple(
        out,
        &SimpleGlyph {
            contour_points: &contour_points,
            points: &points,
            instructions: &instructions,
            bbox,
            overlap: false,
        },
        PointEncoding::Mtx,
    )?;
    Ok(())
}

fn composite(streams: &mut Streams<'_>, out: &mut Vec<u8>) -> Result<(), ContainerError> {
    out.extend_from_slice(&(-1i16).to_be_bytes());
    out.extend_from_slice(streams.glyphs.bytes(8)?);
    loop {
        let flags = streams.glyphs.u16_be()?;
        let arguments = if flags & ARGS_ARE_WORDS != 0 { 4 } else { 2 };
        let transform = if flags & HAVE_2X2 != 0 {
            8
        } else if flags & HAVE_XY_SCALE != 0 {
            4
        } else if flags & HAVE_SCALE != 0 {
            2
        } else {
            0
        };
        out.extend_from_slice(&flags.to_be_bytes());
        out.extend_from_slice(streams.glyphs.bytes(2 + arguments + transform)?);
        if flags & MORE_COMPONENTS == 0 {
            if flags & HAVE_INSTRUCTIONS != 0 {
                let instructions = instructions(streams)?;
                let length = u16::try_from(instructions.len())
                    .map_err(|_| ContainerError::GlyphCoordinate)?;
                out.extend_from_slice(&length.to_be_bytes());
                out.extend_from_slice(&instructions);
            }
            return Ok(());
        }
    }
}

/// A glyph's instructions: its push values from block 2 as push instructions, then `codeSize` bytes of block 3.
fn instructions(streams: &mut Streams<'_>) -> Result<Vec<u8>, ContainerError> {
    let push_count = streams.glyphs.u255()?;
    let values = push_values(&mut streams.push, push_count)?;
    let code_size = streams.glyphs.u255()?;
    let mut out = Vec::new();
    write_pushes(&values, &mut out);
    out.extend_from_slice(streams.code.bytes(usize::from(code_size))?);
    if out.len() > usize::from(u16::MAX) {
        return Err(ContainerError::GlyphCoordinate);
    }
    Ok(out)
}

/// The `255SHORT` type (§6.1.2).
fn short255(cursor: &mut Cursor<'_>) -> Result<i16, ContainerError> {
    const FLIP_SIGN: u8 = 250;
    const WORD: u8 = 253;
    const ONE_MORE_BYTE_2: u8 = 254;
    const ONE_MORE_BYTE_1: u8 = 255;
    const LOWEST: i16 = 250;
    let mut code = cursor.u8()?;
    if code == WORD {
        return cursor.i16_be();
    }
    let negative = code == FLIP_SIGN;
    if negative {
        code = cursor.u8()?;
    }
    let value = match code {
        ONE_MORE_BYTE_1 => LOWEST + i16::from(cursor.u8()?),
        ONE_MORE_BYTE_2 => 2 * LOWEST + i16::from(cursor.u8()?),
        _ => i16::from(code),
    };
    Ok(if negative { -value } else { value })
}

/// `count` push values from block 2 (§6.2.2): `255SHORT`s, where a hop code repeats the value two back (`A, x,
/// Hop3, y` is `A, x, A, y, A`; `A, x, Hop4, y, z` is `A, x, A, y, A, z, A`).
fn push_values(push: &mut Cursor<'_>, count: u16) -> Result<Vec<i16>, ContainerError> {
    let count = usize::from(count);
    let mut values: Vec<i16> = Vec::with_capacity(count);
    while values.len() < count {
        let left = count - values.len();
        let code = push.rest().first().copied();
        let hop = match code {
            Some(HOP3) => 3,
            Some(HOP4) => 5,
            _ => {
                values.push(short255(push)?);
                continue;
            }
        };
        let repeated = values
            .len()
            .checked_sub(2)
            .and_then(|at| values.get(at))
            .copied();
        let Some(repeated) = repeated.filter(|_| left >= hop) else {
            return Err(MtxError::HopCode.into());
        };
        push.skip(1)?;
        values.push(repeated);
        for _ in 0..hop / 2 {
            values.push(short255(push)?);
            values.push(repeated);
        }
    }
    Ok(values)
}

/// Push instructions for `values`: each run of bytes (0-255) or words, at most 255 long, as `PUSHB`/`PUSHW` up to
/// eight values and `NPUSHB`/`NPUSHW` beyond.
fn write_pushes(values: &[i16], out: &mut Vec<u8>) {
    let is_byte = |value: &i16| (0..256).contains(value);
    let mut rest = values;
    while let Some(first) = rest.first() {
        let bytes = is_byte(first);
        let run = rest
            .iter()
            .take(255)
            .take_while(|value| is_byte(value) == bytes)
            .count();
        let (now, later) = rest.split_at(run);
        // `run` is 1..=255.
        let run = u8::try_from(run).unwrap_or(u8::MAX);
        match (bytes, run) {
            (true, 1..8) => out.push(PUSHB | (run - 1)),
            (false, 1..8) => out.push(PUSHW | (run - 1)),
            (true, _) => out.extend_from_slice(&[NPUSHB, run]),
            (false, _) => out.extend_from_slice(&[NPUSHW, run]),
        }
        for value in now {
            if bytes {
                out.push(value.to_be_bytes()[1]);
            } else {
                out.extend_from_slice(&value.to_be_bytes());
            }
        }
        rest = later;
    }
}

/// The values `hdmx` predicts from: `head.unitsPerEm` and every glyph's advance width.
struct Metrics {
    units_per_em: u16,
    advances: Vec<u16>,
}

impl Metrics {
    fn read(block: &[u8], records: &[Record], glyph_count: u16) -> Result<Self, ContainerError> {
        let head = required(block, records, b"head")?;
        let hhea = required(block, records, b"hhea")?;
        let hmtx = required(block, records, b"hmtx")?;
        let units_per_em = be_u16(head, 18, b"head")?;
        let metrics = be_u16(hhea, 34, b"hhea")?;
        if units_per_em == 0 || metrics == 0 {
            return Err(ContainerError::ShortTable(TableTag(*b"head")));
        }
        let mut advances = Vec::with_capacity(usize::from(glyph_count));
        let mut last = 0;
        for glyph in 0..usize::from(glyph_count) {
            if glyph < usize::from(metrics) {
                last = be_u16(hmtx, 4 * glyph, b"hmtx")?;
            }
            advances.push(last);
        }
        Ok(Metrics {
            units_per_em,
            advances,
        })
    }
}

/// Reads the magnitude-dependent values of §5.3: `n` one bits, a zero, then (for `n > 0`) a sign bit; bits go from
/// the least significant bit of each byte.
struct MagnitudeReader<'a> {
    data: &'a [u8],
    bit: usize,
    part: Part,
}

impl MagnitudeReader<'_> {
    fn bit(&mut self) -> Result<bool, ContainerError> {
        let byte = self
            .data
            .get(self.bit / 8)
            .ok_or(ContainerError::Truncated(self.part))?;
        let value = byte >> (self.bit % 8) & 1 != 0;
        self.bit += 1;
        Ok(value)
    }

    fn value(&mut self) -> Result<i32, ContainerError> {
        let mut magnitude: i32 = 0;
        while self.bit()? {
            magnitude = magnitude
                .checked_add(1)
                .ok_or(ContainerError::Truncated(self.part))?;
        }
        Ok(if magnitude != 0 && self.bit()? {
            -magnitude
        } else {
            magnitude
        })
    }

    /// The offset of the next whole byte.
    fn next_byte(&self) -> usize {
        self.bit.div_ceil(8)
    }
}

/// The `hdmx` table (§5.4): each width is the advance width scaled to the record's size, plus a magnitude-coded
/// surprise. A version of `0xFFFF - v` marks a table stored as it was, with version `v`.
fn hdmx(
    data: &[u8],
    metrics: &Metrics,
    limits: &ContainerLimits,
) -> Result<Vec<u8>, ContainerError> {
    const TAG: TableTag = TableTag(*b"hdmx");
    let mut cursor = Cursor::new(data, Part::Table(TAG));
    let version = cursor.u16_be()?;
    if version != 0 {
        return stored(data, TAG, version, 0);
    }
    let records = cursor.u16_be()?;
    let record_size = cursor.u32_be()?;
    let glyphs = metrics.advances.len();
    if record_size as usize != (glyphs + 2).next_multiple_of(4) {
        return Err(ContainerError::ShortTable(TAG));
    }
    let mut sizes = Vec::with_capacity(usize::from(records));
    for _ in 0..records {
        sizes.push((cursor.u8()?, cursor.u8()?));
    }
    // Every width takes at least one bit.
    if glyphs * usize::from(records) > cursor.remaining() * 8 {
        return Err(ContainerError::Truncated(Part::Table(TAG)));
    }
    let size = 8 + usize::from(records) * record_size as usize;
    check_output(size as u64, limits)?;
    let mut bits = MagnitudeReader {
        data: cursor.rest(),
        bit: 0,
        part: Part::Table(TAG),
    };
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(&version.to_be_bytes());
    out.extend_from_slice(&records.to_be_bytes());
    out.extend_from_slice(&record_size.to_be_bytes());
    let units_per_em = i64::from(metrics.units_per_em);
    for (ppem, max_width) in sizes {
        let start = out.len();
        out.extend_from_slice(&[ppem, max_width]);
        for &advance in &metrics.advances {
            let scaled =
                (64 * i64::from(ppem) * i64::from(advance) + units_per_em / 2) / units_per_em;
            let predicted = (scaled + 32) / 64;
            let width = predicted + i64::from(bits.value()?);
            out.push(u8::try_from(width).map_err(|_| ContainerError::ShortTable(TAG))?);
        }
        out.resize(start + record_size as usize, 0);
    }
    Ok(out)
}

/// A table stored as it was, with its version `0xFFFF - real` restored, when `real <= highest`.
fn stored(
    data: &[u8],
    tag: TableTag,
    version: u16,
    highest: u16,
) -> Result<Vec<u8>, ContainerError> {
    let real = 0xffff - version;
    if real > highest {
        return Err(ContainerError::ShortTable(tag));
    }
    let mut out = data.to_vec();
    if let Some(slot) = out.get_mut(..2) {
        slot.copy_from_slice(&real.to_be_bytes());
    }
    Ok(out)
}

/// The `VDMX` table (§5.5). The header, ratios and offsets are as the font had them; from `offset[0]` come the
/// groups, each with its multipliers and magnitude-coded errors from the predicted size, `yMax` and `yMin`. The
/// groups are written back from `offset[0]` and every offset must start one.
fn vdmx(data: &[u8], limits: &ContainerLimits) -> Result<Vec<u8>, ContainerError> {
    const TAG: TableTag = TableTag(*b"VDMX");
    let mut cursor = Cursor::new(data, Part::Table(TAG));
    let version = cursor.u16_be()?;
    if version > 1 {
        return stored(data, TAG, version, 1);
    }
    let groups = cursor.u16_be()?;
    let ratios = cursor.u16_be()?;
    cursor.skip(4 * usize::from(ratios))?;
    let mut offsets = Vec::with_capacity(usize::from(ratios));
    for _ in 0..ratios {
        offsets.push(usize::from(cursor.u16_be()?));
    }
    let header_end = cursor.position();
    let first = *offsets.first().ok_or(ContainerError::ShortTable(TAG))?;
    if first < header_end {
        return Err(ContainerError::ShortTable(TAG));
    }
    let mut out = data
        .get(..header_end)
        .ok_or(ContainerError::Truncated(Part::Table(TAG)))?
        .to_vec();
    out.resize(first, 0);
    let mut at = first;
    let mut starts = Vec::with_capacity(usize::from(groups));
    for _ in 0..groups {
        starts.push(out.len());
        let mut group = Cursor::new(data, Part::Table(TAG));
        group.skip(at)?;
        let records = group.u16_be()?;
        let y_max_multiplier = i64::from(group.i16_be()?);
        let y_min_multiplier = i64::from(group.i16_be()?);
        if usize::from(records) > group.remaining() * 8 / 3 {
            return Err(ContainerError::Truncated(Part::Table(TAG)));
        }
        check_output((out.len() + 4 + 6 * usize::from(records)) as u64, limits)?;
        let mut bits = MagnitudeReader {
            data: group.rest(),
            bit: 0,
            part: Part::Table(TAG),
        };
        let mut entries = Vec::with_capacity(6 * usize::from(records));
        let (mut first_size, mut last_size) = (0u8, 0u8);
        // Sizes and heights are computed in i64: a size's error can be as large as the table has bits.
        let mut predicted_size: i64 = 8;
        for record in 0..records {
            let size = predicted_size + i64::from(bits.value()?);
            let y_max = (size * y_max_multiplier + 1024) / 2048 + i64::from(bits.value()?);
            let y_min = -((size * y_min_multiplier + 1024) / 2048) + i64::from(bits.value()?);
            let invalid = |_| ContainerError::ShortTable(TAG);
            entries.extend_from_slice(&u16::try_from(size).map_err(invalid)?.to_be_bytes());
            entries.extend_from_slice(&i16::try_from(y_max).map_err(invalid)?.to_be_bytes());
            entries.extend_from_slice(&i16::try_from(y_min).map_err(invalid)?.to_be_bytes());
            let size_byte = u8::try_from(size).map_err(invalid)?;
            if record == 0 {
                first_size = size_byte;
            }
            last_size = size_byte;
            predicted_size = size + 1;
        }
        out.extend_from_slice(&records.to_be_bytes());
        out.extend_from_slice(&[first_size, last_size]);
        out.extend_from_slice(&entries);
        at = group.position() + bits.next_byte();
    }
    if offsets.iter().any(|offset| !starts.contains(offset)) {
        return Err(ContainerError::ShortTable(TAG));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cvt_values_accumulate_from_one_to_three_byte_differences() {
        let data = [
            0, 6,   // six values
            100, // +100
            248, 10, // +238 + 10
            239, 5, // -5
            247, 0, // -(238 * 8)
            238, 0x80, 0x00, // -32768
            255, 255, // +238 * 8 + 255, wrapping
        ];
        let out = cvt(&data).unwrap();
        let values: Vec<i16> = out
            .chunks(2)
            .map(|pair| i16::from_be_bytes([pair[0], pair[1]]))
            .collect();
        let mut expected = Vec::new();
        let mut value: i16 = 0;
        for delta in [100, 248, -5, -1904, -32768, 2159] {
            value = value.wrapping_add(delta);
            expected.push(value);
        }
        assert_eq!(values, expected);
        assert_eq!(
            cvt(&[0, 3, 1, 2]),
            Err(ContainerError::Truncated(Part::Table(TableTag(*b"cvt "))))
        );
    }

    #[test]
    fn short255_reads_every_code() {
        let read = |bytes: &[u8]| short255(&mut Cursor::new(bytes, Part::Header)).unwrap();
        assert_eq!(read(&[0]), 0);
        assert_eq!(read(&[249]), 249);
        assert_eq!(read(&[251]), 251);
        assert_eq!(read(&[255, 0]), 250);
        assert_eq!(read(&[254, 255]), 755);
        assert_eq!(read(&[250, 7]), -7);
        assert_eq!(read(&[250, 255, 5]), -255);
        assert_eq!(read(&[250, 254, 0]), -500);
        assert_eq!(read(&[250, 253]), -253);
        assert_eq!(read(&[253, 0x80, 0x01]), -32767);
    }

    #[test]
    fn hop_codes_repeat_the_value_two_back() {
        let values =
            |bytes: &[u8], count| push_values(&mut Cursor::new(bytes, Part::Header), count);
        assert_eq!(values(&[9, 1, HOP3, 2], 5).unwrap(), [9, 1, 9, 2, 9]);
        assert_eq!(
            values(&[9, 1, HOP4, 2, 3], 7).unwrap(),
            [9, 1, 9, 2, 9, 3, 9]
        );
        assert_eq!(
            values(&[9, 1, HOP3, 2, HOP3, 4], 8).unwrap(),
            [9, 1, 9, 2, 9, 2, 4, 2]
        );
        // A hop needs two values before it and room for all it writes.
        assert_eq!(values(&[1, HOP3, 2], 4), Err(MtxError::HopCode.into()));
        assert_eq!(values(&[9, 1, HOP3, 2], 4), Err(MtxError::HopCode.into()));
        assert_eq!(
            values(&[9, 1, HOP4, 2, 3], 6),
            Err(MtxError::HopCode.into())
        );
        assert_eq!(
            values(&[1], 2),
            Err(ContainerError::Truncated(Part::Header))
        );
    }

    #[test]
    fn pushes_are_runs_of_one_type() {
        let mut out = Vec::new();
        write_pushes(&[1, 2, 300, -1, 4], &mut out);
        assert_eq!(
            out,
            [PUSHB | 1, 1, 2, PUSHW | 1, 0x01, 0x2c, 0xff, 0xff, PUSHB, 4]
        );
        let mut out = Vec::new();
        write_pushes(&[7; 8], &mut out);
        assert_eq!(out[..2], [NPUSHB, 8]);
        assert_eq!(out.len(), 10);
        let mut out = Vec::new();
        write_pushes(&[-2; 300], &mut out);
        assert_eq!(out[..2], [NPUSHW, 255]);
        assert_eq!(out[2 + 510..2 + 512], [NPUSHW, 45]);
        assert_eq!(out.len(), 2 + 510 + 2 + 90);
        let mut out = Vec::new();
        write_pushes(&[], &mut out);
        assert!(out.is_empty());
    }

    #[test]
    fn magnitude_values_follow_the_specification_example() {
        let mut bits = MagnitudeReader {
            data: &[0x90, 0x48, 0x84],
            bit: 0,
            part: Part::Header,
        };
        let values: Vec<i32> = (0..13).map(|_| bits.value().unwrap()).collect();
        assert_eq!(values, [0, 0, 0, 0, 1, 1, 0, 1, 1, 0, 1, 0, 0]);
        assert_eq!(bits.value(), Err(ContainerError::Truncated(Part::Header)));
        let mut bits = MagnitudeReader {
            data: &[0b0000_1011, 0b0000_0001],
            bit: 0,
            part: Part::Header,
        };
        // 1 1 0 1 -> -2; then 0 0 0 0 -> 0 0 0 0; then 1 ...
        assert_eq!(bits.value().unwrap(), -2);
        assert_eq!(bits.next_byte(), 1);
    }
}
