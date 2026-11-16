//! `ui` serves the bench UI over the private bench data (see [`crate::server`]); `report --single-file` writes the
//! UI's single-file build with the data inlined, for reading without the server, and `report --public` the same page
//! over the presentations cleared for publication, with its images beside it (see [`crate::public`]).

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use clap::Args;
use serde::Serialize;
use slidock_bench_schema::api::{HeadIds, ImageKind, ImageRef, ParityView, RowDetail, Status};
use slidock_bench_schema::results::{Outcome, Results};
use slidock_bench_schema::runs::RunsIndex;

use crate::cache::write_atomic;
use crate::corpus::load_manifest;
use crate::error::{BenchError, Exit, Result};
use crate::exec::{ExecOptions, args as exec_args, exec, exec_text};
use crate::public::{self, Forbidden, PublicScope};
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
    /// One self-contained HTML file.
    #[arg(long, conflicts_with = "public")]
    single_file: bool,
    /// The public report: the presentations of `bench/public.json` only, its images as WebP files beside the page.
    #[arg(long)]
    public: bool,
    /// Where to write it [default: <cache>/report/index.html; with --public, the folder apps/web/public/bench].
    #[arg(long)]
    out: Option<PathBuf>,
    /// The UI's empty report page [default: apps/bench-web/dist/report/index.html].
    #[arg(long)]
    template: Option<PathBuf>,
    /// The runs it carries: `latest`, `baseline` or run keys [default: latest,baseline; with --public, the one run
    /// latest].
    #[arg(long)]
    runs: Option<String>,
    /// The oracle, render and heatmap of rows of these classes, at the metric size (e.g. `mismatch`) [default: none;
    /// with --public, mismatch].
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
    #[serde(skip_serializing_if = "Option::is_none")]
    public: Option<PublicScope>,
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

/// A PNG at the metric size.
fn metric_png(png: &[u8]) -> Option<Vec<u8>> {
    let image = slidock_bench_metrics::decode_png(png).ok()?;
    let (w, h) = slidock_bench_metrics::metric_size(image.width(), image.height());
    let small =
        slidock_bench_metrics::resample_area(&slidock_bench_metrics::over_white(&image), w, h)
            .ok()?;
    crate::explain::encode_png(&small).ok()
}

/// A PNG at the metric size, as a data URI.
fn metric_data_uri(png: &[u8]) -> Option<String> {
    use base64::Engine;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(metric_png(png)?)
    ))
}

fn image_classes(list: Option<&str>) -> Result<Vec<Outcome>> {
    crate::setup::list(list)
        .unwrap_or_default()
        .iter()
        .map(|c| {
            serde_json::from_value(serde_json::Value::String(c.clone()))
                .map_err(|_| BenchError::usage(format!("unknown class {c}")))
        })
        .collect()
}

/// The run keys `latest`, `baseline` and keys name, each once, in order.
fn run_keys(index: &RunsIndex, wanted: &[String]) -> Vec<String> {
    let mut keys: Vec<String> = Vec::new();
    for run in wanted {
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
    keys
}

fn read_template(ctx: &Context, template: Option<PathBuf>) -> Result<String> {
    let template = template.unwrap_or_else(|| {
        ctx.env
            .repo_root
            .join("apps/bench-web/dist/report/index.html")
    });
    std::fs::read_to_string(&template).map_err(BenchError::io(&template))
}

pub async fn report(ctx: &Context, args: ReportArgs) -> Result<Exit> {
    if args.public {
        return public_report(ctx, args).await;
    }
    if !args.single_file {
        return Err(BenchError::usage(
            "report writes --single-file or --public pages",
        ));
    }
    let page = read_template(ctx, args.template)?;
    let image_classes = image_classes(args.images.as_deref())?;
    let data = server_data(ctx, PathBuf::new(), 0).await?;
    let wanted = crate::setup::list(Some(args.runs.as_deref().unwrap_or("latest,baseline")))
        .unwrap_or_default();
    let payload = tokio::task::spawn_blocking(move || -> Result<Payload> {
        let index = runs_index(&data)?;
        let keys = run_keys(&index, &wanted);
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
            public: None,
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

/// The images of a public report, as metric-size PNGs by their path in the report.
type Pictures = Vec<(String, Vec<u8>)>;

/// A public report's payload as built, before [`public::strip`]; its images; and every run key of the bench.
type PublicBuild = (Payload, Pictures, Vec<String>);

/// The quality the public report's WebP images are encoded at (`cwebp -q`).
const WEBP_QUALITY: &str = "85";

/// A PNG as lossy WebP, with libwebp's `cwebp`.
async fn webp(png: Vec<u8>) -> Result<Vec<u8>> {
    let options = ExecOptions {
        input: Some(png),
        timeout: Some(std::time::Duration::from_secs(60)),
        ..ExecOptions::default()
    };
    let arguments = exec_args([
        "-quiet",
        "-q",
        WEBP_QUALITY,
        "-m",
        "6",
        "-metadata",
        "none",
        "-o",
        "-",
        "--",
        "-",
    ]);
    let result = exec("cwebp", &arguments, &options).await?;
    if !result.ok() || result.stdout.is_empty() {
        return Err(BenchError::failed(format!(
            "cwebp failed: {}",
            result.stderr.trim()
        )));
    }
    Ok(result.stdout)
}

/// `report --public`: see [`crate::public`]. Nothing is written when the payload fails [`public::check_payload`].
async fn public_report(ctx: &Context, args: ReportArgs) -> Result<Exit> {
    let repo = ctx.env.repo_root.clone();
    let allow = public::read_allowlist(&repo.join(public::ALLOWLIST))?;
    let page = read_template(ctx, args.template)?;
    if !page.contains("http-equiv=\"Content-Security-Policy\"") {
        return Err(BenchError::refused(
            "the report page has no Content Security Policy: rebuild it with `pnpm --filter @slidock/bench-web build`",
        ));
    }
    let classes = image_classes(Some(args.images.as_deref().unwrap_or("mismatch")))?;
    let out = args.out.unwrap_or_else(|| repo.join(public::PUBLIC_DIR));
    public::check_out_dir(&out)?;
    let cwebp = exec_text(
        "cwebp",
        &exec_args(["-version"]),
        &ExecOptions::timeout(std::time::Duration::from_secs(10)),
    )
    .await
    .map_err(|_| {
        BenchError::refused(
            "the public report encodes its images with libwebp's cwebp, which is not on PATH",
        )
    })?;
    let data = server_data(ctx, PathBuf::new(), 0).await?;
    let manifest = data.manifest.clone();
    let wanted =
        crate::setup::list(Some(args.runs.as_deref().unwrap_or("latest"))).unwrap_or_default();
    let note = format!(
        "The public report includes images of {} rows only.",
        classes
            .iter()
            .map(|c| serde_json::to_value(c)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default())
            .collect::<Vec<_>>()
            .join(", ")
    );
    let scope = allow.clone();
    let wanted_classes = classes.clone();
    let (payload, pictures, all_keys) =
        tokio::task::spawn_blocking(move || -> Result<PublicBuild> {
            let index = runs_index(&data)?;
            let keys = run_keys(&index, &wanted);
            if keys.len() != 1 {
                return Err(BenchError::usage(format!(
                    "the public report publishes one run, and --runs names {}",
                    keys.len()
                )));
            }
            let mut payload = Payload {
                schema: "slidock-bench-report@1",
                status: None,
                runs: RunsIndex {
                    schema: index.schema,
                    runs: Vec::new(),
                },
                results: BTreeMap::new(),
                parity: BTreeMap::new(),
                rows: BTreeMap::new(),
                images: BTreeMap::new(),
                public: Some(PublicScope {
                    images: wanted_classes.clone(),
                }),
            };
            let mut pictures = Vec::new();
            for key in &keys {
                let (Some(entry), Some(results)) = (
                    index.runs.iter().find(|r| r.key.as_str() == key),
                    crate::server::public_results(&data, key),
                ) else {
                    return Err(BenchError::refused(format!("no run {key}")));
                };
                let results = public::scope_results(results, &scope)?;
                payload
                    .runs
                    .runs
                    .push(public::scope_entry(entry.clone(), &results));
                for (id, record) in &results.rows {
                    let mut published = BTreeMap::new();
                    if record
                        .outcome
                        .classifier()
                        .is_some_and(|c| wanted_classes.contains(&c))
                    {
                        for (kind, name) in INLINED {
                            let Some(png) = crate::server::public_png(&data, key, id, name)
                                .as_deref()
                                .and_then(metric_png)
                            else {
                                continue;
                            };
                            let path = public::image_path(id, name);
                            if kind == ImageKind::Render {
                                payload
                                    .images
                                    .insert(format!("{} {id} {name}", public::RUN), path.clone());
                            }
                            published.insert(kind, path.clone());
                            pictures.push((path, png));
                        }
                    }
                    if let Ok(Some(detail)) = crate::server::public_row_detail(&data, key, id) {
                        payload.rows.insert(
                            format!("{} {id}", public::RUN),
                            public::scope_detail(detail, &published, &note),
                        );
                    }
                }
                payload.results.insert(public::RUN.to_owned(), results);
            }
            let all_keys = index
                .runs
                .iter()
                .map(|r| r.key.as_str().to_owned())
                .collect();
            Ok((payload, pictures, all_keys))
        })
        .await
        .map_err(|e| BenchError::failed(e.to_string()))??;

    let mut value =
        serde_json::to_value(&payload).map_err(|e| BenchError::failed(e.to_string()))?;
    public::strip(&mut value);
    let mut forbidden = Forbidden::generic()
        .repository(&repo, &allow)?
        .manifest(&manifest, &allow)
        .run_keys(all_keys.iter().map(String::as_str))
        .text(ctx.cache.root.to_string_lossy())
        .text(ctx.env.corpus_dir.to_string_lossy());
    for name in ["HOME", "USER"] {
        if let Ok(value) = std::env::var(name) {
            forbidden = forbidden.text(value);
        }
    }
    let mut problems = public::check_payload(&value, &allow, &forbidden);
    let encoded_paths: BTreeSet<String> = pictures.iter().map(|(path, _)| path.clone()).collect();
    if public::image_paths(&value) != encoded_paths {
        problems.push("the payload names other images than the ones the report writes".to_owned());
    }
    if !problems.is_empty() {
        for problem in &problems {
            eprintln!("  {problem}");
        }
        return Err(BenchError::failed(format!(
            "the public report would publish what it must not ({} findings): nothing written",
            problems.len()
        )));
    }

    let mut encoded = Vec::with_capacity(pictures.len());
    for (path, png) in pictures {
        encoded.push((path, webp(png).await?));
    }
    let images = out.join(public::IMAGES_DIR);
    if images.is_dir() {
        std::fs::remove_dir_all(&images).map_err(BenchError::io(&images))?;
    }
    let mut image_bytes = 0;
    for (path, bytes) in &encoded {
        image_bytes += bytes.len();
        write_atomic(&out.join(path), bytes)?;
    }
    let json = serde_json::to_string(&value).map_err(|e| BenchError::failed(e.to_string()))?;
    let index = out.join("index.html");
    write_atomic(&index, inject(&page, &json)?)?;
    let rows: usize = payload.results.values().map(|r| r.rows.len()).sum();
    let decks: BTreeSet<_> = payload
        .results
        .values()
        .flat_map(|r| r.rows.keys().map(|id| id.presentation().clone()))
        .collect();
    println!(
        "public report: {} ({} runs, {} decks, {rows} rows, {} images of {image_bytes} bytes with {}, page {} bytes)",
        out.display(),
        payload.runs.runs.len(),
        decks.len(),
        encoded.len(),
        cwebp.lines().next().unwrap_or_default().trim(),
        std::fs::metadata(&index).map(|m| m.len()).unwrap_or(0)
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
