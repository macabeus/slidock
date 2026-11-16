//! The player every deck embeds, `@slidock/player`'s IIFE build, and the presenter view a deck embeds when it ships
//! one, the presenter's IIFE build, each embedded from the crate's `player/` folder by `build.rs` (see there for how a
//! player build reaches it); and the boot script that starts the player on the deck page.

use slidock_canon::sha256_hex;
use slidock_html::deck::PlayerStamp;

mod built {
    include!(concat!(env!("OUT_DIR"), "/player.rs"));
}

/// `player.iife.js`, which defines the global `Slidock`.
pub const SCRIPT: &str = include_str!(concat!(env!("OUT_DIR"), "/player.iife.js"));

/// `presenter.iife.js`, which defines the global `SlidockPresenter` on a page whose `Slidock` is defined.
pub const PRESENTER: &str = include_str!(concat!(env!("OUT_DIR"), "/presenter.iife.js"));

/// The version of the embedded player (and presenter view).
pub const VERSION: &str = built::VERSION;

/// Starts the player on the deck page with the deck JSON beside it, with no presenter view.
pub const BOOT: &str = "Slidock.mount(document.body, Slidock.inlineDeck(document))";

/// Starts the player on the deck page with the deck JSON beside it and the presenter view.
pub const BOOT_WITH_PRESENTER: &str =
    "Slidock.mount(document.body, Slidock.inlineDeck(document), { presenter: SlidockPresenter })";

/// The page's scripts after the deck JSON, in order: the player, the presenter view's when it ships one, the boot.
pub fn scripts(presenter: bool) -> Vec<&'static str> {
    if presenter {
        vec![SCRIPT, PRESENTER, BOOT_WITH_PRESENTER]
    } else {
        vec![SCRIPT, BOOT]
    }
}

pub(crate) fn stamp(presenter: bool) -> PlayerStamp {
    PlayerStamp {
        version: VERSION.to_owned(),
        sha256: sha256_hex(SCRIPT),
        presenter: presenter.then(|| sha256_hex(PRESENTER)),
    }
}
