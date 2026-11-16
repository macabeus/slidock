//! Bounds-checked big-endian reads at absolute file positions, and the table directory of one face.
//!
//! Positions are `u64` so that offset arithmetic on 32-bit fields never wraps, on wasm32 included.

use std::collections::BTreeMap;

use read_fonts::FontData;
use read_fonts::types::Scalar;

use super::{Region, SfntError, TableTag};

/// The TrueType and OpenType sfnt versions a face's directory may start with: 1.0, `OTTO` and `true`.
const SFNT_VERSIONS: [u32; 3] = [0x0001_0000, 0x4f54_544f, 0x7472_7565];

const MAX_TABLES: u16 = 512;

pub(super) struct Reader<'a> {
    data: FontData<'a>,
}

impl<'a> Reader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        Reader {
            data: FontData::new(bytes),
        }
    }

    fn len(&self) -> u64 {
        self.data.len() as u64
    }

    /// Fails unless `length` bytes at `offset` lie inside the file.
    pub(super) fn check(&self, offset: u64, length: u64, what: Region) -> Result<(), SfntError> {
        if offset.saturating_add(length) > self.len() {
            return Err(SfntError::OutsideFile(what));
        }
        Ok(())
    }

    fn read<T: Scalar>(&self, at: u64) -> Result<T, SfntError> {
        let at = usize::try_from(at).map_err(|_| SfntError::OutsideFile(Region::FontData))?;
        self.data
            .read_at(at)
            .map_err(|_| SfntError::OutsideFile(Region::FontData))
    }

    pub(super) fn u16(&self, at: u64) -> Result<u16, SfntError> {
        self.read(at)
    }

    pub(super) fn i16(&self, at: u64) -> Result<i16, SfntError> {
        self.read(at)
    }

    pub(super) fn u32(&self, at: u64) -> Result<u32, SfntError> {
        self.read(at)
    }

    pub(super) fn i32(&self, at: u64) -> Result<i32, SfntError> {
        self.read(at)
    }

    pub(super) fn tag(&self, at: u64) -> Result<TableTag, SfntError> {
        self.read::<u32>(at).map(|raw| TableTag(raw.to_be_bytes()))
    }

    /// The `length` bytes at `start`, which must lie inside the file.
    pub(super) fn bytes(
        &self,
        start: u64,
        length: u64,
        what: Region,
    ) -> Result<&'a [u8], SfntError> {
        self.check(start, length, what)?;
        let range = usize::try_from(start)
            .ok()
            .zip(usize::try_from(start + length).ok())
            .and_then(|(start, end)| self.data.as_bytes().get(start..end));
        range.ok_or(SfntError::OutsideFile(what))
    }
}

/// Where a table lies in the file.
#[derive(Clone, Copy, Debug)]
pub(super) struct Table {
    pub(super) offset: u64,
    pub(super) length: u64,
}

/// A face's tables by tag. A tag the directory lists twice names its last record.
pub(super) struct Directory {
    tables: BTreeMap<TableTag, Table>,
}

impl Directory {
    /// Reads the directory at `offset`. Every table record must lie inside the file, and the records together must
    /// be no longer than the file: overlapping records could otherwise make a small file stand for gigabytes of
    /// tables to whatever copies them.
    pub(super) fn read(reader: &Reader<'_>, offset: u64) -> Result<Directory, SfntError> {
        let version = reader.u32(offset)?;
        if !SFNT_VERSIONS.contains(&version) {
            return Err(SfntError::NotSfnt);
        }
        let count = reader.u16(offset + 4)?;
        if count > MAX_TABLES {
            return Err(SfntError::TableCount(count));
        }
        let mut tables = BTreeMap::new();
        let mut total: u64 = 0;
        for i in 0..u64::from(count) {
            let record = offset + 12 + i * 16;
            let table = Table {
                offset: u64::from(reader.u32(record + 8)?),
                length: u64::from(reader.u32(record + 12)?),
            };
            let tag = reader.tag(record)?;
            reader.check(table.offset, table.length, Region::Table(tag))?;
            total += table.length;
            if total > reader.len() {
                return Err(SfntError::TablesOverlap);
            }
            tables.insert(tag, table);
        }
        Ok(Directory { tables })
    }

    pub(super) fn get(&self, tag: &[u8; 4]) -> Option<Table> {
        self.tables.get(&TableTag(*tag)).copied()
    }

    /// Every table, by tag.
    pub(super) fn iter(&self) -> impl Iterator<Item = (TableTag, Table)> + '_ {
        self.tables.iter().map(|(tag, table)| (*tag, *table))
    }

    pub(super) fn has(&self, tag: &[u8; 4]) -> bool {
        self.tables.contains_key(&TableTag(*tag))
    }

    /// A table the face cannot do without, at least `min_length` bytes long.
    pub(super) fn required(&self, tag: &[u8; 4], min_length: u64) -> Result<Table, SfntError> {
        let table = self
            .get(tag)
            .ok_or(SfntError::MissingTable(TableTag(*tag)))?;
        if table.length < min_length {
            return Err(SfntError::TruncatedTable(TableTag(*tag)));
        }
        Ok(table)
    }
}
