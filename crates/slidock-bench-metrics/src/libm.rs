//! The elementary functions `color@1` and `text@2` call. Their pinned numbers come from JavaScript's `Math` on macOS,
//! which is the platform libm: [`HostLibm`] (feature `host-libm`) is that libm, and reproduces them bit for bit on
//! macOS arm64. [`Fdlibm`] is slidock-math's fdlibm, the same bits on every target (wasm included); it agrees with
//! Apple's libm to within an ulp, so its scores match the pinned ones on the results grid but not always in the last
//! bit. `sqrt` is IEEE-exact everywhere and needs no choice.

/// The transcendental functions of JavaScript's `Math` the metrics use.
pub trait Libm {
    fn sin(x: f64) -> f64;
    fn cos(x: f64) -> f64;
    fn exp(x: f64) -> f64;
    fn atan2(y: f64, x: f64) -> f64;
    fn cbrt(x: f64) -> f64;
    /// Two-argument `Math.hypot`.
    fn hypot(x: f64, y: f64) -> f64;
}

/// slidock-math's fdlibm: identical results on every target.
#[derive(Clone, Copy, Debug)]
pub enum Fdlibm {}

impl Libm for Fdlibm {
    fn sin(x: f64) -> f64 {
        slidock_math::sin(x)
    }
    fn cos(x: f64) -> f64 {
        slidock_math::cos(x)
    }
    fn exp(x: f64) -> f64 {
        slidock_math::exp(x)
    }
    fn atan2(y: f64, x: f64) -> f64 {
        slidock_math::atan2(y, x)
    }
    fn cbrt(x: f64) -> f64 {
        slidock_math::cbrt(x)
    }
    fn hypot(x: f64, y: f64) -> f64 {
        slidock_math::hypot(x, y)
    }
}

/// The platform libm (through `std`): what JavaScriptCore's `Math` returns on the same host. Each function stays a
/// call of its own: inlined, `sin` and `cos` of one argument fuse into `sincos`, whose `sin` differs from `sin` in
/// the last bit for some arguments.
#[cfg(feature = "host-libm")]
#[derive(Clone, Copy, Debug)]
pub enum HostLibm {}

#[cfg(feature = "host-libm")]
#[allow(
    clippy::disallowed_methods,
    reason = "the pinned numbers come from the host libm; Fdlibm is the portable choice"
)]
impl Libm for HostLibm {
    #[inline(never)]
    fn sin(x: f64) -> f64 {
        x.sin()
    }
    #[inline(never)]
    fn cos(x: f64) -> f64 {
        x.cos()
    }
    #[inline(never)]
    fn exp(x: f64) -> f64 {
        x.exp()
    }
    #[inline(never)]
    fn atan2(y: f64, x: f64) -> f64 {
        y.atan2(x)
    }
    #[inline(never)]
    fn cbrt(x: f64) -> f64 {
        x.cbrt()
    }
    #[inline(never)]
    fn hypot(x: f64, y: f64) -> f64 {
        x.hypot(y)
    }
}
