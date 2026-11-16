//! Bytes arrive only through a [`RangeReader`], so a frontend reads just the ranges it asks for: a Keynote
//! presentation can be a gigabyte, and its index is about a megabyte. Reads are synchronous; in the browser the
//! compiler runs in a worker over `FileReaderSync`, natively over positioned file reads.

use std::borrow::Cow;
use std::cell::Cell;

use slidock_canon::Sha256Hex;
use thiserror::Error;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum RangeError {
    #[error("Range {offset}+{len} is past the end ({size} bytes)")]
    PastEnd { offset: u64, len: u64, size: u64 },
    #[error("Invalid range {offset}+{len}")]
    Invalid { offset: u64, len: u64 },
    /// The host failed to read a range inside the source.
    #[error("Reading {offset}+{len} failed: {message}")]
    Io {
        offset: u64,
        len: u64,
        message: String,
    },
}

/// A source read by byte range.
pub trait RangeReader {
    /// The source's size in bytes.
    fn len(&self) -> u64;

    /// `len` bytes from `offset`; [`RangeError`] when the range is outside the source.
    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError>;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl<R: RangeReader + ?Sized> RangeReader for &R {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        (**self).read_at(offset, len)
    }
}

impl<R: RangeReader + ?Sized> RangeReader for Box<R> {
    fn len(&self) -> u64 {
        (**self).len()
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        (**self).read_at(offset, len)
    }
}

/// `offset..offset + len` as a range of a source of `size` bytes.
pub fn check_range(size: u64, offset: u64, len: u64) -> Result<std::ops::Range<u64>, RangeError> {
    let end = offset
        .checked_add(len)
        .ok_or(RangeError::Invalid { offset, len })?;
    if end > size {
        return Err(RangeError::PastEnd { offset, len, size });
    }
    Ok(offset..end)
}

/// Bytes in memory as a source; reads borrow them.
#[derive(Clone, Debug, Default)]
pub struct BytesReader<T>(pub T);

impl<T: AsRef<[u8]>> BytesReader<T> {
    pub fn new(bytes: T) -> Self {
        Self(bytes)
    }

    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T: AsRef<[u8]>> RangeReader for BytesReader<T> {
    fn len(&self) -> u64 {
        self.0.as_ref().len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        let bytes = self.0.as_ref();
        let range = check_range(bytes.len() as u64, offset, len as u64)?;
        // In range of an in-memory slice, so both ends fit in usize.
        let (start, end) = (range.start as usize, range.end as usize);
        bytes
            .get(start..end)
            .map(Cow::Borrowed)
            .ok_or(RangeError::PastEnd {
                offset,
                len: len as u64,
                size: bytes.len() as u64,
            })
    }
}

/// `len` bytes of a source from `offset`, as a source of their own.
#[derive(Clone, Debug)]
pub struct SubReader<R> {
    inner: R,
    offset: u64,
    len: u64,
}

impl<R: RangeReader> SubReader<R> {
    pub fn new(inner: R, offset: u64, len: u64) -> Result<Self, RangeError> {
        check_range(inner.len(), offset, len)?;
        Ok(Self { inner, offset, len })
    }

    /// Where this range starts in the inner source.
    pub fn offset(&self) -> u64 {
        self.offset
    }
}

impl<R: RangeReader> RangeReader for SubReader<R> {
    fn len(&self) -> u64 {
        self.len
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        check_range(self.len, offset, len as u64)?;
        self.inner.read_at(self.offset + offset, len)
    }
}

/// A source that counts the bytes its reads return.
#[derive(Debug)]
pub struct CountingReader<R> {
    inner: R,
    read: Cell<u64>,
}

impl<R: RangeReader> CountingReader<R> {
    pub fn new(inner: R) -> Self {
        Self {
            inner,
            read: Cell::new(0),
        }
    }

    pub fn bytes_read(&self) -> u64 {
        self.read.get()
    }
}

impl<R: RangeReader> RangeReader for CountingReader<R> {
    fn len(&self) -> u64 {
        self.inner.len()
    }

    fn read_at(&self, offset: u64, len: usize) -> Result<Cow<'_, [u8]>, RangeError> {
        let bytes = self.inner.read_at(offset, len)?;
        self.read.set(self.read.get() + bytes.len() as u64);
        Ok(bytes)
    }
}

/// The first `len` bytes, or the whole source when it is shorter.
pub fn read_head<R: RangeReader + ?Sized>(
    reader: &R,
    len: usize,
) -> Result<Cow<'_, [u8]>, RangeError> {
    let len = usize::try_from(reader.len()).map_or(len, |size| len.min(size));
    reader.read_at(0, len)
}

/// How much of a source [`sha256_range`] holds in memory at once.
pub const HASH_CHUNK: usize = 8 * 1024 * 1024;

/// The lowercase hex SHA-256 of `len` bytes of a source from `offset`, read [`HASH_CHUNK`] bytes at a time.
pub fn sha256_range<R: RangeReader + ?Sized>(
    reader: &R,
    offset: u64,
    len: u64,
) -> Result<String, RangeError> {
    check_range(reader.len(), offset, len)?;
    let mut hash = Sha256Hex::new();
    let mut at = 0;
    while at < len {
        let chunk = (len - at).min(HASH_CHUNK as u64);
        // At most HASH_CHUNK, so it fits in usize.
        hash.update(reader.read_at(offset + at, chunk as usize)?);
        at += chunk;
    }
    Ok(hash.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_and_refuses_ranges_outside_the_source() {
        let reader = BytesReader::new(b"0123456789");
        assert_eq!(&*reader.read_at(2, 3).unwrap(), b"234");
        assert_eq!(
            reader.read_at(8, 3),
            Err(RangeError::PastEnd {
                offset: 8,
                len: 3,
                size: 10
            })
        );
        assert!(matches!(
            reader.read_at(u64::MAX, 1),
            Err(RangeError::Invalid { .. })
        ));
        assert!(matches!(reader.read_at(0, 0), Ok(Cow::Borrowed(b""))));
    }

    #[test]
    fn offsets_a_sub_range_and_counts_what_is_read() {
        let counting = CountingReader::new(BytesReader::new(b"0123456789"));
        let sub = SubReader::new(&counting, 4, 4).unwrap();
        assert_eq!(&*sub.read_at(1, 2).unwrap(), b"56");
        assert!(matches!(sub.read_at(3, 2), Err(RangeError::PastEnd { .. })));
        assert_eq!(counting.bytes_read(), 2);
        assert!(SubReader::new(&counting, 8, 4).is_err());
    }

    #[test]
    fn reads_a_head_no_longer_than_the_source() {
        let reader = BytesReader::new(b"abc");
        assert_eq!(&*read_head(&reader, 4096).unwrap(), b"abc");
    }

    #[test]
    fn hashes_bytes_and_ranges_alike() {
        let bytes = b"abc".repeat(1000);
        let reader = BytesReader::new(&bytes);
        let whole = "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad";
        assert_eq!(sha256_range(&reader, 3, 3).unwrap(), whole);
        assert_eq!(
            sha256_range(&reader, 0, 3000).unwrap(),
            slidock_canon::sha256_hex(&bytes)
        );
        assert!(sha256_range(&reader, 2999, 2).is_err());
    }
}
