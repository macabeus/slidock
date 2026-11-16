//! A deterministic zip writer that streams: no seeking, so every byte goes to the sink once, in order.
//!
//! - Every entry is dated 1980-01-01 00:00 (the earliest DOS date), made by "MS-DOS" with no attributes, so
//!   extractors give the files their default permissions, and carries no extra field but ZIP64's.
//! - Text is deflated by `miniz_oxide` at [`DEFLATE_LEVEL`], whole, with its sizes and CRC in the local header.
//! - Media are stored, streamed from the host: the local header announces a data descriptor (flag bit 3), which
//!   carries the CRC and sizes after the bytes; the central directory repeats them.
//! - An entry is at most 4 GiB - 2 bytes. Offsets, the directory and the entry count switch to ZIP64 records when
//!   they reach the 32-bit (16-bit for the count) limits, so a deck past 4 GiB still opens.

use crate::BundleError;
use crate::output::Chunks;

/// The `miniz_oxide` level text entries are deflated at (zlib's default).
pub const DEFLATE_LEVEL: u8 = 6;

const LOCAL_HEADER: u32 = 0x0403_4b50;
const DATA_DESCRIPTOR: u32 = 0x0807_4b50;
const CENTRAL_HEADER: u32 = 0x0201_4b50;
const ZIP64_END: u32 = 0x0606_4b50;
const ZIP64_LOCATOR: u32 = 0x0706_4b50;
const END: u32 = 0x0605_4b50;
const ZIP64_EXTRA: u16 = 0x0001;

const STORED: u16 = 0;
const DEFLATE: u16 = 8;
/// General purpose flag bit 3: the CRC and sizes follow the data.
const HAS_DESCRIPTOR: u16 = 0x0008;
/// Version 2.0: deflate and data descriptors.
const VERSION_20: u16 = 20;
/// Version 4.5: ZIP64.
const VERSION_45: u16 = 45;
/// 1980-01-01 in DOS date format; the time is 00:00:00.
const DOS_DATE: u16 = (1 << 5) | 1;
const DOS_TIME: u16 = 0;

const U16_LIMIT: u64 = 0xffff;
const U32_LIMIT: u64 = 0xffff_ffff;

/// The CRC-32 of ZIP (IEEE 802.3, reflected, polynomial 0xEDB88320).
#[derive(Clone, Copy, Debug)]
pub struct Crc32(u32);

/// The CRC of each byte value, then of each byte value followed by 1 to 7 zero bytes: eight bytes per step
/// ("slicing by 8").
#[allow(
    clippy::indexing_slicing,
    reason = "const tables built over their own indexes"
)]
const CRC_TABLES: [[u32; 256]; 8] = {
    let mut tables = [[0u32; 256]; 8];
    let mut i = 0;
    while i < 256 {
        let mut crc = i as u32;
        let mut bit = 0;
        while bit < 8 {
            crc = if crc & 1 == 1 {
                0xedb8_8320 ^ (crc >> 1)
            } else {
                crc >> 1
            };
            bit += 1;
        }
        tables[0][i] = crc;
        i += 1;
    }
    let mut k = 1;
    while k < 8 {
        let mut i = 0;
        while i < 256 {
            let previous = tables[k - 1][i];
            tables[k][i] = (previous >> 8) ^ tables[0][(previous & 0xff) as usize];
            i += 1;
        }
        k += 1;
    }
    tables
};

/// The entry of table `k` for the low byte of `value`.
#[allow(
    clippy::indexing_slicing,
    reason = "k < 8 at every call and a byte indexes a 256-entry table"
)]
fn crc_entry(k: usize, value: u32) -> u32 {
    CRC_TABLES[k][(value & 0xff) as usize]
}

impl Default for Crc32 {
    fn default() -> Self {
        Crc32(0xffff_ffff)
    }
}

impl Crc32 {
    pub fn update(&mut self, bytes: &[u8]) {
        let mut crc = self.0;
        let (octets, rest) = bytes.as_chunks::<8>();
        for &[a, b, c, d, e, f, g, h] in octets {
            let low = u32::from_le_bytes([a, b, c, d]) ^ crc;
            let high = u32::from_le_bytes([e, f, g, h]);
            crc = crc_entry(7, low)
                ^ crc_entry(6, low >> 8)
                ^ crc_entry(5, low >> 16)
                ^ crc_entry(4, low >> 24)
                ^ crc_entry(3, high)
                ^ crc_entry(2, high >> 8)
                ^ crc_entry(1, high >> 16)
                ^ crc_entry(0, high >> 24);
        }
        for &byte in rest {
            crc = crc_entry(0, crc ^ u32::from(byte)) ^ (crc >> 8);
        }
        self.0 = crc;
    }

    pub fn finish(self) -> u32 {
        !self.0
    }

    pub fn of(bytes: &[u8]) -> u32 {
        let mut crc = Crc32::default();
        crc.update(bytes);
        crc.finish()
    }
}

/// An entry as the central directory lists it.
#[derive(Clone, Debug)]
struct Entry {
    name: String,
    method: u16,
    flags: u16,
    crc: u32,
    compressed: u32,
    size: u32,
    offset: u64,
}

/// A stored entry whose local header is written and whose bytes are streaming.
#[derive(Debug)]
#[must_use = "a stored entry is listed once its descriptor is written"]
pub struct OpenEntry {
    name: String,
    offset: u64,
    data_start: u64,
}

/// The entries written so far and where the output stands.
#[derive(Debug)]
pub struct ZipWriter {
    entries: Vec<Entry>,
    /// Offsets and directory sizes from this value on are written as ZIP64 (`0xffffffff` and a 64-bit field).
    zip64_from: u64,
}

impl Default for ZipWriter {
    fn default() -> Self {
        ZipWriter {
            entries: Vec::new(),
            zip64_from: U32_LIMIT,
        }
    }
}

fn u16_of(value: usize, what: &str) -> Result<u16, BundleError> {
    u16::try_from(value).map_err(|_| BundleError::Zip(format!("{what} is too long for a zip")))
}

/// An entry size as its 32-bit field; `0xffffffff` would announce ZIP64, so it is refused too.
fn size_field(size: u64, name: &str) -> Result<u32, BundleError> {
    u32::try_from(size)
        .ok()
        .filter(|&size| u64::from(size) < U32_LIMIT)
        .ok_or_else(|| BundleError::Zip(format!("{name} is 4 GiB or more")))
}

struct Bytes(Vec<u8>);

impl Bytes {
    fn u16(&mut self, value: u16) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }
    fn u32(&mut self, value: u32) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }
    fn u64(&mut self, value: u64) -> &mut Self {
        self.0.extend_from_slice(&value.to_le_bytes());
        self
    }
    fn raw(&mut self, value: &[u8]) -> &mut Self {
        self.0.extend_from_slice(value);
        self
    }
}

impl ZipWriter {
    #[cfg(test)]
    fn with_zip64_from(zip64_from: u64) -> ZipWriter {
        ZipWriter {
            entries: Vec::new(),
            zip64_from,
        }
    }

    fn local_header(
        name: &str,
        method: u16,
        flags: u16,
        (crc, compressed, size): (u32, u32, u32),
    ) -> Result<Vec<u8>, BundleError> {
        let mut header = Bytes(Vec::with_capacity(30 + name.len()));
        header
            .u32(LOCAL_HEADER)
            .u16(VERSION_20)
            .u16(flags)
            .u16(method)
            .u16(DOS_TIME)
            .u16(DOS_DATE)
            .u32(crc)
            .u32(compressed)
            .u32(size)
            .u16(u16_of(name.len(), name)?)
            .u16(0)
            .raw(name.as_bytes());
        Ok(header.0)
    }

    /// Writes `data` as a deflated entry.
    pub fn deflated(
        &mut self,
        name: &str,
        data: &[u8],
        out: &mut Chunks<'_>,
    ) -> Result<(), BundleError> {
        let deflated = miniz_oxide::deflate::compress_to_vec(data, DEFLATE_LEVEL);
        let crc = Crc32::of(data);
        let size = size_field(data.len() as u64, name)?;
        let compressed = size_field(deflated.len() as u64, name)?;
        let offset = out.position();
        out.write(&Self::local_header(
            name,
            DEFLATE,
            0,
            (crc, compressed, size),
        )?)?;
        out.write(&deflated)?;
        self.entries.push(Entry {
            name: name.to_owned(),
            method: DEFLATE,
            flags: 0,
            crc,
            compressed,
            size,
            offset,
        });
        Ok(())
    }

    /// Writes the local header of a stored entry whose bytes follow: the caller writes them, then
    /// [`ZipWriter::close_stored`].
    pub fn open_stored(
        &mut self,
        name: &str,
        out: &mut Chunks<'_>,
    ) -> Result<OpenEntry, BundleError> {
        let offset = out.position();
        out.write(&Self::local_header(
            name,
            STORED,
            HAS_DESCRIPTOR,
            (0, 0, 0),
        )?)?;
        Ok(OpenEntry {
            name: name.to_owned(),
            offset,
            data_start: out.position(),
        })
    }

    /// Ends a stored entry whose bytes had `crc`: writes its data descriptor and lists it.
    pub fn close_stored(
        &mut self,
        entry: OpenEntry,
        crc: u32,
        out: &mut Chunks<'_>,
    ) -> Result<(), BundleError> {
        let size = size_field(out.position() - entry.data_start, &entry.name)?;
        let mut descriptor = Bytes(Vec::with_capacity(16));
        descriptor.u32(DATA_DESCRIPTOR).u32(crc).u32(size).u32(size);
        out.write(&descriptor.0)?;
        self.entries.push(Entry {
            name: entry.name,
            method: STORED,
            flags: HAS_DESCRIPTOR,
            crc,
            compressed: size,
            size,
            offset: entry.offset,
        });
        Ok(())
    }

    /// Writes the central directory and the end records.
    pub fn finish(self, out: &mut Chunks<'_>) -> Result<(), BundleError> {
        let directory_offset = out.position();
        let mut directory = Bytes(Vec::new());
        for entry in &self.entries {
            let zip64 = entry.offset >= self.zip64_from;
            directory
                .u32(CENTRAL_HEADER)
                .u16(VERSION_20)
                .u16(if zip64 { VERSION_45 } else { VERSION_20 })
                .u16(entry.flags)
                .u16(entry.method)
                .u16(DOS_TIME)
                .u16(DOS_DATE)
                .u32(entry.crc)
                .u32(entry.compressed)
                .u32(entry.size)
                .u16(u16_of(entry.name.len(), &entry.name)?)
                .u16(if zip64 { 12 } else { 0 })
                .u16(0)
                .u16(0)
                .u16(0)
                .u32(0)
                .u32(if zip64 {
                    U32_LIMIT as u32
                } else {
                    entry.offset as u32
                })
                .raw(entry.name.as_bytes());
            if zip64 {
                directory.u16(ZIP64_EXTRA).u16(8).u64(entry.offset);
            }
        }
        out.write(&directory.0)?;

        let count = self.entries.len() as u64;
        let directory_size = directory.0.len() as u64;
        let zip64 = count >= U16_LIMIT
            || directory_size >= self.zip64_from
            || directory_offset >= self.zip64_from;
        let mut end = Bytes(Vec::new());
        if zip64 {
            let zip64_end = out.position();
            end.u32(ZIP64_END)
                .u64(44)
                .u16(VERSION_45)
                .u16(VERSION_45)
                .u32(0)
                .u32(0)
                .u64(count)
                .u64(count)
                .u64(directory_size)
                .u64(directory_offset)
                .u32(ZIP64_LOCATOR)
                .u32(0)
                .u64(zip64_end)
                .u32(1);
        }
        let count_field = if count >= U16_LIMIT {
            U16_LIMIT as u16
        } else {
            count as u16
        };
        let field = |value: u64| {
            if value >= self.zip64_from {
                U32_LIMIT as u32
            } else {
                value as u32
            }
        };
        end.u32(END)
            .u16(0)
            .u16(0)
            .u16(count_field)
            .u16(count_field)
            .u32(field(directory_size))
            .u32(field(directory_offset))
            .u16(0);
        out.write(&end.0)
    }
}

#[cfg(test)]
mod tests {
    use slidock_io::zip::{ZipArchive, ZipLimits};
    use slidock_io::{BytesReader, RangeReader};

    use super::*;
    use crate::BundleSink;

    #[test]
    fn computes_the_zip_crc() {
        assert_eq!(Crc32::of(b""), 0);
        assert_eq!(Crc32::of(b"123456789"), 0xcbf4_3926);
        let mut split = Crc32::default();
        split.update(b"1234");
        split.update(b"56789");
        assert_eq!(split.finish(), 0xcbf4_3926);
        // Eight bytes at a time and one at a time agree, whatever the split.
        let bytes: Vec<u8> = (0..1000u32).map(|i| (i * 7 + i / 13) as u8).collect();
        let bitwise = |bytes: &[u8]| {
            let mut crc = 0xffff_ffffu32;
            for &byte in bytes {
                crc ^= u32::from(byte);
                for _ in 0..8 {
                    crc = if crc & 1 == 1 {
                        0xedb8_8320 ^ (crc >> 1)
                    } else {
                        crc >> 1
                    };
                }
            }
            !crc
        };
        for len in [0, 1, 7, 8, 9, 15, 16, 17, 999, 1000] {
            assert_eq!(Crc32::of(&bytes[..len]), bitwise(&bytes[..len]), "{len}");
            for at in [0, 1, 3, 8, len / 2] {
                let at = at.min(len);
                let mut split = Crc32::default();
                split.update(&bytes[..at]);
                split.update(&bytes[at..len]);
                assert_eq!(split.finish(), bitwise(&bytes[..len]), "{len} at {at}");
            }
        }
    }

    struct Collect(Vec<u8>);

    impl BundleSink for Collect {
        fn write(&mut self, _: &str, bytes: &[u8]) -> Result<(), BundleError> {
            self.0.extend_from_slice(bytes);
            Ok(())
        }
    }

    /// A zip of a deflated text entry and a stored one, every offset from `zip64_from` on as ZIP64.
    pub(crate) fn sample(zip64_from: u64) -> Vec<u8> {
        let mut collect = Collect(Vec::new());
        let mut out = Chunks::new(&mut collect);
        let mut zip = ZipWriter::with_zip64_from(zip64_from);
        let text = "<p>hello</p>\n".repeat(100);
        zip.deflated("index.html", text.as_bytes(), &mut out)
            .unwrap();
        let entry = zip.open_stored("media/a.png", &mut out).unwrap();
        let media: Vec<u8> = (0..=255u8).cycle().take(3000).collect();
        out.write(&media).unwrap();
        zip.close_stored(entry, Crc32::of(&media), &mut out)
            .unwrap();
        zip.deflated("LICENSES.txt", b"", &mut out).unwrap();
        zip.finish(&mut out).unwrap();
        out.finish().unwrap();
        collect.0
    }

    fn read_back(bytes: Vec<u8>) {
        let archive = ZipArchive::open(BytesReader::new(bytes), ZipLimits::default()).unwrap();
        let names: Vec<&str> = archive.entries().iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["index.html", "media/a.png", "LICENSES.txt"]);
        let text = archive.read(&archive.entries()[0]).unwrap();
        assert_eq!(&*text, "<p>hello</p>\n".repeat(100).as_bytes());
        let media = archive.open_entry(&archive.entries()[1]).unwrap();
        assert_eq!(media.len(), 3000);
        assert_eq!(media.read_at(256, 2).unwrap().as_ref(), &[0, 1]);
        assert!(archive.read(&archive.entries()[2]).unwrap().is_empty());
    }

    #[test]
    fn writes_a_zip_the_reader_opens_with_32_bit_and_with_zip64_records() {
        let plain = sample(U32_LIMIT);
        assert_eq!(plain, sample(U32_LIMIT), "deterministic");
        read_back(plain.clone());
        let zip64 = sample(0);
        assert_ne!(zip64, plain);
        read_back(zip64);
    }

    #[test]
    fn dates_every_entry_1980_and_stores_media_with_a_descriptor() {
        let bytes = sample(U32_LIMIT);
        assert_eq!(&bytes[..4], b"PK\x03\x04");
        // The first local header: version, flags, method, time, date.
        assert_eq!(&bytes[4..14], &[20, 0, 0, 0, 8, 0, 0, 0, 0x21, 0]);
        let archive =
            ZipArchive::open(BytesReader::new(bytes.clone()), ZipLimits::default()).unwrap();
        let media = &archive.entries()[1];
        let at = media.local_header_offset as usize;
        assert_eq!(&bytes[at + 6..at + 8], &HAS_DESCRIPTOR.to_le_bytes());
        assert_eq!(media.method, STORED);
        assert_eq!(media.compressed_size, 3000);
        // The descriptor after the bytes holds what the directory holds.
        let end = at + 30 + media.name.len() + 3000;
        let content: Vec<u8> = (0..=255u8).cycle().take(3000).collect();
        let mut descriptor = DATA_DESCRIPTOR.to_le_bytes().to_vec();
        descriptor.extend(Crc32::of(&content).to_le_bytes());
        descriptor.extend(3000u32.to_le_bytes());
        descriptor.extend(3000u32.to_le_bytes());
        assert_eq!(&bytes[end..end + 16], descriptor.as_slice());
    }

    #[test]
    fn refuses_an_entry_of_4_gib() {
        assert!(size_field(U32_LIMIT - 1, "x").is_ok());
        assert_eq!(
            size_field(U32_LIMIT, "media/x.mp4"),
            Err(BundleError::Zip("media/x.mp4 is 4 GiB or more".into()))
        );
    }
}
