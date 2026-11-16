//! Images as the metrics see them: PNGs decoded to 8-bit RGBA as pngjs decodes them, composited over white, brought to
//! the metric size (960 px on the long side) by area averaging, and reduced to luma.

use std::io::Cursor;

use crate::MetricsError;
use crate::js::{js_max, js_min, js_round, to_uint8};

/// Oracle rasters and renders put this many pixels on the long side.
pub const RASTER_LONG_SIDE_PX: u32 = 1920;
/// The metrics run at half the raster size.
pub const METRIC_LONG_SIDE_PX: u32 = 960;
/// The largest decoded image `decode_png` accepts, in bytes.
const DECODE_LIMIT_BYTES: usize = 1 << 30;

/// 8-bit RGBA, rows top to bottom, at least one pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rgba {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

impl Rgba {
    pub fn new(width: usize, height: usize, data: Vec<u8>) -> Result<Self, MetricsError> {
        if width == 0 || height == 0 {
            return Err(MetricsError::Empty { width, height });
        }
        let len = data.len();
        if width.checked_mul(height).and_then(|n| n.checked_mul(4)) != Some(len) {
            return Err(MetricsError::ImageData { width, height, len });
        }
        Ok(Rgba {
            width,
            height,
            data,
        })
    }

    /// An opaque image of one grey `value`.
    pub fn blank(width: usize, height: usize, value: u8) -> Result<Self, MetricsError> {
        let Some(len) = width.checked_mul(height).and_then(|n| n.checked_mul(4)) else {
            return Err(MetricsError::ImageData {
                width,
                height,
                len: 0,
            });
        };
        let mut data = vec![value; len];
        for [_, _, _, alpha] in data.as_chunks_mut::<4>().0 {
            *alpha = 255;
        }
        Rgba::new(width, height, data)
    }

    pub fn width(&self) -> usize {
        self.width
    }

    pub fn height(&self) -> usize {
        self.height
    }

    pub fn data(&self) -> &[u8] {
        &self.data
    }

    pub fn into_data(self) -> Vec<u8> {
        self.data
    }

    pub(crate) fn check_same_size(&self, other: &Rgba) -> Result<(), MetricsError> {
        if self.width == other.width && self.height == other.height {
            Ok(())
        } else {
            Err(MetricsError::SizeMismatch {
                a_width: self.width,
                a_height: self.height,
                b_width: other.width,
                b_height: other.height,
            })
        }
    }
}

/// Decodes a PNG to 8-bit RGBA as pngjs 7 does: palettes expanded (an index past the palette is an error), a
/// `tRNS` colour of a grey or RGB image made fully transparent black, bit depths other than 8 scaled with
/// `⌊v·255/max + ½⌋` (16-bit samples rounded, not truncated), and gamma ignored.
pub fn decode_png(bytes: &[u8]) -> Result<Rgba, MetricsError> {
    let mut decoder = png::Decoder::new_with_limits(
        Cursor::new(bytes),
        png::Limits {
            bytes: DECODE_LIMIT_BYTES,
        },
    );
    decoder.set_transformations(png::Transformations::IDENTITY);
    let mut reader = decoder.read_info()?;
    let size = reader
        .output_buffer_size()
        .ok_or_else(|| MetricsError::Png("image too large".into()))?;
    let mut buffer = vec![0; size];
    let frame = reader.next_frame(&mut buffer)?;
    let info = reader.info();
    let (width, height) = (frame.width as usize, frame.height as usize);
    let depth = info.bit_depth as u8;
    let channels = match frame.color_type {
        png::ColorType::Grayscale | png::ColorType::Indexed => 1,
        png::ColorType::GrayscaleAlpha => 2,
        png::ColorType::Rgb => 3,
        png::ColorType::Rgba => 4,
    };
    let max = (1u32 << depth) - 1;
    let palette = match frame.color_type {
        png::ColorType::Indexed => Some(palette_of(info)?),
        _ => None,
    };
    let trans = match frame.color_type {
        png::ColorType::Grayscale => trans_color(raw_trns(bytes), 1)?,
        png::ColorType::Rgb => trans_color(raw_trns(bytes), 3)?,
        _ => None,
    };
    let mut data = Vec::with_capacity(width.saturating_mul(height).saturating_mul(4));
    for row in buffer.chunks(frame.line_size).take(height) {
        let mut samples = Samples {
            row,
            depth,
            next: 0,
        };
        for _ in 0..width {
            let mut raw = [0u32; 4];
            for slot in raw.iter_mut().take(channels) {
                *slot = samples.next();
            }
            let pixel = match channels {
                1 => [raw[0], raw[0], raw[0], max],
                2 => [raw[0], raw[0], raw[0], raw[1]],
                3 => [raw[0], raw[1], raw[2], max],
                _ => raw,
            };
            if let Some(palette) = &palette {
                let index = pixel[0] as usize;
                let colour = palette
                    .get(index)
                    .ok_or(MetricsError::PaletteIndex(index as u16))?;
                data.extend_from_slice(colour);
                continue;
            }
            let transparent = match &trans {
                Some(colour) if colour.len() == 1 => colour.first() == Some(&pixel[0]),
                Some(colour) => colour.get(..3) == pixel.get(..3),
                None => false,
            };
            for value in pixel {
                let value = if transparent { 0 } else { value };
                let scaled = if depth == 8 {
                    value as u8
                } else {
                    to_uint8((f64::from(value) * 255.0 / f64::from(max) + 0.5).floor())
                };
                data.push(scaled);
            }
        }
    }
    Rgba::new(width, height, data)
}

/// The palette's RGBA entries, alpha from `tRNS` (255 past its end).
fn palette_of(info: &png::Info<'_>) -> Result<Vec<[u8; 4]>, MetricsError> {
    let rgb = info
        .palette
        .as_deref()
        .ok_or_else(|| MetricsError::Png("Expected palette not found".into()))?;
    let mut palette: Vec<[u8; 4]> = rgb
        .as_chunks::<3>()
        .0
        .iter()
        .map(|&[r, g, b]| [r, g, b, 255])
        .collect();
    if let Some(alphas) = info.trns.as_deref() {
        if alphas.len() > palette.len() {
            return Err(MetricsError::Png(
                "More transparent colors than palette size".into(),
            ));
        }
        for (entry, &alpha) in palette.iter_mut().zip(alphas) {
            entry[3] = alpha;
        }
    }
    Ok(palette)
}

/// The data of the `tRNS` chunk before the first `IDAT`, as stored: the decoder keeps only the low bytes of an
/// 8-bit image's 16-bit transparent colour, where pngjs compares all 16 bits.
fn raw_trns(bytes: &[u8]) -> Option<&[u8]> {
    let mut at = 8;
    loop {
        let len = u32::from_be_bytes(bytes.get(at..at + 4)?.try_into().ok()?) as usize;
        let kind = bytes.get(at + 4..at + 8)?;
        if kind == b"IDAT" {
            return None;
        }
        let data = bytes.get(at + 8..(at + 8).checked_add(len)?)?;
        if kind == b"tRNS" {
            return Some(data);
        }
        at += 12 + len;
    }
}

/// A grey or RGB image's transparent colour: big-endian 16-bit samples, compared with the raw samples.
fn trans_color(trns: Option<&[u8]>, samples: usize) -> Result<Option<Vec<u32>>, MetricsError> {
    let Some(bytes) = trns else { return Ok(None) };
    let words = (0..samples)
        .map(|i| match (bytes.get(2 * i), bytes.get(2 * i + 1)) {
            (Some(&high), Some(&low)) => Ok(u32::from(high) << 8 | u32::from(low)),
            _ => Err(MetricsError::Png("tRNS chunk too short".into())),
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Some(words))
}

/// The samples of one row, most significant bits first.
struct Samples<'a> {
    row: &'a [u8],
    depth: u8,
    /// The next sample's bit offset.
    next: usize,
}

impl Samples<'_> {
    fn next(&mut self) -> u32 {
        let bit = self.next;
        self.next += usize::from(self.depth);
        let byte = |i: usize| u32::from(self.row.get(i).copied().unwrap_or(0));
        match self.depth {
            16 => byte(bit / 8) << 8 | byte(bit / 8 + 1),
            8 => byte(bit / 8),
            depth => {
                let shift = 8 - usize::from(depth) - bit % 8;
                (byte(bit / 8) >> shift) & ((1 << depth) - 1)
            }
        }
    }
}

/// The image over a white background, fully opaque.
pub fn over_white(image: &Rgba) -> Rgba {
    let mut data = Vec::with_capacity(image.data.len());
    for &[r, g, b, a] in image.data.as_chunks::<4>().0 {
        let alpha = f64::from(a) / 255.0;
        for c in [r, g, b] {
            data.push(to_uint8(js_round(
                f64::from(c) * alpha + 255.0 * (1.0 - alpha),
            )));
        }
        data.push(255);
    }
    Rgba {
        width: image.width,
        height: image.height,
        data,
    }
}

/// The metric size of a raster: 960 px on the long side, aspect kept, at least one pixel.
pub fn metric_size(width: usize, height: usize) -> (usize, usize) {
    let (w, h) = (width as f64, height as f64);
    let scale = f64::from(METRIC_LONG_SIDE_PX) / js_max(w, h);
    let side = |v: f64| js_max(1.0, js_round(v * scale)) as usize;
    (side(w), side(h))
}

/// A page or slide of `w × h` points at the raster size: `(width, height, pixels per point)`.
pub fn raster_size(w: f64, h: f64) -> (f64, f64, f64) {
    let px_per_pt = f64::from(RASTER_LONG_SIDE_PX) / js_max(w, h);
    (js_round(w * px_per_pt), js_round(h * px_per_pt), px_per_pt)
}

/// Box (area-average) resampling: each target pixel averages the source area it covers, weighted by overlap.
pub fn resample_area(image: &Rgba, width: usize, height: usize) -> Result<Rgba, MetricsError> {
    if image.width == width && image.height == height {
        return Ok(image.clone());
    }
    let mut out = Vec::with_capacity(width.saturating_mul(height).saturating_mul(4));
    let sx = image.width as f64 / width as f64;
    let sy = image.height as f64 / height as f64;
    let (source_w, source_h) = (image.width as f64, image.height as f64);
    let stride = image.width * 4;
    for ty in 0..height {
        let y0 = ty as f64 * sy;
        let y1 = y0 + sy;
        for tx in 0..width {
            let x0 = tx as f64 * sx;
            let x1 = x0 + sx;
            let mut sum = [0.0f64; 4];
            let mut total = 0.0;
            let (y_start, y_end) = (y0.floor() as usize, js_min(y1.ceil(), source_h) as usize);
            let (x_start, x_end) = (x0.floor() as usize, js_min(x1.ceil(), source_w) as usize);
            for y in y_start..y_end {
                let yf = y as f64;
                let wy = js_min(yf + 1.0, y1) - js_max(yf, y0);
                let row = image
                    .data
                    .get(y * stride..(y + 1) * stride)
                    .unwrap_or_default();
                for x in x_start..x_end {
                    let xf = x as f64;
                    let w = wy * (js_min(xf + 1.0, x1) - js_max(xf, x0));
                    if let Some(pixel) = row.get(x * 4..x * 4 + 4) {
                        for (s, &c) in sum.iter_mut().zip(pixel) {
                            *s += f64::from(c) * w;
                        }
                    }
                    total += w;
                }
            }
            out.extend(sum.iter().map(|s| to_uint8(js_round(s / total))));
        }
    }
    Rgba::new(width, height, out)
}

/// Both images over white at the metric size of the first (the oracle).
pub fn metric_pair(oracle: &Rgba, render: &Rgba) -> Result<(Rgba, Rgba), MetricsError> {
    let (width, height) = metric_size(oracle.width, oracle.height);
    Ok((
        resample_area(&over_white(oracle), width, height)?,
        resample_area(&over_white(render), width, height)?,
    ))
}

/// Rec. 709 luma of the sRGB values (no linearisation), one `f32` per pixel.
pub fn luma(image: &Rgba) -> Vec<f32> {
    image
        .data
        .as_chunks::<4>()
        .0
        .iter()
        .map(|&[r, g, b, _]| {
            (0.2126 * f64::from(r) + 0.7152 * f64::from(g) + 0.0722 * f64::from(b)) as f32
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn encode(
        width: u32,
        height: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        data: &[u8],
    ) -> Vec<u8> {
        encode_with(width, height, color, depth, data, |_| {})
    }

    fn encode_with(
        width: u32,
        height: u32,
        color: png::ColorType,
        depth: png::BitDepth,
        data: &[u8],
        setup: impl FnOnce(&mut png::Encoder<'_, &mut Vec<u8>>),
    ) -> Vec<u8> {
        let mut out = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut out, width, height);
            encoder.set_color(color);
            encoder.set_depth(depth);
            setup(&mut encoder);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(data).unwrap();
        }
        out
    }

    #[test]
    fn decodes_rgb_and_rgba() {
        let rgb = encode(
            2,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[1, 2, 3, 4, 5, 6],
        );
        assert_eq!(
            decode_png(&rgb).unwrap().data(),
            &[1, 2, 3, 255, 4, 5, 6, 255]
        );
        let rgba = encode(
            1,
            1,
            png::ColorType::Rgba,
            png::BitDepth::Eight,
            &[9, 8, 7, 6],
        );
        assert_eq!(decode_png(&rgba).unwrap().data(), &[9, 8, 7, 6]);
    }

    #[test]
    fn sixteen_bit_samples_round_to_eight() {
        // 0x00FF: ⌊255·255/65535 + ½⌋ = 1, where keeping the high byte gives 0.
        let data = [0x00, 0xFF, 0x80, 0x00, 0xFF, 0xFF];
        let png = encode(1, 1, png::ColorType::Rgb, png::BitDepth::Sixteen, &data);
        assert_eq!(decode_png(&png).unwrap().data(), &[1, 128, 255, 255]);
    }

    #[test]
    fn low_bit_grey_scales_and_keeps_alpha_opaque() {
        // Two 2-bit pixels: 1 and 3.
        let png = encode(
            2,
            1,
            png::ColorType::Grayscale,
            png::BitDepth::Two,
            &[0b0111_0000],
        );
        assert_eq!(
            decode_png(&png).unwrap().data(),
            &[85, 85, 85, 255, 255, 255, 255, 255]
        );
    }

    #[test]
    fn palette_with_transparency() {
        let png = encode_with(
            2,
            1,
            png::ColorType::Indexed,
            png::BitDepth::Eight,
            &[1, 0],
            |e| {
                e.set_palette(vec![10, 20, 30, 40, 50, 60]);
                e.set_trns(vec![7]);
            },
        );
        assert_eq!(
            decode_png(&png).unwrap().data(),
            &[40, 50, 60, 255, 10, 20, 30, 7]
        );
    }

    #[test]
    fn palette_index_out_of_range_is_an_error() {
        let png = encode_with(
            1,
            1,
            png::ColorType::Indexed,
            png::BitDepth::Eight,
            &[2],
            |e| {
                e.set_palette(vec![10, 20, 30, 40, 50, 60]);
            },
        );
        assert_eq!(decode_png(&png), Err(MetricsError::PaletteIndex(2)));
    }

    #[test]
    fn transparent_colour_becomes_transparent_black() {
        let png = encode_with(
            2,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[1, 2, 3, 4, 5, 6],
            |e| {
                e.set_trns(vec![0, 4, 0, 5, 0, 6]);
            },
        );
        assert_eq!(
            decode_png(&png).unwrap().data(),
            &[1, 2, 3, 255, 0, 0, 0, 0]
        );
    }

    #[test]
    fn transparent_colour_compares_all_sixteen_bits() {
        // 0x0104 is not the 8-bit sample 4: the pixel stays opaque, though the decoder keeps only the low byte.
        let png = encode_with(
            1,
            1,
            png::ColorType::Rgb,
            png::BitDepth::Eight,
            &[4, 5, 6],
            |e| {
                e.set_trns(vec![1, 4, 0, 5, 0, 6]);
            },
        );
        assert_eq!(decode_png(&png).unwrap().data(), &[4, 5, 6, 255]);
    }

    #[test]
    fn over_white_rounds_each_channel() {
        let image = Rgba::new(1, 1, vec![0, 100, 255, 128]).unwrap();
        // 100·128/255 + 255·127/255 = 177.196…; 0 → 127.0; 255 stays 255.
        assert_eq!(over_white(&image).data(), &[127, 177, 255, 255]);
    }

    #[test]
    fn metric_size_halves_the_raster() {
        assert_eq!(metric_size(1920, 1080), (960, 540));
        assert_eq!(metric_size(1080, 1920), (540, 960));
        assert_eq!(metric_size(1920, 1), (960, 1));
        assert_eq!(metric_size(1919, 1440), (960, 720));
    }

    #[test]
    fn raster_size_of_a_slide() {
        assert_eq!(raster_size(720.0, 405.0), (1920.0, 1080.0, 1920.0 / 720.0));
    }

    #[test]
    fn resample_averages_by_area() {
        // 3 → 2 px: the middle source pixel is split half and half.
        let image = Rgba::new(
            3,
            1,
            vec![0, 0, 0, 255, 90, 90, 90, 255, 255, 255, 255, 255],
        )
        .unwrap();
        let out = resample_area(&image, 2, 1).unwrap();
        // (0·1 + 90·½)/1.5 = 30; (90·½ + 255·1)/1.5 = 200.
        assert_eq!(out.data(), &[30, 30, 30, 255, 200, 200, 200, 255]);
        assert_eq!(resample_area(&image, 3, 1).unwrap(), image);
    }

    #[test]
    fn luma_is_stored_as_f32() {
        let image = Rgba::new(1, 1, vec![10, 20, 30, 255]).unwrap();
        let want = (0.2126 * 10.0 + 0.7152 * 20.0 + 0.0722 * 30.0) as f32;
        assert_eq!(luma(&image), vec![want]);
    }

    #[test]
    fn rgba_rejects_bad_sizes() {
        assert!(matches!(
            Rgba::new(0, 1, vec![]),
            Err(MetricsError::Empty { .. })
        ));
        assert!(matches!(
            Rgba::new(1, 1, vec![0; 3]),
            Err(MetricsError::ImageData { .. })
        ));
        assert_eq!(Rgba::blank(1, 1, 7).unwrap().data(), &[7, 7, 7, 255]);
    }
}
