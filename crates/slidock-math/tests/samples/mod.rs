//! The sample generator of `tests/data/samples.mjs`, step for step: the same seeds and IEEE operations give the
//! same inputs the vector files were made from.

#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code shared by several test crates"
)]

pub fn canonical_bits(x: f64) -> u64 {
    if x.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        x.to_bits()
    }
}

/// splitmix64.
pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Self {
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    pub fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / 9_007_199_254_740_992.0
    }

    pub fn below(&mut self, n: f64) -> i32 {
        (self.unit() * n).floor() as i32
    }
}

/// FNV-1a over the row key's UTF-16 code units (all ASCII here).
pub fn seed_for(key: &str) -> u64 {
    key.bytes().fold(0xcbf2_9ce4_8422_2325, |h, c| {
        (h ^ u64::from(c)).wrapping_mul(0x100_0000_01b3)
    })
}

pub fn pow2(e: i32) -> f64 {
    if e >= -1022 {
        f64::from_bits(((e + 1023) as u64) << 52)
    } else {
        f64::from_bits(1u64 << (e + 1074))
    }
}

fn ulp_step(x: f64, k: i32) -> f64 {
    f64::from_bits(x.to_bits().wrapping_add_signed(i64::from(k)))
}

/// A double whose high word is uniform in `[lo, lo + count)`, under a random low word.
fn high_word(lo: u32, count: u32, rng: &mut Rng) -> f64 {
    let high = lo.wrapping_add(rng.below(f64::from(count)) as u32);
    let low = rng.next() as u32;
    f64::from_bits((u64::from(high) << 32) | u64::from(low))
}

/// One distribution of `DISTRIBUTIONS`.
pub enum Distribution {
    Uniform(f64, f64),
    SignedPow2(i32, i32),
    PositivePow2(i32, i32),
    AnyBits,
    Subnormal,
    NearHalfPi(f64),
    NearOne,
    /// High words uniform in `[lo, lo + count)` under a random low word, one `(lo, count)` per argument (the last
    /// one repeats); params are hex.
    HighWord(Vec<(u32, u32)>),
    /// n·(π/2) for n < count, plus an offset drawn as `HighWord` from `(lo, count)`: the trigonometric kernels see
    /// the offset's high word with a tail; params are hex.
    ReducedHighWord(u32, (u32, u32)),
}

impl Distribution {
    pub fn parse(name: &str, params: &str) -> Distribution {
        if name == "highWord" {
            let words: Vec<u32> = params
                .split(',')
                .map(|p| u32::from_str_radix(p, 16).unwrap())
                .collect();
            assert!(
                !words.is_empty() && words.len().is_multiple_of(2),
                "highWord {params}"
            );
            return Distribution::HighWord(words.chunks(2).map(|w| (w[0], w[1])).collect());
        }
        if name == "reducedHighWord" {
            let words: Vec<u32> = params
                .split(',')
                .map(|p| u32::from_str_radix(p, 16).unwrap())
                .collect();
            let [count, lo, span] = words[..] else {
                panic!("reducedHighWord {params}")
            };
            return Distribution::ReducedHighWord(count, (lo, span));
        }
        let f: Vec<f64> = params
            .split(',')
            .filter(|p| !p.is_empty())
            .map(|p| p.parse().unwrap())
            .collect();
        match name {
            "uniform" => Distribution::Uniform(f[0], f[1]),
            "signedPow2" => Distribution::SignedPow2(f[0] as i32, f[1] as i32),
            "positivePow2" => Distribution::PositivePow2(f[0] as i32, f[1] as i32),
            "anyBits" => Distribution::AnyBits,
            "subnormal" => Distribution::Subnormal,
            "nearHalfPi" => Distribution::NearHalfPi(f[0]),
            "nearOne" => Distribution::NearOne,
            _ => panic!("unknown distribution {name}"),
        }
    }

    /// The input for argument `arg` (0-based) of one call.
    pub fn draw(&self, arg: usize, rng: &mut Rng) -> f64 {
        match *self {
            Distribution::Uniform(a, b) => a + (b - a) * rng.unit(),
            Distribution::SignedPow2(lo, count) => {
                let u = rng.unit();
                (u - 0.5) * pow2(lo + rng.below(f64::from(count)))
            }
            Distribution::PositivePow2(lo, count) => {
                let u = rng.unit();
                u * pow2(lo + rng.below(f64::from(count)))
            }
            Distribution::AnyBits => f64::from_bits(rng.next()),
            Distribution::Subnormal => f64::from_bits(rng.next() & 0x800f_ffff_ffff_ffff),
            Distribution::NearHalfPi(count) => {
                let n = rng.below(count);
                ulp_step(
                    f64::from(n) * (std::f64::consts::PI / 2.0),
                    rng.below(17.0) - 8,
                )
            }
            Distribution::NearOne => {
                let sign = if rng.below(2.0) == 0 { 1.0 } else { -1.0 };
                sign * (1.0 - pow2(-1 - rng.below(52.0)))
            }
            Distribution::HighWord(ref ranges) => {
                let (lo, count) = ranges[arg.min(ranges.len() - 1)];
                high_word(lo, count, rng)
            }
            Distribution::ReducedHighWord(count, (lo, span)) => {
                let n = rng.below(f64::from(count));
                f64::from(n) * (std::f64::consts::PI / 2.0) + high_word(lo, span, rng)
            }
        }
    }
}

pub fn parse_bits(hex: &str) -> f64 {
    f64::from_bits(u64::from_str_radix(hex, 16).unwrap())
}

/// Distance in ulps between two doubles (0 for two NaNs, and for +0 against -0 only when equal bits).
pub fn ulps(a: f64, b: f64) -> u64 {
    if a.is_nan() && b.is_nan() {
        return 0;
    }
    if a.is_nan() || b.is_nan() {
        return u64::MAX;
    }
    // Map the sign-magnitude bit patterns onto a monotonic integer line.
    let line = |x: f64| {
        let bits = x.to_bits() as i64;
        if bits < 0 { i64::MIN - bits } else { bits }
    };
    line(a).abs_diff(line(b))
}

/// A sweep row of `fdlibm-sweep*.tsv`: `fn  dist  params  count  sha256`.
pub struct Sweep {
    pub name: String,
    pub key: String,
    pub count: usize,
    pub digest: String,
}

pub fn sweeps(text: &str) -> Vec<Sweep> {
    text.lines()
        .filter(|line| !line.starts_with('#'))
        .map(|line| {
            let cols: Vec<&str> = line.split('\t').collect();
            Sweep {
                name: cols[0].to_owned(),
                key: format!("{}\t{}\t{}", cols[0], cols[1], cols[2]),
                count: cols[3].parse().unwrap(),
                digest: cols[4].to_owned(),
            }
        })
        .collect()
}

impl Sweep {
    /// The SHA-256 of `f`'s big-endian result bits over the row's samples; `f` returns `None` to leave a sample
    /// out, and the digest is `None` when `f` leaves out every sample.
    pub fn digest_of(&self, f: impl Fn(&[f64]) -> Option<f64>) -> Option<String> {
        let mut cols = self.key.split('\t');
        let (_, dist, params) = (
            cols.next().unwrap(),
            cols.next().unwrap(),
            cols.next().unwrap(),
        );
        let distribution = Distribution::parse(dist, params);
        let mut rng = Rng::new(seed_for(&self.key));
        let arity = if self.name == "atan2" || self.name == "hypot" {
            2
        } else {
            1
        };
        let mut hasher = slidock_canon::Sha256Hex::new();
        let mut hashed = 0;
        for _ in 0..self.count {
            let args: Vec<f64> = (0..arity)
                .map(|arg| distribution.draw(arg, &mut rng))
                .collect();
            if let Some(r) = f(&args) {
                hasher.update(canonical_bits(r).to_be_bytes());
                hashed += 1;
            }
        }
        (hashed > 0).then(|| hasher.finish())
    }
}

/// Whether hypot(x, y) takes fdlibm's subnormal-b branch, where FreeBSD's fix (which the port has and the JDK's
/// StrictMath lacks) changes the result. `Jdk.java` has the same predicate.
pub fn subnormal_hypot(x: f64, y: f64) -> bool {
    let high = |v: f64| (v.to_bits() >> 32) as i32 & 0x7fff_ffff;
    let (mut ha, mut hb, mut b) = (high(x), high(y), y);
    if hb > ha {
        (ha, hb, b) = (hb, ha, x);
    }
    ha - hb <= 0x3c0_0000 && hb <= 0x000f_ffff && b != 0.0
}
