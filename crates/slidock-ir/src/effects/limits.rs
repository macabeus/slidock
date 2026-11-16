//! Bounds on motion data, in two groups. The readers' bounds hold the untrusted timing trees (PPTX `p:timing`, ODP
//! `anim:*`, Keynote builds): a reader stops at them, failing closed with a diagnostic, before anything reaches the
//! IR. The validator does not check them: the IR's timeline has no bound of its own, so the parity path refuses nothing
//! it accepted. The effects' bounds, from [`TRACKS`] on, are the validator's.

/// Timeline steps a reader lowers on one slide.
pub const STEPS_PER_SLIDE: usize = 2_000;

/// Nodes a reader visits in one slide's timing tree before it gives up on the rest.
pub const TIMING_NODES: usize = 20_000;

/// Nesting depth of a timing tree.
pub const TIMING_DEPTH: usize = 32;

/// Bytes of one formula (`fmla`, an ODF value expression).
pub const FORMULA_BYTES: usize = 1_024;

/// Nesting depth of one formula's parse tree.
pub const FORMULA_DEPTH: usize = 32;

/// Keyframe tracks of one tier-2 effect (the validator's bounds from here on).
pub const TRACKS: usize = 32;

/// Keyframes of one track.
pub const KEYFRAMES: usize = 64;

/// Points of one time filter (PPTX `tmFilter`, a LibreOffice leaf's `smil:keySplines`); a reader drops a longer
/// one.
pub const TIME_FILTER_POINTS: usize = 64;

/// Segments of one motion path.
pub const PATH_SEGMENTS: usize = 4_096;

/// The most times a step repeats (`repeat.count`), and the most flashes of a blink.
pub const REPEAT_COUNT: u32 = 1_000;

/// Spokes of a wheel.
pub const SPOKES: u8 = 8;

/// The largest spin, in degrees (either way): ten turns.
pub const SPIN_DEGREES: f64 = 3_600.0;

/// The largest scale factor of an emphasis.
pub const SCALE: f64 = 100.0;

/// The largest blur, in slide pixels.
pub const BLUR_PX: f64 = 1_000.0;

/// The largest hue turn of a colour shift, in degrees (either way).
pub const HUE_SHIFT_DEGREES: f64 = 360.0;

/// The largest bezier control-point ordinate (either way).
pub const BEZIER_Y: f64 = 10.0;
