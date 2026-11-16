//! Oracles: PDF copies, page sizes, rasters and words through poppler in the renderer
//! container, Keynote's attended export, its font check, and the per-run resolution of pages.

pub mod fonts_check;
pub mod keynote;
pub mod pdf;
pub mod raster;
pub mod resolve;
pub mod text_boxes;
