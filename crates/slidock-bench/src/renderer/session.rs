//! The host side of the renderer: a CDP client connected to the container's headless
//! shell. Each slide renders in a fresh browser context set up as Playwright 1.60's `newContext` and `newPage` set one
//! up, and is captured as its `screenshot` captures: the bytes then match a Playwright render's. The browser
//! runs in the container; this process routes its requests and reads its screenshots.

use std::sync::Arc;
use std::time::Duration;

use base64::Engine;
use chromiumoxide::Page;
use chromiumoxide::cdp::browser_protocol::emulation::{
    MediaFeature, ScreenOrientation, ScreenOrientationType, SetDeviceMetricsOverrideParams,
    SetEmulatedMediaParams, SetFocusEmulationEnabledParams, SetLocaleOverrideParams,
    SetScriptExecutionDisabledParams, SetTimezoneOverrideParams, SetUserAgentOverrideParams,
};
use chromiumoxide::cdp::browser_protocol::fetch::{
    self, EventRequestPaused, FailRequestParams, FulfillRequestParams, HeaderEntry, RequestPattern,
    RequestStage,
};
use chromiumoxide::cdp::browser_protocol::network::{self, ErrorReason};
use chromiumoxide::cdp::browser_protocol::page::{
    AddScriptToEvaluateOnNewDocumentParams, CaptureScreenshotFormat, CaptureScreenshotParams,
    CreateIsolatedWorldParams, EventLoadEventFired, FontFamilies, GetLayoutMetricsParams,
    NavigateParams, SetFontFamiliesParams, Viewport,
};
use chromiumoxide::cdp::browser_protocol::target::{
    CreateBrowserContextParams, CreateTargetParams,
};
use chromiumoxide::cdp::browser_protocol::{css, dom};
use chromiumoxide::cdp::js_protocol::runtime::{EvaluateParams, ExecutionContextId};
use chromiumoxide::handler::HandlerConfig;
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use slidock_format::markers::{FAILED, FOREIGN_MARKER};
use tokio::task::JoinHandle;

use super::shadow_font::shadow_font;
use crate::error::{BenchError, Result};

/// The CDP client, stamped as `renderer.cdpClient`.
pub const CDP_CLIENT: &str = "chromiumoxide@0.9.1";

/// Rendering stops after this long per row.
pub const RENDER_TIMEOUT: Duration = Duration::from_secs(30);

/// Paints every glyph transparent and keeps everything else: the slide with its text hidden.
const NO_TEXT_STYLE: &str = concat!(
    "*, *::before, *::after, *::marker { color: transparent !important; -webkit-text-fill-color: transparent !important;",
    " -webkit-text-stroke-color: transparent !important; text-shadow: none !important;",
    " text-decoration-color: transparent !important; } svg text { fill: transparent !important; stroke: transparent !important; }"
);

/// The measure the page runs after the screenshot.
pub const MEASURE_JS: &str = include_str!("measure.js");

/// Playwright's screenshot preparation (`inPagePrepareForScreenshots` with `caret: 'hide'` and
/// `animations: 'disabled'`), run in the utility world before each screenshot.
const PREPARE_SCREENSHOT: &str = include_str!("prepare-screenshot.js");
const CLEANUP_SCREENSHOT: &str = "window.__pwCleanupScreenshot && window.__pwCleanupScreenshot()";

/// The page's own animation frames a screenshot waits for ([`screenshot`]).
const SETTLE_FRAMES: u32 = 2;

/// The default font families Playwright sets for a headless Linux browser.
fn linux_font_families() -> FontFamilies {
    FontFamilies {
        standard: Some("Times New Roman".into()),
        fixed: Some("Monospace".into()),
        serif: Some("Times New Roman".into()),
        sans_serif: Some("Arial".into()),
        cursive: Some("Comic Sans MS".into()),
        fantasy: Some("Impact".into()),
        math: None,
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MeasuredWord {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub text: String,
    /// The IR id of the nearest element (`data-el`).
    pub el: Option<String>,
    /// The document-order index of the nearest text box.
    pub text_box: Option<u32>,
    pub family: String,
    /// Font size in points.
    pub size: f64,
    pub color: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasuredElement {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub el: String,
}

/// A table cell's box: its table's IR id, its row, and its place among the row's printed cells.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasuredCell {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub el: String,
    pub row: u32,
    pub index: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MeasuredImage {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
    pub el: Option<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Measure {
    pub words: Vec<MeasuredWord>,
    pub elements: Vec<MeasuredElement>,
    /// None in a measure saved before cells were measured.
    #[serde(default)]
    pub cells: Vec<MeasuredCell>,
    pub images: Vec<MeasuredImage>,
}

/// `Math.round(v * 1e4) / 1e4 + 0`.
fn r4(value: f64) -> f64 {
    slidock_canon::js_round(value * 10_000.0) / 10_000.0 + 0.0
}

impl Measure {
    /// The measure as saved: boxes on a 1e-4 pt grid.
    pub fn rounded(&self) -> Measure {
        Measure {
            words: self
                .words
                .iter()
                .map(|w| MeasuredWord {
                    x: r4(w.x),
                    y: r4(w.y),
                    w: r4(w.w),
                    h: r4(w.h),
                    size: r4(w.size),
                    ..w.clone()
                })
                .collect(),
            elements: self
                .elements
                .iter()
                .map(|e| MeasuredElement {
                    x: r4(e.x),
                    y: r4(e.y),
                    w: r4(e.w),
                    h: r4(e.h),
                    el: e.el.clone(),
                })
                .collect(),
            cells: self
                .cells
                .iter()
                .map(|c| MeasuredCell {
                    x: r4(c.x),
                    y: r4(c.y),
                    w: r4(c.w),
                    h: r4(c.h),
                    ..c.clone()
                })
                .collect(),
            images: self
                .images
                .iter()
                .map(|i| MeasuredImage {
                    x: r4(i.x),
                    y: r4(i.y),
                    w: r4(i.w),
                    h: r4(i.h),
                    el: i.el.clone(),
                })
                .collect(),
        }
    }
}

/// What the router answers for one request.
#[derive(Clone, Debug)]
pub enum Response {
    Fulfill {
        status: u16,
        content_type: Option<&'static str>,
        body: Vec<u8>,
    },
    /// Fails the request as blocked by the client.
    Abort,
}

/// The renderer's network: every request the page makes is answered by this.
pub trait Router: Send + Sync {
    fn route(&self, url: &str) -> Response;
}

/// One render: the slide document's URL, its size in points and the device pixels per point.
pub struct RenderRequest {
    pub url: String,
    pub size: (f64, f64),
    pub px_per_pt: f64,
    pub router: Arc<dyn Router>,
}

/// One slide of a bundled deck, mounted by the deck's own player: the deck page's URL, the slide's id, its size in
/// points and the device pixels per point.
pub struct MountRequest {
    pub url: String,
    pub slide: String,
    pub size: (f64, f64),
    pub px_per_pt: f64,
    /// The slide after its last build (exits applied), not with every element shown.
    pub end_state: bool,
    pub router: Arc<dyn Router>,
}

/// The host lane's mount: the slide as the player's still render shows it, in a shadow root on the deck page.
pub const MOUNT_JS: &str = include_str!("mount.js");

/// The fonts lane's walk of the mounted slide's text nodes, with the font-family each is set in.
pub const TEXT_NODES_JS: &str = include_str!("text-nodes.js");

/// The fonts lane's shadow of the families the mounted slide names and the deck ships no face of: a face that never
/// loads, so a font the renderer has installed under that name never draws in the shipped stand-in's place.
pub const SHADOW_INSTALLED_JS: &str = include_str!("shadow-installed.js");

/// A font that drew glyphs of a text node (`CSS.PlatformFontUsage`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlatformFont {
    /// The family name the font's own tables give.
    pub family: String,
    pub postscript_name: String,
    /// A web font (here, a face of the deck's `<style id="sd-fonts">`); otherwise one of the renderer's fonts.
    pub custom: bool,
    pub glyphs: u32,
}

/// A text node of a mounted slide: its text, the computed font-family of its element, and the fonts that drew it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextNodeFonts {
    pub text: String,
    pub family: String,
    pub fonts: Vec<PlatformFont>,
    /// slidock's own label, not the deck's text ([`own_labels`]).
    pub own_label: bool,
}

/// The elements whose text slidock writes itself, in a generic font of its own: a foreign object's marker
/// (`Image not included`) and a failed container's diagnostic code.
fn own_labels() -> String {
    format!("[{FOREIGN_MARKER}],[{FAILED}]")
}

/// A mounted slide's render and the fonts that drew each of its text nodes.
pub struct Attributed {
    pub png: Vec<u8>,
    pub nodes: Vec<TextNodeFonts>,
}

/// The render, the same render with all text painted out, and the measure.
pub struct Rendered {
    pub png: Vec<u8>,
    pub textless: Vec<u8>,
    pub measure: Measure,
}

#[derive(Debug, thiserror::Error)]
pub enum RenderError {
    #[error("render took longer than {} s", RENDER_TIMEOUT.as_secs())]
    Timeout,
    #[error("{0}")]
    Crash(String),
}

fn crash(error: impl std::fmt::Display) -> RenderError {
    RenderError::Crash(error.to_string())
}

/// A connected browser. Dropping it ends the connection.
pub struct Browser {
    inner: chromiumoxide::Browser,
    handler: JoinHandle<()>,
    user_agent: String,
    product: String,
}

impl Drop for Browser {
    fn drop(&mut self) {
        self.handler.abort();
    }
}

impl Browser {
    pub async fn connect(ws_endpoint: &str) -> Result<Browser> {
        let config = HandlerConfig {
            viewport: None,
            request_timeout: Duration::from_secs(60),
            ..HandlerConfig::default()
        };
        let (inner, mut handler) = chromiumoxide::Browser::connect_with_config(ws_endpoint, config)
            .await
            .map_err(|error| BenchError::failed(format!("CDP connect {ws_endpoint}: {error}")))?;
        let handler = tokio::spawn(async move { while handler.next().await.is_some() {} });
        let version = inner
            .version()
            .await
            .map_err(|error| BenchError::failed(format!("Browser.getVersion: {error}")))?;
        Ok(Browser {
            inner,
            handler,
            user_agent: version.user_agent,
            product: version.product,
        })
    }

    /// The browser's version as Playwright reports it: `HeadlessChrome/148.0.7778.0` → `148.0.7778.0`.
    pub fn chromium_version(&self) -> String {
        self.product
            .rsplit('/')
            .next()
            .unwrap_or(&self.product)
            .to_owned()
    }

    /// One slide in a fresh context: the slide's own size in CSS pixels, scaled to the raster size, fully built
    /// (animations off, scripts off), fonts and images loaded, then a screenshot of exactly the slide, the measure,
    /// and a second screenshot with every glyph painted out.
    pub async fn render(
        &self,
        request: &RenderRequest,
    ) -> std::result::Result<Rendered, RenderError> {
        self.render_after(request, SETTLE_FRAMES).await
    }

    /// [`Browser::render`] with each screenshot `frames` animation frames after the page's last change.
    async fn render_after(
        &self,
        request: &RenderRequest,
        frames: u32,
    ) -> std::result::Result<Rendered, RenderError> {
        let context = self
            .inner
            .create_browser_context(CreateBrowserContextParams {
                dispose_on_detach: Some(true),
                ..CreateBrowserContextParams::default()
            })
            .await
            .map_err(crash)?;
        let result = tokio::time::timeout(
            RENDER_TIMEOUT,
            self.render_in(context.clone(), request, frames),
        )
        .await;
        let _ = self.inner.dispose_browser_context(context).await;
        match result {
            Ok(rendered) => rendered,
            Err(_) => Err(RenderError::Timeout),
        }
    }

    async fn render_in(
        &self,
        context: chromiumoxide::cdp::browser_protocol::browser::BrowserContextId,
        request: &RenderRequest,
        frames: u32,
    ) -> std::result::Result<Rendered, RenderError> {
        let page = self
            .inner
            .new_page(CreateTargetParams {
                url: "about:blank".into(),
                browser_context_id: Some(context),
                ..CreateTargetParams::default()
            })
            .await
            .map_err(crash)?;
        let (w, h) = request.size;
        let css = (w * 4.0 / 3.0, h * 4.0 / 3.0);
        let viewport = (css.0.ceil() as i64, css.1.ceil() as i64);
        self.set_up(&page, viewport, request.px_per_pt * 0.75, false)
            .await?;
        let router = request.router.clone();
        let routing = route_requests(&page, router).await?;
        let result = capture(&page, &request.url, css, viewport, frames).await;
        routing.abort();
        result
    }

    /// A slide of a bundled deck in a fresh context set up as [`Browser::render`] sets one up, with JavaScript on:
    /// the deck page loads and boots its player, the slide is mounted fully built in a shadow root at the deck's
    /// size, and the screenshot is taken as the standalone render's is.
    pub async fn render_mounted(
        &self,
        request: &MountRequest,
    ) -> std::result::Result<Vec<u8>, RenderError> {
        let context = self
            .inner
            .create_browser_context(CreateBrowserContextParams {
                dispose_on_detach: Some(true),
                ..CreateBrowserContextParams::default()
            })
            .await
            .map_err(crash)?;
        let result =
            tokio::time::timeout(RENDER_TIMEOUT, self.mount_in(context.clone(), request)).await;
        let _ = self.inner.dispose_browser_context(context).await;
        match result {
            Ok(rendered) => rendered,
            Err(_) => Err(RenderError::Timeout),
        }
    }

    /// A slide of a bundled deck mounted as [`Browser::render_mounted`] mounts it, with every family it names that
    /// the deck ships no face of shadowed by a face that never loads; its screenshot, then the fonts that drew every
    /// text node of its shadow tree (`CSS.getPlatformFontsForNode`).
    pub async fn attribute_mounted(
        &self,
        request: &MountRequest,
    ) -> std::result::Result<Attributed, RenderError> {
        let context = self
            .inner
            .create_browser_context(CreateBrowserContextParams {
                dispose_on_detach: Some(true),
                ..CreateBrowserContextParams::default()
            })
            .await
            .map_err(crash)?;
        let result =
            tokio::time::timeout(RENDER_TIMEOUT, self.attribute_in(context.clone(), request)).await;
        let _ = self.inner.dispose_browser_context(context).await;
        match result {
            Ok(attributed) => attributed,
            Err(_) => Err(RenderError::Timeout),
        }
    }

    async fn attribute_in(
        &self,
        context: chromiumoxide::cdp::browser_protocol::browser::BrowserContextId,
        request: &MountRequest,
    ) -> std::result::Result<Attributed, RenderError> {
        let page = self
            .inner
            .new_page(CreateTargetParams {
                url: "about:blank".into(),
                browser_context_id: Some(context),
                ..CreateTargetParams::default()
            })
            .await
            .map_err(crash)?;
        let (w, h) = request.size;
        let css = (w * 4.0 / 3.0, h * 4.0 / 3.0);
        let viewport = (css.0.ceil() as i64, css.1.ceil() as i64);
        self.set_up(&page, viewport, request.px_per_pt * 0.75, true)
            .await?;
        let routing = route_requests(&page, request.router.clone()).await?;
        let result = async {
            mount(&page, request, css).await?;
            let shadow = base64::engine::general_purpose::STANDARD.encode(shadow_font());
            evaluate(&page, &format!("({SHADOW_INSTALLED_JS})({shadow:?})"), None).await?;
            let png = capture_mounted(&page, css, viewport).await?;
            let nodes = text_node_fonts(&page).await?;
            Ok(Attributed { png, nodes })
        }
        .await;
        routing.abort();
        result
    }

    async fn mount_in(
        &self,
        context: chromiumoxide::cdp::browser_protocol::browser::BrowserContextId,
        request: &MountRequest,
    ) -> std::result::Result<Vec<u8>, RenderError> {
        let page = self
            .inner
            .new_page(CreateTargetParams {
                url: "about:blank".into(),
                browser_context_id: Some(context),
                ..CreateTargetParams::default()
            })
            .await
            .map_err(crash)?;
        let (w, h) = request.size;
        let css = (w * 4.0 / 3.0, h * 4.0 / 3.0);
        let viewport = (css.0.ceil() as i64, css.1.ceil() as i64);
        self.set_up(&page, viewport, request.px_per_pt * 0.75, true)
            .await?;
        let routing = route_requests(&page, request.router.clone()).await?;
        let result = mount_and_capture(&page, request, css, viewport).await;
        routing.abort();
        result
    }

    /// What Playwright's page initialisation sets for the bench's context options (viewport, device scale factor,
    /// locale `en-US`, timezone UTC, light colour scheme, reduced motion, JavaScript off).
    async fn set_up(
        &self,
        page: &Page,
        viewport: (i64, i64),
        device_scale_factor: f64,
        scripts: bool,
    ) -> std::result::Result<(), RenderError> {
        self.set_up_with(page, viewport, device_scale_factor, scripts, "reduce")
            .await
    }

    /// [`Browser::set_up`] with a `prefers-reduced-motion` of its own (the motion lane plays effects in full).
    async fn set_up_with(
        &self,
        page: &Page,
        viewport: (i64, i64),
        device_scale_factor: f64,
        scripts: bool,
        reduced_motion: &str,
    ) -> std::result::Result<(), RenderError> {
        page.execute(SetFocusEmulationEnabledParams::new(true))
            .await
            .map_err(crash)?;
        let metrics = SetDeviceMetricsOverrideParams {
            screen_width: Some(viewport.0),
            screen_height: Some(viewport.1),
            screen_orientation: Some(ScreenOrientation::new(
                ScreenOrientationType::LandscapePrimary,
                0,
            )),
            ..SetDeviceMetricsOverrideParams::new(
                viewport.0,
                viewport.1,
                device_scale_factor,
                false,
            )
        };
        page.execute(metrics).await.map_err(crash)?;
        if !scripts {
            page.execute(SetScriptExecutionDisabledParams::new(true))
                .await
                .map_err(crash)?;
        }
        page.execute(SetUserAgentOverrideParams {
            accept_language: Some("en-US".into()),
            ..SetUserAgentOverrideParams::new(self.user_agent.clone())
        })
        .await
        .map_err(crash)?;
        page.execute(SetLocaleOverrideParams {
            locale: Some("en-US".into()),
        })
        .await
        .map_err(crash)?;
        page.execute(SetTimezoneOverrideParams::new("UTC"))
            .await
            .map_err(crash)?;
        page.execute(SetFontFamiliesParams::new(linux_font_families()))
            .await
            .map_err(crash)?;
        let feature = |name: &str, value: &str| MediaFeature::new(name, value);
        page.execute(SetEmulatedMediaParams {
            media: Some(String::new()),
            features: Some(vec![
                feature("prefers-color-scheme", "light"),
                feature("prefers-reduced-motion", reduced_motion),
                feature("forced-colors", "none"),
                feature("prefers-contrast", "no-preference"),
            ]),
        })
        .await
        .map_err(crash)?;
        page.execute(network::EnableParams::default())
            .await
            .map_err(crash)?;
        page.execute(network::SetCacheDisabledParams::new(true))
            .await
            .map_err(crash)?;
        Ok(())
    }
}

/// A deck page the motion and shell lanes drive: JavaScript on, effects in full, a script evaluated before the deck
/// page's own, and every request the page makes recorded (and, with a router, answered by it).
pub struct MotionPageRequest {
    pub url: String,
    /// The page's size in CSS pixels.
    pub size: (f64, f64),
    /// Evaluated in every new document before its scripts.
    pub init_script: String,
    /// `None` lets the page load its own files (a `file:` deck); requests it makes are still recorded.
    pub router: Option<Arc<dyn Router>>,
}

/// An open deck page in its own browser context. [`MotionPage::close`] disposes of the context.
pub struct MotionPage {
    page: Page,
    context: chromiumoxide::cdp::browser_protocol::browser::BrowserContextId,
    tasks: Vec<JoinHandle<()>>,
    requests: Arc<std::sync::Mutex<Vec<String>>>,
    clip: (f64, f64),
}

impl MotionPage {
    /// Evaluates an expression (awaiting a promise) and returns its value.
    pub async fn call(
        &self,
        expression: &str,
    ) -> std::result::Result<serde_json::Value, RenderError> {
        match tokio::time::timeout(MOTION_CALL_TIMEOUT, evaluate(&self.page, expression, None))
            .await
        {
            Ok(value) => value,
            Err(_) => Err(RenderError::Timeout),
        }
    }

    /// A screenshot of the page's top-left `size`, animations as they are.
    pub async fn shot(&self) -> std::result::Result<Vec<u8>, RenderError> {
        let capture = self.page.execute(CaptureScreenshotParams {
            format: Some(CaptureScreenshotFormat::Png),
            clip: Some(Viewport {
                x: 0.0,
                y: 0.0,
                width: self.clip.0,
                height: self.clip.1,
                scale: 1.0,
            }),
            capture_beyond_viewport: Some(false),
            ..CaptureScreenshotParams::default()
        });
        let shot = match tokio::time::timeout(MOTION_CALL_TIMEOUT, capture).await {
            Ok(shot) => shot.map_err(crash)?,
            Err(_) => return Err(RenderError::Timeout),
        };
        let data: &str = shot.result.data.as_ref();
        base64::engine::general_purpose::STANDARD
            .decode(data)
            .map_err(|error| RenderError::Crash(format!("screenshot: {error}")))
    }

    /// The page's target id, for traces.
    pub fn id(&self) -> String {
        self.page.target_id().inner().chars().take(6).collect()
    }

    /// Every URL the page requested, in order.
    pub fn requests(&self) -> Vec<String> {
        self.requests
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default()
    }

    pub async fn close(self, browser: &Browser) {
        for task in &self.tasks {
            task.abort();
        }
        let _ = browser.inner.dispose_browser_context(self.context).await;
    }
}

/// How long one call into a motion page may take: a move waits for the slide's fonts and images.
pub const MOTION_CALL_TIMEOUT: Duration = Duration::from_secs(120);

impl Browser {
    /// Opens a deck page for the motion lane: a fresh context set up as [`Browser::render`] sets one up, but with
    /// JavaScript on, `prefers-reduced-motion: no-preference` and one device pixel per CSS pixel.
    pub async fn open_motion_page(
        &self,
        request: &MotionPageRequest,
    ) -> std::result::Result<MotionPage, RenderError> {
        let context = self
            .inner
            .create_browser_context(CreateBrowserContextParams {
                dispose_on_detach: Some(true),
                ..CreateBrowserContextParams::default()
            })
            .await
            .map_err(crash)?;
        let opened =
            tokio::time::timeout(MOTION_CALL_TIMEOUT, self.open_in(context.clone(), request)).await;
        match opened {
            Ok(Ok(page)) => Ok(page),
            Ok(Err(error)) => {
                let _ = self.inner.dispose_browser_context(context).await;
                Err(error)
            }
            Err(_) => {
                let _ = self.inner.dispose_browser_context(context).await;
                Err(RenderError::Timeout)
            }
        }
    }

    async fn open_in(
        &self,
        context: chromiumoxide::cdp::browser_protocol::browser::BrowserContextId,
        request: &MotionPageRequest,
    ) -> std::result::Result<MotionPage, RenderError> {
        let page = self
            .inner
            .new_page(CreateTargetParams {
                url: "about:blank".into(),
                browser_context_id: Some(context.clone()),
                ..CreateTargetParams::default()
            })
            .await
            .map_err(crash)?;
        let viewport = (request.size.0.ceil() as i64, request.size.1.ceil() as i64);
        self.set_up_with(&page, viewport, 1.0, true, "no-preference")
            .await?;
        page.execute(AddScriptToEvaluateOnNewDocumentParams::new(
            request.init_script.clone(),
        ))
        .await
        .map_err(crash)?;
        let requests = Arc::new(std::sync::Mutex::new(Vec::new()));
        let mut sent = page
            .event_listener::<network::EventRequestWillBeSent>()
            .await
            .map_err(crash)?;
        let log = requests.clone();
        let mut tasks = vec![tokio::spawn(async move {
            while let Some(event) = sent.next().await {
                if let Ok(mut list) = log.lock() {
                    list.push(event.request.url.clone());
                }
            }
        })];
        if let Some(router) = &request.router {
            tasks.push(route_requests(&page, router.clone()).await?);
        }
        let mut loaded = page
            .event_listener::<EventLoadEventFired>()
            .await
            .map_err(crash)?;
        let navigated = page
            .execute(NavigateParams::new(request.url.as_str()))
            .await
            .map_err(crash)?;
        if let Some(error) = &navigated.result.error_text {
            for task in &tasks {
                task.abort();
            }
            return Err(RenderError::Crash(format!(
                "navigate {}: {error}",
                request.url
            )));
        }
        loaded.next().await;
        Ok(MotionPage {
            page,
            context,
            tasks,
            requests,
            clip: (viewport.0 as f64, viewport.1 as f64),
        })
    }
}

/// Answers every request the page makes with the router, from the moment `Fetch` is enabled.
async fn route_requests(
    page: &Page,
    router: Arc<dyn Router>,
) -> std::result::Result<JoinHandle<()>, RenderError> {
    let mut paused = page
        .event_listener::<EventRequestPaused>()
        .await
        .map_err(crash)?;
    let responder = page.clone();
    let task = tokio::spawn(async move {
        while let Some(event) = paused.next().await {
            let answer = router.route(&event.request.url);
            let id = event.request_id.clone();
            let _ = match answer {
                Response::Abort => responder
                    .execute(FailRequestParams::new(id, ErrorReason::BlockedByClient))
                    .await
                    .map(|_| ()),
                Response::Fulfill {
                    status,
                    content_type,
                    body,
                } => {
                    let mut headers = Vec::new();
                    if let Some(content_type) = content_type {
                        headers.push(HeaderEntry::new("content-type", content_type));
                    }
                    headers.push(HeaderEntry::new("content-length", body.len().to_string()));
                    let params = FulfillRequestParams {
                        response_headers: Some(headers),
                        body: Some(
                            base64::engine::general_purpose::STANDARD
                                .encode(&body)
                                .into(),
                        ),
                        ..FulfillRequestParams::new(id, i64::from(status))
                    };
                    responder.execute(params).await.map(|_| ())
                }
            };
        }
    });
    page.execute(fetch::EnableParams {
        patterns: Some(vec![RequestPattern {
            url_pattern: Some("*".into()),
            resource_type: None,
            request_stage: Some(RequestStage::Request),
        }]),
        handle_auth_requests: Some(true),
    })
    .await
    .map_err(crash)?;
    Ok(task)
}

/// Evaluates an expression, awaiting a promise, and returns its value as JSON.
async fn evaluate(
    page: &Page,
    expression: &str,
    context: Option<ExecutionContextId>,
) -> std::result::Result<serde_json::Value, RenderError> {
    let mut params = EvaluateParams::new(expression);
    params.await_promise = Some(true);
    params.return_by_value = Some(true);
    params.context_id = context;
    let response = page.execute(params).await.map_err(crash)?;
    let result = &response.result;
    if let Some(exception) = &result.exception_details {
        return Err(RenderError::Crash(format!(
            "evaluate: {}",
            exception
                .exception
                .as_ref()
                .and_then(|e| e.description.clone())
                .unwrap_or_else(|| exception.text.clone())
        )));
    }
    Ok(result
        .result
        .value
        .clone()
        .unwrap_or(serde_json::Value::Null))
}

/// Loads the slide and takes the two screenshots and the measure.
async fn capture(
    page: &Page,
    url: &str,
    css: (f64, f64),
    viewport: (i64, i64),
    frames: u32,
) -> std::result::Result<Rendered, RenderError> {
    let mut loaded = page
        .event_listener::<EventLoadEventFired>()
        .await
        .map_err(crash)?;
    let navigated = page
        .execute(NavigateParams::new(url))
        .await
        .map_err(crash)?;
    if let Some(error) = &navigated.result.error_text {
        return Err(RenderError::Crash(format!("navigate {url}: {error}")));
    }
    loaded.next().await;
    evaluate(
        page,
        "(async () => { await document.fonts.ready; await Promise.all([...document.images].map((image) => image.decode().catch(() => undefined))); })()",
        None,
    )
    .await?;
    let frame = page
        .mainframe()
        .await
        .map_err(crash)?
        .ok_or_else(|| RenderError::Crash("the page has no main frame".into()))?;
    let world = page
        .execute(CreateIsolatedWorldParams {
            frame_id: frame,
            world_name: Some("__slidock_bench_utility__".into()),
            grant_univeral_access: Some(true),
        })
        .await
        .map_err(crash)?
        .result
        .execution_context_id;
    let png = screenshot(page, world, css, viewport, frames).await?;
    let value = evaluate(page, &format!("({MEASURE_JS})()"), None).await?;
    let measure: Measure = serde_json::from_value(value)
        .map_err(|error| RenderError::Crash(format!("measure: {error}")))?;
    // Appended by hand: a page without JavaScript fires no load event for an added style.
    let style = serde_json::to_string(NO_TEXT_STYLE).map_err(crash)?;
    evaluate(
        page,
        &format!(
            "(() => {{ const style = document.createElement('style'); style.textContent = {style}; document.head.append(style); }})()"
        ),
        None,
    )
    .await?;
    let textless = screenshot(page, world, css, viewport, frames).await?;
    Ok(Rendered {
        png,
        textless,
        measure,
    })
}

/// Loads the deck page, mounts the slide and takes the screenshot.
async fn mount_and_capture(
    page: &Page,
    request: &MountRequest,
    css: (f64, f64),
    viewport: (i64, i64),
) -> std::result::Result<Vec<u8>, RenderError> {
    mount(page, request, css).await?;
    capture_mounted(page, css, viewport).await
}

/// Loads the deck page and mounts the slide.
async fn mount(
    page: &Page,
    request: &MountRequest,
    css: (f64, f64),
) -> std::result::Result<(), RenderError> {
    let mut loaded = page
        .event_listener::<EventLoadEventFired>()
        .await
        .map_err(crash)?;
    let navigated = page
        .execute(NavigateParams::new(request.url.as_str()))
        .await
        .map_err(crash)?;
    if let Some(error) = &navigated.result.error_text {
        return Err(RenderError::Crash(format!(
            "navigate {}: {error}",
            request.url
        )));
    }
    loaded.next().await;
    let slide = serde_json::to_string(&request.slide).map_err(crash)?;
    evaluate(
        page,
        &format!(
            "({MOUNT_JS})({slide}, {}, {}, {})",
            css.0, css.1, request.end_state
        ),
        None,
    )
    .await?;
    Ok(())
}

/// The screenshot of a mounted slide.
async fn capture_mounted(
    page: &Page,
    css: (f64, f64),
    viewport: (i64, i64),
) -> std::result::Result<Vec<u8>, RenderError> {
    let frame = page
        .mainframe()
        .await
        .map_err(crash)?
        .ok_or_else(|| RenderError::Crash("the page has no main frame".into()))?;
    let world = page
        .execute(CreateIsolatedWorldParams {
            frame_id: frame,
            world_name: Some("__slidock_bench_utility__".into()),
            grant_univeral_access: Some(true),
        })
        .await
        .map_err(crash)?
        .result
        .execution_context_id;
    screenshot(page, world, css, viewport, SETTLE_FRAMES).await
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PageTextNode {
    text: String,
    family: String,
    own_label: bool,
}

/// The text nodes of the shadow tree under the element that carries `data-sd-layer`, in tree order.
fn shadow_text_nodes(document: &dom::Node) -> Vec<&dom::Node> {
    let is_layer = |node: &dom::Node| {
        node.attributes
            .as_ref()
            .is_some_and(|attributes| attributes.iter().step_by(2).any(|a| a == "data-sd-layer"))
    };
    let mut stack = vec![document];
    let mut root = None;
    while let Some(node) = stack.pop() {
        if is_layer(node) {
            root = node.shadow_roots.as_ref().and_then(|roots| roots.first());
            break;
        }
        for children in [&node.children, &node.shadow_roots].into_iter().flatten() {
            stack.extend(children.iter().rev());
        }
    }
    let mut out = Vec::new();
    let mut stack: Vec<&dom::Node> = root.into_iter().collect();
    while let Some(node) = stack.pop() {
        if node.node_type == 3 {
            out.push(node);
        }
        if let Some(children) = &node.children {
            stack.extend(children.iter().rev());
        }
    }
    out
}

/// Every text node of the mounted slide with the fonts that drew it: the page lists the nodes with their computed
/// font-family, CDP's DOM walk gives their ids, and the two must agree node for node.
async fn text_node_fonts(page: &Page) -> std::result::Result<Vec<TextNodeFonts>, RenderError> {
    page.execute(dom::EnableParams::default())
        .await
        .map_err(crash)?;
    page.execute(css::EnableParams::default())
        .await
        .map_err(crash)?;
    let listed: Vec<PageTextNode> = serde_json::from_value(
        evaluate(
            page,
            &format!("({TEXT_NODES_JS})({:?})", own_labels()),
            None,
        )
        .await?,
    )
    .map_err(|error| RenderError::Crash(format!("text nodes: {error}")))?;
    let document = page
        .execute(dom::GetDocumentParams {
            depth: Some(-1),
            pierce: Some(true),
        })
        .await
        .map_err(crash)?
        .result
        .root;
    let walked = shadow_text_nodes(&document);
    if walked.len() != listed.len()
        || walked
            .iter()
            .zip(&listed)
            .any(|(node, page_node)| node.node_value != page_node.text)
    {
        return Err(RenderError::Crash(format!(
            "the page lists {} text nodes and the DOM walk finds {} others",
            listed.len(),
            walked.len()
        )));
    }
    let mut out = Vec::with_capacity(listed.len());
    for (node, page_node) in walked.into_iter().zip(listed) {
        let fonts = page
            .execute(css::GetPlatformFontsForNodeParams::new(node.node_id))
            .await
            .map_err(crash)?
            .result
            .fonts
            .into_iter()
            .map(|font| PlatformFont {
                family: font.family_name,
                postscript_name: font.post_script_name,
                custom: font.is_custom_font,
                glyphs: font.glyph_count.max(0.0).min(f64::from(u32::MAX)) as u32,
            })
            .collect();
        out.push(TextNodeFonts {
            text: page_node.text,
            family: page_node.family,
            fonts,
            own_label: page_node.own_label,
        });
    }
    Ok(out)
}

/// `page.screenshot({clip: {x: 0, y: 0, width, height}, animations: 'disabled', caret: 'hide', scale: 'device'})`:
/// the preparation in the utility world, fonts awaited, `frames` of the page's own animation frames, the clip in
/// document coordinates, then the cleanup.
///
/// The frames make the capture deterministic. A screenshot taken before the page has drawn its last change forces
/// the frame that rasters it, and Chromium's software raster of that change then varies from one render to the next:
/// a shape edge near the change (a stroke's outermost anti-aliased column) comes out a few levels off in some renders,
/// with the same document, fonts and size, and stays so for the page's later screenshots. Rastered on a frame of the
/// page's own, the change draws as a fresh load of the changed page draws it, every time. The capture waits for two
/// frames: the second begins after the frame that painted the change.
async fn screenshot(
    page: &Page,
    world: ExecutionContextId,
    css: (f64, f64),
    viewport: (i64, i64),
    frames: u32,
) -> std::result::Result<Vec<u8>, RenderError> {
    evaluate(
        page,
        &format!("({PREPARE_SCREENSHOT})(undefined, true, true, false)"),
        Some(world),
    )
    .await?;
    let _ = evaluate(page, "document.fonts.ready", Some(world)).await;
    if frames > 0 {
        evaluate(page, &after_frames(frames), Some(world)).await?;
    }
    let metrics = page
        .execute(GetLayoutMetricsParams::default())
        .await
        .map_err(crash)?;
    let visual = &metrics.result.css_visual_viewport;
    let clip_w = css.0.min(viewport.0 as f64).max(0.0);
    let clip_h = css.1.min(viewport.1 as f64).max(0.0);
    let clip = Viewport {
        x: visual.page_x,
        y: visual.page_y,
        width: (clip_w / visual.scale + 1e-3).floor(),
        height: (clip_h / visual.scale + 1e-3).floor(),
        scale: visual.scale,
    };
    let shot = page
        .execute(CaptureScreenshotParams {
            format: Some(CaptureScreenshotFormat::Png),
            clip: Some(clip),
            capture_beyond_viewport: Some(false),
            ..CaptureScreenshotParams::default()
        })
        .await
        .map_err(crash)?;
    let _ = evaluate(page, CLEANUP_SCREENSHOT, Some(world)).await;
    let data: &str = shot.result.data.as_ref();
    base64::engine::general_purpose::STANDARD
        .decode(data)
        .map_err(|error| RenderError::Crash(format!("screenshot: {error}")))
}

/// A promise that resolves at the start of the page's `frames`-th animation frame from now.
fn after_frames(frames: u32) -> String {
    format!(
        "new Promise((done) => {{ let left = {frames}; const next = () => (--left > 0 ? requestAnimationFrame(next) : done()); requestAnimationFrame(next); }})"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn measures_round_to_the_grid() {
        let measure = Measure {
            words: vec![MeasuredWord {
                x: 1.234_56,
                y: -0.000_01,
                w: 2.0,
                h: 3.000_05,
                text: "a".into(),
                el: None,
                text_box: Some(0),
                family: "Arial".into(),
                size: 18.000_04,
                color: "rgb(0, 0, 0)".into(),
            }],
            ..Measure::default()
        };
        let rounded = measure.rounded();
        let word = &rounded.words[0];
        assert_eq!(
            (word.x, word.y, word.h, word.size),
            (1.2346, 0.0, 3.0001, 18.0)
        );
        assert!(word.y.is_sign_positive());
    }

    #[test]
    fn the_wait_counts_the_pages_animation_frames() {
        let two = after_frames(2);
        assert!(two.starts_with("new Promise("));
        assert!(two.contains("let left = 2;"));
        assert_eq!(two.matches("requestAnimationFrame(").count(), 2);
        assert!(after_frames(1).contains("let left = 1;"));
    }

    /// A page whose change Chromium rasters two ways when the capture forces the frame: a stroked rounded rectangle
    /// beside a text box, and many lines of text elsewhere, whose painting-out the textless screenshot rasters.
    fn changed_page() -> String {
        let lines: String = (0..80)
            .map(|i| {
                format!(
                    "<div style=\"position:absolute;left:{}pt;top:{}pt;width:460pt\"><p style=\"font-size:24pt;font-family:sans-serif\">Lorem ipsum dolor sit amet consectetur {i}</p></div>",
                    20 + (i % 4) * 470,
                    470 + (i / 4) * 30
                )
            })
            .collect();
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><style>html,body{{margin:0;padding:0}}p{{margin:0}}</style></head><body>\
             <main style=\"position:relative;width:1920pt;height:1080pt;overflow:hidden\">\
             <div style=\"position:absolute;left:0pt;top:0pt;width:1920pt;height:1080pt;background-color:#c8f6ff\"></div>\
             <div style=\"position:absolute;left:75.397pt;top:318.973pt;width:1809.56pt;translate:0 -50%;padding:4pt;box-sizing:border-box\">\
             <p style=\"line-height:1.07125;font-size:80pt;font-family:sans-serif\">Lorem ipsum dolor sit amet, consectetur adipiscing elit, sed do eiusmod tempor incididunt ut labore et dolore.</p></div>\
             {lines}\
             <div style=\"position:absolute;left:1531.303pt;top:365.248pt;width:294.683pt;height:97.679pt\">\
             <svg viewBox=\"0 0 294.683 97.679\" preserveAspectRatio=\"none\" width=\"100%\" height=\"100%\" style=\"position:absolute;left:0pt;top:0pt;overflow:visible\">\
             <path d=\"M0 15C0 6.716 6.716 0 15 0L279.683 0C287.967 0 294.683 6.716 294.683 15L294.683 82.679C294.683 90.963 287.967 97.679 279.683 97.679L15 97.679C6.716 97.679 0 90.963 0 82.679Z\" stroke-width=\"5\" style=\"fill:none;stroke:#dd695c\"></path>\
             </svg></div></main></body></html>"
        )
    }

    struct OnePage(String);

    impl Router for OnePage {
        fn route(&self, url: &str) -> Response {
            if url == crate::renderer::route::slide_url("changed") {
                Response::Fulfill {
                    status: 200,
                    content_type: Some("text/html"),
                    body: self.0.clone().into_bytes(),
                }
            } else {
                Response::Abort
            }
        }
    }

    /// Against a running renderer (`SLIDOCK_BENCH_RENDERER_HTTP`, e.g. the address `slidock-bench renderer start`
    /// prints; skipped without it): renders of a slide whose textless screenshot follows a change are byte-equal,
    /// and the same renders without the frames are not.
    #[tokio::test]
    async fn renders_are_the_same_bytes_once_the_page_drew_its_change() {
        let Ok(http) = std::env::var("SLIDOCK_BENCH_RENDERER_HTTP") else {
            eprintln!("skipped: SLIDOCK_BENCH_RENDERER_HTTP names no running renderer");
            return;
        };
        let ws = crate::renderer::docker::wait_for_browser(&http, Duration::from_secs(30))
            .await
            .unwrap();
        let browser = Browser::connect(&ws).await.unwrap();
        let request = RenderRequest {
            url: crate::renderer::route::slide_url("changed"),
            size: (1920.0, 1080.0),
            px_per_pt: 1.0,
            router: Arc::new(OnePage(changed_page())),
        };
        let mut settled = std::collections::BTreeSet::new();
        for _ in 0..24 {
            let rendered = browser.render(&request).await.unwrap();
            settled.insert((
                crate::hash::sha256(&rendered.png),
                crate::hash::sha256(&rendered.textless),
            ));
        }
        assert_eq!(settled.len(), 1, "{settled:?}");
        // The negative control: captured on the frame they force, the textless renders vary.
        let mut forced = std::collections::BTreeSet::new();
        for _ in 0..48 {
            let rendered = browser.render_after(&request, 0).await.unwrap();
            forced.insert(crate::hash::sha256(&rendered.textless));
            if forced.len() > 1 {
                break;
            }
        }
        assert!(forced.len() > 1, "48 forced renders drew alike");
    }

    #[test]
    fn the_measure_is_the_reference_function() {
        assert!(MEASURE_JS.contains("function measureSlide()"));
        assert!(MEASURE_JS.contains("[data-sd-text]"));
        assert!(PREPARE_SCREENSHOT.starts_with("function inPagePrepareForScreenshots("));
    }
}
