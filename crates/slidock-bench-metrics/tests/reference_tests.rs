//! The metrics' and the classifier's behaviour tests, with their image builders.

#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use slidock_bench_metrics::{
    Classification, ColorScore, CompileFailure, Decline, FailedReason, Fdlibm, Outcome, Rect,
    RenderErrorReason, Rgba, RowFacts, Scores, SsimSummary, TextScore, TextWord, align_words,
    classify, color_difference, deciding_decline, delta_e00, lcs_pairs, luma, metric_size,
    normalise_word, over_white, percentile, pixel_score, quantise, resample_area, srgb_to_lab,
    ssim, text_ink, text_score,
};

/// An image filled by `paint(x, y) → [r, g, b, a]`.
fn image(width: usize, height: usize, paint: impl Fn(usize, usize) -> [u8; 4]) -> Rgba {
    let mut bytes = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            bytes.extend(paint(x, y));
        }
    }
    Rgba::new(width, height, bytes).unwrap()
}

/// Stripes and a gradient, so windows have variance.
fn texture(x: usize, y: usize) -> [u8; 4] {
    [
        ((x * 7 + y * 3) % 256) as u8,
        ((x * 13) % 256) as u8,
        ((y * 11) % 256) as u8,
        255,
    ]
}

#[test]
fn resample_averages_areas_and_keeps_a_same_size_image() {
    let checker = image(
        4,
        2,
        |x, _| if x % 2 == 1 { [255; 4] } else { [0, 0, 0, 255] },
    );
    assert_eq!(
        resample_area(&checker, 2, 1).unwrap().data(),
        &[128, 128, 128, 255, 128, 128, 128, 255]
    );
    assert_eq!(resample_area(&checker, 4, 2).unwrap(), checker);
    assert_eq!(metric_size(1920, 1080), (960, 540));
    assert_eq!(metric_size(1440, 1920), (720, 960));
}

#[test]
fn over_white_composites_transparency() {
    assert_eq!(
        over_white(&image(1, 1, |_, _| [0, 0, 0, 128])).data(),
        &[127, 127, 127, 255]
    );
}

#[test]
fn percentiles_and_quantisation() {
    let values: Vec<f64> = (1..=10).map(f64::from).collect();
    assert_eq!(percentile(&values, 0.95), 10.0);
    assert_eq!(percentile(&values, 0.5), 5.0);
    assert_eq!(quantise(0.123456), 0.1235);
    assert_eq!(quantise(-0.00001).to_bits(), 0.0f64.to_bits());
}

#[test]
fn ssim_is_one_on_identical_images() {
    let a = image(64, 48, texture);
    let score = ssim(&a, &a).unwrap();
    assert!((score.mean - 1.0).abs() < 1e-10 && (score.p1 - 1.0).abs() < 1e-10);
    assert_eq!((score.map.columns, score.map.rows), (15, 11));
}

#[test]
fn ssim_drops_with_noise_and_its_p1_catches_a_local_miss() {
    let a = image(64, 48, texture);
    let noisy = image(64, 48, |x, y| {
        let t = texture(x, y);
        let add = |c: usize| (usize::from(t[c]) + ((x + y + c) * 37) % 41).min(255) as u8;
        [add(0), add(1), add(2), 255]
    });
    assert!(ssim(&a, &noisy).unwrap().mean < 0.95);
    let blotted = image(64, 48, |x, y| {
        if (20..28).contains(&x) && (20..28).contains(&y) {
            [255; 4]
        } else {
            texture(x, y)
        }
    });
    let score = ssim(&a, &blotted).unwrap();
    assert!(score.mean > 0.9);
    assert!(score.p1 < 0.3);
}

#[test]
fn delta_e_matches_sharma_wu_dalal() {
    let cases = [
        ([50.0, 2.6772, -79.7751], [50.0, 0.0, -82.7485], 2.0425),
        ([50.0, 0.0, 0.0], [50.0, -1.0, 2.0], 2.3669),
        ([50.0, 2.5, 0.0], [73.0, 25.0, -18.0], 27.1492),
        ([2.0776, 0.0795, -1.135], [0.9033, -0.0636, -0.5514], 0.9082),
    ];
    for (a, b, want) in cases {
        assert!((delta_e00::<Fdlibm>(a, b) - want).abs() < 5e-5);
    }
}

#[test]
fn srgb_white_and_black_in_lab() {
    let white = srgb_to_lab::<Fdlibm>(255, 255, 255).map(|v| (v * 100.0).round() / 100.0 + 0.0);
    assert_eq!(white, [100.0, 0.0, 0.0]);
    assert_eq!(srgb_to_lab::<Fdlibm>(0, 0, 0), [0.0, 0.0, 0.0]);
}

#[test]
fn color_catches_a_red_green_swap_of_equal_luma() {
    let red = image(32, 32, |_, _| [255, 0, 0, 255]);
    let green = image(32, 32, |_, _| [0, 76, 0, 255]);
    assert_eq!(
        color_difference::<Fdlibm>(&red, &red).unwrap(),
        ColorScore {
            mean: 0.0,
            p99: 0.0
        }
    );
    assert!(ssim(&red, &green).unwrap().mean > 0.999);
    assert!(color_difference::<Fdlibm>(&red, &green).unwrap().mean > 40.0);
}

#[test]
fn pixel_counts_differing_pixels() {
    let a = image(10, 10, texture);
    let b = image(10, 10, |x, y| {
        if x == 3 && y == 3 {
            [255, 0, 255, 255]
        } else {
            texture(x, y)
        }
    });
    assert_eq!(pixel_score(&a, &a).unwrap(), 1.0);
    assert!((pixel_score(&a, &b).unwrap() - 0.99).abs() < 1e-10);
}

fn rect(x: f64, y: f64) -> Rect {
    Rect {
        x,
        y,
        w: 20.0,
        h: 10.0,
    }
}

fn words(items: &[(&str, Rect)]) -> Vec<TextWord> {
    items
        .iter()
        .map(|(text, rect)| TextWord {
            text: (*text).into(),
            rect: *rect,
        })
        .collect()
}

#[test]
fn normalise_ligatures_and_zero_width_keeping_case() {
    assert_eq!(normalise_word("\u{FB01}ne\u{200B}"), "fine");
    assert_eq!(normalise_word("Fine"), "Fine");
}

#[test]
fn align_by_lcs_then_leftovers_by_text_and_nearest_centre() {
    assert_eq!(
        lcs_pairs(&["a", "b", "c"], &["a", "c"], |x, y| x == y),
        vec![(0, 0), (2, 1)]
    );
    let oracle = words(&[
        ("Hello", rect(0.0, 0.0)),
        ("\u{FB01}ne", rect(30.0, 0.0)),
        ("world", rect(60.0, 0.0)),
        ("gone", rect(90.0, 0.0)),
    ]);
    // The render reorders two words and drops one.
    let dom = words(&[
        ("world", rect(61.0, 0.0)),
        ("Hello", rect(1.0, 0.0)),
        ("fine", rect(30.0, 1.0)),
    ]);
    let alignment = align_words::<Fdlibm>(&oracle, &dom, 720.0, None).unwrap();
    assert_eq!(alignment.pairs, vec![(0, 1), (1, 2), (2, 0)]);
    assert_eq!((alignment.missing, alignment.extra), (vec![3], vec![]));
    let alignment = align_words::<Fdlibm>(&oracle, &dom, 720.0, None).unwrap();
    let score = text_score::<Fdlibm>(&oracle, &dom, &alignment, 720.0).unwrap();
    assert_eq!(
        (score.oracle_words, score.dom_words, score.matched),
        (4, 3, 3)
    );
    assert_eq!((score.recall, score.precision), (0.75, 1.0));
    assert!((score.median_pm.unwrap() - 1000.0 / 720.0).abs() < 1e-6);
}

#[test]
fn a_leftover_beyond_70_per_mille_stays_unmatched() {
    let oracle = words(&[("a", rect(0.0, 0.0)), ("b", rect(20.0, 0.0))]);
    let dom = words(&[("b", rect(20.0, 0.0)), ("a", rect(500.0, 0.0))]);
    let alignment = align_words::<Fdlibm>(&oracle, &dom, 720.0, None).unwrap();
    assert_eq!(alignment.pairs, vec![(1, 0)]);
    assert_eq!((alignment.missing, alignment.extra), (vec![0], vec![1]));
}

#[test]
fn a_word_without_ink_of_its_own_is_missing() {
    // A dark block where the word sits, in both renders: only the text's own pixels differ between them.
    let picture = |x: usize, y: usize| {
        if (50..70).contains(&x) && (10..20).contains(&y) {
            [0, 0, 0, 255]
        } else {
            [255; 4]
        }
    };
    let textless = image(100, 40, picture);
    let render = image(100, 40, |x, y| {
        if (10..30).contains(&x) && (10..20).contains(&y) {
            [0, 0, 0, 255]
        } else {
            picture(x, y)
        }
    });
    let (with_text, without) = (luma(&over_white(&render)), luma(&over_white(&textless)));
    let px_per_pt = render.width() as f64 / 100.0;
    let ink = |r: &Rect| text_ink(&with_text, &without, render.width(), r, px_per_pt);
    assert_eq!(ink(&rect(10.0, 10.0)), 255.0);
    assert_eq!(ink(&rect(50.0, 10.0)), 0.0);
    let oracle = words(&[("seen", rect(10.0, 10.0)), ("pictured", rect(50.0, 10.0))]);
    let alignment = align_words::<Fdlibm>(&oracle, &oracle, 100.0, Some(&ink)).unwrap();
    assert_eq!(
        (
            alignment.pairs.clone(),
            alignment.missing.clone(),
            alignment.inkless.clone()
        ),
        (vec![(0, 0)], vec![1], vec![1])
    );
    assert_eq!(
        text_score::<Fdlibm>(&oracle, &oracle, &alignment, 100.0)
            .unwrap()
            .recall,
        0.5
    );
}

#[test]
fn a_slide_without_oracle_words_has_full_recall_and_no_positions() {
    let dom = words(&[("extra", rect(0.0, 0.0))]);
    let alignment = slidock_bench_metrics::Alignment {
        extra: vec![0],
        ..Default::default()
    };
    let score = text_score::<Fdlibm>(&[], &dom, &alignment, 720.0).unwrap();
    assert_eq!(
        score,
        TextScore {
            oracle_words: 0,
            dom_words: 1,
            matched: 0,
            recall: 1.0,
            precision: 0.0,
            median_pm: None,
            p95_pm: None,
            height_ratio_median: None,
        }
    );
}

fn text(recall: f64, p95: Option<f64>, oracle_words: usize) -> TextScore {
    TextScore {
        oracle_words,
        dom_words: 10,
        matched: oracle_words,
        recall,
        precision: 1.0,
        median_pm: p95.map(|_| 0.5),
        p95_pm: p95,
        height_ratio_median: Some(1.0),
    }
}

fn scores(mean: f64, p1: f64, color: f64, text: TextScore) -> Scores {
    Scores {
        ssim: SsimSummary { mean, p1 },
        color: ColorScore {
            mean: color,
            p99: color * 2.0,
        },
        pixel: 0.99,
        text,
    }
}

fn decline(code: &str, via: Option<&str>) -> Decline {
    Decline {
        code: code.into(),
        feature: None,
        via: via.map(Into::into),
    }
}

#[test]
fn failed_declined_and_render_error_come_before_any_score() {
    let good = scores(0.99, 0.9, 1.0, text(1.0, Some(1.0), 10));
    let facts = |f: RowFacts| classify(&f);
    assert_eq!(
        facts(RowFacts {
            compile: Some(CompileFailure::Timeout),
            scores: Some(good),
            ..RowFacts::default()
        }),
        Classification::Failed {
            reason: FailedReason::CompileTimeout,
            code: None,
            memory: false
        }
    );
    assert_eq!(
        facts(RowFacts {
            compile: Some(CompileFailure::Memory),
            ..RowFacts::default()
        }),
        Classification::Failed {
            reason: FailedReason::CompileCrash,
            code: None,
            memory: true
        }
    );
    assert_eq!(
        facts(RowFacts {
            missing: true,
            declines: vec![decline("IR0910", None)],
            ..RowFacts::default()
        }),
        Classification::Failed {
            reason: FailedReason::RowMissing,
            code: None,
            memory: false
        }
    );
    assert_eq!(
        facts(RowFacts {
            failed_code: Some("IR0908".into()),
            declines: vec![decline("IR0910", None)],
            ..RowFacts::default()
        }),
        Classification::Failed {
            reason: FailedReason::SlideFailed,
            code: Some("IR0908".into()),
            memory: false
        }
    );
    assert_eq!(
        facts(RowFacts {
            declines: vec![decline("MID5601", Some("l1"))],
            render_error: Some(RenderErrorReason::OracleMissing),
            ..RowFacts::default()
        }),
        Classification::Declined(decline("MID5601", Some("l1")))
    );
    assert_eq!(
        facts(RowFacts {
            render_error: Some(RenderErrorReason::RenderTimeout),
            scores: Some(good),
            ..RowFacts::default()
        }),
        Classification::RenderError(RenderErrorReason::RenderTimeout)
    );
}

#[test]
fn the_lowest_decline_code_decides_the_slides_own_first() {
    let list = [
        decline("HTML6002", None),
        decline("IR0910", Some("m1")),
        decline("MID5601", None),
    ];
    assert_eq!(
        deciding_decline(&list),
        Some(&decline("IR0910", Some("m1")))
    );
    let list = [decline("IR0910", Some("l1")), decline("IR0910", None)];
    assert_eq!(deciding_decline(&list), Some(&decline("IR0910", None)));
}

#[test]
fn match_close_and_mismatch_by_every_threshold() {
    let class = |s: Scores| {
        classify(&RowFacts {
            scores: Some(s),
            ..RowFacts::default()
        })
        .outcome()
    };
    let t = || text(1.0, Some(1.0), 10);
    assert_eq!(class(scores(0.98, 0.7, 2.0, t())), Outcome::Match);
    // A small local miss: a good mean over a low tail.
    assert_eq!(class(scores(0.98, 0.5, 2.0, t())), Outcome::Close);
    // A hue error of equal luma.
    assert_eq!(class(scores(0.98, 0.7, 5.0, t())), Outcome::Close);
    assert_eq!(class(scores(0.98, 0.7, 9.0, t())), Outcome::Mismatch);
    assert_eq!(
        class(scores(0.98, 0.7, 2.0, text(1.0, Some(5.0), 10))),
        Outcome::Close
    );
    assert_eq!(
        class(scores(0.98, 0.7, 2.0, text(0.95, Some(1.0), 10))),
        Outcome::Close
    );
    assert_eq!(
        class(scores(0.98, 0.7, 2.0, text(0.8, Some(1.0), 10))),
        Outcome::Mismatch
    );
    assert_eq!(class(scores(0.89, 0.7, 2.0, t())), Outcome::Mismatch);
    // A textless slide counts as placed.
    assert_eq!(
        class(scores(0.98, 0.7, 2.0, text(1.0, None, 0))),
        Outcome::Match
    );
}
