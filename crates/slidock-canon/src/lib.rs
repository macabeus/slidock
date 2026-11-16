//! Canonical JSON printing and the JavaScript number semantics the compiler's outputs depend on: ECMAScript
//! `Number::toString` (via `ryu-js`), `Number(text)`, `Math.round`, `toFixed`, UTF-16 length and slicing helpers, key
//! ordering by UTF-16 code units, and SHA-256 hex digests. Every byte-exact output in slidock goes through this crate.

mod error;
mod fixed;
mod hash;
mod json;
mod number;
mod to_number;
pub mod utf16;
mod whitespace;

pub use error::{JsonError, NumberError};
pub use hash::{Sha256Hex, sha256_canonical, sha256_hex};
pub use json::{Json, Layout, to_canonical_json, value_to_canonical_json, write_json_string};
pub use number::{
    EXACT_STEPS, Grid, MAX_FIXED_DIGITS, format_grid, format_number, format_steps, grid_steps,
    js_round, to_fixed, to_js_string,
};
pub use to_number::js_number;
pub use whitespace::{is_js_whitespace, js_trim, js_trim_end, js_trim_start};
