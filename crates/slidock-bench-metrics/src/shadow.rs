//! `shadow@1`: how far the render departs from its oracle where shadows fall. Each
//! shadowed element gets a ring: its measured box shifted by the shadow's page offset and grown by `2.5σ + 1` pt (by
//! 20 pt all round for a shadow the compiler reported as not drawn, whose offset is unknown), minus the box itself.
//! A ring's score is the RMSE of the two prepared images over its pixels, RGB channels on a 0–1 scale.

use crate::MetricsError;
use crate::image::Rgba;
use crate::score::Size;
use crate::text::Rect;

pub const SHADOW_VERSION: &str = "shadow@1";

/// How far a drawn shadow's ring reaches past its offset box, in shadow σ.
pub const RING_SIGMAS: f64 = 2.5;
/// What every drawn shadow's ring adds to that, in points: one raster pixel at the metric size.
pub const RING_PAD_PT: f64 = 1.0;
/// The ring of a shadow that is not drawn, all round the box, in points.
pub const NOT_DRAWN_RING_PT: f64 = 20.0;

/// The shadow an element casts, as far as the ring needs it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ShadowExtent {
    /// A shadow in the IR: its page offset and Gaussian σ, in points.
    Drawn { dx: f64, dy: f64, sigma: f64 },
    /// A shadow the compiler reported as not drawn.
    NotDrawn,
}

/// A shadowed element: its id (`data-el`), its measured box in slide points and its shadow.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowTarget {
    pub el: String,
    pub rect: Rect,
    pub extent: ShadowExtent,
}

impl ShadowTarget {
    /// The ring's outer box, in slide points.
    pub fn outer(&self) -> Rect {
        let (dx, dy, grow) = match self.extent {
            ShadowExtent::Drawn { dx, dy, sigma } => (dx, dy, RING_SIGMAS * sigma + RING_PAD_PT),
            ShadowExtent::NotDrawn => (0.0, 0.0, NOT_DRAWN_RING_PT),
        };
        Rect {
            x: self.rect.x + dx - grow,
            y: self.rect.y + dy - grow,
            w: self.rect.w + 2.0 * grow,
            h: self.rect.h + 2.0 * grow,
        }
    }
}

/// One element's ring: how many metric pixels it holds and the RMSE over them.
#[derive(Clone, Debug, PartialEq)]
pub struct Ring {
    pub el: String,
    pub pixels: usize,
    pub rmse: f64,
    /// The element's shadow is not drawn.
    pub not_drawn: bool,
}

/// A row's `shadow@1`: the number of rings, the worst and mean ring RMSE, and the element of the worst.
#[derive(Clone, Debug, PartialEq)]
pub struct ShadowScore {
    pub elements: usize,
    pub worst_rmse: f64,
    pub mean_rmse: f64,
    pub worst_el: String,
}

fn contains(rect: &Rect, x: f64, y: f64) -> bool {
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

/// The rings of `targets` on the two prepared (metric-size, over white) images of a slide of `slide` points, worst
/// first (ties by element id). A pixel belongs to a ring when its centre lies in the outer box and outside the
/// element's box; rings without a pixel on the image are left out.
pub fn shadow_rings(
    oracle: &Rgba,
    render: &Rgba,
    targets: &[ShadowTarget],
    slide: Size,
) -> Result<Vec<Ring>, MetricsError> {
    oracle.check_same_size(render)?;
    let (width, height) = (oracle.width(), oracle.height());
    let kx = width as f64 / slide.w;
    let ky = height as f64 / slide.h;
    let span = |from: f64, to: f64, k: f64, limit: usize| {
        let start = (from * k).floor().max(0.0);
        let end = (to * k).ceil().min(limit as f64);
        if end > start {
            (start as usize, end as usize)
        } else {
            (0, 0)
        }
    };
    let mut rings = Vec::new();
    for target in targets {
        let outer = target.outer();
        let (x0, x1) = span(outer.x, outer.x + outer.w, kx, width);
        let (y0, y1) = span(outer.y, outer.y + outer.h, ky, height);
        let mut pixels = 0usize;
        let mut sum = 0.0f64;
        for y in y0..y1 {
            let cy = (y as f64 + 0.5) / ky;
            for x in x0..x1 {
                let cx = (x as f64 + 0.5) / kx;
                if !contains(&outer, cx, cy) || contains(&target.rect, cx, cy) {
                    continue;
                }
                let at = (y * width + x) * 4;
                let (Some(a), Some(b)) =
                    (oracle.data().get(at..at + 3), render.data().get(at..at + 3))
                else {
                    continue;
                };
                for (p, q) in a.iter().zip(b) {
                    let d = (f64::from(*p) - f64::from(*q)) / 255.0;
                    sum += d * d;
                }
                pixels += 1;
            }
        }
        if pixels > 0 {
            rings.push(Ring {
                el: target.el.clone(),
                pixels,
                rmse: (sum / (3 * pixels) as f64).sqrt(),
                not_drawn: target.extent == ShadowExtent::NotDrawn,
            });
        }
    }
    rings.sort_by(|a, b| b.rmse.total_cmp(&a.rmse).then_with(|| a.el.cmp(&b.el)));
    Ok(rings)
}

/// The row's score from its rings (worst first, as [`shadow_rings`] orders them); none without a ring.
pub fn shadow_score(rings: &[Ring]) -> Option<ShadowScore> {
    let worst = rings.first()?;
    let sum: f64 = rings.iter().map(|ring| ring.rmse).sum();
    Some(ShadowScore {
        elements: rings.len(),
        worst_rmse: worst.rmse,
        mean_rmse: sum / rings.len() as f64,
        worst_el: worst.el.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A white 96 × 54 image (1 px per pt for a 96 × 54 pt slide) with a black box at (20, 20, 30 × 10) and, when
    /// given, a grey shadow box of the same size offset by `shadow`.
    fn slide_image(shadow: Option<(usize, usize)>) -> Rgba {
        let (w, h) = (96, 54);
        let mut data = vec![255u8; w * h * 4];
        let mut paint = |x0: usize, y0: usize, value: u8| {
            for y in y0..y0 + 10 {
                for x in x0..x0 + 30 {
                    let at = (y * w + x) * 4;
                    data[at..at + 3].copy_from_slice(&[value; 3]);
                }
            }
        };
        if let Some((dx, dy)) = shadow {
            paint(20 + dx, 20 + dy, 128);
        }
        paint(20, 20, 0);
        Rgba::new(w, h, data).unwrap()
    }

    const SLIDE: Size = Size { w: 96.0, h: 54.0 };

    fn target(extent: ShadowExtent) -> ShadowTarget {
        ShadowTarget {
            el: "a".into(),
            rect: Rect {
                x: 20.0,
                y: 20.0,
                w: 30.0,
                h: 10.0,
            },
            extent,
        }
    }

    const DRAWN: ShadowExtent = ShadowExtent::Drawn {
        dx: 4.0,
        dy: 4.0,
        sigma: 0.0,
    };

    #[test]
    fn a_matching_shadow_scores_zero() {
        let image = slide_image(Some((4, 4)));
        let rings = shadow_rings(&image, &image, &[target(DRAWN)], SLIDE).unwrap();
        assert_eq!(rings.len(), 1);
        assert_eq!(rings[0].rmse, 0.0);
        assert!(!rings[0].not_drawn);
    }

    #[test]
    fn a_missing_shadow_fills_its_ring() {
        let oracle = slide_image(Some((4, 4)));
        let render = slide_image(None);
        let rings = shadow_rings(&oracle, &render, &[target(DRAWN)], SLIDE).unwrap();
        // The ring is the box shifted by (4, 4) and grown by 1 pt, minus the box: 32 × 12 − the overlap 27 × 7,
        // of which the shadow covers 30 × 10 − 26 × 6 (the part the box does not hide), at 127/255 on each channel.
        let ring = &rings[0];
        assert_eq!(ring.pixels, 32 * 12 - 27 * 7);
        let shadow = (30 * 10 - 26 * 6) as f64;
        let expected = (shadow * (127.0f64 / 255.0) * (127.0 / 255.0) / ring.pixels as f64).sqrt();
        assert!((ring.rmse - expected).abs() < 1e-12, "{}", ring.rmse);
    }

    #[test]
    fn a_shifted_shadow_scores_between() {
        let oracle = slide_image(Some((4, 4)));
        let missing = shadow_rings(&oracle, &slide_image(None), &[target(DRAWN)], SLIDE).unwrap();
        let shifted =
            shadow_rings(&oracle, &slide_image(Some((6, 4))), &[target(DRAWN)], SLIDE).unwrap();
        assert!(shifted[0].rmse > 0.0);
        assert!(shifted[0].rmse < missing[0].rmse);
    }

    #[test]
    fn the_ring_leaves_out_the_box() {
        // Only the box differs: no ring pixel sees it.
        let oracle = slide_image(None);
        let mut data = oracle.data().to_vec();
        for y in 20..30 {
            for x in 20..50 {
                let at = (y * 96 + x) * 4;
                data[at..at + 3].copy_from_slice(&[255, 0, 0]);
            }
        }
        let render = Rgba::new(96, 54, data).unwrap();
        let rings = shadow_rings(&oracle, &render, &[target(DRAWN)], SLIDE).unwrap();
        assert_eq!(rings[0].rmse, 0.0);
    }

    #[test]
    fn rings_scale_with_sigma_and_cover_undrawn_shadows_all_round() {
        let blurred = target(ShadowExtent::Drawn {
            dx: 0.0,
            dy: 0.0,
            sigma: 2.0,
        });
        // Grown by 2.5 · 2 + 1 = 6 pt.
        assert_eq!(
            blurred.outer(),
            Rect {
                x: 14.0,
                y: 14.0,
                w: 42.0,
                h: 22.0
            }
        );
        let undrawn = target(ShadowExtent::NotDrawn);
        assert_eq!(
            undrawn.outer(),
            Rect {
                x: 0.0,
                y: 0.0,
                w: 70.0,
                h: 50.0
            }
        );
        let image = slide_image(None);
        let rings = shadow_rings(&image, &image, &[undrawn], SLIDE).unwrap();
        assert_eq!(rings[0].pixels, 70 * 50 - 30 * 10);
        assert!(rings[0].not_drawn);
    }

    #[test]
    fn rings_off_the_image_are_left_out_and_others_are_clipped() {
        let image = slide_image(None);
        let off = ShadowTarget {
            el: "off".into(),
            rect: Rect {
                x: 200.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            extent: DRAWN,
        };
        let corner = ShadowTarget {
            el: "corner".into(),
            rect: Rect {
                x: -10.0,
                y: -10.0,
                w: 12.0,
                h: 12.0,
            },
            extent: ShadowExtent::NotDrawn,
        };
        let rings = shadow_rings(&image, &image, &[off, corner], SLIDE).unwrap();
        assert_eq!(rings.len(), 1);
        // The corner's outer box reaches 22 pt into the image; its box covers 2 × 2 of that.
        assert_eq!(rings[0].pixels, 22 * 22 - 2 * 2);
    }

    #[test]
    fn the_score_takes_the_worst_ring_and_the_mean() {
        let oracle = slide_image(Some((4, 4)));
        let render = slide_image(None);
        let far = ShadowTarget {
            el: "b".into(),
            rect: Rect {
                x: 70.0,
                y: 5.0,
                w: 10.0,
                h: 10.0,
            },
            extent: DRAWN,
        };
        let rings = shadow_rings(&oracle, &render, &[far, target(DRAWN)], SLIDE).unwrap();
        assert_eq!(
            rings.iter().map(|r| r.el.as_str()).collect::<Vec<_>>(),
            ["a", "b"]
        );
        let score = shadow_score(&rings).unwrap();
        assert_eq!(score.elements, 2);
        assert_eq!(score.worst_el, "a");
        assert_eq!(score.worst_rmse, rings[0].rmse);
        assert_eq!(score.mean_rmse, rings[0].rmse / 2.0);
        assert_eq!(shadow_score(&[]), None);
    }

    #[test]
    fn sizes_must_match() {
        let a = Rgba::blank(4, 4, 255).unwrap();
        let b = Rgba::blank(4, 5, 255).unwrap();
        assert!(shadow_rings(&a, &b, &[], SLIDE).is_err());
    }
}
