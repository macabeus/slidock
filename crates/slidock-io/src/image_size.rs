//! Image dimensions, resolution and animation from header bytes alone, for a frontend to record. The first 64 KiB is
//! enough for every format here but JPEG, PNG and TIFF, whose frame header, animation chunk or later pages can sit
//! further: their readers read on from the source. An SVG is sized only once the whole document passes the
//! allow-list (`svg.rs`), so an SVG without a size is one the output does not show. Every read is bounds-checked, so a
//! truncated or lying header yields `None`.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::heif::{HeifLimits, read_heif};
use crate::range::{RangeError, RangeReader};
use crate::sniff::Mime;
use crate::svg::{MAX_SVG_BYTES, SvgFacts, SvgRefusal, check_svg};
use crate::tiff::{MAX_TIFF_PAGES, MAX_TIFF_TAGS};

/// Stated resolution, in dots per inch.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Dpi {
    pub x: f64,
    pub y: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ImageSize {
    pub width: u32,
    pub height: u32,
    /// Stated resolution, when the file states one in absolute units.
    pub dpi: Option<Dpi>,
    /// Frames of an animated image: declared by APNG, counted for GIF and WebP within the bytes given (a lower bound
    /// when they end first). `None` for still images.
    pub frames: Option<u32>,
}

impl ImageSize {
    const fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            dpi: None,
            frames: None,
        }
    }
}

const METERS_PER_INCH: f64 = 0.0254;

/// Reads at byte offsets of a file, of which a view may hold only parts: a read outside them is absent, or zero.
trait ByteView {
    fn slice(&self, at: u64, len: u64) -> Option<&[u8]>;

    fn has(&self, at: u64, len: u64) -> bool {
        self.slice(at, len).is_some()
    }

    fn u8(&self, at: u64) -> u8 {
        self.slice(at, 1)
            .and_then(|b| b.first().copied())
            .unwrap_or(0)
    }

    fn u16(&self, at: u64, little: bool) -> u16 {
        let Some(&[a, b]) = self.slice(at, 2) else {
            return 0;
        };
        if little {
            u16::from_le_bytes([a, b])
        } else {
            u16::from_be_bytes([a, b])
        }
    }

    fn u24le(&self, at: u64) -> u32 {
        let Some(&[a, b, c]) = self.slice(at, 3) else {
            return 0;
        };
        u32::from_le_bytes([a, b, c, 0])
    }

    fn u32(&self, at: u64, little: bool) -> u32 {
        let Some(&[a, b, c, d]) = self.slice(at, 4) else {
            return 0;
        };
        if little {
            u32::from_le_bytes([a, b, c, d])
        } else {
            u32::from_be_bytes([a, b, c, d])
        }
    }

    /// Whether the bytes at `at` are `text`.
    fn ascii(&self, at: u64, text: &[u8]) -> bool {
        self.slice(at, text.len() as u64) == Some(text)
    }
}

/// A file's first bytes.
struct Bytes<'a>(&'a [u8]);

/// `len` bytes of `data` from `at`, when it holds them.
fn slice_of(data: &[u8], at: u64, len: u64) -> Option<&[u8]> {
    let start = usize::try_from(at).ok()?;
    let end = start.checked_add(usize::try_from(len).ok()?)?;
    data.get(start..end)
}

impl ByteView for Bytes<'_> {
    fn slice(&self, at: u64, len: u64) -> Option<&[u8]> {
        slice_of(self.0, at, len)
    }
}

/// Byte ranges of a file fetched one by one: a read inside a fetched range answers, others are absent.
#[derive(Default)]
struct SparseBytes<'a> {
    chunks: Vec<(u64, Cow<'a, [u8]>)>,
}

impl<'a> SparseBytes<'a> {
    fn add(&mut self, at: u64, data: Cow<'a, [u8]>) {
        self.chunks.push((at, data));
    }
}

impl ByteView for SparseBytes<'_> {
    fn slice(&self, at: u64, len: u64) -> Option<&[u8]> {
        self.chunks
            .iter()
            .find_map(|(start, data)| slice_of(data, at.checked_sub(*start)?, len))
    }
}

fn positive(size: Option<ImageSize>) -> Option<ImageSize> {
    size.filter(|size| size.width > 0 && size.height > 0)
}

/// The bytes one PNG chunk step reads at most: its length and type, and a pHYs (9) or acTL (8) chunk's fields.
const PNG_STEP_BYTES: u64 = 17;

fn png_header(b: &impl ByteView) -> Option<ImageSize> {
    if !b.has(16, 8) || !b.ascii(12, b"IHDR") {
        return None;
    }
    Some(ImageSize::new(b.u32(16, false), b.u32(20, false)))
}

/// Where a walk over PNG chunks stopped.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum PngWalk {
    /// At the image data, which the animation chunk precedes.
    Done,
    /// At the chunk whose header or fields the bytes do not hold.
    Needs(u64),
}

/// Walks a PNG's chunks from `at` into `size`, up to its image data.
fn png_chunks(b: &impl ByteView, mut at: u64, size: &mut ImageSize) -> PngWalk {
    while b.has(at, 8) {
        let length = u64::from(b.u32(at, false));
        let data = at + 8;
        if b.ascii(at + 4, b"IDAT") || b.ascii(at + 4, b"IEND") {
            return PngWalk::Done;
        }
        if b.ascii(at + 4, b"pHYs") && length >= 9 {
            if !b.has(data, 9) {
                return PngWalk::Needs(at);
            }
            if b.u8(data + 8) == 1 {
                size.dpi = Some(Dpi {
                    x: f64::from(b.u32(data, false)) * METERS_PER_INCH,
                    y: f64::from(b.u32(data + 4, false)) * METERS_PER_INCH,
                });
            }
        }
        if b.ascii(at + 4, b"acTL") && length >= 8 {
            if !b.has(data, 8) {
                return PngWalk::Needs(at);
            }
            let frames = b.u32(data, false);
            if frames > 1 {
                size.frames = Some(frames);
            }
        }
        at = data + length + 4;
    }
    PngWalk::Needs(at)
}

fn png(b: &Bytes<'_>) -> Option<ImageSize> {
    let mut size = png_header(b)?;
    (png_chunks(b, 8, &mut size) == PngWalk::Done).then_some(size)
}

/// Where a JPEG walk stands after one segment.
#[derive(Clone, Copy, Debug, PartialEq)]
enum JpegStep {
    Size(ImageSize),
    Stop,
    /// The next segment, and the resolution so far.
    Next(u64, Option<Dpi>),
}

/// The bytes one JPEG segment step reads at most: its marker and length, and a JFIF or frame header's fields.
const JPEG_STEP_BYTES: u64 = 16;

/// One segment of a JPEG walk at `at`, which `b` holds from `at` for as many bytes as it has.
fn jpeg_step(b: &Bytes<'_>, at: u64, mut dpi: Option<Dpi>) -> JpegStep {
    if b.u8(at) != 0xff {
        return JpegStep::Stop;
    }
    let marker = b.u8(at + 1);
    if marker == 0xff {
        return JpegStep::Next(at + 1, dpi);
    }
    if matches!(marker, 0xd8 | 0x01 | 0xd0..=0xd7) {
        return JpegStep::Next(at + 2, dpi);
    }
    let length = u64::from(b.u16(at + 2, false));
    if length < 2 {
        return JpegStep::Stop;
    }
    if marker == 0xe0 && b.ascii(at + 4, b"JFIF\0") && b.has(at + 11, 5) {
        let unit = b.u8(at + 11);
        let (x, y) = (b.u16(at + 12, false), b.u16(at + 14, false));
        if x > 0 && y > 0 {
            let (x, y) = (f64::from(x), f64::from(y));
            match unit {
                1 => dpi = Some(Dpi { x, y }),
                2 => {
                    dpi = Some(Dpi {
                        x: x * 2.54,
                        y: y * 2.54,
                    })
                }
                _ => {}
            }
        }
    }
    let is_frame = (0xc0..=0xcf).contains(&marker) && !matches!(marker, 0xc4 | 0xc8 | 0xcc);
    if is_frame {
        if !b.has(at + 5, 4) {
            return JpegStep::Stop;
        }
        return JpegStep::Size(ImageSize {
            dpi,
            ..ImageSize::new(
                u32::from(b.u16(at + 7, false)),
                u32::from(b.u16(at + 5, false)),
            )
        });
    }
    JpegStep::Next(at + 2 + length, dpi)
}

/// The JPEG walk within `b`; where the bytes end first, the segment it would read next.
fn jpeg_walk(b: &Bytes<'_>) -> JpegStep {
    let mut step = JpegStep::Next(2, None);
    while let JpegStep::Next(at, dpi) = step {
        if !b.has(at, 4) {
            break;
        }
        step = jpeg_step(b, at, dpi);
    }
    step
}

/// At most this many JPEG segments are walked for an EXIF orientation, and IFD0 entries read.
const MAX_EXIF_STEPS: usize = 256;

/// A JPEG's EXIF orientation (TIFF tag 274 of its first `Exif` APP1 segment's IFD0, 1–8), read from the segments
/// before its first scan: `None` when it states none (which reads as 1, as stored).
pub fn jpeg_orientation(bytes: &[u8]) -> Option<u16> {
    let b = Bytes(bytes);
    if b.u16(0, false) != 0xffd8 {
        return None;
    }
    let mut at = 2u64;
    for _ in 0..MAX_EXIF_STEPS {
        if b.u8(at) != 0xff || !b.has(at, 4) {
            return None;
        }
        let marker = b.u8(at + 1);
        if matches!(marker, 0xda | 0xd9) {
            return None;
        }
        let length = u64::from(b.u16(at + 2, false));
        if length < 2 {
            return None;
        }
        if marker == 0xe1 && b.ascii(at + 4, b"Exif\0\0") {
            let tiff = at + 10;
            let little = match b.slice(tiff, 2) {
                Some(b"II") => true,
                Some(b"MM") => false,
                _ => return None,
            };
            let ifd = tiff + u64::from(b.u32(tiff + 4, little));
            if !b.has(ifd, 2) {
                return None;
            }
            let entries = usize::from(b.u16(ifd, little)).min(MAX_EXIF_STEPS);
            return (0..entries as u64)
                .map(|index| ifd + 2 + 12 * index)
                .take_while(|entry| b.has(*entry, 12))
                .find(|entry| b.u16(*entry, little) == 0x0112 && b.u16(entry + 2, little) == 3)
                .map(|entry| b.u16(entry + 8, little));
        }
        at += 2 + length;
    }
    None
}

fn jpeg(b: &Bytes<'_>) -> Option<ImageSize> {
    match jpeg_walk(b) {
        JpegStep::Size(size) => Some(size),
        _ => None,
    }
}

/// Skips GIF data sub-blocks from `at`; the offset after the terminator, or `None` past the bytes.
fn skip_sub_blocks(b: &Bytes<'_>, mut at: u64) -> Option<u64> {
    while b.has(at, 1) {
        let length = u64::from(b.u8(at));
        at += 1 + length;
        if length == 0 {
            return Some(at);
        }
    }
    None
}

/// The bytes of a GIF colour table its packed flags declare.
fn gif_table(flags: u8) -> u64 {
    if flags & 0x80 != 0 {
        3 << ((flags & 7) + 1)
    } else {
        0
    }
}

fn gif(b: &Bytes<'_>) -> Option<ImageSize> {
    if !b.has(6, 7) {
        return None;
    }
    let mut size = ImageSize::new(u32::from(b.u16(6, true)), u32::from(b.u16(8, true)));
    let mut at = 13 + gif_table(b.u8(10));
    let mut frames = 0;
    while b.has(at, 1) {
        let next = match b.u8(at) {
            0x3b => break,
            0x21 => skip_sub_blocks(b, at + 2),
            0x2c => {
                frames += 1;
                if !b.has(at, 10) {
                    break;
                }
                skip_sub_blocks(b, at + 10 + gif_table(b.u8(at + 9)) + 1)
            }
            _ => break,
        };
        match next {
            Some(next) => at = next,
            None => break,
        }
    }
    if frames > 1 {
        size.frames = Some(frames);
    }
    Some(size)
}

fn webp(b: &Bytes<'_>) -> Option<ImageSize> {
    if b.ascii(12, b"VP8 ") && b.has(26, 4) {
        return Some(ImageSize::new(
            u32::from(b.u16(26, true) & 0x3fff),
            u32::from(b.u16(28, true) & 0x3fff),
        ));
    }
    if b.ascii(12, b"VP8L") && b.has(21, 4) {
        let bits = b.u32(21, true);
        return Some(ImageSize::new(
            (bits & 0x3fff) + 1,
            ((bits >> 14) & 0x3fff) + 1,
        ));
    }
    if b.ascii(12, b"VP8X") && b.has(24, 6) {
        let mut size = ImageSize::new(b.u24le(24) + 1, b.u24le(27) + 1);
        if b.u8(20) & 0x02 != 0 {
            let mut frames = 0;
            let mut at = 12;
            while b.has(at, 8) {
                if b.ascii(at, b"ANMF") {
                    frames += 1;
                }
                let length = u64::from(b.u32(at + 4, true));
                at += 8 + length + length % 2;
            }
            if frames > 1 {
                size.frames = Some(frames);
            }
        }
        return Some(size);
    }
    None
}

/// A TIFF tag's field type and where its value (or the offset of its values) sits.
#[derive(Clone, Copy)]
struct TagEntry {
    field_type: u16,
    at: u64,
}

const TIFF_SHORT: u16 = 3;
const TIFF_LONG: u16 = 4;
const TIFF_RATIONAL: u16 = 5;

/// One TIFF page at `ifd`: its size and resolution (none without both dimensions), and the next page's offset.
fn tiff_page(b: &impl ByteView, ifd: u64, little: bool) -> Option<(Option<ImageSize>, u64)> {
    if !b.has(ifd, 2) {
        return None;
    }
    let count = u64::from(b.u16(ifd, little));
    if count > MAX_TIFF_TAGS || !b.has(ifd + 2, count * 12 + 4) {
        return None;
    }
    let mut tags = BTreeMap::new();
    for i in 0..count {
        let at = ifd + 2 + i * 12;
        tags.insert(
            b.u16(at, little),
            TagEntry {
                field_type: b.u16(at + 2, little),
                at: at + 8,
            },
        );
    }
    let integer = |tag: u16| {
        let entry = tags.get(&tag)?;
        match entry.field_type {
            TIFF_SHORT => Some(u32::from(b.u16(entry.at, little))),
            TIFF_LONG => Some(b.u32(entry.at, little)),
            _ => None,
        }
    };
    let rational = |tag: u16| {
        let entry = tags
            .get(&tag)
            .filter(|entry| entry.field_type == TIFF_RATIONAL)?;
        let offset = u64::from(b.u32(entry.at, little));
        if !b.has(offset, 8) {
            return None;
        }
        let denominator = b.u32(offset + 4, little);
        (denominator != 0).then(|| f64::from(b.u32(offset, little)) / f64::from(denominator))
    };
    let next = u64::from(b.u32(ifd + 2 + count * 12, little));
    let (Some(width), Some(height)) = (integer(256), integer(257)) else {
        return Some((None, next));
    };
    let mut size = ImageSize::new(width, height);
    let unit = integer(296).unwrap_or(2);
    if let (Some(x), Some(y)) = (rational(282), rational(283))
        && x != 0.0
        && y != 0.0
        && (unit == 2 || unit == 3)
    {
        size.dpi = Some(if unit == 2 {
            Dpi { x, y }
        } else {
            Dpi {
                x: x * 2.54,
                y: y * 2.54,
            }
        });
    }
    Some((Some(size), next))
}

fn area(size: &ImageSize) -> u64 {
    u64::from(size.width) * u64::from(size.height)
}

/// The page the TIFF decoder decodes, the largest of the chain (the first of equal ones); `None` when `b` does not
/// hold every page's tags, or the chain is longer than the decoder reads.
fn tiff(b: &impl ByteView) -> Option<ImageSize> {
    let little = b.ascii(0, b"II");
    if !b.has(4, 4) {
        return None;
    }
    let mut largest: Option<ImageSize> = None;
    let mut visited = Vec::new();
    let mut ifd = u64::from(b.u32(4, little));
    while ifd != 0 && !visited.contains(&ifd) {
        if visited.len() as u64 == MAX_TIFF_PAGES {
            return None;
        }
        visited.push(ifd);
        let (size, next) = tiff_page(b, ifd, little)?;
        if let Some(size) = size
            && largest.is_none_or(|largest| area(&size) > area(&largest))
        {
            largest = Some(size);
        }
        ifd = next;
    }
    largest
}

fn bmp(b: &Bytes<'_>) -> Option<ImageSize> {
    if !b.has(14, 4) {
        return None;
    }
    let header = b.u32(14, true);
    if header == 12 && b.has(18, 4) {
        return Some(ImageSize::new(
            u32::from(b.u16(18, true)),
            u32::from(b.u16(20, true)),
        ));
    }
    if !b.has(18, 8) {
        return None;
    }
    let signed = |at: u64| b.u32(at, true).cast_signed().unsigned_abs();
    let mut size = ImageSize::new(signed(18), signed(22));
    if header >= 40 && b.has(38, 8) {
        let (x, y) = (b.u32(38, true), b.u32(42, true));
        if x > 0 && y > 0 {
            size.dpi = Some(Dpi {
                x: f64::from(x) * METERS_PER_INCH,
                y: f64::from(y) * METERS_PER_INCH,
            });
        }
    }
    Some(size)
}

/// The size of a raster image of a sniffed type, or `None` for other types and unreadable headers.
pub fn image_size(head: &[u8], mime: Mime) -> Option<ImageSize> {
    let b = Bytes(head);
    positive(match mime {
        Mime::Png => png(&b),
        Mime::Jpeg => jpeg(&b),
        Mime::Gif => gif(&b),
        Mime::Webp => webp(&b),
        Mime::Tiff => tiff(&b),
        Mime::Bmp => bmp(&b),
        _ => None,
    })
}

/// A TIFF's pages read through the source: the header, then each page's tags and resolution values.
fn tiff_from<R: RangeReader + ?Sized>(source: &R) -> Result<Option<ImageSize>, RangeError> {
    let size = source.len();
    if size < 8 {
        return Ok(None);
    }
    let mut b = SparseBytes::default();
    let fetch = |b: &mut SparseBytes<'_>, at: u64, len: u64| -> Result<(), RangeError> {
        let end = at.saturating_add(len).min(size);
        if at < end {
            // At most a page's tags, so it fits in usize.
            b.add(
                at,
                Cow::Owned(source.read_at(at, (end - at) as usize)?.into_owned()),
            );
        }
        Ok(())
    };
    fetch(&mut b, 0, 8)?;
    let little = b.ascii(0, b"II");
    let mut visited = Vec::new();
    let mut ifd = u64::from(b.u32(4, little));
    while ifd != 0 && !visited.contains(&ifd) && (visited.len() as u64) < MAX_TIFF_PAGES {
        visited.push(ifd);
        fetch(&mut b, ifd, 2)?;
        let count = u64::from(b.u16(ifd, little));
        if count > MAX_TIFF_TAGS {
            return Ok(None);
        }
        fetch(&mut b, ifd + 2, count * 12 + 4)?;
        for i in 0..count {
            let at = ifd + 2 + i * 12;
            let tag = b.u16(at, little);
            if (tag == 282 || tag == 283) && b.u16(at + 2, little) == TIFF_RATIONAL {
                let offset = u64::from(b.u32(at + 8, little));
                fetch(&mut b, offset, 8)?;
            }
        }
        ifd = u64::from(b.u32(ifd + 2 + count * 12, little));
    }
    Ok(tiff(&b))
}

/// How many PNG chunks a walk past the head reads: text, colour profile and metadata chunks take a handful.
const MAX_PNG_CHUNKS: usize = 256;

/// A PNG's chunks read on from the source past `head`, one small read each, up to its image data.
fn png_from<R: RangeReader + ?Sized>(
    head: &[u8],
    source: &R,
) -> Result<Option<ImageSize>, RangeError> {
    let mut b = SparseBytes::default();
    b.add(0, Cow::Borrowed(head));
    let Some(mut size) = png_header(&b) else {
        return Ok(None);
    };
    let mut walk = png_chunks(&b, 8, &mut size);
    let mut count = 0;
    while let PngWalk::Needs(at) = walk {
        if count == MAX_PNG_CHUNKS || at.saturating_add(8) > source.len() {
            return Ok(None);
        }
        let len = PNG_STEP_BYTES.min(source.len() - at);
        b.add(
            at,
            Cow::Owned(source.read_at(at, len as usize)?.into_owned()),
        );
        walk = png_chunks(&b, at, &mut size);
        // The source ends inside the chunk's fields.
        if walk == PngWalk::Needs(at) {
            return Ok(None);
        }
        count += 1;
    }
    Ok(Some(size))
}

/// How many JPEG segments a walk past the head reads: camera and editor metadata take a handful.
const MAX_JPEG_SEGMENTS: usize = 256;

fn jpeg_from<R: RangeReader + ?Sized>(
    head: &[u8],
    source: &R,
) -> Result<Option<ImageSize>, RangeError> {
    let mut step = jpeg_walk(&Bytes(head));
    let mut count = 0;
    while let JpegStep::Next(at, dpi) = step {
        if count == MAX_JPEG_SEGMENTS {
            break;
        }
        if at.saturating_add(4) > source.len() {
            return Ok(None);
        }
        let len = JPEG_STEP_BYTES.min(source.len() - at);
        let window = source.read_at(at, len as usize)?;
        step = match jpeg_step(&Bytes(&window), 0, dpi) {
            JpegStep::Next(next, dpi) => JpegStep::Next(at + next, dpi),
            other => other,
        };
        count += 1;
    }
    Ok(match step {
        JpegStep::Size(size) => Some(size),
        _ => None,
    })
}

/// Whether the SVG a source holds passes [`check_svg`], read whole (`head` is its first bytes): its size, or why not.
pub fn check_svg_source<R: RangeReader + ?Sized>(
    head: &[u8],
    source: &R,
) -> Result<Result<SvgFacts, SvgRefusal>, RangeError> {
    let len = source.len();
    if len > MAX_SVG_BYTES {
        return Ok(Err(SvgRefusal::TooLarge));
    }
    let whole = if head.len() as u64 >= len {
        Cow::Borrowed(head)
    } else {
        // At most MAX_SVG_BYTES, so it fits in usize.
        source.read_at(0, len as usize)?
    };
    Ok(check_svg(&whole))
}

/// An SVG's size, when the whole document passes [`check_svg`]: `None` marks an SVG the output does not show.
fn svg_from<R: RangeReader + ?Sized>(
    head: &[u8],
    source: &R,
) -> Result<Option<ImageSize>, RangeError> {
    Ok(check_svg_source(head, source)?
        .ok()
        .map(|facts| ImageSize::new(facts.width, facts.height)))
}

/// The size of an image from its head, read on from the source where the head is not enough: an SVG's from the whole
/// document once it passes the allow-list; a JPEG's frame header
/// can sit past large metadata segments (EXIF thumbnails, ICC profiles, Photoshop data), which the walk skips with one
/// small read each, as can an APNG's animation chunk past large text or profile chunks; a TIFF's tags can follow its
/// pixels, and its largest page can come after others. A HEIC's is the size its primary image shows at once its
/// crop and quarter turns apply, from its `meta` box (`read_heif`): `None` when the reader plans no decode of it.
pub fn read_image_size<R: RangeReader + ?Sized>(
    head: &[u8],
    mime: Mime,
    source: &R,
) -> Result<Option<ImageSize>, RangeError> {
    if mime == Mime::Svg {
        return svg_from(head, source);
    }
    if mime == Mime::Heic {
        // Sizing decodes nothing, so no pixel limit applies; a file the reader refuses (a box past its end
        // included) is unsized.
        return Ok(read_heif(source, &HeifLimits::with_pixels(u64::MAX))
            .ok()
            .and_then(|image| {
                let (width, height) = image.size();
                positive(Some(ImageSize::new(width, height)))
            }));
    }
    let size = image_size(head, mime);
    if size.is_some() || head.len() as u64 >= source.len() {
        return Ok(size);
    }
    Ok(positive(match mime {
        Mime::Tiff => tiff_from(source)?,
        Mime::Png => png_from(head, source)?,
        Mime::Jpeg => jpeg_from(head, source)?,
        _ => None,
    }))
}
