// ====================================================
// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
// Developed at SunSoft, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
// ====================================================

//! A model of the engine that generated `tests/data/fdlibm.tsv`, which accounts for every bit where Node's `Math`
//! differs from the port.
//!
//! V8's `Math.sin`, `cos`, `tan`, `asin`, `acos`, `atan`, `atan2`, `exp` and `log` are fdlibm's C code, built by
//! clang. On arm64 clang contracts `a * b + c` inside one C expression into a fused multiply-add
//! (`-ffp-contract=on`), so the engine rounds once where the port, like JavaScript, rounds twice. V8 also has two
//! deliberate changes: `exp(1)` returns `Math.E`, and `atan2` takes FreeBSD's `m &= 1` for |y/x| > 2^60.
//!
//! The model is fdlibm with each contraction written as [`mad`], following the C source's statements:
//! - `model::<false, false>` (no contraction, no V8 changes) must equal the port on every vector: the model is the
//!   same algorithm as the port (which `tests/jdk.rs` checks against fdlibm itself);
//! - `model::<true, true>` must equal the engine on every vector and sweep: the algorithm is fdlibm.
//!
//! `kernel_rem_pio2` is shared with the port: its products are exact (24-bit chunks), so contraction does not
//! change them.

#![allow(
    clippy::excessive_precision,
    reason = "fdlibm's constants are kept as published"
)]

use super::*;

/// `a * b + c`: one rounding when `FUSED` (a contracted expression), two otherwise.
fn mad<const FUSED: bool>(a: f64, b: f64, c: f64) -> f64 {
    if FUSED { a.mul_add(b, c) } else { a * b + c }
}

fn k_sin<const F: bool>(x: f64, y: f64, has_tail: bool) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix < 0x3e40_0000 && to_int32(x) == 0 {
        return x;
    }
    let z = x * x;
    let v = z * x;
    let r = mad::<F>(z, mad::<F>(z, mad::<F>(z, mad::<F>(z, S6, S5), S4), S3), S2);
    if !has_tail {
        return mad::<F>(v, mad::<F>(z, r, S1), x);
    }
    // x-((z*(half*y-v*r)-y)-v*S1)
    let a = mad::<F>(0.5, y, -(v * r));
    let b = mad::<F>(z, a, -y);
    x - mad::<F>(-v, S1, b)
}

fn k_cos<const F: bool>(x: f64, y: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix < 0x3e40_0000 && to_int32(x) == 0 {
        return 1.0;
    }
    let z = x * x;
    let r = z * mad::<F>(
        z,
        mad::<F>(z, mad::<F>(z, mad::<F>(z, mad::<F>(z, C6, C5), C4), C3), C2),
        C1,
    );
    if ix < 0x3fd3_3333 {
        // one - (0.5*z - (z*r - x*y))
        return 1.0 - mad::<F>(0.5, z, -mad::<F>(z, r, -(x * y)));
    }
    let qx = if ix > 0x3fe9_0000 {
        0.28125
    } else {
        from_words(ix - 0x0020_0000, 0)
    };
    let hz = mad::<F>(0.5, z, -qx);
    let a = 1.0 - qx;
    a - (hz - mad::<F>(z, r, -(x * y)))
}

fn k_tan<const F: bool>(mut x: f64, mut y: f64, iy: i32) -> f64 {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix < 0x3e30_0000 && to_int32(x) == 0 {
        if (ix as u32 | low_word(x)) == 0 && iy == -1 {
            return 1.0 / x.abs();
        }
        if iy == 1 {
            return x;
        }
        let w = x + y;
        let z = with_low_word(w, 0);
        let v = y - (z - x);
        let a = -1.0 / w;
        let t = with_low_word(a, 0);
        let s = mad::<F>(t, z, 1.0);
        return mad::<F>(a, mad::<F>(t, v, s), t);
    }
    if ix >= 0x3fe5_9428 {
        if hx < 0 {
            x = -x;
            y = -y;
        }
        let z = PIO4 - x;
        let w = PIO4_LO - y;
        x = z + w;
        y = 0.0;
    }
    let z = x * x;
    let mut w = z * z;
    let mut r = mad::<F>(
        w,
        mad::<F>(
            w,
            mad::<F>(w, mad::<F>(w, mad::<F>(w, T11, T9), T7), T5),
            T3,
        ),
        T1,
    );
    let mut v = z * mad::<F>(
        w,
        mad::<F>(
            w,
            mad::<F>(w, mad::<F>(w, mad::<F>(w, T12, T10), T8), T6),
            T4,
        ),
        T2,
    );
    let s = z * x;
    r = mad::<F>(z, mad::<F>(s, r + v, y), y);
    r = mad::<F>(T0, s, r);
    w = x + r;
    if ix >= 0x3fe5_9428 {
        v = f64::from(iy);
        return f64::from(1 - (shr(hx, 30) & 2)) * mad::<F>(-2.0, x - ((w * w) / (w + v) - r), v);
    }
    if iy == 1 {
        return w;
    }
    let z = with_low_word(w, 0);
    let v = r - (z - x);
    let a = -1.0 / w;
    let t = with_low_word(a, 0);
    let s = mad::<F>(t, z, 1.0);
    mad::<F>(a, mad::<F>(t, v, s), t)
}

fn rem_pio2_model<const F: bool>(x: f64) -> (i32, f64, f64) {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return (0, x, 0.0);
    }
    if ix < 0x4002_d97c {
        if hx > 0 {
            let mut z = x - PIO2_1;
            if ix != 0x3ff9_21fb {
                let y0 = z - PIO2_1T;
                return (1, y0, (z - y0) - PIO2_1T);
            }
            z -= PIO2_2;
            let y0 = z - PIO2_2T;
            return (1, y0, (z - y0) - PIO2_2T);
        }
        let mut z = x + PIO2_1;
        if ix != 0x3ff9_21fb {
            let y0 = z + PIO2_1T;
            return (-1, y0, (z - y0) + PIO2_1T);
        }
        z += PIO2_2;
        let y0 = z + PIO2_2T;
        return (-1, y0, (z - y0) + PIO2_2T);
    }
    if ix <= 0x4139_21fb {
        let t = x.abs();
        let n = to_int32(mad::<F>(t, INVPIO2, 0.5));
        let f_n = f64::from(n);
        let mut r = mad::<F>(-f_n, PIO2_1, t);
        let mut w = f_n * PIO2_1T;
        let mut y0;
        if n < 32 && at(&NPIO2_HW, n - 1) != Some(ix) {
            y0 = r - w;
        } else {
            let j = ix >> 20;
            y0 = r - w;
            let mut i = j - ((high_word(y0) >> 20) & 0x7ff);
            if i > 16 {
                let mut tt = r;
                w = f_n * PIO2_2;
                r = tt - w;
                w = mad::<F>(f_n, PIO2_2T, -((tt - r) - w));
                y0 = r - w;
                i = j - ((high_word(y0) >> 20) & 0x7ff);
                if i > 49 {
                    tt = r;
                    w = f_n * PIO2_3;
                    r = tt - w;
                    w = mad::<F>(f_n, PIO2_3T, -((tt - r) - w));
                    y0 = r - w;
                }
            }
        }
        let y1 = (r - y0) - w;
        if hx < 0 {
            return (-n, -y0, -y1);
        }
        return (n, y0, y1);
    }
    // Infinities, NaNs and the Payne–Hanek range have no contractible expression.
    rem_pio2(x)
}

pub fn sin<const F: bool>(x: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return k_sin::<F>(x, 0.0, false);
    }
    if ix >= 0x7ff0_0000 {
        return f64::NAN;
    }
    let (n, y0, y1) = rem_pio2_model::<F>(x);
    match n & 3 {
        0 => k_sin::<F>(y0, y1, true),
        1 => k_cos::<F>(y0, y1),
        2 => -k_sin::<F>(y0, y1, true),
        _ => -k_cos::<F>(y0, y1),
    }
}

pub fn cos<const F: bool>(x: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return k_cos::<F>(x, 0.0);
    }
    if ix >= 0x7ff0_0000 {
        return f64::NAN;
    }
    let (n, y0, y1) = rem_pio2_model::<F>(x);
    match n & 3 {
        0 => k_cos::<F>(y0, y1),
        1 => -k_sin::<F>(y0, y1, true),
        2 => -k_cos::<F>(y0, y1),
        _ => k_sin::<F>(y0, y1, true),
    }
}

pub fn tan<const F: bool>(x: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return k_tan::<F>(x, 0.0, 1);
    }
    if ix >= 0x7ff0_0000 {
        return f64::NAN;
    }
    let (n, y0, y1) = rem_pio2_model::<F>(x);
    k_tan::<F>(y0, y1, 1 - ((n & 1) << 1))
}

fn poly_p<const F: bool>(t: f64) -> f64 {
    t * mad::<F>(
        t,
        mad::<F>(
            t,
            mad::<F>(t, mad::<F>(t, mad::<F>(t, PS5, PS4), PS3), PS2),
            PS1,
        ),
        PS0,
    )
}

fn poly_q<const F: bool>(t: f64) -> f64 {
    mad::<F>(
        t,
        mad::<F>(t, mad::<F>(t, mad::<F>(t, QS4, QS3), QS2), QS1),
        1.0,
    )
}

pub fn asin<const F: bool>(x: f64) -> f64 {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3ff0_0000 {
        if ((ix - 0x3ff0_0000) as u32 | low_word(x)) == 0 {
            return mad::<F>(x, PIO2_HI, x * PIO2_LO);
        }
        return f64::NAN;
    }
    if ix < 0x3fe0_0000 {
        if ix < 0x3e40_0000 {
            return x;
        }
        let t = x * x;
        let p = poly_p::<F>(t);
        let q = poly_q::<F>(t);
        let w = p / q;
        return mad::<F>(x, w, x);
    }
    let w = 1.0 - x.abs();
    let t = w * 0.5;
    let p = poly_p::<F>(t);
    let q = poly_q::<F>(t);
    let s = t.sqrt();
    let result = if ix >= 0x3fef_3333 {
        let w = p / q;
        PIO2_HI - mad::<F>(2.0, mad::<F>(s, w, s), -PIO2_LO)
    } else {
        let w = with_low_word(s, 0);
        let c = mad::<F>(-w, w, t) / (s + w);
        let r = p / q;
        let p = mad::<F>(2.0 * s, r, -mad::<F>(-2.0, c, PIO2_LO));
        let q = mad::<F>(-2.0, w, PIO4_HI);
        PIO4_HI - (p - q)
    };
    if hx > 0 { result } else { -result }
}

pub fn acos<const F: bool>(x: f64) -> f64 {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3ff0_0000 {
        if ((ix - 0x3ff0_0000) as u32 | low_word(x)) == 0 {
            return if hx > 0 {
                0.0
            } else {
                mad::<F>(2.0, PIO2_LO, PI)
            };
        }
        return f64::NAN;
    }
    if ix < 0x3fe0_0000 {
        if ix <= 0x3c60_0000 {
            return PIO2_HI + PIO2_LO;
        }
        let z = x * x;
        let r = poly_p::<F>(z) / poly_q::<F>(z);
        return PIO2_HI - (x - mad::<F>(-x, r, PIO2_LO));
    }
    if hx < 0 {
        let z = (1.0 + x) * 0.5;
        let p = poly_p::<F>(z);
        let q = poly_q::<F>(z);
        let s = z.sqrt();
        let r = p / q;
        let w = mad::<F>(r, s, -PIO2_LO);
        return mad::<F>(-2.0, s + w, PI);
    }
    let z = (1.0 - x) * 0.5;
    let s = z.sqrt();
    let df = with_low_word(s, 0);
    let c = mad::<F>(-df, df, z) / (s + df);
    let r = poly_p::<F>(z) / poly_q::<F>(z);
    let w = mad::<F>(r, s, c);
    2.0 * (df + w)
}

pub fn atan<const F: bool>(mut x: f64) -> f64 {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    let id: i32;
    if ix >= 0x4410_0000 {
        if ix > 0x7ff0_0000 || (ix == 0x7ff0_0000 && low_word(x) != 0) {
            return x + x;
        }
        return if hx > 0 {
            ATAN_HI[3] + ATAN_LO[3]
        } else {
            -ATAN_HI[3] - ATAN_LO[3]
        };
    }
    if ix < 0x3fdc_0000 {
        if ix < 0x3e20_0000 {
            return x;
        }
        id = -1;
    } else {
        x = x.abs();
        if ix < 0x3ff3_0000 {
            if ix < 0x3fe6_0000 {
                id = 0;
                x = mad::<F>(2.0, x, -1.0) / (2.0 + x);
            } else {
                id = 1;
                x = (x - 1.0) / (x + 1.0);
            }
        } else if ix < 0x4003_8000 {
            id = 2;
            x = (x - 1.5) / mad::<F>(1.5, x, 1.0);
        } else {
            id = 3;
            x = -1.0 / x;
        }
    }
    let z = x * x;
    let w = z * z;
    let s1 = z * mad::<F>(
        w,
        mad::<F>(
            w,
            mad::<F>(w, mad::<F>(w, mad::<F>(w, AT10, AT8), AT6), AT4),
            AT2,
        ),
        AT0,
    );
    let s2 = w * mad::<F>(
        w,
        mad::<F>(w, mad::<F>(w, mad::<F>(w, AT9, AT7), AT5), AT3),
        AT1,
    );
    let Ok(id) = usize::try_from(id) else {
        return mad::<F>(-x, s1 + s2, x);
    };
    let z = ATAN_HI[id] - (mad::<F>(x, s1 + s2, -ATAN_LO[id]) - x);
    if hx < 0 { -z } else { z }
}

pub fn atan2<const F: bool, const V8: bool>(y: f64, x: f64) -> f64 {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    let lx = low_word(x);
    let hy = high_word(y);
    let iy = hy & 0x7fff_ffff;
    let ly = low_word(y);
    if x.is_nan() || y.is_nan() {
        return x + y;
    }
    if (hx.wrapping_sub(0x3ff0_0000) as u32 | lx) == 0 {
        return atan::<F>(y);
    }
    let mut m = ((hy as u32 >> 31) & 1) | ((hx as u32 >> 30) & 2);
    if (iy as u32 | ly) == 0 {
        return match m {
            0 | 1 => y,
            2 => PI,
            _ => -PI,
        };
    }
    if (ix as u32 | lx) == 0 {
        return if hy < 0 { -PIO2_HI } else { PIO2_HI };
    }
    if ix == 0x7ff0_0000 {
        if iy == 0x7ff0_0000 {
            return [PIO4_HI, -PIO4_HI, 3.0 * PIO4_HI, -3.0 * PIO4_HI][m as usize];
        }
        return [0.0, -0.0, PI, -PI][m as usize];
    }
    if iy == 0x7ff0_0000 {
        return if hy < 0 { -PIO2_HI } else { PIO2_HI };
    }
    let k = (iy - ix) >> 20;
    let z = if k > 60 {
        if V8 {
            m &= 1;
        }
        mad::<F>(0.5, PI_LO, PIO2_HI)
    } else if hx < 0 && k < -60 {
        0.0
    } else {
        atan::<F>((y / x).abs())
    };
    match m {
        0 => z,
        1 => -z,
        2 => PI - (z - PI_LO),
        _ => (z - PI_LO) - PI,
    }
}

pub fn exp<const F: bool, const V8: bool>(mut x: f64) -> f64 {
    let mut hx = high_word(x);
    let xsb = (hx as u32 >> 31) & 1;
    hx &= 0x7fff_ffff;
    if hx >= 0x4086_2e42 {
        if hx >= 0x7ff0_0000 {
            if ((hx & 0xf_ffff) as u32 | low_word(x)) != 0 {
                return x + x;
            }
            return if xsb == 0 { x } else { 0.0 };
        }
        if x > O_THRESHOLD {
            return f64::INFINITY;
        }
        if x < U_THRESHOLD {
            return 0.0;
        }
    }
    let (mut hi, mut lo, mut k) = (0.0, 0.0, 0);
    if hx > 0x3fd6_2e42 {
        if hx < 0x3ff0_a2b2 {
            if V8 && x == 1.0 {
                return std::f64::consts::E;
            }
            hi = if xsb == 0 { x - LN2_HI } else { x + LN2_HI };
            lo = if xsb == 0 { LN2_LO } else { -LN2_LO };
            k = 1 - 2 * xsb as i32;
        } else {
            k = to_int32(mad::<F>(INVLN2, x, if xsb == 0 { 0.5 } else { -0.5 }));
            let t = f64::from(k);
            hi = mad::<F>(-t, LN2_HI, x);
            lo = t * LN2_LO;
        }
        x = hi - lo;
    } else if hx < 0x3e30_0000 {
        return 1.0 + x;
    }
    let t = x * x;
    let c = mad::<F>(
        -t,
        mad::<F>(t, mad::<F>(t, mad::<F>(t, mad::<F>(t, P5, P4), P3), P2), P1),
        x,
    );
    if k == 0 {
        return 1.0 - ((x * c) / (c - 2.0) - x);
    }
    let y = 1.0 - ((lo - (x * c) / (2.0 - c)) - hi);
    if k >= -1021 {
        return with_high_word(y, high_word(y).wrapping_add(shl(k, 20)));
    }
    with_high_word(y, high_word(y).wrapping_add(shl(k + 1000, 20))) * TWOM1000
}

pub fn log<const F: bool>(mut x: f64) -> f64 {
    let mut hx = high_word(x);
    let lx = low_word(x);
    let mut k = 0;
    if hx < 0x0010_0000 {
        if ((hx & 0x7fff_ffff) as u32 | lx) == 0 {
            return f64::NEG_INFINITY;
        }
        if hx < 0 {
            return f64::NAN;
        }
        k -= 54;
        x *= TWO54;
        hx = high_word(x);
    }
    if hx >= 0x7ff0_0000 {
        return x + x;
    }
    k += (hx >> 20) - 1023;
    hx &= 0x000f_ffff;
    let mut i = (hx + 0x95f64) & 0x10_0000;
    x = with_high_word(x, hx | (i ^ 0x3ff0_0000));
    k += i >> 20;
    let f = x - 1.0;
    let dk = f64::from(k);
    if (0x000f_ffff & (2 + hx)) < 3 {
        if f == 0.0 {
            return if k == 0 {
                0.0
            } else {
                mad::<F>(dk, LN2_HI, dk * LN2_LO)
            };
        }
        let r = (f * f) * mad::<F>(-0.33333333333333333, f, 0.5);
        return if k == 0 {
            f - r
        } else {
            mad::<F>(dk, LN2_HI, -(mad::<F>(-dk, LN2_LO, r) - f))
        };
    }
    let s = f / (2.0 + f);
    let z = s * s;
    i = hx - 0x6147a;
    let w = z * z;
    let j = 0x6b851 - hx;
    let t1 = w * mad::<F>(w, mad::<F>(w, LG6, LG4), LG2);
    let t2 = z * mad::<F>(w, mad::<F>(w, mad::<F>(w, LG7, LG5), LG3), LG1);
    i |= j;
    let r = t2 + t1;
    if i > 0 {
        let hfsq = 0.5 * f * f;
        if k == 0 {
            return f - mad::<F>(-s, hfsq + r, hfsq);
        }
        return mad::<F>(
            dk,
            LN2_HI,
            -((hfsq - mad::<F>(s, hfsq + r, dk * LN2_LO)) - f),
        );
    }
    if k == 0 {
        return mad::<F>(-s, f - r, f);
    }
    mad::<F>(dk, LN2_HI, -(mad::<F>(s, f - r, -(dk * LN2_LO)) - f))
}

#[path = "../../tests/samples/mod.rs"]
mod samples;

use samples::{canonical_bits, parse_bits, sweeps};

/// The model functions, `FUSED` and `V8` as in the module doc.
fn model<const FUSED: bool, const V8: bool>(name: &str, args: &[f64]) -> Option<f64> {
    Some(match (name, args) {
        ("sin", [x]) => sin::<FUSED>(*x),
        ("cos", [x]) => cos::<FUSED>(*x),
        ("tan", [x]) => tan::<FUSED>(*x),
        ("asin", [x]) => asin::<FUSED>(*x),
        ("acos", [x]) => acos::<FUSED>(*x),
        ("atan", [x]) => atan::<FUSED>(*x),
        ("exp", [x]) => exp::<FUSED, V8>(*x),
        ("log", [x]) => log::<FUSED>(*x),
        ("atan2", [y, x]) => atan2::<FUSED, V8>(*y, *x),
        // V8's cbrt is FreeBSD's later algorithm and its hypot is its own: no fdlibm model.
        _ => return None,
    })
}

fn port(name: &str, args: &[f64]) -> f64 {
    match (name, args) {
        ("sin", [x]) => super::sin(*x),
        ("cos", [x]) => super::cos(*x),
        ("tan", [x]) => super::tan(*x),
        ("asin", [x]) => super::asin(*x),
        ("acos", [x]) => super::acos(*x),
        ("atan", [x]) => super::atan(*x),
        ("exp", [x]) => super::exp(*x),
        ("log", [x]) => super::log(*x),
        ("cbrt", [x]) => super::cbrt(*x),
        ("atan2", [y, x]) => super::atan2(*y, *x),
        ("hypot", [x, y]) => super::hypot(*x, *y),
        _ => panic!("unknown function {name}"),
    }
}

const VECTORS: &str = include_str!("../../tests/data/fdlibm.tsv");
const SWEEPS: &str = include_str!("../../tests/data/fdlibm-sweep.tsv");

/// `(fn, args, engine result)` of every vector.
fn vectors() -> Vec<(&'static str, Vec<f64>, f64)> {
    VECTORS
        .lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            let (result, args) = cols[2..].split_last().unwrap();
            (
                cols[0],
                args.iter().map(|h| parse_bits(h)).collect(),
                parse_bits(result),
            )
        })
        .collect()
}

/// Whether the engine fused multiply-adds: the generator records its architecture.
fn engine_fuses() -> bool {
    let header = VECTORS
        .lines()
        .find(|line| line.starts_with("# engine"))
        .unwrap();
    assert!(
        header.contains("arch=arm64") || header.contains("arch=x64"),
        "unmodelled engine: {header}"
    );
    header.contains("arch=arm64")
}

#[test]
fn the_model_without_contraction_is_the_port() {
    let mut checked = 0;
    for (name, args, _) in vectors() {
        if let Some(m) = model::<false, false>(name, &args) {
            assert_eq!(
                canonical_bits(m),
                canonical_bits(port(name, &args)),
                "{name}{args:?}"
            );
            checked += 1;
        }
    }
    assert!(checked > 15_000, "{checked}");
}

#[test]
fn the_contracted_model_is_the_engine() {
    let fused = engine_fuses();
    let engine = |name: &str, args: &[f64]| {
        if fused {
            model::<true, true>(name, args)
        } else {
            model::<false, true>(name, args)
        }
    };
    let mut checked = 0;
    for (name, args, expected) in vectors() {
        if let Some(m) = engine(name, &args) {
            assert_eq!(
                canonical_bits(m),
                canonical_bits(expected),
                "{name}{args:?}: {m:e} vs {expected:e}"
            );
            checked += 1;
        }
    }
    assert!(checked > 15_000, "{checked}");
    let mut rows = 0;
    for row in sweeps(SWEEPS) {
        let digest = row.digest_of(|args| engine(&row.name, args)).unwrap();
        assert_eq!(digest, row.digest, "{} ({} samples)", row.key, row.count);
        rows += 1;
    }
    assert!(rows >= 30, "{rows}");
}

/// Negative control for the model: contraction is visible in the vectors, so an unfused model cannot pass.
#[test]
fn the_engine_is_not_the_unfused_port() {
    if !engine_fuses() {
        return;
    }
    let differing = vectors()
        .iter()
        .filter(|(name, args, expected)| {
            model::<false, true>(name, args)
                .is_some_and(|m| canonical_bits(m) != canonical_bits(*expected))
        })
        .count();
    assert!(differing > 0);
}
