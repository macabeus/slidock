//! `Number.prototype.toFixed` from the exact binary value: x = m · 2^e, so x · 10^digits rounded half up is an
//! integer computation.

/// A non-negative integer in base 2^32, least significant limb first.
struct Big(Vec<u32>);

impl Big {
    fn new(value: u64) -> Big {
        let mut big = Big(vec![value as u32, (value >> 32) as u32]);
        big.trim();
        big
    }

    fn trim(&mut self) {
        while self.0.last() == Some(&0) {
            self.0.pop();
        }
    }

    fn mul_small(&mut self, k: u32) {
        let mut carry = 0u64;
        for limb in &mut self.0 {
            let t = u64::from(*limb) * u64::from(k) + carry;
            *limb = t as u32;
            carry = t >> 32;
        }
        if carry > 0 {
            self.0.push(carry as u32);
        }
    }

    /// Adds 2^k.
    fn add_pow2(&mut self, k: usize) {
        let (word, bit) = (k / 32, k % 32);
        if self.0.len() <= word {
            self.0.resize(word + 1, 0);
        }
        let mut carry = 1u64 << bit;
        for limb in self.0.iter_mut().skip(word) {
            let t = u64::from(*limb) + carry;
            *limb = t as u32;
            carry = t >> 32;
            if carry == 0 {
                break;
            }
        }
        if carry > 0 {
            self.0.push(carry as u32);
        }
    }

    /// Divides by 2^k, rounding down.
    fn shr(&mut self, k: usize) {
        let (words, bits) = (k / 32, k % 32);
        if words >= self.0.len() {
            self.0.clear();
            return;
        }
        self.0.drain(..words);
        if bits > 0 {
            let mut carry = 0u32;
            for limb in self.0.iter_mut().rev() {
                let next = *limb << (32 - bits);
                *limb = (*limb >> bits) | carry;
                carry = next;
            }
        }
        self.trim();
    }

    /// Divides by `d`, returning the remainder.
    fn div_small(&mut self, d: u32) -> u32 {
        let mut rem = 0u64;
        for limb in self.0.iter_mut().rev() {
            let t = (rem << 32) | u64::from(*limb);
            *limb = (t / u64::from(d)) as u32;
            rem = t % u64::from(d);
        }
        self.trim();
        rem as u32
    }

    fn into_decimal(mut self) -> String {
        let mut chunks = Vec::new();
        while !self.0.is_empty() {
            chunks.push(self.div_small(1_000_000_000));
        }
        let mut out = String::new();
        for (i, chunk) in chunks.iter().rev().enumerate() {
            if i == 0 {
                out.push_str(&chunk.to_string());
            } else {
                out.push_str(&format!("{chunk:09}"));
            }
        }
        if out.is_empty() {
            out.push('0');
        }
        out
    }
}

/// The digits of `round_half_up(|x| · 10^digits)` for a finite `x`.
fn scaled_digits(x: f64, digits: u8) -> String {
    let bits = x.abs().to_bits();
    let biased = (bits >> 52) as i32;
    let fraction = bits & ((1 << 52) - 1);
    let (mantissa, exponent) = if biased == 0 {
        (fraction, -1074)
    } else {
        (fraction | (1 << 52), biased - 1075)
    };
    let mut n = Big::new(mantissa);
    for _ in 0..digits {
        n.mul_small(10);
    }
    if exponent >= 0 {
        for _ in 0..exponent {
            n.mul_small(2);
        }
    } else {
        // ⌊(m · 10^d + 2^(k-1)) / 2^k⌋: a tie rounds up, to the larger n.
        let k = exponent.unsigned_abs() as usize;
        n.add_pow2(k - 1);
        n.shr(k);
    }
    n.into_decimal()
}

/// `x.toFixed(digits)` for finite |x| < 1e21.
pub(crate) fn to_fixed_finite(x: f64, digits: u8) -> String {
    let mut out = String::new();
    if x < 0.0 {
        out.push('-');
    }
    let n = scaled_digits(x, digits);
    let digits = usize::from(digits);
    if digits == 0 {
        out.push_str(&n);
        return out;
    }
    let padded = if n.len() <= digits {
        format!("{}{n}", "0".repeat(digits + 1 - n.len()))
    } else {
        n
    };
    if let Some((int, frac)) = padded.split_at_checked(padded.len() - digits) {
        out.push_str(int);
        out.push('.');
        out.push_str(frac);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rounds_the_exact_binary_value() {
        assert_eq!(to_fixed_finite(1.005, 2), "1.00"); // 1.00499999999999989…
        assert_eq!(to_fixed_finite(1.125, 2), "1.13"); // an exact tie rounds up
        assert_eq!(to_fixed_finite(-1.125, 2), "-1.13");
        assert_eq!(to_fixed_finite(-0.0001, 2), "-0.00");
        assert_eq!(to_fixed_finite(-0.0, 2), "0.00");
        assert_eq!(to_fixed_finite(0.5, 0), "1");
        assert_eq!(to_fixed_finite(2.5, 0), "3");
        assert_eq!(to_fixed_finite(123.456, 0), "123");
        assert_eq!(to_fixed_finite(5e-324, 3), "0.000");
        assert_eq!(to_fixed_finite(0.1, 20), "0.10000000000000000555");
        assert_eq!(
            to_fixed_finite(999_999_999_999_999_900_000.0, 1),
            "999999999999999868928.0"
        );
    }

    #[test]
    fn big_integer_steps() {
        let mut n = Big::new(u64::MAX);
        n.mul_small(10);
        assert_eq!(Big(n.0.clone()).into_decimal(), "184467440737095516150");
        n.add_pow2(70);
        n.shr(3);
        assert_eq!(n.into_decimal(), "170632382681813352446");
        assert_eq!(Big::new(0).into_decimal(), "0");
    }
}
