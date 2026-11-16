//! A deck's font sources from the command line: `--fonts` folders as dropped files, Google Fonts by default (from a
//! prepared cache, so no test touches the network) only for catalog families and not at all with `--no-google` or
//! `--offline`, the bundled substitutes from the files built in, the faces drawn shipped, `--fonts-off` as the parity
//! path, and the source flags refused with `--emit`.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::path::Path;
use std::process::{Command, Output};

use serde_json::{Value, json};

const SCHEMA: &str = slidock::SCHEMA_URL;

/// Runs the CLI with `-v`, which prints the fonts each source added and every face shipped.
fn slidock(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_slidock"))
        .args(args)
        .arg("-v")
        .current_dir(dir)
        .env("SLIDOCK_CACHE_DIR", dir.join("cache"))
        .output()
        .unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

/// One slide with one run in `family`.
fn deck(family: &str) -> Value {
    let level = json!({
        "para": { "align": "left", "bullet": { "kind": "none" }, "indent": 0,
                  "lineSpacing": { "basis": "size-1.2", "kind": "multiple", "value": 1 }, "marginLeft": 0,
                  "marginRight": 0, "rtl": false, "spaceAfter": { "kind": "pt", "value": 0 },
                  "spaceBefore": { "kind": "pt", "value": 0 }, "tabSize": 72 },
        "run": { "baseline": 0, "caps": "none", "color": { "kind": "theme", "mods": [], "slot": "tx1" },
                 "font": { "kind": "theme", "slot": "minor" }, "highlight": null, "italic": false,
                 "letterSpacing": 0, "link": null, "scriptScale": 1, "size": 18, "strike": "none",
                 "synthesizeBold": true, "underline": "none", "weight": 400 }
    });
    let text: serde_json::Map<String, Value> =
        (0..9).map(|i| (i.to_string(), level.clone())).collect();
    let shape = json!({
        "id": "ir-s1.t", "origin": { "kind": "agent" }, "kind": "shape",
        "frame": { "x": 60, "y": 160, "w": 600, "h": 80, "rotation": 0, "flipH": false, "flipV": false },
        "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
        "text": { "body": {}, "paragraphs": [{ "level": 0, "props": {}, "inlines": [{ "kind": "run", "text": "Hi",
            "props": { "font": { "kind": "family", "family": family }, "size": 24,
                       "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } } }] }] }
    });
    json!({
        "$schema": SCHEMA, "size": { "w": 720, "h": 405 },
        "defaults": { "body": { "anchor": "top", "autofit": { "kind": "none" },
                                "insets": { "b": 3.6, "l": 7.2, "r": 7.2, "t": 3.6 }, "overflow": "spread",
                                "wrap": "square" }, "text": text },
        "themes": {}, "masters": {}, "layouts": {}, "fonts": {}, "media": {}, "phase": "source",
        "slides": [{ "id": "ir-s1", "origin": { "kind": "agent" }, "hidden": false, "showLayout": true,
                     "timeline": [], "elements": [shape] }]
    })
}

/// Barlow Condensed Regular cut to a few letters (OFL-1.1), from slidock-fonts' fixtures: a face browsers take, so it
/// ships.
fn barlow() -> Vec<u8> {
    std::fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../slidock-fonts/tests/data/container/barlow.ttf"),
    )
    .unwrap()
}

fn workspace(family: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("deck.presentation-ir.json"),
        serde_json::to_vec(&deck(family)).unwrap(),
    )
    .unwrap();
    dir
}

/// Puts a URL's body in the Google Fonts cache.
fn cache(dir: &Path, url: &str, body: &[u8]) {
    let folder = dir.join("cache/google");
    std::fs::create_dir_all(&folder).unwrap();
    std::fs::write(folder.join(slidock_canon::sha256_hex(url.as_bytes())), body).unwrap();
}

#[test]
fn the_source_flags_choose_a_deck_s_sources_and_are_refused_with_emit() {
    let dir = workspace("Roboto");
    for flag in [
        "--offline",
        "--no-google",
        "--system-fonts",
        "--no-embedded",
        "--no-substitutes",
        "--fonts-off",
    ] {
        let out = slidock(
            dir.path(),
            &["compile", "deck.presentation-ir.json", "--emit", "ir", flag],
        );
        assert_eq!(out.status.code(), Some(2), "{flag}");
        let err = stderr(&out);
        assert!(
            err.contains(&format!(
                "the argument '{flag}' cannot be used with '--emit <WHAT>'"
            )) || err.contains(&format!(
                "the argument '--emit <WHAT>' cannot be used with '{flag}'"
            )),
            "{err}"
        );
    }
    let out = slidock(
        dir.path(),
        &[
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "deck.zip",
            "--no-embedded",
            "--offline",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let out = slidock(
        dir.path(),
        &[
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "deck.zip",
            "--fonts-off",
            "--offline",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
    let out = slidock(
        dir.path(),
        &[
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "deck.zip",
            "--fonts-off",
            "--no-google",
        ],
    );
    assert_eq!(out.status.code(), Some(2), "{}", stderr(&out));
}

/// Whether the binary has the bundled substitutes built in (feature `bundled-substitutes`).
const BUILT_IN: bool = cfg!(feature = "bundled-substitutes");

/// A compile whose deck draws Liberation Sans ships the built-in file, or, built without the files, warns and ships
/// none.
fn assert_bundled_liberation_sans(out: &Output) {
    let err = stderr(out);
    let missing = "warning font LiberationSans: not among the bundled files";
    if BUILT_IN {
        assert!(
            err.contains("font bundled: LiberationSans (Liberation Sans substitute)"),
            "{err}"
        );
        assert!(
            err.contains("font ship: Liberation Sans 400 normal (bundled, "),
            "{err}"
        );
        assert!(!err.contains("warning"), "{err}");
    } else {
        assert!(err.contains(missing), "{err}");
        assert!(!err.contains("font bundled"), "{err}");
        assert!(!err.contains("font ship: Liberation Sans"), "{err}");
    }
}

/// The `@font-face` rules of a deck zip's `index.html`.
fn font_faces(zip: &Path) -> usize {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip).unwrap()).unwrap();
    let mut html = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("index.html").unwrap(), &mut html).unwrap();
    html.matches("@font-face{").count()
}

#[test]
fn a_fonts_folder_is_a_deck_s_dropped_files() {
    let dir = workspace("Barlow Condensed");
    std::fs::create_dir(dir.path().join("fonts")).unwrap();
    std::fs::write(dir.path().join("fonts/barlow.ttf"), barlow()).unwrap();
    let out = slidock(
        dir.path(),
        &[
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "deck.zip",
            "--fonts",
            "fonts",
            "--no-google",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains("font user: BarlowCondensed-Regular ("),
        "{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("font ship: Barlow Condensed 400 normal (user, "),
        "{}",
        stderr(&out)
    );
    // The bundled Liberation Sans ships for the deck's default theme font (the paragraph mark), and the last-resort
    // DejaVu Sans for the letters the cut file lacks.
    assert_bundled_liberation_sans(&out);
    assert_eq!(
        stderr(&out).contains("font ship: DejaVu Sans 400 normal (bundled, "),
        BUILT_IN,
        "{}",
        stderr(&out)
    );
    assert_eq!(
        font_faces(&dir.path().join("deck.zip")),
        1 + 2 * usize::from(BUILT_IN)
    );
    // With --fonts-off the folder joins the built-in fonts, as with --emit: nothing is added or shipped.
    let off = slidock(
        dir.path(),
        &[
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "off.zip",
            "--fonts",
            "fonts",
            "--fonts-off",
        ],
    );
    assert_eq!(off.status.code(), Some(0), "{}", stderr(&off));
    assert!(!stderr(&off).contains("font "), "{}", stderr(&off));
    assert_eq!(font_faces(&dir.path().join("off.zip")), 0);
}

/// `font` with `OS/2.fsType` set to `fs_type`.
fn with_fs_type(font: &[u8], fs_type: u16) -> Vec<u8> {
    let mut bytes = font.to_vec();
    let tables = usize::from(u16::from_be_bytes([bytes[4], bytes[5]]));
    let record = (0..tables)
        .map(|i| 12 + 16 * i)
        .find(|&at| &bytes[at..at + 4] == b"OS/2")
        .unwrap();
    let offset = u32::from_be_bytes(bytes[record + 8..record + 12].try_into().unwrap()) as usize;
    bytes[offset + 8..offset + 10].copy_from_slice(&fs_type.to_be_bytes());
    bytes
}

#[test]
fn a_restricted_face_never_draws_in_the_download_the_cli_writes() {
    let dir = workspace("Barlow Condensed");
    std::fs::create_dir(dir.path().join("fonts")).unwrap();
    std::fs::write(
        dir.path().join("fonts/barlow.ttf"),
        with_fs_type(&barlow(), 0x0002),
    )
    .unwrap();
    let out = slidock(
        dir.path(),
        &[
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "deck.zip",
            "--fonts",
            "fonts",
            "--no-google",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let err = stderr(&out);
    assert!(
        err.contains(
            "MID5413 Barlow Condensed (BarlowCondensed-Regular, user): the face is restricted from embedding; its \
             substitute draws instead"
        ),
        "{err}"
    );
    assert!(!err.contains("font ship: Barlow Condensed"), "{err}");
    // The deck's diagnostic says the family was found, and why it is not drawn.
    assert!(
        err.contains(
            "MID5404 font-refused ir-s1: Barlow Condensed is found but cannot be used: its licence (fsType) restricts embedding it, \
             so a download leaves it out; Liberation Sans stands in"
        ),
        "{err}"
    );
    assert_eq!(
        font_faces(&dir.path().join("deck.zip")),
        usize::from(BUILT_IN)
    );
}

#[test]
fn google_fonts_fill_a_catalog_family_from_the_cache() {
    let dir = workspace("Barlow Condensed");
    let css2 = "https://fonts.googleapis.com/css2?family=Barlow+Condensed:wght@400&display=block";
    let file = "https://fonts.gstatic.com/s/barlowcondensed/v1/test.ttf";
    let css = format!(
        "@font-face {{ font-family: 'Barlow Condensed'; font-style: normal; font-weight: 400; src: url({file}) format('truetype'); }}"
    );
    cache(dir.path(), css2, css.as_bytes());
    cache(dir.path(), file, &barlow());
    let out = slidock(
        dir.path(),
        &["compile", "deck.presentation-ir.json", "-o", "deck.zip"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(
        stderr(&out).contains(&format!("font google: BarlowCondensed-Regular ({file})")),
        "{}",
        stderr(&out)
    );
    assert!(
        stderr(&out).contains("font ship: Barlow Condensed 400 normal (google, "),
        "{}",
        stderr(&out)
    );
    // Negative control: with --no-google or --offline, nothing is taken from Google Fonts, cached or not.
    for flag in ["--no-google", "--offline"] {
        let off = slidock(
            dir.path(),
            &[
                "compile",
                "deck.presentation-ir.json",
                "-o",
                "off.zip",
                flag,
            ],
        );
        assert_eq!(off.status.code(), Some(0), "{flag}: {}", stderr(&off));
        assert!(
            !stderr(&off).contains("font google"),
            "{flag}: {}",
            stderr(&off)
        );
        assert!(
            !stderr(&off).contains("googleapis"),
            "{flag}: {}",
            stderr(&off)
        );
        assert!(
            !stderr(&off).contains("font ship: Barlow Condensed"),
            "{flag}: {}",
            stderr(&off)
        );
    }
}

#[test]
fn offline_makes_no_google_fonts_request() {
    // The cache is empty, and every proxy points at a closed port: a request would print a `GET` error.
    let dir = workspace("Barlow Condensed");
    let mut command = Command::new(env!("CARGO_BIN_EXE_slidock"));
    command
        .args([
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "deck.zip",
            "--offline",
        ])
        .current_dir(dir.path())
        .env("SLIDOCK_CACHE_DIR", dir.path().join("cache"));
    for name in [
        "ALL_PROXY",
        "HTTPS_PROXY",
        "HTTP_PROXY",
        "all_proxy",
        "https_proxy",
        "http_proxy",
    ] {
        command.env(name, "http://127.0.0.1:1");
    }
    let out = command.output().unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!stderr(&out).contains("googleapis"), "{}", stderr(&out));
    assert!(!stderr(&out).contains("GET "), "{}", stderr(&out));
    assert!(!dir.path().join("cache").exists());
}

#[test]
fn google_fonts_never_requests_a_family_outside_the_open_catalog() {
    // css2 serves a licensed Helvetica Neue; the catalog has no such family, so no request is made (this test has no
    // network and no cache: a request would print a warning). Its substitute's file is not taken.
    let dir = workspace("Helvetica Neue");
    let out = slidock(
        dir.path(),
        &[
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "deck.zip",
            "--no-substitutes",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!stderr(&out).contains("font "), "{}", stderr(&out));
}

#[test]
fn the_substitute_faces_a_deck_draws_ship_from_the_built_in_files_offline_too() {
    // Helvetica Neue binds to Liberation Sans, a bundled substitute: its file is the web app's WOFF2, built in, so
    // nothing is downloaded or cached.
    let dir = workspace("Helvetica Neue");
    let out = slidock(
        dir.path(),
        &[
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "deck.zip",
            "--offline",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert_bundled_liberation_sans(&out);
    assert!(!dir.path().join("cache").exists());
    assert_eq!(
        font_faces(&dir.path().join("deck.zip")),
        usize::from(BUILT_IN)
    );
    // Negative control: with --no-substitutes the deck ships no face, drawing it with the viewer's fonts.
    let out = slidock(
        dir.path(),
        &[
            "compile",
            "deck.presentation-ir.json",
            "-o",
            "none.zip",
            "--offline",
            "--no-substitutes",
        ],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(!stderr(&out).contains("font bundled"), "{}", stderr(&out));
    assert_eq!(font_faces(&dir.path().join("none.zip")), 0);
}
