//! Builders for unit tests: synthetic records over the schema crate's synthetic results file.

use std::collections::BTreeMap;

use slidock_bench_schema::results::{
    ColorScore, OraclePage, Outcome, Results, RowRecord, Scores, SsimScore, TextScore,
};
use slidock_bench_schema::{Frontend, PresentationId, Sha256, SlideId};

use crate::results::summary;

/// The synthetic bench's presentation, whose `pptx` pair its stamp covers.
pub const PRESENTATION: &str = "demo-basics";

/// A `demo-basics` pptx row.
pub fn record(slide: &str, outcome: Outcome, ssim: Option<f64>) -> RowRecord {
    RowRecord {
        presentation: PresentationId::new(PRESENTATION).unwrap(),
        slide: SlideId::new(slide).unwrap(),
        frontend: Frontend::Pptx,
        oracle: OraclePage { page: 1 },
        outcome: outcome.into(),
        reason: None,
        code: None,
        data: None,
        declined: (outcome == Outcome::Declined).then(|| slidock_bench_schema::results::Decline {
            code: "PX4001".into(),
            feature: None,
            via: None,
        }),
        scores: ssim.map(|mean| Scores {
            ssim: SsimScore { mean, p1: mean },
            color: ColorScore {
                mean: 1.0,
                p99: 2.0,
            },
            pixel: 0.99,
            text: TextScore {
                oracle_words: 10,
                dom_words: 10,
                matched: 10,
                recall: 1.0,
                precision: 1.0,
                median_pm: Some(0.5),
                p95_pm: Some(1.0),
                height_ratio_median: Some(1.0),
            },
        }),
        diagnostics: BTreeMap::new(),
        html: ssim.map(|_| Sha256::of(slide)),
        render: ssim.map(|_| Sha256::of(format!("{slide}.png"))),
        content: Sha256::of("content"),
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

/// Results holding `rows` under the synthetic bench's stamp.
pub fn results(rows: Vec<RowRecord>) -> Results {
    let mut results = Results::parse(include_str!(
        "../../slidock-bench-schema/tests/data/results.json"
    ))
    .unwrap();
    results.rows = rows.into_iter().map(|row| (row.id(), row)).collect();
    results.summary = summary(&results.rows);
    results
}
