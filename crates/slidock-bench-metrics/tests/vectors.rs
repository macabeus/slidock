//! The metrics against their pinned vectors on synthetic inputs (`tests/data/vectors.json`). Numbers that need no
//! elementary function (image preparation, `ssim@1`, `pixel@1`, the text alignment and counts, the classifier) must be
//! bit-identical with `Fdlibm`. Those that do (`color@1`, the text distances) must be bit-identical with `HostLibm` on
//! macOS arm64 (the vectors' host), and equal on the results grid with `Fdlibm`. The vectors pin the metric versions
//! they name: a metric whose numbers change is a new version, with vectors of its own. The file is this crate's own
//! recording (`recording` below), and a test holds it equal to a fresh one.

#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::sync::LazyLock;

use serde_json::Value;
use slidock_bench_metrics::{
    Alignment, ColorScore, Decline, Fdlibm, Lab, Libm, PixelmatchOptions, Rect, Rgba, Scores,
    SsimSummary, TextScore, TextWord, align_words, color_difference, deciding_decline, decode_png,
    delta_e00, js_round, line_breaks, luma, median, metric_pair, metric_size, normalise_word,
    over_white, percentile, pixel_score, pixelmatch, quantise, quantise_scores, region_lab,
    resample_area, scored_class, srgb_to_lab, ssim, text_ink, text_score, tile_lab,
};

static VECTORS: LazyLock<Value> = LazyLock::new(|| {
    serde_json::from_str(include_str!("data/vectors.json")).expect("vectors.json")
});

fn float(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value.as_str().expect("bits"), 16).expect("hex"))
}

fn nullable(value: &Value) -> Option<f64> {
    (!value.is_null()).then(|| float(value))
}

fn int(value: &Value) -> usize {
    value.as_u64().expect("integer") as usize
}

fn same(a: f64, b: f64) -> bool {
    a.to_bits() == b.to_bits()
}

/// `Fdlibm` results: equal on the results grid and within a relative 1e-12 of the reference.
fn close(a: f64, b: f64) -> bool {
    same(quantise(a), quantise(b)) && (a == b || ((a - b) / b).abs() < 1e-12)
}

fn fnv(bytes: impl IntoIterator<Item = u8>) -> u64 {
    let mut h: u32 = 0x811c_9dc5;
    for b in bytes {
        h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
    }
    u64::from(h)
}

fn fnv_f32(values: &[f32]) -> u64 {
    fnv(values.iter().flat_map(|v| v.to_le_bytes()))
}

fn fnv_f64(values: &[f64]) -> u64 {
    fnv(values.iter().flat_map(|v| v.to_le_bytes()))
}

/// xorshift32, as the generator draws it.
struct Rng(u32);

impl Rng {
    fn new(seed: u32) -> Self {
        Rng(if seed == 0 { 1 } else { seed })
    }

    fn next(&mut self) -> u32 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 17;
        self.0 ^= self.0 << 5;
        self.0
    }
}

fn paint(width: usize, height: usize, mut f: impl FnMut(usize, usize) -> [u8; 4]) -> Rgba {
    let mut data = Vec::with_capacity(width * height * 4);
    for y in 0..height {
        for x in 0..width {
            data.extend(f(x, y));
        }
    }
    Rgba::new(width, height, data).unwrap()
}

fn noise(width: usize, height: usize, seed: u32, alpha: bool) -> Rgba {
    let mut rng = Rng::new(seed);
    paint(width, height, |_, _| {
        let v = rng.next();
        [
            v as u8,
            (v >> 8) as u8,
            (v >> 16) as u8,
            if alpha { (v >> 24) as u8 } else { 255 },
        ]
    })
}

fn texture(x: usize, y: usize) -> [u8; 4] {
    [
        ((x * 7 + y * 3) % 256) as u8,
        ((x * 13) % 256) as u8,
        ((y * 11) % 256) as u8,
        255,
    ]
}

fn shapes(shift: usize, grey: u8) -> impl Fn(usize, usize) -> [u8; 4] {
    move |x, y| {
        let inside = |x0: usize, y0: usize, w: usize, h: usize| {
            x >= x0 && x < x0 + w && y >= y0 && y < y0 + h
        };
        for (x0, y0, w, h) in [(10, 8, 20, 12), (40, 20, 25, 18), (5, 35, 60, 6)] {
            if inside(x0 + shift, y0, w, h) {
                return [30, 40, 50, 255];
            }
            if inside(x0 + shift - 1, y0, 1, h) {
                return [grey, grey, grey, 255];
            }
        }
        [255; 4]
    }
}

fn scenes() -> Vec<(&'static str, Rgba, Rgba)> {
    let noisy = |x: usize, y: usize| {
        let t = texture(x, y);
        let add = |c: usize| (usize::from(t[c]) + ((x + y + c) * 37) % 41).min(255) as u8;
        [add(0), add(1), add(2), 255]
    };
    let blotted = |x: usize, y: usize| {
        if (20..28).contains(&x) && (20..28).contains(&y) {
            [255; 4]
        } else {
            texture(x, y)
        }
    };
    let pattern = |x: usize, y: usize| {
        [
            ((x * 3 + y) % 256) as u8,
            ((x ^ y) & 255) as u8,
            ((x * y) % 251) as u8,
            255,
        ]
    };
    vec![
        (
            "texture-noisy",
            paint(64, 48, texture),
            paint(64, 48, noisy),
        ),
        (
            "texture-blotted",
            paint(64, 48, texture),
            paint(64, 48, blotted),
        ),
        (
            "red-green",
            paint(32, 32, |_, _| [255, 0, 0, 255]),
            paint(32, 32, |_, _| [0, 76, 0, 255]),
        ),
        (
            "noise-alpha",
            noise(40, 30, 1, true),
            noise(40, 30, 2, true),
        ),
        (
            "shapes-shifted",
            paint(80, 50, shapes(0, 128)),
            paint(80, 50, shapes(1, 140)),
        ),
        (
            "upsampled",
            paint(97, 61, texture),
            noise(101, 57, 3, false),
        ),
        (
            "downsampled",
            paint(1000, 700, pattern),
            noise(1003, 698, 4, true),
        ),
    ]
}

#[test]
fn scenes_libm_free() {
    let vectors = VECTORS["scenes"].as_array().unwrap();
    let scenes = scenes();
    assert_eq!(vectors.len(), scenes.len());
    for ((name, oracle, render), v) in scenes.iter().zip(vectors) {
        assert_eq!(v["name"], *name);
        let (o, r) = metric_pair(oracle, render).unwrap();
        assert_eq!(
            (o.width(), o.height()),
            (int(&v["metric"]["width"]), int(&v["metric"]["height"])),
            "{name}"
        );
        assert_eq!(
            fnv(over_white(render).into_data()),
            v["overWhite"].as_u64().unwrap(),
            "{name} overWhite"
        );
        assert_eq!(
            fnv(o.data().iter().copied()),
            v["oracle"].as_u64().unwrap(),
            "{name} oracle"
        );
        assert_eq!(
            fnv(r.data().iter().copied()),
            v["render"].as_u64().unwrap(),
            "{name} render"
        );
        assert_eq!(
            fnv_f32(&luma(&r)),
            v["luma"].as_u64().unwrap(),
            "{name} luma"
        );
        let s = ssim(&o, &r).unwrap();
        assert!(same(s.mean, float(&v["ssim"]["mean"])), "{name} ssim.mean");
        assert!(same(s.p1, float(&v["ssim"]["p1"])), "{name} ssim.p1");
        assert_eq!(
            (s.map.columns, s.map.rows),
            (int(&v["ssim"]["columns"]), int(&v["ssim"]["rows"]))
        );
        assert_eq!(
            fnv_f32(&s.map.values),
            v["ssim"]["map"].as_u64().unwrap(),
            "{name} ssim map"
        );
        assert!(
            same(pixel_score(&o, &r).unwrap(), float(&v["pixel"])),
            "{name} pixel"
        );
        for m in v["pixelmatch"].as_array().unwrap() {
            let options = PixelmatchOptions {
                threshold: float(&m["threshold"]),
                include_aa: m["includeAA"].as_bool().unwrap(),
            };
            let metric = pixelmatch(o.data(), r.data(), o.width(), o.height(), options).unwrap();
            assert_eq!(metric, int(&m["metric"]), "{name} pixelmatch {options:?}");
            if !m["raw"].is_null() {
                let raw = pixelmatch(
                    oracle.data(),
                    render.data(),
                    oracle.width(),
                    oracle.height(),
                    options,
                )
                .unwrap();
                assert_eq!(raw, int(&m["raw"]), "{name} raw pixelmatch {options:?}");
            }
        }
    }
}

fn scenes_with<M: Libm>(strict: bool) {
    let check = if strict { same } else { close };
    for ((name, oracle, render), v) in scenes().iter().zip(VECTORS["scenes"].as_array().unwrap()) {
        let (o, r) = metric_pair(oracle, render).unwrap();
        let c = color_difference::<M>(&o, &r).unwrap();
        assert!(
            check(c.mean, float(&v["color"]["mean"])),
            "{name} color.mean {}",
            c.mean
        );
        assert!(
            check(c.p99, float(&v["color"]["p99"])),
            "{name} color.p99 {}",
            c.p99
        );
        let tiles: Vec<f64> = tile_lab::<M>(&o).tiles.into_iter().flatten().collect();
        if strict {
            assert_eq!(
                fnv_f64(&tiles),
                v["tiles"].as_u64().unwrap(),
                "{name} tiles"
            );
        }
        let region = region_lab::<M>(&r, 1.5, 2.25, 17.75, 9.5).unwrap();
        for (got, want) in region.iter().zip(v["regionLab"].as_array().unwrap()) {
            assert!(check(*got, float(want)), "{name} regionLab");
        }
    }
}

fn colours_with<M: Libm>(check: fn(f64, f64) -> bool) {
    let mut tally = (0, 0);
    for v in VECTORS["labs"].as_array().unwrap() {
        let rgb: Vec<u8> = v["rgb"]
            .as_array()
            .unwrap()
            .iter()
            .map(|c| int(c) as u8)
            .collect();
        let lab = srgb_to_lab::<M>(rgb[0], rgb[1], rgb[2]);
        for (got, want) in lab.iter().zip(v["lab"].as_array().unwrap()) {
            assert!(check(*got, float(want)), "lab of {rgb:?}: {got}");
            tally.0 += usize::from(same(*got, float(want)));
        }
    }
    let lab = |v: &Value| -> Lab {
        let a = v.as_array().unwrap();
        [float(&a[0]), float(&a[1]), float(&a[2])]
    };
    for v in VECTORS["deltas"].as_array().unwrap() {
        let got = delta_e00::<M>(lab(&v["a"]), lab(&v["b"]));
        assert!(
            check(got, float(&v["delta"])),
            "ΔE00 {} {}: {got}",
            v["a"],
            v["b"]
        );
        tally.1 += usize::from(same(got, float(&v["delta"])));
    }
    eprintln!(
        "bit-identical: {} Lab components, {} ΔE00",
        tally.0, tally.1
    );
}

#[test]
fn scenes_fdlibm() {
    scenes_with::<Fdlibm>(false);
}

#[test]
fn colours_fdlibm() {
    colours_with::<Fdlibm>(close);
}

#[cfg(all(feature = "host-libm", target_os = "macos"))]
#[test]
fn scenes_host_libm() {
    scenes_with::<slidock_bench_metrics::HostLibm>(true);
}

#[cfg(all(feature = "host-libm", target_os = "macos"))]
#[test]
fn colours_host_libm() {
    colours_with::<slidock_bench_metrics::HostLibm>(same);
}

#[test]
fn resampling_and_metric_sizes() {
    let source = noise(13, 9, 5, true);
    for v in VECTORS["resample"].as_array().unwrap() {
        let (w, h) = (int(&v["width"]), int(&v["height"]));
        let out = resample_area(&source, w, h).unwrap();
        assert_eq!(fnv(out.into_data()), v["hash"].as_u64().unwrap(), "{w}×{h}");
    }
    for v in VECTORS["metricSizes"].as_array().unwrap() {
        let size = metric_size(int(&v["width"]), int(&v["height"]));
        assert_eq!(
            size,
            (int(&v["out"]["width"]), int(&v["out"]["height"])),
            "{v}"
        );
    }
}

#[test]
fn pngs_decode_as_pngjs_does() {
    use base64::Engine as _;
    for v in VECTORS["pngs"].as_array().unwrap() {
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(v["png"].as_str().unwrap())
            .unwrap();
        let decoded = decode_png(&bytes).unwrap();
        assert_eq!(
            hex::encode(decoded.data()),
            v["rgba"].as_str().unwrap(),
            "{} {}",
            v["colorType"],
            v["bitDepth"]
        );
    }
}

#[test]
fn normalised_words() {
    for v in VECTORS["normalise"].as_array().unwrap() {
        assert_eq!(
            normalise_word(v["input"].as_str().unwrap()),
            v["output"].as_str().unwrap(),
            "{:?}",
            v["input"]
        );
    }
}

const VOCAB: [&str; 12] = [
    "the",
    "\u{FB01}le",
    "Slide",
    "a",
    "b",
    "\u{200B}",
    "word",
    "x",
    "Y",
    "z",
    "and",
    "42",
];

fn word_set(seed: u32, n: usize, per_line: usize) -> Vec<TextWord> {
    let mut r = Rng::new(seed);
    (0..n)
        .map(|i| {
            let text = VOCAB[r.next() as usize % VOCAB.len()].to_owned();
            let x = (i % per_line) as f64 * 30.0 + f64::from(r.next() % 7) / 4.0;
            let y = (i / per_line) as f64 * 20.0 + f64::from(r.next() % 5) / 8.0;
            let w = 20.0 + f64::from(r.next() % 9) / 2.0;
            TextWord {
                text,
                rect: Rect { x, y, w, h: 12.0 },
            }
        })
        .collect()
}

fn perturb(words: &[TextWord], seed: u32) -> Vec<TextWord> {
    let mut r = Rng::new(seed);
    let mut out: Vec<TextWord> = Vec::new();
    for word in words {
        let roll = r.next() % 20;
        if roll == 0 {
            continue;
        }
        let text = if roll == 1 {
            VOCAB[r.next() as usize % VOCAB.len()].to_owned()
        } else {
            word.text.clone()
        };
        let x = word.rect.x + (f64::from(r.next() % 41) - 20.0) / 10.0;
        let y = word.rect.y + (f64::from(r.next() % 21) - 10.0) / 10.0;
        let h = word.rect.h * if roll == 2 { 1.5 } else { 1.0 };
        let moved = TextWord {
            text,
            rect: Rect {
                x,
                y,
                w: word.rect.w,
                h,
            },
        };
        if roll == 3 && !out.is_empty() {
            out.insert(out.len() - 1, moved);
        } else {
            out.push(moved);
        }
    }
    out
}

fn boxes(words: &[TextWord]) -> u64 {
    fnv_f64(
        &words
            .iter()
            .flat_map(|w| [w.rect.x, w.rect.y, w.rect.w, w.rect.h])
            .collect::<Vec<_>>(),
    )
}

fn indices(v: &Value) -> Vec<usize> {
    v.as_array().unwrap().iter().map(int).collect()
}

fn alignment_of(v: &Value) -> Alignment {
    Alignment {
        pairs: v["pairs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|p| (int(&p[0]), int(&p[1])))
            .collect(),
        missing: indices(&v["missing"]),
        extra: indices(&v["extra"]),
        inkless: indices(&v["inkless"]),
    }
}

/// The text cases: name, word count, words per line and the two seeds.
const TEXT_CASES: [(&str, usize, usize, u32, u32); 3] = [
    ("small", 60, 8, 11, 12),
    ("medium", 400, 12, 13, 14),
    ("lines", 2100, 20, 15, 16),
];

/// One text case: the oracle words, the DOM words perturbed from them, and the lumas of a render that inks all but
/// every seventh DOM box over its text-less twin, at the case's size (`v` holds the slide's long side, the pixels per
/// point and the image size).
struct TextCase {
    oracle: Vec<TextWord>,
    dom: Vec<TextWord>,
    long_side: f64,
    px: f64,
    width: usize,
    with_text: Vec<f32>,
    without: Vec<f32>,
}

impl TextCase {
    fn new((_, n, per_line, seed_a, seed_b): (&str, usize, usize, u32, u32), v: &Value) -> Self {
        let oracle = word_set(seed_a, n, per_line);
        let dom = perturb(&oracle, seed_b);
        let long_side = float(&v["longSidePt"]);
        let px = float(&v["pxPerPt"]);
        let (width, height) = (int(&v["image"]["width"]), int(&v["image"]["height"]));
        let background = |x: usize, y: usize| [(200 + (x * 3 + y) % 50) as u8, 220, 230, 255];
        let textless = paint(width, height, background);
        let mut data = textless.data().to_vec();
        for (j, word) in dom.iter().enumerate() {
            if j % 7 == 0 {
                continue;
            }
            let b = word.rect;
            let (y1, x1) = (
                ((b.y + b.h) * px).min(height as f64),
                ((b.x + b.w) * px).min(width as f64),
            );
            let mut y = (b.y * px).ceil().max(0.0);
            while y < y1 {
                let mut x = (b.x * px).ceil().max(0.0);
                while x < x1 {
                    let i = (y as usize * width + x as usize) * 4;
                    data[i..i + 4].copy_from_slice(&[20, 20, 20, 255]);
                    x += 1.0;
                }
                y += 1.0;
            }
        }
        let render = Rgba::new(width, height, data).unwrap();
        TextCase {
            with_text: luma(&over_white(&render)),
            without: luma(&over_white(&textless)),
            oracle,
            dom,
            long_side,
            px,
            width,
        }
    }

    fn ink(&self, rect: &Rect) -> f64 {
        text_ink(&self.with_text, &self.without, self.width, rect, self.px)
    }
}

fn text_with<M: Libm>(check: fn(f64, f64) -> bool) {
    for (case, v) in TEXT_CASES
        .into_iter()
        .zip(VECTORS["text"].as_array().unwrap())
    {
        let name = case.0;
        assert_eq!(v["name"], name);
        let text = TextCase::new(case, v);
        let (oracle, dom, long_side) = (&text.oracle, &text.dom, text.long_side);
        assert_eq!(
            (oracle.len(), boxes(oracle)),
            (
                int(&v["oracle"]["count"]),
                v["oracle"]["boxes"].as_u64().unwrap()
            )
        );
        assert_eq!(
            (dom.len(), boxes(dom)),
            (int(&v["dom"]["count"]), v["dom"]["boxes"].as_u64().unwrap())
        );
        let ink = |rect: &Rect| text.ink(rect);
        for (word, want) in dom.iter().zip(v["inks"].as_array().unwrap()) {
            assert!(same(ink(&word.rect), float(want)), "{name} ink");
        }
        let lines: Vec<usize> = line_breaks(oracle, |_, _| true);
        assert_eq!(lines, indices(&v["lines"]), "{name} lines");
        let alignment = align_words::<M>(oracle, dom, long_side, Some(&ink)).unwrap();
        assert_eq!(alignment, alignment_of(&v["alignment"]), "{name} alignment");
        let plain = align_words::<M>(oracle, dom, long_side, None).unwrap();
        assert_eq!(
            plain,
            alignment_of(&v["plain"]),
            "{name} alignment without ink"
        );
        let score = text_score::<M>(oracle, dom, &alignment, long_side).unwrap();
        let s = &v["score"];
        assert_eq!(
            (score.oracle_words, score.dom_words, score.matched),
            (
                int(&s["oracleWords"]),
                int(&s["domWords"]),
                int(&s["matched"])
            )
        );
        assert!(
            same(score.recall, float(&s["recall"]))
                && same(score.precision, float(&s["precision"]))
        );
        for (got, want) in [
            (score.median_pm, nullable(&s["medianPm"])),
            (score.p95_pm, nullable(&s["p95Pm"])),
            (score.height_ratio_median, nullable(&s["heightRatioMedian"])),
        ] {
            match (got, want) {
                (Some(g), Some(w)) => assert!(check(g, w), "{name}: {g} {w}"),
                (g, w) => assert_eq!(g, w, "{name}"),
            }
        }
    }
}

#[test]
fn text_fdlibm() {
    text_with::<Fdlibm>(close);
}

#[cfg(all(feature = "host-libm", target_os = "macos"))]
#[test]
fn text_host_libm() {
    text_with::<slidock_bench_metrics::HostLibm>(same);
}

#[test]
fn numbers_round_and_quantise() {
    for v in VECTORS["numbers"].as_array().unwrap() {
        let x = float(&v["x"]);
        assert!(same(js_round(x), float(&v["round"])), "Math.round({x:e})");
        assert!(same(quantise(x), float(&v["quantise"])), "quantise({x:e})");
    }
    for v in VECTORS["lists"].as_array().unwrap() {
        let values: Vec<f64> = v["values"].as_array().unwrap().iter().map(float).collect();
        let m = median(&values);
        assert!(
            same(m, float(&v["median"])) || (m.is_nan() && float(&v["median"]).is_nan()),
            "median {values:?}"
        );
        let mut sorted = values.clone();
        sorted.sort_by(f64::total_cmp);
        for p in v["percentiles"].as_array().unwrap() {
            let (fraction, want) = (float(&p[0]), float(&p[1]));
            let got = percentile(&sorted, fraction);
            assert!(
                same(got, want) || (got.is_nan() && want.is_nan()),
                "percentile {fraction} of {values:?}"
            );
        }
    }
}

/// The `medians` lists, drawn in order from one generator: short ones every fourth, then increasing, decreasing and
/// zig-zag runs and small quarters, with NaN and `-0` mixed in.
fn median_lists(count: usize) -> Vec<Vec<f64>> {
    let mut rng = Rng::new(10);
    (0..count)
        .map(|t| {
            let n = (if t % 4 == 0 {
                rng.next() % 8
            } else {
                rng.next() % 600
            }) as usize;
            let mut values = Vec::with_capacity(n);
            for j in 0..n {
                let kind = rng.next() % 10;
                values.push(if kind < 2 {
                    f64::NAN
                } else if kind == 2 {
                    -0.0
                } else {
                    let noise = f64::from(rng.next() % 3);
                    match t % 4 {
                        1 => j as f64 + noise,
                        2 => (n - j) as f64 - noise,
                        3 => {
                            (if (j / 50) % 2 == 1 {
                                1000.0 - j as f64
                            } else {
                                j as f64
                            }) + noise
                        }
                        _ => f64::from(rng.next() % 64) / 4.0,
                    }
                });
            }
            values
        })
        .collect()
}

#[test]
fn medians_of_lists_with_nan() {
    let vectors = VECTORS["medians"].as_array().unwrap();
    for (t, (values, v)) in median_lists(vectors.len()).iter().zip(vectors).enumerate() {
        assert_eq!(values.len(), int(&v["n"]));
        let (got, want) = (median(values), float(&v["median"]));
        assert!(
            same(got, want) || (got.is_nan() && want.is_nan()),
            "median of list {t}: {got} ≠ {want}"
        );
    }
}

#[test]
fn line_breaks_at_the_boundary() {
    for v in VECTORS["lineBreaks"].as_array().unwrap() {
        let words: Vec<TextWord> = v["boxes"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| TextWord {
                text: "w".into(),
                rect: Rect {
                    x: float(&b[0]),
                    y: float(&b[1]),
                    w: float(&b[2]),
                    h: float(&b[3]),
                },
            })
            .collect();
        assert_eq!(
            line_breaks(&words, |_, _| true),
            indices(&v["lines"]),
            "{:?}",
            v["boxes"]
        );
    }
}

/// `sin` and `cos` of one argument in one function, where a compiler may fuse them into `sincos`.
#[cfg(all(feature = "host-libm", target_os = "macos"))]
#[inline(never)]
fn sin_and_cos(x: f64) -> (f64, f64) {
    use slidock_bench_metrics::HostLibm;
    (HostLibm::sin(x), HostLibm::cos(x))
}

#[cfg(all(feature = "host-libm", target_os = "macos"))]
#[test]
fn host_sin_and_cos_stay_separate_calls() {
    for v in VECTORS["sincos"].as_array().unwrap() {
        let x = float(&v["x"]);
        let (sin, cos) = sin_and_cos(std::hint::black_box(x));
        assert!(same(sin, float(&v["sin"])), "sin({x:e})");
        assert!(same(cos, float(&v["cos"])), "cos({x:e})");
    }
}

/// `normalise_word` of every scalar value of the block of 0x1000 code points from `start`, in five contexts, hashed.
fn nfkc_block_hash(start: u32) -> u64 {
    let mut h: u32 = 0x811c_9dc5;
    for c in (start..start + 0x1000).filter_map(char::from_u32) {
        let s = c.to_string();
        for context in [
            s.clone(),
            format!("a{s}\u{0316}"),
            format!("a\u{0301}{s}"),
            format!("{s}\u{0301}"),
            format!("\u{1100}{s}"),
        ] {
            for b in normalise_word(&context).bytes().chain([0xFF]) {
                h = (h ^ u32::from(b)).wrapping_mul(0x0100_0193);
            }
        }
    }
    u64::from(h)
}

/// The NFKC block hashes of all 0x110 blocks, on all cores.
fn nfkc_hashes() -> Vec<u64> {
    let starts: Vec<u32> = (0..0x110).map(|block| block * 0x1000).collect();
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    std::thread::scope(|scope| {
        let handles: Vec<_> = starts
            .chunks(starts.len().div_ceil(threads))
            .map(|part| {
                scope.spawn(|| {
                    part.iter()
                        .map(|&start| nfkc_block_hash(start))
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap())
            .collect()
    })
}

/// `normalise_word` of every scalar value in the generator's five contexts, hashed per block of 0x1000 code points:
/// the reference engine's NFKC (ICU, Unicode 17) leaves code points it does not know alone.
#[test]
fn normalisation_of_every_code_point() {
    let want: Vec<u64> = VECTORS["nfkc"]
        .as_array()
        .unwrap()
        .iter()
        .map(|h| h.as_u64().unwrap())
        .collect();
    assert_eq!(want.len(), 0x110);
    let failures: Vec<String> = nfkc_hashes()
        .iter()
        .zip(&want)
        .enumerate()
        .filter(|(_, (got, want))| got != want)
        .map(|(block, _)| format!("U+{:04X}", block * 0x1000))
        .collect();
    assert!(
        failures.is_empty(),
        "blocks differ: {}",
        failures.join(", ")
    );
}

#[test]
fn classes_and_declines() {
    for v in VECTORS["classes"].as_array().unwrap() {
        let p95 = nullable(&v["p95"]);
        let scores = Scores {
            ssim: SsimSummary {
                mean: float(&v["ssim"][0]),
                p1: float(&v["ssim"][1]),
            },
            color: ColorScore {
                mean: float(&v["color"]),
                p99: 0.0,
            },
            pixel: 1.0,
            text: TextScore {
                oracle_words: int(&v["oracleWords"]),
                dom_words: 5,
                matched: 5,
                recall: float(&v["recall"]),
                precision: 1.0,
                median_pm: p95,
                p95_pm: p95,
                height_ratio_median: p95.map(|_| 1.0),
            },
        };
        assert_eq!(
            scored_class(&scores).outcome().as_str(),
            v["class"].as_str().unwrap(),
            "{v}"
        );
    }
    for v in VECTORS["declines"].as_array().unwrap() {
        let declines: Vec<Decline> = v["declines"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| Decline {
                code: d["code"].as_str().unwrap().to_owned(),
                feature: None,
                via: d["via"].as_str().map(str::to_owned),
            })
            .collect();
        let deciding = deciding_decline(&declines).unwrap();
        let index = declines
            .iter()
            .position(|d| std::ptr::eq(d, deciding))
            .unwrap();
        assert_eq!(index, int(&v["deciding"]), "{v}");
    }
}

/// The scores the `quantised` record is of.
fn quantised_input() -> Scores {
    Scores {
        ssim: SsimSummary {
            mean: 0.98765,
            p1: -0.24735,
        },
        color: ColorScore {
            mean: 2.10455,
            p99: 25.17604,
        },
        pixel: 0.96985,
        text: TextScore {
            oracle_words: 5,
            dom_words: 5,
            matched: 5,
            recall: 2.0 / 3.0,
            precision: 1.0,
            median_pm: Some(f64::NAN),
            p95_pm: Some(3.94814),
            height_ratio_median: None,
        },
    }
}

#[test]
fn quantised_record() {
    let q = quantise_scores(&quantised_input());
    let v = &VECTORS["quantised"];
    for (got, want) in [
        (q.ssim.mean, &v["ssim"]["mean"]),
        (q.ssim.p1, &v["ssim"]["p1"]),
        (q.color.mean, &v["color"]["mean"]),
        (q.color.p99, &v["color"]["p99"]),
        (q.pixel, &v["pixel"]),
        (q.text.recall, &v["text"]["recall"]),
        (q.text.precision, &v["text"]["precision"]),
    ] {
        assert!(same(got, float(want)), "{got} {want}");
    }
    assert_eq!(q.text.median_pm, None);
    assert!(v["text"]["medianPm"].is_null() && v["text"]["heightRatioMedian"].is_null());
    assert!(same(q.text.p95_pm.unwrap(), float(&v["text"]["p95Pm"])));
}

/// The recorder of `tests/data/vectors.json`: every value the tests above read, computed by this crate (with
/// `HostLibm` where a number needs an elementary function) from the generators above and from the inputs the file
/// carries that no generator here draws (the scenes' pixelmatch options, the text cases' sizes, the PNGs, colours, Lab
/// pairs, words, numbers, lists, line boxes, sine arguments, class and decline cases). A new metric version records
/// its vectors with `UPDATE_VECTORS=1 cargo test -p slidock-bench-metrics --features host-libm --test vectors`.
#[cfg(all(feature = "host-libm", target_os = "macos"))]
mod recording {
    use super::*;
    use serde_json::json;
    use slidock_bench_metrics::{
        CLASSIFIER_VERSION, COLOR_VERSION, HostLibm, PIXEL_VERSION, SSIM_VERSION, TEXT_VERSION,
    };

    type M = HostLibm;

    /// The file the vectors live in.
    const FILE: &str = "tests/data/vectors.json";

    /// How many DOM words' ink the vectors record.
    const INKS: usize = 40;

    fn bits(x: f64) -> Value {
        json!(format!("{:016x}", x.to_bits()))
    }

    fn optional_bits(x: Option<f64>) -> Value {
        x.map_or(Value::Null, bits)
    }

    fn list(key: &str) -> &'static [Value] {
        VECTORS[key].as_array().unwrap()
    }

    fn scenes() -> Vec<Value> {
        super::scenes()
            .iter()
            .zip(list("scenes"))
            .map(|((name, oracle, render), v)| {
                let (o, r) = metric_pair(oracle, render).unwrap();
                let s = ssim(&o, &r).unwrap();
                let c = color_difference::<M>(&o, &r).unwrap();
                let tiles: Vec<f64> = tile_lab::<M>(&o).tiles.into_iter().flatten().collect();
                let same_size =
                    (oracle.width(), oracle.height()) == (render.width(), render.height());
                let pixelmatches: Vec<Value> = v["pixelmatch"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|m| {
                        let options = PixelmatchOptions {
                            threshold: float(&m["threshold"]),
                            include_aa: m["includeAA"].as_bool().unwrap(),
                        };
                        let metric =
                            pixelmatch(o.data(), r.data(), o.width(), o.height(), options).unwrap();
                        let raw = same_size.then(|| {
                            let (width, height) = (oracle.width(), oracle.height());
                            pixelmatch(oracle.data(), render.data(), width, height, options)
                                .unwrap()
                        });
                        json!({ "threshold": bits(options.threshold), "includeAA": options.include_aa,
                                "metric": metric, "raw": raw })
                    })
                    .collect();
                let region = region_lab::<M>(&r, 1.5, 2.25, 17.75, 9.5).unwrap();
                json!({
                    "name": name,
                    "metric": { "width": o.width(), "height": o.height() },
                    "overWhite": fnv(over_white(render).into_data()),
                    "oracle": fnv(o.data().iter().copied()),
                    "render": fnv(r.data().iter().copied()),
                    "luma": fnv_f32(&luma(&r)),
                    "ssim": { "mean": bits(s.mean), "p1": bits(s.p1), "columns": s.map.columns,
                              "rows": s.map.rows, "map": fnv_f32(&s.map.values) },
                    "tiles": fnv_f64(&tiles),
                    "color": { "mean": bits(c.mean), "p99": bits(c.p99) },
                    "pixel": bits(pixel_score(&o, &r).unwrap()),
                    "pixelmatch": pixelmatches,
                    "regionLab": region.iter().map(|x| bits(*x)).collect::<Vec<_>>(),
                })
            })
            .collect()
    }

    fn alignment(a: &Alignment) -> Value {
        json!({ "pairs": a.pairs.iter().map(|(o, d)| json!([o, d])).collect::<Vec<_>>(),
                "missing": a.missing, "extra": a.extra, "inkless": a.inkless })
    }

    fn text() -> Vec<Value> {
        TEXT_CASES
            .into_iter()
            .zip(list("text"))
            .map(|(case, v)| {
                let text = TextCase::new(case, v);
                let (oracle, dom, long_side) = (&text.oracle, &text.dom, text.long_side);
                let ink = |rect: &Rect| text.ink(rect);
                let aligned = align_words::<M>(oracle, dom, long_side, Some(&ink)).unwrap();
                let plain = align_words::<M>(oracle, dom, long_side, None).unwrap();
                let score = text_score::<M>(oracle, dom, &aligned, long_side).unwrap();
                json!({
                    "name": case.0,
                    "longSidePt": bits(long_side),
                    "pxPerPt": bits(text.px),
                    "image": v["image"],
                    "oracle": { "count": oracle.len(), "boxes": boxes(oracle) },
                    "dom": { "count": dom.len(), "boxes": boxes(dom) },
                    "inks": dom.iter().take(INKS).map(|w| bits(ink(&w.rect))).collect::<Vec<_>>(),
                    "alignment": alignment(&aligned),
                    "plain": alignment(&plain),
                    "score": {
                        "oracleWords": score.oracle_words, "domWords": score.dom_words,
                        "matched": score.matched, "recall": bits(score.recall),
                        "precision": bits(score.precision), "medianPm": optional_bits(score.median_pm),
                        "p95Pm": optional_bits(score.p95_pm),
                        "heightRatioMedian": optional_bits(score.height_ratio_median),
                    },
                    "lines": line_breaks(oracle, |_, _| true),
                })
            })
            .collect()
    }

    fn colours() -> (Vec<Value>, Vec<Value>) {
        let labs = list("labs")
            .iter()
            .map(|v| {
                let rgb: Vec<u8> = v["rgb"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|c| int(c) as u8)
                    .collect();
                let lab = srgb_to_lab::<M>(rgb[0], rgb[1], rgb[2]);
                json!({ "rgb": rgb, "lab": lab.iter().map(|x| bits(*x)).collect::<Vec<_>>() })
            })
            .collect();
        let lab = |v: &Value| -> Lab {
            let a = v.as_array().unwrap();
            [float(&a[0]), float(&a[1]), float(&a[2])]
        };
        let deltas = list("deltas")
            .iter()
            .map(|v| {
                let delta = delta_e00::<M>(lab(&v["a"]), lab(&v["b"]));
                json!({ "a": v["a"], "b": v["b"], "delta": bits(delta) })
            })
            .collect();
        (labs, deltas)
    }

    fn numbers() -> (Vec<Value>, Vec<Value>) {
        let numbers = list("numbers")
            .iter()
            .map(|v| {
                let x = float(&v["x"]);
                json!({ "x": bits(x), "round": bits(js_round(x)), "quantise": bits(quantise(x)) })
            })
            .collect();
        let lists = list("lists")
            .iter()
            .map(|v| {
                let values: Vec<f64> = v["values"].as_array().unwrap().iter().map(float).collect();
                let mut sorted = values.clone();
                sorted.sort_by(f64::total_cmp);
                let percentiles: Vec<Value> = v["percentiles"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|p| {
                        let fraction = float(&p[0]);
                        json!([bits(fraction), bits(percentile(&sorted, fraction))])
                    })
                    .collect();
                json!({ "values": values.iter().map(|x| bits(*x)).collect::<Vec<_>>(),
                        "median": bits(median(&values)), "percentiles": percentiles })
            })
            .collect();
        (numbers, lists)
    }

    fn classes() -> (Vec<Value>, Vec<Value>) {
        let classes = list("classes")
            .iter()
            .map(|v| {
                let p95 = nullable(&v["p95"]);
                let scores = Scores {
                    ssim: SsimSummary {
                        mean: float(&v["ssim"][0]),
                        p1: float(&v["ssim"][1]),
                    },
                    color: ColorScore {
                        mean: float(&v["color"]),
                        p99: 0.0,
                    },
                    pixel: 1.0,
                    text: TextScore {
                        oracle_words: int(&v["oracleWords"]),
                        dom_words: 5,
                        matched: 5,
                        recall: float(&v["recall"]),
                        precision: 1.0,
                        median_pm: p95,
                        p95_pm: p95,
                        height_ratio_median: p95.map(|_| 1.0),
                    },
                };
                json!({ "ssim": v["ssim"], "color": v["color"], "recall": v["recall"],
                        "oracleWords": v["oracleWords"], "p95": v["p95"],
                        "class": scored_class(&scores).outcome().as_str() })
            })
            .collect();
        let declines = list("declines")
            .iter()
            .map(|v| {
                let declines: Vec<Decline> = v["declines"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|d| Decline {
                        code: d["code"].as_str().unwrap().to_owned(),
                        feature: None,
                        via: d["via"].as_str().map(str::to_owned),
                    })
                    .collect();
                let deciding = deciding_decline(&declines).unwrap();
                let index = declines
                    .iter()
                    .position(|d| std::ptr::eq(d, deciding))
                    .unwrap();
                json!({ "declines": v["declines"], "deciding": index })
            })
            .collect();
        (classes, declines)
    }

    fn line_boxes() -> Vec<Value> {
        list("lineBreaks")
            .iter()
            .map(|v| {
                let words: Vec<TextWord> = v["boxes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| TextWord {
                        text: "w".into(),
                        rect: Rect {
                            x: float(&b[0]),
                            y: float(&b[1]),
                            w: float(&b[2]),
                            h: float(&b[3]),
                        },
                    })
                    .collect();
                json!({ "boxes": v["boxes"], "lines": line_breaks(&words, |_, _| true) })
            })
            .collect()
    }

    fn quantised() -> Value {
        let q = quantise_scores(&quantised_input());
        let t = q.text;
        json!({
            "ssim": { "mean": bits(q.ssim.mean), "p1": bits(q.ssim.p1) },
            "color": { "mean": bits(q.color.mean), "p99": bits(q.color.p99) },
            "pixel": bits(q.pixel),
            "text": {
                "oracleWords": bits(t.oracle_words as f64), "domWords": bits(t.dom_words as f64),
                "matched": bits(t.matched as f64), "recall": bits(t.recall),
                "precision": bits(t.precision), "medianPm": optional_bits(t.median_pm),
                "p95Pm": optional_bits(t.p95_pm), "heightRatioMedian": optional_bits(t.height_ratio_median),
            },
        })
    }

    /// The file's text: one member per line, a list of strings inline, every other list one item per line, each
    /// item and every other value as compact JSON.
    fn print(members: Vec<(&str, Value)>) -> String {
        let compact = |value: &Value| serde_json::to_string(value).unwrap();
        let printed: Vec<String> = members
            .iter()
            .map(|(key, value)| {
                let value = match value {
                    Value::Array(items) if items.iter().all(Value::is_string) => {
                        let items: Vec<String> = items.iter().map(compact).collect();
                        format!("[{}]", items.join(", "))
                    }
                    Value::Array(items) => {
                        let items: Vec<String> = items.iter().map(compact).collect();
                        format!("[\n{}\n]", items.join(",\n"))
                    }
                    value => compact(value),
                };
                format!("{}: {value}", compact(&json!(key)))
            })
            .collect();
        format!("{{\n{}\n}}\n", printed.join(",\n"))
    }

    /// The vectors as this crate computes them.
    fn record() -> String {
        let source = noise(13, 9, 5, true);
        let resample = list("resample")
            .iter()
            .map(|v| {
                let (w, h) = (int(&v["width"]), int(&v["height"]));
                let hash = fnv(resample_area(&source, w, h).unwrap().into_data());
                json!({ "width": w, "height": h, "hash": hash })
            })
            .collect();
        let metric_sizes = list("metricSizes")
            .iter()
            .map(|v| {
                let (w, h) = (int(&v["width"]), int(&v["height"]));
                let (out_w, out_h) = metric_size(w, h);
                json!({ "width": w, "height": h, "out": { "width": out_w, "height": out_h } })
            })
            .collect();
        let pngs = list("pngs")
            .iter()
            .map(|v| {
                use base64::Engine as _;
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(v["png"].as_str().unwrap())
                    .unwrap();
                let rgba = hex::encode(decode_png(&bytes).unwrap().data());
                json!({ "colorType": v["colorType"], "bitDepth": v["bitDepth"], "png": v["png"],
                        "rgba": rgba })
            })
            .collect();
        let normalise = list("normalise")
            .iter()
            .map(|v| {
                let input = v["input"].as_str().unwrap();
                json!({ "input": input, "output": normalise_word(input) })
            })
            .collect();
        let medians = median_lists(list("medians").len())
            .iter()
            .map(|values| json!({ "n": values.len(), "median": bits(median(values)) }))
            .collect();
        let sincos = list("sincos")
            .iter()
            .map(|v| {
                let x = float(&v["x"]);
                let (sin, cos) = sin_and_cos(std::hint::black_box(x));
                json!({ "x": bits(x), "sin": bits(sin), "cos": bits(cos) })
            })
            .collect();
        let (labs, deltas) = colours();
        let (numbers, lists) = numbers();
        let (classes, declines) = classes();
        let versions = [
            SSIM_VERSION,
            COLOR_VERSION,
            PIXEL_VERSION,
            TEXT_VERSION,
            CLASSIFIER_VERSION,
        ];
        print(vec![
            ("metrics", json!(versions)),
            ("host", json!("macOS arm64")),
            ("scenes", Value::Array(scenes())),
            ("resample", Value::Array(resample)),
            ("metricSizes", Value::Array(metric_sizes)),
            ("pngs", Value::Array(pngs)),
            ("labs", Value::Array(labs)),
            ("deltas", Value::Array(deltas)),
            ("normalise", Value::Array(normalise)),
            ("text", Value::Array(text())),
            ("numbers", Value::Array(numbers)),
            ("lists", Value::Array(lists)),
            ("classes", Value::Array(classes)),
            ("declines", Value::Array(declines)),
            ("medians", Value::Array(medians)),
            ("lineBreaks", Value::Array(line_boxes())),
            ("sincos", Value::Array(sincos)),
            ("nfkc", json!(nfkc_hashes())),
            ("quantised", quantised()),
        ])
    }

    /// The committed vectors are what this crate records today, byte for byte, so each is reproducible; with
    /// `UPDATE_VECTORS` set the test writes the recording instead.
    #[test]
    fn the_committed_vectors_are_a_fresh_recording() {
        let recorded = record();
        if std::env::var_os("UPDATE_VECTORS").is_some() {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(FILE);
            std::fs::write(&path, &recorded).unwrap();
            eprintln!("recorded {}", path.display());
            return;
        }
        let committed = include_str!("data/vectors.json");
        if let Some((line, (got, want))) = recorded
            .lines()
            .zip(committed.lines())
            .enumerate()
            .find(|(_, (got, want))| got != want)
        {
            let cut = |text: &str| text.chars().take(300).collect::<String>();
            panic!(
                "{FILE}:{}: the recording differs\n recorded: {}\ncommitted: {}",
                line + 1,
                cut(got),
                cut(want)
            );
        }
        assert_eq!(
            recorded, committed,
            "{FILE}: the recording differs in length"
        );
    }
}
