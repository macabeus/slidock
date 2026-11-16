//! A PNG encoder whose bytes depend only on its input: 8-bit RGBA, the Sub filter on every row, zlib at level 6 in
//! pure Rust, one `IDAT` chunk, and no other chunks than `IHDR` and `IEND`.

use png::{BitDepth, ColorType, DeflateCompression, Encoder, Filter};
use thiserror::Error;

/// 8-bit RGBA pixels, row-major, `width * height * 4` bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum PngError {
    #[error("Invalid PNG size {width}x{height}")]
    Size { width: u32, height: u32 },
    #[error("Expected {expected} RGBA bytes")]
    Pixels { expected: u64 },
    #[error("PNG encoding failed: {0}")]
    Encode(String),
}

const LEVEL: u8 = 6;

pub fn encode_png(image: &RgbaImage) -> Result<Vec<u8>, PngError> {
    let RgbaImage {
        width,
        height,
        ref data,
    } = *image;
    if width < 1 || height < 1 {
        return Err(PngError::Size { width, height });
    }
    let expected = u64::from(width) * u64::from(height) * 4;
    if data.len() as u64 != expected {
        return Err(PngError::Pixels { expected });
    }
    let encode = |error: png::EncodingError| PngError::Encode(error.to_string());
    let mut out = Vec::new();
    let mut encoder = Encoder::new(&mut out, width, height);
    encoder.set_color(ColorType::Rgba);
    encoder.set_depth(BitDepth::Eight);
    encoder.set_filter(Filter::Sub);
    encoder.set_deflate_compression(DeflateCompression::Level(LEVEL));
    let mut writer = encoder.write_header().map_err(encode)?;
    writer.write_image_data(data).map_err(encode)?;
    writer.finish().map_err(encode)?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gradient(width: u32, height: u32) -> Vec<u8> {
        (0..width * height)
            .flat_map(|i| {
                [
                    (i % 251) as u8,
                    ((i * 7) % 256) as u8,
                    ((i * 13) % 256) as u8,
                    (255 - (i % 256)) as u8,
                ]
            })
            .collect()
    }

    /// The chunk types of a PNG, in order.
    fn chunks(png: &[u8]) -> Vec<String> {
        let mut types = Vec::new();
        let mut at = 8;
        while at + 8 <= png.len() {
            let length = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
            types.push(String::from_utf8_lossy(&png[at + 4..at + 8]).into_owned());
            at += 12 + length;
        }
        types
    }

    #[test]
    fn decodes_back_to_its_input() {
        let data = gradient(37, 11);
        let png = encode_png(&RgbaImage {
            width: 37,
            height: 11,
            data: data.clone(),
        })
        .unwrap();
        assert_eq!(chunks(&png), ["IHDR", "IDAT", "IEND"]);
        let mut reader = png::Decoder::new(std::io::Cursor::new(&png))
            .read_info()
            .unwrap();
        let mut pixels = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut pixels).unwrap();
        assert_eq!((info.width, info.height), (37, 11));
        assert_eq!(info.color_type, ColorType::Rgba);
        assert_eq!(pixels, data);
        // Every row carries the Sub filter.
        let idat_length = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        let raw = miniz_oxide::inflate::decompress_to_vec_zlib(&png[41..41 + idat_length]).unwrap();
        assert!((0..11).all(|row| raw[row * (37 * 4 + 1)] == 1));
    }

    #[test]
    fn writes_fixed_bytes_for_a_fixed_input() {
        let png = encode_png(&RgbaImage {
            width: 64,
            height: 32,
            data: gradient(64, 32),
        })
        .unwrap();
        assert_eq!(
            slidock_canon::sha256_hex(&png),
            "f511fc67cf178c52f79279e4ecfb01dda71acdf7effae79792bb58b13d6f8813"
        );
    }

    #[test]
    fn refuses_sizes_that_do_not_match_the_pixels() {
        let image = |width, height, len| RgbaImage {
            width,
            height,
            data: vec![0; len],
        };
        assert_eq!(
            encode_png(&image(2, 2, 15)),
            Err(PngError::Pixels { expected: 16 })
        );
        assert_eq!(
            encode_png(&image(0, 2, 0)),
            Err(PngError::Size {
                width: 0,
                height: 2
            })
        );
    }
}
