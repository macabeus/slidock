//! The IR passes, in pipeline order: normalize-frames, bind-placeholders, resolve-inheritance, resolve-theme,
//! cascade-text, number-lists, bind-fonts, text-metrics, lower-geometry, plan-media, normalize-timeline and a11y. Each
//! pass is one module and implements a `slidock-core-api` pass trait; [`passes`] lists them in the order the pass
//! manager runs them. [`collect_glyphs`] runs after them on the resolved presentation: it only reads.

mod a11y;
mod bind_fonts;
mod bind_placeholders;
mod cascade_text;
mod collect_glyphs;
mod container;
mod layers;
mod lower_geometry;
mod lower_gradient;
mod normalize_frames;
mod normalize_timeline;
mod number_lists;
mod plan_media;
mod resolve_inheritance;
mod resolve_theme;
mod roles;
mod shrink_fit;
mod text_metrics;
mod theme_refs;

pub use a11y::A11y;
pub use bind_fonts::{BindFonts, Bound, bind_family};
pub use bind_placeholders::BindPlaceholders;
pub use cascade_text::CascadeText;
pub use collect_glyphs::{FaceGlyphs, GlyphFace, collect_glyphs};
pub use lower_geometry::LowerGeometry;
pub use lower_gradient::{Approximation, LoweredFill, lower_fill};
pub use normalize_frames::{NormalizeFrames, normal_angle};
pub use normalize_timeline::NormalizeTimeline;
pub use number_lists::{NumberLists, number_label};
pub use plan_media::PlanMedia;
pub use resolve_inheritance::ResolveInheritance;
pub use resolve_theme::ResolveTheme;
pub use roles::{RoleClass, TextStyleClass, is_title_role, role_class, text_style_class};
pub use text_metrics::{TextMetrics, basis_line, basis_ratio};
pub use theme_refs::{FALLBACK_FONT, ThemeContext, theme_context};

use slidock_core_api::Pass;

/// The passes that read no font, in order: what `Session::open` runs.
pub fn before_fonts() -> Vec<Pass> {
    vec![
        Pass::slide(NormalizeFrames),
        Pass::presentation(BindPlaceholders),
        Pass::slide(ResolveInheritance),
        Pass::slide(ResolveTheme),
        Pass::slide(CascadeText),
        Pass::slide(NumberLists),
    ]
}

/// The passes from bind-fonts on, in order: what `Session::finish` runs again whenever the fonts change.
pub fn after_fonts() -> Vec<Pass> {
    vec![
        Pass::presentation(BindFonts),
        Pass::slide(TextMetrics),
        Pass::slide(LowerGeometry),
        Pass::presentation(PlanMedia),
        Pass::slide(NormalizeTimeline),
        Pass::slide(A11y),
    ]
}

/// Every pass, in the order the pass manager runs them (verify-source before, verify-resolved after).
pub fn passes() -> Vec<Pass> {
    let mut passes = before_fonts();
    passes.extend(after_fonts());
    passes
}
