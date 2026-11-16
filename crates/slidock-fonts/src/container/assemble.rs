//! Writes decoded tables as a TrueType/OpenType font or collection, as the WOFF2 reference decoder lays them out:
//! table records in tag order, table data in the order the container lists the tables, each padded to four bytes,
//! fresh table checksums and `head.checkSumAdjustment`.

use super::{ContainerError, ContainerLimits, check_output};
use crate::sfnt::TableTag;

const HEADER: usize = 12;
const RECORD: usize = 16;
/// `head.checkSumAdjustment` is the font checksum subtracted from this.
const CHECKSUM_MAGIC: u32 = 0xb1b0_afba;
const ADJUSTMENT_AT: usize = 8;

/// One decoded table.
pub(super) struct Table {
    pub(super) tag: [u8; 4],
    pub(super) data: Vec<u8>,
}

/// The OpenType checksum of `data`: its big-endian `u32`s summed, the last one padded with zeros.
pub(super) fn checksum(data: &[u8]) -> u32 {
    data.chunks(4).fold(0u32, |sum, chunk| {
        let mut word = [0u8; 4];
        word.iter_mut().zip(chunk).for_each(|(w, b)| *w = *b);
        sum.wrapping_add(u32::from_be_bytes(word))
    })
}

/// The checksum of a table as its record states it: `head`'s with `checkSumAdjustment` read as zero.
pub(super) fn table_checksum(tag: [u8; 4], data: &[u8]) -> u32 {
    let sum = checksum(data);
    match data.get(ADJUSTMENT_AT..ADJUSTMENT_AT + 4) {
        Some(adjustment) if tag == *b"head" => {
            let mut word = [0u8; 4];
            word.copy_from_slice(adjustment);
            sum.wrapping_sub(u32::from_be_bytes(word))
        }
        _ => sum,
    }
}

fn padded(length: usize) -> usize {
    length.next_multiple_of(4)
}

/// `searchRange`, `entrySelector` and `rangeShift` for `count` table records.
fn search_fields(count: u16) -> [u16; 3] {
    let selector = if count == 0 { 0 } else { count.ilog2() };
    let range = (1u32 << selector) * 16;
    let shift = u32::from(count) * 16 - range;
    [
        u16::try_from(range).unwrap_or(u16::MAX),
        u16::try_from(selector).unwrap_or(0),
        u16::try_from(shift).unwrap_or(0),
    ]
}

/// Checks that the tables have distinct tags and a usable `head`, and sorts them by tag.
fn sorted<'t>(
    tables: impl IntoIterator<Item = &'t Table>,
) -> Result<Vec<&'t Table>, ContainerError> {
    let mut tables: Vec<&Table> = tables.into_iter().collect();
    tables.sort_by_key(|table| table.tag);
    for pair in tables.windows(2) {
        if let [a, b] = pair
            && a.tag == b.tag
        {
            return Err(ContainerError::DuplicateTable(TableTag(a.tag)));
        }
    }
    if let Some(head) = tables.iter().find(|table| table.tag == *b"head")
        && head.data.len() < ADJUSTMENT_AT + 4
    {
        return Err(ContainerError::ShortTable(TableTag(*b"head")));
    }
    Ok(tables)
}

fn put_u16(out: &mut Vec<u8>, value: u16) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn put_u32(out: &mut Vec<u8>, value: u32) {
    out.extend_from_slice(&value.to_be_bytes());
}

fn set_u32(out: &mut [u8], at: usize, value: u32) {
    if let Some(slot) = out.get_mut(at..at + 4) {
        slot.copy_from_slice(&value.to_be_bytes());
    }
}

fn offset_u32(offset: usize, limit: u64) -> Result<u32, ContainerError> {
    u32::try_from(offset).map_err(|_| ContainerError::TooLarge { limit })
}

/// A single face.
pub(super) fn font(
    flavor: u32,
    tables: &[Table],
    limits: &ContainerLimits,
) -> Result<Vec<u8>, ContainerError> {
    let records = sorted(tables)?;
    let count = u16::try_from(records.len()).map_err(|_| ContainerError::TableCount {
        count: u32::try_from(records.len()).unwrap_or(u32::MAX),
        limit: limits.max_tables,
    })?;
    let directory = HEADER + RECORD * records.len();
    let size = tables.iter().fold(directory, |size, table| {
        size.saturating_add(padded(table.data.len()))
    });
    check_output(size as u64, limits)?;

    // Table data in the order given, records in tag order.
    let mut offsets = Vec::with_capacity(tables.len());
    let mut offset = directory;
    for table in tables {
        offsets.push((table.tag, offset));
        offset += padded(table.data.len());
    }
    let mut out = Vec::with_capacity(size);
    put_u32(&mut out, flavor);
    put_u16(&mut out, count);
    search_fields(count)
        .into_iter()
        .for_each(|v| put_u16(&mut out, v));
    for table in &records {
        let offset = offsets
            .iter()
            .find(|(tag, _)| *tag == table.tag)
            .map_or(0, |(_, offset)| *offset);
        out.extend_from_slice(&table.tag);
        put_u32(&mut out, table_checksum(table.tag, &table.data));
        put_u32(&mut out, offset_u32(offset, limits.max_output_bytes)?);
        put_u32(
            &mut out,
            offset_u32(table.data.len(), limits.max_output_bytes)?,
        );
    }
    let mut head_at = None;
    for table in tables {
        if table.tag == *b"head" {
            head_at = Some(out.len());
        }
        out.extend_from_slice(&table.data);
        out.resize(padded(out.len()), 0);
    }
    if let Some(head_at) = head_at {
        set_u32(&mut out, head_at + ADJUSTMENT_AT, 0);
        let adjustment = CHECKSUM_MAGIC.wrapping_sub(checksum(&out));
        set_u32(&mut out, head_at + ADJUSTMENT_AT, adjustment);
    }
    Ok(out)
}

/// One face of a collection: its flavor and the indices of its tables in the collection's table list.
pub(super) struct Face {
    pub(super) flavor: u32,
    pub(super) tables: Vec<usize>,
}

/// A collection: a `ttcf` header (version 1.0 or 2.0, the latter with an empty `DSIG` record), each face's table
/// directory, then every table once, in list order. Each face's `head.checkSumAdjustment` is computed over that
/// face's directory and tables; faces that share a `head` keep the last face's.
pub(super) fn collection(
    version: u32,
    faces: &[Face],
    tables: &[Table],
    limits: &ContainerLimits,
) -> Result<Vec<u8>, ContainerError> {
    let mut directories = Vec::with_capacity(faces.len());
    for face in faces {
        let mut members = Vec::with_capacity(face.tables.len());
        for &index in &face.tables {
            members.push((index, tables.get(index).ok_or(ContainerError::Unknown)?));
        }
        members.sort_by_key(|(_, table)| table.tag);
        let only_tables: Vec<&Table> = members.iter().map(|(_, table)| *table).collect();
        sorted(only_tables)?;
        directories.push((face.flavor, members));
    }
    let dsig = if version == 0x0002_0000 { 12 } else { 0 };
    let header = 12 + 4 * faces.len() + dsig;
    let directory_bytes = directories.iter().fold(0usize, |size, (_, members)| {
        size.saturating_add(HEADER + RECORD * members.len())
    });
    let first_table = header.saturating_add(directory_bytes);
    let size = tables.iter().fold(first_table, |size, table| {
        size.saturating_add(padded(table.data.len()))
    });
    check_output(size as u64, limits)?;

    let mut table_offsets = Vec::with_capacity(tables.len());
    let mut offset = first_table;
    for table in tables {
        table_offsets.push(offset);
        offset += padded(table.data.len());
    }

    let limit = limits.max_output_bytes;
    let mut out = Vec::with_capacity(size);
    out.extend_from_slice(b"ttcf");
    put_u32(&mut out, version);
    put_u32(&mut out, offset_u32(faces.len(), limit)?);
    let mut directory_at = header;
    for (_, members) in &directories {
        put_u32(&mut out, offset_u32(directory_at, limit)?);
        directory_at += HEADER + RECORD * members.len();
    }
    out.resize(header, 0);
    let mut face_sums = Vec::with_capacity(directories.len());
    for (flavor, members) in &directories {
        let start = out.len();
        let count = u16::try_from(members.len()).map_err(|_| ContainerError::TableCount {
            count: u32::try_from(members.len()).unwrap_or(u32::MAX),
            limit: limits.max_tables,
        })?;
        put_u32(&mut out, *flavor);
        put_u16(&mut out, count);
        search_fields(count)
            .into_iter()
            .for_each(|v| put_u16(&mut out, v));
        let mut sum = 0u32;
        for (index, table) in members {
            let table_sum = table_checksum(table.tag, &table.data);
            sum = sum.wrapping_add(table_sum);
            out.extend_from_slice(&table.tag);
            put_u32(&mut out, table_sum);
            let at = table_offsets.get(*index).copied().unwrap_or(0);
            put_u32(&mut out, offset_u32(at, limit)?);
            put_u32(&mut out, offset_u32(table.data.len(), limit)?);
        }
        let directory_sum = out.get(start..).map_or(0, checksum);
        face_sums.push(sum.wrapping_add(directory_sum));
    }
    for table in tables {
        out.extend_from_slice(&table.data);
        if table.tag == *b"head" {
            let at = out.len() - table.data.len() + ADJUSTMENT_AT;
            set_u32(&mut out, at, 0);
        }
        out.resize(padded(out.len()), 0);
    }
    for ((_, members), face_sum) in directories.iter().zip(face_sums) {
        if let Some((index, _)) = members.iter().find(|(_, table)| table.tag == *b"head") {
            let at = table_offsets.get(*index).copied().unwrap_or(0) + ADJUSTMENT_AT;
            set_u32(&mut out, at, CHECKSUM_MAGIC.wrapping_sub(face_sum));
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksums_pad_the_last_word_and_skip_head_adjustment() {
        assert_eq!(checksum(&[0, 0, 0, 1, 0, 0, 0, 2]), 3);
        assert_eq!(checksum(&[1]), 0x0100_0000);
        let mut head = vec![0u8; 54];
        head[8..12].copy_from_slice(&0x1234_5678u32.to_be_bytes());
        assert_eq!(table_checksum(*b"head", &head), 0);
        assert_eq!(table_checksum(*b"hhea", &head), 0x1234_5678);
    }

    #[test]
    fn search_fields_follow_the_spec() {
        assert_eq!(search_fields(1), [16, 0, 0]);
        assert_eq!(search_fields(9), [128, 3, 16]);
        assert_eq!(search_fields(16), [256, 4, 0]);
    }

    #[test]
    fn a_font_sums_to_the_magic_number() {
        let tables = [
            Table {
                tag: *b"head",
                data: vec![7; 54],
            },
            Table {
                tag: *b"cmap",
                data: vec![1, 2, 3],
            },
        ];
        let font = font(0x0001_0000, &tables, &ContainerLimits::default()).unwrap();
        assert_eq!(checksum(&font), CHECKSUM_MAGIC);
        // Records in tag order (cmap, head), data in the order given (head, cmap).
        assert_eq!(&font[12..16], b"cmap");
        assert_eq!(&font[28..32], b"head");
        assert_eq!(u32::from_be_bytes(font[36..40].try_into().unwrap()), 44);
        assert_eq!(
            u32::from_be_bytes(font[20..24].try_into().unwrap()),
            44 + 56
        );
        assert_eq!(font.len(), 12 + 32 + 4 + 56);
    }

    #[test]
    fn duplicate_tags_and_short_heads_are_refused() {
        let twice = [
            Table {
                tag: *b"cmap",
                data: vec![],
            },
            Table {
                tag: *b"cmap",
                data: vec![],
            },
        ];
        assert_eq!(
            font(0x0001_0000, &twice, &ContainerLimits::default()),
            Err(ContainerError::DuplicateTable(TableTag(*b"cmap")))
        );
        let short = [Table {
            tag: *b"head",
            data: vec![0; 11],
        }];
        assert_eq!(
            font(0x0001_0000, &short, &ContainerLimits::default()),
            Err(ContainerError::ShortTable(TableTag(*b"head")))
        );
    }
}
