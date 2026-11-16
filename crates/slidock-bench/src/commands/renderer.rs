//! `renderer build | start | stop | status`.

use crate::error::{Exit, Result};
use crate::renderer::docker::{ensure_image, remove_orphans, stop_renderer};
use crate::renderer::session::Browser;
use crate::setup::{Context, renderer_stamp};

pub async fn build(ctx: &Context) -> Result<Exit> {
    let image = ensure_image(&ctx.env.bench_dir, true).await?;
    println!("{} {}", image.tag, image.id);
    Ok(Exit::Ok)
}

pub async fn start(ctx: &Context) -> Result<Exit> {
    let (renderer, _) = ctx.renderer(false).await?;
    let browser = Browser::connect(&renderer.ws_endpoint).await?;
    let (identity, _) = renderer_stamp(&renderer, &browser).await?;
    println!("{} {}", renderer.container, renderer.http);
    let value = serde_json::to_value(&identity).unwrap_or_default();
    if let Some(fields) = value.as_object() {
        for (key, value) in fields {
            println!("  {key} {value}");
        }
    }
    println!("  fontSetId {}", renderer.font_set_id);
    Ok(Exit::Ok)
}

pub async fn stop(ctx: &Context) -> Result<Exit> {
    let stopped = stop_renderer(&ctx.env.bench_dir, &ctx.cache.oracle, None).await?;
    println!("{}", if stopped { "stopped" } else { "not running" });
    for name in remove_orphans().await? {
        println!("removed {name}: its command ended without removing it");
    }
    Ok(Exit::Ok)
}
