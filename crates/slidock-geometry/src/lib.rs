//! Shape geometry: the DrawingML guide-formula evaluator, `arcTo` as cubics, the preset shape table (compiled from
//! `data/presets.json` by `build.rs`), path tracing and line-end arrowheads. Coordinates are unquantised `f64`s;
//! lower-geometry and the frontends quantise at their boundary. All trigonometry goes through `slidock-math`.

pub mod arc;
mod builtins;
pub mod formula;
pub mod line_ends;
mod literal;
mod op;
pub mod path;
pub mod presets;

pub use arc::{Arc, Cubic, Point, arc_to};
pub use builtins::BUILTIN_NAMES;
pub use formula::{FormulaError, Guides, builtin_values, js_max, js_min};
pub use line_ends::{StrokeEnds, WithLineEnds, with_line_ends};
pub use literal::parse_literal;
pub use op::Op;
pub use path::{PathBuilder, Segment, ShapePath, map_segment, scale_segment};
pub use presets::{adjust_names, defined_names, preset_paths};
