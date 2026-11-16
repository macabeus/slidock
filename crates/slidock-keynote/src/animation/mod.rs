//! Builds, transitions and Magic Move. The steps' and transitions' IR fields are the
//! parity path's; the native data beside them (the effect vocabulary, timing and match keys) is what the deck path
//! plays, and the parity path leaves it out. Keynote's effect ids map to the vocabulary through `slidock-presets`
//! ([`effects`]); the build chunks become steps ([`timeline`]); a slide's transition keeps its id and, on the deck
//! path, moves to the slide it plays into ([`transition`]);
//! the slides around a Magic Move key their elements ([`magic_move`]).
//!
//! Every field of `KN.TransitionAttributesArchive`, `KN.AnimationAttributesArchive`, `KN.BuildAttributesArchive` and
//! `KN.BuildChunkArchive` is read: it plays, or is reported (KN1404–KN1409) when it cannot. These are read and
//! deliberately change nothing:
//! - `BuildChunkArchive.build_id`: the build's UUID beside the `build` reference, which names the same build;
//! - `BuildAttributesArchive.ChartRotation3D`: a chart build's option (Keynote writes 60 on every build); charts
//!   lower as foreign objects and chart builds play their fallback (KN1402);
//! - `custom_motion_blur` (both archives): a rendering nicety, not a loss;
//! - `custom_effect_timing_curve_theme_name_1..3`: the editor's names for the stored curves, which are what plays;
//! - on a build, the animation attributes' `delay` and `is_automatic` (the chunk's own play, as the IR reads them) and
//!   `color` (a fade-through-colour transition's);
//! - the legacy `database_*` copies when the animation attributes are present (they win); a build or transition
//!   stored only in the legacy fields plays its fallback or is left out, reported (KN1404).

pub mod effects;
pub mod magic_move;
pub mod motion_path;
pub mod timeline;
pub mod transition;

pub use magic_move::{assign_keys, is_magic_move, shared_keys};
pub use timeline::timeline;
pub use transition::{Placement, placed_transition, transition, transition_effect};
