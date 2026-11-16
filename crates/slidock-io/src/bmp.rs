//! A bounded BMP decoder for plan-media's transcode to PNG: uncompressed 1, 4, 8, 16, 24 and 32-bit rows (with
//! bit-field masks for 16 and 32 bits). Run-length and embedded JPEG/PNG BMPs are refused; browsers rarely meet them.

use thiserror::Error;

use crate::png::RgbaImage;

#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{0}")]
pub struct BmpError(pub String);

fn error<T>(message: impl Into<String>) -> Result<T, BmpError> {
    Err(BmpError(message.into()))
}

const BI_RGB: u32 = 0;
const BI_BITFIELDS: u32 = 3;
const BI_ALPHABITFIELDS: u32 = 6;

#[derive(Clone, Copy, Debug)]
struct Mask {
    shift: u32,
    bits: u32,
    mask: u32,
}

fn mask_of(mask: u32) -> Option<Mask> {
    if mask == 0 {
        return None;
    }
    let shift = mask.trailing_zeros();
    let bits = (mask >> shift).trailing_ones();
    Some(Mask { shift, bits, mask })
}

/// A masked channel scaled to 8 bits, rounded as `Math.round` rounds. A mask with gaps scales its bits past the
/// lowest run too, which can pass 255: the value is kept modulo 256, as a `Uint8Array` stores it.
fn channel(value: u32, mask: Option<Mask>, fallback: u8) -> u8 {
    let Some(mask) = mask else {
        return fallback;
    };
    let raw = u64::from((value & mask.mask) >> mask.shift);
    let max = (1u64 << mask.bits) - 1;
    let [low, ..] = ((2 * raw * 255 + max) / (2 * max)).to_le_bytes();
    low
}

fn le16(bytes: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(
        bytes.get(at..at.checked_add(2)?)?.try_into().ok()?,
    ))
}

fn le32(bytes: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(
        bytes.get(at..at.checked_add(4)?)?.try_into().ok()?,
    ))
}

/// Decodes a BMP file into RGBA pixels; refuses anything over `max_pixels`.
pub fn decode_bmp(bytes: &[u8], max_pixels: u64) -> Result<RgbaImage, BmpError> {
    if bytes.len() < 26 || !bytes.starts_with(b"BM") {
        return error("Not a BMP file");
    }
    let u32_at = |at: usize| le32(bytes, at).unwrap_or(0);
    let data_offset = u32_at(10) as usize;
    let header_size = u32_at(14) as usize;
    if header_size < 40 || header_size > bytes.len() - 14 {
        return error("Unsupported BMP header");
    }
    let width = u32_at(18).cast_signed();
    let raw_height = u32_at(22).cast_signed();
    let bit_count = le16(bytes, 28).unwrap_or(0);
    let compression = u32_at(30);
    let height = raw_height.unsigned_abs();
    let top_down = raw_height < 0;
    if width <= 0 || height == 0 {
        return error("Empty BMP");
    }
    let width = width.unsigned_abs();
    let pixels = u64::from(width) * u64::from(height);
    if pixels > max_pixels {
        return error(format!("BMP of {pixels} pixels is over {max_pixels}"));
    }

    let mut masks: Option<[Option<Mask>; 4]> = None;
    if compression == BI_BITFIELDS || compression == BI_ALPHABITFIELDS {
        if bit_count != 16 && bit_count != 32 {
            return error("Bit fields need 16 or 32 bits per pixel");
        }
        // Masks follow a 40-byte header, or sit inside a V4/V5 header.
        let at = 14 + 40;
        if at + 12 > bytes.len() {
            return error("Truncated BMP masks");
        }
        let alpha = if header_size >= 56 || compression == BI_ALPHABITFIELDS {
            match le32(bytes, at + 12) {
                Some(alpha) => alpha,
                None => return error("Truncated BMP masks"),
            }
        } else {
            0
        };
        masks = Some([
            mask_of(u32_at(at)),
            mask_of(u32_at(at + 4)),
            mask_of(u32_at(at + 8)),
            mask_of(alpha),
        ]);
    } else if compression != BI_RGB {
        return error(format!("BMP compression {compression} is not supported"));
    } else if bit_count == 16 {
        masks = Some([mask_of(0x7c00), mask_of(0x03e0), mask_of(0x001f), None]);
    }
    if !matches!(bit_count, 1 | 4 | 8 | 16 | 24 | 32) {
        return error(format!("{bit_count}-bit BMP is not supported"));
    }

    let mut palette: Option<&[u8]> = None;
    if bit_count <= 8 {
        let declared = u32_at(46) as usize;
        let colors = if declared == 0 {
            1 << bit_count
        } else {
            declared
        };
        let at = 14 + header_size;
        match bytes.get(at..at.saturating_add(colors.saturating_mul(4))) {
            Some(table) if colors <= 256 => palette = Some(table),
            _ => return error("Bad BMP palette"),
        }
    }

    let (width, height) = (width as usize, height as usize);
    let stride = width
        .checked_mul(usize::from(bit_count))
        .map(|bits| bits.div_ceil(32) * 4);
    let end = stride
        .and_then(|stride| stride.checked_mul(height))
        .and_then(|size| size.checked_add(data_offset));
    let (Some(stride), Some(true)) = (stride, end.map(|end| end <= bytes.len())) else {
        return error("Truncated BMP pixels");
    };
    let Some(data_len) = width.checked_mul(height).and_then(|n| n.checked_mul(4)) else {
        return error(format!("BMP of {pixels} pixels is over {max_pixels}"));
    };
    let mut data = vec![0u8; data_len];
    let bit_count = usize::from(bit_count);
    for row in 0..height {
        let source = data_offset + if top_down { row } else { height - 1 - row } * stride;
        for x in 0..width {
            let byte = |at: usize| bytes.get(source + at).copied().unwrap_or(0);
            let (r, g, b, a) = if let Some(palette) = palette {
                let bit = x * bit_count;
                let index = (usize::from(byte(bit >> 3)) >> (8 - bit_count - (bit & 7)))
                    & ((1 << bit_count) - 1);
                let entry = |c: usize| palette.get(index * 4 + c).copied();
                let (Some(b), Some(g), Some(r)) = (entry(0), entry(1), entry(2)) else {
                    return error("BMP palette index out of range");
                };
                (r, g, b, 255)
            } else if let Some([red, green, blue, alpha]) = masks {
                let value = if bit_count == 16 {
                    u32::from(le16(bytes, source + x * 2).unwrap_or(0))
                } else {
                    le32(bytes, source + x * 4).unwrap_or(0)
                };
                (
                    channel(value, red, 0),
                    channel(value, green, 0),
                    channel(value, blue, 0),
                    channel(value, alpha, 255),
                )
            } else {
                let at = x * (bit_count / 8);
                // 32-bit rows without masks leave the fourth byte unused: the image is opaque.
                (byte(at + 2), byte(at + 1), byte(at), 255)
            };
            let out = (row * width + x) * 4;
            if let Some(pixel) = data.get_mut(out..out + 4) {
                pixel.copy_from_slice(&[r, g, b, a]);
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
    fn scales_masked_channels_as_math_round_does() {
        for bits in 1..=8u32 {
            let mask = mask_of((1 << bits) - 1).unwrap();
            let max = f64::from((1u32 << bits) - 1);
            for raw in 0..(1u32 << bits) {
                let expected = (f64::from(raw) * 255.0 / max + 0.5).floor();
                assert_eq!(f64::from(channel(raw, Some(mask), 0)), expected);
            }
        }
        let full = mask_of(u32::MAX).unwrap();
        assert_eq!((full.shift, full.bits), (0, 32));
        assert_eq!(channel(u32::MAX, Some(full), 0), 255);
        let high = mask_of(0xff00_0000).unwrap();
        assert_eq!((high.shift, high.bits), (24, 8));
        assert_eq!(channel(0x8000_0000, Some(high), 0), 128);
        // 0b1011 is a run of two bits, max 3, with a bit past it: 11 * 255 / 3 = 935, kept as 935 % 256.
        let gapped = mask_of(0b1011).unwrap();
        assert_eq!((gapped.shift, gapped.bits), (0, 2));
        assert_eq!(channel(0b1011, Some(gapped), 0), 167);
        assert_eq!(channel(0b1000, Some(gapped), 0), 168);
        assert_eq!(channel(0b0011, Some(gapped), 0), 255);
    }
}
