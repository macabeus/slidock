//! The checks `build.rs` runs on the scripts it embeds (the player, and the presenter view's), kept apart from the
//! script so the crate's tests run them too.

/// A script a deck embeds: its file under `player/` (a copy of `@slidock/player`'s IIFE build of the same name), the
/// global it defines, and what the page's boot script calls on it (`player::BOOT`, `player::BOOT_WITH_PRESENTER`).
pub struct Script {
    pub file: &'static str,
    pub global: &'static str,
    pub calls: &'static [&'static str],
}

/// The player: `Slidock.mount(…, Slidock.inlineDeck(…))`.
pub const PLAYER: Script = Script {
    file: "player.iife.js",
    global: "Slidock",
    calls: &["mount", "inlineDeck"],
};

/// The presenter view, which the boot script hands to `Slidock.mount` as it is.
pub const PRESENTER: Script = Script {
    file: "presenter.iife.js",
    global: "SlidockPresenter",
    calls: &[],
};

/// Refuses a script that cannot sit inside `<script>…</script>` unchanged: text that would end the element early
/// (`</script`), change how the parser reads it (`<!--`) or change what the parser hands the engine (a carriage return
/// or a NUL, which it rewrites).
pub fn check_inline(script: &str, path: &str) -> Result<(), String> {
    let lower = script.to_ascii_lowercase();
    for refused in ["</script", "<!--", "\r", "\0"] {
        if lower.contains(refused) {
            return Err(format!(
                "{path} contains {refused:?}, which would break the inline <script> of every deck"
            ));
        }
    }
    Ok(())
}

/// Refuses a script that lacks the global or a call the boot script names: the decks it bundles would not play.
pub fn check_defines(script: &Script, text: &str, path: &str) -> Result<(), String> {
    let missing: Vec<&str> = std::iter::once(script.global)
        .chain(script.calls.iter().copied())
        .filter(|name| !text.contains(name))
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    Err(format!(
        "{path} does not define {}, which every deck's boot script calls: run `pnpm player:build`",
        missing.join(", ")
    ))
}

/// The player's version as `player/version.txt` holds it: one line, a semver-shaped version.
pub fn parse_version(text: &str, path: &str) -> Result<String, String> {
    let version = text.strip_suffix('\n').unwrap_or(text);
    let valid = !version.is_empty()
        && version
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'+'));
    if valid {
        Ok(version.to_owned())
    } else {
        Err(format!(
            "{path}: {text:?} is not one line holding a version"
        ))
    }
}
