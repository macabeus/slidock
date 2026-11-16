//! Deterministic floating-point maths: a port of fdlibm (`sin`, `cos`, `atan2`, `exp`, `log`, `hypot`, …) and the
//! IR quantisation helpers. All transcendental maths in the workspace goes through this crate, so native and
//! wasm builds produce the same bits (clippy's `disallowed_methods` forbids the `f64` methods elsewhere).

mod fdlibm;
mod quantise;

pub use fdlibm::{acos, asin, atan, atan2, cbrt, cos, exp, hypot, log, sin, tan};
pub use quantise::{is_quantised, quantise};
pub use slidock_canon::{Grid, grid_steps};
