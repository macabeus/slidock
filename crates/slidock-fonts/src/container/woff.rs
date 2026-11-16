//! WOFF 1.0 (W3C Recommendation, 2012): a header, a table directory, and each table stored as is or zlib
//! compressed. Extended metadata and private data are skipped.

use miniz_oxide::inflate::TINFLStatus;
use miniz_oxide::inflate::core::{DecompressorOxide, decompress, inflate_flags};

use super::assemble::{self, Table};
use super::cursor::Cursor;
use super::{
    ContainerError, ContainerLimits, ContainerWarning, Format, Part, check_expansion,
    is_sfnt_version,
};
use crate::sfnt::TableTag;

const HEADER: usize = 44;
const ENTRY: usize = 20;

/// Why a WOFF file can't be read.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum WoffError {
    #[error("The WOFF header's length {declared} is not the file's {actual} bytes")]
    Length { declared: u32, actual: usize },
    #[error("The WOFF header's reserved field is not zero")]
    Reserved,
    #[error("WOFF cannot hold a font collection")]
    Collection,
    #[error("The {0} table is not 4-byte aligned, overlaps another or is outside the file")]
    Placement(TableTag),
    #[error("The {0} table's compressed length is more than its length")]
    CompLength(TableTag),
    #[error("The {0} table's zlib data is invalid or not its declared length")]
    Zlib(TableTag),
}

/// A decoded WOFF.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WoffFont {
    pub flavor: u32,
    pub sfnt: Vec<u8>,
    pub warnings: Vec<ContainerWarning>,
}

struct Entry {
    tag: [u8; 4],
    offset: u32,
    comp_length: u32,
    orig_length: u32,
    orig_checksum: u32,
}

/// A zlib stream inflated to exactly `length` bytes, its Adler-32 checked.
fn inflate_exact(data: &[u8], length: usize) -> Option<Vec<u8>> {
    let mut out = vec![0; length];
    let mut state = Box::<DecompressorOxide>::default();
    let flags = inflate_flags::TINFL_FLAG_PARSE_ZLIB_HEADER
        | inflate_flags::TINFL_FLAG_USING_NON_WRAPPING_OUTPUT_BUF;
    let (status, _, written) = decompress(&mut state, data, &mut out, 0, flags);
    (status == TINFLStatus::Done && written == length).then_some(out)
}

/// The sfnt inside a WOFF file.
pub fn decode(bytes: &[u8], limits: &ContainerLimits) -> Result<WoffFont, ContainerError> {
    let mut header = Cursor::new(bytes, Part::Header);
    header.tag()?;
    let flavor = header.u32_be()?;
    let length = header.u32_be()?;
    let num_tables = header.u16_be()?;
    let reserved = header.u16_be()?;
    header.skip(HEADER - 16)?;
    if length as usize != bytes.len() {
        return Err(WoffError::Length {
            declared: length,
            actual: bytes.len(),
        }
        .into());
    }
    if reserved != 0 {
        return Err(WoffError::Reserved.into());
    }
    if flavor == u32::from_be_bytes(*b"ttcf") {
        return Err(WoffError::Collection.into());
    }
    if !is_sfnt_version(flavor) {
        return Err(ContainerError::NotSfnt(Format::Woff));
    }
    if num_tables == 0 || num_tables > limits.max_tables {
        return Err(ContainerError::TableCount {
            count: num_tables.into(),
            limit: limits.max_tables,
        });
    }

    let mut directory = Cursor::new(
        bytes.get(HEADER..).unwrap_or_default(),
        Part::TableDirectory,
    );
    let mut entries = Vec::with_capacity(usize::from(num_tables));
    for _ in 0..num_tables {
        entries.push(Entry {
            tag: directory.tag()?,
            offset: directory.u32_be()?,
            comp_length: directory.u32_be()?,
            orig_length: directory.u32_be()?,
            orig_checksum: directory.u32_be()?,
        });
    }
    let directory_end = (HEADER + ENTRY * entries.len()) as u64;

    let total = entries.iter().fold(0u64, |sum, entry| {
        sum.saturating_add(entry.orig_length.into())
    });
    check_expansion(total, bytes.len(), limits)?;

    // Table data lies after the directory, 4-byte aligned, without overlaps.
    let mut placed: Vec<(u64, u64, [u8; 4])> = entries
        .iter()
        .map(|entry| {
            let start = u64::from(entry.offset);
            (start, start + u64::from(entry.comp_length), entry.tag)
        })
        .collect();
    placed.sort_unstable();
    let mut end_of_previous = directory_end;
    for (start, end, tag) in placed {
        if start % 4 != 0 || start < end_of_previous || end > bytes.len() as u64 {
            return Err(WoffError::Placement(TableTag(tag)).into());
        }
        end_of_previous = end;
    }

    let mut tables = Vec::with_capacity(entries.len());
    let mut warnings = Vec::new();
    for entry in entries {
        let tag = TableTag(entry.tag);
        let start = entry.offset as usize;
        let stored = bytes
            .get(start..start + entry.comp_length as usize)
            .ok_or(WoffError::Placement(tag))?;
        let data = match entry.comp_length.cmp(&entry.orig_length) {
            std::cmp::Ordering::Equal => stored.to_vec(),
            std::cmp::Ordering::Less => {
                inflate_exact(stored, entry.orig_length as usize).ok_or(WoffError::Zlib(tag))?
            }
            std::cmp::Ordering::Greater => return Err(WoffError::CompLength(tag).into()),
        };
        if assemble::table_checksum(entry.tag, &data) != entry.orig_checksum {
            warnings.push(ContainerWarning::ChecksumMismatch(tag));
        }
        tables.push(Table {
            tag: entry.tag,
            data,
        });
    }
    let sfnt = assemble::font(flavor, &tables, limits)?;
    Ok(WoffFont {
        flavor,
        sfnt,
        warnings,
    })
}
