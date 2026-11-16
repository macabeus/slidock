//! Copies the player every deck embeds, and the presenter view a deck embeds when it ships one, into `$OUT_DIR`, with
//! the player's version: `player/player.iife.js`, `player/presenter.iife.js` and `player/version.txt`. They are
//! committed copies of `@slidock/player`'s IIFE builds, which `pnpm player:build` (`packages/player/build.mjs`) writes
//! there, so the crate builds without Node and a packaged crate carries its player; CI checks they equal a fresh
//! build of the player's sources (`pnpm player:check`).
//!
//! The script is written inside `<script>…</script>` as it is, and the page's CSP allows it by the hash of those
//! bytes. Text that would end the element early, change how the parser reads it or change what the parser hands the
//! engine fails the build, and so does a script that lacks what the boot script calls (`build/embed.rs`).

#[path = "build/embed.rs"]
mod embed;

use std::error::Error;
use std::path::{Path, PathBuf};

fn read(path: &Path) -> Result<String, Box<dyn Error>> {
    println!("cargo::rerun-if-changed={}", path.display());
    std::fs::read_to_string(path).map_err(|error| format!("{}: {error}", path.display()).into())
}

fn embed(dir: &Path, script: &embed::Script, out: &Path) -> Result<(), Box<dyn Error>> {
    let path = dir.join(script.file);
    let text = read(&path)?;
    let shown = path.display().to_string();
    embed::check_inline(&text, &shown)?;
    embed::check_defines(script, &text, &shown)?;
    std::fs::write(out.join(script.file), text)?;
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("player");
    let out = PathBuf::from(std::env::var("OUT_DIR")?);
    embed(&dir, &embed::PLAYER, &out)?;
    embed(&dir, &embed::PRESENTER, &out)?;
    let version_file = dir.join("version.txt");
    let version = embed::parse_version(&read(&version_file)?, &version_file.display().to_string())?;
    std::fs::write(
        out.join("player.rs"),
        format!("/// The embedded player's version.\npub const VERSION: &str = {version:?};\n"),
    )?;
    Ok(())
}
