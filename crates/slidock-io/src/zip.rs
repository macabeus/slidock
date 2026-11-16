//! A zip read by byte range. A STORED entry is a slice of the file, so media streams from it; a DEFLATE entry is
//! inflated whole, after its declared sizes pass every limit. Every entry read into memory, stored or inflated, draws
//! on one budget, and overlapping entries are refused, so each region is read as one part. ZIP64 is read: a
//! presentation past 4 GiB, or with its directory past that offset, has its sizes and offsets only there.

use std::borrow::Cow;
use std::cell::Cell;

use indexmap::IndexMap;
use thiserror::Error;

use crate::inflate::inflate_raw_exact;
use crate::range::{BytesReader, RangeError, RangeReader, SubReader};
use crate::text::decode_utf8;

/// Compression method 0.
pub const STORED: u16 = 0;
/// Compression method 8.
pub const DEFLATE: u16 = 8;
/// DEFLATE expands at most this much (258-byte matches coded in 2 bits); a higher declared ratio is a lie.
pub const MAX_DEFLATE_RATIO: u64 = 1032;

const EOCD_SIGNATURE: u32 = 0x0605_4b50;
const ZIP64_LOCATOR_SIGNATURE: u32 = 0x0706_4b50;
const ZIP64_EOCD_SIGNATURE: u32 = 0x0606_4b50;
const CENTRAL_HEADER_SIGNATURE: u32 = 0x0201_4b50;
const LOCAL_HEADER_SIGNATURE: u32 = 0x0403_4b50;
const EOCD_SIZE: u64 = 22;
const ZIP64_LOCATOR_SIZE: u64 = 20;
const ZIP64_EOCD_SIZE: u64 = 56;
const CENTRAL_HEADER_SIZE: usize = 46;
/// A local file header without its name and extra field.
pub const LOCAL_HEADER_SIZE: u64 = 30;
const MAX_COMMENT: u64 = 0xffff;
const ZIP64_EXTRA: u16 = 0x0001;
const U16_MAX: u16 = 0xffff;
const U32_MAX: u64 = 0xffff_ffff;
/// The largest offset or size a JavaScript number holds exactly; larger ZIP64 values are refused.
const MAX_SAFE_INTEGER: u64 = (1 << 53) - 1;
/// `read_many` reads entries this close together as one range.
const MAX_SPAN_GAP: i64 = 64 * 1024;
const MAX_SPAN_BYTES: i64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ZipEntry {
    /// The path inside the zip, `/`-separated, as written.
    pub name: String,
    /// [`STORED`], [`DEFLATE`] or another method, which reading refuses.
    pub method: u16,
    pub encrypted: bool,
    pub compressed_size: u64,
    /// Uncompressed.
    pub size: u64,
    pub local_header_offset: u64,
    /// The name and extra field lengths in the directory, which a local header normally repeats.
    pub header_tail_length: u64,
}

impl ZipEntry {
    /// Where the entry's data ends, if its local header repeats the directory's name and extra field.
    fn estimated_end(&self) -> u64 {
        self.local_header_offset
            .saturating_add(LOCAL_HEADER_SIZE + self.header_tail_length)
            .saturating_add(self.compressed_size)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ZipLimits {
    /// The central directory's size.
    pub directory_bytes: u64,
    pub entries: u64,
    /// One entry inflated or read into memory, by its declared size.
    pub entry_bytes: u64,
    /// Every entry this archive inflates or reads into memory, together.
    pub memory_bytes: u64,
}

const MIB: u64 = 1024 * 1024;

impl Default for ZipLimits {
    fn default() -> Self {
        Self {
            directory_bytes: 64 * MIB,
            entries: 20_000,
            entry_bytes: 64 * MIB,
            memory_bytes: 512 * MIB,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ZipErrorReason {
    NotAZip,
    Corrupt,
    Unsupported,
    DuplicateName,
    OverlappingEntries,
    TooManyEntries,
    EntryTooLarge,
    BudgetExceeded,
    /// The source failed to read a range inside it.
    Read,
}

impl ZipErrorReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            ZipErrorReason::NotAZip => "not-a-zip",
            ZipErrorReason::Corrupt => "corrupt",
            ZipErrorReason::Unsupported => "unsupported",
            ZipErrorReason::DuplicateName => "duplicate-name",
            ZipErrorReason::OverlappingEntries => "overlapping-entries",
            ZipErrorReason::TooManyEntries => "too-many-entries",
            ZipErrorReason::EntryTooLarge => "entry-too-large",
            ZipErrorReason::BudgetExceeded => "budget-exceeded",
            ZipErrorReason::Read => "read",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{message}")]
pub struct ZipError {
    pub reason: ZipErrorReason,
    pub message: String,
    /// The entry the error is about, when it is about one.
    pub entry: Option<String>,
}

impl ZipError {
    fn new(reason: ZipErrorReason, message: impl Into<String>) -> Self {
        Self {
            reason,
            message: message.into(),
            entry: None,
        }
    }

    fn of(reason: ZipErrorReason, message: impl Into<String>, entry: &ZipEntry) -> Self {
        Self {
            reason,
            message: message.into(),
            entry: Some(entry.name.clone()),
        }
    }
}

impl From<RangeError> for ZipError {
    fn from(error: RangeError) -> Self {
        ZipError::new(ZipErrorReason::Read, error.to_string())
    }
}

fn corrupt(message: impl Into<String>) -> ZipError {
    ZipError::new(ZipErrorReason::Corrupt, message)
}

fn corrupt_entry(message: impl Into<String>, entry: &ZipEntry) -> ZipError {
    ZipError::of(ZipErrorReason::Corrupt, message, entry)
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    let b = bytes.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*b.first()?, *b.get(1)?]))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at.checked_add(4)?)?;
    Some(u32::from_le_bytes(b.try_into().ok()?))
}

fn le64(bytes: &[u8], at: usize) -> Option<u64> {
    let b = bytes.get(at..at.checked_add(8)?)?;
    Some(u64::from_le_bytes(b.try_into().ok()?))
}

/// A read length as `usize`; a range that does not fit is past the end of any source.
fn length(len: u64) -> Result<usize, ZipError> {
    usize::try_from(len).map_err(|_| corrupt(format!("A zip range of {len} bytes is too long")))
}

/// A local file header or an empty archive's end record: what a zip starts with.
pub fn is_zip_head(head: &[u8]) -> bool {
    matches!(head, [0x50, 0x4b, 3 | 5, _, ..])
}

/// A zip whose central directory has been read: its entries, and reads of them under its limits.
#[derive(Debug)]
pub struct ZipArchive<R> {
    reader: R,
    limits: ZipLimits,
    entries: Vec<ZipEntry>,
    by_name: IndexMap<String, usize>,
    loaded: Cell<u64>,
}

/// An entry opened by [`ZipArchive::open_entry`]: a slice of the file when STORED, its inflated bytes when DEFLATE.
#[derive(Debug)]
pub enum EntryReader<'a, R> {
    Slice(SubReader<&'a R>),
    Memory(BytesReader<Cow<'a, [u8]>>),
}

impl<R: RangeReader> RangeReader for EntryReader<'_, R> {
    fn len(&self) -> u64 {
        match self {
            EntryReader::Slice(reader) => reader.len(),
            EntryReader::Memory(reader) => reader.len(),
        }
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        match self {
            EntryReader::Slice(reader) => reader.read_at(offset, len),
            EntryReader::Memory(reader) => reader.read_at(offset, len),
        }
    }
}

impl<R: RangeReader> ZipArchive<R> {
    /// Reads the central directory; every limit on it is checked before the directory is read.
    pub fn open(reader: R, limits: ZipLimits) -> Result<Self, ZipError> {
        let entries = read_directory(&reader, &limits)?;
        let mut by_name = IndexMap::with_capacity(entries.len());
        for (index, entry) in entries.iter().enumerate() {
            if by_name.insert(entry.name.clone(), index).is_some() {
                return Err(ZipError::of(
                    ZipErrorReason::DuplicateName,
                    format!("{} appears twice", entry.name),
                    entry,
                ));
            }
        }
        check_overlaps(&entries)?;
        Ok(Self {
            reader,
            limits,
            entries,
            by_name,
            loaded: Cell::new(0),
        })
    }

    /// Directory order.
    pub fn entries(&self) -> &[ZipEntry] {
        &self.entries
    }

    pub fn get(&self, name: &str) -> Option<&ZipEntry> {
        self.by_name
            .get(name)
            .and_then(|&index| self.entries.get(index))
    }

    /// Bytes inflated or read into memory so far, against the archive's budget.
    pub fn memory_bytes(&self) -> u64 {
        self.loaded.get()
    }

    pub fn limits(&self) -> &ZipLimits {
        &self.limits
    }

    pub fn reader(&self) -> &R {
        &self.reader
    }

    /// An entry's bytes as a reader: a slice of the file when STORED, inflated into memory when DEFLATE.
    pub fn open_entry(&self, entry: &ZipEntry) -> Result<EntryReader<'_, R>, ZipError> {
        if entry.method == STORED {
            check_entry(entry, &self.limits)?;
            let offset = self.data_offset(entry)?;
            let slice = SubReader::new(&self.reader, offset, entry.size)
                .map_err(|_| corrupt_entry(past_end(entry), entry))?;
            return Ok(EntryReader::Slice(slice));
        }
        Ok(EntryReader::Memory(BytesReader::new(self.read(entry)?)))
    }

    /// An entry's bytes in memory, at most `entry_bytes`.
    pub fn read(&self, entry: &ZipEntry) -> Result<Cow<'_, [u8]>, ZipError> {
        self.check_entry_bytes(entry)?;
        self.reserve(entry)?;
        let offset = self.data_offset(entry)?;
        let data = self
            .reader
            .read_at(offset, length(entry.compressed_size)?)?;
        self.decode(entry, data)
    }

    /// Several entries' bytes, in file order; entries that sit together in the file are read as one range.
    pub fn read_many(
        &self,
        wanted: &[&ZipEntry],
    ) -> Result<IndexMap<String, Cow<'_, [u8]>>, ZipError> {
        let mut declared: u64 = 0;
        for entry in wanted {
            self.check_entry_bytes(entry)?;
            self.reserve(entry)?;
            declared = declared.saturating_add(entry.size);
        }
        let memory = self.limits.memory_bytes;
        if self.loaded.get().saturating_add(declared) > memory {
            return Err(ZipError::new(
                ZipErrorReason::BudgetExceeded,
                format!("These entries would bring the bytes in memory past {memory}"),
            ));
        }
        let mut sorted: Vec<&ZipEntry> = wanted.to_vec();
        sorted.sort_by_key(|entry| entry.local_header_offset);
        let mut result = IndexMap::with_capacity(sorted.len());
        let mut rest = sorted.as_slice();
        while let Some(first) = rest.first() {
            let span_entries = span(rest);
            rest = rest.get(span_entries.len()..).unwrap_or_default();
            let start = first.local_header_offset;
            let end = span_entries
                .last()
                .map_or(start, |last| last.estimated_end())
                .min(self.reader.len());
            if start > end {
                return Err(corrupt_entry(past_end(first), first));
            }
            let bytes = self.reader.read_at(start, length(end - start)?)?;
            for entry in span_entries {
                let data = match self.slice_of_span(&bytes, start, entry)? {
                    Some(data) => data,
                    // A local header with a longer extra field than the directory's: read the entry on its own.
                    None => {
                        let offset = self.data_offset(entry)?;
                        self.reader
                            .read_at(offset, length(entry.compressed_size)?)?
                    }
                };
                let decoded = self.decode(entry, data)?;
                result.insert(entry.name.clone(), decoded);
            }
        }
        Ok(result)
    }

    /// An entry's compressed bytes inside a span read from `start`, when the span holds all of them.
    fn slice_of_span<'s>(
        &self,
        span: &Cow<'s, [u8]>,
        start: u64,
        entry: &ZipEntry,
    ) -> Result<Option<Cow<'s, [u8]>>, ZipError> {
        let at = entry.local_header_offset - start;
        let span_len = span.len() as u64;
        if at + LOCAL_HEADER_SIZE > span_len {
            return Ok(None);
        }
        let header = span.get(length(at)?..).unwrap_or_default();
        let offset = at + data_start(entry, header)?;
        let end = offset.saturating_add(entry.compressed_size);
        if end > span_len {
            return Ok(None);
        }
        // Inside the span, so both ends fit in usize.
        let range = offset as usize..end as usize;
        Ok(match span {
            Cow::Borrowed(bytes) => bytes.get(range).map(Cow::Borrowed),
            Cow::Owned(bytes) => bytes.get(range).map(|data| Cow::Owned(data.to_vec())),
        })
    }

    fn check_entry_bytes(&self, entry: &ZipEntry) -> Result<(), ZipError> {
        if entry.size > self.limits.entry_bytes {
            return Err(ZipError::of(
                ZipErrorReason::EntryTooLarge,
                format!("{} is {} bytes", entry.name, entry.size),
                entry,
            ));
        }
        Ok(())
    }

    fn reserve(&self, entry: &ZipEntry) -> Result<(), ZipError> {
        check_entry(entry, &self.limits)?;
        let memory = self.limits.memory_bytes;
        if self.loaded.get().saturating_add(entry.size) > memory {
            return Err(ZipError::of(
                ZipErrorReason::BudgetExceeded,
                format!(
                    "{} would bring the archive's bytes in memory past {memory}",
                    entry.name
                ),
                entry,
            ));
        }
        Ok(())
    }

    fn decode<'a>(&self, entry: &ZipEntry, data: Cow<'a, [u8]>) -> Result<Cow<'a, [u8]>, ZipError> {
        if entry.method == STORED {
            self.loaded.set(self.loaded.get() + data.len() as u64);
            return Ok(data);
        }
        let expected = usize::try_from(entry.size).map_err(|_| {
            ZipError::of(
                ZipErrorReason::EntryTooLarge,
                format!("{} is {} bytes", entry.name, entry.size),
                entry,
            )
        })?;
        let out = inflate_raw_exact(&data, expected).map_err(|error| {
            corrupt_entry(
                format!(
                    "{} does not inflate to {} bytes: {error}",
                    entry.name, entry.size
                ),
                entry,
            )
        })?;
        self.loaded.set(self.loaded.get() + out.len() as u64);
        Ok(Cow::Owned(out))
    }

    /// Where an entry's data starts in the file, compressed or not.
    fn data_offset(&self, entry: &ZipEntry) -> Result<u64, ZipError> {
        let size = self.reader.len();
        if entry.local_header_offset.saturating_add(LOCAL_HEADER_SIZE) > size {
            return Err(corrupt_entry(past_end(entry), entry));
        }
        let header = self
            .reader
            .read_at(entry.local_header_offset, LOCAL_HEADER_SIZE as usize)?;
        let offset = entry.local_header_offset + data_start(entry, &header)?;
        if offset.saturating_add(entry.compressed_size) > size {
            return Err(corrupt_entry(past_end(entry), entry));
        }
        Ok(offset)
    }
}

/// The leading entries of `sorted` that one read covers: each starts close to where the one before ends.
fn span<'e, 'a>(sorted: &'e [&'a ZipEntry]) -> &'e [&'a ZipEntry] {
    let Some(first) = sorted.first() else {
        return sorted;
    };
    let start = first.local_header_offset as i64;
    let mut count = 1;
    for pair in sorted.windows(2) {
        let [previous, next] = pair else { break };
        let gap = next.local_header_offset as i64 - previous.estimated_end() as i64;
        let reach = next.estimated_end() as i64 - start;
        if gap > MAX_SPAN_GAP || reach > MAX_SPAN_BYTES {
            break;
        }
        count += 1;
    }
    sorted.get(..count).unwrap_or(sorted)
}

fn past_end(entry: &ZipEntry) -> String {
    format!("{} is past the end of the file", entry.name)
}

/// The offset of an entry's data from its local header.
fn data_start(entry: &ZipEntry, header: &[u8]) -> Result<u64, ZipError> {
    if le32(header, 0) != Some(LOCAL_HEADER_SIGNATURE) {
        return Err(corrupt_entry(
            format!("{} has no local header", entry.name),
            entry,
        ));
    }
    let name = le16(header, 26).unwrap_or(0);
    let extra = le16(header, 28).unwrap_or(0);
    Ok(LOCAL_HEADER_SIZE + u64::from(name) + u64::from(extra))
}

/// No two entries share bytes: one region read as many parts would pass every per-entry limit many times. An entry's
/// data starts at least a local header past its offset, and its compressed bytes follow.
fn check_overlaps(entries: &[ZipEntry]) -> Result<(), ZipError> {
    let mut sorted: Vec<&ZipEntry> = entries.iter().collect();
    sorted.sort_by_key(|entry| entry.local_header_offset);
    for pair in sorted.windows(2) {
        let [previous, entry] = pair else { continue };
        let previous_end = previous
            .local_header_offset
            .saturating_add(LOCAL_HEADER_SIZE)
            .saturating_add(previous.compressed_size);
        if entry.local_header_offset < previous_end {
            return Err(ZipError::of(
                ZipErrorReason::OverlappingEntries,
                format!("{} overlaps {}", entry.name, previous.name),
                entry,
            ));
        }
    }
    Ok(())
}

/// What the directory says about an entry, checked before any of its bytes are read.
fn check_entry(entry: &ZipEntry, limits: &ZipLimits) -> Result<(), ZipError> {
    let name = &entry.name;
    if entry.encrypted {
        return Err(ZipError::of(
            ZipErrorReason::Unsupported,
            format!("{name} is encrypted"),
            entry,
        ));
    }
    if entry.method == STORED {
        if entry.compressed_size != entry.size {
            return Err(corrupt_entry(
                format!("{name} is stored with two different sizes"),
                entry,
            ));
        }
        return Ok(());
    }
    if entry.method != DEFLATE {
        return Err(ZipError::of(
            ZipErrorReason::Unsupported,
            format!("{name} uses compression method {}", entry.method),
            entry,
        ));
    }
    if entry.size > limits.entry_bytes {
        return Err(ZipError::of(
            ZipErrorReason::EntryTooLarge,
            format!(
                "{name} inflates to {} bytes; at most {} are inflated",
                entry.size, limits.entry_bytes
            ),
            entry,
        ));
    }
    if entry.size > entry.compressed_size.saturating_mul(MAX_DEFLATE_RATIO) {
        return Err(corrupt_entry(
            format!("{name} declares more than DEFLATE's {MAX_DEFLATE_RATIO}:1 ratio"),
            entry,
        ));
    }
    if entry.compressed_size > deflate_bound(entry.size) {
        return Err(corrupt_entry(
            format!(
                "{name} is {} bytes compressed, more than DEFLATE writes",
                entry.compressed_size
            ),
            entry,
        ));
    }
    Ok(())
}

/// The most DEFLATE writes for `size` bytes, stored blocks included (zlib's `compressBound`).
fn deflate_bound(size: u64) -> u64 {
    size + size / 4096 + size / 16384 + size / 33_554_432 + 13
}

fn safe_number(value: u64) -> Result<u64, ZipError> {
    if value > MAX_SAFE_INTEGER {
        return Err(corrupt("A zip offset is out of range"));
    }
    Ok(value)
}

struct Directory {
    count: u64,
    size: u64,
    offset: u64,
}

fn read_directory<R: RangeReader>(
    reader: &R,
    limits: &ZipLimits,
) -> Result<Vec<ZipEntry>, ZipError> {
    let size = reader.len();
    if size < EOCD_SIZE {
        return Err(ZipError::new(
            ZipErrorReason::NotAZip,
            "The file is too small to be a zip",
        ));
    }
    let tail_length = size.min(EOCD_SIZE + MAX_COMMENT);
    let tail_offset = size - tail_length;
    let tail = reader.read_at(tail_offset, length(tail_length)?)?;

    // The comment length must reach the end of the file exactly, so a signature inside a comment is passed over.
    let last = tail.len().saturating_sub(EOCD_SIZE as usize);
    let eocd = (0..=last).rev().find(|&at| {
        le32(&tail, at) == Some(EOCD_SIGNATURE)
            && le16(&tail, at + 20)
                .is_some_and(|comment| at + EOCD_SIZE as usize + usize::from(comment) == tail.len())
    });
    let Some(eocd) = eocd else {
        return Err(ZipError::new(
            ZipErrorReason::NotAZip,
            "The file has no zip directory",
        ));
    };

    let field = |at: usize| le32(&tail, eocd + at).map_or(0, u64::from);
    let count = le16(&tail, eocd + 10).unwrap_or(0);
    let mut directory = Directory {
        count: u64::from(count),
        size: field(12),
        offset: field(16),
    };
    if count == U16_MAX || directory.size == U32_MAX || directory.offset == U32_MAX {
        directory = read_zip64_end(reader, tail_offset + eocd as u64)?;
    }
    if directory.count > limits.entries {
        return Err(ZipError::new(
            ZipErrorReason::TooManyEntries,
            format!("The zip has {} entries", directory.count),
        ));
    }
    if directory.size > limits.directory_bytes {
        return Err(corrupt(format!(
            "The zip directory is {} bytes",
            directory.size
        )));
    }
    if directory.offset.saturating_add(directory.size) > size {
        return Err(corrupt("The zip directory is past the end of the file"));
    }

    let bytes = reader.read_at(directory.offset, length(directory.size)?)?;
    let mut entries = Vec::new();
    let mut at = 0;
    for i in 0..directory.count {
        if le32(&bytes, at) != Some(CENTRAL_HEADER_SIGNATURE)
            || at + CENTRAL_HEADER_SIZE > bytes.len()
        {
            return Err(corrupt(format!("Zip directory entry {i} is damaged")));
        }
        let u16_at = |offset: usize| usize::from(le16(&bytes, at + offset).unwrap_or(0));
        let u32_at = |offset: usize| le32(&bytes, at + offset).map_or(0, u64::from);
        let flags = u16_at(8);
        let name_length = u16_at(28);
        let extra_length = u16_at(30);
        let comment_length = u16_at(32);
        let name_start = at + CENTRAL_HEADER_SIZE;
        let extra_start = name_start + name_length;
        let end = extra_start + extra_length + comment_length;
        let (Some(name), Some(extra)) = (
            bytes.get(name_start..extra_start),
            bytes.get(extra_start..extra_start + extra_length),
        ) else {
            return Err(corrupt(format!("Zip directory entry {i} is truncated")));
        };
        if end > bytes.len() {
            return Err(corrupt(format!("Zip directory entry {i} is truncated")));
        }
        let sizes = zip64_sizes(
            extra,
            Sizes {
                size: u32_at(24),
                compressed_size: u32_at(20),
                local_header_offset: u32_at(42),
            },
        )?;
        entries.push(ZipEntry {
            name: decode_utf8(name).into_owned(),
            method: le16(&bytes, at + 10).unwrap_or(0),
            encrypted: flags & 1 != 0,
            compressed_size: sizes.compressed_size,
            size: sizes.size,
            local_header_offset: sizes.local_header_offset,
            header_tail_length: (name_length + extra_length) as u64,
        });
        at = end;
    }
    Ok(entries)
}

fn read_zip64_end<R: RangeReader>(reader: &R, eocd_offset: u64) -> Result<Directory, ZipError> {
    if eocd_offset < ZIP64_LOCATOR_SIZE {
        return Err(corrupt("The ZIP64 locator is missing"));
    }
    let locator = reader.read_at(
        eocd_offset - ZIP64_LOCATOR_SIZE,
        ZIP64_LOCATOR_SIZE as usize,
    )?;
    if le32(&locator, 0) != Some(ZIP64_LOCATOR_SIGNATURE) {
        return Err(corrupt("The ZIP64 locator is missing"));
    }
    let end_offset = safe_number(le64(&locator, 8).unwrap_or(0))?;
    if end_offset + ZIP64_EOCD_SIZE > reader.len() {
        return Err(corrupt(
            "The ZIP64 directory end is past the end of the file",
        ));
    }
    let end = reader.read_at(end_offset, ZIP64_EOCD_SIZE as usize)?;
    if le32(&end, 0) != Some(ZIP64_EOCD_SIGNATURE) {
        return Err(corrupt("The ZIP64 directory end is missing"));
    }
    let field = |at: usize| safe_number(le64(&end, at).unwrap_or(0));
    Ok(Directory {
        count: field(32)?,
        size: field(40)?,
        offset: field(48)?,
    })
}

#[derive(Clone, Copy)]
struct Sizes {
    size: u64,
    compressed_size: u64,
    local_header_offset: u64,
}

/// The ZIP64 extra field holds, in this order, each of these that is 0xFFFFFFFF in the header.
fn zip64_sizes(extra: &[u8], header: Sizes) -> Result<Sizes, ZipError> {
    if header.size != U32_MAX
        && header.compressed_size != U32_MAX
        && header.local_header_offset != U32_MAX
    {
        return Ok(header);
    }
    let mut at = 0;
    while at + 4 <= extra.len() {
        let id = le16(extra, at).unwrap_or(0);
        let field_length = usize::from(le16(extra, at + 2).unwrap_or(0));
        if id == ZIP64_EXTRA {
            let limit = (at + 4 + field_length).min(extra.len());
            let mut field = at + 4;
            let mut next = |value: u64| -> Result<u64, ZipError> {
                if value != U32_MAX {
                    return Ok(value);
                }
                if field + 8 > limit {
                    return Err(corrupt("A ZIP64 extra field is truncated"));
                }
                let value = safe_number(le64(extra, field).unwrap_or(0))?;
                field += 8;
                Ok(value)
            };
            return Ok(Sizes {
                size: next(header.size)?,
                compressed_size: next(header.compressed_size)?,
                local_header_offset: next(header.local_header_offset)?,
            });
        }
        at += 4 + field_length;
    }
    Err(corrupt("An entry needs a ZIP64 extra field and has none"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_what_deflate_writes() {
        assert_eq!(deflate_bound(0), 13);
        assert_eq!(deflate_bound(16384), 16384 + 4 + 1 + 13);
    }

    #[test]
    fn recognises_a_zip_by_its_first_bytes() {
        assert!(is_zip_head(&[0x50, 0x4b, 3, 4]));
        assert!(is_zip_head(&[0x50, 0x4b, 5, 6]));
        assert!(!is_zip_head(&[0x50, 0x4b, 3]));
        assert!(!is_zip_head(b"%PDF-1.7"));
    }
}
