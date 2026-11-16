//! The WebAssembly entry point, run inside a dedicated Web Worker (`packages/wasm/src/worker.ts`): the `Session` API
//! over a JavaScript range reader backed by `FileReaderSync`, one stage per call, and the zip handed out in chunks.
//!
//! - [`host`] is the worker's state machine in plain Rust, which native tests drive;
//! - [`Compiler`] is its wasm-bindgen surface;
//! - [`protocol`] is the messages between the app and the worker.

mod bindings;
pub mod host;
pub mod protocol;
pub mod results;

pub use bindings::{Compiler, RangeSource};
use wasm_bindgen::prelude::wasm_bindgen;

// The wasm build leaves the IR input out: there an IR document is an input no frontend reads (IR0902).
#[cfg(target_arch = "wasm32")]
const _: () = assert!(
    !slidock::IR_INPUT,
    "the wasm build links the IR input (slidock's `ir-input` feature)"
);

/// The version of this crate, as built.
#[wasm_bindgen]
pub fn version() -> String {
    env!("CARGO_PKG_VERSION").to_owned()
}

/// The module's linear memory in bytes (0 outside WebAssembly). Linear memory only grows, so this is the most the
/// compiler has held since the module loaded.
#[wasm_bindgen(js_name = memoryBytes)]
pub fn memory_bytes() -> f64 {
    #[cfg(target_arch = "wasm32")]
    let pages = core::arch::wasm32::memory_size::<0>();
    #[cfg(not(target_arch = "wasm32"))]
    let pages = 0_usize;
    f64::from(u32::try_from(pages).unwrap_or(u32::MAX)) * 65_536.0
}

/// Runs when the module is instantiated: debug builds print a panic's message and location to the console before
/// the module traps.
#[wasm_bindgen(start)]
pub fn start() {
    #[cfg(debug_assertions)]
    console_error_panic_hook::set_once();
}

#[cfg(test)]
mod tests {
    #[test]
    fn version_is_the_crate_version() {
        assert_eq!(super::version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn a_native_build_has_no_linear_memory() {
        assert_eq!(super::memory_bytes(), 0.0);
    }
}
