//! The fonts lane: each deck compiled with its
//! fonts on, every row of its bundled `index.html` mounted by the deck's own player in a renderer that has no font
//! but a last-resort one, and every glyph attributed to the face that drew it.
//!
//! - **Compile.** The native CLI's deck path with its default sources: fonts embedded in the source, then Google
//!   Fonts, then the bundled substitutes. Google is the pinned stand-in of `bench/fonts/fonts-lane.lock.json`: the
//!   CLI's cache (`SLIDOCK_CACHE_DIR`) holds the recorded `css2` responses and their files under the names the CLI
//!   looks up before fetching, so the compiler's own `css2` plan gets the static faces live Google serves, as
//!   `google`. The compile runs with every proxy variable pointing at a closed port, so a request the lock does not
//!   answer fails (a `warning font GET …` line) instead of reaching the network, and the lane reports it. Installed
//!   fonts are off (the CLI's default), so no host font ships.
//! - **Render.** The renderer's image with the `no-fonts` fontconfig (`bench/docker/no-fonts/`), whose /fonts holds
//!   only Unicode's Last Resort face; or, with `--renderer native`, the same Chromium build on the host, where the
//!   host's own fonts are the fallback. Each row is mounted as the host lane mounts it.
//! - **Attribute** ([`attribution`]): per text node, `CSS.getPlatformFontsForNode`.
//!
//! Rows: a corpus pair's frozen rows, and every shown slide of the probe decks `bench/fonts/probes.json` lists. A row
//! that rendered is `fonts-complete` when no glyph was the renderer's, `fonts-fallback` otherwise; a deck that did
//! not bundle fails its rows. A fonts run is its own run folder: `bundles/<p>.<fe>/`, `renders/<row>.png`,
//! `rows/<row>.json` and `results.json`.

pub mod attribution;
pub mod deck;
pub mod lock;
pub mod record;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use serde::Deserialize;
use slidock_bench_metrics::{
    CLASSIFIER_VERSION, COLOR_VERSION, METRIC_LONG_SIDE_PX, PIXEL_VERSION, RASTER_LONG_SIDE_PX,
    SSIM_VERSION, TEXT_VERSION,
};
use slidock_bench_schema::corpus::CorpusManifest;
use slidock_bench_schema::lanes::{
    FontSource, FontStyle, FontsPairReport, FontsRowMetrics, FontsSummary, LaneSummaries,
    RequestedFace, ShippedFace,
};
use slidock_bench_schema::results::{
    FontsOutcome, OraclePage, Outcome, Reason, Results, ResultsSchema, RowOutcome, RowRecord,
};
use slidock_bench_schema::stamp::{
    FontsStamp, MetricsStamp, RenderPolicy, RendererStamp, Selection as StampSelection, Srgb, Stamp,
};
use slidock_bench_schema::{Frontend, Lane, PairKey, PresentationId, RowId, Sha256, SlideId, Tier};
use slidock_fonts::FontOrigin;

use crate::cache::{remove, write_atomic, write_json};
use crate::compile::{COMPILE_TIMEOUT, CompileStatus, compiler_binary, run_compiler_with};
use crate::corpus::{Corpus, Selection, load_manifest, refuse_holdout, select_pairs};
use crate::error::{BenchError, Exit, Result};
use crate::hash::{FileHashes, sha256_file};
use crate::renderer::docker::{PLATFORM_DIGEST, Renderer, Variant, start_variant};
use crate::renderer::native::{NativeBrowser, default_binary};
use crate::renderer::route::{DeckRouter, deck_url};
use crate::renderer::session::{Browser, CDP_CLIENT, MountRequest};
use crate::results::{results_path, row_paths, summary, write_results};
use crate::setup::Context;
use crate::stamp::{compiler_stamp, deps_stamp, harness_hash, host_stamp};

use self::attribution::{LaneFace, attribute};
use self::deck::{BundleFonts, DeckSource, read_bundle};
use self::lock::{LANE_LOCK, LaneFonts, prepare};

/// `fonts@1`.
pub const FONTS_VERSION: &str = "fonts@1";
/// The probe decks the lane renders beside the corpus.
pub const PROBES: &str = "bench/fonts/probes.json";
/// Reviewed rows whose glyphs the renderer's fallback draws, with the reason and where it is tracked.
pub const FALLBACK_ALLOW: &str = "bench/fonts/fallback-allow.json";
/// The renderer variant: `bench/docker/no-fonts/`.
const VARIANT: Variant<'static> = Variant { key: "no-fonts" };

/// Where the lane renders.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RendererChoice {
    /// The renderer image with the `no-fonts` fontconfig, in Docker.
    Container,
    /// The same Chromium build on the host: this binary, or Playwright's install.
    Native(Option<PathBuf>),
}

/// What the fonts lane runs.
pub struct FontsRequest {
    pub tiers: Vec<Tier>,
    pub presentations: Option<Vec<PresentationId>>,
    pub frontends: Option<Vec<Frontend>>,
    pub only: Option<Vec<RowId>>,
    pub concurrency: usize,
    pub renderer: RendererChoice,
}

/// A probe deck of the repository.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probe {
    pub id: PresentationId,
    pub frontend: Frontend,
    /// Relative to the repository.
    pub file: String,
    pub sha256: Sha256,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Probes {
    pub version: u8,
    pub probes: Vec<Probe>,
}

pub fn load_probes(path: &Path) -> Result<Probes> {
    let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
    let probes: Probes = serde_json::from_str(&text)
        .map_err(|error| BenchError::failed(format!("{}: {error}", path.display())))?;
    if probes.version != 1 {
        return Err(BenchError::failed(format!(
            "{}: version is not 1",
            path.display()
        )));
    }
    Ok(probes)
}

/// A reviewed `fonts-fallback` row.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedFallback {
    pub row: RowId,
    /// The renderer-fallback glyphs the review saw: the row must draw exactly as many.
    pub glyphs: u32,
    /// Which characters fall back, and why no shipped face draws them.
    pub reason: String,
    /// Where the fix is tracked.
    pub tracked: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FallbackAllowlist {
    pub version: u8,
    pub rows: Vec<AllowedFallback>,
}

pub fn load_fallback_allowlist(path: &Path) -> Result<FallbackAllowlist> {
    let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
    let list: FallbackAllowlist = serde_json::from_str(&text)
        .map_err(|error| BenchError::failed(format!("{}: {error}", path.display())))?;
    if list.version != 1 {
        return Err(BenchError::failed(format!(
            "{}: version is not 1",
            path.display()
        )));
    }
    Ok(list)
}

/// A run's rows against the allowlist.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FallbackReview {
    /// `fonts-fallback` rows the allowlist does not explain: unlisted, or listed with another glyph count. Each with
    /// its fallback glyphs and the count listed.
    pub unexplained: Vec<(RowId, u32, Option<u32>)>,
    /// Listed rows the run drew with no fallback glyph.
    pub stale: Vec<RowId>,
}

impl FallbackReview {
    pub fn passes(&self) -> bool {
        self.unexplained.is_empty() && self.stale.is_empty()
    }
}

/// Reviews a run's rows: a `fonts-fallback` row is explained only by an entry with exactly its fallback glyphs, and
/// an entry whose row the run drew with no fallback glyph is stale.
pub fn review_fallback(
    rows: &BTreeMap<RowId, RowRecord>,
    allow: &FallbackAllowlist,
) -> FallbackReview {
    let listed = |id: &RowId| allow.rows.iter().find(|entry| &entry.row == id);
    let fallback_glyphs = |row: &RowRecord| {
        (row.outcome == RowOutcome::Fonts(FontsOutcome::FontsFallback))
            .then(|| row.fonts.as_ref().map_or(0, |fonts| fonts.total.fallback))
    };
    let unexplained = rows
        .iter()
        .filter_map(|(id, row)| {
            let glyphs = fallback_glyphs(row)?;
            let entry = listed(id).map(|entry| entry.glyphs);
            (entry != Some(glyphs)).then(|| (id.clone(), glyphs, entry))
        })
        .collect();
    let stale = allow
        .rows
        .iter()
        .filter(|entry| {
            rows.get(&entry.row)
                .is_some_and(|row| row.fonts.is_some() && fallback_glyphs(row).is_none())
        })
        .map(|entry| entry.row.clone())
        .collect();
    FallbackReview { unexplained, stale }
}

/// A row's id, its oracle page, and its metrics and render, or why it has none.
type Checked = (
    RowId,
    u32,
    std::result::Result<(FontsRowMetrics, Vec<u8>), (Reason, String)>,
);

/// A deck the lane compiles, and the rows it renders of it.
struct LanePair {
    pair: PairKey,
    file: PathBuf,
    /// The corpus pair's frozen rows (with their oracle pages); `None` for a probe: every shown slide.
    rows: Option<Vec<(RowId, u32)>>,
    /// The pair's stamp digest.
    digest: Sha256,
}

/// The lane's source of a face the deck data lists.
fn requested_source(source: DeckSource) -> Option<FontSource> {
    match source {
        DeckSource::Embedded => Some(FontSource::Embedded),
        DeckSource::Local => Some(FontSource::Local),
        DeckSource::Google => Some(FontSource::Google),
        DeckSource::User => Some(FontSource::Dropped),
        DeckSource::Bundled => Some(FontSource::Substitute),
        DeckSource::System | DeckSource::Emoji => None,
    }
}

fn origin_source(origin: FontOrigin) -> FontSource {
    match origin {
        FontOrigin::Embedded => FontSource::Embedded,
        FontOrigin::Local => FontSource::Local,
        FontOrigin::Google => FontSource::Google,
        FontOrigin::User => FontSource::Dropped,
        FontOrigin::Bundled => FontSource::Substitute,
    }
}

/// Where every proxy variable points during a lane compile: a closed port, so a request goes nowhere.
const NO_NETWORK_PROXY: &str = "http://127.0.0.1:1";

/// The environment of a lane compile: the CLI's cache, and no network (every proxy variable on a closed port,
/// no host exempt).
pub fn lane_env(cli_cache: &Path) -> BTreeMap<String, String> {
    let mut env = BTreeMap::from([(
        "SLIDOCK_CACHE_DIR".to_owned(),
        cli_cache.to_string_lossy().into_owned(),
    )]);
    for name in ["ALL_PROXY", "HTTPS_PROXY", "HTTP_PROXY"] {
        env.insert(name.to_owned(), NO_NETWORK_PROXY.to_owned());
        env.insert(name.to_lowercase(), NO_NETWORK_PROXY.to_owned());
    }
    for name in ["NO_PROXY", "no_proxy"] {
        env.insert(name.to_owned(), String::new());
    }
    env
}

/// The URLs a compile could not fetch (`warning font GET <url>: …`): requests the lane lock does not answer.
pub fn blocked_fetches(stderr: &str) -> Vec<String> {
    let mut urls: Vec<String> = Vec::new();
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("warning font ") else {
            continue;
        };
        let Some(at) = rest.find("GET https://") else {
            continue;
        };
        let request = rest.get(at + "GET ".len()..).unwrap_or_default();
        let url = request.split(": ").next().unwrap_or_default();
        if !url.is_empty() && !urls.iter().any(|known| known == url) {
            urls.push(url.to_owned());
        }
    }
    urls
}

/// The CLI arguments of a deck compile with its default font sources.
pub fn deck_arguments(file: &Path, frontend: Frontend, out: &Path) -> Vec<String> {
    vec![
        "compile".into(),
        file.to_string_lossy().into_owned(),
        "--frontend".into(),
        frontend.to_string(),
        "--out".into(),
        out.to_string_lossy().into_owned(),
    ]
}

fn style_of(text: &str) -> FontStyle {
    match text {
        "italic" => FontStyle::Italic,
        "oblique" => FontStyle::Oblique,
        _ => FontStyle::Normal,
    }
}

/// The font diagnostics the CLI printed (`warning font MID54xx …`).
pub fn stderr_notices(stderr: &str) -> BTreeMap<String, u32> {
    let mut notices = BTreeMap::new();
    for line in stderr.lines() {
        let Some(rest) = line.strip_prefix("warning font ") else {
            continue;
        };
        let code = rest.split(' ').next().unwrap_or_default();
        let digits = code.strip_prefix("MID").unwrap_or_default();
        if digits.len() == 4 && digits.bytes().all(|b| b.is_ascii_digit()) {
            *notices.entry(code.to_owned()).or_default() += 1;
        }
    }
    notices
}

/// The pair's report and the faces its rows are attributed to.
fn pair_report(
    bundle: &BundleFonts,
    fonts: &LaneFonts,
    stderr: &str,
) -> (FontsPairReport, Vec<LaneFace>) {
    let mut faces = Vec::new();
    let mut lane_faces = Vec::new();
    for face in &bundle.faces {
        let provided = fonts
            .provided
            .iter()
            .find(|file| file.postscript_names.contains(&face.postscript_name));
        let source = match (face.origin, provided) {
            (Some(origin), _) => origin_source(origin),
            (None, Some(file)) => file.source,
            (None, None) => FontSource::Embedded,
        };
        let original = provided
            .filter(|file| file.source == source)
            .and_then(|file| Sha256::new(file.sha256.clone()).ok());
        lane_faces.push(LaneFace {
            family: face.family.clone(),
            postscript_name: face.postscript_name.clone(),
            file_family: face.file_family.clone(),
            source,
            pictographs: face.pictographs.clone(),
        });
        faces.push(ShippedFace {
            family: face.family.clone(),
            style: face.style,
            weight: face.weight.clone(),
            source,
            postscript_name: face.postscript_name.clone(),
            original_sha256: original,
            woff2_sha256: face.sha256.clone(),
            bytes: face.bytes,
        });
    }
    let requested = bundle
        .requested
        .iter()
        .map(|request| RequestedFace {
            family: request.family.clone(),
            style: style_of(&request.style),
            weight: u16::try_from(request.weight).unwrap_or(u16::MAX),
            source: requested_source(request.source),
        })
        .collect();
    let mut notices = bundle.notices.clone();
    for (code, count) in stderr_notices(stderr) {
        *notices.entry(code).or_default() += count;
    }
    let report = FontsPairReport {
        font_bytes: faces.iter().map(|face| face.bytes).sum(),
        css_bytes: bundle.css_bytes,
        faces,
        requested,
        families: BTreeMap::new(),
        notices,
    };
    (report, lane_faces)
}

/// A rendered row's class.
pub fn fonts_class(metrics: &FontsRowMetrics) -> FontsOutcome {
    if metrics.total.fallback == 0 {
        FontsOutcome::FontsComplete
    } else {
        FontsOutcome::FontsFallback
    }
}

/// Per frontend: rows by class, their glyphs, and what the pairs shipped.
pub fn fonts_summary(
    rows: &BTreeMap<RowId, RowRecord>,
    pairs: &BTreeMap<PairKey, FontsPairReport>,
) -> BTreeMap<Frontend, FontsSummary> {
    let mut out: BTreeMap<Frontend, FontsSummary> = BTreeMap::new();
    for row in rows.values() {
        let entry = out.entry(row.frontend).or_default();
        entry.rows += 1;
        if row.outcome == RowOutcome::Fonts(FontsOutcome::FontsComplete) {
            entry.complete += 1;
        }
        if let Some(fonts) = &row.fonts {
            entry.total.add(&fonts.total);
        }
    }
    for (pair, report) in pairs {
        let entry = out.entry(pair.frontend()).or_default();
        entry.faces += report.faces.len() as u32;
        entry.unresolved += report
            .requested
            .iter()
            .filter(|face| face.source.is_none())
            .count() as u32;
        entry.font_bytes += report.font_bytes;
    }
    out
}

/// The lane's renderer while it runs.
pub(crate) enum Running {
    Container(Renderer),
    Native(NativeBrowser),
}

impl Running {
    pub(crate) fn ws_endpoint(&self) -> &str {
        match self {
            Running::Container(renderer) => &renderer.ws_endpoint,
            Running::Native(native) => &native.ws_endpoint,
        }
    }

    pub(crate) async fn stamp(&self, browser: &Browser) -> Result<RendererStamp> {
        let (recipe, platform_digest, image_id, arch, poppler) = match self {
            Running::Container(renderer) => {
                let (arch, poppler) = renderer.facts().await?;
                (
                    renderer.recipe.clone(),
                    PLATFORM_DIGEST.to_owned(),
                    renderer.image_id.clone(),
                    arch,
                    poppler,
                )
            }
            Running::Native(native) => (
                native.recipe.clone(),
                format!("native:{}-{}", std::env::consts::OS, std::env::consts::ARCH),
                "native".to_owned(),
                std::env::consts::ARCH.to_owned(),
                "none".to_owned(),
            ),
        };
        Ok(RendererStamp {
            recipe: Sha256::new(recipe)?,
            platform_digest,
            image_id,
            arch,
            chromium: browser.chromium_version(),
            cdp_client: CDP_CLIENT.to_owned(),
            poppler,
            raster_long_side_px: RASTER_LONG_SIDE_PX,
            metric_long_side_px: METRIC_LONG_SIDE_PX,
            color_profile: Srgb::Value,
            render_policy: RenderPolicy::default(),
        })
    }

    /// Stops the browser: a container of this process's own is removed, the shared one is left running.
    pub(crate) async fn stop(self) -> Result<()> {
        match self {
            Running::Container(renderer) => {
                renderer.release().await?;
            }
            Running::Native(native) => native.stop().await?,
        }
        Ok(())
    }
}

/// The corpus pairs and probe decks a request selects.
fn lane_pairs(
    ctx: &Context,
    manifest: &CorpusManifest,
    request: &FontsRequest,
) -> Result<Vec<LanePair>> {
    let selection = Selection {
        tiers: request.tiers.clone(),
        presentations: request.presentations.clone(),
        frontends: request.frontends.clone(),
    };
    let wanted = |id: &RowId| request.only.as_ref().is_none_or(|only| only.contains(id));
    let mut pairs = Vec::new();
    let pair_selected = |pair: &PairKey| {
        request
            .only
            .as_ref()
            .is_none_or(|only| only.iter().any(|id| &id.pair() == pair))
    };
    let mut corpus: Option<Corpus> = None;
    for (presentation, source) in select_pairs(manifest, &selection) {
        let pair = presentation.pair(source.frontend);
        if !pair_selected(&pair) {
            continue;
        }
        let corpus = match &mut corpus {
            Some(corpus) => corpus,
            None => corpus.insert(Corpus::new(
                ctx.env.require_corpus_dir()?,
                FileHashes::new(&ctx.cache.hashes, true),
                manifest,
            )),
        };
        let file = corpus.source_file(source)?.ok_or_else(|| {
            BenchError::failed(format!("{pair}: {} is not in the corpus", source.file))
        })?;
        let Some((_, digest)) = presentation.corpus_stamp(source.frontend)? else {
            continue;
        };
        let rows = presentation
            .rows(source)
            .into_iter()
            .filter(|row| wanted(&row.id))
            .map(|row| (row.id, row.page))
            .collect();
        pairs.push(LanePair {
            pair,
            file,
            rows: Some(rows),
            digest,
        });
    }
    if request.tiers.contains(&Tier::Probe) {
        for probe in load_probes(&ctx.env.repo_root.join(PROBES))?.probes {
            let pair = PairKey::new(probe.id.clone(), probe.frontend);
            let named = request
                .presentations
                .as_ref()
                .is_none_or(|p| p.contains(&probe.id))
                && request
                    .frontends
                    .as_ref()
                    .is_none_or(|f| f.contains(&probe.frontend));
            if !named || !pair_selected(&pair) {
                continue;
            }
            let file = ctx.env.repo_root.join(&probe.file);
            let actual = sha256_file(&file)?;
            if actual != probe.sha256.as_str() {
                return Err(BenchError::refused(format!(
                    "{}: sha256 {actual}, {PROBES} says {}",
                    probe.file, probe.sha256
                )));
            }
            pairs.push(LanePair {
                pair,
                file,
                rows: None,
                digest: slidock_bench_schema::sha256_json(&(&probe.file, &probe.sha256))?,
            });
        }
    }
    Ok(pairs)
}

/// One row's record, unrendered: the lane fills its class and metrics.
fn record(id: &RowId, page: u32, content: &Sha256) -> RowRecord {
    RowRecord {
        presentation: id.presentation().clone(),
        slide: id.slide().clone(),
        frontend: id.frontend(),
        oracle: OraclePage { page: page.max(1) },
        outcome: Outcome::Failed.into(),
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

/// Runs the fonts lane and writes its run.
pub async fn fonts_lane(ctx: &Context, request: &FontsRequest) -> Result<Exit> {
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    refuse_holdout(
        &manifest,
        &request.tiers,
        request.presentations.as_deref().unwrap_or_default(),
        request.only.as_deref().unwrap_or_default(),
        "run --lane fonts",
    )?;
    let pairs = lane_pairs(ctx, &manifest, request)?;
    if pairs.is_empty() {
        return Err(BenchError::refused("the selection names no deck"));
    }
    let repo = &ctx.env.repo_root;
    let fonts = prepare(&repo.join(LANE_LOCK), &ctx.cache).await?;
    let binary = compiler_binary(repo, &ctx.env.target_dir, ctx.env.compiler.as_deref()).await?;
    let work = fonts.root.join("work");
    let running = match &request.renderer {
        RendererChoice::Container => Running::Container(
            start_variant(
                &ctx.env.bench_dir,
                &fonts.fallback,
                &work,
                &fonts.font_set_id,
                VARIANT,
            )
            .await?,
        ),
        RendererChoice::Native(binary) => {
            let binary = binary.clone().or_else(default_binary).ok_or_else(|| {
                BenchError::refused("no headless shell: set SLIDOCK_BENCH_CHROMIUM")
            })?;
            Running::Native(
                NativeBrowser::start(&ctx.env.bench_dir, &binary, &fonts.root.join("profile"))
                    .await?,
            )
        }
    };
    let outcome = run_lane(ctx, request, &pairs, &fonts, &binary, &running).await;
    running.stop().await?;
    outcome
}

async fn run_lane(
    ctx: &Context,
    request: &FontsRequest,
    pairs: &[LanePair],
    fonts: &LaneFonts,
    binary: &Path,
    running: &Running,
) -> Result<Exit> {
    let browser = Arc::new(Browser::connect(running.ws_endpoint()).await?);
    let repo = &ctx.env.repo_root;
    let mut tiers = request.tiers.clone();
    tiers.sort();
    tiers.dedup();
    let compiler = compiler_stamp(repo).await?;
    let content = compiler.content.clone();
    let stamp = Stamp {
        compiler,
        deps: deps_stamp(repo).await?,
        host: host_stamp(),
        renderer: running.stamp(&browser).await?,
        fonts: FontsStamp {
            font_set_id: Sha256::new(fonts.font_set_id.clone())?,
            lock_sha256: Sha256::new(fonts.lock_sha256.clone())?,
            drift: None,
        },
        metrics: MetricsStamp {
            ssim: SSIM_VERSION.to_owned(),
            color: COLOR_VERSION.to_owned(),
            pixel: PIXEL_VERSION.to_owned(),
            text: TEXT_VERSION.to_owned(),
            shadow: None,
            gradient: None,
            motion: None,
            fonts: Some(FONTS_VERSION.to_owned()),
        },
        classifier: CLASSIFIER_VERSION.to_owned(),
        harness: harness_hash(repo, None).await?,
        corpus: pairs
            .iter()
            .map(|p| (p.pair.clone(), p.digest.clone()))
            .collect(),
        selection: StampSelection { tiers },
        lane: Lane::Fonts,
        partial: request.only.is_some()
            || request.presentations.is_some()
            || request.frontends.is_some(),
        determinism: None,
        parity: None,
        perf: None,
    };
    let dir = ctx.cache.runs.join(stamp.run_key()?.as_str());
    std::fs::create_dir_all(&dir).map_err(BenchError::io(&dir))?;
    let env = lane_env(&fonts.cli_cache);
    let mut records: BTreeMap<RowId, RowRecord> = BTreeMap::new();
    let mut font_pairs: BTreeMap<PairKey, FontsPairReport> = BTreeMap::new();
    let mut failures = 0usize;
    let mut unanswered: Vec<(PairKey, String)> = Vec::new();
    for lane_pair in pairs {
        let pair = &lane_pair.pair;
        let name = format!("{}.{}", pair.presentation(), pair.frontend());
        let bundle_dir = dir.join("bundles").join(&name);
        remove(&bundle_dir)?;
        if let Some(parent) = bundle_dir.parent() {
            std::fs::create_dir_all(parent).map_err(BenchError::io(parent))?;
        }
        let compiled = run_compiler_with(
            binary,
            &deck_arguments(&lane_pair.file, pair.frontend(), &bundle_dir),
            COMPILE_TIMEOUT,
            false,
            env.clone(),
        )
        .await?;
        for url in blocked_fetches(&compiled.stderr) {
            unanswered.push((pair.clone(), url));
        }
        let bundle = match compiled.status {
            CompileStatus::Ok | CompileStatus::Errors => {
                read_bundle(&bundle_dir).map_err(|e| e.to_string())
            }
            other => Err(format!("the deck did not bundle ({other:?})")),
        };
        let reason = match compiled.status {
            CompileStatus::Timeout => Reason::CompileTimeout,
            CompileStatus::Refused => Reason::CompileRefused,
            _ => Reason::CompileCrash,
        };
        let (bundle, report, faces) = match bundle {
            Ok(bundle) => {
                let (report, faces) = pair_report(&bundle, fonts, &compiled.stderr);
                (Some(bundle), Some(report), faces)
            }
            Err(why) => {
                eprintln!("{pair}: {why}");
                (None, None, Vec::new())
            }
        };
        let rows: Vec<(RowId, u32)> = match (&lane_pair.rows, &bundle) {
            (Some(rows), _) => rows.clone(),
            (None, Some(bundle)) => bundle
                .slides
                .iter()
                .filter(|slide| !slide.hidden)
                .zip(1u32..)
                .filter_map(|(slide, page)| {
                    let id = RowId::new(
                        pair.presentation().clone(),
                        SlideId::new(slide.id.clone()).ok()?,
                        pair.frontend(),
                    );
                    request
                        .only
                        .as_ref()
                        .is_none_or(|only| only.contains(&id))
                        .then_some((id, page))
                })
                .collect(),
            (None, None) => Vec::new(),
        };
        eprintln!("{pair}: bundle {:?}, {} rows", compiled.status, rows.len());
        let router = Arc::new(DeckRouter {
            dir: bundle_dir.clone(),
            gif_frames: ctx.cache.gif_frames.clone(),
        });
        let faces = Arc::new(faces);
        let checked: Vec<Checked> = futures::stream::iter(rows.iter())
            .map(|(id, page)| {
                let browser = browser.clone();
                let router = router.clone();
                let faces = faces.clone();
                let bundle = bundle.as_ref();
                async move {
                    let result = match bundle {
                        None => Err((reason, "the deck did not bundle".to_owned())),
                        Some(bundle)
                            if !bundle.slides.iter().any(|s| s.id == id.slide().as_str()) =>
                        {
                            Err((Reason::RowMissing, "the deck has no such slide".to_owned()))
                        }
                        Some(bundle) => {
                            let (w, h) = (bundle.size.w, bundle.size.h);
                            let mount = MountRequest {
                                url: deck_url(),
                                slide: id.slide().as_str().to_owned(),
                                size: (w, h),
                                px_per_pt: f64::from(RASTER_LONG_SIDE_PX) / w.max(h),
                                end_state: false,
                                router,
                            };
                            browser
                                .attribute_mounted(&mount)
                                .await
                                .map(|attributed| {
                                    (attribute(&attributed.nodes, &faces), attributed.png)
                                })
                                .map_err(|error| {
                                    let reason = match error {
                                        crate::renderer::session::RenderError::Timeout => {
                                            Reason::RenderTimeout
                                        }
                                        crate::renderer::session::RenderError::Crash(_) => {
                                            Reason::RenderCrash
                                        }
                                    };
                                    (reason, error.to_string())
                                })
                        }
                    };
                    (id.clone(), *page, result)
                }
            })
            .buffer_unordered(request.concurrency.max(1))
            .collect()
            .await;
        let mut pair_report = report;
        for (id, page, result) in checked {
            let mut row = record(&id, page, &content);
            match result {
                Ok((metrics, png)) => {
                    let paths = row_paths(&dir, &id);
                    write_atomic(&paths.render, &png)?;
                    row.render = Some(Sha256::of(&png));
                    row.outcome = fonts_class(&metrics).into();
                    if let Some(report) = &mut pair_report {
                        for (family, counts) in &metrics.families {
                            report
                                .families
                                .entry(family.clone())
                                .or_default()
                                .add(counts);
                        }
                    }
                    row.fonts = Some(metrics);
                }
                Err((reason, why)) => {
                    eprintln!("{id}: {why}");
                    failures += 1;
                    row.outcome = if reason.is_failed() {
                        Outcome::Failed
                    } else {
                        Outcome::RenderError
                    }
                    .into();
                    row.reason = Some(reason);
                }
            }
            write_json(&row_paths(&dir, &id).record, &row)?;
            records.insert(id, row);
        }
        if let Some(report) = pair_report {
            font_pairs.insert(pair.clone(), report);
        }
    }
    let lanes = fonts_summary(&records, &font_pairs);
    let mut results_summary = summary(&records);
    results_summary.lanes = Some(LaneSummaries {
        fonts: Some(lanes.clone()),
        ..LaneSummaries::default()
    });
    let results = Results {
        schema: ResultsSchema::Value,
        stamp,
        summary: results_summary,
        rows: records,
        holdout: None,
        timelines: None,
        font_pairs: Some(font_pairs),
        shell_pairs: None,
        motion_pairs: None,
        positions: None,
    };
    let path = results_path(&dir);
    write_results(&path, &results)?;
    let allow = load_fallback_allowlist(&repo.join(FALLBACK_ALLOW))?;
    let review = review_fallback(&results.rows, &allow);
    for entry in &allow.rows {
        let explained = results.rows.get(&entry.row).is_some_and(|row| {
            row.outcome == RowOutcome::Fonts(FontsOutcome::FontsFallback)
                && !review.unexplained.iter().any(|(id, ..)| id == &entry.row)
        });
        if explained {
            println!("allowed    {}: {}", entry.row, entry.reason);
        }
    }
    for row in &review.stale {
        println!("stale      {row}: allowlisted, and now fonts-complete; remove its entry");
    }
    for (row, glyphs, listed) in &review.unexplained {
        let fonts = results.rows.get(row).and_then(|r| r.fonts.as_ref());
        let listed = listed.map_or(String::new(), |n| format!(" (allowlisted for {n})"));
        println!(
            "fallback   {row}: {glyphs} glyphs drawn by {}{listed}",
            fonts
                .map(|f| f.fallback_fonts.join(", "))
                .unwrap_or_default()
        );
    }
    for (pair, url) in &unanswered {
        println!(
            "unpinned   {pair}: {url} is not in {LANE_LOCK}; record it with `slidock-bench fonts lane-lock`"
        );
    }
    for (frontend, s) in &lanes {
        let t = &s.total;
        println!(
            "{frontend}: {} rows, {} fonts-complete; {} glyphs: {} exact, {} substitute, {} fallback, {} coverage gaps; \
             {} faces ({} bytes), {} requested faces without a file",
            s.rows,
            s.complete,
            t.glyphs,
            t.exact,
            t.substitute,
            t.fallback,
            t.coverage_gaps,
            s.faces,
            s.font_bytes,
            s.unresolved
        );
    }
    if failures > 0 {
        println!("{failures} rows failed to compile or render");
    }
    println!("results: {}", path.display());
    Ok(Exit::check(
        failures == 0 && review.passes() && unanswered.is_empty(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_font_warnings_are_counted_by_code() {
        let stderr = "font user: Oswald-Regular (x.ttf)\n\
                      warning font MID5410 Oswald Medium 400: the file is Oswald 400\n\
                      warning font MID5410 Roboto Medium 700: …\n\
                      warning font /a/b.ttf: not a font\n\
                      warning font MID5419 Foo: refused\n";
        let notices = stderr_notices(stderr);
        assert_eq!(notices.len(), 2);
        assert_eq!(notices["MID5410"], 2);
        assert_eq!(notices["MID5419"], 1);
    }

    #[test]
    fn each_deck_source_keeps_its_name() {
        assert_eq!(
            requested_source(DeckSource::Google),
            Some(FontSource::Google)
        );
        assert_eq!(
            requested_source(DeckSource::User),
            Some(FontSource::Dropped)
        );
        assert_eq!(
            requested_source(DeckSource::Bundled),
            Some(FontSource::Substitute)
        );
        assert_eq!(requested_source(DeckSource::System), None);
        assert_eq!(origin_source(FontOrigin::Google), FontSource::Google);
        assert_eq!(origin_source(FontOrigin::User), FontSource::Dropped);
        assert_eq!(origin_source(FontOrigin::Bundled), FontSource::Substitute);
    }

    #[test]
    fn a_lane_compile_reaches_no_network() {
        let env = lane_env(Path::new("/c"));
        assert_eq!(env["SLIDOCK_CACHE_DIR"], "/c");
        for name in [
            "ALL_PROXY",
            "all_proxy",
            "HTTPS_PROXY",
            "https_proxy",
            "HTTP_PROXY",
            "http_proxy",
        ] {
            assert_eq!(env[name], NO_NETWORK_PROXY, "{name}");
        }
        // An exempt host in the caller's environment would bypass the proxy.
        assert_eq!(env["NO_PROXY"], "");
        assert_eq!(env["no_proxy"], "");
    }

    #[test]
    fn the_requests_a_compile_could_not_make_are_listed_once() {
        let stderr = "warning font GET https://fonts.googleapis.com/css2?family=Oswald:wght@500&display=block: \
                      io: Connection refused (os error 61)\n\
                      warning font Liberation Sans substitute: GET https://example.org/l.tar.gz: io: refused\n\
                      warning font GET https://fonts.googleapis.com/css2?family=Oswald:wght@500&display=block: x\n\
                      font google: Oswald-Medium (https://fonts.gstatic.com/s/oswald/v1/a.ttf)\n\
                      warning font MID5410 Oswald Medium 400: the file is Oswald 400\n";
        assert_eq!(
            blocked_fetches(stderr),
            [
                "https://fonts.googleapis.com/css2?family=Oswald:wght@500&display=block",
                "https://example.org/l.tar.gz"
            ]
        );
    }

    #[test]
    fn only_reviewed_rows_may_fall_back_and_only_as_reviewed() {
        let rendered = |slide: &str, fallback: u32| {
            let mut row = crate::test_support::record(slide, Outcome::Match, None);
            let mut metrics = attribute(&[], &[]);
            metrics.total.fallback = fallback;
            row.outcome = fonts_class(&metrics).into();
            row.fonts = Some(metrics);
            row
        };
        let mut failed = crate::test_support::record("f", Outcome::Failed, None);
        failed.outcome = Outcome::Failed.into();
        let rows: BTreeMap<RowId, RowRecord> = [
            rendered("a", 0),
            rendered("b", 2),
            rendered("c", 1),
            rendered("d", 400),
            failed,
        ]
        .into_iter()
        .map(|row| (row.id(), row))
        .collect();
        let id = |slide: &str| rendered(slide, 0).id();
        let allow = FallbackAllowlist {
            version: 1,
            rows: [("a", 3), ("b", 2), ("d", 4), ("f", 1)]
                .into_iter()
                .map(|(slide, glyphs)| AllowedFallback {
                    row: id(slide),
                    glyphs,
                    reason: "r".into(),
                    tracked: "t".into(),
                })
                .collect(),
        };
        let review = review_fallback(&rows, &allow);
        // b falls back exactly as reviewed; d was reviewed for 4 glyphs and now draws 400; c is not listed.
        assert_eq!(
            review.unexplained,
            [(id("c"), 1, None), (id("d"), 400, Some(4))]
        );
        // a no longer falls back; f did not render, so it says nothing.
        assert_eq!(review.stale, [id("a")]);
        assert!(!review.passes());
        // A stale entry alone fails the run: the allowlist records exactly what the run draws.
        let stale = FallbackReview {
            unexplained: Vec::new(),
            stale: vec![id("a")],
        };
        assert!(!stale.passes());
        assert!(FallbackReview::default().passes());
    }

    #[test]
    fn the_committed_allowlist_reads() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let allow = load_fallback_allowlist(&repo.join(FALLBACK_ALLOW)).unwrap();
        assert!(
            allow
                .rows
                .iter()
                .all(|entry| !entry.reason.is_empty() && !entry.tracked.is_empty())
        );
    }

    #[test]
    fn the_committed_probes_name_the_probe_decks_by_hash() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let probes = load_probes(&repo.join(PROBES)).unwrap();
        assert!(!probes.probes.is_empty());
        for probe in probes.probes {
            assert_eq!(
                sha256_file(&repo.join(&probe.file)).unwrap(),
                probe.sha256.as_str()
            );
        }
    }
}
