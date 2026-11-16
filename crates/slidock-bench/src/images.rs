//! Comparing two renders of one row (the host lane's mounted against standalone, parity's run against the golden):
//! both at the metric size, over white, scored with `ssim@1` and `color@1`.

use slidock_bench_metrics::{HostLibm, color_difference, decode_png, metric_pair, ssim};

use crate::error::{BenchError, Result};

/// Render-vs-render SSIM and mean ΔE00, at the metric size of `a`.
pub fn render_vs_render(a: &[u8], b: &[u8]) -> Result<(f64, f64)> {
    let a = decode_png(a).map_err(|e| BenchError::failed(format!("png: {e}")))?;
    let b = decode_png(b).map_err(|e| BenchError::failed(format!("png: {e}")))?;
    let (x, y) = metric_pair(&a, &b).map_err(|e| BenchError::failed(e.to_string()))?;
    let structure = ssim(&x, &y).map_err(|e| BenchError::failed(e.to_string()))?;
    let color =
        color_difference::<HostLibm>(&x, &y).map_err(|e| BenchError::failed(e.to_string()))?;
    Ok((
        slidock_bench_metrics::quantise(structure.mean),
        slidock_bench_metrics::quantise(color.mean),
    ))
}

/// Whether two decoded PNGs hold the same pixels (two encoders can write the same pixels differently).
pub fn same_pixels(a: &[u8], b: &[u8]) -> Result<bool> {
    let a = decode_png(a).map_err(|e| BenchError::failed(format!("png: {e}")))?;
    let b = decode_png(b).map_err(|e| BenchError::failed(format!("png: {e}")))?;
    Ok(a.width() == b.width() && a.height() == b.height() && a.data() == b.data())
}

/// Pixels of two same-size PNGs whose channels differ by more than `tolerance`; `None` when the sizes differ.
pub fn specks(a: &[u8], b: &[u8], tolerance: u8) -> Result<Option<u32>> {
    let a = decode_png(a).map_err(|e| BenchError::failed(format!("png: {e}")))?;
    let b = decode_png(b).map_err(|e| BenchError::failed(format!("png: {e}")))?;
    if a.width() != b.width() || a.height() != b.height() {
        return Ok(None);
    }
    let count = a
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.data().as_chunks::<4>().0)
        .filter(|(x, y)| {
            x.iter()
                .zip(y.iter())
                .any(|(p, q)| p.abs_diff(*q) > tolerance)
        })
        .count();
    Ok(Some(u32::try_from(count).unwrap_or(u32::MAX)))
}

/// Whether the static lane's metrics class `b` a `match` of `a`: SSIM mean and 1st percentile and mean ΔE00 within
/// `classifier@2`'s match thresholds, at the metric size (text recall does not apply between two renders).
pub fn metrics_match(a: &[u8], b: &[u8]) -> Result<bool> {
    let a = decode_png(a).map_err(|e| BenchError::failed(format!("png: {e}")))?;
    let b = decode_png(b).map_err(|e| BenchError::failed(format!("png: {e}")))?;
    let (x, y) = metric_pair(&a, &b).map_err(|e| BenchError::failed(e.to_string()))?;
    let structure = ssim(&x, &y).map_err(|e| BenchError::failed(e.to_string()))?;
    let color =
        color_difference::<HostLibm>(&x, &y).map_err(|e| BenchError::failed(e.to_string()))?;
    let threshold = slidock_bench_metrics::MATCH;
    Ok(structure.mean >= threshold.ssim_mean
        && structure.p1 >= threshold.ssim_p1
        && color.mean <= threshold.color_mean)
}

/// Where two frames differ inside a region: how many pixels, and their centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Change {
    pub pixels: u32,
    /// The mean position of those pixels (pixel centres); `None` when none differ.
    pub center: Option<(f64, f64)>,
}

/// The pixels of two same-size decoded frames inside `region` (x, y, width, height; the whole frame when `None`)
/// whose channels differ by more than `tolerance`; `None` when the sizes differ.
pub fn change(
    a: &slidock_bench_metrics::Rgba,
    b: &slidock_bench_metrics::Rgba,
    region: Option<[f64; 4]>,
    tolerance: u8,
) -> Option<Change> {
    let (width, height) = (a.width(), a.height());
    if width != b.width() || height != b.height() {
        return None;
    }
    let [x, y, w, h] = region.unwrap_or([0.0, 0.0, width as f64, height as f64]);
    let clamp = |v: f64, max: usize| -> usize {
        if v.is_nan() || v <= 0.0 {
            0
        } else {
            (v as usize).min(max)
        }
    };
    let (x0, x1) = (clamp(x.floor(), width), clamp((x + w).ceil(), width));
    let (y0, y1) = (clamp(y.floor(), height), clamp((y + h).ceil(), height));
    let (pa, pb) = (a.data(), b.data());
    let (mut pixels, mut sx, mut sy) = (0u64, 0f64, 0f64);
    for row in y0..y1 {
        for column in x0..x1 {
            let at = (row * width + column) * 4;
            let differs = match (pa.get(at..at + 4), pb.get(at..at + 4)) {
                (Some(p), Some(q)) => p.iter().zip(q).any(|(u, v)| u.abs_diff(*v) > tolerance),
                _ => false,
            };
            if differs {
                pixels += 1;
                sx += column as f64 + 0.5;
                sy += row as f64 + 0.5;
            }
        }
    }
    Some(Change {
        pixels: u32::try_from(pixels).unwrap_or(u32::MAX),
        center: (pixels > 0).then(|| (sx / pixels as f64, sy / pixels as f64)),
    })
}

/// Pixels where `a` and `b` differ by more than `tolerance` in a channel and no pixel of the other frame within one
/// device pixel matches: what is left once every edge may land a pixel away, as a composited layer at a fractional
/// offset draws it. `None` when the sizes differ.
pub fn displaced_specks(
    a: &slidock_bench_metrics::Rgba,
    b: &slidock_bench_metrics::Rgba,
    tolerance: u8,
) -> Option<u32> {
    let (width, height) = (a.width(), a.height());
    if width != b.width() || height != b.height() {
        return None;
    }
    let pixel = |data: &'_ [u8], x: usize, y: usize| -> Option<[u8; 4]> {
        let at = (y * width + x) * 4;
        data.get(at..at + 4)?.try_into().ok()
    };
    let near = |p: [u8; 4], q: [u8; 4]| p.iter().zip(q).all(|(u, v)| u.abs_diff(v) <= tolerance);
    let found_near = |p: [u8; 4], other: &[u8], x: usize, y: usize| {
        (y.saturating_sub(1)..=(y + 1).min(height - 1)).any(|row| {
            (x.saturating_sub(1)..=(x + 1).min(width - 1))
                .any(|column| pixel(other, column, row).is_some_and(|q| near(p, q)))
        })
    };
    let (pa, pb) = (a.data(), b.data());
    let mut count = 0u64;
    for y in 0..height {
        for x in 0..width {
            let (Some(p), Some(q)) = (pixel(pa, x, y), pixel(pb, x, y)) else {
                continue;
            };
            if !near(p, q) && !(found_near(p, pb, x, y) && found_near(q, pa, x, y)) {
                count += 1;
            }
        }
    }
    Some(u32::try_from(count).unwrap_or(u32::MAX))
}

#[cfg(test)]
mod change_tests {
    use super::*;

    fn frame(width: usize, height: usize, paint: &[(usize, usize)]) -> slidock_bench_metrics::Rgba {
        let mut data = vec![255u8; width * height * 4];
        for (x, y) in paint {
            let at = (y * width + x) * 4;
            data[at..at + 3].copy_from_slice(&[0, 0, 0]);
        }
        slidock_bench_metrics::Rgba::new(width, height, data).unwrap()
    }

    #[test]
    fn an_edge_a_pixel_away_is_displaced_and_one_further_is_not() {
        // A 3-pixel bar, then the same bar one pixel lower, then three pixels lower.
        let bar = |top: usize| {
            let paint: Vec<(usize, usize)> = (top..top + 3)
                .flat_map(|y| (2..6).map(move |x| (x, y)))
                .collect();
            frame(8, 10, &paint)
        };
        let (a, b, c) = (bar(2), bar(3), bar(5));
        assert_eq!(change(&a, &b, None, 8).unwrap().pixels, 8);
        assert_eq!(displaced_specks(&a, &b, 8), Some(0));
        assert_eq!(displaced_specks(&a, &c, 8), Some(16));
        assert_eq!(displaced_specks(&a, &frame(8, 9, &[]), 8), None);
    }

    #[test]
    fn a_change_counts_and_centres_the_pixels_that_differ_in_its_region() {
        let a = frame(8, 4, &[]);
        let b = frame(8, 4, &[(1, 1), (3, 1), (6, 2)]);
        let all = change(&a, &b, None, 8).unwrap();
        assert_eq!(all.pixels, 3);
        assert_eq!(
            all.center,
            Some(((1.5 + 3.5 + 6.5) / 3.0, (1.5 + 1.5 + 2.5) / 3.0))
        );
        // Only the left half.
        let left = change(&a, &b, Some([0.0, 0.0, 4.0, 4.0]), 8).unwrap();
        assert_eq!((left.pixels, left.center), (2, Some((2.5, 1.5))));
        // A difference within the tolerance does not count; nothing differs, nothing centres.
        assert_eq!(change(&a, &a, None, 8).unwrap().center, None);
        assert!(change(&a, &frame(4, 4, &[]), None, 8).is_none());
        // A region past the frame is clipped to it.
        assert_eq!(
            change(&a, &b, Some([-10.0, -10.0, 100.0, 100.0]), 8)
                .unwrap()
                .pixels,
            3
        );
    }
}
