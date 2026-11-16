//! The wasm leg: the app's production build (`apps/web/dist`) served on a loopback port, and a page in the host's
//! headless shell that starts the app's own compile worker (`assets/compile-worker-*.js`) and compiles the corpus
//! file as the app does. The file reaches the page as a `File` (`DOM.setFileInputFiles`), which the worker reads by
//! ranges, never whole; the page adds the bundled substitutes from the app's `fonts/core/` as the app does. The
//! compile's time is the CPU time of the headless shell's renderer processes (the page and its worker) from the
//! moment the worker is ready to the zip, which the machine's load moves far less than the wall clock.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use axum::Router;
use axum::http::{StatusCode, Uri, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::get;
use chromiumoxide::cdp::browser_protocol::dom::SetFileInputFilesParams;
use chromiumoxide::cdp::js_protocol::runtime::EvaluateParams;
use chromiumoxide::handler::HandlerConfig;
use futures::StreamExt;
use serde::Deserialize;
use slidock_bench_schema::stamp::CompileCost;
use tokio::task::JoinHandle;

use crate::error::{BenchError, Result};
use crate::renderer::native::NativeBrowser;

const HARNESS_JS: &str = include_str!("harness.js");

const HARNESS_HTML: &str = "<!doctype html><html><head><meta charset=\"utf-8\"><title>slidock budgets</title>\
</head><body><input type=\"file\" id=\"deck\"></body></html>";

/// The page the leg opens, beside the app's files.
const HARNESS_PATH: &str = "/__slidock-budgets";

/// The renderer's resolver rules with the loopback address let through: the page reaches the served app and
/// nothing else.
const LOOPBACK_ONLY: &str = "--host-resolver-rules=MAP * ~NOTFOUND, EXCLUDE 127.0.0.1";

/// How often the page's outcome is read while a compile runs.
const POLL: Duration = Duration::from_millis(250);

/// The app's build as the leg serves it.
#[derive(Clone, Debug)]
pub struct AppBuild {
    pub dist: PathBuf,
    /// The compile worker's URL path (`/assets/compile-worker-<hash>.js`).
    pub worker: String,
}

/// The one file of `assets` whose name starts with `prefix` and ends with `suffix`.
fn one_asset(assets: &Path, prefix: &str, suffix: &str) -> Result<PathBuf> {
    let entries = std::fs::read_dir(assets).map_err(BenchError::io(assets))?;
    let mut found: Vec<PathBuf> = Vec::new();
    for entry in entries {
        let entry = entry.map_err(BenchError::io(assets))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with(prefix) && name.ends_with(suffix) {
            found.push(entry.path());
        }
    }
    match found.as_slice() {
        [one] => Ok(one.clone()),
        [] => Err(BenchError::refused(format!(
            "{} holds no {prefix}*{suffix}: run `pnpm --filter @slidock/web build`",
            assets.display()
        ))),
        _ => Err(BenchError::refused(format!(
            "{} holds {} files {prefix}*{suffix}: rebuild it from clean (`rm -rf apps/web/dist`)",
            assets.display(),
            found.len()
        ))),
    }
}

/// The app's build in `dist`, when its wasm module is the measured one (`wasm_sha256`): a build of another module
/// would measure a compiler other than the stamped one.
pub fn app_build(dist: &Path, wasm_sha256: &str) -> Result<AppBuild> {
    let assets = dist.join("assets");
    let worker = one_asset(&assets, "compile-worker-", ".js")?;
    let module = one_asset(&assets, "slidock_wasm_bg-", ".wasm")?;
    if crate::hash::sha256_file(&module)? != wasm_sha256 {
        return Err(BenchError::refused(format!(
            "{} is not the measured wasm module: rebuild the app (`pnpm --filter @slidock/web build`)",
            module.display()
        )));
    }
    let name = worker
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(AppBuild {
        dist: dist.to_path_buf(),
        worker: format!("/assets/{name}"),
    })
}

/// The content type the app's files are served with; anything else is a plain byte stream.
fn content_type(path: &Path) -> &'static str {
    match path.extension().and_then(|e| e.to_str()) {
        Some("html") => "text/html; charset=utf-8",
        Some("js" | "mjs") => "text/javascript; charset=utf-8",
        Some("css") => "text/css; charset=utf-8",
        Some("wasm") => "application/wasm",
        Some("json") => "application/json",
        Some("svg") => "image/svg+xml",
        Some("png") => "image/png",
        Some("woff2") => "font/woff2",
        Some("ttf") => "font/ttf",
        _ => "application/octet-stream",
    }
}

/// The file under `dist` a request path names: only plain segments, so nothing outside `dist` can be named.
fn file_for(dist: &Path, path: &str) -> Option<PathBuf> {
    let path = path.strip_prefix('/')?;
    if path.is_empty() {
        return Some(dist.join("index.html"));
    }
    let mut file = dist.to_path_buf();
    for segment in path.split('/') {
        if segment.is_empty()
            || segment == "."
            || segment == ".."
            || segment.contains(['\\', '%', '\0'])
        {
            return None;
        }
        file.push(segment);
    }
    Some(file)
}

async fn serve_file(dist: PathBuf, uri: Uri) -> Response {
    let Some(file) = file_for(&dist, uri.path()) else {
        return StatusCode::NOT_FOUND.into_response();
    };
    match tokio::fs::read(&file).await {
        Ok(bytes) => ([(header::CONTENT_TYPE, content_type(&file))], bytes).into_response(),
        Err(_) => StatusCode::NOT_FOUND.into_response(),
    }
}

fn router(dist: PathBuf) -> Router {
    Router::new()
        .route(HARNESS_PATH, get(|| async { Html(HARNESS_HTML) }))
        .fallback(move |uri: Uri| serve_file(dist.clone(), uri))
}

mod log {
    use std::path::Path;

    /// The page's wall clock beside the CPU time the leg records.
    pub fn wall(input: &Path, wall_ms: u64, cpu_ms: u64) {
        eprintln!(
            "{}: wasm {wall_ms} ms wall, {cpu_ms} ms CPU",
            input.display()
        );
    }
}

/// What the page reports.
#[derive(Debug, Deserialize)]
#[serde(tag = "state", rename_all = "lowercase")]
enum Outcome {
    Running,
    Ready,
    Done {
        ms: f64,
        #[serde(rename = "peakBytes")]
        peak_bytes: f64,
    },
    Failed {
        message: String,
    },
}

/// A number the page reported, as a whole: rounded up.
fn whole(value: f64, what: &str) -> Result<u64> {
    if value.is_finite() && (0.0..9_007_199_254_740_992.0).contains(&value) {
        Ok(value.ceil() as u64)
    } else {
        Err(BenchError::failed(format!(
            "the page reported {what} {value}"
        )))
    }
}

/// A wasm compile's cost: the browser's CPU time over the compile, and the memory the page reported.
fn cost_of(cpu_ms: u64, peak_bytes: f64) -> Result<CompileCost> {
    Ok(CompileCost {
        ms: cpu_ms,
        peak_bytes: whole(peak_bytes, "a memory of")?,
    })
}

/// The served app, the headless shell and its CDP connection.
pub struct WasmLeg {
    browser: NativeBrowser,
    cdp: chromiumoxide::Browser,
    handler: JoinHandle<()>,
    server: JoinHandle<()>,
    origin: String,
    worker: String,
    /// The headless shell's version (`148.0.7778.0`).
    pub chromium: String,
}

impl WasmLeg {
    /// Serves `app` on a free loopback port and starts the headless shell `binary` with a fresh profile.
    pub async fn start(
        bench_dir: &Path,
        binary: &Path,
        profile: &Path,
        app: &AppBuild,
    ) -> Result<WasmLeg> {
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .map_err(|error| {
                BenchError::failed(format!("no loopback port for the app: {error}"))
            })?;
        let address: SocketAddr = listener
            .local_addr()
            .map_err(|error| BenchError::failed(format!("the app's port: {error}")))?;
        let app_router = router(app.dist.clone());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app_router).await;
        });
        let browser = match NativeBrowser::start_with(
            bench_dir,
            binary,
            profile,
            &[LOOPBACK_ONLY.to_owned()],
        )
        .await
        {
            Ok(browser) => browser,
            Err(error) => {
                server.abort();
                return Err(error);
            }
        };
        let config = HandlerConfig {
            viewport: None,
            request_timeout: Duration::from_secs(60),
            ..HandlerConfig::default()
        };
        let connected =
            chromiumoxide::Browser::connect_with_config(&browser.ws_endpoint, config).await;
        let (cdp, mut events) = match connected {
            Ok(connected) => connected,
            Err(error) => {
                server.abort();
                let _ = browser.stop().await;
                return Err(BenchError::failed(format!("CDP connect: {error}")));
            }
        };
        let handler = tokio::spawn(async move { while events.next().await.is_some() {} });
        let chromium = match cdp.version().await {
            Ok(version) => version
                .product
                .rsplit('/')
                .next()
                .unwrap_or(&version.product)
                .to_owned(),
            Err(error) => {
                handler.abort();
                server.abort();
                let _ = browser.stop().await;
                return Err(BenchError::failed(format!("Browser.getVersion: {error}")));
            }
        };
        Ok(WasmLeg {
            browser,
            cdp,
            handler,
            server,
            origin: format!("http://{address}"),
            worker: app.worker.clone(),
            chromium,
        })
    }

    /// Compiles `input` once in a fresh page and a fresh worker, within `timeout`.
    pub async fn compile(&self, input: &Path, timeout: Duration) -> Result<CompileCost> {
        let page = self
            .cdp
            .new_page("about:blank")
            .await
            .map_err(|error| BenchError::failed(format!("new page: {error}")))?;
        let outcome = self.compile_in(&page, input, timeout).await;
        let _ = page.close().await;
        outcome
    }

    async fn compile_in(
        &self,
        page: &chromiumoxide::Page,
        input: &Path,
        timeout: Duration,
    ) -> Result<CompileCost> {
        let failed = |what: &str, error: chromiumoxide::error::CdpError| {
            BenchError::failed(format!("{}: {what}: {error}", input.display()))
        };
        page.goto(format!("{}{HARNESS_PATH}", self.origin))
            .await
            .map_err(|e| failed("the page", e))?;
        let field = page
            .find_element("#deck")
            .await
            .map_err(|e| failed("#deck", e))?;
        let mut files = SetFileInputFilesParams::new(vec![input.to_string_lossy().into_owned()]);
        files.backend_node_id = Some(field.backend_node_id);
        page.execute(files)
            .await
            .map_err(|e| failed("the file", e))?;
        let worker = serde_json::to_string(&format!("{}{}", self.origin, self.worker))
            .map_err(|error| BenchError::failed(format!("the worker's URL: {error}")))?;
        page.execute(EvaluateParams::new(format!("({HARNESS_JS})({worker})")))
            .await
            .map_err(|e| failed("starting the compile", e))?;
        let deadline = tokio::time::Instant::now() + timeout;
        let mut started: Option<u64> = None;
        loop {
            let mut read = EvaluateParams::new("globalThis.__budgets");
            read.return_by_value = Some(true);
            let value = page
                .execute(read)
                .await
                .map_err(|e| failed("reading the outcome", e))?
                .result
                .result
                .value
                .clone()
                .unwrap_or_default();
            let outcome: Outcome = serde_json::from_value(value.clone()).map_err(|error| {
                BenchError::failed(format!(
                    "{}: the page reported {value}: {error}",
                    input.display()
                ))
            })?;
            match outcome {
                Outcome::Ready if started.is_none() => {
                    started = Some(self.browser.renderer_cpu_ms().await?);
                    page.execute(EvaluateParams::new("globalThis.__budgetsGo()"))
                        .await
                        .map_err(|e| failed("starting the compile", e))?;
                }
                Outcome::Running | Outcome::Ready if tokio::time::Instant::now() < deadline => {
                    tokio::time::sleep(POLL).await;
                }
                Outcome::Running | Outcome::Ready => {
                    return Err(BenchError::failed(format!(
                        "{}: the wasm compile ran past {} s",
                        input.display(),
                        timeout.as_secs()
                    )));
                }
                Outcome::Done { ms, peak_bytes } => {
                    let Some(begun) = started else {
                        return Err(BenchError::failed(format!(
                            "{}: the page finished before it was started",
                            input.display()
                        )));
                    };
                    let cpu = self.browser.renderer_cpu_ms().await?.saturating_sub(begun);
                    log::wall(input, whole(ms, "a time of")?, cpu);
                    return cost_of(cpu, peak_bytes);
                }
                Outcome::Failed { message } => {
                    return Err(BenchError::failed(format!(
                        "{}: the wasm compile failed: {message}",
                        input.display()
                    )));
                }
            }
        }
    }

    /// Stops the browser and the server.
    pub async fn stop(self) -> Result<()> {
        self.handler.abort();
        self.server.abort();
        drop(self.cdp);
        self.browser.stop().await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_name_only_files_under_the_build() {
        let dist = Path::new("/d");
        assert_eq!(file_for(dist, "/"), Some(PathBuf::from("/d/index.html")));
        assert_eq!(
            file_for(dist, "/assets/compile-worker-x.js"),
            Some(PathBuf::from("/d/assets/compile-worker-x.js"))
        );
        for path in [
            "/../etc/passwd",
            "/assets/../../x",
            "/./a",
            "/a//b",
            "/a%2e%2e/b",
            "/a\\b",
            "relative",
        ] {
            assert_eq!(file_for(dist, path), None, "{path}");
        }
    }

    #[test]
    fn the_app_is_served_with_the_types_a_module_worker_and_streaming_wasm_need() {
        assert_eq!(content_type(Path::new("a.wasm")), "application/wasm");
        assert_eq!(
            content_type(Path::new("a.js")),
            "text/javascript; charset=utf-8"
        );
        assert_eq!(
            content_type(Path::new("index.html")),
            "text/html; charset=utf-8"
        );
        assert_eq!(content_type(Path::new("x.bin")), "application/octet-stream");
    }

    #[test]
    fn the_build_must_hold_one_worker_and_the_measured_module() {
        let dir = tempfile::tempdir().unwrap();
        let assets = dir.path().join("assets");
        std::fs::create_dir_all(&assets).unwrap();
        std::fs::write(assets.join("slidock_wasm_bg-a.wasm"), b"module").unwrap();
        let sha = crate::hash::sha256(b"module");
        assert!(
            app_build(dir.path(), &sha)
                .unwrap_err()
                .to_string()
                .contains("compile-worker-")
        );
        std::fs::write(assets.join("compile-worker-1.js"), b"").unwrap();
        let app = app_build(dir.path(), &sha).unwrap();
        assert_eq!(app.worker, "/assets/compile-worker-1.js");
        // Another module than the one measured.
        assert!(app_build(dir.path(), &crate::hash::sha256(b"other")).is_err());
        std::fs::write(assets.join("compile-worker-2.js"), b"").unwrap();
        assert!(
            app_build(dir.path(), &sha)
                .unwrap_err()
                .to_string()
                .contains("2 files")
        );
    }

    #[test]
    fn the_pages_numbers_become_whole_milliseconds_and_bytes() {
        assert_eq!(whole(1234.2, "t").unwrap(), 1235);
        assert_eq!(
            cost_of(7, 65536.0).unwrap(),
            CompileCost {
                ms: 7,
                peak_bytes: 65536
            }
        );
        for value in [f64::NAN, -1.0, f64::INFINITY, 1e300] {
            assert!(whole(value, "t").is_err());
            assert!(cost_of(1, value).is_err());
        }
    }

    #[test]
    fn the_pages_outcome_is_running_ready_done_or_failed() {
        let parse = |text: &str| serde_json::from_str::<Outcome>(text);
        assert!(matches!(
            parse(r#"{"state":"running"}"#),
            Ok(Outcome::Running)
        ));
        assert!(matches!(parse(r#"{"state":"ready"}"#), Ok(Outcome::Ready)));
        assert!(matches!(
            parse(r#"{"state":"done","ms":5.5,"peakBytes":131072}"#),
            Ok(Outcome::Done { .. })
        ));
        assert!(matches!(
            parse(r#"{"state":"failed","message":"x"}"#),
            Ok(Outcome::Failed { .. })
        ));
        assert!(parse(r#"{"state":"paused"}"#).is_err());
        assert!(parse("null").is_err());
    }

    #[tokio::test]
    async fn the_server_answers_the_page_and_the_build() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("assets")).unwrap();
        std::fs::write(dir.path().join("assets/m.wasm"), b"\0asm").unwrap();
        let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let address = listener.local_addr().unwrap();
        let app = router(dir.path().to_path_buf());
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        let get = |path: &str| {
            let url = format!("http://{address}{path}");
            tokio::task::spawn_blocking(move || ureq::get(&url).call())
        };
        let page = get(HARNESS_PATH).await.unwrap().unwrap();
        assert!(
            page.headers()
                .get("content-type")
                .unwrap()
                .to_str()
                .unwrap()
                .starts_with("text/html")
        );
        let module = get("/assets/m.wasm").await.unwrap().unwrap();
        assert_eq!(
            module.headers().get("content-type").unwrap(),
            "application/wasm"
        );
        let missing = get("/assets/none.js").await.unwrap();
        assert!(missing.is_err());
        server.abort();
    }
}
