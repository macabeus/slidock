//! `fonts sync | lock | status | link <dir> [--releases] | lane-lock`; `lane-lock` records the fonts lane's Google
//! stand-in ([`crate::fonts_lane::record`]).

use std::path::Path;

use crate::cache::write_json;
use crate::error::{Exit, Result};
use crate::fonts::{font_set_state, link_fonts, load_font_lock, relock_fonts, sync_fonts};
use crate::setup::Context;

pub async fn sync(ctx: &Context) -> Result<Exit> {
    let lock = load_font_lock(&ctx.env.fonts_lock_path)?;
    let report = sync_fonts(&lock, &ctx.cache).await?;
    println!(
        "{} written, {} already present",
        report.written.len(),
        report.present.len()
    );
    for (file, message) in &report.failed {
        eprintln!("failed {file}: {message}");
    }
    let state = font_set_state(&lock, &ctx.cache)?;
    println!("font set {}", state.present_id);
    Ok(Exit::check(report.failed.is_empty()))
}

pub async fn lock(ctx: &Context) -> Result<Exit> {
    let lock = load_font_lock(&ctx.env.fonts_lock_path)?;
    let relocked = relock_fonts(&lock, &ctx.cache).await?;
    write_json(&ctx.env.fonts_lock_path, &relocked)?;
    println!(
        "{} fonts re-hashed into {}; review the diff",
        relocked.fonts.len(),
        ctx.env.fonts_lock_path.display()
    );
    Ok(Exit::Ok)
}

pub fn status(ctx: &Context) -> Result<Exit> {
    let lock = load_font_lock(&ctx.env.fonts_lock_path)?;
    let state = font_set_state(&lock, &ctx.cache)?;
    println!("lock {}\ncache {}", state.lock_id, state.present_id);
    for file in &state.missing {
        println!("missing {file}");
    }
    for file in &state.extra {
        println!("extra {file}");
    }
    Ok(Exit::check(
        state.missing.is_empty() && state.extra.is_empty(),
    ))
}

pub fn link(ctx: &Context, dir: &Path, releases: bool) -> Result<Exit> {
    let lock = load_font_lock(&ctx.env.fonts_lock_path)?;
    let linked = link_fonts(&lock, &ctx.cache, dir, releases)?;
    println!("{} fonts linked into {}", linked.len(), dir.display());
    Ok(Exit::Ok)
}
