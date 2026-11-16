//! Binding requested fonts to catalog faces.

pub mod css_match;
pub mod synthesis;

pub use css_match::{
    FaceDescriptor, NORMAL_WIDTH, StretchRank, WeightRange, css_match, stretch_rank,
};
pub use synthesis::{Synthesis, synthesis};
