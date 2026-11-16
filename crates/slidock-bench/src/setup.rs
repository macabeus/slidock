//! What every command shares: where things live, and a scoring session: the font set, the renderer and its browser, and
//! the stamp of everything that can move a score.

use std::sync::Arc;

use slidock_bench_metrics::{
    CLASSIFIER_VERSION, COLOR_VERSION, GRADIENT_VERSION, PIXEL_VERSION, SHADOW_VERSION,
    SSIM_VERSION, TEXT_VERSION,
};
use slidock_bench_metrics::{METRIC_LONG_SIDE_PX, RASTER_LONG_SIDE_PX};
use slidock_bench_schema::corpus::{Presentation, Source};
use slidock_bench_schema::stamp::{
    FontsStamp, MetricsStamp, RenderPolicy, RendererStamp, Selection, Srgb, Stamp,
};
use slidock_bench_schema::{Lane, Sha256, Tier, True};

use crate::cache::CacheLayout;
use crate::env::BenchEnv;
use crate::error::{BenchError, Result};
use crate::fonts::{check_font_set, load_font_lock};
use crate::hash::sha256_file;
use crate::renderer::docker::{PLATFORM_DIGEST, Renderer, own_renderer, start_renderer};
use crate::renderer::session::{Browser, CDP_CLIENT};
use crate::stamp::{compiler_stamp, deps_stamp, harness_hash, host_stamp};

/// Where things live, and the cache's layout.
pub struct Context {
    pub env: BenchEnv,
    pub cache: CacheLayout,
}

impl Context {
    pub fn new(env: BenchEnv) -> Context {
        let cache = CacheLayout::new(&env.cache_dir);
        Context { env, cache }
    }

    /// The shared renderer of this cache, started (or reused) for the synced font set; it keeps running.
    pub async fn renderer(&self, allow_font_drift: bool) -> Result<(Renderer, bool)> {
        let (font_set_id, drift) = self.synced_set(allow_font_drift)?;
        let renderer = start_renderer(
            &self.env.bench_dir,
            &self.cache.fonts,
            &self.cache.oracle,
            &font_set_id,
        )
        .await?;
        Ok((renderer, drift))
    }

    /// A renderer for one command over the synced font set ([`own_renderer`]): the command releases it when done.
    pub async fn own_renderer(&self) -> Result<Renderer> {
        let (font_set_id, _) = self.synced_set(false)?;
        own_renderer(
            &self.env.bench_dir,
            &self.cache.fonts,
            &self.cache.oracle,
            &font_set_id,
        )
        .await
    }

    fn synced_set(&self, allow_font_drift: bool) -> Result<(String, bool)> {
        let lock = load_font_lock(&self.env.fonts_lock_path)?;
        check_font_set(&lock, &self.cache, allow_font_drift)
    }
}

/// A scoring session.
pub struct Session {
    pub renderer: Arc<Renderer>,
    pub browser: Arc<Browser>,
    pub poppler: String,
    pub stamp: Stamp,
}

/// The renderer's part of the stamp.
pub async fn renderer_stamp(
    renderer: &Renderer,
    browser: &Browser,
) -> Result<(RendererStamp, String)> {
    let (arch, poppler) = renderer.facts().await?;
    Ok((
        RendererStamp {
            recipe: Sha256::new(renderer.recipe.clone())?,
            platform_digest: PLATFORM_DIGEST.to_owned(),
            image_id: renderer.image_id.clone(),
            arch,
            chromium: browser.chromium_version(),
            cdp_client: CDP_CLIENT.to_owned(),
            poppler: poppler.clone(),
            raster_long_side_px: RASTER_LONG_SIDE_PX,
            metric_long_side_px: METRIC_LONG_SIDE_PX,
            color_profile: Srgb::Value,
            render_policy: RenderPolicy::default(),
        },
        poppler,
    ))
}

pub async fn start_session(
    ctx: &Context,
    pairs: &[(&Presentation, &Source)],
    tiers: &[Tier],
    allow_font_drift: bool,
) -> Result<Session> {
    let (renderer, drift) = ctx.renderer(allow_font_drift).await?;
    let browser = Browser::connect(&renderer.ws_endpoint).await?;
    let (renderer_part, poppler) = renderer_stamp(&renderer, &browser).await?;
    let repo = &ctx.env.repo_root;
    let mut corpus = std::collections::BTreeMap::new();
    for (presentation, source) in pairs {
        if let Some((pair, digest)) = presentation.corpus_stamp(source.frontend)? {
            corpus.insert(pair, digest);
        }
    }
    let mut tiers = tiers.to_vec();
    tiers.sort();
    tiers.dedup();
    let stamp = Stamp {
        compiler: compiler_stamp(repo).await?,
        deps: deps_stamp(repo).await?,
        host: host_stamp(),
        renderer: renderer_part,
        fonts: FontsStamp {
            font_set_id: Sha256::new(renderer.font_set_id.clone())?,
            lock_sha256: Sha256::new(sha256_file(&ctx.env.fonts_lock_path)?)?,
            drift: drift.then_some(True),
        },
        metrics: MetricsStamp {
            ssim: SSIM_VERSION.to_owned(),
            color: COLOR_VERSION.to_owned(),
            pixel: PIXEL_VERSION.to_owned(),
            text: TEXT_VERSION.to_owned(),
            shadow: Some(SHADOW_VERSION.to_owned()),
            gradient: Some(GRADIENT_VERSION.to_owned()),
            motion: None,
            fonts: None,
        },
        classifier: CLASSIFIER_VERSION.to_owned(),
        harness: harness_hash(repo, None).await?,
        corpus,
        selection: Selection { tiers },
        lane: Lane::Static,
        partial: false,
        determinism: None,
        parity: None,
        perf: None,
    };
    Ok(Session {
        renderer: Arc::new(renderer),
        browser: Arc::new(browser),
        poppler,
        stamp,
    })
}

/// A comma-separated list, trimmed, empty items dropped.
pub fn list(value: Option<&str>) -> Option<Vec<String>> {
    value.map(|v| {
        v.split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
            .collect()
    })
}

/// Parses each item of a list, naming the first that does not parse.
pub fn parse_list<T: std::str::FromStr>(value: Option<&str>, what: &str) -> Result<Option<Vec<T>>> {
    list(value)
        .map(|items| {
            items
                .iter()
                .map(|item| {
                    item.parse()
                        .map_err(|_| BenchError::usage(format!("unknown {what} {item}")))
                })
                .collect()
        })
        .transpose()
}

/// A tier by its name.
pub fn parse_tier(text: &str) -> Option<Tier> {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).ok()
}
