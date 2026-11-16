//! `ui` serves the bench UI over the private bench data (see [`crate::server`]); `report --single-file` writes the
//! UI's single-file build with the data inlined, for reading without the server.

use std::collections::BTreeMap;
use std::path::PathBuf;

use clap::Args;
use serde::Serialize;
use slidock_bench_schema::api::{HeadIds, ImageKind, ImageRef, ParityView, RowDetail, Status};
use slidock_bench_schema::results::{Outcome, Results};
use slidock_bench_schema::runs::RunsIndex;

use crate::cache::write_atomic;
use crate::corpus::load_manifest;
use crate::error::{BenchError, Exit, Result};
use crate::server::{ServerData, runs_index, serve};
use crate::setup::Context;

#[derive(Debug, Args)]
pub struct UiArgs {
    #[arg(long, default_value_t = 5290)]
    port: u16,
    /// The built bench UI [default: apps/bench-web/dist].
    #[arg(long)]
    dist: Option<PathBuf>,
}

#[derive(Debug, Args)]
pub struct ReportArgs {
    /// One self-contained HTML file (the only form).
    #[arg(long)]
    single_file: bool,
    /// Where to write it [default: <cache>/report/index.html].
    #[arg(long)]
    out: Option<PathBuf>,
    /// The UI's empty report page [default: apps/bench-web/dist/report/index.html].
    #[arg(long)]
    template: Option<PathBuf>,
    /// The runs it carries: `latest`, `baseline` or run keys.
    #[arg(long, default_value = "latest,baseline")]
    runs: String,
    /// Inlines the oracle, render and heatmap of rows of these classes, at the metric size (e.g. `mismatch`).
    #[arg(long)]
    images: Option<String>,
}

async fn server_data(ctx: &Context, dist: PathBuf, port: u16) -> Result<ServerData> {
    let repo = &ctx.env.repo_root;
    let head = HeadIds {
        commit: crate::git::head_commit(repo).await?,
        content: crate::stamp::compiler_stamp(repo).await?.content,
        harness: crate::stamp::harness_hash(repo, None).await?,
    };
    Ok(ServerData::new(
        ctx.cache.runs.clone(),
        ctx.env.baseline_dir.clone(),
        ctx.cache.root.clone(),
        ctx.env.bench_dir.clone(),
        ctx.env.corpus_dir.clone(),
        dist,
        load_manifest(&ctx.env.manifest_path)?,
        head,
        crate::commands::parity::current_reference_dir(ctx),
        port,
    ))
}

pub async fn ui(ctx: &Context, args: UiArgs) -> Result<Exit> {
    let dist = args
        .dist
        .unwrap_or_else(|| ctx.env.repo_root.join("apps/bench-web/dist"));
    if !dist.join("index.html").is_file() {
        return Err(BenchError::refused(format!(
            "{} holds no built bench UI: run `pnpm --filter @slidock/bench-web build`, or pass --dist",
            dist.display()
        )));
    }
    serve(server_data(ctx, dist, args.port).await?).await?;
    Ok(Exit::Ok)
}

/// `slidock-bench-report@1`, what the single-file page reads (`apps/bench-web/src/data/report.ts`).
#[derive(Serialize)]
struct Payload {
    schema: &'static str,
    status: Option<Status>,
    runs: RunsIndex,
    results: BTreeMap<String, Results>,
    parity: BTreeMap<String, ParityView>,
    rows: BTreeMap<String, RowDetail>,
    images: BTreeMap<String, String>,
}

/// The images a report can inline, at the metric size.
const INLINED: [(ImageKind, &str); 3] = [
    (ImageKind::Oracle, "oracle"),
    (ImageKind::Render, "render"),
    (ImageKind::Heatmap, "heatmap"),
];

/// The page's empty data slot.
const DATA_SLOT: &str =
    "<script type=\"application/json\" id=\"slidock-bench-data\">__SLIDOCK_BENCH_DATA__</script>";

/// JSON that is safe inside a `<script>` element.
fn script_safe(json: &str) -> String {
    json.replace('<', "\\u003c")
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029")
}

/// Fills the page's one empty slot with `json`.
pub fn inject(page: &str, json: &str) -> Result<String> {
    let mut found = page.match_indices(DATA_SLOT);
    let (Some((at, _)), None) = (found.next(), found.next()) else {
        return Err(BenchError::failed(
            "the report page must hold exactly one empty slidock-bench-data slot",
        ));
    };
    let filled = DATA_SLOT.replace("__SLIDOCK_BENCH_DATA__", &script_safe(json));
    let mut out = String::with_capacity(page.len() + filled.len());
    out.push_str(page.get(..at).unwrap_or_default());
    out.push_str(&filled);
    out.push_str(page.get(at + DATA_SLOT.len()..).unwrap_or_default());
    Ok(out)
}

/// A PNG at the metric size, as a data URI.
fn metric_data_uri(png: &[u8]) -> Option<String> {
    use base64::Engine;
    let image = slidock_bench_metrics::decode_png(png).ok()?;
    let (w, h) = slidock_bench_metrics::metric_size(image.width(), image.height());
    let small =
        slidock_bench_metrics::resample_area(&slidock_bench_metrics::over_white(&image), w, h)
            .ok()?;
    let encoded = crate::explain::encode_png(&small).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(encoded)
    ))
}

pub async fn report(ctx: &Context, args: ReportArgs) -> Result<Exit> {
    if !args.single_file {
        return Err(BenchError::usage("report writes --single-file pages only"));
    }
    let template = args.template.unwrap_or_else(|| {
        ctx.env
            .repo_root
            .join("apps/bench-web/dist/report/index.html")
    });
    let page = std::fs::read_to_string(&template).map_err(BenchError::io(&template))?;
    let image_classes: Vec<Outcome> = crate::setup::list(args.images.as_deref())
        .unwrap_or_default()
        .iter()
        .map(|c| {
            serde_json::from_value(serde_json::Value::String(c.clone()))
                .map_err(|_| BenchError::usage(format!("unknown class {c}")))
        })
        .collect::<Result<_>>()?;
    let data = server_data(ctx, PathBuf::new(), 0).await?;
    let wanted = crate::setup::list(Some(&args.runs)).unwrap_or_default();
    let payload = tokio::task::spawn_blocking(move || -> Result<Payload> {
        let index = runs_index(&data)?;
        let mut keys: Vec<String> = Vec::new();
        for run in &wanted {
            let key = match run.as_str() {
                "latest" => index
                    .runs
                    .iter()
                    .rev()
                    .find(|r| r.lane == slidock_bench_schema::Lane::Static)
                    .map(|r| r.key.as_str().to_owned()),
                "baseline" => index
                    .runs
                    .iter()
                    .rev()
                    .find(|r| r.baseline.is_some())
                    .map(|r| r.key.as_str().to_owned()),
                key => Some(key.to_owned()),
            };
            if let Some(key) = key
                && !keys.contains(&key)
            {
                keys.push(key);
            }
        }
        let mut payload = Payload {
            schema: "slidock-bench-report@1",
            status: Some(crate::server::status_for_report(&data)),
            runs: RunsIndex {
                schema: index.schema,
                runs: index
                    .runs
                    .iter()
                    .filter(|r| keys.contains(&r.key.as_str().to_owned()))
                    .cloned()
                    .collect(),
            },
            results: BTreeMap::new(),
            parity: BTreeMap::new(),
            rows: BTreeMap::new(),
            images: BTreeMap::new(),
        };
        for key in &keys {
            let Some(results) = crate::server::public_results(&data, key) else {
                return Err(BenchError::refused(format!("no run {key}")));
            };
            if let Some(view) = crate::server::public_parity(&data, key) {
                payload.parity.insert(key.clone(), view);
            }
            for (id, record) in &results.rows {
                let class = record.outcome.classifier();
                let inline = class.is_some_and(|c| image_classes.contains(&c));
                let mut uris = BTreeMap::new();
                if inline {
                    for (kind, name) in INLINED {
                        if let Some(uri) = crate::server::public_png(&data, key, id, name)
                            .as_deref()
                            .and_then(metric_data_uri)
                        {
                            // The row's detail names each picture; the index holds the render too, which the
                            // drawer shows as another run's base render.
                            if kind == ImageKind::Render {
                                payload
                                    .images
                                    .insert(format!("{key} {id} {name}"), uri.clone());
                            }
                            uris.insert(kind, uri);
                        }
                    }
                }
                if let Ok(Some(mut detail)) = crate::server::public_row_detail(&data, key, id) {
                    // The page has no API: an image is the inlined one, or the command that shows it.
                    for (kind, image) in &mut detail.images {
                        *image = match (uris.get(kind), &*image) {
                            (Some(uri), ImageRef::Found { from, run, .. }) => ImageRef::Found {
                                url: uri.clone(),
                                from: *from,
                                run: run.clone(),
                            },
                            (_, ImageRef::Missing { missing }) => ImageRef::Missing {
                                missing: missing.clone(),
                            },
                            _ => ImageRef::Missing {
                                missing: "slidock-bench ui".to_owned(),
                            },
                        };
                    }
                    payload.rows.insert(format!("{key} {id}"), detail);
                }
            }
            payload.results.insert(key.clone(), results);
        }
        Ok(payload)
    })
    .await
    .map_err(|e| BenchError::failed(e.to_string()))??;
    let json = serde_json::to_string(&payload).map_err(|e| BenchError::failed(e.to_string()))?;
    let out = args
        .out
        .unwrap_or_else(|| ctx.cache.root.join("report/index.html"));
    write_atomic(&out, inject(&page, &json)?)?;
    println!(
        "report: {} ({} runs, {} rows, {} images, {} bytes)",
        out.display(),
        payload.runs.runs.len(),
        payload.rows.len(),
        payload.images.len(),
        std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0)
    );
    Ok(Exit::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_slot_is_filled_once_and_script_safe() {
        let page = format!("<head>{DATA_SLOT}</head>");
        let filled = inject(&page, "{\"a\":\"</script>\"}").unwrap();
        assert!(filled.contains("{\"a\":\"\\u003c/script>\"}"));
        assert!(!filled.contains("__SLIDOCK_BENCH_DATA__"));
        assert!(inject("<head></head>", "{}").is_err());
        assert!(inject(&format!("{DATA_SLOT}{DATA_SLOT}"), "{}").is_err());
    }
}
