//! A face's file kept to measure its glyph advances: the horizontal metrics (`hmtx`, and `HVAR` at a variable
//! face's instance) of the glyph `cmap` maps each character to, unshaped (no kerning, ligatures or fallback).

use std::fmt;
use std::sync::Arc;

use read_fonts::TableProvider;
use skrifa::instance::Size;
use skrifa::{FontRef, MetadataProvider};

/// Face `index` of a font file.
#[derive(Clone)]
pub struct FaceFile {
    bytes: Arc<[u8]>,
    index: u32,
}

impl FaceFile {
    pub fn new(bytes: Arc<[u8]>, index: u32) -> FaceFile {
        FaceFile { bytes, index }
    }

    /// The advance of each character of `text` in ems, at weight `weight` on a `wght` axis (clamped to it; a face
    /// without one ignores it), `None` for a character the face maps to no glyph; `None` when the face does not read.
    pub fn advances(&self, text: &str, weight: u32) -> Option<Vec<Option<f64>>> {
        let font = FontRef::from_index(&self.bytes, self.index).ok()?;
        let upm = f64::from(font.head().ok()?.units_per_em());
        if upm <= 0.0 {
            return None;
        }
        let location = font.axes().location([("wght", weight as f32)]);
        let metrics = font.glyph_metrics(Size::unscaled(), &location);
        let charmap = font.charmap();
        Some(
            text.chars()
                .map(|ch| {
                    let glyph = charmap.map(ch)?;
                    metrics
                        .advance_width(glyph)
                        .map(|advance| f64::from(advance) / upm)
                })
                .collect(),
        )
    }
}

impl PartialEq for FaceFile {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
            && (Arc::ptr_eq(&self.bytes, &other.bytes) || self.bytes == other.bytes)
    }
}

impl Eq for FaceFile {}

impl fmt::Debug for FaceFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FaceFile")
            .field("bytes", &self.bytes.len())
            .field("index", &self.index)
            .finish()
    }
}
