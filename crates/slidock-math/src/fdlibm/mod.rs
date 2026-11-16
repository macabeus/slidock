// ====================================================
// Copyright (C) 1993 by Sun Microsystems, Inc. All rights reserved.
//
// Developed at SunSoft, a Sun Microsystems, Inc. business.
// Permission to use, copy, modify, and distribute this
// software is freely granted, provided that this notice
// is preserved.
// ====================================================

//! Transcendental functions translated from fdlibm 5.3, with FreeBSD msun's fix to hypot's subnormal scaling (its
//! `e_hypot.c` carries the same notice). They run the same IEEE operations on every target, so results are
//! bit-identical natively and on wasm32. Errors are below one ulp. `tests/jdk.rs` checks every function bit for bit
//! against the JDK's StrictMath, itself a port of fdlibm 5.3; Node's `Math` can differ in the last bit (`v8_model`).
//!
//! Integer steps keep JavaScript's int32 semantics: `x | 0` is [`to_int32`], shifts mask their count, and the
//! words written back into a double wrap.

#![allow(
    clippy::excessive_precision,
    clippy::approx_constant,
    reason = "fdlibm's constants are kept as published"
)]

/// The high 32 bits of a double, as a signed word.
fn high_word(x: f64) -> i32 {
    (x.to_bits() >> 32) as u32 as i32
}

/// The low 32 bits of a double.
fn low_word(x: f64) -> u32 {
    x.to_bits() as u32
}

fn from_words(high: i32, low: u32) -> f64 {
    f64::from_bits((u64::from(high as u32) << 32) | u64::from(low))
}

fn with_high_word(x: f64, high: i32) -> f64 {
    from_words(high, low_word(x))
}

fn with_low_word(x: f64, low: u32) -> f64 {
    from_words(high_word(x), low)
}

/// ECMAScript ToInt32 (`x | 0`): truncation, then reduction modulo 2^32.
fn to_int32(x: f64) -> i32 {
    if !x.is_finite() {
        return 0;
    }
    let t = x.trunc();
    if t >= f64::from(i32::MIN) && t <= f64::from(i32::MAX) {
        return t as i32;
    }
    t.rem_euclid(4_294_967_296.0) as u32 as i32
}

/// JavaScript `a >> n`.
fn shr(a: i32, n: i32) -> i32 {
    a.wrapping_shr(n as u32)
}

/// JavaScript `a << n`.
fn shl(a: i32, n: i32) -> i32 {
    a.wrapping_shl(n as u32)
}

/// x · 2^n, exact unless the result over- or underflows; n within ±2098.
fn scalbn(mut x: f64, mut n: i32) -> f64 {
    while n > 1023 {
        x *= from_words(0x7fe0_0000, 0);
        n -= 1023;
    }
    while n < -1022 {
        x *= from_words(0x0010_0000, 0);
        n += 1022;
    }
    x * from_words(shl(n + 1023, 20), 0)
}

// Kernels on [-π/4, π/4]: x + y is the reduced argument, y its tail.

const S1: f64 = -1.66666666666666324348e-1;
const S2: f64 = 8.33333333332248946124e-3;
const S3: f64 = -1.98412698298579493134e-4;
const S4: f64 = 2.75573137070700676789e-6;
const S5: f64 = -2.50507602534068634195e-8;
const S6: f64 = 1.58969099521155010221e-10;

fn kernel_sin(x: f64, y: f64, has_tail: bool) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix < 0x3e40_0000 && to_int32(x) == 0 {
        return x; // |x| < 2^-27
    }
    let z = x * x;
    let v = z * x;
    let r = S2 + z * (S3 + z * (S4 + z * (S5 + z * S6)));
    if !has_tail {
        return x + v * (S1 + z * r);
    }
    x - (z * (0.5 * y - v * r) - y - v * S1)
}

const C1: f64 = 4.16666666666666019037e-2;
const C2: f64 = -1.38888888888741095749e-3;
const C3: f64 = 2.48015872894767294178e-5;
const C4: f64 = -2.75573143513906633035e-7;
const C5: f64 = 2.0875723212981748279e-9;
const C6: f64 = -1.13596475577881948265e-11;

fn kernel_cos(x: f64, y: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix < 0x3e40_0000 && to_int32(x) == 0 {
        return 1.0; // |x| < 2^-27
    }
    let z = x * x;
    let r = z * (C1 + z * (C2 + z * (C3 + z * (C4 + z * (C5 + z * C6)))));
    if ix < 0x3fd3_3333 {
        return 1.0 - (0.5 * z - (z * r - x * y)); // |x| < 0.3
    }
    let qx = if ix > 0x3fe9_0000 {
        0.28125
    } else {
        from_words(ix - 0x0020_0000, 0)
    }; // x/4
    let hz = 0.5 * z - qx;
    let a = 1.0 - qx;
    a - (hz - (z * r - x * y))
}

const T0: f64 = 3.33333333333334091986e-1;
const T1: f64 = 1.33333333333201242699e-1;
const T2: f64 = 5.39682539762260521377e-2;
const T3: f64 = 2.18694882948595424599e-2;
const T4: f64 = 8.86323982359930005737e-3;
const T5: f64 = 3.59207910759131235356e-3;
const T6: f64 = 1.45620945432529025516e-3;
const T7: f64 = 5.88041240820264096874e-4;
const T8: f64 = 2.46463134818469906812e-4;
const T9: f64 = 7.817944429395570923e-5;
const T10: f64 = 7.14072491382608190305e-5;
const T11: f64 = -1.85586374855275456654e-5;
const T12: f64 = 2.59073051863633712884e-5;
const PIO4: f64 = 7.85398163397448278999e-1;
const PIO4_LO: f64 = 3.06161699786838301793e-17;

/// tan(x + y), or -1/tan(x + y) when `reciprocal` is set.
fn kernel_tan(mut x: f64, mut y: f64, reciprocal: bool) -> f64 {
    let sign: i32 = if reciprocal { -1 } else { 1 };
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix < 0x3e30_0000 && to_int32(x) == 0 {
        // |x| < 2^-28
        if (ix as u32 | low_word(x)) == 0 && reciprocal {
            return 1.0 / x.abs();
        }
        if !reciprocal {
            return x;
        }
        // -1 / (x + y), computed carefully.
        let w = x + y;
        let z = with_low_word(w, 0);
        let v = y - (z - x);
        let a = -1.0 / w;
        let t = with_low_word(a, 0);
        let s = 1.0 + t * z;
        return t + a * (s + t * v);
    }
    if ix >= 0x3fe5_9428 {
        // |x| >= 0.6744
        if hx < 0 {
            x = -x;
            y = -y;
        }
        x = PIO4 - x + (PIO4_LO - y);
        y = 0.0;
    }
    let z = x * x;
    let mut w = z * z;
    // x^5 (T1 + x^2 T2 + ...) split into odd and even polynomials in x^4.
    let mut r = T1 + w * (T3 + w * (T5 + w * (T7 + w * (T9 + w * T11))));
    let mut v = z * (T2 + w * (T4 + w * (T6 + w * (T8 + w * (T10 + w * T12)))));
    let s = z * x;
    r = y + z * (s * (r + v) + y);
    r += T0 * s;
    w = x + r;
    if ix >= 0x3fe5_9428 {
        v = f64::from(sign);
        return f64::from(1 - (shr(hx, 30) & 2)) * (v - 2.0 * (x - ((w * w) / (w + v) - r)));
    }
    if !reciprocal {
        return w;
    }
    // -1 / (x + r), computed accurately.
    let zz = with_low_word(w, 0);
    let vv = r - (zz - x);
    let a = -1.0 / w;
    let t = with_low_word(a, 0);
    let ss = 1.0 + t * zz;
    t + a * (ss + t * vv)
}

// Argument reduction: x = n·π/2 + (y0 + y1).

/// 2/π in 24-bit chunks.
const TWO_OVER_PI: [i32; 66] = [
    0xa2f983, 0x6e4e44, 0x1529fc, 0x2757d1, 0xf534dd, 0xc0db62, 0x95993c, 0x439041, 0xfe5163,
    0xabdebb, 0xc561b7, 0x246e3a, 0x424dd2, 0xe00649, 0x2eea09, 0xd1921c, 0xfe1deb, 0x1cb129,
    0xa73ee8, 0x8235f5, 0x2ebb44, 0x84e99c, 0x7026b4, 0x5f7e41, 0x3991d6, 0x398353, 0x39f49c,
    0x845f8b, 0xbdf928, 0x3b1ff8, 0x97ffde, 0x05980f, 0xef2f11, 0x8b5a0a, 0x6d1f6d, 0x367ecf,
    0x27cb09, 0xb74f46, 0x3f669e, 0x5fea2d, 0x7527ba, 0xc7ebe5, 0xf17b3d, 0x0739f7, 0x8a5292,
    0xea6bfb, 0x5fb11f, 0x8d5d08, 0x560330, 0x46fc7b, 0x6babf0, 0xcfbc20, 0x9af436, 0x1da9e3,
    0x91615e, 0xe61b08, 0x659985, 0x5f14a0, 0x68408d, 0xffd880, 0x4d7327, 0x310606, 0x1556ca,
    0x73a8c9, 0x60e27b, 0xc08c6b,
];

/// High words of n·π/2 for n = 1..32, where a quick reduction may cancel.
const NPIO2_HW: [i32; 32] = [
    0x3ff921fb, 0x400921fb, 0x4012d97c, 0x401921fb, 0x401f6a7a, 0x4022d97c, 0x4025fdbb, 0x402921fb,
    0x402c463a, 0x402f6a7a, 0x4031475c, 0x4032d97c, 0x40346b9c, 0x4035fdbb, 0x40378fdb, 0x403921fb,
    0x403ab41b, 0x403c463a, 0x403dd85a, 0x403f6a7a, 0x40407e4c, 0x4041475c, 0x4042106c, 0x4042d97c,
    0x4043a28c, 0x40446b9c, 0x404534ac, 0x4045fdbb, 0x4046c6cb, 0x40478fdb, 0x404858eb, 0x404921fb,
];

const TWO24: f64 = 1.6777216e7;
const TWON24: f64 = 5.9604644775390625e-8;
const INVPIO2: f64 = 6.36619772367581382433e-1;
const PIO2_1: f64 = 1.57079632673412561417;
const PIO2_1T: f64 = 6.07710050650619224932e-11;
const PIO2_2: f64 = 6.0771005063039659766e-11;
const PIO2_2T: f64 = 2.02226624879595063154e-21;
const PIO2_3: f64 = 2.0222662487111664558e-21;
const PIO2_3T: f64 = 8.47842766036889956997e-32;

/// π/2 in 24-bit pieces, for the large-argument kernel.
const PIO2_PIECES: [f64; 8] = [
    1.57079625129699707031,
    7.54978941586159635335e-8,
    5.39030252995776476554e-15,
    3.28200341580791294123e-22,
    1.27065575308067607349e-29,
    1.22933308981111328932e-36,
    2.73370053816464559624e-44,
    2.16741683877804819444e-51,
];

/// Looks an `i32` index up in a table, as JavaScript reads an array: outside it there is nothing.
fn at<T: Copy>(table: &[T], i: i32) -> Option<T> {
    usize::try_from(i).ok().and_then(|i| table.get(i)).copied()
}

/// A work array of the reduction kernel. Reads outside it give `None` (JavaScript's `undefined`), writes there are
/// dropped; neither happens for finite arguments.
struct Work<T: Copy, const N: usize>([T; N]);

impl<T: Copy, const N: usize> Work<T, N> {
    fn get(&self, i: i32) -> Option<T> {
        at(&self.0, i)
    }

    fn set(&mut self, i: i32, value: T) {
        if let Some(slot) = usize::try_from(i).ok().and_then(|i| self.0.get_mut(i)) {
            *slot = value;
        }
    }
}

impl<const N: usize> Work<f64, N> {
    /// `undefined` is NaN in arithmetic.
    fn f(&self, i: i32) -> f64 {
        self.get(i).unwrap_or(f64::NAN)
    }
}

impl<const N: usize> Work<i32, N> {
    /// `undefined` is 0 in bitwise arithmetic.
    fn i(&self, i: i32) -> i32 {
        self.get(i).unwrap_or(0)
    }
}

fn two_over_pi(j: i32) -> f64 {
    at(&TWO_OVER_PI, j).map_or(f64::NAN, f64::from)
}

/// Payne–Hanek reduction for |x| ≥ 2^19·π/2 (fdlibm's `__kernel_rem_pio2` at double precision): `x` holds the
/// argument in 24-bit chunks scaled by 2^-e0. Returns n mod 8 and the remainder `(y0, y1)`.
fn kernel_rem_pio2(x: &[f64], e0: i32) -> (i32, f64, f64) {
    const JK: i32 = 4; // init_jk[prec = 2]
    const JP: i32 = JK;
    let jx = i32::try_from(x.len()).unwrap_or(1) - 1;
    let xs = |j: i32| at(x, j).unwrap_or(f64::NAN);
    let mut jv = (e0 - 3) / 24;
    if jv < 0 {
        jv = 0;
    }
    let mut q0 = e0 - 24 * (jv + 1);

    let mut f = Work([0.0; 20]);
    let mut q = Work([0.0; 20]);
    let mut fq = Work([0.0; 20]);
    let mut iq = Work([0i32; 20]);
    for i in 0..=jx + JK {
        let j = jv - jx + i;
        f.set(i, if j < 0 { 0.0 } else { two_over_pi(j) });
    }
    for i in 0..=JK {
        let mut fw = 0.0;
        for j in 0..=jx {
            fw += xs(j) * f.f(jx + i - j);
        }
        q.set(i, fw);
    }

    let mut jz = JK;
    let mut z;
    let mut n;
    let mut ih;
    loop {
        // Distill q[] into iq[] in reverse.
        let mut i = 0;
        let mut j = jz;
        z = q.f(jz);
        while j > 0 {
            let fw = to_int32(TWON24 * z);
            iq.set(i, to_int32(z - TWO24 * f64::from(fw)));
            z = q.f(j - 1) + f64::from(fw);
            i += 1;
            j -= 1;
        }

        z = scalbn(z, q0);
        z -= 8.0 * (z * 0.125).floor(); // trim off integers >= 8
        n = to_int32(z);
        z -= f64::from(n);
        ih = 0;
        if q0 > 0 {
            // iq[jz - 1] is needed to determine n.
            let k = shr(iq.i(jz - 1), 24 - q0);
            n = n.wrapping_add(k);
            iq.set(jz - 1, iq.i(jz - 1).wrapping_sub(shl(k, 24 - q0)));
            ih = shr(iq.i(jz - 1), 23 - q0);
        } else if q0 == 0 {
            ih = shr(iq.i(jz - 1), 23);
        } else if z >= 0.5 {
            ih = 2;
        }

        if ih > 0 {
            // q > 0.5: take 1 - q.
            n = n.wrapping_add(1);
            let mut carry = 0;
            for k in 0..jz {
                let v = iq.i(k);
                if carry == 0 {
                    if v != 0 {
                        carry = 1;
                        iq.set(k, 0x100_0000_i32.wrapping_sub(v));
                    }
                } else {
                    iq.set(k, 0xff_ffff_i32.wrapping_sub(v));
                }
            }
            if q0 == 1 {
                iq.set(jz - 1, iq.i(jz - 1) & 0x7f_ffff);
            } else if q0 == 2 {
                iq.set(jz - 1, iq.i(jz - 1) & 0x3f_ffff);
            }
            if ih == 2 {
                z = 1.0 - z;
                if carry != 0 {
                    z -= scalbn(1.0, q0);
                }
            }
        }

        // Recompute with more terms when the result cancelled to zero.
        if z == 0.0 {
            let mut any = 0;
            let mut k = jz - 1;
            while k >= JK {
                any |= iq.i(k);
                k -= 1;
            }
            if any == 0 {
                let mut k = 1;
                while iq.get(JK - k) == Some(0) {
                    k += 1;
                }
                for m in jz + 1..=jz + k {
                    f.set(jx + m, two_over_pi(jv + m));
                    let mut fw = 0.0;
                    for l in 0..=jx {
                        fw += xs(l) * f.f(jx + m - l);
                    }
                    q.set(m, fw);
                }
                jz += k;
                continue;
            }
        }
        break;
    }

    // Chop off zero terms, or break z into 24-bit chunks.
    if z == 0.0 {
        jz -= 1;
        q0 -= 24;
        while iq.get(jz) == Some(0) {
            jz -= 1;
            q0 -= 24;
        }
    } else {
        z = scalbn(z, -q0);
        if z >= TWO24 {
            let fw = to_int32(TWON24 * z);
            iq.set(jz, to_int32(z - TWO24 * f64::from(fw)));
            jz += 1;
            q0 += 24;
            iq.set(jz, fw);
        } else {
            iq.set(jz, to_int32(z));
        }
    }

    // The integer chunks back to doubles, then times π/2.
    let mut fw = scalbn(1.0, q0);
    let mut i = jz;
    while i >= 0 {
        q.set(i, fw * f64::from(iq.i(i)));
        fw *= TWON24;
        i -= 1;
    }
    let mut i = jz;
    while i >= 0 {
        let mut sum = 0.0;
        let mut k = 0;
        while k <= JP && k <= jz - i {
            sum += at(&PIO2_PIECES, k).unwrap_or(f64::NAN) * q.f(i + k);
            k += 1;
        }
        fq.set(jz - i, sum);
        i -= 1;
    }

    let mut sum = 0.0;
    let mut i = jz;
    while i >= 0 {
        sum += fq.f(i);
        i -= 1;
    }
    let y0 = if ih == 0 { sum } else { -sum };
    let mut tail = fq.f(0) - sum;
    for i in 1..=jz {
        tail += fq.f(i);
    }
    let y1 = if ih == 0 { tail } else { -tail };
    (n & 7, y0, y1)
}

/// `(n, y0, y1)` with x = n·π/2 + y0 + y1, |y0 + y1| ≤ π/4.
fn rem_pio2(x: f64) -> (i32, f64, f64) {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        // |x| ≤ π/4
        return (0, x, 0.0);
    }
    if ix < 0x4002_d97c {
        // |x| < 3π/4: n = ±1
        let sign = if hx > 0 { 1.0 } else { -1.0 };
        let mut z = x - sign * PIO2_1;
        if ix != 0x3ff9_21fb {
            // 33 + 53 bits of π are enough.
            let y0 = z - sign * PIO2_1T;
            return (sign as i32, y0, z - y0 - sign * PIO2_1T);
        }
        // Near π/2: 33 + 33 + 53 bits.
        z -= sign * PIO2_2;
        let y0 = z - sign * PIO2_2T;
        return (sign as i32, y0, z - y0 - sign * PIO2_2T);
    }
    if ix <= 0x4139_21fb {
        // |x| ≤ 2^19·π/2: medium size
        let t = x.abs();
        let n = to_int32(t * INVPIO2 + 0.5);
        let f_n = f64::from(n);
        let mut r = t - f_n * PIO2_1;
        let mut w = f_n * PIO2_1T; // first round, good to 85 bits
        let mut y0;
        if n < 32 && at(&NPIO2_HW, n - 1) != Some(ix) {
            y0 = r - w; // no cancellation
        } else {
            let j = ix >> 20;
            y0 = r - w;
            let mut i = j - ((high_word(y0) >> 20) & 0x7ff);
            if i > 16 {
                // Second iteration, good to 118 bits.
                let mut tt = r;
                w = f_n * PIO2_2;
                r = tt - w;
                w = f_n * PIO2_2T - (tt - r - w);
                y0 = r - w;
                i = j - ((high_word(y0) >> 20) & 0x7ff);
                if i > 49 {
                    // Third iteration, 151 bits: covers every case.
                    tt = r;
                    w = f_n * PIO2_3;
                    r = tt - w;
                    w = f_n * PIO2_3T - (tt - r - w);
                    y0 = r - w;
                }
            }
        }
        let y1 = r - y0 - w;
        if hx < 0 {
            return (-n, -y0, -y1);
        }
        return (n, y0, y1);
    }
    if ix >= 0x7ff0_0000 {
        // Infinity or NaN.
        return (0, f64::NAN, f64::NAN);
    }
    // Large arguments: z = |x| · 2^-(ilogb(x) - 23), split into three 24-bit chunks.
    let e0 = (ix >> 20) - 1046;
    let mut z = from_words(ix - shl(e0, 20), low_word(x));
    let t0 = f64::from(to_int32(z));
    z = (z - t0) * TWO24;
    let t1 = f64::from(to_int32(z));
    z = (z - t1) * TWO24;
    let t2 = z;
    let tx = [t0, t1, t2];
    let len = if t2 != 0.0 {
        3
    } else if t1 != 0.0 {
        2
    } else {
        1
    };
    let (n, y0, y1) = kernel_rem_pio2(tx.get(..len).unwrap_or(&tx), e0);
    if hx < 0 {
        return (-n, -y0, -y1);
    }
    (n, y0, y1)
}

pub fn sin(x: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return kernel_sin(x, 0.0, false);
    }
    if ix >= 0x7ff0_0000 {
        return f64::NAN;
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => kernel_sin(y0, y1, true),
        1 => kernel_cos(y0, y1),
        2 => -kernel_sin(y0, y1, true),
        _ => -kernel_cos(y0, y1),
    }
}

pub fn cos(x: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return kernel_cos(x, 0.0);
    }
    if ix >= 0x7ff0_0000 {
        return f64::NAN;
    }
    let (n, y0, y1) = rem_pio2(x);
    match n & 3 {
        0 => kernel_cos(y0, y1),
        1 => -kernel_sin(y0, y1, true),
        2 => -kernel_cos(y0, y1),
        _ => kernel_sin(y0, y1, true),
    }
}

pub fn tan(x: f64) -> f64 {
    let ix = high_word(x) & 0x7fff_ffff;
    if ix <= 0x3fe9_21fb {
        return kernel_tan(x, 0.0, false);
    }
    if ix >= 0x7ff0_0000 {
        return f64::NAN;
    }
    let (n, y0, y1) = rem_pio2(x);
    kernel_tan(y0, y1, n & 1 != 0)
}

// Inverse trigonometry.

const PIO2_HI: f64 = 1.570796326794896558;
const PIO2_LO: f64 = 6.12323399573676603587e-17;
const PIO4_HI: f64 = 7.85398163397448278999e-1;
const PI: f64 = 3.141592653589793116;
const PI_LO: f64 = 1.2246467991473532e-16;

const PS0: f64 = 1.66666666666666657415e-1;
const PS1: f64 = -3.25565818622400915405e-1;
const PS2: f64 = 2.01212532134862925881e-1;
const PS3: f64 = -4.00555345006794114027e-2;
const PS4: f64 = 7.91534994289814532176e-4;
const PS5: f64 = 3.4793310759602116757e-5;
const QS1: f64 = -2.40339491173441421878;
const QS2: f64 = 2.02094576023350569471;
const QS3: f64 = -6.8828397160545329303e-1;
const QS4: f64 = 7.70381505559019352791e-2;

/// asin(√t)'s rational approximation R(t) = p(t)/q(t) shared by asin and acos.
fn asin_ratio(t: f64) -> f64 {
    let p = t * (PS0 + t * (PS1 + t * (PS2 + t * (PS3 + t * (PS4 + t * PS5)))));
    let q = 1.0 + t * (QS1 + t * (QS2 + t * (QS3 + t * QS4)));
    p / q
}

pub fn asin(x: f64) -> f64 {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3ff0_0000 {
        // |x| ≥ 1
        if ((ix - 0x3ff0_0000) as u32 | low_word(x)) == 0 {
            return x * PIO2_HI + x * PIO2_LO;
        }
        return f64::NAN;
    }
    if ix < 0x3fe0_0000 {
        // |x| < 0.5
        if ix < 0x3e40_0000 {
            return x; // |x| < 2^-27
        }
        return x + x * asin_ratio(x * x);
    }
    // 0.5 ≤ |x| < 1
    let w = 1.0 - x.abs();
    let t = w * 0.5;
    let r = asin_ratio(t);
    let s = t.sqrt();
    let result = if ix >= 0x3fef_3333 {
        // |x| > 0.975
        PIO2_HI - (2.0 * (s + s * r) - PIO2_LO)
    } else {
        let ws = with_low_word(s, 0);
        let c = (t - ws * ws) / (s + ws);
        let p = 2.0 * s * r - (PIO2_LO - 2.0 * c);
        let q = PIO4_HI - 2.0 * ws;
        PIO4_HI - (p - q)
    };
    if hx > 0 { result } else { -result }
}

pub fn acos(x: f64) -> f64 {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    if ix >= 0x3ff0_0000 {
        // |x| ≥ 1
        if ((ix - 0x3ff0_0000) as u32 | low_word(x)) == 0 {
            return if hx > 0 { 0.0 } else { PI + 2.0 * PIO2_LO };
        }
        return f64::NAN;
    }
    if ix < 0x3fe0_0000 {
        // |x| < 0.5
        if ix <= 0x3c60_0000 {
            return PIO2_HI + PIO2_LO; // |x| < 2^-57
        }
        return PIO2_HI - (x - (PIO2_LO - x * asin_ratio(x * x)));
    }
    if hx < 0 {
        // x < -0.5
        let z = (1.0 + x) * 0.5;
        let s = z.sqrt();
        let w = asin_ratio(z) * s - PIO2_LO;
        return PI - 2.0 * (s + w);
    }
    // x > 0.5
    let z = (1.0 - x) * 0.5;
    let s = z.sqrt();
    let df = with_low_word(s, 0);
    let c = (z - df * df) / (s + df);
    let w = asin_ratio(z) * s + c;
    2.0 * (df + w)
}

const ATAN_HI: [f64; 4] = [
    4.63647609000806093515e-1,
    7.85398163397448278999e-1,
    9.82793723247329054082e-1,
    1.570796326794896558,
];
const ATAN_LO: [f64; 4] = [
    2.26987774529616870924e-17,
    3.06161699786838301793e-17,
    1.39033110312309984516e-17,
    6.12323399573676603587e-17,
];
const AT0: f64 = 3.33333333333329318027e-1;
const AT1: f64 = -1.99999999998764832476e-1;
const AT2: f64 = 1.42857142725034663711e-1;
const AT3: f64 = -1.1111110405462355788e-1;
const AT4: f64 = 9.09088713343650656196e-2;
const AT5: f64 = -7.69187620504482999495e-2;
const AT6: f64 = 6.66107313738753120669e-2;
const AT7: f64 = -5.83357013379057348645e-2;
const AT8: f64 = 4.97687799461593236017e-2;
const AT9: f64 = -3.6531572744216915527e-2;
const AT10: f64 = 1.62858201153657823623e-2;

/// The reduction interval of `atan`: its breakpoint `atan_hi + atan_lo` and the reduced argument.
#[derive(Clone, Copy)]
enum AtanRange {
    /// |x| < 7/16: no reduction.
    Small,
    /// One of the four breakpoints atan(1/2), atan(1), atan(3/2), atan(∞).
    Reduced(usize),
}

pub fn atan(mut x: f64) -> f64 {
    let hx = high_word(x);
    let ix = hx & 0x7fff_ffff;
    let range;
    if ix >= 0x4410_0000 {
        // |x| ≥ 2^66
        if ix > 0x7ff0_0000 || (ix == 0x7ff0_0000 && low_word(x) != 0) {
            return x + x; // NaN
        }
        let (hi, lo) = (ATAN_HI[3], ATAN_LO[3]);
        return if hx > 0 { hi + lo } else { -hi - lo };
    }
    if ix < 0x3fdc_0000 {
        // |x| < 0.4375
        if ix < 0x3e20_0000 {
            return x; // |x| < 2^-29
        }
        range = AtanRange::Small;
    } else {
        x = x.abs();
        if ix < 0x3ff3_0000 {
            // |x| < 1.1875
            if ix < 0x3fe6_0000 {
                // 7/16 ≤ |x| < 11/16
                range = AtanRange::Reduced(0);
                x = (2.0 * x - 1.0) / (2.0 + x);
            } else {
                // 11/16 ≤ |x| < 19/16
                range = AtanRange::Reduced(1);
                x = (x - 1.0) / (x + 1.0);
            }
        } else if ix < 0x4003_8000 {
            // |x| < 2.4375
            range = AtanRange::Reduced(2);
            x = (x - 1.5) / (1.0 + 1.5 * x);
        } else {
            // 2.4375 ≤ |x| < 2^66
            range = AtanRange::Reduced(3);
            x = -1.0 / x;
        }
    }
    let z = x * x;
    let w = z * z;
    // The odd and even halves of sum(AT[i] z^(i+1)).
    let s1 = z * (AT0 + w * (AT2 + w * (AT4 + w * (AT6 + w * (AT8 + w * AT10)))));
    let s2 = w * (AT1 + w * (AT3 + w * (AT5 + w * (AT7 + w * AT9))));
    match range {
        AtanRange::Small => x - x * (s1 + s2),
        AtanRange::Reduced(id) => {
            let hi = ATAN_HI.get(id).copied().unwrap_or(f64::NAN);
            let lo = ATAN_LO.get(id).copied().unwrap_or(f64::NAN);
            let result = hi - (x * (s1 + s2) - lo - x);
            if hx < 0 { -result } else { result }
        }
    }
}

pub fn atan2(y: f64, x: f64) -> f64 {
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
        return atan(y); // x = 1
    }
    let m = ((hy as u32 >> 31) & 1) | ((hx as u32 >> 30) & 2); // 2·sign(x) + sign(y)

    if (iy as u32 | ly) == 0 {
        // y = ±0
        return match m {
            0 | 1 => y,
            2 => PI,
            _ => -PI,
        };
    }
    if (ix as u32 | lx) == 0 {
        return if hy < 0 { -PIO2_HI } else { PIO2_HI }; // x = ±0
    }
    if ix == 0x7ff0_0000 {
        // x = ±∞
        if iy == 0x7ff0_0000 {
            return match m {
                0 => PIO4_HI,
                1 => -PIO4_HI,
                2 => 3.0 * PIO4_HI,
                _ => -3.0 * PIO4_HI,
            };
        }
        return match m {
            0 => 0.0,
            1 => -0.0,
            2 => PI,
            _ => -PI,
        };
    }
    if iy == 0x7ff0_0000 {
        return if hy < 0 { -PIO2_HI } else { PIO2_HI }; // y = ±∞
    }

    let k = (iy - ix) >> 20;
    let z = if k > 60 {
        PIO2_HI + 0.5 * PI_LO // |y/x| > 2^60
    } else if hx < 0 && k < -60 {
        0.0 // |y|/x < -2^60
    } else {
        atan((y / x).abs())
    };
    match m {
        0 => z,
        1 => -z,
        2 => PI - (z - PI_LO),
        _ => z - PI_LO - PI,
    }
}

// Exponential and logarithm.

const O_THRESHOLD: f64 = 7.09782712893383973096e2;
const U_THRESHOLD: f64 = -7.4513321910194110842e2;
const LN2_HI: f64 = 6.9314718036912381649e-1;
const LN2_LO: f64 = 1.90821492927058770002e-10;
const INVLN2: f64 = 1.442695040888963387;
const TWOM1000: f64 = 9.3326361850321887899e-302;
const P1: f64 = 1.66666666666666019037e-1;
const P2: f64 = -2.77777777770155933842e-3;
const P3: f64 = 6.61375632143793436117e-5;
const P4: f64 = -1.6533902205465251539e-6;
const P5: f64 = 4.13813679705723846039e-8;

pub fn exp(mut x: f64) -> f64 {
    let mut hx = high_word(x);
    let xsb = (hx as u32 >> 31) & 1;
    hx &= 0x7fff_ffff;

    if hx >= 0x4086_2e42 {
        // |x| ≥ 709.78...
        if hx >= 0x7ff0_0000 {
            if ((hx & 0xf_ffff) as u32 | low_word(x)) != 0 {
                return x + x; // NaN
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

    let mut hi = 0.0;
    let mut lo = 0.0;
    let mut k = 0;
    if hx > 0x3fd6_2e42 {
        // |x| > 0.5 ln2
        if hx < 0x3ff0_a2b2 {
            // and |x| < 1.5 ln2
            hi = if xsb == 0 { x - LN2_HI } else { x + LN2_HI };
            lo = if xsb == 0 { LN2_LO } else { -LN2_LO };
            k = 1 - 2 * xsb as i32;
        } else {
            k = to_int32(INVLN2 * x + if xsb == 0 { 0.5 } else { -0.5 });
            hi = x - f64::from(k) * LN2_HI; // exact here
            lo = f64::from(k) * LN2_LO;
        }
        x = hi - lo;
    } else if hx < 0x3e30_0000 {
        // |x| < 2^-28
        return 1.0 + x;
    }

    let t = x * x;
    let c = x - t * (P1 + t * (P2 + t * (P3 + t * (P4 + t * P5))));
    if k == 0 {
        return 1.0 - ((x * c) / (c - 2.0) - x);
    }
    let y = 1.0 - (lo - (x * c) / (2.0 - c) - hi);
    if k >= -1021 {
        return with_high_word(y, high_word(y).wrapping_add(shl(k, 20)));
    }
    with_high_word(y, high_word(y).wrapping_add(shl(k + 1000, 20))) * TWOM1000
}

const TWO54: f64 = 1.8014398509481984e16;
const LG1: f64 = 6.66666666666673513e-1;
const LG2: f64 = 3.999999999940941908e-1;
const LG3: f64 = 2.857142874366239149e-1;
const LG4: f64 = 2.222219843214978396e-1;
const LG5: f64 = 1.818357216161805012e-1;
const LG6: f64 = 1.531383769920937332e-1;
const LG7: f64 = 1.479819860511658591e-1;

pub fn log(mut x: f64) -> f64 {
    let mut hx = high_word(x);
    let lx = low_word(x);
    let mut k = 0;
    if hx < 0x0010_0000 {
        // x < 2^-1022
        if ((hx & 0x7fff_ffff) as u32 | lx) == 0 {
            return f64::NEG_INFINITY;
        }
        if hx < 0 {
            return f64::NAN;
        }
        k -= 54; // subnormal: scale up
        x *= TWO54;
        hx = high_word(x);
    }
    if hx >= 0x7ff0_0000 {
        return x + x;
    }
    k += (hx >> 20) - 1023;
    hx &= 0x000f_ffff;
    let mut i = (hx + 0x95f64) & 0x10_0000;
    x = with_high_word(x, hx | (i ^ 0x3ff0_0000)); // normalise x or x/2
    k += i >> 20;
    let f = x - 1.0;
    let dk = f64::from(k);
    if (0x000f_ffff & (2 + hx)) < 3 {
        // |f| < 2^-20
        if f == 0.0 {
            return if k == 0 {
                0.0
            } else {
                dk * LN2_HI + dk * LN2_LO
            };
        }
        let r = f * f * (0.5 - 0.3333333333333333 * f);
        return if k == 0 {
            f - r
        } else {
            dk * LN2_HI - (r - dk * LN2_LO - f)
        };
    }
    let s = f / (2.0 + f);
    let z = s * s;
    i = hx - 0x6147a;
    let w = z * z;
    let j = 0x6b851 - hx;
    let t1 = w * (LG2 + w * (LG4 + w * LG6));
    let t2 = z * (LG1 + w * (LG3 + w * (LG5 + w * LG7)));
    i |= j;
    let r = t2 + t1;
    if i > 0 {
        let hfsq = 0.5 * f * f;
        if k == 0 {
            return f - (hfsq - s * (hfsq + r));
        }
        return dk * LN2_HI - (hfsq - (s * (hfsq + r) + dk * LN2_LO) - f);
    }
    if k == 0 {
        return f - s * (f - r);
    }
    dk * LN2_HI - (s * (f - r) - dk * LN2_LO - f)
}

// Hypotenuse and cube root.

pub fn hypot(x: f64, y: f64) -> f64 {
    let mut ha = high_word(x) & 0x7fff_ffff;
    let mut hb = high_word(y) & 0x7fff_ffff;
    let (mut a, mut b) = (x, y);
    if hb > ha {
        (a, b, ha, hb) = (y, x, hb, ha);
    }
    a = with_high_word(a, ha); // |a|
    b = with_high_word(b, hb); // |b|
    if ha - hb > 0x3c0_0000 {
        return a + b; // a/b > 2^60
    }
    let mut k = 0;
    if ha > 0x5f30_0000 {
        // a > 2^500
        if ha >= 0x7ff0_0000 {
            // Infinity or NaN: an infinity wins over NaN.
            let mut w = a + b;
            if ((ha & 0xf_ffff) as u32 | low_word(a)) == 0 {
                w = a;
            }
            if ((hb ^ 0x7ff0_0000) as u32 | low_word(b)) == 0 {
                w = b;
            }
            return w;
        }
        // Scale a and b by 2^-600.
        ha -= 0x2580_0000;
        hb -= 0x2580_0000;
        k += 600;
        a = with_high_word(a, ha);
        b = with_high_word(b, hb);
    }
    if hb < 0x20b0_0000 {
        // b < 2^-500
        if hb <= 0x000f_ffff {
            // Subnormal b or zero.
            if (hb as u32 | low_word(b)) == 0 {
                return a;
            }
            let t1 = from_words(0x7fd0_0000, 0); // 2^1022
            b *= t1;
            a *= t1;
            k -= 1022;
            ha = high_word(a);
            hb = high_word(b);
            if hb > ha {
                (a, b, ha, hb) = (b, a, hb, ha);
            }
        } else {
            // Scale a and b by 2^600.
            ha = ha.wrapping_add(0x2580_0000);
            hb = hb.wrapping_add(0x2580_0000);
            k -= 600;
            a = with_high_word(a, ha);
            b = with_high_word(b, hb);
        }
    }
    // Medium size a and b.
    let mut w = a - b;
    if w > b {
        let t1 = from_words(ha, 0);
        let t2 = a - t1;
        w = (t1 * t1 - (b * -b - t2 * (a + t1))).sqrt();
    } else {
        a += a;
        let y1 = from_words(hb, 0);
        let y2 = b - y1;
        let t1 = from_words(ha.wrapping_add(0x0010_0000), 0);
        let t2 = a - t1;
        w = (t1 * y1 - (w * -w - (t1 * y2 + t2 * b))).sqrt();
    }
    if k != 0 {
        from_words(0x3ff0_0000_i32.wrapping_add(shl(k, 20)), 0) * w
    } else {
        w
    }
}

const B1: i32 = 715_094_163; // (682 - 0.03306235651) · 2^20
const B2: i32 = 696_219_795; // (664 - 0.03306235651) · 2^20
const CB_C: f64 = 5.42857142857142815906e-1; // 19/35
const CB_D: f64 = -7.0530612244897961105e-1; // -864/1225
const CB_E: f64 = 1.41428571428571436819; // 99/70
const CB_F: f64 = 1.6071428571428572063; // 45/28
const CB_G: f64 = 3.57142857142857150787e-1; // 5/14

pub fn cbrt(mut x: f64) -> f64 {
    let mut hx = high_word(x);
    let sign = hx & i32::MIN;
    hx ^= sign;
    if hx >= 0x7ff0_0000 {
        return x + x; // NaN, ±∞
    }
    if (hx as u32 | low_word(x)) == 0 {
        return x; // ±0
    }
    x = with_high_word(x, hx); // |x|

    // A rough cube root to 5 bits.
    let mut t = if hx < 0x0010_0000 {
        // Subnormal.
        let t = from_words(0x4350_0000, 0) * x; // 2^54 · x
        with_high_word(t, high_word(t) / 3 + B2)
    } else {
        from_words(hx / 3 + B1, 0)
    };

    // To 23 bits.
    let mut r = (t * t) / x;
    let mut s = CB_C + r * t;
    t *= CB_G + CB_F / (s + CB_E + CB_D / s);

    // Chop to 20 bits and round up, so t is larger than cbrt(x).
    t = from_words(high_word(t).wrapping_add(1), 0);

    // One Newton step to 53 bits, with an error below 0.667 ulps.
    s = t * t; // exact
    r = x / s;
    let w = t + t;
    r = (r - t) / (w + r); // r - s is exact
    t += t * r;

    with_high_word(t, high_word(t) | sign)
}

#[cfg(test)]
mod v8_model;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn to_int32_wraps_like_javascript() {
        assert_eq!(to_int32(3.7), 3);
        assert_eq!(to_int32(-3.7), -3);
        assert_eq!(to_int32(f64::NAN), 0);
        assert_eq!(to_int32(f64::INFINITY), 0);
        assert_eq!(to_int32(2_147_483_648.0), i32::MIN);
        assert_eq!(to_int32(4_294_967_297.0), 1);
        assert_eq!(to_int32(-2_147_483_649.0), i32::MAX);
    }

    #[test]
    fn words_round_trip() {
        let x = -1.5e-300;
        assert_eq!(from_words(high_word(x), low_word(x)).to_bits(), x.to_bits());
        assert_eq!(with_low_word(1.0, 1).to_bits(), 0x3ff0_0000_0000_0001);
        assert_eq!(with_high_word(1.0, 0x4000_0000), 2.0);
    }

    #[test]
    fn scalbn_is_exact() {
        assert_eq!(scalbn(1.0, 1023).to_bits(), 0x7fe0_0000_0000_0000);
        assert_eq!(scalbn(1.0, -1074), f64::from_bits(1));
        assert_eq!(scalbn(3.0, 2000), f64::INFINITY);
        assert_eq!(scalbn(f64::MAX, -2000).to_bits(), 0x02ef_ffff_ffff_ffff);
        assert_eq!(scalbn(1.0, -2098), 0.0);
    }
}
