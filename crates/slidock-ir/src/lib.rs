//! The presentation IR, version 1 (`$schema` `urn:slidock:presentation-ir:1`): source and resolved phase types with
//! serde, units and quantisation, ids, the diagnostics registry, the verifier and its invariants, and the feature
//! census.

pub(crate) mod de;

pub mod canonical;
pub mod color;
pub mod diagnostics;
pub mod effects;
pub mod elements;
pub mod features;
pub mod fixture;
pub mod fonts;
pub mod formats;
pub mod frame;
pub mod geometry;
pub mod ids;
pub mod json_value;
pub mod media;
pub mod origin;
pub mod paint;
pub mod parse;
pub mod path;
pub mod presentation;
pub mod presets;
pub mod resolved;
pub mod shadow;
pub mod source;
pub mod table;
pub mod text;
pub mod timeline;
pub mod units;
pub mod validate;
pub mod verify;

pub use de::{Nullable, double_option, nullable};
