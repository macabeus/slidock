//! What a host needs to ship a deck's fonts: the passes' collect-glyphs as the session's glyph
//! collector, the session options that read the input's embedded fonts and ship the faces drawn, and the shipped
//! faces as the bundle writes them. The CLI and the wasm host share these, so they ship the same bytes.

use slidock_bundle::{ServedFace, ShippedFace};
use slidock_core::shipping::{self, CollectGlyphs, GlyphUse};
use slidock_core::{FrontendRegistry, Pipeline, SessionOptions};
use slidock_fonts::FontCatalog;
use slidock_ir::resolved;

/// `slidock_passes::collect_glyphs`.
#[derive(Clone, Copy, Debug, Default)]
pub struct PassGlyphs;

impl CollectGlyphs for PassGlyphs {
    fn collect(
        &self,
        presentation: &resolved::Presentation,
        catalog: &FontCatalog,
    ) -> Result<Vec<GlyphUse>, String> {
        let faces = slidock_passes::collect_glyphs(presentation, catalog)
            .map_err(|error| error.to_string())?;
        Ok(faces
            .into_iter()
            .map(|(face, glyphs)| GlyphUse {
                family: face.family,
                postscript_name: face.postscript_name,
                weight: face.weight,
                italic: face.italic,
                sha256: face.sha256,
                codepoints: glyphs.codepoints,
                weights: glyphs.weights,
                small_caps: glyphs.small_caps,
                requested: glyphs.requested,
                forced: glyphs.forced,
            })
            .collect())
    }
}

/// The session options of a deck that takes fonts, off the parity path: the input's embedded fonts are read,
/// `finish` ships the faces the deck draws, and the native motion data is kept. Every other option is
/// [`SessionOptions::new`]'s.
pub fn with_fonts<'a>(
    registry: &'a FrontendRegistry,
    pipeline: &'a Pipeline,
) -> SessionOptions<'a> {
    SessionOptions {
        embedded_fonts: true,
        glyphs: Some(&PassGlyphs),
        native_motion: true,
        ..SessionOptions::new(registry, pipeline)
    }
}

/// The faces a session shipped, as the bundle writes them.
pub fn bundled_faces(faces: &[shipping::ShippedFace]) -> Vec<ShippedFace> {
    faces
        .iter()
        .map(|face| ShippedFace {
            rules: face.rules.clone(),
            licence: face.licence.clone(),
            serves: face
                .serves
                .iter()
                .map(|served| ServedFace {
                    family: served.family.clone(),
                    weight: served.weight,
                    style: served.style,
                })
                .collect(),
        })
        .collect()
}
