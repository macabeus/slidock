//! `pixel@1`, reported only (the classifier ignores it): the share of pixels pixelmatch 5.3.0 finds equal, with
//! threshold 0.1 and anti-aliased pixels not counted. The pixelmatch port below counts differences and draws no
//! output image.

use crate::MetricsError;
use crate::image::Rgba;

pub const PIXEL_VERSION: &str = "pixel@1";

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PixelmatchOptions {
    /// Matching threshold, 0 to 1; smaller is more sensitive.
    pub threshold: f64,
    /// Counts anti-aliased pixels as differences instead of detecting them.
    pub include_aa: bool,
}

impl Default for PixelmatchOptions {
    fn default() -> Self {
        PixelmatchOptions {
            threshold: 0.1,
            include_aa: false,
        }
    }
}

/// An RGBA buffer with its width and height, for neighbour lookups.
struct Image<'a> {
    data: &'a [u8],
    width: usize,
    height: usize,
}

impl Image<'_> {
    fn rgba(&self, pos: usize) -> [u8; 4] {
        match self.data.get(pos..pos + 4) {
            Some(&[r, g, b, a]) => [r, g, b, a],
            _ => [0; 4],
        }
    }
}

/// The number of differing pixels between two RGBA buffers of `width × height`.
pub fn pixelmatch(
    img1: &[u8],
    img2: &[u8],
    width: usize,
    height: usize,
    options: PixelmatchOptions,
) -> Result<usize, MetricsError> {
    if img1.len() != img2.len() {
        return Err(MetricsError::LengthMismatch {
            a: img1.len(),
            b: img2.len(),
        });
    }
    if Some(img1.len()) != width.checked_mul(height).and_then(|n| n.checked_mul(4)) {
        return Err(MetricsError::ImageData {
            width,
            height,
            len: img1.len(),
        });
    }
    if img1 == img2 {
        return Ok(0);
    }
    let a = Image {
        data: img1,
        width,
        height,
    };
    let b = Image {
        data: img2,
        width,
        height,
    };
    // The maximum acceptable square YIQ distance; 35215 is the largest the metric can give.
    let max_delta = 35215.0 * options.threshold * options.threshold;
    let mut diff = 0;
    for y in 0..height {
        for x in 0..width {
            let pos = (y * width + x) * 4;
            let delta = color_delta(&a, &b, pos, pos, false);
            if delta.abs() > max_delta
                && (options.include_aa || !(antialiased(&a, x, y, &b) || antialiased(&b, x, y, &a)))
            {
                diff += 1;
            }
        }
    }
    Ok(diff)
}

/// Whether a pixel is likely part of anti-aliasing (Vysniauskas, "Anti-aliased Pixel and Intensity Slope Detector",
/// 2009).
fn antialiased(img: &Image<'_>, x1: usize, y1: usize, img2: &Image<'_>) -> bool {
    let x0 = x1.saturating_sub(1);
    let y0 = y1.saturating_sub(1);
    let x2 = (x1 + 1).min(img.width - 1);
    let y2 = (y1 + 1).min(img.height - 1);
    let pos = (y1 * img.width + x1) * 4;
    let mut zeroes = usize::from(x1 == x0 || x1 == x2 || y1 == y0 || y1 == y2);
    let (mut min, mut max) = (0.0, 0.0);
    let (mut min_at, mut max_at) = ((0, 0), (0, 0));
    for x in x0..=x2 {
        for y in y0..=y2 {
            if x == x1 && y == y1 {
                continue;
            }
            // The brightness difference between the centre and this neighbour.
            let delta = color_delta(img, img, pos, (y * img.width + x) * 4, true);
            if delta == 0.0 {
                zeroes += 1;
                // More than two equal neighbours: not anti-aliasing.
                if zeroes > 2 {
                    return false;
                }
            } else if delta < min {
                min = delta;
                min_at = (x, y);
            } else if delta > max {
                max = delta;
                max_at = (x, y);
            }
        }
    }
    // Without both a darker and a brighter neighbour, it is not anti-aliasing.
    if min == 0.0 || max == 0.0 {
        return false;
    }
    // Anti-aliased when the darkest or the brightest neighbour has 3+ equal neighbours in both images.
    (has_many_siblings(img, min_at) && has_many_siblings(img2, min_at))
        || (has_many_siblings(img, max_at) && has_many_siblings(img2, max_at))
}

/// Whether a pixel has 3+ adjacent pixels of the same colour.
fn has_many_siblings(img: &Image<'_>, (x1, y1): (usize, usize)) -> bool {
    let x0 = x1.saturating_sub(1);
    let y0 = y1.saturating_sub(1);
    let x2 = (x1 + 1).min(img.width - 1);
    let y2 = (y1 + 1).min(img.height - 1);
    let centre = img.rgba((y1 * img.width + x1) * 4);
    let mut zeroes = usize::from(x1 == x0 || x1 == x2 || y1 == y0 || y1 == y2);
    for x in x0..=x2 {
        for y in y0..=y2 {
            if x == x1 && y == y1 {
                continue;
            }
            if img.rgba((y * img.width + x) * 4) == centre {
                zeroes += 1;
            }
            if zeroes > 2 {
                return true;
            }
        }
    }
    false
}

/// The squared YIQ distance between two pixels (Kotsarenko and Ramos), negative when the first pixel is the brighter;
/// with `y_only`, the brightness difference alone.
fn color_delta(img1: &Image<'_>, img2: &Image<'_>, k: usize, m: usize, y_only: bool) -> f64 {
    let p = img1.rgba(k);
    let q = img2.rgba(m);
    if p == q {
        return 0.0;
    }
    let blended = |[r, g, b, a]: [u8; 4]| {
        let (r, g, b) = (f64::from(r), f64::from(g), f64::from(b));
        if a < 255 {
            let a = f64::from(a) / 255.0;
            (blend(r, a), blend(g, a), blend(b, a))
        } else {
            (r, g, b)
        }
    };
    let (r1, g1, b1) = blended(p);
    let (r2, g2, b2) = blended(q);
    let y1 = rgb2y(r1, g1, b1);
    let y2 = rgb2y(r2, g2, b2);
    let y = y1 - y2;
    if y_only {
        return y;
    }
    let i = rgb2i(r1, g1, b1) - rgb2i(r2, g2, b2);
    let q = rgb2q(r1, g1, b1) - rgb2q(r2, g2, b2);
    let delta = 0.5053 * y * y + 0.299 * i * i + 0.1957 * q * q;
    if y1 > y2 { -delta } else { delta }
}

fn rgb2y(r: f64, g: f64, b: f64) -> f64 {
    r * 0.29889531 + g * 0.58662247 + b * 0.11448223
}

fn rgb2i(r: f64, g: f64, b: f64) -> f64 {
    r * 0.59597799 - g * 0.27417610 - b * 0.32180189
}

fn rgb2q(r: f64, g: f64, b: f64) -> f64 {
    r * 0.21147017 - g * 0.52261711 + b * 0.31114694
}

/// A semi-transparent colour blended with white.
fn blend(c: f64, a: f64) -> f64 {
    255.0 + (c - 255.0) * a
}

/// `1 − differing pixels / pixels` for two same-size images at the metric size.
pub fn pixel_score(a: &Rgba, b: &Rgba) -> Result<f64, MetricsError> {
    a.check_same_size(b)?;
    let diff = pixelmatch(
        a.data(),
        b.data(),
        a.width(),
        a.height(),
        PixelmatchOptions::default(),
    )?;
    Ok(1.0 - diff as f64 / (a.width() * a.height()) as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn image(width: usize, height: usize, pixels: &[[u8; 4]]) -> Rgba {
        Rgba::new(width, height, pixels.iter().flatten().copied().collect()).unwrap()
    }

    #[test]
    fn identical_images_score_one() {
        let a = Rgba::blank(4, 4, 30).unwrap();
        assert_eq!(pixel_score(&a, &a).unwrap(), 1.0);
    }

    #[test]
    fn a_lone_changed_pixel_counts() {
        let a = Rgba::blank(3, 3, 255).unwrap();
        let mut data = a.data().to_vec();
        data[16..19].copy_from_slice(&[0, 0, 0]);
        let b = Rgba::new(3, 3, data).unwrap();
        assert_eq!(
            pixelmatch(a.data(), b.data(), 3, 3, PixelmatchOptions::default()).unwrap(),
            1
        );
        assert_eq!(pixel_score(&a, &b).unwrap(), 1.0 - 1.0 / 9.0);
    }

    #[test]
    fn small_differences_are_below_the_threshold() {
        let a = Rgba::blank(2, 2, 100).unwrap();
        let b = Rgba::blank(2, 2, 104).unwrap();
        assert_eq!(
            pixelmatch(a.data(), b.data(), 2, 2, PixelmatchOptions::default()).unwrap(),
            0
        );
    }

    #[test]
    fn an_edge_between_flat_regions_is_anti_aliasing() {
        // A grey pixel between a black and a white run, in an image whose other copy has it black.
        let (k, g, w) = ([0, 0, 0, 255], [128, 128, 128, 255], [255, 255, 255, 255]);
        let a = image(5, 3, &[k, k, g, w, w, k, k, g, w, w, k, k, g, w, w]);
        let b = image(5, 3, &[k, k, k, w, w, k, k, k, w, w, k, k, k, w, w]);
        let strict = PixelmatchOptions {
            include_aa: true,
            ..PixelmatchOptions::default()
        };
        assert_eq!(pixelmatch(a.data(), b.data(), 5, 3, strict).unwrap(), 3);
        assert_eq!(
            pixelmatch(a.data(), b.data(), 5, 3, PixelmatchOptions::default()).unwrap(),
            0
        );
    }

    #[test]
    fn sizes_must_match() {
        assert!(pixelmatch(&[0; 4], &[0; 8], 1, 1, PixelmatchOptions::default()).is_err());
        assert!(pixelmatch(&[0; 8], &[0; 8], 1, 1, PixelmatchOptions::default()).is_err());
    }
}
