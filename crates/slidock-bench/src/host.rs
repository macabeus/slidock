//! The host lane: each rendered row of a static run, mounted by the deck's own player in a shadow root
//! on the bundled `index.html`, must render the same PNG as the standalone `--emit documents` render the static run
//! scored. The deck is bundled with the same compiler and fonts (`--fonts-off`: the parity path, no font source and
//! no face shipped), and rendered by the same renderer under the same render policy, so any difference is the
//! mount's: the player's stage, the shadow root, or the bundle's composition.
//!
//! The standalone document shows every element, the player's end state hides what an exit build removed: the mount
//! clears the build state, so both show the same elements (`--end-state` mounts the end state instead, which differs
//! exactly on the slides with exit builds; that state is the motion lane's).
//!
//! A host run is the static run's records with a `host` check each, stamped `lane: host`, in its own run folder:
//! `bundles/<p>.<fe>/` (the decks), `mounted/<row>.png` and `results.json`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use futures::StreamExt;
use slidock_bench_schema::lanes::{FrameMatch, FrameSummary, HostCheck, LaneSummaries};
use slidock_bench_schema::results::{Results, ResultsSchema, RowRecord};
use slidock_bench_schema::{Frontend, Lane, PresentationId, RowId, Sha256};

use crate::cache::{remove, write_atomic};
use crate::compile::{COMPILE_TIMEOUT, CompileStatus, read_pair_compile, run_compiler};
use crate::corpus::{Corpus, Selection, load_manifest, refuse_holdout, select_pairs};
use crate::error::{BenchError, Exit, Result};
use crate::hash::FileHashes;
use crate::images::{render_vs_render, same_pixels};
use crate::renderer::route::{DeckRouter, deck_url};
use crate::renderer::session::{Browser, MountRequest};
use crate::results::{
    read_results, resolve_results, results_path, row_file_name, row_paths, summary, write_results,
};
use crate::setup::{Context, start_session};
use crate::stamp::content_id_at;

/// What the host lane checks.
pub struct HostRequest {
    /// The static run: `latest`, a run key or a path.
    pub from: String,
    pub presentations: Option<Vec<PresentationId>>,
    pub frontends: Option<Vec<Frontend>>,
    pub only: Option<Vec<RowId>>,
    pub concurrency: usize,
    /// Mounts each slide after its last build, exits applied, instead of with every element shown.
    pub end_state: bool,
}

/// The mounted render of a row, and how it compares.
struct Checked {
    id: RowId,
    check: std::result::Result<(HostCheck, Vec<u8>), String>,
}

fn mounted_path(dir: &Path, row: &RowId) -> PathBuf {
    dir.join("mounted")
        .join(format!("{}.png", row_file_name(row)))
}

/// Mounts one row and compares it with the standalone render.
async fn check_row(
    browser: &Browser,
    request: MountRequest,
    record: &RowRecord,
    standalone_png: &Path,
) -> std::result::Result<(HostCheck, Vec<u8>), String> {
    let standalone = record
        .render
        .clone()
        .ok_or_else(|| "the static row has no render".to_owned())?;
    let png = browser
        .render_mounted(&request)
        .await
        .map_err(|error| error.to_string())?;
    let mounted = Sha256::of(&png);
    if mounted == standalone {
        return Ok((
            HostCheck {
                status: FrameMatch::Identical,
                standalone,
                mounted,
                ssim: None,
            },
            png,
        ));
    }
    let reference = std::fs::read(standalone_png)
        .map_err(|error| format!("{}: {error}", standalone_png.display()))?;
    let same = same_pixels(&reference, &png).map_err(|e| e.to_string())?;
    let (ssim, _) = render_vs_render(&reference, &png).map_err(|e| e.to_string())?;
    Ok((
        HostCheck {
            status: if same {
                FrameMatch::Identical
            } else {
                FrameMatch::Differs
            },
            standalone,
            mounted,
            ssim: Some(ssim),
        },
        png,
    ))
}

/// Per frontend: rows checked, identical, differing.
pub fn host_summary(rows: &BTreeMap<RowId, RowRecord>) -> BTreeMap<Frontend, FrameSummary> {
    let mut out: BTreeMap<Frontend, FrameSummary> = BTreeMap::new();
    for row in rows.values() {
        let Some(check) = &row.host else { continue };
        let entry = out.entry(row.frontend).or_default();
        entry.rows += 1;
        match check.status {
            FrameMatch::Identical => entry.identical += 1,
            FrameMatch::Antialiased => entry.antialiased += 1,
            FrameMatch::Differs => entry.differs += 1,
        }
    }
    out
}

/// Runs the host lane over a static run and writes the host run.
pub async fn host_lane(ctx: &Context, request: &HostRequest) -> Result<Exit> {
    let static_path = resolve_results(&ctx.cache.runs, &ctx.env.baseline_path(), &request.from)?;
    let static_dir = static_path
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| BenchError::failed("the static results have no run folder"))?;
    let base = read_results(&static_path)?;
    if base.stamp.lane != Lane::Static {
        return Err(BenchError::refused(format!(
            "{} is a {:?} run; the host lane checks a static run",
            static_path.display(),
            base.stamp.lane
        )));
    }
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    refuse_holdout(
        &manifest,
        &base.stamp.selection.tiers,
        request.presentations.as_deref().unwrap_or_default(),
        request.only.as_deref().unwrap_or_default(),
        "run --lane host",
    )?;
    let binary = crate::compile::compiler_binary(
        &ctx.env.repo_root,
        &ctx.env.target_dir,
        ctx.env.compiler.as_deref(),
    )
    .await?;
    let selection = Selection {
        tiers: base.stamp.selection.tiers.clone(),
        presentations: request.presentations.clone(),
        frontends: request.frontends.clone(),
    };
    let pairs: Vec<_> = select_pairs(&manifest, &selection)
        .into_iter()
        .filter(|(p, s)| {
            base.rows
                .keys()
                .any(|id| id.presentation() == &p.id && id.frontend() == s.frontend)
        })
        .collect();
    let session = start_session(ctx, &pairs, &base.stamp.selection.tiers, false).await?;
    let mut drift = Vec::new();
    // The content ids cover the bench too; the decks need only the compiler to be the static run's.
    let repo = &ctx.env.repo_root;
    let compiler = content_id_at(repo, None, false).await?;
    let compiled_with = if base.stamp.compiler.dirty {
        None
    } else {
        Some(content_id_at(repo, Some(&base.stamp.compiler.commit), false).await?)
    };
    if compiled_with.as_ref() != Some(&compiler) {
        drift.push("compiler");
    }
    if session.stamp.renderer != base.stamp.renderer {
        drift.push("renderer");
    }
    if session.stamp.fonts != base.stamp.fonts {
        drift.push("fonts");
    }
    if !drift.is_empty() {
        return Err(BenchError::refused(format!(
            "the static run was made under another {}: run the static lane again first",
            drift.join(", ")
        )));
    }
    let mut stamp = base.stamp.clone();
    stamp.lane = Lane::Host;
    stamp.harness = session.stamp.harness.clone();
    // A narrowed check names only the pairs it covers: the run key (and so the folder) is its own.
    let narrowed =
        request.only.is_some() || request.presentations.is_some() || request.frontends.is_some();
    stamp.partial = base.stamp.partial || narrowed;
    if narrowed {
        stamp.corpus.retain(|pair, _| {
            base.rows.keys().any(|id| {
                &id.pair() == pair
                    && pairs
                        .iter()
                        .any(|(p, s)| &p.id == id.presentation() && s.frontend == id.frontend())
                    && request.only.as_ref().is_none_or(|only| only.contains(id))
            })
        });
    }
    stamp.determinism = None;
    stamp.parity = None;
    stamp.perf = None;
    let mut dir = ctx.cache.runs.join(stamp.run_key()?.as_str());
    if request.end_state {
        dir = dir.join("end-state");
    }
    std::fs::create_dir_all(&dir).map_err(BenchError::io(&dir))?;
    let corpus = Corpus::new(
        ctx.env.require_corpus_dir()?,
        FileHashes::new(&ctx.cache.hashes, true),
        &manifest,
    );

    let mut records: BTreeMap<RowId, RowRecord> = BTreeMap::new();
    let mut failures: Vec<(RowId, String)> = Vec::new();
    for (presentation, source) in &pairs {
        let pair = format!("{}.{}", presentation.id, source.frontend);
        let rows: Vec<(&RowId, &RowRecord)> = base
            .rows
            .iter()
            .filter(|(id, _)| {
                id.presentation() == &presentation.id
                    && id.frontend() == source.frontend
                    && request.only.as_ref().is_none_or(|only| only.contains(id))
            })
            .collect();
        if rows.is_empty() {
            continue;
        }
        let file = corpus.source_file(source)?.ok_or_else(|| {
            BenchError::failed(format!(
                "{}: {} is not in the corpus",
                presentation.id, source.file
            ))
        })?;
        let bundle = dir.join("bundles").join(&pair);
        remove(&bundle)?;
        if let Some(parent) = bundle.parent() {
            std::fs::create_dir_all(parent).map_err(BenchError::io(parent))?;
        }
        let arguments: Vec<String> = vec![
            "compile".into(),
            file.to_string_lossy().into_owned(),
            "--frontend".into(),
            source.frontend.to_string(),
            "--out".into(),
            bundle.to_string_lossy().into_owned(),
            "--fonts".into(),
            ctx.cache.fonts.to_string_lossy().into_owned(),
            "--fonts-off".into(),
        ];
        let outcome = run_compiler(&binary, &arguments, COMPILE_TIMEOUT, true).await?;
        let compiled = read_pair_compile(
            &static_dir,
            presentation.id.as_str(),
            source.frontend.as_str(),
        );
        eprintln!("{pair}: bundle {:?}, {} rows", outcome.status, rows.len());
        let size = compiled.size;
        let bundled = matches!(outcome.status, CompileStatus::Ok | CompileStatus::Errors)
            && bundle.join("index.html").is_file();
        let router: Arc<DeckRouter> = Arc::new(DeckRouter {
            dir: bundle.clone(),
            gif_frames: ctx.cache.gif_frames.clone(),
        });
        let browser = session.browser.clone();
        let checked: Vec<Checked> = futures::stream::iter(rows.iter())
            .map(|(id, record)| {
                let router = router.clone();
                let browser = browser.clone();
                let standalone = row_paths(&static_dir, id).render;
                async move {
                    let check = match (bundled, size, &record.render) {
                        (false, _, _) => {
                            Err(format!("the deck did not bundle ({:?})", outcome.status))
                        }
                        (_, None, _) => Err("the static compile has no slide size".to_owned()),
                        (_, _, None) => Err("not rendered by the static lane".to_owned()),
                        (true, Some((w, h)), Some(_)) => {
                            let request = MountRequest {
                                url: deck_url(),
                                slide: id.slide().as_str().to_owned(),
                                size: (w, h),
                                px_per_pt: f64::from(slidock_bench_metrics::RASTER_LONG_SIDE_PX)
                                    / w.max(h),
                                end_state: request.end_state,
                                router,
                            };
                            check_row(&browser, request, record, &standalone).await
                        }
                    };
                    Checked {
                        id: (*id).clone(),
                        check,
                    }
                }
            })
            .buffer_unordered(request.concurrency.max(1))
            .collect()
            .await;
        for Checked { id, check } in checked {
            let Some(record) = base.rows.get(&id) else {
                continue;
            };
            let mut record = record.clone();
            match check {
                Ok((host, png)) => {
                    write_atomic(&mounted_path(&dir, &id), &png)?;
                    if host.status == FrameMatch::Differs {
                        eprintln!(
                            "{id}: mounted render differs (ssim {})",
                            host.ssim
                                .map_or_else(|| "–".into(), slidock_canon::to_js_string)
                        );
                    }
                    record.host = Some(host);
                }
                Err(why) => {
                    if record.render.is_some() {
                        eprintln!("{id}: {why}");
                        failures.push((id.clone(), why));
                    }
                }
            }
            records.insert(id, record);
        }
    }
    let mut summary = summary(&records);
    let lanes = host_summary(&records);
    summary.lanes = Some(LaneSummaries {
        host: Some(lanes.clone()),
        ..LaneSummaries::default()
    });
    let results = Results {
        schema: ResultsSchema::Value,
        stamp,
        summary,
        rows: records,
        holdout: None,
        timelines: None,
        font_pairs: None,
        shell_pairs: None,
        motion_pairs: None,
        positions: None,
    };
    let path = results_path(&dir);
    write_results(&path, &results)?;
    let mut differs = 0;
    for (frontend, s) in &lanes {
        println!(
            "{frontend}: {} rows mounted: {} identical, {} differ",
            s.rows, s.identical, s.differs
        );
        differs += s.differs;
    }
    if !failures.is_empty() {
        println!("{} rows could not be mounted", failures.len());
    }
    println!("results: {}", path.display());
    Ok(Exit::check(differs == 0 && failures.is_empty()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::record;
    use slidock_bench_schema::results::Outcome;

    #[test]
    fn the_summary_counts_checked_rows_per_frontend() {
        let mut a = record("a", Outcome::Match, Some(0.99));
        let mut b = record("b", Outcome::Match, Some(0.99));
        let c = record("c", Outcome::Declined, None);
        let check = |status| HostCheck {
            status,
            standalone: Sha256::of("x"),
            mounted: Sha256::of("y"),
            ssim: None,
        };
        a.host = Some(check(FrameMatch::Identical));
        b.host = Some(check(FrameMatch::Differs));
        let rows: BTreeMap<RowId, RowRecord> = [a, b, c].into_iter().map(|r| (r.id(), r)).collect();
        let summary = host_summary(&rows);
        assert_eq!(
            summary.get(&Frontend::Pptx),
            Some(&FrameSummary {
                rows: 2,
                identical: 1,
                differs: 1,
                antialiased: 0,
            })
        );
    }
}
