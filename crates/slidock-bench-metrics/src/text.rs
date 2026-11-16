//! `text@2`: where the words are. Oracle words (pdftotext) and rendered DOM words (only those the renderer's
//! measure counts as visible) are aligned by longest common subsequence on normalised text, leftovers are matched by
//! equal text and the nearest centre, and a matched word whose text leaves no ink of its own counts as missing: the
//! render must differ inside the word's box from a second render with the text painted out. Positions are in
//! per-mille of the slide's long side, so presentations of different sizes share thresholds.

use unicode_normalization::UnicodeNormalization;

use crate::MetricsError;
use crate::js::{is_js_whitespace, js_max, js_min, median, percentile, typed_order};
use crate::libm::Libm;

pub const TEXT_VERSION: &str = "text@2";
/// LCS runs on words up to this many per side; beyond it, on lines.
const LCS_LIMIT: usize = 2_000;
/// Leftovers match within this distance (‰ of the long side).
const LEFTOVER_RADIUS_PM: f64 = 70.0;
/// A matched word's text must change the render by this much luma (of 255) somewhere in its box.
pub const INK_DIFFERENCE: f64 = 12.0;

/// A box in points, y down from the slide's top-left corner.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn centre(&self) -> (f64, f64) {
        (self.x + self.w / 2.0, self.y + self.h / 2.0)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextWord {
    pub text: String,
    pub rect: Rect,
}

fn is_zero_width(c: char) -> bool {
    matches!(c, '\u{200B}'..='\u{200D}' | '\u{2060}' | '\u{FEFF}')
}

/// NFKC (the `ﬁ` ligature reads `fi`), zero-width characters removed, then JavaScript's `trim`; case kept.
pub fn normalise_word(text: &str) -> String {
    let normal: String = text.nfkc().filter(|&c| !is_zero_width(c)).collect();
    normal.trim_matches(is_js_whitespace).to_owned()
}

/// The line of each word: a new line when a word's vertical centre leaves the previous word's box (or `same_group`
/// says the two words are apart).
pub fn line_breaks(words: &[TextWord], same_group: impl Fn(usize, usize) -> bool) -> Vec<usize> {
    let mut lines = Vec::with_capacity(words.len());
    let mut line: Option<usize> = None;
    let mut previous: Option<&TextWord> = None;
    for (i, word) in words.iter().enumerate() {
        let same_line = previous.is_some_and(|previous| {
            same_group(i - 1, i)
                && (word.rect.centre().1 - previous.rect.centre().1).abs()
                    <= js_max(word.rect.h, previous.rect.h) / 2.0
        });
        let current = match (line, same_line) {
            (Some(line), true) => line,
            (Some(line), false) => line + 1,
            (None, _) => 0,
        };
        line = Some(current);
        lines.push(current);
        previous = Some(word);
    }
    lines
}

/// Index pairs of a longest common subsequence of two sequences, earliest first on ties.
pub fn lcs_pairs<T>(a: &[T], b: &[T], equal: impl Fn(&T, &T) -> bool) -> Vec<(usize, usize)> {
    let (n, m) = (a.len(), b.len());
    let at = |i: usize, j: usize| i * (m + 1) + j;
    let mut table = vec![0u32; (n + 1) * (m + 1)];
    let cell = |table: &[u32], i: usize, j: usize| table.get(at(i, j)).copied().unwrap_or(0);
    for (i, x) in a.iter().enumerate().rev() {
        for (j, y) in b.iter().enumerate().rev() {
            let value = if equal(x, y) {
                cell(&table, i + 1, j + 1) + 1
            } else {
                cell(&table, i + 1, j).max(cell(&table, i, j + 1))
            };
            if let Some(slot) = table.get_mut(at(i, j)) {
                *slot = value;
            }
        }
    }
    let mut pairs = Vec::new();
    let (mut i, mut j) = (0, 0);
    while let (Some(x), Some(y)) = (a.get(i), b.get(j)) {
        if equal(x, y) {
            pairs.push((i, j));
            i += 1;
            j += 1;
        } else if cell(&table, i + 1, j) >= cell(&table, i, j + 1) {
            i += 1;
        } else {
            j += 1;
        }
    }
    pairs
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Alignment {
    /// Matched (oracle index, DOM index) pairs, in oracle order.
    pub pairs: Vec<(usize, usize)>,
    /// Unmatched oracle words, including those whose DOM match left no ink.
    pub missing: Vec<usize>,
    /// Unmatched DOM words.
    pub extra: Vec<usize>,
    /// DOM words matched by text whose box shows no ink on the render.
    pub inkless: Vec<usize>,
}

/// One line of words for the line-level LCS.
struct Line {
    indices: Vec<usize>,
    text: String,
}

fn lines_of(words: &[TextWord], texts: &[String]) -> Vec<Line> {
    let mut lines: Vec<Line> = Vec::new();
    for (i, line) in line_breaks(words, |_, _| true).into_iter().enumerate() {
        if lines.len() <= line {
            lines.push(Line {
                indices: Vec::new(),
                text: String::new(),
            });
        }
        if let Some(entry) = lines.get_mut(line) {
            entry.indices.push(i);
        }
    }
    for line in &mut lines {
        let words: Vec<&str> = line
            .indices
            .iter()
            .filter_map(|&i| texts.get(i).map(String::as_str))
            .collect();
        line.text = words.join(" ");
    }
    lines
}

/// Aligns oracle words with DOM words (both in reading order). `ink` gives the ink a DOM word's own text puts in its
/// box ([`text_ink`]); without it every word has ink. A matched line with fewer DOM words than oracle words is an
/// error (the row fails there).
pub fn align_words<M: Libm>(
    oracle: &[TextWord],
    dom: &[TextWord],
    long_side_pt: f64,
    ink: Option<&dyn Fn(&Rect) -> f64>,
) -> Result<Alignment, MetricsError> {
    let a: Vec<String> = oracle
        .iter()
        .map(|word| normalise_word(&word.text))
        .collect();
    let b: Vec<String> = dom.iter().map(|word| normalise_word(&word.text)).collect();
    let mut pairs = if a.len() <= LCS_LIMIT && b.len() <= LCS_LIMIT {
        lcs_pairs(&a, &b, |x, y| x == y)
    } else {
        let la = lines_of(oracle, &a);
        let lb = lines_of(dom, &b);
        let mut pairs = Vec::new();
        for (i, j) in lcs_pairs(&la, &lb, |x, y| x.text == y.text) {
            let (Some(line_a), Some(line_b)) = (la.get(i), lb.get(j)) else {
                continue;
            };
            for (k, &index) in line_a.indices.iter().enumerate() {
                pairs.push((
                    index,
                    *line_b.indices.get(k).ok_or(MetricsError::LineWords)?,
                ));
            }
        }
        pairs
    };
    let mut used_a = vec![false; a.len()];
    let mut used_b = vec![false; b.len()];
    for &(i, j) in &pairs {
        mark(&mut used_a, i);
        mark(&mut used_b, j);
    }
    let radius = (LEFTOVER_RADIUS_PM / 1000.0) * long_side_pt;
    for (i, (text, word)) in a.iter().zip(oracle).enumerate() {
        if is_used(&used_a, i) || text.is_empty() {
            continue;
        }
        let (cx, cy) = word.rect.centre();
        let mut best: Option<usize> = None;
        let mut best_distance = f64::INFINITY;
        for (j, (other, candidate)) in b.iter().zip(dom).enumerate() {
            if is_used(&used_b, j) || other != text {
                continue;
            }
            let (dx, dy) = candidate.rect.centre();
            let distance = M::hypot(cx - dx, cy - dy);
            if distance <= radius && distance < best_distance {
                best = Some(j);
                best_distance = distance;
            }
        }
        if let Some(j) = best {
            pairs.push((i, j));
            mark(&mut used_a, i);
            mark(&mut used_b, j);
        }
    }
    pairs.sort_by_key(|&(i, _)| i);
    let mut inkless = Vec::new();
    let mut kept = Vec::with_capacity(pairs.len());
    for (i, j) in pairs {
        let has_ink = match ink {
            Some(ink) => {
                let word = dom
                    .get(j)
                    .ok_or(MetricsError::PairIndex { oracle: i, dom: j })?;
                ink(&word.rect) >= INK_DIFFERENCE
            }
            None => true,
        };
        if has_ink {
            kept.push((i, j));
        } else {
            inkless.push(j);
        }
    }
    let mut kept_a = vec![false; a.len()];
    for &(i, _) in &kept {
        mark(&mut kept_a, i);
    }
    inkless.sort_unstable();
    Ok(Alignment {
        missing: (0..a.len())
            .filter(|&i| a.get(i).is_some_and(|t| !t.is_empty()) && !is_used(&kept_a, i))
            .collect(),
        extra: (0..b.len())
            .filter(|&j| b.get(j).is_some_and(|t| !t.is_empty()) && !is_used(&used_b, j))
            .collect(),
        pairs: kept,
        inkless,
    })
}

fn mark(flags: &mut [bool], i: usize) {
    if let Some(flag) = flags.get_mut(i) {
        *flag = true;
    }
}

fn is_used(flags: &[bool], i: usize) -> bool {
    flags.get(i).copied().unwrap_or(false)
}

/// A matched pair's offset and distance in ‰ of the long side, and its height ratio (DOM over oracle).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PairMeasure {
    pub dx_pm: f64,
    pub dy_pm: f64,
    pub distance_pm: f64,
    pub height_ratio: f64,
}

pub fn pair_measures<M: Libm>(
    oracle: &[TextWord],
    dom: &[TextWord],
    pairs: &[(usize, usize)],
    long_side_pt: f64,
) -> Result<Vec<PairMeasure>, MetricsError> {
    pairs
        .iter()
        .map(|&(i, j)| {
            let (Some(o), Some(d)) = (oracle.get(i), dom.get(j)) else {
                return Err(MetricsError::PairIndex { oracle: i, dom: j });
            };
            let (ox, oy) = o.rect.centre();
            let (dx, dy) = d.rect.centre();
            Ok(PairMeasure {
                dx_pm: ((dx - ox) / long_side_pt) * 1000.0,
                dy_pm: ((dy - oy) / long_side_pt) * 1000.0,
                distance_pm: (M::hypot(dx - ox, dy - oy) / long_side_pt) * 1000.0,
                height_ratio: if o.rect.h > 0.0 {
                    d.rect.h / o.rect.h
                } else {
                    1.0
                },
            })
        })
        .collect()
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct TextScore {
    pub oracle_words: usize,
    pub dom_words: usize,
    pub matched: usize,
    pub recall: f64,
    pub precision: f64,
    /// Median and 95th percentile of matched word-centre distances, ‰ of the long side; `None` with no match.
    pub median_pm: Option<f64>,
    pub p95_pm: Option<f64>,
    /// Median of DOM height over oracle height across matched words.
    pub height_ratio_median: Option<f64>,
}

pub fn text_score<M: Libm>(
    oracle: &[TextWord],
    dom: &[TextWord],
    alignment: &Alignment,
    long_side_pt: f64,
) -> Result<TextScore, MetricsError> {
    let count = |words: &[TextWord]| {
        words
            .iter()
            .filter(|word| !normalise_word(&word.text).is_empty())
            .count()
    };
    let oracle_words = count(oracle);
    let dom_words = count(dom);
    let matched = alignment.pairs.len();
    let measures = pair_measures::<M>(oracle, dom, &alignment.pairs, long_side_pt)?;
    let mut distances: Vec<f64> = measures.iter().map(|m| m.distance_pm).collect();
    distances.sort_by(|p, q| typed_order(*p, *q));
    let ratios: Vec<f64> = measures.iter().map(|m| m.height_ratio).collect();
    let share = |part: usize, whole: usize| {
        if whole == 0 {
            1.0
        } else {
            part as f64 / whole as f64
        }
    };
    let any = matched > 0;
    Ok(TextScore {
        oracle_words,
        dom_words,
        matched,
        recall: share(matched, oracle_words),
        precision: share(matched, dom_words),
        median_pm: any.then(|| median(&distances)),
        p95_pm: any.then(|| percentile(&distances, 0.95)),
        height_ratio_median: any.then(|| median(&ratios)),
    })
}

/// The ink a word's text puts in its box: the largest luma difference inside the box between the render and the
/// same render with all text painted transparent, `px_per_pt` pixels per point. White text on white, text under an
/// opaque element and transparent text over a picture of itself all score 0. A pixel past the end reads as NaN, as
/// an out-of-range typed-array read does, and makes the ink NaN.
pub fn text_ink(
    render: &[f32],
    textless: &[f32],
    width: usize,
    rect: &Rect,
    px_per_pt: f64,
) -> f64 {
    if width == 0 {
        return 0.0;
    }
    let w = width as f64;
    let height = render.len() as f64 / w;
    let x0 = js_max(0.0, (rect.x * px_per_pt).floor());
    let y0 = js_max(0.0, (rect.y * px_per_pt).floor());
    let x1 = js_min(w, ((rect.x + rect.w) * px_per_pt).ceil());
    let y1 = js_min(height, ((rect.y + rect.h) * px_per_pt).ceil());
    let read = |values: &[f32], index: f64| {
        values
            .get(index as usize)
            .map_or(f64::NAN, |&v| f64::from(v))
    };
    let mut ink = 0.0;
    let mut y = y0;
    while y < y1 {
        let mut x = x0;
        while x < x1 {
            let index = y * w + x;
            ink = js_max(ink, (read(render, index) - read(textless, index)).abs());
            x += 1.0;
        }
        y += 1.0;
    }
    ink
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::libm::Fdlibm;

    fn word(text: &str, x: f64, y: f64) -> TextWord {
        TextWord {
            text: text.into(),
            rect: Rect {
                x,
                y,
                w: 10.0,
                h: 10.0,
            },
        }
    }

    #[test]
    fn normalise_applies_nfkc_and_js_trim() {
        assert_eq!(normalise_word("\u{FB01}ne"), "fine");
        assert_eq!(normalise_word("a\u{200B}b"), "ab");
        assert_eq!(normalise_word("\u{2060}\u{3000}x\u{00A0}"), "x");
        // U+0085 is not ECMAScript whitespace.
        assert_eq!(normalise_word("\u{0085}x"), "\u{0085}x");
        assert_eq!(normalise_word("Case"), "Case");
    }

    #[test]
    fn line_breaks_follow_vertical_centres() {
        let words = [
            word("a", 0.0, 0.0),
            word("b", 20.0, 4.0),
            word("c", 0.0, 20.0),
            word("d", 20.0, 24.0),
        ];
        assert_eq!(line_breaks(&words, |_, _| true), vec![0, 0, 1, 1]);
        assert_eq!(line_breaks(&words, |a, _| a != 0), vec![0, 1, 2, 2]);
    }

    #[test]
    fn a_centre_half_the_taller_box_away_stays_on_the_line() {
        let at = |y: f64, h: f64| TextWord {
            text: "w".into(),
            rect: Rect {
                x: 0.0,
                y,
                w: 10.0,
                h,
            },
        };
        // Centres 2 and 8: 6 pt apart, half the 12 pt box, in either order.
        assert_eq!(
            line_breaks(&[at(0.0, 4.0), at(2.0, 12.0)], |_, _| true),
            vec![0, 0]
        );
        assert_eq!(
            line_breaks(&[at(2.0, 12.0), at(0.0, 4.0)], |_, _| true),
            vec![0, 0]
        );
        let past = 2.0 + 1.0 / 1024.0;
        assert_eq!(
            line_breaks(&[at(0.0, 4.0), at(past, 12.0)], |_, _| true),
            vec![0, 1]
        );
    }

    #[test]
    fn lcs_prefers_the_earliest_pairs() {
        let a = ["x", "y", "x"];
        let b = ["x"];
        assert_eq!(lcs_pairs(&a, &b, |p, q| p == q), vec![(0, 0)]);
        let a = ["a", "b", "c", "d"];
        let b = ["b", "d", "a"];
        assert_eq!(lcs_pairs(&a, &b, |p, q| p == q), vec![(1, 0), (3, 1)]);
    }

    #[test]
    fn leftovers_match_the_nearest_equal_word_in_radius() {
        // LCS pairs the "b"s; the "a" then pairs with the nearer of the two DOM "a"s.
        let oracle = [word("a", 100.0, 0.0), word("b", 0.0, 0.0)];
        let dom = [
            word("b", 0.0, 0.0),
            word("a", 140.0, 0.0),
            word("a", 110.0, 0.0),
        ];
        let alignment = align_words::<Fdlibm>(&oracle, &dom, 1000.0, None).unwrap();
        assert_eq!(alignment.pairs, vec![(0, 2), (1, 0)]);
        assert_eq!(alignment.extra, vec![1]);
        assert!(alignment.missing.is_empty());
    }

    #[test]
    fn of_equally_near_leftovers_the_first_wins() {
        let oracle = [word("a", 100.0, 0.0), word("b", 0.0, 0.0)];
        let dom = [
            word("b", 0.0, 0.0),
            word("a", 110.0, 0.0),
            word("a", 90.0, 0.0),
        ];
        let alignment = align_words::<Fdlibm>(&oracle, &dom, 1000.0, None).unwrap();
        assert_eq!(alignment.pairs, vec![(0, 1), (1, 0)]);
    }

    #[test]
    fn leftovers_outside_the_radius_stay_missing() {
        // A 1000 pt long side: the radius is 70 pt.
        let oracle = [word("a", 0.0, 0.0), word("b", 0.0, 0.0)];
        let near = [word("b", 0.0, 0.0), word("a", 70.0, 0.0)];
        let far = [word("b", 0.0, 0.0), word("a", 70.5, 0.0)];
        assert_eq!(
            align_words::<Fdlibm>(&oracle, &near, 1000.0, None)
                .unwrap()
                .pairs,
            vec![(0, 1), (1, 0)]
        );
        let alignment = align_words::<Fdlibm>(&oracle, &far, 1000.0, None).unwrap();
        assert_eq!(alignment.pairs, vec![(1, 0)]);
        assert_eq!(alignment.missing, vec![0]);
        assert_eq!(alignment.extra, vec![1]);
    }

    #[test]
    fn inkless_pairs_become_missing_not_extra() {
        let oracle = [word("a", 0.0, 0.0), word("b", 20.0, 0.0)];
        let dom = [word("a", 0.0, 0.0), word("b", 20.0, 0.0)];
        let ink = |rect: &Rect| if rect.x > 10.0 { 11.9 } else { 12.0 };
        let alignment = align_words::<Fdlibm>(&oracle, &dom, 1000.0, Some(&ink)).unwrap();
        assert_eq!(alignment.pairs, vec![(0, 0)]);
        assert_eq!(alignment.missing, vec![1]);
        assert_eq!(alignment.inkless, vec![1]);
        assert!(alignment.extra.is_empty());
    }

    #[test]
    fn long_texts_align_by_lines() {
        let row = |y: f64, n: usize, text: &str| {
            (0..n)
                .map(|k| word(text, k as f64 * 20.0, y))
                .collect::<Vec<_>>()
        };
        let mut oracle = row(0.0, 1500, "x");
        oracle.extend(row(100.0, 600, "y"));
        let mut dom = row(0.0, 1500, "x");
        dom.extend(row(100.0, 600, "z"));
        let alignment = align_words::<Fdlibm>(&oracle, &dom, 100_000.0, None).unwrap();
        assert_eq!(alignment.pairs.len(), 1500);
        assert_eq!(alignment.pairs.last(), Some(&(1499, 1499)));
        assert_eq!(alignment.missing.len(), 600);
    }

    #[test]
    fn a_matched_line_short_of_words_is_an_error() {
        // "a b" as one DOM word against two oracle words on the same line.
        let mut oracle: Vec<TextWord> = (0..2001)
            .map(|k| word("q", k as f64 * 20.0, 1000.0))
            .collect();
        oracle.push(word("a", 0.0, 0.0));
        oracle.push(word("b", 20.0, 0.0));
        let dom = [word("a b", 0.0, 0.0)];
        assert_eq!(
            align_words::<Fdlibm>(&oracle, &dom, 1000.0, None),
            Err(MetricsError::LineWords)
        );
    }

    #[test]
    fn score_counts_and_places_words() {
        let oracle = [
            word("a", 0.0, 0.0),
            word("", 5.0, 5.0),
            word("b", 20.0, 0.0),
        ];
        let mut dom = vec![
            word("a", 3.0, 4.0),
            word("b", 20.0, 0.0),
            word("c", 40.0, 0.0),
        ];
        dom[1].rect.h = 20.0;
        dom[1].rect.y = -5.0;
        let alignment = align_words::<Fdlibm>(&oracle, &dom, 1000.0, None).unwrap();
        let score = text_score::<Fdlibm>(&oracle, &dom, &alignment, 1000.0).unwrap();
        assert_eq!(
            (score.oracle_words, score.dom_words, score.matched),
            (2, 3, 2)
        );
        assert_eq!(score.recall, 1.0);
        assert_eq!(score.precision, 2.0 / 3.0);
        assert_eq!(score.median_pm, Some(2.5));
        // 95th percentile of [0, 5] nearest rank: index ⌈1.9⌉ − 1 = 1; the ratios 1 and 2 have median 1.5.
        assert_eq!(score.p95_pm, Some(5.0));
        assert_eq!(score.height_ratio_median, Some(1.5));
    }

    #[test]
    fn nothing_to_match_scores_full_recall_and_no_distances() {
        let score = text_score::<Fdlibm>(&[], &[], &Alignment::default(), 720.0).unwrap();
        assert_eq!((score.recall, score.precision), (1.0, 1.0));
        assert_eq!(
            (score.median_pm, score.p95_pm, score.height_ratio_median),
            (None, None, None)
        );
    }

    #[test]
    fn ink_is_the_largest_difference_in_the_box() {
        let render = [0.0, 0.0, 0.0, 200.0, 0.0, 0.0];
        let textless = [0.0; 6];
        let rect = |x: f64, y: f64| Rect {
            x,
            y,
            w: 0.5,
            h: 0.5,
        };
        // 3×2 pixels, 2 px per point: the box (0.5, 0.5)–(1, 1) covers pixel (1, 1) → 0; (0, 0.5) covers (0, 1).
        assert_eq!(text_ink(&render, &textless, 3, &rect(0.0, 0.5), 2.0), 200.0);
        assert_eq!(text_ink(&render, &textless, 3, &rect(0.5, 0.5), 2.0), 0.0);
        assert_eq!(text_ink(&render, &textless, 3, &rect(-5.0, -5.0), 2.0), 0.0);
        assert!(text_ink(&render, &textless[..4], 3, &rect(0.5, 0.5), 2.0).is_nan());
    }
}
