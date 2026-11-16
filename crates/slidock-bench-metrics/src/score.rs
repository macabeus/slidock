//! A render scored against its oracle page, and the record's quantised scores.

use crate::MetricsError;
use crate::classify::{Scores, SsimSummary};
use crate::color::{ColorScore, color_difference};
use crate::image::{Rgba, luma, metric_pair, over_white};
use crate::js::{js_max, quantise};
use crate::libm::Libm;
use crate::pixel::pixel_score;
use crate::ssim::ssim;
use crate::text::{Rect, TextScore, TextWord, align_words, text_ink, text_score};

/// A size in points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Size {
    pub w: f64,
    pub h: f64,
}

/// A word of `pdftotext -bbox-layout`, its box in the page's units, y down.
#[derive(Clone, Debug, PartialEq)]
pub struct OracleWord {
    pub text: String,
    pub x_min: f64,
    pub y_min: f64,
    pub x_max: f64,
    pub y_max: f64,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct OracleLine {
    pub words: Vec<OracleWord>,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct OracleBlock {
    pub lines: Vec<OracleLine>,
}

/// One page of `pdftotext -bbox-layout`: its size as the tool reports it (NaN when missing) and its words.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct OracleText {
    pub width: f64,
    pub height: f64,
    pub blocks: Vec<OracleBlock>,
}

/// JavaScript's `a || b` on numbers: `b` when `a` is 0 or NaN.
fn or(a: f64, b: f64) -> f64 {
    if a == 0.0 || a.is_nan() { b } else { a }
}

/// The oracle words in slide points: the page may use other units than the slide (Keynote's does). `page` is the
/// PDF page size, used where the text layer reports none.
pub fn oracle_words(text: &OracleText, page: Size, slide: Size) -> Vec<TextWord> {
    let sx = slide.w / or(text.width, page.w);
    let sy = slide.h / or(text.height, page.h);
    text.blocks
        .iter()
        .flat_map(|block| &block.lines)
        .flat_map(|line| &line.words)
        .map(|word| TextWord {
            text: word.text.clone(),
            rect: Rect {
                x: word.x_min * sx,
                y: word.y_min * sy,
                w: (word.x_max - word.x_min) * sx,
                h: (word.y_max - word.y_min) * sy,
            },
        })
        .collect()
}

/// The metrics of a render against its oracle page. `oracle` and `render` are the decoded rasters, `textless` the
/// render with all text painted out, `oracle_text` from [`oracle_words`], `dom` the measured visible DOM words in
/// slide points, `slide` the slide size in points.
pub fn score_render<M: Libm>(
    oracle: &Rgba,
    render: &Rgba,
    textless: &Rgba,
    oracle_text: &[TextWord],
    dom: &[TextWord],
    slide: Size,
) -> Result<Scores, MetricsError> {
    let (oracle_metric, render_metric) = metric_pair(oracle, render)?;
    let structure = ssim(&oracle_metric, &render_metric)?;
    let long_side_pt = js_max(slide.w, slide.h);
    let with_text = luma(&over_white(render));
    let without = luma(&over_white(textless));
    let px_per_pt = render.width() as f64 / slide.w;
    let ink = |rect: &Rect| text_ink(&with_text, &without, render.width(), rect, px_per_pt);
    let alignment = align_words::<M>(oracle_text, dom, long_side_pt, Some(&ink))?;
    Ok(Scores {
        ssim: SsimSummary {
            mean: structure.mean,
            p1: structure.p1,
        },
        color: color_difference::<M>(&oracle_metric, &render_metric)?,
        pixel: pixel_score(&oracle_metric, &render_metric)?,
        text: text_score::<M>(oracle_text, dom, &alignment, long_side_pt)?,
    })
}

/// A nullable score on the results grid: NaN becomes `None`.
fn quantise_optional(value: Option<f64>) -> Option<f64> {
    value.filter(|v| !v.is_nan()).map(quantise)
}

/// Every score of a record on the results grid (1e-4); counts stay as they are.
pub fn quantise_scores(scores: &Scores) -> Scores {
    let text = scores.text;
    Scores {
        ssim: SsimSummary {
            mean: quantise(scores.ssim.mean),
            p1: quantise(scores.ssim.p1),
        },
        color: ColorScore {
            mean: quantise(scores.color.mean),
            p99: quantise(scores.color.p99),
        },
        pixel: quantise(scores.pixel),
        text: TextScore {
            recall: quantise(text.recall),
            precision: quantise(text.precision),
            median_pm: quantise_optional(text.median_pm),
            p95_pm: quantise_optional(text.p95_pm),
            height_ratio_median: quantise_optional(text.height_ratio_median),
            ..text
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::classify::{ScoredClass, scored_class};
    use crate::libm::Fdlibm;

    fn page(words: &[(&str, f64, f64, f64, f64)], width: f64) -> OracleText {
        OracleText {
            width,
            height: width / 2.0,
            blocks: vec![OracleBlock {
                lines: vec![OracleLine {
                    words: words
                        .iter()
                        .map(|&(text, x_min, y_min, x_max, y_max)| OracleWord {
                            text: text.into(),
                            x_min,
                            y_min,
                            x_max,
                            y_max,
                        })
                        .collect(),
                }],
            }],
        }
    }

    #[test]
    fn oracle_words_scale_into_slide_points() {
        let text = page(&[("Hi", 10.0, 20.0, 30.0, 40.0)], 200.0);
        let words = oracle_words(&text, Size { w: 1.0, h: 1.0 }, Size { w: 400.0, h: 200.0 });
        assert_eq!(
            words[0].rect,
            Rect {
                x: 20.0,
                y: 40.0,
                w: 40.0,
                h: 40.0
            }
        );
        // A text layer without a size falls back to the page size.
        let text = page(&[("Hi", 10.0, 20.0, 30.0, 40.0)], f64::NAN);
        let words = oracle_words(
            &text,
            Size { w: 100.0, h: 50.0 },
            Size { w: 400.0, h: 200.0 },
        );
        assert_eq!(
            words[0].rect,
            Rect {
                x: 40.0,
                y: 80.0,
                w: 80.0,
                h: 80.0
            }
        );
    }

    #[test]
    fn a_render_equal_to_its_oracle_matches() {
        let image = Rgba::blank(32, 16, 255).unwrap();
        let scores =
            score_render::<Fdlibm>(&image, &image, &image, &[], &[], Size { w: 24.0, h: 12.0 })
                .unwrap();
        assert_eq!(scores.ssim, SsimSummary { mean: 1.0, p1: 1.0 });
        assert_eq!(scores.pixel, 1.0);
        assert_eq!(scored_class(&scores), ScoredClass::Match);
    }

    #[test]
    fn words_without_ink_are_missing() {
        // The text-less twin equals the render, so the matched word leaves no ink.
        let image = Rgba::blank(32, 16, 255).unwrap();
        let word = TextWord {
            text: "a".into(),
            rect: Rect {
                x: 1.0,
                y: 1.0,
                w: 4.0,
                h: 4.0,
            },
        };
        let words = [word];
        let scores = score_render::<Fdlibm>(
            &image,
            &image,
            &image,
            &words,
            &words,
            Size { w: 24.0, h: 12.0 },
        )
        .unwrap();
        assert_eq!((scores.text.matched, scores.text.recall), (0, 0.0));
        assert_eq!(scored_class(&scores), ScoredClass::Mismatch);
    }

    #[test]
    fn quantised_scores() {
        let image = Rgba::blank(8, 8, 255).unwrap();
        let mut scores =
            score_render::<Fdlibm>(&image, &image, &image, &[], &[], Size { w: 8.0, h: 8.0 })
                .unwrap();
        scores.ssim.p1 = -0.247_36;
        scores.text.recall = 2.0 / 3.0;
        scores.text.median_pm = Some(f64::NAN);
        scores.text.p95_pm = Some(3.948_149);
        let q = quantise_scores(&scores);
        assert_eq!(q.ssim.p1, -0.2474);
        assert_eq!(q.text.recall, 0.6667);
        assert_eq!(q.text.median_pm, None);
        assert_eq!(q.text.p95_pm, Some(3.9481));
    }
}
