//! The shell lane: the zip a user downloads, unzipped and opened from
//! `file://`, shows what the motion lane scored. Each deck of a motion run is bundled again as a zip (the same
//! arguments, the same pinned fonts), which must hold the `index.html` the motion run played; its file list must hold
//! every file the markup names and no absolute or external URL. Opened from `file://` with no router:
//! - **as a user opens it** (`shell.js` only watches): its own boot script must mount its player under its own
//!   Content Security Policy, the page must report no policy violation and no error, and its first position, once
//!   at rest, must show exactly the motion run's settled frame there (unless that slide shows a video or an animated
//!   GIF, whose frame depends on time);
//! - **walked** (the motion lane's page script, which mounts the deck's player itself with media still): it must make
//!   no request beyond its own files, and show at every position (shown slide, step) the motion run's settled frame.
//!
//! A shell run is its own run folder: `zips/<p>.<fe>.zip`, `frames/<sha256>.png` and `results.json`; the unzipped decks
//! sit under the cache's work folder (the renderer's `/work`).

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use slidock_bench_metrics::{
    CLASSIFIER_VERSION, COLOR_VERSION, PIXEL_VERSION, SSIM_VERSION, TEXT_VERSION,
};
use slidock_bench_schema::lanes::{
    ANTIALIAS_CHANNEL, ANTIALIAS_PIXELS, FrameMatch, FrameSummary, LaneSummaries, ShellBoot,
    ShellCheck, ShellPairCheck,
};
use slidock_bench_schema::results::{Outcome, Results, ResultsSchema, RowRecord};
use slidock_bench_schema::stamp::{MetricsStamp, Selection as StampSelection, Stamp};
use slidock_bench_schema::{Frontend, Lane, PairKey, RowId, RowStep, Sha256, SlideId};

use super::deck::{MotionDeck, parse_deck};
use super::drive::{FrameStore, MOTION_JS, frames_dir};
use super::{
    LanePair, MOTION_VERSION, MotionRequest, RendererChoice, corpus_stamp, fonts_stamp,
    lane_deck_arguments, lane_pairs, pinned_set, start_renderer,
};
use crate::cache::{remove, write_atomic};
use crate::compile::{COMPILE_TIMEOUT, CompileStatus, compiler_binary, run_compiler_with};
use crate::corpus::load_manifest;
use crate::error::{BenchError, Exit, Result};
use crate::fonts_lane::lane_env;
use crate::fonts_lane::lock::{LANE_LOCK, prepare};
use crate::renderer::route::first_frame_png;
use crate::renderer::session::{Browser, MotionPage, MotionPageRequest, RenderError};
use crate::results::{read_results, resolve_results, results_path, write_results};
use crate::setup::Context;
use crate::stamp::{compiler_stamp, deps_stamp, harness_hash, host_stamp};

/// Schemes a deck opened from `file://` may load.
const OWN_SCHEMES: [&str; 4] = ["file:", "data:", "blob:", "about:"];
/// The watch on a deck opened as a user opens it.
pub const SHELL_JS: &str = include_str!("shell.js");
/// How long a booted deck may take to come to rest on its first slide (ms).
const BOOT_SETTLE_MS: u32 = 30_000;

/// Whether slide `id`'s markup shows media whose frame depends on time: a video, or a GIF (which may animate).
pub fn time_media(html: &str, id: &str) -> bool {
    let open = format!("<template data-sd-slide=\"{id}\">");
    let Some(markup) = super::deck::between(html, &open, "</template>") else {
        return false;
    };
    markup.contains("<video") || markup.to_ascii_lowercase().contains(".gif\"")
}

#[derive(serde::Deserialize)]
struct Settled {
    mounted: bool,
    body: bool,
    errors: Vec<String>,
    position: Option<serde_json::Value>,
    chrome: Vec<[f64; 4]>,
    /// The stage sits on whole pixels: a deck whose size is not whole pixels is centred a fraction off, and resampled.
    aligned: bool,
}

/// How many pixels of two same-size PNGs differ by more than `tolerance` in a channel outside `masks` (x, y, width,
/// height, widened to whole pixels and one more each side, for an edge antialiased into its neighbours); `None` when
/// they do not decode to one size.
pub fn strays_outside(a: &[u8], b: &[u8], masks: &[[f64; 4]], tolerance: u8) -> Option<u32> {
    let a = slidock_bench_metrics::decode_png(a).ok()?;
    let b = slidock_bench_metrics::decode_png(b).ok()?;
    if a.width() != b.width() || a.height() != b.height() {
        return None;
    }
    let width = a.width();
    let masked = |x: usize, y: usize| {
        masks.iter().any(|[mx, my, mw, mh]| {
            let (x, y) = (x as f64, y as f64);
            x + 2.0 > mx.floor()
                && x < (mx + mw).ceil() + 1.0
                && y + 2.0 > my.floor()
                && y < (my + mh).ceil() + 1.0
        })
    };
    let count = a
        .data()
        .as_chunks::<4>()
        .0
        .iter()
        .zip(b.data().as_chunks::<4>().0)
        .enumerate()
        .filter(|(i, (p, q))| {
            p.iter()
                .zip(q.iter())
                .any(|(u, v)| u.abs_diff(*v) > tolerance)
                && !masked(i % width, i / width)
        })
        .count();
    Some(u32::try_from(count).unwrap_or(u32::MAX))
}

/// Opens the deck as a user does, lets it boot and come to rest, and checks its first position against `expected`
/// (`None` when that frame depends on time).
async fn boot(
    browser: &Browser,
    store: &FrameStore,
    scored_frames: &FrameStore,
    url: &str,
    size: (f64, f64),
    first: Option<(usize, &Sha256)>,
) -> std::result::Result<ShellBoot, RenderError> {
    let page = browser
        .open_motion_page(&MotionPageRequest {
            url: url.to_owned(),
            size,
            init_script: SHELL_JS.to_owned(),
            router: None,
        })
        .await?;
    let result = async {
        let settled: Settled = serde_json::from_value(
            page.call(&format!("window.__sdShell.settle({BOOT_SETTLE_MS})"))
                .await?,
        )
        .map_err(|e| RenderError::Crash(format!("settle: {e}")))?;
        let mut errors = settled.errors;
        if settled.mounted && !settled.body {
            errors.push("the deck mounted its player elsewhere than the page body".to_owned());
        }
        let at = |key: &str| {
            settled
                .position
                .as_ref()
                .and_then(|p| p.get(key))
                .and_then(serde_json::Value::as_u64)
        };
        let resting =
            first.filter(|(index, _)| at("slide") == Some(*index as u64) && at("step") == Some(0));
        let first = match resting {
            Some((_, expected)) if settled.mounted => {
                let png = page.shot().await?;
                let frame = Sha256::of(&png);
                let path = store.path(&frame);
                if !path.is_file() {
                    write_atomic(&path, &png).map_err(|e| RenderError::Crash(e.to_string()))?;
                }
                let scored = std::fs::read(scored_frames.path(expected)).unwrap_or_default();
                let strays = |tolerance| strays_outside(&png, &scored, &settled.chrome, tolerance);
                let status = if &frame == expected || strays(0) == Some(0) {
                    FrameMatch::Identical
                } else if !settled.aligned
                    && strays(ANTIALIAS_CHANNEL).is_some_and(|n| n <= ANTIALIAS_PIXELS)
                {
                    FrameMatch::Antialiased
                } else {
                    FrameMatch::Differs
                };
                Some(ShellCheck {
                    status,
                    frame,
                    expected: expected.clone(),
                })
            }
            _ => None,
        };
        Ok(ShellBoot {
            mounted: settled.mounted,
            errors,
            first,
            chrome: settled.chrome,
        })
    }
    .await;
    page.close(browser).await;
    result
}

/// The newest motion run, or the one `from` names.
fn motion_run(ctx: &Context, from: &str) -> Result<PathBuf> {
    if from != "latest" {
        return resolve_results(&ctx.cache.runs, &ctx.env.baseline_path(), from);
    }
    let mut newest: Option<(std::time::Duration, PathBuf)> = None;
    let entries = std::fs::read_dir(&ctx.cache.runs).map_err(BenchError::io(&ctx.cache.runs))?;
    for entry in entries.filter_map(|e| e.ok()) {
        let path = results_path(&entry.path());
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if !text.contains("\"lane\": \"motion\"") {
            continue;
        }
        let Some(modified) = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        else {
            continue;
        };
        if newest.as_ref().is_none_or(|(at, _)| modified > *at) {
            newest = Some((modified, path));
        }
    }
    newest.map(|(_, path)| path).ok_or_else(|| {
        BenchError::refused(
            "the bench holds no motion run; run `slidock-bench run --lane motion` first",
        )
    })
}

/// The files `index.html`'s markup names (outside its scripts), and the ones that are absolute or external.
pub fn references(html: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut markup = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(open) = rest.find("<script") {
        markup.push_str(rest.get(..open).unwrap_or_default());
        let after = rest.get(open..).unwrap_or_default();
        match after.find("</script>") {
            Some(close) => rest = after.get(close + "</script>".len()..).unwrap_or_default(),
            None => {
                rest = "";
                break;
            }
        }
    }
    markup.push_str(rest);
    let mut values = Vec::new();
    for attribute in ["src=\"", "poster=\"", "href=\"", "srcset=\""] {
        let mut at = 0;
        while let Some(found) = markup.get(at..).and_then(|m| m.find(attribute)) {
            let start = at + found + attribute.len();
            // A link's target is navigation, not something the deck loads.
            let tag = markup
                .get(..at + found)
                .and_then(|before| before.rfind('<'))
                .and_then(|open| markup.get(open + 1..at + found))
                .and_then(|inside| inside.split_whitespace().next())
                .unwrap_or_default();
            if tag.eq_ignore_ascii_case("a") {
                at = start;
                continue;
            }
            let end = markup
                .get(start..)
                .and_then(|m| m.find('"'))
                .map_or(markup.len(), |e| start + e);
            values.push(
                markup
                    .get(start..end)
                    .unwrap_or_default()
                    .replace("&amp;", "&"),
            );
            at = end;
        }
    }
    let mut at = 0;
    while let Some(found) = markup.get(at..).and_then(|m| m.find("url(")) {
        let start = at + found + "url(".len();
        let end = markup
            .get(start..)
            .and_then(|m| m.find(')'))
            .map_or(markup.len(), |e| start + e);
        let value = markup.get(start..end).unwrap_or_default();
        values.push(
            value
                .trim_matches(|c| c == '"' || c == '\'' || c == ' ')
                .replace("&amp;", "&"),
        );
        at = end;
    }
    let (mut files, mut external) = (BTreeSet::new(), BTreeSet::new());
    for value in values {
        let value = value.trim();
        if value.is_empty()
            || value.starts_with('#')
            || value.starts_with("data:")
            || value.starts_with("blob:")
        {
            continue;
        }
        let lower = value.to_ascii_lowercase();
        if lower.contains(':')
            || value.starts_with('/')
            || value.starts_with("//")
            || value.starts_with('\\')
        {
            external.insert(value.to_owned());
        } else {
            let path = value.split(['?', '#']).next().unwrap_or_default();
            files.insert(path.trim_start_matches("./").to_owned());
        }
    }
    (files, external)
}

/// Unzips a deck under `into`; returns its file names.
fn unzip(zip: &Path, into: &Path) -> Result<Vec<String>> {
    use slidock_io::FileReader;
    use slidock_io::zip::{ZipArchive, ZipLimits};
    let unreadable =
        |error: &dyn std::fmt::Display| BenchError::failed(format!("{}: {error}", zip.display()));
    let reader = FileReader::open(zip).map_err(BenchError::io(zip))?;
    let limits = ZipLimits {
        entry_bytes: 4 << 30,
        memory_bytes: u64::MAX,
        ..ZipLimits::default()
    };
    let archive = ZipArchive::open(reader, limits).map_err(|e| unreadable(&e))?;
    remove(into)?;
    let mut names = Vec::new();
    for entry in archive.entries() {
        let relative = Path::new(&entry.name);
        if entry.name.ends_with('/') {
            continue;
        }
        if !relative
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
        {
            return Err(BenchError::failed(format!(
                "{}: entry {} leaves the deck",
                zip.display(),
                entry.name
            )));
        }
        let data = archive.read(entry).map_err(|e| unreadable(&e))?;
        write_atomic(&into.join(relative), &data)?;
        names.push(entry.name.clone());
    }
    names.sort();
    Ok(names)
}

/// The first frame of each animated GIF of an unzipped deck, by its path, as `data:` PNGs in a JSON object: the
/// static lane's render policy (`renderer::route`), which the page applies to the slides before it plays them, so a
/// frame never depends on when it is taken.
fn gif_stills(dir: &Path, files: &[String]) -> Result<String> {
    use base64::Engine;
    let mut stills = serde_json::Map::new();
    for name in files
        .iter()
        .filter(|name| name.to_ascii_lowercase().ends_with(".gif"))
    {
        let path = dir.join(name);
        let gif = std::fs::read(&path).map_err(BenchError::io(&path))?;
        let png = first_frame_png(&gif)?;
        let data = base64::engine::general_purpose::STANDARD.encode(png);
        stills.insert(name.clone(), format!("data:image/png;base64,{data}").into());
    }
    Ok(serde_json::Value::Object(stills).to_string())
}

/// The URL of an unzipped `index.html` for the renderer: a host path, or its place under the container's `/work`.
fn file_url(index: &Path, choice: &RendererChoice, work: &Path) -> String {
    let path = match choice {
        RendererChoice::Native(_) => index.to_path_buf(),
        RendererChoice::Container => match index.strip_prefix(work) {
            Ok(inside) => Path::new("/work").join(inside),
            Err(_) => index.to_path_buf(),
        },
    };
    let text = path.to_string_lossy().replace(' ', "%20");
    format!("file://{text}")
}

/// A deck booted as a user opens it, its positions and the URLs it requested, or why it could not be walked.
type Walked = std::result::Result<(ShellBoot, Vec<Seen>, Vec<String>), String>;

/// One position's frame in the unzipped deck.
struct Seen {
    id: RowId,
    frame: std::result::Result<Sha256, String>,
}

async fn positions(
    page: &MotionPage,
    store: &FrameStore,
    deck: &MotionDeck,
    pair: &PairKey,
) -> std::result::Result<Vec<Seen>, RenderError> {
    page.call("window.__sdMotion.boot()").await?;
    page.call("window.__sdMotion.pending().then(async (r) => { let n = r; while (n.batch) n = await window.__sdMotion.finish(); return null; })")
        .await?;
    let mut out = Vec::new();
    for index in deck.shown() {
        let Some(slide) = deck.slides.get(index) else {
            continue;
        };
        let Ok(id) = SlideId::new(slide.id.clone()) else {
            continue;
        };
        for step in 0..=slide.clicks() {
            let row = RowId::new(pair.presentation().clone(), id.clone(), pair.frontend())
                .with_step(RowStep::Stage(step));
            page.call(&format!("window.__sdMotion.cut({index}, {step})"))
                .await?;
            let png = page.shot().await?;
            let sha = Sha256::of(&png);
            let path = store.path(&sha);
            if !path.is_file() {
                write_atomic(&path, &png).map_err(|e| RenderError::Crash(e.to_string()))?;
            }
            out.push(Seen {
                id: row,
                frame: Ok(sha),
            });
        }
    }
    Ok(out)
}

/// The pairs whose `corpus` digest ([`corpus_stamp`]) differs from the motion run's: decks compiled from other inputs
/// or fonts than the motion run's, whose frames the shell lane cannot be held to.
fn drawn_otherwise(
    corpus: &BTreeMap<PairKey, Sha256>,
    motion: &BTreeMap<PairKey, Sha256>,
) -> Vec<String> {
    corpus
        .iter()
        .filter(|(pair, digest)| motion.get(*pair) != Some(*digest))
        .map(|(pair, _)| pair.to_string())
        .collect()
}

/// Runs the shell lane over a motion run.
pub async fn shell_lane(ctx: &Context, request: &MotionRequest, from: &str) -> Result<Exit> {
    let motion_path = motion_run(ctx, from)?;
    let motion = read_results(&motion_path)?;
    if motion.stamp.lane != Lane::Motion {
        return Err(BenchError::refused(format!(
            "{} is a {:?} run; the shell lane checks a motion run",
            motion_path.display(),
            motion.stamp.lane
        )));
    }
    let motion_dir = motion_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| BenchError::failed("the motion results have no run folder"))?;
    let scored = motion.positions.clone().unwrap_or_default();
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    let pairs: Vec<LanePair> = lane_pairs(
        ctx,
        &manifest,
        &request.tiers,
        request.presentations.as_ref(),
        request.frontends.as_ref(),
    )?
    .into_iter()
    .filter(|p| motion.stamp.corpus.contains_key(&p.pair))
    .collect();
    if pairs.is_empty() {
        return Err(BenchError::refused(
            "the selection names no deck of the motion run",
        ));
    }
    let repo = &ctx.env.repo_root;
    let fonts = prepare(&repo.join(LANE_LOCK), &ctx.cache).await?;
    let binary = compiler_binary(repo, &ctx.env.target_dir, ctx.env.compiler.as_deref()).await?;
    let running = start_renderer(ctx, &request.renderer, &fonts.root).await?;
    let browser = Browser::connect(running.ws_endpoint()).await;
    let outcome = match browser {
        Ok(browser) => {
            run_shell(
                ctx,
                request,
                &motion,
                &motion_dir,
                &scored,
                &pairs,
                &fonts,
                &binary,
                &running,
                &browser,
                &ctx.cache.oracle,
            )
            .await
        }
        Err(error) => Err(error),
    };
    running.stop().await?;
    outcome
}

#[allow(clippy::too_many_arguments)]
async fn run_shell(
    ctx: &Context,
    request: &MotionRequest,
    motion: &Results,
    motion_dir: &Path,
    scored: &BTreeMap<RowId, Sha256>,
    pairs: &[LanePair],
    fonts: &crate::fonts_lane::lock::LaneFonts,
    binary: &Path,
    running: &crate::fonts_lane::Running,
    browser: &Browser,
    work: &Path,
) -> Result<Exit> {
    let repo = &ctx.env.repo_root;
    let compiler = compiler_stamp(repo).await?;
    let content = compiler.content.clone();
    let pinned = pinned_set(ctx);
    let corpus = corpus_stamp(pairs, pinned.as_ref())?;
    let differ = drawn_otherwise(&corpus, &motion.stamp.corpus);
    if !differ.is_empty() {
        return Err(BenchError::refused(format!(
            "{}: the motion run compiled it from other inputs or fonts (its corpus digest differs, as after a \
             `fonts sync`); rerun the motion lane",
            differ.join(", ")
        )));
    }
    let stamp = Stamp {
        compiler,
        deps: deps_stamp(repo).await?,
        host: host_stamp(),
        renderer: running.stamp(browser).await?,
        fonts: fonts_stamp(fonts)?,
        metrics: MetricsStamp {
            ssim: SSIM_VERSION.to_owned(),
            color: COLOR_VERSION.to_owned(),
            pixel: PIXEL_VERSION.to_owned(),
            text: TEXT_VERSION.to_owned(),
            shadow: None,
            gradient: None,
            motion: Some(MOTION_VERSION.to_owned()),
            fonts: None,
        },
        classifier: CLASSIFIER_VERSION.to_owned(),
        harness: harness_hash(repo, None).await?,
        corpus,
        selection: StampSelection {
            tiers: motion.stamp.selection.tiers.clone(),
        },
        lane: Lane::Shell,
        partial: request.presentations.is_some() || request.frontends.is_some(),
        determinism: None,
        parity: None,
        perf: None,
    };
    let key = stamp.run_key()?;
    let dir = ctx.cache.runs.join(key.as_str());
    std::fs::create_dir_all(&dir).map_err(BenchError::io(&dir))?;
    let store = Arc::new(FrameStore {
        dir: frames_dir(&dir),
    });
    let scored_store = FrameStore {
        dir: frames_dir(motion_dir),
    };
    let scored_frames = &scored_store;
    let env = lane_env(&fonts.cli_cache);
    let mut records: BTreeMap<RowId, RowRecord> = BTreeMap::new();
    let mut shell_pairs: BTreeMap<PairKey, ShellPairCheck> = BTreeMap::new();
    let mut failures: Vec<String> = Vec::new();
    // Per deck: its pair, its deck, its URL, its GIF stills, and its first position with the frame scored there
    // (`None` when that frame depends on time).
    #[allow(clippy::type_complexity)]
    let mut opened: Vec<(
        PairKey,
        MotionDeck,
        String,
        String,
        Option<(usize, Sha256)>,
        bool,
    )> = Vec::new();
    for lane_pair in pairs {
        let pair = &lane_pair.pair;
        let name = format!("{}.{}", pair.presentation(), pair.frontend());
        let zip = dir.join("zips").join(format!("{name}.zip"));
        if let Some(parent) = zip.parent() {
            std::fs::create_dir_all(parent).map_err(BenchError::io(parent))?;
        }
        let compiled = run_compiler_with(
            binary,
            &lane_deck_arguments(
                lane_pair,
                pinned.as_ref().map(|set| set.dir.as_path()),
                &zip,
            ),
            COMPILE_TIMEOUT,
            true,
            env.clone(),
        )
        .await?;
        if !matches!(compiled.status, CompileStatus::Ok | CompileStatus::Errors) {
            failures.push(format!(
                "{pair}: the zip did not bundle ({:?})",
                compiled.status
            ));
            continue;
        }
        let unzipped = work.join("shell").join(key.as_str()).join(&name);
        let files = unzip(&zip, &unzipped)?;
        let index = unzipped.join("index.html");
        let html_bytes = std::fs::read(&index).map_err(BenchError::io(&index))?;
        let html = String::from_utf8_lossy(&html_bytes);
        let scored_index =
            std::fs::read(motion_dir.join("bundles").join(&name).join("index.html")).ok();
        let same_deck = scored_index.as_deref() == Some(html_bytes.as_slice());
        let (named, external) = references(&html);
        let present: BTreeSet<&str> = files.iter().map(String::as_str).collect();
        let missing: Vec<String> = named
            .iter()
            .filter(|f| !present.contains(f.as_str()))
            .cloned()
            .collect();
        let (size, slides) = match parse_deck(&html) {
            Ok(deck) => deck,
            Err(why) => {
                failures.push(format!("{pair}: {why}"));
                continue;
            }
        };
        shell_pairs.insert(
            pair.clone(),
            ShellPairCheck {
                files: files.len() as u32,
                missing,
                external: external.into_iter().collect(),
                requests: Vec::new(),
                deck: if same_deck {
                    FrameMatch::Identical
                } else {
                    FrameMatch::Differs
                },
                boot: ShellBoot {
                    mounted: false,
                    errors: vec!["not opened".to_owned()],
                    first: None,
                    chrome: Vec::new(),
                },
            },
        );
        let deck = MotionDeck {
            size,
            slides,
            index: Sha256::of(&html_bytes),
        };
        // The first position the booted deck rests on, and the frame the motion run scored there.
        let first_shown = deck.shown().first().copied();
        let first = first_shown.and_then(|index| {
            let slide = deck.slides.get(index)?;
            let id = SlideId::new(slide.id.clone()).ok()?;
            let row = RowId::new(pair.presentation().clone(), id, pair.frontend())
                .with_step(RowStep::Stage(0));
            Some((index, scored.get(&row)?.clone()))
        });
        let timed = first_shown
            .and_then(|index| deck.slides.get(index))
            .is_some_and(|slide| time_media(&html, &slide.id));
        opened.push((
            pair.clone(),
            deck,
            file_url(&index, &request.renderer, work),
            gif_stills(&unzipped, &files)?,
            first,
            timed,
        ));
    }
    let seen: Vec<(PairKey, Walked)> = futures::stream::iter(opened.iter())
        .map(|(pair, deck, url, stills, first, timed)| {
            let store = store.clone();
            async move {
                let size = (deck.size.w * 4.0 / 3.0, deck.size.h * 4.0 / 3.0);
                let expected = first
                    .as_ref()
                    .filter(|_| !timed)
                    .map(|(index, sha)| (*index, sha));
                let booted = match boot(browser, &store, scored_frames, url, size, expected).await {
                    Ok(booted) => booted,
                    Err(error) => return (pair.clone(), Err(format!("booting it: {error}"))),
                };
                let page = browser
                    .open_motion_page(&MotionPageRequest {
                        url: url.clone(),
                        size,
                        init_script: format!("window.__sdMotionStills = {stills};\n{MOTION_JS}"),
                        router: None,
                    })
                    .await;
                let result = match page {
                    Ok(page) => {
                        let walked = positions(&page, &store, deck, pair).await;
                        let requests = page.requests();
                        page.close(browser).await;
                        walked
                            .map(|seen| (booted, seen, requests))
                            .map_err(|e| e.to_string())
                    }
                    Err(error) => Err(error.to_string()),
                };
                (pair.clone(), result)
            }
        })
        .buffer_unordered(request.concurrency.max(1))
        .collect()
        .await;
    for (pair, result) in seen {
        match result {
            Err(why) => failures.push(format!("{pair}: {why}")),
            Ok((booted, positions, requests)) => {
                if let Some(check) = shell_pairs.get_mut(&pair) {
                    check.boot = booted;
                    let mut outside: Vec<String> = requests
                        .into_iter()
                        .filter(|url| !OWN_SCHEMES.iter().any(|scheme| url.starts_with(scheme)))
                        .collect();
                    outside.sort();
                    outside.dedup();
                    check.requests = outside;
                }
                for seen in positions {
                    let mut row = super::record(&seen.id, 1, &content);
                    row.stage = match seen.id.step() {
                        RowStep::Stage(k) => Some(k),
                        _ => None,
                    };
                    match (seen.frame, scored.get(&seen.id)) {
                        (Ok(frame), Some(expected)) => {
                            let identical = &frame == expected;
                            row.outcome = if identical {
                                Outcome::Match
                            } else {
                                Outcome::Mismatch
                            }
                            .into();
                            row.render = Some(frame.clone());
                            row.shell = Some(ShellCheck {
                                status: if identical {
                                    FrameMatch::Identical
                                } else {
                                    FrameMatch::Differs
                                },
                                frame,
                                expected: expected.clone(),
                            });
                        }
                        (Ok(_), None) => {
                            failures
                                .push(format!("{}: the motion run scored no frame there", seen.id));
                            continue;
                        }
                        (Err(why), _) => {
                            failures.push(format!("{}: {why}", seen.id));
                            continue;
                        }
                    }
                    records.insert(seen.id.clone(), row);
                }
            }
        }
    }
    let mut lanes: BTreeMap<Frontend, FrameSummary> = BTreeMap::new();
    for row in records.values() {
        let Some(check) = &row.shell else { continue };
        let entry = lanes.entry(row.frontend).or_default();
        entry.rows += 1;
        match check.status {
            FrameMatch::Identical => entry.identical += 1,
            FrameMatch::Antialiased => entry.antialiased += 1,
            FrameMatch::Differs => entry.differs += 1,
        }
    }
    let mut results_summary = crate::results::summary(&records);
    results_summary.lanes = Some(LaneSummaries {
        shell: Some(lanes.clone()),
        ..LaneSummaries::default()
    });
    let results = Results {
        schema: ResultsSchema::Value,
        stamp,
        summary: results_summary,
        rows: records,
        holdout: None,
        timelines: None,
        font_pairs: None,
        shell_pairs: Some(shell_pairs),
        motion_pairs: None,
        positions: None,
    };
    let path = results_path(&dir);
    write_results(&path, &results)?;
    let mut ok = failures.is_empty();
    for (pair, check) in results.shell_pairs.iter().flatten() {
        let frames: Vec<&RowRecord> = results
            .rows
            .values()
            .filter(|r| &r.id().pair() == pair)
            .collect();
        let identical = frames
            .iter()
            .filter(|r| {
                r.shell
                    .as_ref()
                    .is_some_and(|s| s.status == FrameMatch::Identical)
            })
            .count();
        println!(
            "{pair}: {} files; booted by its own script: {}{}; positions {identical}/{} identical to the motion run; deck {}; {} missing, {} external, {} requests",
            check.files,
            if check.boot.mounted { "yes" } else { "NO" },
            match &check.boot.first {
                Some(first) if first.status == FrameMatch::Identical =>
                    ", its first position the one scored",
                Some(first) if first.status == FrameMatch::Antialiased =>
                    ", its first position the one scored, resampled a fraction of a pixel off",
                Some(_) => ", its first position DIFFERS from the one scored",
                None if check.boot.mounted =>
                    ", its first position not compared (media that moves)",
                None => "",
            },
            frames.len(),
            if check.deck == FrameMatch::Identical {
                "the one scored"
            } else {
                "DIFFERS from the one scored"
            },
            check.missing.len(),
            check.external.len(),
            check.requests.len()
        );
        for file in &check.missing {
            println!("missing    {pair}: {file}");
        }
        for url in &check.external {
            println!("external   {pair}: {url}");
        }
        for url in &check.requests {
            println!("request    {pair}: {url}");
        }
        for error in &check.boot.errors {
            println!("boot       {pair}: {error}");
        }
        for row in frames.iter().filter(|r| {
            r.shell
                .as_ref()
                .is_some_and(|s| s.status == FrameMatch::Differs)
        }) {
            println!("differs    {}", row.id());
        }
        ok &= identical == frames.len()
            && check.boot.passes()
            && check.deck == FrameMatch::Identical
            && check.missing.is_empty()
            && check.external.is_empty()
            && check.requests.is_empty();
    }
    for failure in &failures {
        println!("failed     {failure}");
    }
    let unzipped = work.join("shell").join(key.as_str());
    remove(&unzipped)?;
    println!("results: {}", path.display());
    Ok(Exit::check(ok))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_deck_that_drew_other_fonts_than_the_motion_run_is_named() {
        let pair = |id: &str| -> PairKey { format!("{id}:keynote").parse().unwrap() };
        let motion = BTreeMap::from([
            (pair("keynote-talk"), Sha256::of("pinned")),
            (pair("retro"), Sha256::of("retro")),
            (pair("other"), Sha256::of("other")),
        ]);
        let same = BTreeMap::from([
            (pair("keynote-talk"), Sha256::of("pinned")),
            (pair("retro"), Sha256::of("retro")),
        ]);
        assert!(drawn_otherwise(&same, &motion).is_empty());
        let unsynced = BTreeMap::from([
            (pair("keynote-talk"), Sha256::of("unpinned")),
            (pair("retro"), Sha256::of("retro")),
        ]);
        assert_eq!(
            drawn_otherwise(&unsynced, &motion),
            ["keynote-talk:keynote"]
        );
    }

    #[test]
    fn the_markup_names_its_files_and_nothing_outside() {
        let html = r##"<html><head><style>.a{background:url("media/a.png")}</style>
<script>const s = "<img src=\"https://example.org/x.png\">";</script></head><body>
<template data-sd-slide="a"><img src="media/b.png"><video poster="media/c.png" src="./media/d.mp4"></video>
<a href="#x">x</a><a href="https://example.org/talk">talk</a><img src="data:image/png;base64,AAAA"><img src="https://cdn.example.org/e.png">
<img src="/abs/f.png"><svg><image href="//host/g.png"/></svg></template></body></html>"##;
        let (files, external) = references(html);
        assert_eq!(
            files.into_iter().collect::<Vec<_>>(),
            ["media/a.png", "media/b.png", "media/c.png", "media/d.mp4"]
        );
        assert_eq!(
            external.into_iter().collect::<Vec<_>>(),
            [
                "//host/g.png",
                "/abs/f.png",
                "https://cdn.example.org/e.png"
            ]
        );
    }

    #[test]
    fn a_slide_with_a_video_or_a_gif_shows_media_that_moves() {
        let html = r#"<template data-sd-slide="a"><img src="media/x.png"></template><template data-sd-slide="b"><img src="media/y.GIF"></template><template data-sd-slide="c"><video poster="media/p.png"></video></template>"#;
        assert!(!time_media(html, "a"));
        assert!(time_media(html, "b"));
        assert!(time_media(html, "c"));
        assert!(!time_media(html, "missing"));
    }

    #[test]
    fn a_boot_passes_when_the_deck_mounted_itself_reported_nothing_and_shows_what_was_scored() {
        let first = |status| ShellCheck {
            status,
            frame: Sha256::of("a"),
            expected: Sha256::of("a"),
        };
        let boot = |mounted, errors: &[&str], first| ShellBoot {
            mounted,
            errors: errors.iter().map(|e| (*e).to_owned()).collect(),
            first,
            chrome: Vec::new(),
        };
        assert!(boot(true, &[], Some(first(FrameMatch::Identical))).passes());
        assert!(boot(true, &[], None).passes());
        assert!(!boot(false, &[], None).passes());
        assert!(!boot(true, &["csp script-src inline"], None).passes());
        assert!(!boot(true, &[], Some(first(FrameMatch::Differs))).passes());
        // A deck a fraction of a pixel off its page, within the antialias bound.
        assert!(boot(true, &[], Some(first(FrameMatch::Antialiased))).passes());
    }

    #[test]
    fn frames_compare_outside_the_players_chrome() {
        let png = |paint: &[(usize, usize)]| {
            let mut data = vec![255u8; 8 * 4 * 4];
            for (x, y) in paint {
                let at = (y * 8 + x) * 4;
                data[at] = 0;
            }
            let image = slidock_bench_metrics::Rgba::new(8, 4, data).unwrap();
            crate::explain::encode_png(&image).unwrap()
        };
        let (clean, counter) = (png(&[]), png(&[(6, 3), (7, 3)]));
        assert_eq!(strays_outside(&clean, &clean, &[], 0), Some(0));
        assert_eq!(strays_outside(&clean, &counter, &[], 0), Some(2));
        // Within the tolerance a channel does not count.
        assert_eq!(strays_outside(&clean, &counter, &[], 255), Some(0));
        // The counter's box (at 6.5..8 × 3.2..4, a pixel wider each side) is left out.
        assert_eq!(
            strays_outside(&clean, &counter, &[[6.5, 3.2, 1.5, 0.8]], 0),
            Some(0)
        );
        assert_eq!(
            strays_outside(&clean, &counter, &[[0.0, 0.0, 2.0, 2.0]], 0),
            Some(2)
        );
        assert_eq!(strays_outside(&clean, b"not a png", &[], 0), None);
    }

    #[test]
    fn a_container_reads_the_deck_under_its_work_folder() {
        let work = Path::new("/cache/oracle");
        let index = work.join("shell/k/a b/index.html");
        assert_eq!(
            file_url(&index, &RendererChoice::Container, work),
            "file:///work/shell/k/a%20b/index.html"
        );
        assert_eq!(
            file_url(&index, &RendererChoice::Native(None), work),
            "file:///cache/oracle/shell/k/a%20b/index.html"
        );
    }
}
