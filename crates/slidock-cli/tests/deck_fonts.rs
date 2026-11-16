//! A corpus deck compiled with its embedded fonts (`--features golden`, `$SLIDOCK_PRIVATE/corpus` as the golden files
//! pin it; skips when they are absent): the `pptx-talk` PPTX's 20 EOT parts unwrap, 12 prove their identity and 8 are
//! demoted (their `name` and `OS/2` tables give another family, weight or style than the slot they fill), and the
//! faces the deck draws ship, each listed in `LICENSES.txt`. With Google Fonts off, the demoted Regular files draw the
//! Medium faces no source has, and the bundled Liberation Sans (the web app's file, built into the CLI) draws Arial.
#![cfg(feature = "golden")]
#![allow(clippy::unwrap_used, clippy::indexing_slicing, reason = "test code")]

mod support;

use std::io::Read;
use std::path::Path;
use std::process::{Command, Output};

use serde_json::Value;
use support::golden;

const DECK: &str = "corpus:pptx-talk.pptx";

fn entry(zip: &Path, name: &str) -> String {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip).unwrap()).unwrap();
    let mut text = String::new();
    archive
        .by_name(name)
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    text
}

fn compile(input: &Path, zip: &Path, cache: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_slidock"))
        .args([
            "compile".as_ref(),
            input.as_os_str(),
            "-o".as_ref(),
            zip.as_os_str(),
        ])
        .args(["--no-google", "-v"])
        .args(extra)
        .env("SLIDOCK_CACHE_DIR", cache)
        .output()
        .unwrap()
}

/// The deck JSON's faces in `index.html`: family, weight, style and source.
fn deck_faces(html: &str) -> Vec<String> {
    let start = html.find("\"fonts\":[{").unwrap() + "\"fonts\":".len();
    let mut stream = serde_json::Deserializer::from_str(&html[start..]).into_iter::<Value>();
    let fonts = stream.next().unwrap().unwrap();
    fonts
        .as_array()
        .unwrap()
        .iter()
        .map(|face| {
            format!(
                "{} {} {} {}",
                face["family"].as_str().unwrap(),
                face["weight"],
                face["style"].as_str().unwrap(),
                face["source"].as_str().unwrap()
            )
        })
        .collect()
}

#[test]
fn ships_the_decks_embedded_faces_after_the_identity_check() {
    let Some(golden) = golden() else {
        return;
    };
    let input = golden.input(DECK);
    let mut deck = zip::ZipArchive::new(std::fs::File::open(&input).unwrap()).unwrap();
    let parts = (0..deck.len())
        .filter(|&i| {
            deck.by_index(i)
                .unwrap()
                .name()
                .unwrap()
                .ends_with(".fntdata")
        })
        .count();
    assert_eq!(parts, 20);

    let out = tempfile::tempdir().unwrap();
    let zip = out.path().join("deck.zip");
    let cache = tempfile::tempdir().unwrap();
    let run = compile(&input, &zip, cache.path(), &[]);
    let stderr = String::from_utf8(run.stderr).unwrap();
    assert!(run.status.code().unwrap() <= 1, "{stderr}");
    let warnings: Vec<&str> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("warning font "))
        .collect();
    let demoted: Vec<&str> = warnings
        .iter()
        .filter_map(|line| line.strip_prefix("MID5410 "))
        .map(|line| line.split(':').next().unwrap())
        .collect();
    assert_eq!(
        demoted,
        [
            "Oswald Medium 400",
            "Oswald Medium 700",
            "Roboto Medium 400",
            "Roboto Medium 700",
            "Roboto Medium 400 italic",
            "Roboto Medium 700 italic",
            "Barlow Condensed 700",
            "Barlow Condensed 700 italic",
        ]
    );
    // Every other part was read and accepted: no unreadable, refused or unshipped face, no missing substitute.
    assert_eq!(warnings.len(), demoted.len(), "{warnings:?}");
    // The embedded fonts are unwrapped and shipped: no diagnostic says otherwise.
    for code in ["PX2501", "HTML6004"] {
        assert!(!stderr.contains(code), "{code}: {stderr}");
    }
    let shipped: Vec<&str> = stderr
        .lines()
        .filter_map(|line| line.strip_prefix("font ship: "))
        .collect();
    let name = |line: &&str| line.split(" (").next().unwrap().to_owned();
    assert_eq!(
        shipped.iter().map(name).collect::<Vec<_>>(),
        [
            "Barlow Condensed 400 normal",
            "Liberation Sans 400 normal",
            "Liberation Sans 700 normal",
            "Oswald 400 normal",
            "Oswald 700 normal",
            "Roboto 400 normal",
            "Roboto 700 normal",
            "Roboto Mono 400 normal",
            // The last resort: the ❖ and ☺ the Barlow Condensed subset lacks.
            "DejaVu Sans 400 normal",
        ]
    );
    for line in &shipped {
        let source = if line.starts_with("Liberation Sans") || line.starts_with("DejaVu Sans") {
            "(bundled, "
        } else {
            "(embedded, "
        };
        assert!(line.contains(source) && line.contains(", subset"), "{line}");
    }
    // The Roboto and Oswald regular files are embedded for Roboto Medium and Oswald Medium too, which no source has:
    // they draw the Medium the deck draws (500), as demoted files.
    let stand_ins: Vec<String> = shipped
        .iter()
        .filter(|line| line.ends_with(", demoted)"))
        .map(name)
        .collect();
    assert_eq!(stand_ins, ["Oswald 400 normal", "Roboto 400 normal"]);

    let html = entry(&zip, "index.html");
    for code in ["PX2501", "HTML6004"] {
        assert!(!html.contains(code), "{code}");
    }
    assert_eq!(html.matches("@font-face{").count(), 9);
    // Every stack names the shipped families, never the requested Arial an installed font could draw.
    assert!(!html.contains("font-family:&quot;Arial"));
    assert!(html.contains("font-family:&quot;Liberation Sans&quot;,sans-serif"));
    assert!(
        html.contains(
            "font-family:&quot;Barlow Condensed&quot;,&quot;DejaVu Sans&quot;,sans-serif"
        )
    );
    let faces = deck_faces(&html);
    for face in [
        "Arial 400 normal bundled",
        "Arial 700 normal bundled",
        "Oswald Medium 500 normal embedded",
        "Roboto Medium 500 normal embedded",
    ] {
        assert!(faces.iter().any(|f| f == face), "{face}: {faces:?}");
    }
    let licences = entry(&zip, "LICENSES.txt");
    for face in shipped {
        let origin = if face.starts_with("Liberation Sans") || face.starts_with("DejaVu Sans") {
            "a slidock substitute"
        } else {
            "embedded in the presentation"
        };
        let name = face.split(" (").next().unwrap();
        assert!(licences.contains(&format!("{name} ({origin})")), "{name}");
    }

    // Without the embedded fonts, the frontend's PX2501 and the backend's HTML6004 stand: nothing unwraps them.
    let without = out.path().join("without.zip");
    let run = compile(&input, &without, cache.path(), &["--no-embedded"]);
    let stderr = String::from_utf8(run.stderr).unwrap();
    assert_eq!(stderr.matches("PX2501").count(), 6, "{stderr}");
    assert!(stderr.contains("HTML6004"), "{stderr}");
}
