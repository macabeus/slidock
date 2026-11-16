//! The checks `build.rs` runs on the player it embeds.

#[path = "../build/embed.rs"]
mod embed;

use embed::{PLAYER as SCRIPT, PRESENTER, check_defines, check_inline, parse_version};
use slidock_bundle::player;

const PLAYER: &str =
    "var Slidock=(()=>{var d={};v(d,{inlineDeck:()=>i,mount:()=>m,version:()=>b});return d})();";

#[test]
fn the_names_checked_are_the_ones_the_boot_script_calls() {
    for boot in [player::BOOT, player::BOOT_WITH_PRESENTER] {
        for name in SCRIPT.calls {
            assert!(
                boot.contains(&format!("{}.{name}(", SCRIPT.global)),
                "{name}"
            );
        }
        assert_eq!(boot.matches(&format!("{}.", SCRIPT.global)).count(), 2);
    }
    assert!(!player::BOOT.contains(PRESENTER.global));
    assert!(player::BOOT_WITH_PRESENTER.contains(&format!("presenter: {} }}", PRESENTER.global)));
}

#[test]
fn embeds_the_crate_s_copies_once_they_pass_the_checks() {
    for (script, embedded) in [(SCRIPT, player::SCRIPT), (PRESENTER, player::PRESENTER)] {
        let path = format!("{}/player/{}", env!("CARGO_MANIFEST_DIR"), script.file);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), embedded, "{path}");
        assert_eq!(check_inline(embedded, &path), Ok(()));
        assert_eq!(check_defines(&script, embedded, &path), Ok(()));
    }
}

#[test]
fn a_player_that_defines_the_boot_calls_passes() {
    assert_eq!(check_defines(&SCRIPT, PLAYER, "p.js"), Ok(()));
}

#[test]
fn refuses_a_player_without_what_the_boot_script_calls() {
    // The stub player: it exports only its version.
    let stub = "var Slidock=(()=>{var d={};v(d,{version:()=>b});var b=\"0.1.0\";return d})();";
    let error = check_defines(&SCRIPT, stub, "p.js").unwrap_err();
    assert!(
        error.contains("p.js does not define mount, inlineDeck"),
        "{error}"
    );
    let no_global = PLAYER.replace("Slidock", "Player");
    let error = check_defines(&SCRIPT, &no_global, "p.js").unwrap_err();
    assert!(error.contains("define Slidock,"), "{error}");
}

#[test]
fn checks_the_presenter_view_for_its_global() {
    let presenter = "(()=>{globalThis.SlidockPresenter={}})();";
    assert_eq!(check_defines(&PRESENTER, presenter, "s.js"), Ok(()));
    let error = check_defines(&PRESENTER, PLAYER, "s.js").unwrap_err();
    assert!(
        error.contains("s.js does not define SlidockPresenter"),
        "{error}"
    );
}

#[test]
fn refuses_a_script_that_cannot_sit_inline() {
    assert_eq!(check_inline(PLAYER, "p.js"), Ok(()));
    for bad in ["a</SCRIPT>", "a<!--b", "a\r\nb", "a\0b"] {
        assert!(check_inline(bad, "p.js").is_err(), "{bad:?}");
    }
}

#[test]
fn reads_the_version_file() {
    assert_eq!(parse_version("0.1.0\n", "v").unwrap(), "0.1.0");
    assert_eq!(parse_version("1.2.3-rc.1", "v").unwrap(), "1.2.3-rc.1");
    for bad in ["", "\n", "0.1.0\n\n", "0.1.0\r\n", "0.1 0\n", "\"0.1.0\"\n"] {
        assert!(parse_version(bad, "v").is_err(), "{bad:?}");
    }
}
