//! WOFF2 encoding with `ttf2woff2`: no `glyf`/`loca` transform (some decoders reject transformed output whose
//! glyph offsets overflow a short `loca`), one thread, and a quality fixed by the input size, so the same face
//! always encodes to the same bytes, natively and in wasm.

use std::panic::{AssertUnwindSafe, catch_unwind};

use ttf2woff2::{BrotliQuality, EncodeOptions};

use crate::limits::FontLimits;
use crate::sfnt::read_sfnt;

/// Inputs at least this large encode at brotli quality 9 instead of 11.
const QUALITY_11_BELOW: usize = 256 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum EncodeError {
    #[error("The font file is {0} bytes, over the limit")]
    TooLarge(usize),
    #[error("Only a single TrueType or OpenType font encodes: {0}")]
    NotSfnt(String),
    #[error("The WOFF2 encoder failed: {0}")]
    Failed(String),
    #[error("The WOFF2 encoder stopped on a malformed font")]
    Panicked,
}

/// The brotli quality for an input of `len` bytes.
pub fn quality_for(len: usize) -> u8 {
    if len < QUALITY_11_BELOW { 11 } else { 9 }
}

/// A single TrueType or OpenType font as WOFF2.
pub fn encode_woff2(sfnt: &[u8], limits: &FontLimits) -> Result<Vec<u8>, EncodeError> {
    if sfnt.len() > limits.font_bytes {
        return Err(EncodeError::TooLarge(sfnt.len()));
    }
    // The encoder slices tables by their directory entries: every one must lie inside the file, and the file must
    // be one face.
    match read_sfnt(sfnt) {
        Ok(faces) if faces.len() == 1 && sfnt.get(..4) != Some(b"ttcf") => {}
        Ok(_) => return Err(EncodeError::NotSfnt("a collection".to_owned())),
        Err(error) => return Err(EncodeError::NotSfnt(error.to_string())),
    }
    let options = EncodeOptions {
        quality: BrotliQuality::from(quality_for(sfnt.len())),
        transform_glyf_loca: false,
        threads: None,
    };
    catch_unwind(AssertUnwindSafe(|| {
        ttf2woff2::encode_with_options(sfnt, options)
    }))
    .map_err(|_| EncodeError::Panicked)?
    .map_err(|error| EncodeError::Failed(error.to_string()))
}
