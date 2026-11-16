//! TIFF and BMP media as PNG, through the deterministic encoder: the `transcode` action of a media plan, the same
//! bytes on every host.

use thiserror::Error;

use crate::bmp::{BmpError, decode_bmp};
use crate::png::{PngError, encode_png};
use crate::sniff::{Mime, sniff_mime};
use crate::tiff::{TiffError, decode_tiff};

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum TranscodeError {
    #[error(transparent)]
    Tiff(#[from] TiffError),
    #[error(transparent)]
    Bmp(#[from] BmpError),
    #[error(transparent)]
    Png(#[from] PngError),
}

/// A TIFF or BMP file as PNG bytes; refuses images over `max_pixels`.
pub fn transcode_to_png(bytes: &[u8], max_pixels: u64) -> Result<Vec<u8>, TranscodeError> {
    let image = match sniff_mime(bytes.get(..64).unwrap_or(bytes)) {
        Some(Mime::Bmp) => decode_bmp(bytes, max_pixels)?,
        _ => decode_tiff(bytes, max_pixels)?,
    };
    Ok(encode_png(&image)?)
}
