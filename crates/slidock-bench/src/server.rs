//! `slidock-bench ui`: the bench UI's built app and a GET-only JSON
//! and PNG API over the private bench data, on 127.0.0.1 only. A request is answered only when its `Host` is
//! `127.0.0.1:<port>` or `localhost:<port>`; every answer is `nosniff`, `same-origin` and `no-referrer`, with no
//! CORS. Holdout ids get the same 404 as unknown ids, before any file is read.
//!
//! | Path | Answer |
//! |---|---|
//! | `api/status` | [`Status`] |
//! | `api/runs` | [`RunsIndex`]: every run folder of the cache, and the baseline |
//! | `api/runs/<run>/results` | [`Results`] without the holdout (`latest` and `baseline` resolve) |
//! | `api/runs/<run>/rows/<row>` | [`RowDetail`] |
//! | `api/runs/<run>/parity` | [`ParityView`] |
//! | `api/png/<run>/<row>/<kind>` | the row's `oracle`, `render`, `heatmap`, `golden` or `golden-diff` PNG, or a motion row's `start`, `mid`, `end` or `still` |

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use axum::Router;
use axum::extract::{Path as UrlPath, Request, State};
use axum::http::{HeaderValue, Method, StatusCode, header};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use slidock_bench_metrics::{CLOSE, MATCH, decode_png, metric_pair, ssim};
use slidock_bench_schema::api::{
    CloseThresholds, Explain, HeadIds, ImageFrom, ImageKind, ImageRef, LatestRun, MatchThresholds,
    ParityView, ResultsSource, RowCommands, RowDetail, RowRun, RowViewer, RunState, Status,
    Thresholds,
};
use slidock_bench_schema::corpus::{CorpusManifest, Oracle};
use slidock_bench_schema::parity::{Allowlist, ParityReport};
use slidock_bench_schema::results::Results;
use slidock_bench_schema::runs::{RunEntry, RunsIndex, RunsSchema};
use slidock_bench_schema::{Lane, PresentationId, RowId, Sha256, True};

use crate::commands::explain::{EXPLAIN_RUN_FILE, explain_dir};
use crate::compile::read_pair_compile;
use crate::error::{BenchError, Result};
use crate::explain::{encode_png, heatmap};
use crate::hash::sha256_file;
use crate::results::{results_path, row_paths};

/// What the server reads.
pub struct ServerData {
    pub runs: PathBuf,
    pub baseline_dir: PathBuf,
    pub cache: PathBuf,
    pub bench_dir: PathBuf,
    pub corpus_dir: PathBuf,
    /// The built bench UI (`apps/bench-web/dist`).
    pub dist: PathBuf,
    pub manifest: CorpusManifest,
    pub head: HeadIds,
    /// The `parity reference` run folder: the golden output rendered by this bench, when one is of the committed
    /// goldens.
    pub golden_renders: Option<PathBuf>,
    pub port: u16,
    /// Oracle page rasters found by sha256.
    rasters: Mutex<BTreeMap<String, Option<PathBuf>>>,
    /// Parsed results files, by path, while their modification time holds.
    results: Mutex<BTreeMap<PathBuf, (std::time::Duration, Arc<Results>)>>,
}

/// The golden output's render of a row by `parity reference`, where its folder is known.
fn golden_render(data: &ServerData, row: &RowId) -> Option<PathBuf> {
    data.golden_renders
        .as_ref()
        .map(|dir| row_paths(dir, row).render)
}

impl ServerData {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        runs: PathBuf,
        baseline_dir: PathBuf,
        cache: PathBuf,
        bench_dir: PathBuf,
        corpus_dir: PathBuf,
        dist: PathBuf,
        manifest: CorpusManifest,
        head: HeadIds,
        golden_renders: Option<PathBuf>,
        port: u16,
    ) -> ServerData {
        ServerData {
            runs,
            baseline_dir,
            cache,
            bench_dir,
            corpus_dir,
            dist,
            manifest,
            head,
            golden_renders,
            port,
            rasters: Mutex::new(BTreeMap::new()),
            results: Mutex::new(BTreeMap::new()),
        }
    }

    fn holdout(&self) -> BTreeSet<PresentationId> {
        crate::corpus::holdout_ids(&self.manifest)
    }

    /// A results file, parsed once per modification.
    fn read(&self, path: &Path) -> Result<Arc<Results>> {
        let time = std::fs::metadata(path)
            .and_then(|m| m.modified())
            .map_err(BenchError::io(path))?
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        let cached = self
            .results
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(path)
            .filter(|(at, _)| *at == time)
            .map(|(_, results)| results.clone());
        if let Some(results) = cached {
            return Ok(results);
        }
        let results = Arc::new(crate::results::read_results(path)?);
        self.results
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(path.to_path_buf(), (time, results.clone()));
        Ok(results)
    }

    fn baseline_file(&self) -> PathBuf {
        self.baseline_dir.join("results.json")
    }
}

/// RFC 3339 UTC of a time since the epoch, to the second.
pub fn rfc3339(since_epoch: std::time::Duration) -> String {
    let secs = since_epoch.as_secs() as i64;
    let days = secs.div_euclid(86_400);
    let rest = secs.rem_euclid(86_400);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        (rest % 3600) / 60,
        rest % 60
    )
}

fn modified(path: &Path) -> String {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or_else(|| "1970-01-01T00:00:00Z".to_owned(), rfc3339)
}

/// Results with the holdout's rows, pairs and aggregate removed.
fn without_holdout(mut results: Results, holdout: &BTreeSet<PresentationId>) -> Results {
    results
        .rows
        .retain(|id, _| !holdout.contains(id.presentation()));
    results
        .stamp
        .corpus
        .retain(|pair, _| !holdout.contains(pair.presentation()));
    results.holdout = None;
    results
}

/// Every run folder of the cache, and the baseline, oldest first.
pub fn runs_index(data: &ServerData) -> Result<RunsIndex> {
    let baseline = data.read(&data.baseline_file()).ok();
    let baseline_key = baseline.as_ref().and_then(|b| b.stamp.run_key().ok());
    let mut runs = Vec::new();
    let mut entries: Vec<PathBuf> = std::fs::read_dir(&data.runs)
        .map(|dir| dir.filter_map(|e| e.ok()).map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    for dir in entries {
        let is_link = std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_symlink());
        let path = results_path(&dir);
        if is_link || !path.is_file() {
            continue;
        }
        let Ok(results) = data.read(&path) else {
            continue;
        };
        let key = results.stamp.run_key()?;
        if dir.file_name().and_then(|n| n.to_str()) != Some(key.as_str()) {
            continue;
        }
        runs.push(entry(
            &results,
            key.clone(),
            modified(&path),
            baseline_key.as_ref() == Some(&key),
        )?);
    }
    if let (Some(baseline), Some(key)) = (&baseline, &baseline_key)
        && !runs.iter().any(|r: &RunEntry| &r.key == key)
    {
        runs.push(entry(
            baseline,
            key.clone(),
            modified(&data.baseline_file()),
            true,
        )?);
    }
    runs.sort_by(|a, b| {
        a.recorded_at
            .cmp(&b.recorded_at)
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(RunsIndex {
        schema: RunsSchema::Value,
        runs,
    })
}

fn entry(results: &Results, key: Sha256, recorded_at: String, baseline: bool) -> Result<RunEntry> {
    Ok(RunEntry {
        key,
        stamp: slidock_bench_schema::sha256_json(&results.stamp)?,
        lane: results.stamp.lane,
        compiler: results.stamp.compiler.clone(),
        harness: results.stamp.harness.clone(),
        partial: results.stamp.partial,
        recorded_at,
        baseline: (baseline && results.stamp.lane == Lane::Static).then_some(True),
        summary: results.summary.clone(),
    })
}

/// A run reference: `latest`, `baseline`, or a run key, as its key and results file.
fn resolve_run(data: &ServerData, run: &str) -> Option<(Sha256, PathBuf)> {
    let baseline = || data.read(&data.baseline_file()).ok()?.stamp.run_key().ok();
    let key = match run {
        "latest" => {
            let target = std::fs::read_link(data.runs.join("latest")).ok()?;
            Sha256::new(target.to_string_lossy().into_owned()).ok()?
        }
        "baseline" => baseline()?,
        other => Sha256::new(other.to_owned()).ok()?,
    };
    let folder = results_path(&data.runs.join(key.as_str()));
    if folder.is_file() {
        return Some((key, folder));
    }
    (baseline().as_ref() == Some(&key)).then(|| (key, data.baseline_file()))
}

fn status(data: &ServerData) -> Status {
    let latest = std::fs::read_link(data.runs.join("latest"))
        .ok()
        .map(|target| data.runs.join(target));
    let latest_results = latest
        .as_ref()
        .and_then(|dir| data.read(&results_path(dir)).ok());
    let baselines = [
        (Lane::Static, "results.json"),
        (Lane::Motion, "motion-results.json"),
        (Lane::Fonts, "fonts-results.json"),
    ]
    .into_iter()
    .filter(|(_, file)| data.baseline_dir.join(file).is_file())
    .map(|(lane, _)| lane)
    .collect();
    Status {
        head: data.head.clone(),
        baselines,
        latest: latest_results.as_ref().map(|r| LatestRun {
            partial: r.stamp.partial,
        }),
        holdout: false,
        parity: latest.is_some_and(|dir| dir.join("parity.json").is_file()),
        thresholds: Thresholds {
            matched: MatchThresholds {
                ssim_mean: MATCH.ssim_mean,
                ssim_p1: MATCH.ssim_p1,
                color_mean: MATCH.color_mean,
                text_recall: MATCH.text_recall,
                p95_pm: MATCH.p95_pm,
            },
            close: CloseThresholds {
                ssim_mean: CLOSE.ssim_mean,
                color_mean: CLOSE.color_mean,
                text_recall: CLOSE.text_recall,
                p95_pm: CLOSE.p95_pm,
            },
        },
    }
}

/// `encodeURIComponent`.
fn encode(text: &str) -> String {
    let mut out = String::new();
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

fn png_url(run: &Sha256, row: &RowId, kind: &str) -> String {
    format!(
        "api/png/{}/{}/{kind}",
        encode(run.as_str()),
        encode(&row.to_string())
    )
}

/// The oracle page raster of a row: the cached PNG whose sha256 the manifest records for that page.
fn oracle_raster(data: &ServerData, row: &RowId, page: u32) -> Option<PathBuf> {
    let presentation = data
        .manifest
        .presentations
        .iter()
        .find(|p| &p.id == row.presentation())?;
    let hashes = match &presentation.oracle {
        Oracle::PdfFile(o) => o.page_hashes.as_ref(),
        Oracle::KeynoteExport(o) => o.page_hashes.as_ref(),
    }?;
    let index = usize::try_from(page).ok()?.checked_sub(1)?;
    for (key, pages) in hashes {
        let Some(wanted) = pages.get(index) else {
            continue;
        };
        if let Some(found) = data
            .rasters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(wanted.as_str())
        {
            return found.clone();
        }
        let dir = data.cache.join("oracle/raster").join(key);
        let candidates = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok());
        let found = candidates
            .map(|e| e.path().join(format!("{page}.png")))
            .find(|path| {
                path.is_file() && sha256_file(path).ok().as_deref() == Some(wanted.as_str())
            });
        data.rasters
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(wanted.as_str().to_owned(), found.clone());
        if found.is_some() {
            return found;
        }
    }
    None
}

/// The explain folder of a row, when it explained `run`.
fn explain_for(data: &ServerData, row: &RowId) -> Option<(PathBuf, String)> {
    let dir = explain_dir(&data.cache, row);
    let run = std::fs::read_to_string(dir.join(EXPLAIN_RUN_FILE)).ok()?;
    Some((dir, run.trim().to_owned()))
}

fn run_dir_of(data: &ServerData, key: &Sha256) -> Option<PathBuf> {
    let dir = data.runs.join(key.as_str());
    dir.is_dir().then_some(dir)
}

fn row_detail(data: &ServerData, run: &str, row: &RowId) -> Result<Option<RowDetail>> {
    let Some((key, file)) = resolve_run(data, run) else {
        return Ok(None);
    };
    let results = data.read(&file)?;
    let Some(record) = results.rows.get(row) else {
        return Ok(None);
    };
    let baseline_key = data
        .read(&data.baseline_file())
        .ok()
        .and_then(|b| b.stamp.run_key().ok());
    let run_dir = run_dir_of(data, &key);
    let state = match &run_dir {
        None => RunState::Collected,
        Some(dir) => {
            let saved = std::fs::read_to_string(row_paths(dir, row).record)
                .ok()
                .and_then(|text| {
                    serde_json::from_str::<slidock_bench_schema::results::RowRecord>(&text).ok()
                });
            if saved.as_ref() == Some(record) {
                RunState::Present
            } else {
                RunState::Rescored
            }
        }
    };
    let presentation = data
        .manifest
        .presentations
        .iter()
        .find(|p| &p.id == row.presentation());
    let known_loss = presentation
        .and_then(|p| p.known_loss.as_ref())
        .and_then(|k| k.get(&row.frontend()))
        .cloned()
        .unwrap_or_default();
    let diagnostics = run_dir.as_ref().and_then(|dir| {
        let compile = read_pair_compile(dir, row.presentation().as_str(), row.frontend().as_str());
        compile
            .report
            .as_ref()
            .map(|report| crate::runner::applied_diagnostics(Some(report), row.slide().as_str()))
    });
    let explained = explain_for(data, row).filter(|(_, from)| from == key.as_str());
    let render = run_dir
        .as_ref()
        .map(|dir| row_paths(dir, row).render)
        .filter(|p| p.is_file() && state == RunState::Present)
        .map(|_| ImageFrom::Run)
        .or_else(|| {
            explained
                .as_ref()
                .filter(|(dir, _)| dir.join("render.png").is_file())
                .map(|_| ImageFrom::Explain)
        });
    let oracle = oracle_raster(data, row, record.oracle.page).is_some();
    let found = |kind: &str, from: ImageFrom| ImageRef::Found {
        url: png_url(&key, row, kind),
        from,
        run: (from == ImageFrom::Explain).then(|| key.as_str().to_owned()),
    };
    let explain_command = format!("slidock-bench explain {row} --from {}", key.as_str());
    let mut images = BTreeMap::new();
    images.insert(
        ImageKind::Oracle,
        if oracle {
            found("oracle", ImageFrom::OracleCache)
        } else {
            ImageRef::Missing {
                missing: format!("slidock-bench oracle --presentation {}", row.presentation()),
            }
        },
    );
    images.insert(
        ImageKind::Render,
        match render {
            Some(from) => found("render", from),
            None => ImageRef::Missing {
                missing: format!("slidock-bench run --only {row}"),
            },
        },
    );
    images.insert(
        ImageKind::Heatmap,
        match (&explained, render, oracle) {
            (Some((dir, _)), _, _) if dir.join("heatmap.png").is_file() => {
                found("heatmap", ImageFrom::Explain)
            }
            (_, Some(from), true) => found("heatmap", from),
            _ => ImageRef::Missing {
                missing: explain_command.clone(),
            },
        },
    );
    if golden_render(data, row).is_some_and(|path| path.is_file()) {
        images.insert(
            ImageKind::Golden,
            found("golden", ImageFrom::GoldenReference),
        );
        if render.is_some() {
            images.insert(
                ImageKind::GoldenDiff,
                found("golden-diff", ImageFrom::GoldenReference),
            );
        }
    }
    let explain = explain_for(data, row).and_then(|(dir, from)| {
        let report: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("report.json")).ok()?).ok()?;
        let same = serde_json::to_value(record).ok().as_ref() == report.get("record");
        Some(Explain {
            run: Sha256::new(from.clone()).ok(),
            matches_row: same && from == key.as_str(),
            elements: serde_json::from_value(report.get("elements")?.clone()).ok()?,
            missing: serde_json::from_value(report.get("missing")?.clone()).ok()?,
        })
    });
    let source_file = presentation
        .and_then(|p| p.source(row.frontend()))
        .map(|s| s.file.clone())
        .unwrap_or_default();
    Ok(Some(RowDetail {
        id: row.clone(),
        source: if baseline_key.as_ref() == Some(&key) {
            ResultsSource::Baseline
        } else {
            ResultsSource::Latest
        },
        run: RowRun {
            key: Some(key.clone()),
            state,
        },
        record: record.clone(),
        known_loss,
        diagnostics,
        images,
        explain,
        commands: RowCommands {
            explain: explain_command,
            run: format!("slidock-bench run --only {row}"),
        },
        viewer: RowViewer {
            available: data.corpus_dir.is_dir(),
            file: source_file,
        },
    }))
}

fn decode(bytes: &[u8]) -> Option<slidock_bench_metrics::Rgba> {
    decode_png(bytes).ok()
}

/// The heatmap of `b` against `a` (the base), at the metric size.
fn heat(a: &[u8], b: &[u8]) -> Option<Vec<u8>> {
    let (x, y) = metric_pair(&decode(a)?, &decode(b)?).ok()?;
    let map = ssim(&x, &y).ok()?.map;
    encode_png(&heatmap(&x, &map).ok()?).ok()
}

fn row_png(data: &ServerData, run: &str, row: &RowId, kind: &str) -> Option<Vec<u8>> {
    let (key, file) = resolve_run(data, run)?;
    let results = data.read(&file).ok()?;
    let record = results.rows.get(row)?;
    let explained = explain_for(data, row).filter(|(_, from)| from == key.as_str());
    // A motion or shell run keeps its frames by sha256.
    let frame = |sha: Option<&Sha256>| -> Option<Vec<u8>> {
        let sha = sha?;
        let path = run_dir_of(data, &key)?
            .join("frames")
            .join(format!("{sha}.png"));
        let png = std::fs::read(path).ok()?;
        (&Sha256::of(&png) == sha).then_some(png)
    };
    let render = || -> Option<Vec<u8>> {
        run_dir_of(data, &key)
            .and_then(|dir| std::fs::read(row_paths(&dir, row).render).ok())
            .filter(|png| record.render.as_ref() == Some(&Sha256::of(png)))
            .or_else(|| frame(record.render.as_ref()))
            .or_else(|| std::fs::read(explained.as_ref()?.0.join("render.png")).ok())
    };
    let oracle = || std::fs::read(oracle_raster(data, row, record.oracle.page)?).ok();
    let motion = record.motion.as_ref();
    let strip = motion.and_then(|m| m.frames.as_ref());
    let golden = || std::fs::read(golden_render(data, row)?).ok();
    match kind {
        "oracle" => oracle(),
        "render" => render(),
        "heatmap" => explained
            .as_ref()
            .and_then(|(dir, _)| std::fs::read(dir.join("heatmap.png")).ok())
            .or_else(|| heat(&oracle()?, &render()?)),
        "golden" => golden(),
        "golden-diff" => heat(&golden()?, &render()?),
        "start" => frame(strip.map(|f| &f.start)),
        "mid" => frame(strip.map(|f| &f.mid)),
        "end" => frame(strip.map(|f| &f.end)),
        "still" => frame(
            motion
                .and_then(|m| m.end_state.as_ref())
                .map(|c| &c.expected),
        ),
        _ => None,
    }
}

fn parity_view(data: &ServerData, run: &str) -> Option<ParityView> {
    let (key, _) = resolve_run(data, run)?;
    let dir = run_dir_of(data, &key)?;
    let report =
        ParityReport::parse(&std::fs::read_to_string(dir.join("parity.json")).ok()?).ok()?;
    let allowlist =
        Allowlist::parse(&std::fs::read_to_string(data.bench_dir.join("parity/allow.json")).ok()?)
            .ok()
            .unwrap_or_default();
    Some(ParityView {
        report,
        allowlist: allowlist.0,
    })
}

/// The status a report carries.
pub fn status_for_report(data: &ServerData) -> Status {
    status(data)
}

/// A run's results without the holdout.
pub fn public_results(data: &ServerData, run: &str) -> Option<Results> {
    let (_, file) = resolve_run(data, run)?;
    data.read(&file)
        .ok()
        .map(|r| without_holdout((*r).clone(), &data.holdout()))
}

/// A run's parity view.
pub fn public_parity(data: &ServerData, run: &str) -> Option<ParityView> {
    parity_view(data, run)
}

/// A row's detail.
pub fn public_row_detail(data: &ServerData, run: &str, row: &RowId) -> Result<Option<RowDetail>> {
    if data.holdout().contains(row.presentation()) {
        return Ok(None);
    }
    row_detail(data, run, row)
}

/// A row's image.
pub fn public_png(data: &ServerData, run: &str, row: &RowId, kind: &str) -> Option<Vec<u8>> {
    if data.holdout().contains(row.presentation()) {
        return None;
    }
    row_png(data, run, row, kind)
}

type Shared = Arc<ServerData>;

fn not_found() -> Response {
    (
        StatusCode::NOT_FOUND,
        [(header::CONTENT_TYPE, "application/json")],
        "{\"error\":\"not found\"}",
    )
        .into_response()
}

fn json<T: serde::Serialize>(value: &T) -> Response {
    match serde_json::to_vec(value) {
        Ok(body) => ([(header::CONTENT_TYPE, "application/json")], body).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

/// Runs blocking file work off the async threads.
async fn blocking<F: FnOnce() -> Response + Send + 'static>(work: F) -> Response {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|_| StatusCode::INTERNAL_SERVER_ERROR.into_response())
}

/// A row id that parses and names no holdout presentation.
fn public_row(data: &ServerData, row: &str) -> Option<RowId> {
    let id: RowId = row.parse().ok()?;
    (!data.holdout().contains(id.presentation())).then_some(id)
}

async fn api_status(State(data): State<Shared>) -> Response {
    blocking(move || json(&status(&data))).await
}

async fn api_runs(State(data): State<Shared>) -> Response {
    blocking(move || match runs_index(&data) {
        Ok(index) => json(&index),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    })
    .await
}

async fn api_results(State(data): State<Shared>, UrlPath(run): UrlPath<String>) -> Response {
    blocking(move || {
        let Some((_, file)) = resolve_run(&data, &run) else {
            return not_found();
        };
        match data.read(&file) {
            Ok(results) => json(&without_holdout((*results).clone(), &data.holdout())),
            Err(_) => not_found(),
        }
    })
    .await
}

async fn api_row(
    State(data): State<Shared>,
    UrlPath((run, row)): UrlPath<(String, String)>,
) -> Response {
    blocking(move || {
        let Some(row) = public_row(&data, &row) else {
            return not_found();
        };
        match row_detail(&data, &run, &row) {
            Ok(Some(detail)) => json(&detail),
            _ => not_found(),
        }
    })
    .await
}

async fn api_parity(State(data): State<Shared>, UrlPath(run): UrlPath<String>) -> Response {
    blocking(move || {
        let Some(mut view) = parity_view(&data, &run) else {
            return not_found();
        };
        let holdout = data.holdout();
        if let Some(compiler) = view.report.compiler.as_mut() {
            compiler
                .rows
                .retain(|id, _| !holdout.contains(id.presentation()));
        }
        json(&view)
    })
    .await
}

async fn api_png(
    State(data): State<Shared>,
    UrlPath((run, row, kind)): UrlPath<(String, String, String)>,
) -> Response {
    blocking(move || {
        let Some(row) = public_row(&data, &row) else {
            return not_found();
        };
        match row_png(&data, &run, &row, &kind) {
            Some(png) => ([(header::CONTENT_TYPE, "image/png")], png).into_response(),
            None => not_found(),
        }
    })
    .await
}

fn static_type(path: &str) -> &'static str {
    match path.rsplit('.').next().unwrap_or("") {
        "html" => "text/html; charset=utf-8",
        "js" | "mjs" => "text/javascript; charset=utf-8",
        "css" => "text/css; charset=utf-8",
        "svg" => "image/svg+xml",
        "png" => "image/png",
        "json" => "application/json",
        "woff2" => "font/woff2",
        "ico" => "image/x-icon",
        _ => "application/octet-stream",
    }
}

/// The built app's files; `/` is its `index.html`. A path that leaves the folder is a 404.
async fn app_file(State(data): State<Shared>, request: Request) -> Response {
    let path = request.uri().path().trim_start_matches('/').to_owned();
    blocking(move || {
        let relative = if path.is_empty() {
            "index.html".to_owned()
        } else {
            path
        };
        if relative
            .split('/')
            .any(|part| part == ".." || part.is_empty() || part.starts_with('.'))
        {
            return not_found();
        }
        match std::fs::read(data.dist.join(&relative)) {
            Ok(body) => ([(header::CONTENT_TYPE, static_type(&relative))], body).into_response(),
            Err(_) => not_found(),
        }
    })
    .await
}

/// GET and HEAD only, from this server's own host names, with the security headers on every answer.
async fn guard(State(data): State<Shared>, request: Request, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or("");
    let allowed = [
        format!("127.0.0.1:{}", data.port),
        format!("localhost:{}", data.port),
    ];
    let mut response = if !allowed.iter().any(|a| a == host) {
        (StatusCode::MISDIRECTED_REQUEST, "unknown host").into_response()
    } else if request.method() != Method::GET && request.method() != Method::HEAD {
        let mut refused = StatusCode::METHOD_NOT_ALLOWED.into_response();
        refused
            .headers_mut()
            .insert(header::ALLOW, HeaderValue::from_static("GET, HEAD"));
        refused
    } else {
        next.run(request).await
    };
    let headers = response.headers_mut();
    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        "cross-origin-resource-policy",
        HeaderValue::from_static("same-origin"),
    );
    headers.insert(
        header::REFERRER_POLICY,
        HeaderValue::from_static("no-referrer"),
    );
    headers.insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

pub fn router(data: Shared) -> Router {
    Router::new()
        .route("/api/status", get(api_status))
        .route("/api/runs", get(api_runs))
        .route("/api/runs/{run}/results", get(api_results))
        .route("/api/runs/{run}/rows/{row}", get(api_row))
        .route("/api/runs/{run}/parity", get(api_parity))
        .route("/api/png/{run}/{row}/{kind}", get(api_png))
        .route("/api/{*rest}", get(|| async { not_found() }))
        .fallback(app_file)
        .layer(middleware::from_fn_with_state(data.clone(), guard))
        .with_state(data)
}

/// Serves until the process is stopped.
pub async fn serve(data: ServerData) -> Result<()> {
    let port = data.port;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", port))
        .await
        .map_err(|e| BenchError::failed(format!("127.0.0.1:{port}: {e}")))?;
    println!("slidock-bench ui: http://127.0.0.1:{port}/");
    axum::serve(listener, router(Arc::new(data)))
        .await
        .map_err(|e| BenchError::failed(format!("serve: {e}")))
}

/// An empty body, for tests.
#[cfg(test)]
fn empty() -> axum::body::Body {
    axum::body::Body::empty()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The `/api/…` paths of `text` on the lines `keep` selects, with `{name}` written `<name>`: each path from an
    /// `api/` that ends no longer word (`/api/`, `` `api/ ``, `GET api/`, not `openapi/`) to the first character no
    /// route holds.
    fn api_paths(text: &str, keep: impl Fn(&str) -> bool) -> BTreeSet<String> {
        let route_char = |c: char| c.is_ascii_alphanumeric() || "/_-<>{}*?=&".contains(c);
        let word_char = |c: char| c.is_alphanumeric() || "_-.".contains(c);
        let mut paths = BTreeSet::new();
        for line in text.lines().filter(|line| keep(line.trim_start())) {
            for (at, _) in line.match_indices("api/") {
                if line[..at].chars().last().is_some_and(word_char) {
                    continue;
                }
                let path: String = line[at..].chars().take_while(|&c| route_char(c)).collect();
                paths.insert(format!("/{}", path.replace('{', "<").replace('}', ">")));
            }
        }
        paths
    }

    /// The routes `router` serves, but the catch-all 404 of every other `/api/` path.
    fn served_routes() -> BTreeSet<String> {
        let source = include_str!("server.rs");
        let start = source.find("pub fn router(").unwrap();
        let body = &source[start..start + source[start..].find("\n}\n").unwrap()];
        let routes = api_paths(body, |line| line.starts_with(".route(\"/api/"));
        routes
            .into_iter()
            .filter(|route| !route.contains("<*"))
            .collect()
    }

    #[test]
    fn the_docs_name_the_routes_the_router_serves() {
        let served = served_routes();
        assert_eq!(served.len(), 6, "{served:?}");
        // This module's table and bench/README.md's list every route, and only those.
        let table = api_paths(include_str!("server.rs"), |line| {
            line.starts_with("//! | `api/")
        });
        assert_eq!(table, served);
        let readme = api_paths(include_str!("../../../bench/README.md"), |line| {
            line.starts_with("| `/api/")
        });
        assert_eq!(readme, served);
        // The wire types' docs name only served routes (none reads a query string).
        let schema = include_str!("../../slidock-bench-schema/src/api.rs");
        let named = api_paths(schema, |line| {
            line.starts_with("///") || line.starts_with("//!")
        });
        assert!(named.len() >= 4, "{named:?}");
        let stale: Vec<&String> = named.difference(&served).collect();
        assert!(
            stale.is_empty(),
            "api.rs names routes the server does not serve: {stale:?}"
        );
    }

    #[test]
    fn a_stale_route_in_the_docs_is_found() {
        let served = served_routes();
        let named = api_paths(
            "/// `GET /api/rows/<row>?source=`\n/// `GET /api/runs/<run>/parity`: the run's\n\
             /// The manifest, served at GET api/manifest (or http://127.0.0.1:4173/api/holdout).\n\
             let x = \"/api/nope\";",
            |line| line.starts_with("///"),
        );
        assert_eq!(
            named,
            BTreeSet::from([
                "/api/holdout".to_owned(),
                "/api/manifest".to_owned(),
                "/api/rows/<row>?source=".to_owned(),
                "/api/runs/<run>/parity".to_owned()
            ])
        );
        let stale: Vec<&String> = named.difference(&served).collect();
        assert_eq!(
            stale,
            ["/api/holdout", "/api/manifest", "/api/rows/<row>?source="]
        );
        // `api/` inside a longer word or path is not a route.
        assert!(
            api_paths("/// see rapi/x, docs.api/y, my-api/w and openapi/z", |_| {
                true
            })
            .is_empty()
        );
    }

    #[test]
    fn rfc3339_from_the_epoch() {
        use std::time::Duration;
        assert_eq!(rfc3339(Duration::from_secs(0)), "1970-01-01T00:00:00Z");
        assert_eq!(
            rfc3339(Duration::from_secs(951_782_400)),
            "2000-02-29T00:00:00Z"
        );
        assert_eq!(
            rfc3339(Duration::from_secs(1_793_398_245)),
            "2026-10-30T22:10:45Z"
        );
    }

    #[test]
    fn components_encode_as_encode_uri_component() {
        assert_eq!(encode("p:s#t@500:pptx"), "p%3As%23t%40500%3Apptx");
    }

    fn data(dir: &Path) -> Shared {
        let manifest: CorpusManifest = slidock_bench_schema::parse(include_str!(
            "../../slidock-bench-schema/tests/data/manifest.json"
        ))
        .unwrap();
        Arc::new(ServerData::new(
            dir.join("runs"),
            dir.join("baseline"),
            dir.to_path_buf(),
            dir.join("bench"),
            dir.join("corpus"),
            dir.join("dist"),
            manifest,
            HeadIds {
                commit: "0".repeat(40),
                content: Sha256::of("c"),
                harness: Sha256::of("h"),
            },
            Some(dir.join("parity/golden-0123456789ab")),
            5290,
        ))
    }

    async fn get_status(
        router: Router,
        method: Method,
        host: &str,
        path: &str,
    ) -> (StatusCode, Option<String>) {
        use tower::ServiceExt;
        let request = Request::builder()
            .method(method)
            .uri(path)
            .header(header::HOST, host)
            .body(empty())
            .unwrap();
        let response = router.oneshot(request).await.unwrap();
        let nosniff = response
            .headers()
            .get(header::X_CONTENT_TYPE_OPTIONS)
            .map(|v| v.to_str().unwrap().to_owned());
        (response.status(), nosniff)
    }

    #[tokio::test]
    async fn only_gets_from_its_own_host_are_answered() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("dist")).unwrap();
        std::fs::write(dir.path().join("dist/index.html"), "<!doctype html>").unwrap();
        let app = router(data(dir.path()));
        let ok = get_status(app.clone(), Method::GET, "127.0.0.1:5290", "/").await;
        assert_eq!(ok, (StatusCode::OK, Some("nosniff".into())));
        assert_eq!(
            get_status(app.clone(), Method::GET, "evil.test", "/")
                .await
                .0,
            StatusCode::MISDIRECTED_REQUEST
        );
        assert_eq!(
            get_status(app.clone(), Method::POST, "localhost:5290", "/api/status")
                .await
                .0,
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            get_status(app.clone(), Method::GET, "localhost:5290", "/../secret")
                .await
                .0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get_status(app.clone(), Method::GET, "localhost:5290", "/api/status")
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            get_status(
                app,
                Method::GET,
                "localhost:5290",
                "/api/runs/latest/results"
            )
            .await
            .0,
            StatusCode::NOT_FOUND
        );
    }
}
