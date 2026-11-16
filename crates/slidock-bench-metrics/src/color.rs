//! `color@1`: CIE Lab (sRGB, D65) averaged over 8×8 tiles, then CIEDE2000 per tile. It catches hue errors of equal
//! luminance (a wrong accent slot, a red↔green swap), to which luma SSIM is blind.

use core::f64::consts::PI;
use core::marker::PhantomData;

use crate::MetricsError;
use crate::image::Rgba;
use crate::js::{percentile, pow_int, typed_order};
use crate::libm::Libm;

pub const COLOR_VERSION: &str = "color@1";
const TILE: usize = 8;
const WHITE: [f64; 3] = [0.95047, 1.0, 1.08883];

/// CIE L*a*b*.
pub type Lab = [f64; 3];

/// sRGB to linear light per 8-bit value: `v/12.92` up to 0.04045, else `((v + 0.055)/1.055)^2.4` correctly rounded
/// (as macOS's `pow` returns it for every entry), `v = c/255`.
#[rustfmt::skip]
const LINEAR_BITS: [u64; 256] = [
    0x00000000_00000000, 0x3f33e456_77c176f7, 0x3f43e456_77c176f7, 0x3f4dd681_b3a23272,
    0x3f53e456_77c176f7, 0x3f58dd6c_15b1d4b4, 0x3f5dd681_b3a23272, 0x3f6167cb_a8c94818,
    0x3f63e456_77c176f7, 0x3f6660e1_46b9a5d5, 0x3f68dd6c_15b1d4b4, 0x3f6b6a31_b5259c99,
    0x3f6e1e31_d70c99dd, 0x3f707c38_bf8583a9, 0x3f71fcc2_beed6421, 0x3f7390ff_af95e279,
    0x3f753936_cc7bc928, 0x3f76f5ad_db50c915, 0x3f78c6a9_4031b561, 0x3f7aac6c_0fb97351,
    0x3f7ca738_1f9f602b, 0x3f7eb74e_160978d0, 0x3f806e76_bbda92b8, 0x3f818c2a_5a8a8044,
    0x3f82b4e0_9b3f0ae3, 0x3f83e8b7_b3bde965, 0x3f8527cd_60af8b85, 0x3f86723e_ea8d3709,
    0x3f87c829_2a3db6b3, 0x3f8929a8_8d67b521, 0x3f8a96d9_1a8016bd, 0x3f8c0fd6_7499fab6,
    0x3f8d94bb_defd740e, 0x3f8f25a4_4089883f, 0x3f906155_1372c694, 0x3f9135f3_e4c2cce2,
    0x3f9210bb_8642b172, 0x3f92f1b8_c1ae46bd, 0x3f93d8f8_39b79c0b, 0x3f94c686_6b3e9fa4,
    0x3f95ba6f_ae794313, 0x3f96b4c0_380d2dee, 0x3f97b584_1a1bf3ac, 0x3f98bcc7_4542addb,
    0x3f99ca95_898dc8b5, 0x3f9adefa_9761c020, 0x3f9bfa02_00597bd9, 0x3f9d1bb7_381aec1f,
    0x3f9e4425_95227bca, 0x3f9f7358_5185e1b5, 0x3fa054ad_45d76878, 0x3fa0f31b_a386ff26,
    0x3fa194fc_b663747b, 0x3fa23a55_e62a662a, 0x3fa2e32c_8e148d11, 0x3fa38f85_fd21eacf,
    0x3fa43f67_766310ff, 0x3fa4f2d6_313fa8d0, 0x3fa5a9d7_59ba5ed0, 0x3fa66470_10b254ee,
    0x3fa722a5_6c2239ee, 0x3fa7e47c_775d2427, 0x3fa8a9fa_33494b07, 0x3fa97323_9698b9cc,
    0x3faa3ffd_8e001389, 0x3fab108c_fc6b7fbc, 0x3fabe4d6_bb31d522, 0x3facbcdf_9a4616f2,
    0x3fad98ac_60675833, 0x3fae7841_cb4f16df, 0x3faf5ba4_8fde2048, 0x3fb0216c_ad240765,
    0x3fb096f2_671eb815, 0x3fb10e65_c38a5192, 0x3fb187c9_0bf8bce2, 0x3fb2031e_85f5d6da,
    0x3fb28068_731a1952, 0x3fb2ffa9_111cb94b, 0x3fb380e2_99e53f92, 0x3fb40417_439ca10f,
    0x3fb48949_40bddbfb, 0x3fb5107a_c0261e59, 0x3fb599ad_ed247aac, 0x3fb624e4_ef892ed4,
    0x3fb6b221_ebb4817e, 0x3fb74167_02a539d1, 0x3fb7d2b6_5206b527, 0x3fb86611_f43e9e6a,
    0x3fb8fb7c_007a4a70, 0x3fb992f6_8abbbc89, 0x3fba2c83_a3e6566d, 0x3fbac825_59cb3644,
    0x3fbb65dd_b7354604, 0x3fbc05ae_c3f4fe5e, 0x3fbca79a_84ebe030, 0x3fbd4ba2_fc17a6a5,
    0x3fbdf1ca_289d34b8, 0x3fbe9a12_06d34003, 0x3fbf447c_904cbb4e, 0x3fbff10b_bbe302c2,
    0x3fc04fe0_bedfe5f1, 0x3fc0a84f_e3b36d8f, 0x3fc101d4_43dfc06f, 0x3fc15c6e_d58eefdf,
    0x3fc1b820_8da5fef0, 0x3fc214ea_5fc9514a, 0x3fc272cd_3e610123, 0x3fc2d1ca_1a9d1cfb,
    0x3fc331e1_e479cdf5, 0x3fc39315_8ac3674e, 0x3fc3f565_fb1a5fd5, 0x3fc458d4_21f735df,
    0x3fc4bd60_eaae3e73, 0x3fc5230d_3f736034, 0x3fc589da_095dbaa1, 0x3fc5f1c8_306b3a3c,
    0x3fc65ad8_9b841a2b, 0x3fc6c50c_307e53bf, 0x3fc73063_d420fc80, 0x3fc79ce0_6a279303,
    0x3fc80a82_d5453b5d, 0x3fc8794b_f727eb3f, 0x3fc8e93c_b07b8679, 0x3fc95a55_e0ecec0b,
    0x3fc9cc98_672cf47e, 0x3fca4005_20f3619c, 0x3fcab49c_eb01c003, 0x3fcb2a60_a1263b0a,
    0x3fcba151_1e3e632d, 0x3fcc196f_3c39e76f, 0x3fcc92bb_d41d41fe, 0x3fcd0d37_be045851,
    0x3fcd88e3_d1250f68, 0x3fce05c0_e3d1d3e0, 0x3fce83cf_cb7c16f0, 0x3fcf0311_5cb6bfd3,
    0x3fcf8386_6b38924d, 0x3fd00297_e4ef4553, 0x3fd04407_2557177a, 0x3fd08611_5f6beb3a,
    0x3fd0c8b6_fb5c735e, 0x3fd10bf8_60ef039a, 0x3fd14fd5_f782a5a6, 0x3fd19450_26102997,
    0x3fd1d967_532b31b1, 0x3fd21f1b_e50339e7, 0x3fd2656e_41649ae3, 0x3fd2ac5e_cdb988f8,
    0x3fd2f3ed_ef0b0ed8, 0x3fd33c1c_0a020438, 0x3fd384e9_82e800b1, 0x3fd3ce56_bda84a81,
    0x3fd41864_1dd0c1bc, 0x3fd46312_0692c7af, 0x3fd4ae60_dac4229d, 0x3fd4fa50_fcdfde15,
    0x3fd546e2_cf0727a9, 0x3fd59416_b3022858, 0x3fd5e1ed_0a40daab, 0x3fd63066_35dbdd7b,
    0x3fd67f82_969543a2, 0x3fd6cf42_8cd96079, 0x3fd71fa6_78bf915d, 0x3fd770ae_ba0b042a,
    0x3fd7c25b_b02b7ac5, 0x3fd814ad_ba3e0bd9, 0x3fd867a5_370de0b1, 0x3fd8bb42_8514f067,
    0x3fd90f86_027cb84e, 0x3fd96470_0d1ef1b1, 0x3fd9ba01_02864521, 0x3fda1039_3feefafd,
    0x3fda6719_2247a9be, 0x3fdabea1_0631e195, 0x3fdb16d1_4802d5ca, 0x3fdb6faa_43c403bb,
    0x3fdbc92c_5533d785, 0x3fdc2357_d7c64e5d, 0x3fdc7e2d_26a596de, 0x3fdcd9ac_9cb2aef2,
    0x3fdd35d6_9485ffc5, 0x3fdd92ab_686ff782, 0x3fddf02b_7279a10d, 0x3fde4e57_0c6539c5,
    0x3fdead2e_8faec526, 0x3fdf0cb2_558c9ea4, 0x3fdf6ce2_b6f00983, 0x3fdfcdc0_0c85bec2,
    0x3fe017a5_575b3cb2, 0x3fe048c1_7ad3c04b, 0x3fe07a34_9c9d9837, 0x3fe0abfe_e888c050,
    0x3fe0de20_8a4444c8, 0x3fe11099_ad5e83eb, 0x3fe1436a_7d456eef, 0x3fe17693_2546ca12,
    0x3fe1aa13_d0906bda, 0x3fe1ddec_aa307b85, 0x3fe2121d_dd15aece, 0x3fe246a7_940f86d1,
    0x3fe27b89_f9ce8c4b, 0x3fe2b0c5_38e48b07, 0x3fe2e659_7bc4cca0, 0x3fe31c46_ecc4528d,
    0x3fe3528d_b61a0f73, 0x3fe3892e_01df1fcc, 0x3fe3c027_fa0f01eb, 0x3fe3f77b_c887cd3b,
    0x3fe42f29_970a68f8, 0x3fe46731_8f3ac22d, 0x3fe49f93_daa00113, 0x3fe4d850_a2a4bde1,
    0x3fe51168_109734e5, 0x3fe54ada_4da97a1b, 0x3fe584a7_82f1ac23, 0x3fe5becf_d96a2698,
    0x3fe5f953_79f1b3ed, 0x3fe63432_8d4bbe97, 0x3fe66f6d_3c2081cf, 0x3fe6ab03_aefd39aa,
    0x3fe6e6f6_0e5452b1, 0x3fe72344_827d98f6, 0x3fe75fef_33b6669b, 0x3fe79cf6_4a21d1e2,
    0x3fe7da59_edc8dab0, 0x3fe8181a_469a9787, 0x3fe85637_7c6c6224, 0x3fe894b1_b6fa0377,
    0x3fe8d389_1de5df49, 0x3fe912bd_d8b91f45, 0x3fe95250_0ee3dda5, 0x3fe9923f_e7bd4f67,
    0x3fe9d28d_8a83edfc, 0x3fea1339_1e5da09f, 0x3fea5442_ca57e52e, 0x3fea95aa_b567f88f,
    0x3fead771_066afec2, 0x3feb1995_e4262a69, 0x3feb5c19_7546e3f8, 0x3feb9efb_e062f086,
    0x3febe23d_4bf8981b, 0x3fec25dd_de6ecbbb, 0x3fec69dd_be154af1, 0x3fecae3d_1124c90b,
    0x3fecf2fb_fdbf11f1, 0x3fed381a_a9ef2e82, 0x3fed7d99_3ba988d4, 0x3fedc377_d8cc0fd5,
    0x3fee09b6_a71e5aa6, 0x3fee5055_cc51cbb4, 0x3fee9755_6e01b351, 0x3feedeb5_b1b37216,
    0x3fef2676_bcd69ade, 0x3fef6e98_b4c51466, 0x3fefb71b_bec33ab2, 0x3ff00000_00000000,
];

fn linear(c: u8) -> f64 {
    f64::from_bits(LINEAR_BITS.get(usize::from(c)).copied().unwrap_or(0))
}

fn lab_f<M: Libm>(t: f64) -> f64 {
    if t > 216.0 / 24389.0 {
        M::cbrt(t)
    } else {
        ((24389.0 / 27.0) * t + 16.0) / 116.0
    }
}

/// An 8-bit sRGB colour in CIE Lab.
pub fn srgb_to_lab<M: Libm>(r: u8, g: u8, b: u8) -> Lab {
    let (r, g, b) = (linear(r), linear(g), linear(b));
    let x = (0.4124564 * r + 0.3575761 * g + 0.1804375 * b) / WHITE[0];
    let y = (0.2126729 * r + 0.7151522 * g + 0.072175 * b) / WHITE[1];
    let z = (0.0193339 * r + 0.119192 * g + 0.9503041 * b) / WHITE[2];
    let (fx, fy, fz) = (lab_f::<M>(x), lab_f::<M>(y), lab_f::<M>(z));
    [116.0 * fy - 16.0, 500.0 * (fx - fy), 200.0 * (fy - fz)]
}

fn deg(rad: f64) -> f64 {
    (rad * 180.0) / PI
}

fn rad(degrees: f64) -> f64 {
    (degrees * PI) / 180.0
}

/// The hue angle in degrees, 0 for a neutral.
fn hue<M: Libm>(a: f64, b: f64) -> f64 {
    if a == 0.0 && b == 0.0 {
        0.0
    } else {
        (deg(M::atan2(b, a)) + 360.0) % 360.0
    }
}

/// CIEDE2000 colour difference (Sharma, Wu and Dalal's formulation).
pub fn delta_e00<M: Libm>([l1, a1, b1]: Lab, [l2, a2, b2]: Lab) -> f64 {
    let p7 = |x: f64| pow_int(x, 7);
    let sq = |x: f64| pow_int(x, 2);
    let c1 = M::hypot(a1, b1);
    let c2 = M::hypot(a2, b2);
    let cbar = (c1 + c2) / 2.0;
    let g = 0.5 * (1.0 - (p7(cbar) / (p7(cbar) + p7(25.0))).sqrt());
    let a1p = (1.0 + g) * a1;
    let a2p = (1.0 + g) * a2;
    let c1p = M::hypot(a1p, b1);
    let c2p = M::hypot(a2p, b2);
    let h1p = hue::<M>(a1p, b1);
    let h2p = hue::<M>(a2p, b2);
    let dlp = l2 - l1;
    let dcp = c2p - c1p;
    let chroma = c1p * c2p != 0.0;
    let mut dhp = 0.0;
    if chroma {
        dhp = h2p - h1p;
        if dhp > 180.0 {
            dhp -= 360.0;
        } else if dhp < -180.0 {
            dhp += 360.0;
        }
    }
    let d_hp = 2.0 * (c1p * c2p).sqrt() * M::sin(rad(dhp / 2.0));
    let lbarp = (l1 + l2) / 2.0;
    let cbarp = (c1p + c2p) / 2.0;
    let mut hbarp = h1p + h2p;
    if chroma {
        if (h1p - h2p).abs() <= 180.0 {
            hbarp /= 2.0;
        } else if h1p + h2p < 360.0 {
            hbarp = (h1p + h2p + 360.0) / 2.0;
        } else {
            hbarp = (h1p + h2p - 360.0) / 2.0;
        }
    }
    let t = 1.0 - 0.17 * M::cos(rad(hbarp - 30.0))
        + 0.24 * M::cos(rad(2.0 * hbarp))
        + 0.32 * M::cos(rad(3.0 * hbarp + 6.0))
        - 0.2 * M::cos(rad(4.0 * hbarp - 63.0));
    let d_theta = 30.0 * M::exp(-sq((hbarp - 275.0) / 25.0));
    let rc = 2.0 * (p7(cbarp) / (p7(cbarp) + p7(25.0))).sqrt();
    let sl = 1.0 + (0.015 * sq(lbarp - 50.0)) / (20.0 + sq(lbarp - 50.0)).sqrt();
    let sc = 1.0 + 0.045 * cbarp;
    let sh = 1.0 + 0.015 * cbarp * t;
    let rt = -M::sin(rad(2.0 * d_theta)) * rc;
    (sq(dlp / sl) + sq(dcp / sc) + sq(d_hp / sh) + rt * (dcp / sc) * (d_hp / sh)).sqrt()
}

/// The mean Lab of every 8×8 tile (edge tiles hold fewer pixels), row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct TileLab {
    pub columns: usize,
    pub rows: usize,
    pub tiles: Vec<Lab>,
}

/// Sums the per-pixel Lab of a region and divides by its pixel count.
struct LabMean<M> {
    sum: Lab,
    n: usize,
    libm: PhantomData<M>,
}

impl<M: Libm> LabMean<M> {
    fn new() -> Self {
        LabMean {
            sum: [0.0; 3],
            n: 0,
            libm: PhantomData,
        }
    }

    fn add(&mut self, [r, g, b, _]: [u8; 4]) {
        let [l, a, bb] = srgb_to_lab::<M>(r, g, b);
        self.sum[0] += l;
        self.sum[1] += a;
        self.sum[2] += bb;
        self.n += 1;
    }

    fn mean(&self) -> Lab {
        let n = self.n as f64;
        [self.sum[0] / n, self.sum[1] / n, self.sum[2] / n]
    }
}

pub fn tile_lab<M: Libm>(image: &Rgba) -> TileLab {
    let (width, height) = (image.width(), image.height());
    let columns = width.div_ceil(TILE);
    let rows = height.div_ceil(TILE);
    let data = image.data();
    let mut tiles = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for column in 0..columns {
            let mut mean = LabMean::<M>::new();
            for y in row * TILE..((row + 1) * TILE).min(height) {
                let start = (y * width + column * TILE) * 4;
                let end = (y * width + ((column + 1) * TILE).min(width)) * 4;
                for &pixel in data.get(start..end).unwrap_or_default().as_chunks::<4>().0 {
                    mean.add(pixel);
                }
            }
            tiles.push(mean.mean());
        }
    }
    TileLab {
        columns,
        rows,
        tiles,
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColorScore {
    pub mean: f64,
    pub p99: f64,
}

/// Tile ΔE00 between two same-size images at the metric size: the mean and the 99th percentile.
pub fn color_difference<M: Libm>(a: &Rgba, b: &Rgba) -> Result<ColorScore, MetricsError> {
    a.check_same_size(b)?;
    let ta = tile_lab::<M>(a).tiles;
    let tb = tile_lab::<M>(b).tiles;
    let differences: Vec<f64> = ta
        .iter()
        .zip(&tb)
        .map(|(x, y)| delta_e00::<M>(*x, *y))
        .collect();
    let total = differences.iter().fold(0.0, |sum, value| sum + value);
    let mut sorted = differences.clone();
    sorted.sort_by(|p, q| typed_order(*p, *q));
    Ok(ColorScore {
        mean: total / differences.len() as f64,
        p99: percentile(&sorted, 0.99),
    })
}

/// The mean Lab of a region in metric pixels (`[x0, x1) × [y0, y1)`, rounded outward, clamped), for explain's
/// restyled check; `None` when the region holds no pixel.
pub fn region_lab<M: Libm>(image: &Rgba, x0: f64, y0: f64, x1: f64, y1: f64) -> Option<Lab> {
    let (width, height) = (image.width() as f64, image.height() as f64);
    let (xs, xe) = (
        crate::js::js_max(0.0, x0.floor()),
        crate::js::js_min(width, x1.ceil()),
    );
    let (ys, ye) = (
        crate::js::js_max(0.0, y0.floor()),
        crate::js::js_min(height, y1.ceil()),
    );
    let mut mean = LabMean::<M>::new();
    let mut y = ys;
    while y < ye {
        let mut x = xs;
        while x < xe {
            let i = ((y * width + x) * 4.0) as usize;
            if let Some(&[r, g, b, a]) = image.data().get(i..i + 4) {
                mean.add([r, g, b, a]);
            }
            x += 1.0;
        }
        y += 1.0;
    }
    (mean.n > 0).then(|| mean.mean())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::libm::Fdlibm;

    #[test]
    fn linear_table_below_the_knee_is_v_over_12_92() {
        for c in 0..=10u8 {
            let v = f64::from(c) / 255.0;
            assert!(v <= 0.04045);
            assert_eq!(linear(c).to_bits(), (v / 12.92).to_bits(), "{c}");
        }
        assert!(f64::from(11u8) / 255.0 > 0.04045);
        assert_eq!(linear(255), 1.0);
    }

    #[cfg(target_os = "macos")]
    #[test]
    #[allow(
        clippy::disallowed_methods,
        reason = "checks the table against the host pow it was taken from"
    )]
    fn linear_table_above_the_knee_is_pow() {
        for c in 11..=255u8 {
            let v = f64::from(c) / 255.0;
            assert_eq!(
                linear(c).to_bits(),
                ((v + 0.055) / 1.055).powf(2.4).to_bits(),
                "{c}"
            );
        }
    }

    #[test]
    fn white_and_black_in_lab() {
        let white = srgb_to_lab::<Fdlibm>(255, 255, 255);
        // The matrix and white point are rounded to 7 digits: white lands within 1e-5 of (100, 0, 0).
        assert!(
            white
                .iter()
                .zip([100.0, 0.0, 0.0])
                .all(|(v, want)| (v - want).abs() < 1e-4),
            "{white:?}"
        );
        assert_eq!(srgb_to_lab::<Fdlibm>(0, 0, 0), [0.0, 0.0, 0.0]);
    }

    #[test]
    fn delta_e_of_sharma_pairs() {
        // Sharma, Wu and Dalal (2005), table 1: pairs 1, 7, 13 and 17.
        let pairs = [
            ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
            ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
            ([50.0, 2.4900, -0.0010], [50.0, -2.4900, 0.0011], 7.2195),
            ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
        ];
        for (a, b, want) in pairs {
            let got = delta_e00::<Fdlibm>(a, b);
            assert!((got - want).abs() < 5e-5, "{a:?} {b:?}: {got} not {want}");
        }
        assert_eq!(delta_e00::<Fdlibm>([50.0, 0.0, 0.0], [50.0, 0.0, 0.0]), 0.0);
    }

    #[test]
    fn edge_tiles_hold_fewer_pixels() {
        let image = Rgba::blank(10, 9, 255).unwrap();
        let tiles = tile_lab::<Fdlibm>(&image);
        assert_eq!((tiles.columns, tiles.rows, tiles.tiles.len()), (2, 2, 4));
    }

    #[test]
    fn identical_images_have_no_difference() {
        let image = Rgba::blank(16, 16, 128).unwrap();
        assert_eq!(
            color_difference::<Fdlibm>(&image, &image).unwrap(),
            ColorScore {
                mean: 0.0,
                p99: 0.0
            }
        );
    }

    #[test]
    fn region_lab_clamps_and_reports_empty() {
        let image = Rgba::blank(4, 4, 0).unwrap();
        assert_eq!(
            region_lab::<Fdlibm>(&image, -2.0, -2.0, 1.5, 0.5),
            Some([0.0, 0.0, 0.0])
        );
        assert_eq!(region_lab::<Fdlibm>(&image, 5.0, 0.0, 6.0, 1.0), None);
        assert_eq!(region_lab::<Fdlibm>(&image, f64::NAN, 0.0, 1.0, 1.0), None);
    }
}
