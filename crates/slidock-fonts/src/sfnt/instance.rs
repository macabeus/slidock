//! A variable face's metrics at the weight a run draws: the default values plus their `MVAR` deltas, and for
//! TrueType outlines the glyph box (every glyph's, and À's top) drawn at that location, each point rounded as
//! `fontTools.misc.roundTools.otRound` rounds it, as an instancer writes them.
//!
//! `hasc`, `hdsc` and `hlgp` vary the `hhea` line as well as the `OS/2` typo line, as FreeType and HarfBuzz apply
//! them to the line a renderer draws with whichever table it came from; `hcla` and `hcld` vary the win metrics.

use read_fonts::TableProvider;
use read_fonts::types::Tag;
use skrifa::instance::{LocationRef, Size};
use skrifa::outline::DrawSettings;
use skrifa::outline::pen::ControlBoundsPen;
use skrifa::{GlyphId, MetadataProvider};
use slidock_ir::fonts::{FontMetrics, GlyphBounds};

use super::reader::{Directory, Reader};
use super::{A_GRAVE, SfntError, face_offset, read_face};
use crate::limits::FontLimits;

/// `x` rounded half up, as fontTools rounds coordinates.
fn ot_round(x: f64) -> i32 {
    // Font coordinates fit in i32 many times over; `as` saturates the absurd.
    (x + 0.5).floor() as i32
}

/// The metrics of face `index` at weight `weight` (user units, clamped to the `wght` axis). A face without a `wght`
/// axis, or one whose variation data cannot be read, measures as its default instance; glyph boxes keep their
/// default values for CFF outlines and for faces of more than `limits.glyphs` glyphs.
pub fn metrics_at(
    bytes: &[u8],
    index: u32,
    weight: u32,
    limits: &FontLimits,
) -> Result<FontMetrics, SfntError> {
    if bytes.len() > limits.font_bytes {
        return Err(SfntError::TooLarge(bytes.len()));
    }
    let reader = Reader::new(bytes);
    let offset = face_offset(&reader, index)?;
    let face = read_face(&reader, offset)?;
    let mut metrics = face.metrics;
    let tables = Directory::read(&reader, offset)?;
    if !tables.has(b"fvar") {
        return Ok(metrics);
    }
    let font = read_fonts::FontRef::from_index(bytes, index)
        .map_err(|error| SfntError::Unreadable(error.to_string()))?;
    let location = font.axes().location([("wght", weight as f32)]);
    let coords = location.coords();

    if let Ok(mvar) = font.mvar() {
        let delta = |tag: &[u8; 4]| -> f64 {
            mvar.metric_delta(Tag::new(tag), coords)
                .map_or(0.0, |delta| delta.to_f64())
        };
        let vary = |value: i32, tag: &[u8; 4]| ot_round(f64::from(value) + delta(tag));
        metrics.hhea.ascender = vary(metrics.hhea.ascender, b"hasc");
        metrics.hhea.descender = vary(metrics.hhea.descender, b"hdsc");
        metrics.hhea.line_gap = vary(metrics.hhea.line_gap, b"hlgp");
        metrics.typo.ascender = vary(metrics.typo.ascender, b"hasc");
        metrics.typo.descender = vary(metrics.typo.descender, b"hdsc");
        metrics.typo.line_gap = vary(metrics.typo.line_gap, b"hlgp");
        metrics.win.ascent = vary(metrics.win.ascent, b"hcla");
        metrics.win.descent = vary(metrics.win.descent, b"hcld");
    }

    let glyphs = tables
        .get(b"maxp")
        .map(|maxp| reader.u16(maxp.offset + 4))
        .transpose()?
        .unwrap_or(0);
    if tables.has(b"glyf") && u32::from(glyphs) <= limits.glyphs {
        let at = LocationRef::new(coords);
        let outlines = font.outline_glyphs();
        let top_of = |glyph: GlyphId| -> Option<(i32, i32)> {
            let outline = outlines.get(glyph)?;
            let mut pen = ControlBoundsPen::new();
            outline
                .draw(DrawSettings::unhinted(Size::unscaled(), at), &mut pen)
                .ok()?;
            let bounds = pen.bounding_box()?;
            Some((
                ot_round(f64::from(bounds.y_min)),
                ot_round(f64::from(bounds.y_max)),
            ))
        };
        let mut extent: Option<(i32, i32)> = None;
        for glyph in 0..glyphs {
            if let Some((y_min, y_max)) = top_of(GlyphId::new(u32::from(glyph))) {
                extent = Some(match extent {
                    Some((low, high)) => (low.min(y_min), high.max(y_max)),
                    None => (y_min, y_max),
                });
            }
        }
        if let Some((y_min, y_max)) = extent {
            metrics.bounds = Some(GlyphBounds { y_min, y_max });
        }
        if metrics.grave_top.is_some() {
            let grave = font.charmap().map(A_GRAVE);
            metrics.grave_top = grave.and_then(top_of).map(|(_, top)| top);
        }
    }
    Ok(metrics)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_half_up_like_fonttools() {
        assert_eq!(ot_round(0.5), 1);
        assert_eq!(ot_round(-0.5), 0);
        assert_eq!(ot_round(-1.5), -1);
        assert_eq!(ot_round(2.4999), 2);
    }
}
