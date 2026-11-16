//! `oracle [--presentation …] [--stages] [--record] [--accept-oracle]`: the attended command that builds and verifies
//! oracles. Keynote exports happen here only (with the Automation probe and the font check); every oracle's pages are
//! rasterised and their words extracted, and the page-raster hashes are the oracle's recorded identity.
//!
//! A Keynote export whose file differs from the recorded one (Keynote writes its creation date into the PDF) is
//! accepted by `--record` when every page raster equals the rasters the manifest records under the same raster key:
//! the pixels are the reviewed ones.
//!
//! `--stages` makes the motion lane's stage oracles instead: each Keynote deck exported by Keynote 14.5 with
//! `all stages:true` into `$SLIDOCK_STAGES_ORACLES/<sourceSha>/<exporter>-all-stages.pdf` (an export already there is
//! kept, and Keynote is not asked anything), its pages rasterised, and with `--record` their raster hashes, the file's
//! sha256, the exporter and the pages each shown slide prints (`pagesPerRow`, by the deck's timelines) pinned in the
//! manifest's `stages`; a re-export with the same rasters is the same oracle.
//!
//! `--movie` checks the motion lane's transition oracles instead: each Keynote deck's movie export in
//! `$SLIDOCK_MOVIE_ORACLES/<sourceSha>/` (made by `bench/scripts/export-keynote-movie.applescript`; this command never
//! makes one), its transition spans (recorded, else proposed from Keynote's export timing, [`movies`]) checked against
//! where the movie changes, and with `--record` the spans, the file's sha256 and the RGBA sha256 of every frame the
//! lane scores, under the decoder's key, pinned in `movies.json` beside the manifest.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use clap::Args;
use futures::StreamExt;
use slidock_bench_schema::corpus::{
    CorpusManifest, KeynoteExporter, KeynoteStages, Oracle, PageHashes, Presentation, StagesOracle,
};
use slidock_bench_schema::{Frontend, PresentationId, Sha256, SlideId, Tier};

use crate::cache::remove;
use crate::compile::{COMPILE_TIMEOUT, CompileStatus, compiler_binary, run_compiler};
use crate::corpus::{Corpus, load_manifest, refuse_holdout, save_manifest};
use crate::error::{BenchError, Exit, Result};
use crate::hash::FileHashes;
use crate::motion_lane::deck::read_ir;
use crate::motion_lane::{ir_arguments, movies, stages};
use crate::oracle::fonts_check::{embedded_fonts, exported_font_names, missing_fonts};
use crate::oracle::keynote::{
    ExportKind, KEYNOTE_BUNDLE_IDS, export_keynote, export_path, exporter_id, installed_exporter,
    is_running, probe_keynote, quit_keynote, stages_export_path,
};
use crate::oracle::resolve::{oracle_page, pages_key, presentation_oracle};
use crate::renderer::docker::{Renderer, released};
use crate::setup::{Context, parse_list};

#[derive(Debug, Args)]
pub struct OracleArgs {
    #[arg(long)]
    presentation: Option<String>,
    /// Records the page rasters (and a Keynote export's sha256 and fonts) in the manifest.
    #[arg(long)]
    record: bool,
    /// Accepts an oracle whose export or rasters differ from the recorded ones.
    #[arg(long)]
    accept_oracle: bool,
    /// Keynote's all-stages exports (the motion lane's stage oracles) instead of the static oracles.
    #[arg(long, conflicts_with = "movie")]
    stages: bool,
    /// Keynote's movie exports (the motion lane's transition oracles) instead of the static oracles.
    #[arg(long)]
    movie: bool,
}

#[derive(Clone, Copy)]
struct Mode {
    record: bool,
    accept: bool,
}

/// The page rasters an oracle records under a raster key.
fn recorded_pages<'a>(oracle: &'a Oracle, key: &str) -> Option<&'a Vec<Sha256>> {
    match oracle {
        Oracle::PdfFile(o) => o.page_hashes.as_ref(),
        Oracle::KeynoteExport(o) => o.page_hashes.as_ref(),
    }?
    .get(key)
}

/// Whether `--record` takes a Keynote export whose file is not the recorded one: with `--accept-oracle`, or when
/// no export is recorded yet, or when every page raster equals the recorded rasters under the same raster key.
fn accepts_new_export(
    mode: Mode,
    export_recorded: bool,
    recorded: Option<&Vec<Sha256>>,
    hashes: &[Sha256],
) -> bool {
    mode.accept
        || (mode.record && (!export_recorded || recorded.is_some_and(|known| known == hashes)))
}

/// The PostScript names a deck's export must embed, proposed from the compiler's source-phase IR, or why none are.
async fn requested_fonts(
    ctx: &Context,
    file: &Path,
) -> Result<std::result::Result<Vec<String>, String>> {
    let binary = compiler_binary(
        &ctx.env.repo_root,
        &ctx.env.target_dir,
        ctx.env.compiler.as_deref(),
    )
    .await?;
    let dir = ctx
        .cache
        .root
        .join("tmp")
        .join(format!("fonts-{}", std::process::id()));
    crate::cache::remove(&dir)?;
    let arguments = vec![
        "compile".to_owned(),
        file.to_string_lossy().into_owned(),
        "--phase".to_owned(),
        "source".to_owned(),
        "--emit".to_owned(),
        "ir".to_owned(),
        "--out".to_owned(),
        dir.to_string_lossy().into_owned(),
    ];
    let outcome = run_compiler(&binary, &arguments, Duration::from_secs(600), false).await?;
    let ir = std::fs::read_to_string(dir.join("presentation-ir.json"));
    crate::cache::remove(&dir)?;
    use crate::compile::CompileStatus;
    if !matches!(outcome.status, CompileStatus::Ok | CompileStatus::Errors) {
        let first = outcome.stderr.lines().next().unwrap_or("").to_owned();
        return Ok(Err(format!(
            "the compiler cannot lower it yet ({:?}: {first})",
            outcome.status
        )));
    }
    let value: serde_json::Value = match ir.ok().and_then(|text| serde_json::from_str(&text).ok()) {
        Some(value) => value,
        None => return Ok(Err("the compiler wrote no readable IR".to_owned())),
    };
    let names: BTreeSet<String> = exported_font_names(&value).into_iter().collect();
    Ok(Ok(names.into_iter().collect()))
}

/// Exports a Keynote deck with the recorded exporter, falling back to Creator Studio for a file 14.5 refuses.
async fn keynote_export(
    ctx: &Context,
    corpus: &Corpus,
    presentation: &Presentation,
    recorded: &KeynoteExporter,
    source_name: &str,
    mode: Mode,
    kind: ExportKind,
) -> Result<std::result::Result<KeynoteExporter, String>> {
    let Some(source) = presentation.sources.iter().find(|s| s.file == source_name) else {
        return Ok(Err(format!("{source_name} is not a source")));
    };
    let Some(sha) = &source.sha256 else {
        return Ok(Err(format!("{source_name} is not a hashed source")));
    };
    let file = corpus
        .source_file(source)?
        .ok_or_else(|| BenchError::failed(format!("{source_name} is not in the corpus")))?;
    let mut candidates = vec![recorded.bundle_id.clone()];
    candidates.extend(
        KEYNOTE_BUNDLE_IDS
            .iter()
            .filter(|id| **id != recorded.bundle_id)
            .map(|id| (*id).to_owned()),
    );
    let script = ctx.env.bench_dir.join("scripts/export-keynote.applescript");
    for bundle_id in candidates {
        let Ok(installed) = installed_exporter(&bundle_id).await else {
            continue;
        };
        if bundle_id == recorded.bundle_id
            && exporter_id(&installed) != exporter_id(recorded)
            && !mode.accept
        {
            return Ok(Err(format!(
                "the installed exporter is {}, the manifest records {}",
                exporter_id(&installed),
                exporter_id(recorded)
            )));
        }
        let out = match kind {
            ExportKind::Final => export_path(&ctx.env.keynote_dir, sha.as_str(), &installed),
            ExportKind::AllStages => {
                stages_export_path(&ctx.env.stages_dir, sha.as_str(), &installed)
            }
        };
        if !out.exists() {
            probe_keynote(&bundle_id).await?;
            println!(
                "  exporting with {} (Keynote opens on screen)",
                exporter_id(&installed)
            );
        }
        match export_keynote(&script, &out, &file, &installed, kind).await? {
            Ok(_) => {
                if bundle_id != recorded.bundle_id {
                    println!(
                        "  NOTE {} refused {source_name}; exported with {}",
                        exporter_id(recorded),
                        exporter_id(&installed)
                    );
                }
                return Ok(Ok(installed));
            }
            Err(reason) => println!("  {} refused: {reason}", exporter_id(&installed)),
        }
    }
    Ok(Err(
        "no installed Keynote exported the presentation".to_owned()
    ))
}

async fn check_presentation(
    ctx: &Context,
    corpus: &Corpus,
    renderer: &Arc<Renderer>,
    poppler: &str,
    presentation: &Presentation,
    mode: Mode,
) -> Result<(bool, Option<Presentation>)> {
    println!("{}", presentation.id);
    let mismatches = corpus.verify(presentation)?;
    if !mismatches.is_empty() {
        for m in mismatches {
            println!(
                "  corpus-hash-mismatch {}: expected {}",
                m.file,
                m.expected.as_deref().unwrap_or("none")
            );
        }
        return Ok((false, None));
    }
    let mut entry = presentation.clone();
    if let Oracle::KeynoteExport(oracle) = &presentation.oracle {
        match keynote_export(
            ctx,
            corpus,
            presentation,
            &oracle.exporter,
            &oracle.source,
            mode,
            ExportKind::Final,
        )
        .await?
        {
            Ok(exporter) => {
                if let Oracle::KeynoteExport(o) = &mut entry.oracle {
                    o.exporter = exporter;
                }
            }
            Err(problem) => {
                println!("  oracle-missing: {problem}");
                return Ok((false, None));
            }
        }
    }
    let oracle =
        match presentation_oracle(renderer, corpus, &ctx.env.keynote_dir, &entry, true).await? {
            Ok(oracle) => oracle,
            Err(problem) => {
                println!("  {}: {}", problem.reason, problem.detail);
                return Ok((false, None));
            }
        };
    let numbers: Vec<u32> = (1..=oracle.pages.len() as u32).collect();
    let pages: Vec<Result<_>> = futures::stream::iter(numbers)
        .map(|page| oracle_page(renderer, &oracle, page, poppler, false))
        .buffered(4)
        .collect()
        .await;
    let pages = pages.into_iter().collect::<Result<Vec<_>>>()?;
    let key = pages_key(&oracle.pages, poppler)?;
    let hashes: Vec<Sha256> = pages
        .iter()
        .map(|page| Sha256::new(page.raster.sha256.clone()))
        .collect::<std::result::Result<_, _>>()?;
    let blocks: usize = pages.iter().map(|p| p.text.blocks.len()).sum();
    println!(
        "  {} pages, raster key {}, {blocks} text blocks",
        pages.len(),
        key.get(..12).unwrap_or(&key)
    );
    let recorded_rasters = recorded_pages(&presentation.oracle, &key).cloned();

    if let Oracle::KeynoteExport(keynote) = &mut entry.oracle {
        let recorded = keynote.sha256.as_ref().map(|s| s.as_str().to_owned());
        if recorded.as_deref() != Some(oracle.pdf.sha256.as_str()) {
            let why = if recorded.is_some() {
                "the export differs from the recorded one"
            } else {
                "no export sha256 is recorded"
            };
            let accepted =
                accepts_new_export(mode, recorded.is_some(), recorded_rasters.as_ref(), &hashes);
            println!(
                "  oracle-identity: {why}{}",
                if accepted { " (recorded)" } else { "" }
            );
            if !accepted {
                return Ok((false, None));
            }
        }
        let fonts = match (&keynote.fonts, mode.accept) {
            (Some(fonts), false) => Some(fonts.clone()),
            _ if !mode.record && !mode.accept => {
                println!(
                    "  oracle-font-unchecked: no expected fonts are recorded; run `oracle --record` and review them"
                );
                None
            }
            _ => {
                let source = presentation
                    .sources
                    .iter()
                    .find(|s| s.file == keynote.source);
                let file = match source {
                    Some(source) => corpus.source_file(source)?,
                    None => None,
                };
                match file {
                    Some(file) => match requested_fonts(ctx, &file).await? {
                        Ok(names) => Some(names),
                        Err(why) => {
                            println!(
                                "  oracle-font-unchecked: {why}{}",
                                if mode.accept {
                                    "; recorded with no fonts"
                                } else {
                                    ""
                                }
                            );
                            mode.accept.then(Vec::new)
                        }
                    },
                    None => None,
                }
            }
        };
        let Some(fonts) = fonts else {
            return Ok((false, None));
        };
        let missing = missing_fonts(&fonts, &embedded_fonts(renderer, &oracle.pdf).await?);
        if !missing.is_empty() {
            println!(
                "  oracle-font-missing: Keynote substituted {}; install them and export again",
                missing.join(", ")
            );
            return Ok((false, None));
        }
        println!("  fonts: all {} expected faces embedded", fonts.len());
        keynote.sha256 = Some(Sha256::new(oracle.pdf.sha256.clone())?);
        keynote.fonts = Some(fonts);
    }
    let mut ok = true;
    for source in &entry.sources {
        if let Some(rows) = &source.rows
            && rows.len() != pages.len()
        {
            println!(
                "  oracle-page-count: {} has {} rows, the oracle {} pages",
                source.frontend,
                rows.len(),
                pages.len()
            );
            ok = false;
        }
    }
    let recorded = recorded_rasters;
    if recorded.is_none() && !mode.record && !mode.accept {
        println!(
            "  oracle-identity: no page rasters are recorded under this raster key; run with --record"
        );
        return Ok((false, None));
    }
    let changed = recorded.as_ref().is_some_and(|known| known != &hashes);
    if changed {
        println!(
            "  the oracle's page rasters changed: an oracle-identity change{}",
            if mode.accept { " (accepted)" } else { "" }
        );
        if !mode.accept {
            return Ok((false, None));
        }
    }
    if !(mode.record || mode.accept) {
        return Ok((ok, None));
    }
    let set = |hashes_map: &mut Option<std::collections::BTreeMap<String, Vec<Sha256>>>| {
        hashes_map
            .get_or_insert_with(Default::default)
            .insert(key.clone(), hashes.clone());
    };
    match &mut entry.oracle {
        Oracle::PdfFile(o) => set(&mut o.page_hashes),
        Oracle::KeynoteExport(o) => set(&mut o.page_hashes),
    }
    Ok((ok, Some(entry)))
}

/// A Keynote deck's all-stages export: made (attended) unless it is on disk, rasterised in the renderer, then checked
/// against the page rasters the manifest pins ([`stages::identity`]) and against the pages each shown slide prints by
/// the deck's timelines ([`stage_rows`], which must add up to the export), and with `--record` (or `--accept-oracle`)
/// pinned there ([`stages_decision`]): the rasters under their raster key, the file's sha256, the exporter and the
/// rows (`pagesPerRow`). An export already on disk is read under the recorded exporter's name without asking Keynote
/// anything.
async fn stages_presentation(
    ctx: &Context,
    corpus: &Corpus,
    binary: &Path,
    renderer: &Renderer,
    poppler: &str,
    presentation: &Presentation,
    mode: Mode,
) -> Result<(bool, Option<Presentation>)> {
    let Oracle::KeynoteExport(oracle) = &presentation.oracle else {
        return Ok((true, None));
    };
    println!("{}", presentation.id);
    let Some((source, sha)) = presentation
        .sources
        .iter()
        .find(|s| s.file == oracle.source)
        .and_then(|s| Some((s, s.sha256.as_ref()?)))
    else {
        println!("  oracle-missing: {} is not a hashed source", oracle.source);
        return Ok((false, None));
    };
    let recorded_exporter = presentation
        .stages
        .as_ref()
        .map_or(&oracle.exporter, |stages| &stages.exporter);
    let on_disk = stages_export_path(&ctx.env.stages_dir, sha.as_str(), recorded_exporter);
    let exporter = if on_disk.exists() {
        recorded_exporter.clone()
    } else {
        match keynote_export(
            ctx,
            corpus,
            presentation,
            &oracle.exporter,
            &oracle.source,
            mode,
            ExportKind::AllStages,
        )
        .await?
        {
            Ok(exporter) => exporter,
            Err(problem) => {
                println!("  oracle-missing: {problem}");
                return Ok((false, None));
            }
        }
    };
    let path = stages_export_path(&ctx.env.stages_dir, sha.as_str(), &exporter);
    let export = Sha256::new(crate::hash::sha256_file(&path)?)?;
    println!("  {} sha256 {export}", path.display());
    let pdf = crate::oracle::pdf::oracle_copy(&renderer.work, &path, export.as_str())?;
    let sizes = crate::oracle::pdf::page_sizes(renderer, &pdf).await?;
    let pages = crate::oracle::resolve::PresentationOracle { pdf, pages: sizes };
    let numbers: Vec<u32> = (1..=pages.pages.len() as u32).collect();
    let rasters: Vec<Result<_>> = futures::stream::iter(numbers)
        .map(|page| oracle_page(renderer, &pages, page, poppler, false))
        .buffered(4)
        .collect()
        .await;
    let hashes: Vec<Sha256> = rasters
        .into_iter()
        .map(|page| page.and_then(|page| Ok(Sha256::new(page.raster.sha256)?)))
        .collect::<Result<_>>()?;
    let key = pages_key(&pages.pages, poppler)?;
    println!(
        "  {} pages, raster key {}",
        hashes.len(),
        key.get(..12).unwrap_or(&key)
    );
    let identity = stages::identity(presentation.stages.as_ref(), &key, &hashes, export.as_str());
    // The pages each shown slide prints by the deck's timelines: they must add up to the export, and once recorded
    // they hold the page map the motion lane reads.
    let file = corpus.source_file(source)?.ok_or_else(|| {
        BenchError::failed(format!(
            "{}: {} is not in the corpus",
            presentation.id, source.file
        ))
    })?;
    let rows = stage_rows(ctx, binary, &file, sha.as_str()).await?;
    let recorded_rows = presentation
        .stages
        .as_ref()
        .and_then(|s| s.pages_per_row.as_ref());
    let rows_check = stages::check_rows(recorded_rows, &rows, hashes.len() as u32);
    let decision = stages_decision(&identity, &rows_check, mode);
    for line in &decision.lines {
        println!("  {line}");
    }
    if !decision.record {
        return Ok((decision.passed, None));
    }
    let mut entry = presentation.clone();
    let mut pages_per_row = BTreeMap::new();
    for (slide, n) in rows {
        pages_per_row.insert(SlideId::new(slide)?, n);
    }
    let page_hashes = pinned_rasters(presentation.stages.as_ref(), &export, key, hashes);
    entry.stages = Some(StagesOracle {
        kind: KeynoteStages::Value,
        exporter,
        sha256: Some(export),
        page_hashes: Some(page_hashes),
        pages_per_row: Some(pages_per_row),
    });
    Ok((true, Some(entry)))
}

/// The page rasters a stages entry pins once an export's rasters under `key` are recorded: those, beside the rasters
/// the entry pins under other raster keys (another renderer or poppler) when they are of the same export file. An
/// export whose file differs from the recorded one keeps only its own.
fn pinned_rasters(
    recorded: Option<&StagesOracle>,
    export: &Sha256,
    key: String,
    hashes: Vec<Sha256>,
) -> PageHashes {
    let mut pinned = recorded
        .filter(|r| r.sha256.as_ref() == Some(export))
        .and_then(|r| r.page_hashes.clone())
        .unwrap_or_default();
    pinned.insert(key, hashes);
    pinned
}

/// What `oracle --stages` does with an export: whether it passes, whether its stages entry is (re)written, and what
/// it prints.
#[derive(Debug, PartialEq, Eq)]
struct StagesDecision {
    passed: bool,
    record: bool,
    lines: Vec<String>,
}

/// An export whose page rasters are the pinned ones and whose rows are the recorded ones passes as it is. `--record`
/// pins rasters and rows where none are; `--accept-oracle` also takes rasters or rows that differ from the pinned
/// ones. Rows that do not add up to the export's pages are never recorded.
fn stages_decision(identity: &stages::Identity, rows: &stages::Rows, mode: Mode) -> StagesDecision {
    let mut lines = Vec::new();
    if let stages::Rows::PageCount(why) = rows {
        lines.push(format!("oracle-page-count: {why}"));
        return StagesDecision {
            passed: false,
            record: false,
            lines,
        };
    }
    let unpinned = matches!(
        identity,
        stages::Identity::File | stages::Identity::Unpinned
    );
    let unrecorded = matches!(rows, stages::Rows::Unrecorded);
    let differs = matches!(identity, stages::Identity::Differs(_))
        || matches!(rows, stages::Rows::Differs(_));
    if !unpinned && !unrecorded && !differs {
        return StagesDecision {
            passed: true,
            record: false,
            lines,
        };
    }
    let record = mode.accept || (mode.record && !differs);
    // What is missing is reported as missing, or as what `--record` wrote; what differs, as accepted.
    let accepted = if record { " (accepted)" } else { "" };
    match identity {
        stages::Identity::Differs(why) => lines.push(format!("oracle-identity: {why}{accepted}")),
        _ if unpinned && record => lines.push("page rasters pinned".to_owned()),
        _ if unpinned => lines.push("oracle-identity: no page rasters are pinned".to_owned()),
        _ => {}
    }
    match rows {
        stages::Rows::Differs(why) => lines.push(format!("stage-count: {why}{accepted}")),
        stages::Rows::Unrecorded if record => lines.push("pagesPerRow recorded".to_owned()),
        stages::Rows::Unrecorded => lines.push("pagesPerRow is not recorded".to_owned()),
        _ => {}
    }
    // Without `--record`, an export pinned by its file or with no rows recorded yet is still the reviewed one.
    let pending = !differs && !matches!(identity, stages::Identity::Unpinned);
    if !record && pending {
        lines.push("`--record` pins what is missing".to_owned());
    }
    StagesDecision {
        passed: record || pending,
        record,
        lines,
    }
}

/// The frames the lane reads from a movie, by `<p>:<slide>:keynote#t@<pm>`: the transition rows and the two ends.
const MOVIE_ROWS: [u32; 5] = [0, 250, 500, 750, 1000];
/// Pixels two held frames of a movie may differ by (its own blips stay under it).
const MOVIE_STILL: usize = 500;

/// A Keynote deck's movie export: its spans (the record's, else proposed from the deck's IR) checked against where
/// the movie changes, its sha256 and its scored frames against the record's pins, and with `--record` (or
/// `--accept-oracle` when they differ) the record written.
async fn movie_presentation(
    ctx: &Context,
    corpus: &Corpus,
    binary: &Path,
    decoder: &movies::Decoder,
    presentation: &Presentation,
    recorded: Option<&movies::MovieOracle>,
    mode: Mode,
) -> Result<(bool, Option<movies::MovieOracle>)> {
    let Oracle::KeynoteExport(oracle) = &presentation.oracle else {
        return Ok((true, None));
    };
    let Some((source, sha)) = presentation
        .sources
        .iter()
        .find(|s| s.file == oracle.source)
        .and_then(|s| Some((s, s.sha256.as_ref()?)))
    else {
        return Ok((true, None));
    };
    let Some(file) = movies::movie_of(&ctx.env.movies_dir, sha.as_str()) else {
        if recorded.is_some() {
            println!(
                "{}\n  oracle-missing: no movie in {}/{sha}/",
                presentation.id,
                ctx.env.movies_dir.display()
            );
            return Ok((false, None));
        }
        return Ok((true, None));
    };
    println!("{}", presentation.id);
    let movie_sha = Sha256::new(crate::hash::sha256_file(&file)?)?;
    println!("  {} sha256 {movie_sha}", file.display());
    let deck = corpus.source_file(source)?.ok_or_else(|| {
        BenchError::failed(format!(
            "{}: {} is not in the corpus",
            presentation.id, source.file
        ))
    })?;
    let (slides, size) = deck_ir(ctx, binary, &deck, sha.as_str()).await?;
    let transitions = match movies::transitions_of(&slides) {
        Ok(transitions) => transitions,
        Err(why) => {
            println!("  movie-timing: {why}");
            return Ok((false, None));
        }
    };
    const FPS: u32 = 60;
    let proposed = movies::proposed_spans(&transitions, FPS);
    let spans = recorded.map_or_else(|| proposed.clone(), |r| r.transitions.clone());
    if recorded.is_some_and(|r| r.transitions != proposed) {
        println!("  the recorded spans are not Keynote's export timing: they are kept as reviewed");
    }
    // Where the movie holds and moves around each span.
    let checks: Vec<u32> = spans.values().flat_map(movies::span_checks).collect();
    let frames = decoder.frames(&file, size, &checks).await?;
    let mut problems = Vec::new();
    for (slide, span) in &spans {
        for problem in movies::span_problems(span, &frames, MOVIE_STILL) {
            problems.push(format!(
                "{slide} (frames {}+{}): {problem}",
                span.start, span.frames
            ));
        }
    }
    // The frames the lane scores.
    let key = decoder.key();
    let mut wanted: Vec<(String, u32)> = Vec::new();
    for (slide, span) in &spans {
        for pm in MOVIE_ROWS {
            let row = format!("{}:{slide}:keynote#t@{pm}", presentation.id);
            wanted.push((row, span.frame(pm).0));
        }
    }
    let indices: Vec<u32> = wanted.iter().map(|(_, n)| *n).collect();
    let scored = decoder.frames(&file, size, &indices).await?;
    let hashes: BTreeMap<String, Sha256> = wanted
        .iter()
        .filter_map(|(row, n)| Some((row.clone(), movies::frame_hash(scored.get(n)?))))
        .collect();
    let pins = recorded.and_then(|r| r.frame_hashes.get(&key));
    let same_file = recorded.is_some_and(|r| r.sha256 == movie_sha);
    let same_frames = pins.is_some_and(|p| p == &hashes);
    let mut lines = Vec::new();
    for problem in &problems {
        lines.push(format!("movie-span: {problem}"));
    }
    let pinned = same_file || same_frames;
    match recorded {
        None => lines.push("unrecorded".to_owned()),
        Some(_) if !pinned => lines.push(format!(
            "oracle-identity: the movie is not the recorded file{}",
            if pins.is_some() {
                ", and its frames differ from the pinned ones"
            } else {
                ""
            }
        )),
        Some(_) if pins.is_none() => {
            lines.push("no frames are pinned under this decoder".to_owned())
        }
        Some(_) if !same_frames => {
            lines.push("oracle-identity: frames differ from the pinned ones".to_owned())
        }
        Some(_) => {}
    }
    for line in &lines {
        println!("  {line}");
    }
    println!(
        "  {} transitions, {} frames, decoder {} ({})",
        spans.len(),
        hashes.len(),
        key.get(..12).unwrap_or(&key),
        decoder.version
    );
    let clean = problems.is_empty() && pinned && pins.is_none_or(|p| p == &hashes);
    let record =
        (mode.record && problems.is_empty() && (recorded.is_none() || pinned)) || mode.accept;
    if !record {
        return Ok((clean, None));
    }
    let mut frame_hashes = recorded.map(|r| r.frame_hashes.clone()).unwrap_or_default();
    if !same_file {
        frame_hashes.clear();
    }
    frame_hashes.insert(key, hashes);
    Ok((
        true,
        Some(movies::MovieOracle {
            exporter: recorded.map_or_else(|| oracle.exporter.clone(), |r| r.exporter.clone()),
            file: file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default(),
            sha256: movie_sha,
            fps: FPS,
            width: size.0,
            height: size.1,
            transitions: spans,
            frame_hashes,
        }),
    ))
}

/// A Keynote deck's IR with the motion its deck plays, and its slide size in whole points (Keynote's movie pixels).
async fn deck_ir(
    ctx: &Context,
    binary: &Path,
    file: &Path,
    sha: &str,
) -> Result<(Vec<crate::motion_lane::deck::IrSlide>, (u32, u32))> {
    let out = ctx.cache.root.join("oracle-movie-ir").join(sha);
    remove(&out)?;
    let outcome = run_compiler(
        binary,
        &ir_arguments(file, Frontend::Keynote, &out),
        COMPILE_TIMEOUT,
        true,
    )
    .await?;
    if !matches!(outcome.status, CompileStatus::Ok | CompileStatus::Errors) {
        return Err(BenchError::failed(format!(
            "{}: the IR did not compile ({:?})",
            file.display(),
            outcome.status
        )));
    }
    let path = out.join("presentation-ir.json");
    let slides = read_ir(&path)?;
    let text = std::fs::read_to_string(&path).map_err(BenchError::io(&path))?;
    let ir: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| BenchError::failed(e.to_string()))?;
    let side = |name: &str| ir["size"][name].as_f64().map_or(0, |v| v.round() as u32);
    remove(&out)?;
    Ok((slides, (side("w"), side("h"))))
}

/// `oracle --movie`: every selected Keynote deck's movie oracle, and with `--record` the records written beside the
/// manifest (re-read first, so a record another command wrote meanwhile is kept).
async fn movie_oracles(
    ctx: &Context,
    corpus: &Corpus,
    selected: &[Presentation],
    mode: Mode,
) -> Result<Exit> {
    let decoder = movies::Decoder::find().await?;
    let binary = compiler_binary(
        &ctx.env.repo_root,
        &ctx.env.target_dir,
        ctx.env.compiler.as_deref(),
    )
    .await?;
    let path = movies::record_path(&ctx.env.manifest_path);
    let records = movies::load_records(&path)?;
    let mut ok = true;
    let mut updated = BTreeMap::new();
    for presentation in selected {
        let recorded = records.movies.get(presentation.id.as_str());
        let (passed, entry) =
            movie_presentation(ctx, corpus, &binary, &decoder, presentation, recorded, mode)
                .await?;
        ok &= passed;
        if let Some(entry) = entry {
            updated.insert(presentation.id.as_str().to_owned(), entry);
        }
    }
    if !updated.is_empty() {
        let mut fresh = movies::load_records(&path)?;
        fresh.movies.extend(updated);
        movies::save_records(&path, &fresh)?;
        println!("recorded in {}; review the diff", path.display());
    }
    Ok(Exit::check(ok))
}

/// The pages each shown slide of a Keynote deck prints by its timelines (`pagesPerRow`): its IR with the motion its
/// deck plays ([`ir_arguments`]), read with the stage oracles' page map ([`stages::ir_stage_pages`]).
async fn stage_rows(
    ctx: &Context,
    binary: &Path,
    file: &Path,
    sha: &str,
) -> Result<BTreeMap<String, u32>> {
    let out = ctx.cache.root.join("oracle-stages-ir").join(sha);
    remove(&out)?;
    let outcome = run_compiler(
        binary,
        &ir_arguments(file, Frontend::Keynote, &out),
        COMPILE_TIMEOUT,
        true,
    )
    .await?;
    if !matches!(outcome.status, CompileStatus::Ok | CompileStatus::Errors) {
        return Err(BenchError::failed(format!(
            "{}: the IR did not compile ({:?})",
            file.display(),
            outcome.status
        )));
    }
    let slides = read_ir(&out.join("presentation-ir.json"))?;
    remove(&out)?;
    Ok(stages::pages_per_row(&stages::ir_stage_pages(&slides).0))
}

/// Every selected presentation's oracle, checked (and with `--record` recorded) in `renderer`; Keynote is quit after
/// when this command launched it for an export.
async fn check_all(
    ctx: &Context,
    corpus: &Corpus,
    selected: &[Presentation],
    launched: &[&str],
    renderer: &Renderer,
    stages: bool,
    mode: Mode,
) -> Result<(bool, Vec<Presentation>)> {
    let (_, poppler) = renderer.facts().await?;
    let renderer = Arc::new(renderer.clone());
    let binary = if stages {
        Some(
            compiler_binary(
                &ctx.env.repo_root,
                &ctx.env.target_dir,
                ctx.env.compiler.as_deref(),
            )
            .await?,
        )
    } else {
        None
    };
    let mut ok = true;
    let mut updated = Vec::new();
    let mut failure = None;
    for presentation in selected {
        let checked = if let Some(binary) = &binary {
            stages_presentation(ctx, corpus, binary, &renderer, &poppler, presentation, mode).await
        } else {
            check_presentation(ctx, corpus, &renderer, &poppler, presentation, mode).await
        };
        match checked {
            Ok((passed, entry)) => {
                ok &= passed;
                updated.extend(entry);
            }
            Err(error) => {
                failure = Some(error);
                break;
            }
        }
    }
    // Keynote is quit only when this command started it for an export: with every export on disk it never runs.
    if crate::oracle::keynote::EXPORTED.load(std::sync::atomic::Ordering::SeqCst) {
        for bundle in launched {
            if is_running(bundle).await.unwrap_or(false) {
                quit_keynote(bundle).await?;
            }
        }
    }
    match failure {
        Some(error) => Err(error),
        None => Ok((ok, updated)),
    }
}

pub async fn oracle(ctx: &Context, args: OracleArgs) -> Result<Exit> {
    let manifest: CorpusManifest = load_manifest(&ctx.env.manifest_path)?;
    let named: Option<Vec<PresentationId>> =
        parse_list(args.presentation.as_deref(), "presentation")?;
    refuse_holdout(
        &manifest,
        &[],
        named.as_deref().unwrap_or_default(),
        &[],
        "oracle",
    )?;
    let corpus_dir = ctx.env.require_corpus_dir()?.to_path_buf();
    let selected: Vec<Presentation> = manifest
        .presentations
        .iter()
        .filter(|p| p.tier != Tier::Holdout && named.as_ref().is_none_or(|ids| ids.contains(&p.id)))
        .cloned()
        .collect();
    let mut launched = Vec::new();
    for bundle in KEYNOTE_BUNDLE_IDS {
        if !is_running(bundle).await.unwrap_or(true) {
            launched.push(bundle);
        }
    }
    let mode = Mode {
        record: args.record,
        accept: args.accept_oracle,
    };
    let corpus = Corpus::new(
        &corpus_dir,
        FileHashes::new(&ctx.cache.hashes, true),
        &manifest,
    );
    if args.movie {
        return movie_oracles(ctx, &corpus, &selected, mode).await;
    }
    // The renderer is released (removed when it is this command's own) whether the checks pass, fail or err.
    let renderer = ctx.own_renderer().await?;
    let checked = check_all(
        ctx,
        &corpus,
        &selected,
        &launched,
        &renderer,
        args.stages,
        mode,
    )
    .await;
    let (ok, updated) = released(checked, renderer.release().await)?;
    if mode.record || mode.accept {
        // Re-read, so an entry another command added meanwhile is kept.
        let mut manifest = load_manifest(&ctx.env.manifest_path)?;
        for entry in updated {
            if let Some(slot) = manifest.presentations.iter_mut().find(|p| p.id == entry.id) {
                *slot = entry;
            }
        }
        manifest.validate()?;
        save_manifest(&ctx.env.manifest_path, &manifest)?;
        println!(
            "recorded in {}; review the diff",
            ctx.env.manifest_path.display()
        );
    }
    Ok(Exit::check(ok))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::motion_lane::stages::{Identity, Rows};

    const CHECK: Mode = Mode {
        record: false,
        accept: false,
    };
    const RECORD: Mode = Mode {
        record: true,
        accept: false,
    };
    const ACCEPT: Mode = Mode {
        record: false,
        accept: true,
    };

    #[test]
    fn record_takes_a_new_export_only_with_the_recorded_rasters() {
        let pages: Vec<Sha256> = (0..3).map(|k| Sha256::of(k.to_string())).collect();
        let mut moved = pages.clone();
        moved[1] = Sha256::of("x");
        // No export recorded yet: the first `--record` takes it.
        assert!(accepts_new_export(RECORD, false, None, &pages));
        // An export recorded: a new one only with every page raster equal under the same key.
        assert!(accepts_new_export(RECORD, true, Some(&pages), &pages));
        assert!(!accepts_new_export(RECORD, true, Some(&moved), &pages));
        assert!(!accepts_new_export(RECORD, true, None, &pages));
        let mut extra = pages.clone();
        extra.push(Sha256::of("extra"));
        assert!(!accepts_new_export(RECORD, true, Some(&pages), &extra));
        // A check never takes it; `--accept-oracle` always does.
        assert!(!accepts_new_export(CHECK, true, Some(&pages), &pages));
        assert!(!accepts_new_export(CHECK, false, None, &pages));
        assert!(accepts_new_export(ACCEPT, true, Some(&moved), &pages));
    }

    fn decide(identity: Identity, rows: Rows, mode: Mode) -> (bool, bool) {
        let decision = stages_decision(&identity, &rows, mode);
        (decision.passed, decision.record)
    }

    #[test]
    fn record_pins_page_rasters_and_rows_where_none_are() {
        // Pinned rasters, recorded rows: nothing to do.
        assert_eq!(
            decide(Identity::Pages, Rows::Recorded, CHECK),
            (true, false)
        );
        assert_eq!(
            decide(Identity::Pages, Rows::Recorded, RECORD),
            (true, false)
        );
        // Rows not recorded yet: they pass as the reviewed export, and `--record` records them.
        assert_eq!(
            decide(Identity::Pages, Rows::Unrecorded, CHECK),
            (true, false)
        );
        assert_eq!(
            decide(Identity::Pages, Rows::Unrecorded, RECORD),
            (true, true)
        );
        assert_eq!(
            decide(Identity::File, Rows::Unrecorded, RECORD),
            (true, true)
        );
        // Nothing pinned: only `--record` passes it.
        assert_eq!(
            decide(Identity::Unpinned, Rows::Unrecorded, CHECK),
            (false, false)
        );
        assert_eq!(
            decide(Identity::Unpinned, Rows::Unrecorded, RECORD),
            (true, true)
        );
        // Rows that differ from the record, or rasters that differ from the pin, need `--accept-oracle`.
        let moved = || Rows::Differs("kn-1 prints 2 pages, its timeline gives 1".into());
        assert_eq!(decide(Identity::Pages, moved(), CHECK), (false, false));
        assert_eq!(decide(Identity::Pages, moved(), RECORD), (false, false));
        assert_eq!(decide(Identity::Pages, moved(), ACCEPT), (true, true));
        let other = || Identity::Differs("pages 3 differ".into());
        assert_eq!(decide(other(), Rows::Recorded, RECORD), (false, false));
        assert_eq!(decide(other(), Rows::Recorded, ACCEPT), (true, true));
        // Rows that do not add up to the export are never recorded.
        let short =
            || Rows::PageCount("the export has 5 pages, the deck's timelines imply 4".into());
        assert_eq!(decide(Identity::Pages, short(), ACCEPT), (false, false));
        let decision = stages_decision(&Identity::Pages, &moved(), CHECK);
        assert_eq!(
            decision.lines,
            ["stage-count: kn-1 prints 2 pages, its timeline gives 1"]
        );
    }

    #[test]
    fn each_line_says_what_is_missing_or_what_was_recorded() {
        let lines = |identity: Identity, rows: Rows, mode: Mode| {
            stages_decision(&identity, &rows, mode).lines
        };
        assert_eq!(
            lines(Identity::Pages, Rows::Unrecorded, CHECK),
            [
                "pagesPerRow is not recorded",
                "`--record` pins what is missing"
            ]
        );
        assert_eq!(
            lines(Identity::Pages, Rows::Unrecorded, RECORD),
            ["pagesPerRow recorded"]
        );
        assert_eq!(
            lines(Identity::Unpinned, Rows::Unrecorded, CHECK),
            [
                "oracle-identity: no page rasters are pinned",
                "pagesPerRow is not recorded"
            ]
        );
        assert_eq!(
            lines(Identity::File, Rows::Unrecorded, RECORD),
            ["page rasters pinned", "pagesPerRow recorded"]
        );
        assert_eq!(
            lines(
                Identity::Differs("pages 3 differ".into()),
                Rows::Recorded,
                ACCEPT
            ),
            ["oracle-identity: pages 3 differ (accepted)"]
        );
        // Nothing missing: nothing printed.
        assert!(lines(Identity::Pages, Rows::Recorded, RECORD).is_empty());
    }

    #[test]
    fn recording_keeps_the_rasters_of_the_same_export_under_other_keys() {
        let sha = |s: &str| Sha256::of(s);
        let recorded = StagesOracle {
            kind: KeynoteStages::Value,
            exporter: KeynoteExporter {
                bundle_id: "com.apple.iWork.Keynote".into(),
                version: "14.4".into(),
                build: "7043.0.93".into(),
            },
            sha256: Some(sha("export")),
            page_hashes: Some(BTreeMap::from([
                ("other".to_owned(), vec![sha("o1")]),
                ("this".to_owned(), vec![sha("old")]),
            ])),
            pages_per_row: None,
        };
        let pinned = pinned_rasters(
            Some(&recorded),
            &sha("export"),
            "this".into(),
            vec![sha("t1")],
        );
        assert_eq!(
            pinned,
            BTreeMap::from([
                ("other".to_owned(), vec![sha("o1")]),
                ("this".to_owned(), vec![sha("t1")]),
            ])
        );
        // Another export file: the other keys' rasters are of the old one.
        let pinned = pinned_rasters(Some(&recorded), &sha("new"), "this".into(), vec![sha("t1")]);
        assert_eq!(
            pinned,
            BTreeMap::from([("this".to_owned(), vec![sha("t1")])])
        );
        let pinned = pinned_rasters(None, &sha("new"), "this".into(), vec![sha("t1")]);
        assert_eq!(
            pinned,
            BTreeMap::from([("this".to_owned(), vec![sha("t1")])])
        );
    }
}
