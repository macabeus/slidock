//! `explain <row>`: why a row scored as it did, from a run's saved compiled output, rendered again (the ink check needs
//! the text-less twin, which only a fresh render has). Writes `oracle.png`, `render.png`, `heatmap.png`,
//! `side-by-side.png`, `report.json` and `run.txt` under `<cache>/explain/<row>/` (colons as `__`), with `golden.png`
//! and `golden-diff.png` when `parity reference` rendered the golden output of the row, and prints the per-element
//! findings, the `shadow@1` rings and the `gradient@1` regions, worst first.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use clap::Args;
use serde::Serialize;
use slidock_bench_metrics::{
    HostLibm, Rect, Size, TextWord, align_words, decode_png, gradient_regions, luma, metric_pair,
    oracle_words, over_white, shadow_rings, ssim, text_ink,
};
use slidock_bench_schema::RowId;
use slidock_bench_schema::api::{ElementFinding, MissingLine};
use slidock_bench_schema::results::RowRecord;
use slidock_format::paths::file_id;

use crate::cache::write_atomic;
use crate::compile::read_pair_compile;
use crate::corpus::{Corpus, load_manifest, refuse_holdout};
use crate::error::{BenchError, Exit, Result};
use crate::explain::{
    ExplainInput, encode_png, explain_elements, explain_text, heatmap, side_by_side,
};
use crate::gradient::{
    RegionFinding, gradient_targets, ir_gradients, region_findings, regions_text,
};
use crate::hash::FileHashes;
use crate::oracle::resolve::OracleResolver;
use crate::renderer::route::{PresentationRouter, composed_documents, read_compiled, slide_url};
use crate::renderer::session::RenderRequest;
use crate::results::{read_results, resolve_results, row_file_name, row_paths};
use crate::runner::{row_approximated_gradients, row_diagnostic_counts, row_not_drawn_shadows};
use crate::setup::{Context, start_session};
use crate::shadow::{RingFinding, ir_shadows, ring_findings, rings_text, shadow_targets};

#[derive(Debug, Args)]
pub struct ExplainArgs {
    row: RowId,
    /// The run whose compiled output is explained: `latest`, a run key or a path.
    #[arg(long, default_value = "latest")]
    from: String,
}

#[derive(Serialize)]
struct Report<'a> {
    row: &'a RowId,
    record: &'a RowRecord,
    elements: &'a [ElementFinding],
    missing: &'a [MissingLine],
    /// `shadow@1`'s rings, worst first.
    rings: &'a [RingFinding],
    /// `gradient@1`'s regions, worst first.
    regions: &'a [RegionFinding],
}

/// The file that names the run an explain folder came from.
pub const EXPLAIN_RUN_FILE: &str = "run.txt";

/// Where `explain` writes a row's pictures and report.
pub fn explain_dir(cache: &Path, row: &RowId) -> PathBuf {
    cache.join("explain").join(row_file_name(row))
}

pub async fn explain(ctx: &Context, args: ExplainArgs) -> Result<Exit> {
    let row = &args.row;
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    refuse_holdout(&manifest, &[], &[], std::slice::from_ref(row), "explain")?;
    let results_file = resolve_results(&ctx.cache.runs, &ctx.env.baseline_path(), &args.from)?;
    let run_dir = results_file
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| BenchError::failed("the results have no run folder"))?;
    let results = read_results(&results_file)?;
    let record = results.rows.get(row).ok_or_else(|| {
        BenchError::refused(format!("{row} is not in {}", results_file.display()))
    })?;
    let presentation = manifest
        .presentations
        .iter()
        .find(|p| &p.id == row.presentation())
        .ok_or_else(|| {
            BenchError::refused(format!("{} is not in the manifest", row.presentation()))
        })?;
    let slide = row.slide().as_str();
    let frontend = row.frontend();
    let overlay = crate::compile::overlay_dir(&run_dir);
    let from_overlay = read_pair_compile(&overlay, presentation.id.as_str(), frontend.as_str());
    let (compile, base) = if from_overlay.slides.contains_key(slide) {
        (from_overlay, overlay)
    } else {
        (
            read_pair_compile(&run_dir, presentation.id.as_str(), frontend.as_str()),
            run_dir.clone(),
        )
    };
    let ir_path = crate::compile::pair_paths(&base, presentation.id.as_str(), frontend.as_str()).ir;
    let Some((w, h)) = compile
        .size
        .filter(|_| compile.slides.contains_key(slide) && ir_path.is_file())
    else {
        return Err(BenchError::refused(format!(
            "{row} has no compiled slide in the run ({:?})",
            record.outcome
        )));
    };
    let corpus = Corpus::new(
        ctx.env.require_corpus_dir()?,
        FileHashes::new(&ctx.cache.hashes, true),
        &manifest,
    );
    let session = start_session(ctx, &[], &[], false).await?;
    let resolver = OracleResolver::new(
        session.renderer.clone(),
        session.poppler.clone(),
        ctx.env.keynote_dir.clone(),
        false,
    );
    let oracle = resolver
        .resolve(&corpus, presentation)
        .await?
        .map_err(|reason| BenchError::refused(format!("the oracle is not available: {reason}")))?;
    let page = oracle
        .page(record.oracle.page)
        .await
        .map_err(|error| BenchError::failed(format!("oracle page: {error:?}")))?;
    let files = read_compiled(&compile.dir)?;
    let documents = composed_documents(&files)?;
    let shot = session
        .browser
        .render(&RenderRequest {
            url: slide_url(&file_id(slide)),
            size: (w, h),
            px_per_pt: f64::from(slidock_bench_metrics::RASTER_LONG_SIDE_PX) / w.max(h),
            router: Arc::new(PresentationRouter {
                dir: compile.dir.clone(),
                documents,
                gif_frames: ctx.cache.gif_frames.clone(),
            }),
        })
        .await
        .map_err(|error| BenchError::failed(format!("render: {error}")))?;
    let oracle_png = std::fs::read(&page.raster.path).map_err(BenchError::io(&page.raster.path))?;
    let decode =
        |bytes: &[u8]| decode_png(bytes).map_err(|e| BenchError::failed(format!("png: {e}")));
    let oracle_image = decode(&oracle_png)?;
    let render = decode(&shot.png)?;
    let textless = decode(&shot.textless)?;
    let (oracle_metric, render_metric) =
        metric_pair(&oracle_image, &render).map_err(|e| BenchError::failed(e.to_string()))?;
    let structure =
        ssim(&oracle_metric, &render_metric).map_err(|e| BenchError::failed(e.to_string()))?;
    let text = page.text.to_metrics();
    let slide_size = Size { w, h };
    let oracle_text = oracle_words(
        &text,
        Size {
            w: page.size.w,
            h: page.size.h,
        },
        slide_size,
    );
    let oracle_lines: Vec<usize> = text
        .blocks
        .iter()
        .flat_map(|block| &block.lines)
        .enumerate()
        .flat_map(|(index, line)| line.words.iter().map(move |_| index))
        .collect();
    let dom: Vec<TextWord> = shot
        .measure
        .words
        .iter()
        .map(|word| TextWord {
            text: word.text.clone(),
            rect: Rect {
                x: word.x,
                y: word.y,
                w: word.w,
                h: word.h,
            },
        })
        .collect();
    let with_text = luma(&over_white(&render));
    let without = luma(&over_white(&textless));
    let px_per_pt = render.width() as f64 / w;
    let ink = |rect: &Rect| text_ink(&with_text, &without, render.width(), rect, px_per_pt);
    let alignment = align_words::<HostLibm>(&oracle_text, &dom, w.max(h), Some(&ink))
        .map_err(|e| BenchError::failed(e.to_string()))?;
    let ir: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&ir_path).map_err(BenchError::io(&ir_path))?)
            .map_err(|e| BenchError::failed(format!("{}: {e}", ir_path.display())))?;
    let (elements, missing) = explain_elements(&ExplainInput {
        ir: &ir,
        measure: &shot.measure,
        oracle: &oracle_text,
        oracle_lines: &oracle_lines,
        dom: &dom,
        alignment: &alignment,
        slide: (w, h),
        oracle_metric: &oracle_metric,
        render_metric: &render_metric,
    })?;
    let not_drawn = row_not_drawn_shadows(compile.report.as_ref(), slide);
    let targets = shadow_targets(
        &ir_shadows(&ir),
        &not_drawn.iter().map(String::as_str).collect::<Vec<_>>(),
        &shot.measure,
    );
    let rings = ring_findings(
        &shadow_rings(&oracle_metric, &render_metric, &targets, slide_size)
            .map_err(|e| BenchError::failed(e.to_string()))?,
    );
    let approximated = row_approximated_gradients(compile.report.as_ref(), slide);
    let gradients = gradient_targets(
        &ir_gradients(&ir),
        &approximated.iter().map(String::as_str).collect::<Vec<_>>(),
        &shot.measure,
        slide,
        slide_size,
    );
    let regions = region_findings(
        &gradient_regions(&oracle_metric, &render_metric, &gradients, slide_size)
            .map_err(|e| BenchError::failed(e.to_string()))?,
    );
    let out = explain_dir(&ctx.cache.root, row);
    let heat = heatmap(&oracle_metric, &structure.map)?;
    write_atomic(&out.join("oracle.png"), &oracle_png)?;
    write_atomic(&out.join("render.png"), &shot.png)?;
    write_atomic(&out.join("heatmap.png"), encode_png(&heat)?)?;
    write_atomic(
        &out.join("side-by-side.png"),
        encode_png(&side_by_side(&[&oracle_metric, &render_metric, &heat])?)?,
    )?;
    // The golden output's render of the row, from `parity reference`, and where this render departs from it.
    let golden_png = crate::commands::parity::current_reference_dir(ctx)
        .and_then(|dir| std::fs::read(row_paths(&dir, row).render).ok());
    if let Some(golden_png) = golden_png {
        let golden_image = decode(&golden_png)?;
        let (a, b) =
            metric_pair(&golden_image, &render).map_err(|e| BenchError::failed(e.to_string()))?;
        let map = ssim(&a, &b)
            .map_err(|e| BenchError::failed(e.to_string()))?
            .map;
        write_atomic(&out.join("golden.png"), &golden_png)?;
        write_atomic(
            &out.join("golden-diff.png"),
            encode_png(&heatmap(&a, &map)?)?,
        )?;
    }
    write_atomic(
        &out.join("report.json"),
        slidock_bench_schema::to_canonical(&Report {
            row,
            record,
            elements: &elements,
            missing: &missing,
            rings: &rings,
            regions: &regions,
        })?,
    )?;
    let run_key = run_dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    write_atomic(&out.join(EXPLAIN_RUN_FILE), format!("{run_key}\n"))?;
    let diagnostics: Vec<(String, u32)> = row_diagnostic_counts(compile.report.as_ref(), slide)
        .into_iter()
        .collect();
    print!(
        "{}",
        explain_text(&row.to_string(), record, &diagnostics, &elements, &missing)
    );
    print!("{}", rings_text(&rings));
    print!("{}", regions_text(&regions));
    println!("images: {}", out.display());
    Ok(Exit::Ok)
}
