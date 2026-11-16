//! `gradient@1`: how far the render departs from its oracle where gradients are drawn.
//! Each gradient element gets a region: its box less the boxes it leaves out (the slide background leaves out every
//! measured element), eroded by 1 pt so the edges' anti-aliasing stays out. A region's score is the RMSE of the two
//! prepared images over its pixels, RGB channels on a 0–1 scale.

use crate::MetricsError;
use crate::image::Rgba;
use crate::score::Size;
use crate::text::Rect;

pub const GRADIENT_VERSION: &str = "gradient@1";

/// How far a region is eroded on every side, in points: its box shrinks and the boxes it leaves out grow by this.
pub const REGION_INSET_PT: f64 = 1.0;

/// A gradient element: its id (`data-el`, or `bg` for the slide background), its box in slide points, the boxes it
/// leaves out, and whether a gradient-approximated diagnostic names it.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientTarget {
    pub el: String,
    pub rect: Rect,
    pub excluded: Vec<Rect>,
    pub approximated: bool,
}

/// One element's region: how many metric pixels it holds and the RMSE over them.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub el: String,
    pub pixels: usize,
    pub rmse: f64,
    /// A gradient-approximated diagnostic names the element.
    pub approximated: bool,
}

/// A row's `gradient@1`: the number of regions, the worst and mean region RMSE, and the element of the worst.
#[derive(Clone, Debug, PartialEq)]
pub struct GradientScore {
    pub elements: usize,
    pub worst_rmse: f64,
    pub mean_rmse: f64,
    pub worst_el: String,
}

fn contains(rect: &Rect, x: f64, y: f64) -> bool {
    x >= rect.x && x < rect.x + rect.w && y >= rect.y && y < rect.y + rect.h
}

fn grown(rect: &Rect, by: f64) -> Rect {
    Rect {
        x: rect.x - by,
        y: rect.y - by,
        w: rect.w + 2.0 * by,
        h: rect.h + 2.0 * by,
    }
}

/// The regions of `targets` on the two prepared (metric-size, over white) images of a slide of `slide` points, worst
/// first (ties by element id). A pixel belongs to a region when its centre lies in the box shrunk by
/// [`REGION_INSET_PT`] and outside every left-out box grown by as much; regions without a pixel are left out.
pub fn gradient_regions(
    oracle: &Rgba,
    render: &Rgba,
    targets: &[GradientTarget],
    slide: Size,
) -> Result<Vec<Region>, MetricsError> {
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
    let mut regions = Vec::new();
    for target in targets {
        let inner = grown(&target.rect, -REGION_INSET_PT);
        if inner.w <= 0.0 || inner.h <= 0.0 {
            continue;
        }
        let excluded: Vec<Rect> = target
            .excluded
            .iter()
            .map(|rect| grown(rect, REGION_INSET_PT))
            .collect();
        let (x0, x1) = span(inner.x, inner.x + inner.w, kx, width);
        let (y0, y1) = span(inner.y, inner.y + inner.h, ky, height);
        let mut pixels = 0usize;
        let mut sum = 0.0f64;
        for y in y0..y1 {
            let cy = (y as f64 + 0.5) / ky;
            for x in x0..x1 {
                let cx = (x as f64 + 0.5) / kx;
                if !contains(&inner, cx, cy) || excluded.iter().any(|r| contains(r, cx, cy)) {
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
            regions.push(Region {
                el: target.el.clone(),
                pixels,
                rmse: (sum / (3 * pixels) as f64).sqrt(),
                approximated: target.approximated,
            });
        }
    }
    regions.sort_by(|a, b| b.rmse.total_cmp(&a.rmse).then_with(|| a.el.cmp(&b.el)));
    Ok(regions)
}

/// The row's score from its regions (worst first, as [`gradient_regions`] orders them); none without a region.
pub fn gradient_score(regions: &[Region]) -> Option<GradientScore> {
    let worst = regions.first()?;
    let sum: f64 = regions.iter().map(|region| region.rmse).sum();
    Some(GradientScore {
        elements: regions.len(),
        worst_rmse: worst.rmse,
        mean_rmse: sum / regions.len() as f64,
        worst_el: worst.el.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SLIDE: Size = Size { w: 96.0, h: 54.0 };

    /// A white 96 × 54 image (1 px per pt for a 96 × 54 pt slide) with a horizontal grey ramp over the box
    /// (20, 20, 40 × 20): column x of the box takes `ramp(x)`.
    fn slide_image(ramp: impl Fn(usize) -> u8) -> Rgba {
        let (w, h) = (96, 54);
        let mut data = vec![255u8; w * h * 4];
        for y in 20..40 {
            for x in 20..60 {
                let at = (y * w + x) * 4;
                data[at..at + 3].copy_from_slice(&[ramp(x - 20); 3]);
            }
        }
        Rgba::new(w, h, data).unwrap()
    }

    /// Black to white, left to right.
    fn ramp(x: usize) -> u8 {
        (x * 255 / 39) as u8
    }

    fn target(el: &str) -> GradientTarget {
        GradientTarget {
            el: el.into(),
            rect: Rect {
                x: 20.0,
                y: 20.0,
                w: 40.0,
                h: 20.0,
            },
            excluded: Vec::new(),
            approximated: false,
        }
    }

    #[test]
    fn a_matching_gradient_scores_zero() {
        let image = slide_image(ramp);
        let regions = gradient_regions(&image, &image, &[target("a")], SLIDE).unwrap();
        assert_eq!(regions.len(), 1);
        assert_eq!(regions[0].rmse, 0.0);
        // The box eroded by 1 pt on every side.
        assert_eq!(regions[0].pixels, 38 * 18);
    }

    #[test]
    fn a_missing_gradient_scores_its_whole_ramp() {
        let oracle = slide_image(ramp);
        let render = slide_image(|_| 255);
        let regions = gradient_regions(&oracle, &render, &[target("a")], SLIDE).unwrap();
        // Columns 1..=38 of the ramp (the eroded box), each against white on every channel.
        let sum: f64 = (1..39)
            .map(|x| {
                let d = (255.0 - f64::from(ramp(x))) / 255.0;
                d * d
            })
            .sum();
        let expected = (sum / 38.0).sqrt();
        assert!(
            (regions[0].rmse - expected).abs() < 1e-12,
            "{}",
            regions[0].rmse
        );
        assert!(regions[0].rmse > 0.5);
    }

    #[test]
    fn a_reversed_gradient_scores_worse_than_a_shifted_one() {
        let oracle = slide_image(ramp);
        let reversed = slide_image(|x| ramp(39 - x));
        let shifted = slide_image(|x| ramp((x + 1).min(39)));
        let score = |render: &Rgba| {
            gradient_regions(&oracle, render, &[target("a")], SLIDE).unwrap()[0].rmse
        };
        assert!(score(&shifted) > 0.0);
        assert!(score(&shifted) < 0.05);
        assert!(score(&reversed) > 0.5);
    }

    #[test]
    fn the_region_leaves_out_its_edges_and_the_excluded_boxes() {
        // Only the box's outermost pixel ring and a 4 × 4 square inside it differ: no region pixel sees them.
        let oracle = slide_image(ramp);
        let mut data = oracle.data().to_vec();
        for y in 20..40 {
            for x in 20..60 {
                let edge = x == 20 || x == 59 || y == 20 || y == 39;
                let square = (30..34).contains(&x) && (25..29).contains(&y);
                if edge || square {
                    let at = (y * 96 + x) * 4;
                    data[at..at + 3].copy_from_slice(&[255, 0, 0]);
                }
            }
        }
        let render = Rgba::new(96, 54, data).unwrap();
        let mut cut = target("a");
        cut.excluded.push(Rect {
            x: 31.0,
            y: 26.0,
            w: 2.0,
            h: 2.0,
        });
        let regions = gradient_regions(&oracle, &render, &[cut], SLIDE).unwrap();
        assert_eq!(regions[0].rmse, 0.0);
        // The square grown by 1 pt is 4 × 4.
        assert_eq!(regions[0].pixels, 38 * 18 - 4 * 4);
        // Without the cut, the square counts.
        let whole = gradient_regions(&oracle, &render, &[target("a")], SLIDE).unwrap();
        assert!(whole[0].rmse > 0.0);
    }

    #[test]
    fn regions_off_the_image_or_too_small_are_left_out() {
        let image = slide_image(ramp);
        let off = GradientTarget {
            rect: Rect {
                x: 200.0,
                y: 0.0,
                w: 10.0,
                h: 10.0,
            },
            ..target("off")
        };
        let thin = GradientTarget {
            rect: Rect {
                x: 10.0,
                y: 10.0,
                w: 2.0,
                h: 30.0,
            },
            ..target("thin")
        };
        let covered = GradientTarget {
            excluded: vec![Rect {
                x: 0.0,
                y: 0.0,
                w: 96.0,
                h: 54.0,
            }],
            ..target("covered")
        };
        let regions = gradient_regions(&image, &image, &[off, thin, covered], SLIDE).unwrap();
        assert!(regions.is_empty());
    }

    #[test]
    fn the_score_takes_the_worst_region_and_the_mean() {
        let oracle = slide_image(ramp);
        let render = slide_image(|x| ramp(39 - x));
        let far = GradientTarget {
            rect: Rect {
                x: 70.0,
                y: 5.0,
                w: 10.0,
                h: 10.0,
            },
            approximated: true,
            ..target("b")
        };
        let regions = gradient_regions(&oracle, &render, &[far, target("a")], SLIDE).unwrap();
        assert_eq!(
            regions
                .iter()
                .map(|r| (r.el.as_str(), r.approximated))
                .collect::<Vec<_>>(),
            [("a", false), ("b", true)]
        );
        let score = gradient_score(&regions).unwrap();
        assert_eq!(score.elements, 2);
        assert_eq!(score.worst_el, "a");
        assert_eq!(score.worst_rmse, regions[0].rmse);
        assert_eq!(score.mean_rmse, regions[0].rmse / 2.0);
        assert_eq!(gradient_score(&[]), None);
    }

    #[test]
    fn sizes_must_match() {
        let a = Rgba::blank(4, 4, 255).unwrap();
        let b = Rgba::blank(4, 5, 255).unwrap();
        assert!(gradient_regions(&a, &b, &[], SLIDE).is_err());
    }
}
