//! `ssim@1`: structural similarity of luma over 8×8 windows at a stride of 4, with the standard constants for 8-bit
//! values. The mean dilutes a small miss; the 1st percentile of the windows keeps it. The window map feeds explain's
//! heatmap.

use crate::MetricsError;
use crate::image::{Rgba, luma};
use crate::js::{percentile, pow_int, typed_order};

pub const SSIM_VERSION: &str = "ssim@1";
const WINDOW: usize = 8;
const STRIDE: usize = 4;

/// The SSIM of every window, row-major.
#[derive(Clone, Debug, PartialEq)]
pub struct SsimMap {
    pub columns: usize,
    pub rows: usize,
    pub values: Vec<f32>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SsimScore {
    pub mean: f64,
    pub p1: f64,
    pub map: SsimMap,
}

/// Windows along a side of `len` pixels: `max(1, ⌊(len − 8)/4⌋ + 1)`.
fn windows(len: usize) -> usize {
    let len = i64::try_from(len).unwrap_or(i64::MAX);
    let count = (len - WINDOW as i64).div_euclid(STRIDE as i64) + 1;
    usize::try_from(count.max(1)).unwrap_or(1)
}

/// SSIM of two same-size images (already at the metric size, over white).
pub fn ssim(a: &Rgba, b: &Rgba) -> Result<SsimScore, MetricsError> {
    a.check_same_size(b)?;
    let c1 = pow_int(0.01 * 255.0, 2);
    let c2 = pow_int(0.03 * 255.0, 2);
    let x = luma(a);
    let y = luma(b);
    let (width, height) = (a.width(), a.height());
    let columns = windows(width);
    let rows = windows(height);
    let (span_x, span_y) = (WINDOW.min(width), WINDOW.min(height));
    let n = (span_x * span_y) as f64;
    let mut values = Vec::with_capacity(columns * rows);
    for row in 0..rows {
        for column in 0..columns {
            let (mut sx, mut sy, mut sxx, mut syy, mut sxy) = (0.0, 0.0, 0.0, 0.0, 0.0);
            for dy in 0..span_y {
                let start = (row * STRIDE + dy) * width + column * STRIDE;
                let xs = x.get(start..start + span_x).unwrap_or_default();
                let ys = y.get(start..start + span_x).unwrap_or_default();
                for (&u, &v) in xs.iter().zip(ys) {
                    let (u, v) = (f64::from(u), f64::from(v));
                    sx += u;
                    sy += v;
                    sxx += u * u;
                    syy += v * v;
                    sxy += u * v;
                }
            }
            let mx = sx / n;
            let my = sy / n;
            let vx = sxx / n - mx * mx;
            let vy = syy / n - my * my;
            let cov = sxy / n - mx * my;
            let value = ((2.0 * mx * my + c1) * (2.0 * cov + c2))
                / ((mx * mx + my * my + c1) * (vx + vy + c2));
            values.push(value as f32);
        }
    }
    let mut sorted = values.clone();
    sorted.sort_by(|p, q| typed_order(f64::from(*p), f64::from(*q)));
    let total: f64 = values
        .iter()
        .fold(0.0, |sum, &value| sum + f64::from(value));
    Ok(SsimScore {
        mean: total / values.len() as f64,
        p1: percentile(&sorted, 0.01),
        map: SsimMap {
            columns,
            rows,
            values,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_images_score_one() {
        let image = Rgba::blank(16, 12, 200).unwrap();
        let score = ssim(&image, &image).unwrap();
        assert_eq!((score.map.columns, score.map.rows), (3, 2));
        assert_eq!(score.mean, 1.0);
        assert_eq!(score.p1, 1.0);
    }

    #[test]
    fn windows_cover_small_images() {
        assert_eq!(windows(5), 1);
        assert_eq!(windows(8), 1);
        assert_eq!(windows(11), 1);
        assert_eq!(windows(12), 2);
        assert_eq!(windows(960), 239);
    }

    #[test]
    fn sizes_must_match() {
        let a = Rgba::blank(8, 8, 0).unwrap();
        let b = Rgba::blank(9, 8, 0).unwrap();
        assert!(matches!(
            ssim(&a, &b),
            Err(MetricsError::SizeMismatch { .. })
        ));
    }

    #[test]
    fn constants_are_the_squares() {
        assert_eq!(pow_int(0.01 * 255.0, 2), (0.01 * 255.0) * (0.01 * 255.0));
    }
}
