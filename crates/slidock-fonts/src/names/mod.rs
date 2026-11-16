//! What font names spell: weight names inside family names, PostScript names, and generic families.

pub mod generic;
pub mod postscript;
pub mod weights;

pub use generic::generic_of;
pub use postscript::{PostscriptFace, postscript_face, strip_subset_tag};
pub use weights::{WeightedFamily, split_weight_name, weight_of_name};
