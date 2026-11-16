//! The bench's image and text metrics (`ssim@1`, `color@1`, `pixel@1`, `text@2`, `shadow@1`, `gradient@1`) and its
//! classifier (`classifier@2`), each version pinned bit for bit by its vectors (`tests/data/vectors.json`) and its
//! corpus goldens, with no I/O: images come in as PNG bytes or RGBA buffers, words as boxes in slide points.
//!
//! The arithmetic keeps one order of operations, with JavaScript's semantics where they differ from Rust's
//! (`Math.round`, `Math.max`, typed-array stores and sorts, `f32` storage of luma and SSIM windows). The elementary
//! functions come from a [`Libm`]: [`HostLibm`] gives the pinned numbers in the last bit on macOS arm64, [`Fdlibm`]
//! is portable.

#![forbid(unsafe_code)]

mod classify;
mod color;
mod error;
mod gradient;
mod image;
mod js;
mod libm;
mod pixel;
mod score;
mod shadow;
mod sort;
mod ssim;
mod text;

pub use classify::{
    CLASSIFIER_VERSION, CLOSE, Classification, CloseThresholds, CompileFailure, Decline,
    FailedReason, MATCH, MatchThresholds, Outcome, RenderErrorReason, RowFacts, ScoredClass,
    Scores, SsimSummary, classify, deciding_decline, scored_class, unscored_class,
};
pub use color::{
    COLOR_VERSION, ColorScore, Lab, TileLab, color_difference, delta_e00, region_lab, srgb_to_lab,
    tile_lab,
};
pub use error::MetricsError;
pub use gradient::{
    GRADIENT_VERSION, GradientScore, GradientTarget, REGION_INSET_PT, Region, gradient_regions,
    gradient_score,
};
pub use image::{
    METRIC_LONG_SIDE_PX, RASTER_LONG_SIDE_PX, Rgba, decode_png, luma, metric_pair, metric_size,
    over_white, raster_size, resample_area,
};
pub use js::{js_round, median, percentile, quantise};
#[cfg(feature = "host-libm")]
pub use libm::HostLibm;
pub use libm::{Fdlibm, Libm};
pub use pixel::{PIXEL_VERSION, PixelmatchOptions, pixel_score, pixelmatch};
pub use score::{
    OracleBlock, OracleLine, OracleText, OracleWord, Size, oracle_words, quantise_scores,
    score_render,
};
pub use shadow::{
    NOT_DRAWN_RING_PT, RING_PAD_PT, RING_SIGMAS, Ring, SHADOW_VERSION, ShadowExtent, ShadowScore,
    ShadowTarget, shadow_rings, shadow_score,
};
pub use ssim::{SSIM_VERSION, SsimMap, SsimScore, ssim};
pub use text::{
    Alignment, INK_DIFFERENCE, PairMeasure, Rect, TEXT_VERSION, TextScore, TextWord, align_words,
    lcs_pairs, line_breaks, normalise_word, pair_measures, text_ink, text_score,
};
