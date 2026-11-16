//! A bounded TIFF decoder for plan-media's transcodes. It walks the IFD chain itself, so a chain that loops or runs
//! long ends at a visited offset or the page cap, and every tag's values must lie inside the file. It reads the
//! largest page: strips of 8 or 16-bit chunky samples, grey (either polarity), RGB or palette colour, with an alpha
//! sample (straightened when associated); uncompressed, LZW, Deflate or PackBits, with the horizontal predictor. Every
//! strip decodes into at most the bytes its rows hold, so a strip's data stays within the image. Tiles, planar
//! samples and other compressions are refused.

use std::collections::BTreeMap;

use thiserror::Error;

use crate::inflate::unzlib_into;
use crate::png::RgbaImage;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct TiffError(pub String);

fn error<T>(message: impl Into<String>) -> Result<T, TiffError> {
    Err(TiffError(message.into()))
}

/// How many pages a chain may hold, and tags a page; the sizer reads within the same limits.
pub const MAX_TIFF_PAGES: u64 = 64;
pub const MAX_TIFF_TAGS: u64 = 1024;

const IMAGE_WIDTH: u16 = 256;
const IMAGE_LENGTH: u16 = 257;
const BITS_PER_SAMPLE: u16 = 258;
const COMPRESSION: u16 = 259;
const PHOTOMETRIC: u16 = 262;
const STRIP_OFFSETS: u16 = 273;
const SAMPLES_PER_PIXEL: u16 = 277;
const ROWS_PER_STRIP: u16 = 278;
const STRIP_BYTE_COUNTS: u16 = 279;
const PLANAR_CONFIGURATION: u16 = 284;
const PREDICTOR: u16 = 317;
const COLOR_MAP: u16 = 320;
const TILE_WIDTH: u16 = 322;
const EXTRA_SAMPLES: u16 = 338;

const COMPRESSION_NONE: u32 = 1;
const COMPRESSION_LZW: u32 = 5;
const COMPRESSION_DEFLATE: u32 = 8;
const COMPRESSION_ADOBE_DEFLATE: u32 = 32946;
const COMPRESSION_PACK_BITS: u32 = 32773;

/// Bytes per value of each TIFF field type (BYTE, ASCII, SHORT, LONG, RATIONAL, …).
fn type_size(field_type: u16) -> Option<u64> {
    match field_type {
        1 | 2 | 6 | 7 => Some(1),
        3 | 8 => Some(2),
        4 | 9 => Some(4),
        5 | 10 | 16 => Some(8),
        _ => None,
    }
}

/// One page's integer (SHORT and LONG) tags.
#[derive(Debug, Default)]
struct Page {
    tags: BTreeMap<u16, Vec<u32>>,
}

impl Page {
    fn one(&self, tag: u16) -> Option<u32> {
        self.tags
            .get(&tag)
            .and_then(|values| values.first().copied())
    }

    fn area(&self) -> u64 {
        u64::from(self.one(IMAGE_WIDTH).unwrap_or(0))
            * u64::from(self.one(IMAGE_LENGTH).unwrap_or(0))
    }
}

struct Reader<'a> {
    bytes: &'a [u8],
    little: bool,
}

impl Reader<'_> {
    fn need(&self, at: u64, len: u64) -> Result<std::ops::Range<usize>, TiffError> {
        let end = at
            .checked_add(len)
            .filter(|&end| end <= self.bytes.len() as u64);
        match end {
            // Inside the file, so both ends fit in usize.
            Some(end) => Ok(at as usize..end as usize),
            None => error("A TIFF offset lies past the file"),
        }
    }

    fn u16(&self, at: u64) -> Result<u16, TiffError> {
        let range = self.need(at, 2)?;
        let bytes: [u8; 2] = self
            .bytes
            .get(range)
            .and_then(|b| b.try_into().ok())
            .unwrap_or_default();
        Ok(if self.little {
            u16::from_le_bytes(bytes)
        } else {
            u16::from_be_bytes(bytes)
        })
    }

    fn u32(&self, at: u64) -> Result<u32, TiffError> {
        let range = self.need(at, 4)?;
        let bytes: [u8; 4] = self
            .bytes
            .get(range)
            .and_then(|b| b.try_into().ok())
            .unwrap_or_default();
        Ok(if self.little {
            u32::from_le_bytes(bytes)
        } else {
            u32::from_be_bytes(bytes)
        })
    }
}

/// One IFD's integer tags (later entries replace earlier ones); tags of other types are skipped.
fn read_page(r: &Reader<'_>, at: u64) -> Result<(Page, u64), TiffError> {
    let count = u64::from(r.u16(at)?);
    if count > MAX_TIFF_TAGS {
        return error(format!("A TIFF page holds {count} tags"));
    }
    r.need(at + 2, count * 12 + 4)?;
    let mut page = Page::default();
    for i in 0..count {
        let entry = at + 2 + i * 12;
        let (tag, field_type, n) = (
            r.u16(entry)?,
            r.u16(entry + 2)?,
            u64::from(r.u32(entry + 4)?),
        );
        let Some(size) = type_size(field_type).filter(|_| field_type == 3 || field_type == 4)
        else {
            continue;
        };
        let bytes = n * size;
        let values = if bytes <= 4 {
            entry + 8
        } else {
            u64::from(r.u32(entry + 8)?)
        };
        // A tag's values lie inside the file, whatever its count says.
        r.need(values, bytes)?;
        let read = (0..n)
            .map(|k| {
                if field_type == 3 {
                    r.u16(values + k * 2).map(u32::from)
                } else {
                    r.u32(values + k * 4)
                }
            })
            .collect::<Result<Vec<u32>, TiffError>>()?;
        page.tags.insert(tag, read);
    }
    Ok((page, u64::from(r.u32(at + 2 + count * 12)?)))
}

fn pages_of(r: &Reader<'_>) -> Result<Vec<Page>, TiffError> {
    let mut pages = Vec::new();
    let mut visited = Vec::new();
    let mut at = u64::from(r.u32(4)?);
    while at != 0 {
        if visited.contains(&at) {
            break;
        }
        if pages.len() as u64 == MAX_TIFF_PAGES {
            return error(format!("A TIFF holds more than {MAX_TIFF_PAGES} pages"));
        }
        visited.push(at);
        let (page, next) = read_page(r, at)?;
        pages.push(page);
        at = next;
    }
    Ok(pages)
}

/// TIFF's LZW: MSB-first codes of 9 to 12 bits, with the early change; output stops at `limit` bytes.
fn lzw(data: &[u8], limit: usize) -> Result<Vec<u8>, TiffError> {
    let mut out = Vec::with_capacity(limit);
    let mut table: Vec<Vec<u8>> = Vec::with_capacity(4096);
    let reset = |table: &mut Vec<Vec<u8>>| {
        table.clear();
        table.extend((0..=255u8).map(|byte| vec![byte]));
        table.extend([Vec::new(), Vec::new()]);
    };
    reset(&mut table);
    let total_bits = data.len() * 8;
    let (mut bit, mut width) = (0usize, 9usize);
    let mut previous: Option<Vec<u8>> = None;
    while bit + width <= total_bits && out.len() < limit {
        let mut code = 0usize;
        for _ in 0..width {
            let byte = data.get(bit >> 3).copied().unwrap_or(0);
            code = (code << 1) | usize::from((byte >> (7 - (bit & 7))) & 1);
            bit += 1;
        }
        if code == 257 {
            break;
        }
        if code == 256 {
            reset(&mut table);
            width = 9;
            previous = None;
            continue;
        }
        let entry = match (table.get(code), &previous) {
            (Some(entry), _) => entry.clone(),
            (None, Some(previous)) if code == table.len() => {
                let mut entry = previous.clone();
                entry.extend(previous.first().copied());
                entry
            }
            _ => return error("A TIFF LZW code is out of range"),
        };
        let room = limit - out.len();
        out.extend(entry.iter().take(room));
        if let Some(previous) = &previous
            && table.len() < 4096
        {
            let mut grown = previous.clone();
            grown.extend(entry.first().copied());
            table.push(grown);
        }
        previous = Some(entry);
        if table.len() + 1 >= 1 << width && width < 12 {
            width += 1;
        }
    }
    Ok(out)
}

fn pack_bits(data: &[u8], limit: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(limit);
    let mut at = 0;
    while at < data.len() && out.len() < limit {
        let n = data.get(at).copied().unwrap_or(0).cast_signed();
        at += 1;
        if n >= 0 {
            let literal = usize::from(n.unsigned_abs()) + 1;
            let run = literal.min(data.len() - at).min(limit - out.len());
            out.extend(data.get(at..at + run).unwrap_or_default());
            at += literal;
        } else if n != -128 && at < data.len() {
            let run = (1 + usize::from(n.unsigned_abs())).min(limit - out.len());
            let byte = data.get(at).copied().unwrap_or(0);
            at += 1;
            out.resize(out.len() + run, byte);
        }
    }
    out
}

fn decompress(compression: u32, data: &[u8], limit: usize) -> Result<Vec<u8>, TiffError> {
    match compression {
        COMPRESSION_NONE => Ok(data.get(..limit).unwrap_or(data).to_vec()),
        COMPRESSION_LZW => lzw(data, limit),
        // Inflating into a buffer of the strip's size stops there.
        COMPRESSION_DEFLATE | COMPRESSION_ADOBE_DEFLATE => unzlib_into(data, limit)
            .map_err(|cause| TiffError(format!("A TIFF strip does not inflate: {cause}"))),
        COMPRESSION_PACK_BITS => Ok(pack_bits(data, limit)),
        _ => error(format!("TIFF compression {compression} is not supported")),
    }
}

/// `x * 255 / a` rounded half up, as `Math.round` rounds it.
fn straighten(x: u8, a: u8) -> u8 {
    let (x, a) = (u32::from(x), u32::from(a));
    ((2 * x * 255 + a) / (2 * a)).min(255) as u8
}

/// The largest page of a TIFF as RGBA; refuses anything over `max_pixels`, and every form it does not read.
pub fn decode_tiff(bytes: &[u8], max_pixels: u64) -> Result<RgbaImage, TiffError> {
    let little = match bytes {
        [b'I', b'I', ..] if bytes.len() >= 8 => true,
        [b'M', b'M', ..] if bytes.len() >= 8 => false,
        _ => return error("Not a TIFF file"),
    };
    let r = Reader { bytes, little };
    if r.u16(2)? != 42 {
        return error("Not a classic TIFF file");
    }
    let pages = pages_of(&r)?;
    // The first of the largest pages.
    let page = pages.iter().rev().max_by_key(|page| page.area());
    let Some(page) = page.filter(|page| page.area() > 0) else {
        return error("The TIFF holds no image");
    };
    let width = page.one(IMAGE_WIDTH).unwrap_or(0);
    let height = page.one(IMAGE_LENGTH).unwrap_or(0);
    let pixels = u64::from(width) * u64::from(height);
    if pixels > max_pixels {
        return error(format!("The TIFF's {pixels} pixels are over {max_pixels}"));
    }

    let samples = page.one(SAMPLES_PER_PIXEL).unwrap_or(1);
    let bits = page
        .tags
        .get(&BITS_PER_SAMPLE)
        .cloned()
        .unwrap_or_else(|| vec![1]);
    let depth = bits.first().copied();
    let photometric = page.one(PHOTOMETRIC).unwrap_or(1);
    let compression = page.one(COMPRESSION).unwrap_or(1);
    let predictor = page.one(PREDICTOR).unwrap_or(1);
    if page.tags.contains_key(&TILE_WIDTH) {
        return error("Tiled TIFFs are not supported");
    }
    if page.one(PLANAR_CONFIGURATION).unwrap_or(1) != 1 {
        return error("Planar TIFFs are not supported");
    }
    let depth = match depth {
        Some(depth @ (8 | 16)) if bits.iter().all(|&b| b == depth) => depth,
        _ => {
            let joined: Vec<String> = bits.iter().map(u32::to_string).collect();
            return error(format!(
                "{}-bit TIFF samples are not supported",
                joined.join("/")
            ));
        }
    };
    if !(1..=4).contains(&samples) {
        return error(format!("TIFFs of {samples} samples are not supported"));
    }
    if predictor != 1 && predictor != 2 {
        return error(format!("TIFF predictor {predictor} is not supported"));
    }
    let colour = if photometric == 2 { 3 } else { 1 };
    if photometric > 3 || samples < colour {
        return error(format!(
            "TIFF photometric {photometric} with {samples} samples is not supported"
        ));
    }
    let alpha = (samples > colour).then(|| page.one(EXTRA_SAMPLES).unwrap_or(2));
    let levels = 1usize << depth;
    let palette = if photometric == 3 {
        match page.tags.get(&COLOR_MAP) {
            Some(map) if map.len() >= 3 * levels && depth == 8 => Some(map),
            _ => return error("A palette TIFF without its colour map"),
        }
    } else {
        None
    };

    let offsets = page
        .tags
        .get(&STRIP_OFFSETS)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let counts = page
        .tags
        .get(&STRIP_BYTE_COUNTS)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let rows_per_strip = page.one(ROWS_PER_STRIP).unwrap_or(height).min(height);
    let sample_bytes = (depth / 8) as usize;
    let (width, height, samples) = (width as usize, height as usize, samples as usize);
    let row_bytes = width.checked_mul(samples * sample_bytes);
    let raw_len = row_bytes.and_then(|row_bytes| row_bytes.checked_mul(height));
    let data_len = width.checked_mul(4).and_then(|row| row.checked_mul(height));
    let (Some(row_bytes), Some(_), Some(data_len)) = (row_bytes, raw_len, data_len) else {
        return error(format!("The TIFF's {pixels} pixels do not fit in memory"));
    };
    let strips = if rows_per_strip == 0 {
        usize::MAX
    } else {
        height.div_ceil(rows_per_strip as usize)
    };
    if offsets.len() < strips || counts.len() < strips {
        return error("The TIFF lacks strips");
    }
    let rows_per_strip = rows_per_strip as usize;

    if predictor == 2 && depth != 8 {
        return error("The TIFF predictor needs 8-bit samples");
    }

    // Strip by strip: each strip's rows decode, undo the predictor and land in the RGBA image before the next
    // strip decodes, so the samples are never held whole beside it. A strip shorter than its rows reads as zeros.
    let pixel_bytes = samples * sample_bytes;
    let mut data = vec![0u8; data_len];
    for (s, (&offset, &count)) in offsets.iter().zip(counts).take(strips).enumerate() {
        let range = r.need(u64::from(offset), u64::from(count))?;
        let rows = rows_per_strip.min(height - s * rows_per_strip);
        let mut raw = decompress(
            compression,
            bytes.get(range).unwrap_or_default(),
            rows * row_bytes,
        )?;
        raw.resize(rows * row_bytes, 0);
        if predictor == 2 {
            for row in raw.chunks_mut(row_bytes.max(1)) {
                for at in samples..row.len() {
                    let left = row.get(at - samples).copied().unwrap_or(0);
                    if let Some(byte) = row.get_mut(at) {
                        *byte = byte.wrapping_add(left);
                    }
                }
            }
        }
        let sample = |at: usize| -> u8 {
            let at = if depth == 16 && little { at + 1 } else { at };
            raw.get(at).copied().unwrap_or(0)
        };
        let first = s * rows_per_strip * width * 4;
        let target = data
            .get_mut(first..first + rows * width * 4)
            .unwrap_or_default();
        for (p, [r, g, b, a]) in target.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            let at = p * pixel_bytes;
            if let Some(palette) = palette {
                let index = usize::from(raw.get(at).copied().unwrap_or(0));
                let entry = |plane: usize| {
                    (palette.get(plane * levels + index).copied().unwrap_or(0) >> 8) as u8
                };
                (*r, *g, *b) = (entry(0), entry(1), entry(2));
            } else if colour == 3 {
                (*r, *g, *b) = (
                    sample(at),
                    sample(at + sample_bytes),
                    sample(at + 2 * sample_bytes),
                );
            } else {
                let grey = if photometric == 0 {
                    255 - sample(at)
                } else {
                    sample(at)
                };
                (*r, *g, *b) = (grey, grey, grey);
            }
            *a = if alpha.is_some() {
                sample(at + colour as usize * sample_bytes)
            } else {
                255
            };
            // An associated alpha premultiplied the colour; PNG's is straight.
            if alpha == Some(1) && *a > 0 && *a < 255 {
                for c in [&mut *r, &mut *g, &mut *b] {
                    *c = straighten(*c, *a);
                }
            }
        }
    }
    Ok(RgbaImage {
        width: width as u32,
        height: height as u32,
        data,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn straightens_as_math_round_does() {
        for a in 1..=254u8 {
            for x in 0..=a {
                let expected = ((f64::from(x) * 255.0) / f64::from(a) + 0.5)
                    .floor()
                    .min(255.0);
                assert_eq!(f64::from(straighten(x, a)), expected, "{x} / {a}");
            }
        }
    }

    #[test]
    fn decodes_lzw_with_the_early_change() {
        // Codes 256 (clear), 'a', 'b', 258 ("ab"), 257 (end), 9 bits each, MSB first.
        let codes = [256u16, 97, 98, 258, 257];
        let mut bits = String::new();
        for code in codes {
            bits.push_str(&format!("{code:09b}"));
        }
        while !bits.len().is_multiple_of(8) {
            bits.push('0');
        }
        let data: Vec<u8> = (0..bits.len() / 8)
            .map(|i| u8::from_str_radix(&bits[i * 8..i * 8 + 8], 2).unwrap())
            .collect();
        assert_eq!(lzw(&data, 100).unwrap(), b"abab");
        assert_eq!(lzw(&data, 3).unwrap(), b"aba");
    }

    #[test]
    fn unpacks_pack_bits_runs() {
        // A literal run of 3, then 3 repeats of 9, then a no-op.
        assert_eq!(
            pack_bits(&[2, 1, 2, 3, 0xfe, 9, 0x80], 100),
            [1, 2, 3, 9, 9, 9]
        );
        assert_eq!(pack_bits(&[2, 1, 2, 3, 0xfe, 9], 4), [1, 2, 3, 9]);
        // A literal that claims more bytes than follow.
        assert_eq!(pack_bits(&[5, 1], 100), [1]);
    }
}
