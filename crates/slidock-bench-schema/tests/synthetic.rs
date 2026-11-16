//! The synthetic bench files under `tests/data/` that the bench UI develops against: a corpus manifest, results of
//! every lane, a runs index and a parity report, all made up (no corpus name, text or hash). They are built here
//! from readable builders; the test checks each committed file equals its build, parses and validates it, and
//! prints it back byte for byte. `UPDATE_GOLDEN=1 cargo test -p slidock-bench-schema --test synthetic` rewrites them.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use slidock_bench_schema::corpus::{
    CorpusManifest, FontsExpect, KeynoteExporter, KeynoteOracle, KeynoteStages, KnownLoss,
    LossKind, Oracle, PdfExporter, PdfOracle, Presentation, Source, StagesOracle,
};
use slidock_bench_schema::lanes::{
    DeliveryCounts, EffectCounts, EffectPlay, EffectRef, FontSource, FontStyle, FontsPairReport,
    FontsRowMetrics, FontsSummary, FrameCheck, FrameMatch, FrameStrip, FrameSummary, GlyphCounts,
    HostCheck, LaneSummaries, MotionClasses, MotionItem, MotionPairReport, MotionRowMetrics,
    MotionSummary, MotionVerdict, RequestedFace, RevealCheck, RevealMiss, ShippedFace, StepCounts,
    StepPlay, StepTiming, TimelineFidelity, TimelineField, TimelineMismatch, TimelineSummary,
    TransitionPlay, VerdictCounts,
};
use slidock_bench_schema::parity::{
    AllowEntry, Allowlist, CompilerParity, CompilerRow, GoldenIdentity, IrEntry, IrParity,
    IrStatus, ParityReference, ParityReport, ReferenceRow, RenderVsRender, ReportReference,
    ReportSchema,
};
use slidock_bench_schema::results::{
    ColorScore, Decline, Distribution, FontsOutcome, FrontendSummary, Mean, MotionOutcome,
    OraclePage, Outcome, Reason, Results, ResultsSchema, RowOutcome, RowRecord, Scores, SsimScore,
    Summary, TextScore,
};
use slidock_bench_schema::runs::{RunEntry, RunsIndex, RunsSchema};
use slidock_bench_schema::stamp::{
    CompilerStamp, DepsStamp, FontsStamp, HostStamp, MetricsStamp, ParityStamp, RenderPolicy,
    RendererStamp, Selection, Srgb, Stamp,
};
use slidock_bench_schema::{
    Frontend, Lane, PairKey, PresentationId, RowId, RowStep, Sha256, SlideId, Tier, True,
    to_canonical,
};

fn data_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/data")
}

fn sha(seed: &str) -> Sha256 {
    Sha256::of(seed)
}

/// `Math.round(v * 1e4) / 1e4 + 0`, the results grid.
fn quantise(v: f64) -> f64 {
    let x = v * 10_000.0;
    let f = x.floor();
    (if x - f >= 0.5 { f + 1.0 } else { f }) / 10_000.0 + 0.0
}

/// A deterministic stream of numbers in [0, 1).
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> f64 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 11) as f64 / (1u64 << 53) as f64
    }
}

// The corpus: three made-up presentations.

const BASICS: &str = "demo-basics";
const KEYNOTE: &str = "demo-keynote";
const TABLES: &str = "demo-tables";

fn slides(prefix: &str, first: u32, count: u32) -> Vec<SlideId> {
    (first..first + count)
        .map(|n| SlideId::new(format!("{prefix}{n}")).unwrap())
        .collect()
}

fn source(frontend: Frontend, file: &str, rows: Vec<SlideId>) -> Source {
    Source {
        frontend,
        file: file.into(),
        sha256: Some(sha(file)),
        bytes: Some(10_000 + file.len() as u64 * 977),
        rows: Some(rows),
        row_count: None,
        rows_sha256: None,
        reference: None,
    }
}

fn keynote_exporter() -> KeynoteExporter {
    KeynoteExporter {
        bundle_id: "com.apple.iWork.Keynote".into(),
        version: "14.5".into(),
        build: "7045.0.17".into(),
    }
}

fn page_hashes(seed: &str, pages: usize) -> BTreeMap<String, Vec<Sha256>> {
    BTreeMap::from([(
        sha("raster-key").to_string(),
        (1..=pages)
            .map(|k| sha(&format!("{seed}/page{k}")))
            .collect(),
    )])
}

fn manifest() -> CorpusManifest {
    let features = |names: &[&str]| {
        Some(
            names
                .iter()
                .map(|n| serde_json::from_value(n.to_string().into()).unwrap())
                .collect(),
        )
    };
    CorpusManifest {
        version: Default::default(),
        presentations: vec![
            Presentation {
                id: PresentationId::new(BASICS).unwrap(),
                tier: Tier::Smoke,
                title: Some("Demo basics".into()),
                features: features(&["placeholder", "text.run", "slide.transition"]),
                sources: vec![
                    source(Frontend::Pptx, "Demo Basics.pptx", slides("pp-", 9001, 6)),
                    source(Frontend::Odp, "Demo Basics.odp", slides("od-p", 901, 6)),
                ],
                oracle: Oracle::PdfFile(PdfOracle {
                    file: "Demo Basics.pdf".into(),
                    sha256: Some(sha("Demo Basics.pdf")),
                    exporter: Some(PdfExporter {
                        app: "Google Slides".into(),
                        via: Some("PDF Creator 'Google'".into()),
                        version: None,
                    }),
                    page_hashes: Some(page_hashes(BASICS, 6)),
                }),
                stages: None,
                known_loss: Some(BTreeMap::from([(
                    Frontend::Odp,
                    vec![KnownLoss {
                        kind: LossKind::Source,
                        feature: serde_json::from_value("text.autofit.shrink".into()).unwrap(),
                        note: "The export keeps the autofit flag, not the scale it reached".into(),
                    }],
                )])),
                fonts_expect: Some(FontsExpect {
                    embedded: None,
                    open_families: Some(vec!["Inter".into(), "Roboto".into()]),
                }),
            },
            Presentation {
                id: PresentationId::new(KEYNOTE).unwrap(),
                tier: Tier::Real,
                title: Some("Demo Keynote".into()),
                features: features(&["text.run", "slide.timeline", "slide.transition"]),
                sources: vec![source(
                    Frontend::Keynote,
                    "Demo Keynote.key",
                    slides("kn-", 1001, 8),
                )],
                oracle: Oracle::KeynoteExport(KeynoteOracle {
                    source: "Demo Keynote.key".into(),
                    exporter: keynote_exporter(),
                    sha256: Some(sha("Demo Keynote export")),
                    fonts: Some(vec!["HelveticaNeue".into(), "HelveticaNeue-Bold".into()]),
                    page_hashes: Some(page_hashes(KEYNOTE, 8)),
                }),
                stages: Some(StagesOracle {
                    kind: KeynoteStages::Value,
                    exporter: keynote_exporter(),
                    sha256: Some(sha("Demo Keynote stages")),
                    page_hashes: Some(page_hashes("demo-keynote-stages", 10)),
                    pages_per_row: Some(
                        slides("kn-", 1001, 8)
                            .into_iter()
                            .enumerate()
                            .map(|(i, slide)| (slide, if i == 2 { 3 } else { 1 }))
                            .collect(),
                    ),
                }),
                known_loss: Some(BTreeMap::from([(
                    Frontend::Keynote,
                    vec![KnownLoss {
                        kind: LossKind::Oracle,
                        feature: serde_json::from_value("slide.timeline".into()).unwrap(),
                        note: "Objects with a build-out are absent from the export".into(),
                    }],
                )])),
                fonts_expect: None,
            },
            Presentation {
                id: PresentationId::new(TABLES).unwrap(),
                tier: Tier::Real,
                title: Some("Demo Tables".into()),
                features: features(&["element.table", "text.run"]),
                sources: vec![
                    source(Frontend::Pptx, "Demo Tables.pptx", slides("pp-", 9101, 4)),
                    source(Frontend::Odp, "Demo Tables.odp", slides("od-p", 911, 4)),
                ],
                oracle: Oracle::PdfFile(PdfOracle {
                    file: "Demo Tables.pdf".into(),
                    sha256: Some(sha("Demo Tables.pdf")),
                    exporter: None,
                    page_hashes: Some(page_hashes(TABLES, 4)),
                }),
                stages: None,
                known_loss: None,
                fonts_expect: None,
            },
        ],
    }
}

// Scoring, as classifier@2 decides.

fn scored_class(s: &Scores) -> Outcome {
    let placed = |limit: f64| s.text.oracle_words == 0 || s.text.p95_pm.is_some_and(|p| p <= limit);
    if s.ssim.mean >= 0.97
        && s.ssim.p1 >= 0.6
        && s.color.mean <= 3.0
        && s.text.recall >= 0.98
        && placed(2.8)
    {
        Outcome::Match
    } else if s.ssim.mean >= 0.9 && s.color.mean <= 8.0 && s.text.recall >= 0.9 && placed(11.0) {
        Outcome::Close
    } else {
        Outcome::Mismatch
    }
}

fn scores(rng: &mut Lcg, quality: f64) -> Scores {
    let words = 4 + (rng.next() * 40.0) as u32;
    let recall = (quality + rng.next() * (1.0 - quality)).min(1.0);
    let matched = ((words as f64) * recall).round() as u32;
    let p95 = quantise((1.0 - quality) * 30.0 * rng.next() + 0.3);
    Scores {
        ssim: SsimScore {
            mean: quantise(quality + (1.0 - quality) * rng.next() * 0.8),
            p1: quantise(quality - 0.2 - rng.next() * 0.3),
        },
        color: ColorScore {
            mean: quantise((1.0 - quality) * 40.0 * rng.next() + 0.2),
            p99: quantise((1.0 - quality) * 120.0 * rng.next() + 2.0),
        },
        pixel: quantise(quality + (1.0 - quality) * 0.9),
        text: TextScore {
            oracle_words: words,
            dom_words: words,
            matched,
            recall: quantise(matched as f64 / words as f64),
            precision: quantise(matched as f64 / words as f64),
            median_pm: Some(quantise(p95 * 0.6)),
            p95_pm: Some(p95),
            height_ratio_median: Some(quantise(1.0 + (rng.next() - 0.5) * 0.04)),
        },
    }
}

fn record(id: &RowId, page: u32, content: &Sha256) -> RowRecord {
    RowRecord {
        presentation: id.presentation().clone(),
        slide: id.slide().clone(),
        frontend: id.frontend(),
        oracle: OraclePage { page },
        outcome: Outcome::Mismatch.into(),
        reason: None,
        code: None,
        data: None,
        declined: None,
        scores: None,
        diagnostics: BTreeMap::new(),
        html: None,
        render: None,
        content: content.clone(),
        reference: None,
        known_loss: None,
        stage: None,
        transition_pm: None,
        host: None,
        fonts: None,
        motion: None,
        shadow: None,
        gradient: None,
        shell: None,
    }
}

/// The static row of every expected row; `run` shifts the scores, so runs differ.
fn static_rows(manifest: &CorpusManifest, run: u64) -> BTreeMap<RowId, RowRecord> {
    let content = sha(&format!("compiler-{run}"));
    let mut rng = Lcg(run);
    let mut rows = BTreeMap::new();
    for (i, expected) in manifest
        .expected_rows(&Tier::DEFAULT)
        .into_iter()
        .enumerate()
    {
        let id = expected.id;
        let mut row = record(&id, expected.page, &content);
        row.diagnostics.insert("MID5602".into(), 1 + (i % 3) as u32);
        let quality = match id.frontend() {
            Frontend::Keynote => 0.96,
            Frontend::Pptx => 0.94,
            _ => 0.86,
        } + run as f64 * 0.004;
        match i {
            3 => {
                row.outcome = Outcome::Declined.into();
                row.declined = Some(Decline {
                    code: "PX4001".into(),
                    feature: Some("element.chart".into()),
                    via: None,
                });
                row.diagnostics.insert("PX4001".into(), 1);
            }
            9 => {
                row.outcome = Outcome::Failed.into();
                row.reason = Some(Reason::SlideFailed);
                row.code = Some("IR0901".into());
                row.diagnostics.insert("IR0901".into(), 1);
            }
            14 => {
                row.outcome = Outcome::RenderError.into();
                row.reason = Some(Reason::OracleFontMissing);
                row.html = Some(sha(&format!("{id}/html")));
            }
            _ => {
                let s = scores(&mut rng, quality.min(0.999));
                row.outcome = scored_class(&s).into();
                row.scores = Some(s);
                row.html = Some(sha(&format!("{id}/html")));
                row.render = Some(sha(&format!("{id}/render/{run}")));
            }
        }
        if id.frontend() == Frontend::Odp && i % 2 == 0 {
            row.known_loss = Some(vec![LossKind::Source]);
        }
        rows.insert(id, row);
    }
    rows
}

fn nearest_rank(sorted: &[f64], p: f64) -> f64 {
    let n = sorted.len();
    let idx = ((p * n as f64).ceil() as usize)
        .saturating_sub(1)
        .min(n - 1);
    sorted[idx]
}

fn median(sorted: &[f64]) -> f64 {
    let n = sorted.len();
    if n % 2 == 1 {
        sorted[n / 2]
    } else {
        (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0
    }
}

/// `summarise`: class counts, ssim median and p10 over scored rows, mean text recall.
fn summarise<'a>(rows: impl Iterator<Item = &'a RowRecord>) -> FrontendSummary {
    let mut summary = FrontendSummary::default();
    let mut ssims = Vec::new();
    let mut recalls = Vec::new();
    for row in rows {
        summary.rows += 1;
        let Some(outcome) = row.outcome.classifier() else {
            continue;
        };
        match outcome {
            Outcome::Match => summary.matched += 1,
            Outcome::Close => summary.close += 1,
            Outcome::Mismatch => summary.mismatch += 1,
            Outcome::Declined => summary.declined += 1,
            Outcome::RenderError => summary.render_error += 1,
            Outcome::Failed => summary.failed += 1,
        }
        if let Some(s) = &row.scores {
            ssims.push(s.ssim.mean);
            recalls.push(s.text.recall);
        }
    }
    ssims.sort_by(f64::total_cmp);
    if !ssims.is_empty() {
        summary.ssim = Distribution {
            median: Some(quantise(median(&ssims))),
            p10: Some(quantise(nearest_rank(&ssims, 0.1))),
        };
        summary.text_recall = Mean {
            mean: Some(quantise(recalls.iter().sum::<f64>() / recalls.len() as f64)),
        };
    }
    summary
}

fn summary(rows: &BTreeMap<RowId, RowRecord>) -> Summary {
    let mut by_frontend = BTreeMap::new();
    for frontend in Frontend::ALL {
        let own: Vec<&RowRecord> = rows.values().filter(|r| r.frontend == frontend).collect();
        if !own.is_empty() {
            by_frontend.insert(frontend, summarise(own.into_iter()));
        }
    }
    let mut declines = BTreeMap::new();
    for row in rows.values() {
        if let Some(decline) = &row.declined {
            *declines.entry(decline.code.clone()).or_insert(0) += 1;
        }
    }
    Summary {
        by_frontend,
        reference: BTreeMap::new(),
        declines,
        lanes: None,
    }
}

fn stamp(manifest: &CorpusManifest, run: u64, lane: Lane) -> Stamp {
    let mut corpus = BTreeMap::new();
    for presentation in &manifest.presentations {
        for source in &presentation.sources {
            let (pair, digest) = presentation.corpus_stamp(source.frontend).unwrap().unwrap();
            corpus.insert(pair, digest);
        }
    }
    Stamp {
        compiler: CompilerStamp {
            content: sha(&format!("compiler-{run}")),
            commit: format!("{:040x}", run * 0x1234_5678_9abc),
            dirty: run == 4,
        },
        deps: DepsStamp {
            cargo_lock: sha("cargo-lock-closure"),
            rustc: "1.99.1".into(),
            wasm_pack: "0.13.1".into(),
            node: "24.x".into(),
        },
        host: HostStamp {
            platform: "darwin".into(),
            arch: "arm64".into(),
        },
        renderer: RendererStamp {
            recipe: sha("renderer-recipe"),
            platform_digest:
                "sha256:beddddcf96f19abec078cd2f3da676f40abd460e0e734ad5eed73627ce161d33".into(),
            image_id: format!("sha256:{}", sha("image")),
            arch: "aarch64".into(),
            chromium: "148.0.7778.0".into(),
            cdp_client: "chromiumoxide@0.9.1".into(),
            poppler: "24.02.0".into(),
            raster_long_side_px: 1920,
            metric_long_side_px: 960,
            color_profile: Srgb::Value,
            render_policy: RenderPolicy::default(),
        },
        fonts: FontsStamp {
            font_set_id: sha("font-set"),
            lock_sha256: sha("fonts.lock.json"),
            drift: None,
        },
        metrics: MetricsStamp {
            ssim: "ssim@1".into(),
            color: "color@1".into(),
            pixel: "pixel@1".into(),
            text: "text@2".into(),
            shadow: (lane == Lane::Static).then(|| "shadow@1".into()),
            gradient: (lane == Lane::Static).then(|| "gradient@1".into()),
            motion: (lane == Lane::Motion).then(|| "motion@1".into()),
            fonts: (lane == Lane::Fonts).then(|| "fonts@1".into()),
        },
        classifier: "classifier@2".into(),
        harness: sha("harness"),
        corpus,
        selection: Selection {
            tiers: vec![Tier::Real, Tier::Smoke],
        },
        lane,
        partial: false,
        determinism: (lane == Lane::Static).then(|| sha(&format!("determinism-{run}"))),
        parity: None,
        perf: None,
    }
}

fn static_results(manifest: &CorpusManifest, run: u64) -> Results {
    let rows = static_rows(manifest, run);
    Results {
        schema: ResultsSchema::Value,
        stamp: stamp(manifest, run, Lane::Static),
        summary: summary(&rows),
        rows,
        holdout: None,
        timelines: None,
        font_pairs: None,
        shell_pairs: None,
        motion_pairs: None,
        positions: None,
    }
}

fn host_results(manifest: &CorpusManifest) -> Results {
    let mut rows = static_rows(manifest, 5);
    let mut host = BTreeMap::new();
    for (i, (id, row)) in rows.iter_mut().enumerate() {
        let Some(render) = row.render.clone() else {
            continue;
        };
        let mounted = if i == 7 {
            sha(&format!("{id}/mounted"))
        } else {
            render.clone()
        };
        let status = if mounted == render {
            FrameMatch::Identical
        } else {
            FrameMatch::Differs
        };
        row.host = Some(HostCheck {
            status,
            standalone: render,
            mounted,
            ssim: (status == FrameMatch::Differs).then_some(0.9987),
        });
        let entry: &mut FrameSummary = host.entry(id.frontend()).or_default();
        entry.rows += 1;
        entry.identical += u32::from(status == FrameMatch::Identical);
        entry.differs += u32::from(status == FrameMatch::Differs);
    }
    let mut summary = summary(&rows);
    summary.lanes = Some(LaneSummaries {
        host: Some(host),
        ..LaneSummaries::default()
    });
    Results {
        schema: ResultsSchema::Value,
        stamp: stamp(manifest, 5, Lane::Host),
        summary,
        rows,
        holdout: None,
        timelines: None,
        font_pairs: None,
        shell_pairs: None,
        motion_pairs: None,
        positions: None,
    }
}

/// The glyphs of a family on a row: `exact` from `source`, `substitute` from bundled faces, `fallback` from none.
fn glyphs(exact: u32, source: FontSource, substitute: u32, fallback: u32) -> GlyphCounts {
    let mut sources = BTreeMap::new();
    if exact > 0 {
        sources.insert(source, exact);
    }
    if substitute > 0 {
        *sources.entry(FontSource::Substitute).or_default() += substitute;
    }
    GlyphCounts {
        glyphs: exact + substitute + fallback,
        exact,
        substitute,
        fallback,
        coverage_gaps: fallback,
        sources,
    }
}

fn share(part: u32, whole: u32) -> Option<f64> {
    (whole > 0).then(|| quantise(f64::from(part) / f64::from(whole)))
}

fn fonts_results(manifest: &CorpusManifest) -> Results {
    let mut rows = static_rows(manifest, 5);
    rows.retain(|id, _| id.presentation().as_str() == BASICS);
    let mut fonts: BTreeMap<Frontend, FontsSummary> = BTreeMap::new();
    let mut pair_glyphs: BTreeMap<Frontend, BTreeMap<String, GlyphCounts>> = BTreeMap::new();
    for (i, (id, row)) in rows.iter_mut().enumerate() {
        fonts.entry(id.frontend()).or_default().rows += 1;
        if matches!(
            row.outcome,
            RowOutcome::Classifier(Outcome::Failed | Outcome::RenderError)
        ) {
            continue;
        }
        let complete = i % 4 != 1;
        let families: BTreeMap<String, GlyphCounts> = [
            ("Inter".to_owned(), glyphs(120, FontSource::Google, 0, 0)),
            (
                "Calibri".to_owned(),
                glyphs(0, FontSource::Google, 36, if complete { 0 } else { 4 }),
            ),
        ]
        .into_iter()
        .collect();
        let mut total = GlyphCounts::default();
        for counts in families.values() {
            total.add(counts);
        }
        row.outcome = if complete {
            FontsOutcome::FontsComplete
        } else {
            FontsOutcome::FontsFallback
        }
        .into();
        row.scores = None;
        row.declined = None;
        row.fonts = Some(FontsRowMetrics {
            exact_share: share(total.exact, total.glyphs),
            substitute_share: share(total.substitute, total.glyphs),
            fallback_share: share(total.fallback, total.glyphs),
            fallback_fonts: if complete {
                vec![]
            } else {
                vec!["Last Resort High-Efficiency".into()]
            },
            total: total.clone(),
            families: families.clone(),
            equivalence: None,
        });
        let entry = fonts.entry(id.frontend()).or_default();
        entry.complete += u32::from(complete);
        entry.total.add(&total);
        let pair = pair_glyphs.entry(id.frontend()).or_default();
        for (family, counts) in families {
            pair.entry(family).or_default().add(&counts);
        }
    }
    let face = |family: &str, weight: &str, style: FontStyle, source: FontSource| ShippedFace {
        family: family.into(),
        style,
        weight: weight.into(),
        source,
        postscript_name: format!("{}-{weight}", family.replace(' ', "")),
        original_sha256: (source != FontSource::Embedded)
            .then(|| sha(&format!("{family}-{weight}-original"))),
        woff2_sha256: sha(&format!("{family}-{weight}-woff2")),
        bytes: 9_216,
    };
    let requested = |family: &str, weight: u16, source: Option<FontSource>| RequestedFace {
        family: family.into(),
        style: FontStyle::Normal,
        weight,
        source,
    };
    let report = FontsPairReport {
        faces: vec![
            face("Inter", "100 900", FontStyle::Normal, FontSource::Google),
            face("Roboto", "400", FontStyle::Italic, FontSource::Embedded),
            face("Carlito", "400", FontStyle::Normal, FontSource::Substitute),
        ],
        requested: vec![
            requested("Calibri", 400, Some(FontSource::Substitute)),
            requested("Inter", 400, Some(FontSource::Google)),
            requested("Inter", 700, Some(FontSource::Google)),
            requested("Inter", 800, None),
        ],
        font_bytes: 27_648,
        css_bytes: 37_120,
        families: BTreeMap::new(),
        notices: [("MID5401".to_owned(), 2)].into_iter().collect(),
    };
    let font_pairs: BTreeMap<_, _> = [Frontend::Pptx, Frontend::Odp]
        .into_iter()
        .map(|frontend| {
            let mut report = report.clone();
            report.families = pair_glyphs.remove(&frontend).unwrap_or_default();
            (manifest.presentations[0].pair(frontend), report)
        })
        .collect();
    for (pair, report) in &font_pairs {
        let entry = fonts.get_mut(&pair.frontend()).unwrap();
        entry.faces += report.faces.len() as u32;
        entry.unresolved += report
            .requested
            .iter()
            .filter(|f| f.source.is_none())
            .count() as u32;
        entry.font_bytes += report.font_bytes;
    }
    let mut summary = summary(&rows);
    summary.lanes = Some(LaneSummaries {
        fonts: Some(fonts),
        ..LaneSummaries::default()
    });
    let mut stamp = stamp(manifest, 5, Lane::Fonts);
    stamp
        .corpus
        .retain(|pair, _| pair.presentation().as_str() == BASICS);
    stamp.selection.tiers = vec![Tier::Smoke];
    stamp.metrics.fonts = Some("fonts@1".into());
    Results {
        schema: ResultsSchema::Value,
        stamp,
        summary,
        rows,
        holdout: None,
        timelines: None,
        font_pairs: Some(font_pairs),
        shell_pairs: None,
        motion_pairs: None,
        positions: None,
    }
}

fn frames(id: &RowId) -> FrameStrip {
    FrameStrip {
        start: sha(&format!("{id}/start")),
        mid: sha(&format!("{id}/mid")),
        end: sha(&format!("{id}/end")),
    }
}

fn identical(of: &str) -> FrameCheck {
    FrameCheck {
        status: FrameMatch::Identical,
        expected: sha(of),
        actual: sha(of),
    }
}

/// A Keynote deck whose first build slide is scored against its stages oracle (classifier classes), a slide whose
/// stages no oracle scores (motion classes, one of them a mismatch), and a dissolve into a slide.
fn motion_results(manifest: &CorpusManifest) -> Results {
    let content = sha("compiler-5");
    let keynote = &manifest.presentations[1];
    let ids = keynote.rows(&keynote.sources[0]);
    let built = ids[2].id.clone();
    let transition_slide = ids[3].id.clone();
    let unscored = ids[4].id.clone();
    let mut rng = Lcg(42);
    let mut rows = BTreeMap::new();
    let mut positions = BTreeMap::new();
    for (page, k) in (3..).zip(0..3u32) {
        let id = built.with_step(RowStep::Stage(k));
        let mut row = record(&id, page, &content);
        let s = scores(&mut rng, 0.97);
        row.outcome = scored_class(&s).into();
        row.scores = Some(s);
        row.html = Some(sha(&format!("{id}/html")));
        row.render = Some(sha(&format!("{id}/render")));
        row.stage = Some(k);
        let still = format!("{built}/still/{k}");
        row.motion = Some(MotionRowMetrics {
            frames: Some(frames(&id)),
            end_state: Some(identical(&still)),
            settled: Some(identical(&still)),
            start_state: Some(true),
            reveal: Some(RevealCheck {
                units: 3,
                wrong: Vec::new(),
            }),
            monotonic: (k > 0).then_some(true),
            ..MotionRowMetrics::default()
        });
        positions.insert(id.clone(), sha(&still));
        rows.insert(id, row);
    }
    for (k, outcome) in [
        (0u32, MotionOutcome::MotionMatch),
        (1, MotionOutcome::MotionFallback),
        (2, MotionOutcome::MotionMismatch),
    ] {
        let id = unscored.with_step(RowStep::Stage(k));
        let mut row = record(&id, 5, &content);
        row.outcome = outcome.into();
        row.render = Some(sha(&format!("{id}/end")));
        row.stage = Some(k);
        let still = format!("{unscored}/still/{k}");
        let differs = outcome == MotionOutcome::MotionMismatch;
        row.motion = Some(MotionRowMetrics {
            frames: Some(frames(&id)),
            end_state: Some(if differs {
                FrameCheck {
                    status: FrameMatch::Differs,
                    expected: sha(&still),
                    actual: sha(&format!("{id}/end")),
                }
            } else {
                identical(&still)
            }),
            ssim: differs.then_some(0.9312),
            delta_e: differs.then_some(1.8424),
            settled: Some(identical(&still)),
            start_state: Some(true),
            reveal: Some(RevealCheck {
                units: 2,
                wrong: if differs {
                    vec![RevealMiss {
                        element: "kn-1005.4".into(),
                        paragraph: Some(1),
                        expected: false,
                        actual: Some(true),
                    }]
                } else {
                    Vec::new()
                },
            }),
            monotonic: (k > 0).then_some(!differs),
            failures: if differs {
                vec![
                    "end-state: kn-1005.4 opacity ends at 0.6, settles at 1".into(),
                    "reveal: kn-1005.4 ¶1 shows".into(),
                    "moves: moves: expected motion between its ends, played no frame between its ends differs from both".into(),
                ]
            } else {
                Vec::new()
            },
            ..MotionRowMetrics::default()
        });
        positions.insert(id.clone(), sha(&still));
        rows.insert(id, row);
    }
    let effect = EffectRef {
        id: "keynote:apple:dissolve".into(),
        fallback: "dissolve".into(),
        played: EffectPlay::Native,
    };
    for (pm, outcome) in [
        (250u16, MotionOutcome::MotionMatch),
        (500, MotionOutcome::MotionMatch),
        (750, MotionOutcome::MotionClose),
    ] {
        let id = transition_slide.with_step(RowStep::Transition(pm));
        let mut row = record(&id, 4, &content);
        row.outcome = outcome.into();
        row.render = Some(sha(&format!("{id}/render")));
        row.transition_pm = Some(pm);
        row.motion = Some(MotionRowMetrics {
            effect: Some(effect.clone()),
            frames: Some(FrameStrip {
                start: sha(&format!("{transition_slide}/t0")),
                mid: sha(&format!("{id}/render")),
                end: sha(&format!("{transition_slide}/t1000")),
            }),
            moving: Some(outcome == MotionOutcome::MotionMatch),
            ..MotionRowMetrics::default()
        });
        rows.insert(id, row);
    }
    positions.insert(
        transition_slide.with_step(RowStep::Stage(0)),
        sha(&format!("{transition_slide}/t1000")),
    );
    let step = |index: u32, group: u32, native: &str, effect: Option<&str>, played| StepPlay {
        index,
        group,
        trigger: if index == 0 { "click" } else { "afterPrevious" }.into(),
        kind: "entrance".into(),
        target: format!("kn-1003.{} ¶0–2", index + 3),
        native: native.into(),
        effect: effect.map(str::to_owned),
        params: (index == 0).then(|| r#"{"dir":"u","distance":"offSlide"}"#.to_owned()),
        delivery: Some("byParagraph".into()),
        fallback: "dissolve".into(),
        played,
        expected: StepTiming {
            start_ms: f64::from(index) * 500.0,
            duration_ms: 500.0,
        },
        actual: Some(StepTiming {
            start_ms: f64::from(index) * 500.0,
            duration_ms: 500.0,
        }),
        played_in: Some(group),
        units: 3,
    };
    let timelines = BTreeMap::from([
        (
            built.clone(),
            TimelineFidelity {
                steps: StepCounts {
                    expected: 2,
                    played: 2,
                },
                order: true,
                durations_exact: true,
                delays_exact: true,
                delivery: DeliveryCounts {
                    paragraphs: 3,
                    words: 0,
                    chars: 0,
                },
                effects: BTreeMap::from([
                    (
                        "keynote:apple:move in".into(),
                        EffectCounts {
                            native: 1,
                            fallback: 0,
                            unsupported: 0,
                        },
                    ),
                    (
                        "keynote:apple:sparkle".into(),
                        EffectCounts {
                            native: 0,
                            fallback: 1,
                            unsupported: 0,
                        },
                    ),
                ]),
                plays: vec![
                    step(
                        0,
                        1,
                        "keynote:apple:move in",
                        Some("flyIn"),
                        EffectPlay::Native,
                    ),
                    step(1, 1, "keynote:apple:sparkle", None, EffectPlay::Fallback),
                ],
                ir: Some(true),
                ..TimelineFidelity::default()
            },
        ),
        (
            transition_slide.clone(),
            TimelineFidelity {
                order: true,
                durations_exact: true,
                delays_exact: true,
                transition: Some(TransitionPlay {
                    effect: effect.clone(),
                    name: Some("dissolve".into()),
                    params: None,
                    expected_ms: 700.0,
                    actual_ms: Some(700.0),
                    from: Some(identical(&format!("{transition_slide}/t0"))),
                    to: Some(identical(&format!("{transition_slide}/t1000"))),
                }),
                ir: Some(true),
                ..TimelineFidelity::default()
            },
        ),
        (
            unscored.clone(),
            TimelineFidelity {
                steps: StepCounts {
                    expected: 2,
                    played: 2,
                },
                order: true,
                durations_exact: false,
                delays_exact: true,
                delivery: DeliveryCounts {
                    paragraphs: 2,
                    words: 0,
                    chars: 0,
                },
                plays: vec![step(
                    0,
                    2,
                    "keynote:apple:pop",
                    Some("pop"),
                    EffectPlay::Native,
                )],
                mismatches: vec![TimelineMismatch {
                    step: Some(0),
                    field: TimelineField::Duration,
                    expected: "500".into(),
                    actual: "400".into(),
                }],
                diagnostics: BTreeMap::from([("RT0006".into(), 1)]),
                ..TimelineFidelity::default()
            },
        ),
    ]);
    let pair = PairKey::new(PresentationId::new(KEYNOTE).unwrap(), Frontend::Keynote);
    let motion_pairs = BTreeMap::from([(
        pair,
        MotionPairReport {
            slides: 3,
            steps: 4,
            transitions: 1,
            native: 1,
            fallback_allowed: 1,
            accepted: 1,
            dropped: 0,
            mismatched: 1,
            probe: false,
            items: vec![
                MotionItem {
                    slide: built.slide().clone(),
                    key: None,
                    item: "step 1".into(),
                    native: "keynote:apple:sparkle".into(),
                    expected: None,
                    played: "fallback:dissolve".into(),
                    verdict: MotionVerdict::Declared,
                    why: None,
                },
                MotionItem {
                    slide: built.slide().clone(),
                    key: None,
                    item: "step 0".into(),
                    native: "keynote:apple:move in".into(),
                    expected: Some("flyIn".into()),
                    played: "flyIn".into(),
                    verdict: MotionVerdict::Accepted,
                    why: Some("an exit delivered by character leaves the shape's fill".into()),
                },
                MotionItem {
                    slide: unscored.slide().clone(),
                    key: None,
                    item: "step 0".into(),
                    native: "keynote:apple:pop".into(),
                    expected: Some("pop".into()),
                    played: "pop".into(),
                    verdict: MotionVerdict::Mismatched,
                    why: Some("duration 400 ms, the deck says 500 ms".into()),
                },
            ],
        },
    )]);
    let scored: Vec<&RowRecord> = rows
        .values()
        .filter(|r| r.stage.is_some() && r.outcome.classifier().is_some())
        .collect();
    let motion = MotionSummary {
        stages: summarise(scored.into_iter()),
        stage_checks: MotionClasses {
            rows: 3,
            motion_match: 1,
            motion_close: 0,
            motion_mismatch: 1,
            motion_fallback: 1,
        },
        transitions: MotionClasses {
            rows: 3,
            motion_match: 2,
            motion_close: 1,
            motion_mismatch: 0,
            motion_fallback: 0,
        },
        effects: EffectCounts {
            native: 3,
            fallback: 1,
            unsupported: 0,
        },
        end_state: FrameSummary {
            rows: 6,
            identical: 5,
            differs: 1,
            antialiased: 0,
        },
        timelines: TimelineSummary {
            slides: 3,
            exact: 2,
        },
        items: VerdictCounts {
            native: 1,
            fallback_allowed: 1,
            accepted: 1,
            dropped: 0,
            mismatched: 1,
        },
    };
    let mut summary = Summary {
        by_frontend: BTreeMap::from([(
            Frontend::Keynote,
            summarise(
                rows.values()
                    .filter(|r| r.stage.is_some() && r.outcome.classifier().is_some()),
            ),
        )]),
        ..Summary::default()
    };
    summary.lanes = Some(LaneSummaries {
        motion: Some(BTreeMap::from([(Frontend::Keynote, motion)])),
        ..LaneSummaries::default()
    });
    let mut stamp = stamp(manifest, 5, Lane::Motion);
    stamp
        .corpus
        .retain(|pair, _| pair.presentation().as_str() == KEYNOTE);
    stamp.selection.tiers = vec![Tier::Real];
    Results {
        schema: ResultsSchema::Value,
        stamp,
        summary,
        rows,
        holdout: None,
        timelines: Some(timelines),
        font_pairs: None,
        shell_pairs: None,
        motion_pairs: Some(motion_pairs),
        positions: Some(positions),
    }
}

fn runs(manifest: &CorpusManifest) -> RunsIndex {
    let mut runs: Vec<RunEntry> = (1..=5)
        .map(|run| {
            let results = static_results(manifest, run);
            RunEntry {
                key: results.stamp.run_key().unwrap(),
                stamp: slidock_bench_schema::sha256_json(&results.stamp).unwrap(),
                lane: Lane::Static,
                compiler: results.stamp.compiler.clone(),
                harness: results.stamp.harness.clone(),
                partial: false,
                recorded_at: format!("2026-10-{:02}T09:30:00Z", 20 + run * 2),
                baseline: (run == 3).then_some(True),
                summary: results.summary,
            }
        })
        .collect();
    let motion = motion_results(manifest);
    runs.push(RunEntry {
        key: motion.stamp.run_key().unwrap(),
        stamp: slidock_bench_schema::sha256_json(&motion.stamp).unwrap(),
        lane: Lane::Motion,
        compiler: motion.stamp.compiler.clone(),
        harness: motion.stamp.harness.clone(),
        partial: false,
        recorded_at: "2026-10-30T10:15:00Z".into(),
        baseline: None,
        summary: motion.summary,
    });
    RunsIndex {
        schema: RunsSchema::Value,
        runs,
    }
}

/// A reference from run 1's rows standing in for the golden's, and run 5 compared against it.
fn parity(manifest: &CorpusManifest) -> (ParityReport, Allowlist) {
    let reference_rows = static_rows(manifest, 1);
    let ours = static_rows(manifest, 5);
    let reference = ParityReference {
        schema: Default::default(),
        golden: GoldenIdentity {
            commit: format!("{:040x}", 0x601d),
            manifest: sha("golden-manifest"),
        },
        harness: sha("harness"),
        renderer_recipe: sha("renderer-recipe"),
        font_set_id: sha("font-set"),
        metrics: BTreeMap::from([
            ("color".into(), "color@1".into()),
            ("pixel".into(), "pixel@1".into()),
            ("ssim".into(), "ssim@1".into()),
            ("text".into(), "text@2".into()),
        ]),
        classifier: "classifier@2".into(),
        rows: reference_rows
            .iter()
            .enumerate()
            .map(|(i, (id, row))| {
                // Every third row renders as slidock's does; one of them prints different HTML.
                let render = if i % 3 == 0 {
                    ours[id].render.clone()
                } else {
                    row.render.clone()
                };
                let html = if i == 6 {
                    Some(sha("golden-html"))
                } else {
                    ours[id].html.clone()
                };
                (
                    id.clone(),
                    ReferenceRow {
                        html,
                        render,
                        outcome: row.outcome.classifier().unwrap(),
                        reason: row.reason,
                        code: row.code.clone(),
                        data: row.data.clone(),
                        declined: row.declined.clone(),
                        scores: row.scores,
                        diagnostics: row.diagnostics.clone(),
                        timeline: None,
                        transition: None,
                    },
                )
            })
            .collect(),
    };
    let drifting = reference
        .rows
        .iter()
        .enumerate()
        .find(|(i, (id, row))| i % 3 != 0 && row.render.is_some() && ours[*id].render.is_some())
        .map(|(_, (id, _))| id.clone())
        .unwrap();
    let allowlist = Allowlist(vec![AllowEntry {
        row: drifting.clone(),
        reason: "slidock reads the stored line spacing literally".into(),
        path: "$.slides[0].elements".into(),
        review_by: "2027-01-31".into(),
        status: None,
    }]);
    let mut ours_missing = ours.clone();
    let missing = ours.keys().last().unwrap().clone();
    ours_missing.remove(&missing);
    let rows = reference
        .rows
        .iter()
        .map(|(id, row)| {
            let mut compared = CompilerRow::compare(id, row, ours_missing.get(id), &allowlist);
            if compared.status.needs_review() && compared.outcome.is_some() {
                compared.render_vs_render = Some(RenderVsRender {
                    ssim: 0.9874,
                    delta_e: 1.2031,
                });
                compared.first_ir_diff = Some("$.slides[0].elements[1].frame.y".into());
            }
            (id.clone(), compared)
        })
        .collect();
    let compiler = CompilerParity::build(rows, &ours_missing, false);
    let ir = IrParity {
        containers: BTreeMap::from([
            (
                format!("{BASICS}:pptx:pp-9001"),
                IrEntry {
                    status: IrStatus::Equal,
                    path: None,
                },
            ),
            (
                format!("{BASICS}:odp:od-p902"),
                IrEntry {
                    status: IrStatus::Differs,
                    path: Some("$.elements[1].frame.y".into()),
                },
            ),
        ]),
    };
    let stamp_results = static_results(manifest, 5);
    let report = ParityReport {
        schema: ReportSchema::Value,
        reference: ReportReference {
            sha256: slidock_bench_schema::sha256_json(&reference).unwrap(),
            golden: reference.golden.clone(),
            renderer_recipe: reference.renderer_recipe.clone(),
            font_set_id: reference.font_set_id.clone(),
        },
        run: stamp_results.stamp.run_key().unwrap(),
        compiler: Some(compiler),
        ir: Some(ir),
    };
    (report, allowlist)
}

/// Each file and its canonical text.
fn build() -> BTreeMap<&'static str, String> {
    let manifest = manifest();
    let mut latest = static_results(&manifest, 5);
    let (report, allowlist) = parity(&manifest);
    latest.stamp.parity = Some(ParityStamp {
        golden: report.reference.golden.clone(),
        reference: report.reference.sha256.clone(),
        report: report.sha256().unwrap(),
    });
    BTreeMap::from([
        ("manifest.json", manifest.to_canonical().unwrap()),
        ("results.json", latest.to_canonical().unwrap()),
        (
            "baseline-results.json",
            static_results(&manifest, 3).to_canonical().unwrap(),
        ),
        (
            "host-results.json",
            host_results(&manifest).to_canonical().unwrap(),
        ),
        (
            "fonts-results.json",
            fonts_results(&manifest).to_canonical().unwrap(),
        ),
        (
            "motion-results.json",
            motion_results(&manifest).to_canonical().unwrap(),
        ),
        ("runs.json", runs(&manifest).to_canonical().unwrap()),
        ("parity.json", report.to_canonical().unwrap()),
        ("allow.json", to_canonical(&allowlist).unwrap()),
    ])
}

#[test]
fn the_committed_synthetic_files_are_the_built_ones() {
    let built = build();
    let dir = data_dir();
    if std::env::var("UPDATE_GOLDEN").is_ok() {
        std::fs::create_dir_all(&dir).unwrap();
        for (name, text) in &built {
            std::fs::write(dir.join(name), text).unwrap();
        }
        return;
    }
    for (name, text) in &built {
        let committed = std::fs::read_to_string(dir.join(name))
            .unwrap_or_else(|e| panic!("{name}: {e} (UPDATE_GOLDEN=1 writes it)"));
        assert!(
            committed == *text,
            "{name} drifted: UPDATE_GOLDEN=1 rewrites it"
        );
    }
}

#[test]
fn the_synthetic_files_parse_validate_and_print_back() {
    let dir = data_dir();
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
    let manifest = read("manifest.json");
    assert_eq!(
        CorpusManifest::parse(&manifest)
            .unwrap()
            .to_canonical()
            .unwrap(),
        manifest
    );
    for name in [
        "results.json",
        "baseline-results.json",
        "host-results.json",
        "fonts-results.json",
        "motion-results.json",
    ] {
        let text = read(name);
        let results = Results::parse(&text).unwrap_or_else(|e| panic!("{name}: {e}"));
        assert_eq!(results.to_canonical().unwrap(), text, "{name}");
    }
    let runs = read("runs.json");
    assert_eq!(
        RunsIndex::parse(&runs).unwrap().to_canonical().unwrap(),
        runs
    );
    let parity = read("parity.json");
    assert_eq!(
        ParityReport::parse(&parity)
            .unwrap()
            .to_canonical()
            .unwrap(),
        parity
    );
    let allow = read("allow.json");
    assert_eq!(
        to_canonical(&Allowlist::parse(&allow).unwrap()).unwrap(),
        allow
    );
}

#[test]
fn the_synthetic_files_cover_what_the_ui_shows() {
    let manifest = manifest();
    let latest = static_results(&manifest, 5);
    let outcomes: std::collections::BTreeSet<Outcome> = latest
        .rows
        .values()
        .filter_map(|r| r.outcome.classifier())
        .collect();
    assert_eq!(outcomes.len(), 6, "{outcomes:?}");
    assert!(latest.rows.values().any(|r| r.flagged()));
    let (report, allowlist) = parity(&manifest);
    let compiler = report.compiler.unwrap();
    let statuses: Vec<_> = compiler.counts.keys().copied().collect();
    use slidock_bench_schema::parity::ParityStatus::*;
    for status in [Identical, PixelIdentical, Missing] {
        assert!(statuses.contains(&status), "{status:?} in {statuses:?}");
    }
    assert!(
        statuses
            .iter()
            .any(|s| matches!(s, DriftBetter | DriftSameClass | Regressed))
    );
    assert!(compiler.rows[&allowlist.0[0].row].allowed.is_some());
    let motion = motion_results(&manifest);
    assert!(
        motion
            .rows
            .values()
            .any(|r| matches!(r.outcome, RowOutcome::Motion(_)))
    );
    assert!(
        motion
            .rows
            .keys()
            .any(|id| matches!(id.step(), RowStep::Stage(_)))
    );
    // Made-up names only: every presentation, title and file is a demo one, and every slide id sits in the
    // synthetic ranges (`tests/golden.rs` checks the files against the private manifest).
    for p in &manifest.presentations {
        assert!(p.id.as_str().starts_with("demo-"), "{}", p.id);
        assert!(p.title.as_deref().unwrap().starts_with("Demo "), "{}", p.id);
        for source in &p.sources {
            assert!(source.file.starts_with("Demo "), "{}", source.file);
            for slide in source.rows.as_ref().unwrap() {
                let n: u32 = slide
                    .as_str()
                    .trim_start_matches(|c: char| !c.is_ascii_digit())
                    .parse()
                    .unwrap();
                assert!(n >= 900, "{slide}");
            }
        }
    }
}

#[test]
fn the_runs_index_and_parity_report_name_each_run_by_its_folder_key() {
    let dir = data_dir();
    let read = |name: &str| std::fs::read_to_string(dir.join(name)).unwrap();
    let latest = Results::parse(&read("results.json")).unwrap();
    let baseline = Results::parse(&read("baseline-results.json")).unwrap();
    let runs = RunsIndex::parse(&read("runs.json")).unwrap();
    let report = ParityReport::parse(&read("parity.json")).unwrap();
    assert!(latest.stamp.parity.is_some());
    let key = latest.stamp.run_key().unwrap();
    assert_eq!(report.run, key);
    let keys: Vec<&Sha256> = runs.runs.iter().map(|run| &run.key).collect();
    assert!(keys.contains(&&key));
    assert!(keys.contains(&&baseline.stamp.run_key().unwrap()));
}
