use thiserror::Error;

/// Why a metric could not be computed. The bench classes such a row `render-error/render-crash`.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum MetricsError {
    #[error("png: {0}")]
    Png(String),
    #[error("index {0} not in palette")]
    PaletteIndex(u16),
    #[error("an image needs at least one pixel, not {width}×{height}")]
    Empty { width: usize, height: usize },
    #[error("{len} bytes of image data do not make {width}×{height} RGBA pixels")]
    ImageData {
        width: usize,
        height: usize,
        len: usize,
    },
    #[error("images of different sizes: {a_width}×{a_height} and {b_width}×{b_height}")]
    SizeMismatch {
        a_width: usize,
        a_height: usize,
        b_width: usize,
        b_height: usize,
    },
    #[error("image buffers of {a} and {b} bytes")]
    LengthMismatch { a: usize, b: usize },
    #[error("a matched line has more oracle words than DOM words")]
    LineWords,
    #[error("word pair ({oracle}, {dom}) is out of range")]
    PairIndex { oracle: usize, dom: usize },
}

impl From<png::DecodingError> for MetricsError {
    fn from(error: png::DecodingError) -> Self {
        MetricsError::Png(error.to_string())
    }
}
