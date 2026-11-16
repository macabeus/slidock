//! The renderer: the pinned container (or, for the fonts lane, the same Chromium on the host), the CDP session, the
//! request router and the in-page measure.

pub mod docker;
pub mod native;
pub mod route;
pub mod session;
pub mod shadow_font;
