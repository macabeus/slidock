//! Keynote's attended catalog (`probes/keynote/RUNBOOK.md`): Keynote 14.5's own files, read from
//! `$SLIDOCK_PRIVATE/probes-incoming/keynote` (default `~/.slidock`), pinned by sha256 and never committed. A test
//! skips with its reason printed when the files are absent, and fails when one is present with another hash.
//!
//! - `transition-catalog.key`: one slide per transition the scripting dictionary knows (42 applied, 1.5 s, no
//!   direction stored); `transition-catalog.pptx` is Keynote's PPTX export of it, `transition-catalog.m4v` its movie
//!   (`probes/keynote/movie-transitions.sh` measures it: 42 transitions of 1.5 s after 5 s holds, slide N's effect
//!   between slides N and N + 1, slide 3's fade through black dark at mid-transition, the wipe's edge feathered and
//!   moving right, the iris opening from the centre).
//! - `pptx-import.key`: `probes/out/motion-probe.pptx` opened in Keynote and saved: Keynote's reading of every probe
//!   effect, direction and trigger, held against the probe's expectations (`probes/out/motion-probe.expect.json`).
//! - `build-a.key`, `build-b.key`, `build-wipe.key`: copies of `pptx-import.key` trimmed to its slides 142–159,
//!   160–180 and 148–151, each exported as a self-playing 1080p movie (`.m4v`). `build-a.pairs.json` and
//!   `build-b.pairs.json` pair every build of the first two with its run of change in the movie (time, length, box);
//!   the contact sheets (`build-*-iris-zoom.png`, `build-wipe-148.png`) and `build-movie-readings.md` hold what the
//!   movies show: the build wipe's feathered edge, the iris as a circle, an exit's in/out naming what the element
//!   does, the zoom fading, and the self-playing cadence.
//! - `build-subset.key`: `pptx-import.key` trimmed to its slides 142–210; `build-c1.m4v`…`build-c4.m4v` are movies of
//!   its slides 40–47 (64 motion-path presets) and `build-d1.m4v` of its slides 48–65 (slide 65: a custom curve).
//!   `build-paths.json` tracks each motion-path build's element frame by frame in those movies, in slide units.
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::path::PathBuf;

use serde_json::{Value, json};
use slidock_io::{FileReader, RangeReader, ZipArchive, ZipLimits, sha256_range};
use support::harness::{Lowered, lower_native_reader, lower_reader};

/// The attended files, with their sha256 as `manifest.json` records them.
const PINS: [(&str, &str); 24] = [
    (
        "transition-catalog.key",
        "70c791971c821335f20ab053fa9860106614c7604a0ea0e559a40f0cda22295e",
    ),
    (
        "transition-catalog.pptx",
        "3a38ebecc2c15f9ac435f5d7b77fddbc09d2630df2f2023e9aabe79a73d0fd58",
    ),
    (
        "transition-catalog.m4v",
        "dee11562cba7fd842e4f18bfb76913897ee9d7643daeb179b2894b52027d5eca",
    ),
    (
        "pptx-import.key",
        "343426f79593086c38ef43da672f1cf9ffaf3c587ba818e08970a23af7868697",
    ),
    (
        "motion-probe-source.pptx",
        "6bd51a2685d22cb1774f10b3fd156316d34c4d43e9aa181eb23e5fbcfadc849f",
    ),
    (
        "build-a.key",
        "011d45997a71b4267b9125810066ac1071a319e2e28c60b2fafff0f288841333",
    ),
    (
        "build-a.m4v",
        "028ac4d1990e92493ab6be986447c572d9d0f47f60db1c10f12de7e2c9bd36f5",
    ),
    (
        "build-a.pairs.json",
        "8287fa6ded27da870759780d672ea16844ae88a9f9258130168f5651ff6360c7",
    ),
    (
        "build-a-iris-zoom.png",
        "c5c61440ea7f7c4cfaaeb163db216b59c1861bf31db2579a8058fbf19a3df6a6",
    ),
    (
        "build-b.key",
        "f8ec605fd35db7eb06312d8929760e325fc6360995a6b5d0f4a81f26eab4d32a",
    ),
    (
        "build-b.m4v",
        "cdacc9fa68e9e9069fbd84f0d3c0c0877fdb5f4d43ad16b8710c334b734f9179",
    ),
    (
        "build-b.pairs.json",
        "a9cfc30956138d5a32580b98a9aaa9f563495aa7150f16c66b3ecfab52ca79c0",
    ),
    (
        "build-b-iris-zoom.png",
        "4ee17a71b3004f53b2fe50b7fc1d7816ed134a33073d03c3d9918b6be20700a7",
    ),
    (
        "build-wipe.key",
        "ee12c491a1bd6a48ec26f968e5245647faff3baa40c887a13ffd4cd7f555a94d",
    ),
    (
        "build-wipe.m4v",
        "5fcaa73a7f9e79c6b74e8e5e561be130db1282d83ed21d647914f5a1527fd06f",
    ),
    (
        "build-wipe-148.png",
        "14322e57c93969c2ec45f2b557316e296ebba4c3516bc3eddf5c05d6ffb1c242",
    ),
    (
        "build-movie-readings.md",
        "1980aaec677b3d4aa9e8bdb13c903d3920563dab5c90b8a7bec20c69816b41f7",
    ),
    (
        "build-subset.key",
        "4a2ecbb67214900a7ecde2658f6870cfe1b0d80a15002ef0ec7f3682a6fc6f09",
    ),
    (
        "build-c1.m4v",
        "9ad69e1e50130e58eb010dbbc4e685d3a5cbd1368a0175a2ec2256fdb07ee4da",
    ),
    (
        "build-c2.m4v",
        "009758abe17c4231fdd3287286828166ab8234907478c8882ae5b60b9ba05250",
    ),
    (
        "build-c3.m4v",
        "22bbfa8c50735a4b769d6bf8f299a7aec56783ac0b6c973947f90326b7b097e8",
    ),
    (
        "build-c4.m4v",
        "3d213326beef29f1cb1fa7cc2f9fc0c9569b38590e1418bc863dc9e94a635547",
    ),
    (
        "build-d1.m4v",
        "6835c1c164a02c1f03c1808ff3e43b6d1ec59cb87d1ab8ca42416fda6aee550f",
    ),
    (
        "build-paths.json",
        "7272813bf3a1acb746dc5d713f46dfa7792468222401909e03bc01c3805c4bcc",
    ),
];

/// Keynote's id for each catalog slide's transition, slide 1 first: the dictionary's term on the slide's title,
/// stored as this id (`transition-catalog.tsv`).
const CATALOG: [&str; 44] = [
    "apple:magic-move-implied-motion-path",
    "apple:dissolve",
    "com.apple.iWork.Keynote.BLTFadeThruColor",
    "apple:fade-and-move",
    "apple:slide",
    "apple:push",
    "apple:reveal",
    "apple:wipe",
    "apple:wipe-iris",
    "apple:scale",
    "apple:FlipThrough",
    "com.apple.iWork.Keynote.KLNSwap",
    "apple:ca-swing",
    "com.apple.iWork.Keynote.BLTSwoosh",
    "apple:twirl",
    "com.apple.iWork.Keynote.BUKTwist",
    "apple:3D-cube",
    "apple:doorway",
    "apple:bounce",
    "apple:droplet",
    "apple:fall",
    "apple:revolve",
    "com.apple.iWork.Keynote.BUKFlop",
    "apple:apple-grid",
    "com.apple.iWork.Keynote.BLTMosaicFlip",
    "apple:pageflip",
    "apple:ca-isometric",
    "apple:pivot",
    "com.apple.iWork.Keynote.BLTReflection",
    "com.apple.iWork.Keynote.BLTRevolvingDoor",
    "com.apple.iWork.Keynote.BLTBlinds",
    "apple:ClotheslinePush",
    "com.apple.iWork.Keynote.KLNColorPlanes",
    "com.apple.iWork.Keynote.KLNConfetti",
    "apple:ca-text-shimmer",
    "apple:ca-text-sparkle",
    "apple:ca-cube",
    "apple:ca-dissolve-and-flip",
    "apple:ca-pop",
    "apple:ca-push",
    "apple:ca-revolve",
    "apple:ca-zoom",
    "none",
    "none",
];

/// The probe slides whose builds Keynote's import changed (its warnings: emphasis builds removed, build order
/// changed), left out of the build comparison.
const IMPORT_CHANGED: [u64; 9] = [160, 161, 162, 163, 164, 165, 191, 196, 197];

/// The probe slides written anew after Keynote imported the probe (Flash Once's switch as `effects.xml` writes it, no
/// longer held), left out of the source comparison.
const REWRITTEN: [usize; 1] = [145];

/// The probe slides appended after Keynote imported the probe (`ft-line-fade`, a line's fade), left out of the source
/// comparison with the entries that list them.
const APPENDED: [usize; 1] = [211];

/// The probe's slide number of a part of an appended slide (`ppt/slides/slideN.xml` or its rels), or `None`.
fn appended_slide(name: &str) -> Option<usize> {
    let rest = name
        .strip_prefix("ppt/slides/_rels/slide")
        .and_then(|rest| rest.strip_suffix(".xml.rels"))
        .or_else(|| {
            name.strip_prefix("ppt/slides/slide")
                .and_then(|rest| rest.strip_suffix(".xml"))
        })?;
    rest.parse().ok().filter(|n| APPENDED.contains(n))
}

/// `xml` without its first element that starts with `open` and holds `needle`, and that element's text.
fn without_element(xml: &str, open: &str, needle: &str) -> (String, String) {
    let mut from = 0;
    while let Some(at) = xml[from..].find(open) {
        let start = from + at;
        let end = start + xml[start..].find("/>").unwrap() + "/>".len();
        if xml[start..end].contains(needle) {
            return (
                format!("{}{}", &xml[..start], &xml[end..]),
                xml[start..end].to_owned(),
            );
        }
        from = end;
    }
    panic!("no {open} element holds {needle}");
}

/// A part that lists the slides, without the entries of the appended ones: the content type override, the
/// presentation's relationship, and the slide id that names it.
fn without_appended(name: &str, xml: &str) -> String {
    let mut xml = xml.to_owned();
    for n in APPENDED {
        let part = format!("slides/slide{n}.xml\"");
        xml = match name {
            "[Content_Types].xml" => without_element(&xml, "<Override ", &format!("/ppt/{part}")).0,
            "ppt/_rels/presentation.xml.rels" => without_element(&xml, "<Relationship ", &part).0,
            "ppt/presentation.xml" => xml,
            _ => return xml,
        };
    }
    xml
}

fn incoming() -> PathBuf {
    let private = std::env::var("SLIDOCK_PRIVATE")
        .unwrap_or_else(|_| format!("{}/.slidock", std::env::var("HOME").unwrap()));
    PathBuf::from(private).join("probes-incoming/keynote")
}

/// The pinned file `name`, or `None` (with the reason printed) when it is absent.
fn attended(name: &str) -> Option<FileReader> {
    let path = incoming().join(name);
    let Ok(reader) = FileReader::open(&path) else {
        eprintln!(
            "skipped: {} is absent (the attended Keynote step, probes/keynote/RUNBOOK.md)",
            path.display()
        );
        return None;
    };
    let pin = PINS
        .iter()
        .find(|(file, _)| *file == name)
        .map(|(_, sha)| *sha)
        .unwrap();
    let sha = sha256_range(&reader, 0, reader.len()).unwrap();
    assert_eq!(sha, pin, "{} is not the pinned file", path.display());
    Some(reader)
}

fn slides(lowered: &Lowered) -> &Vec<Value> {
    lowered.json["slides"].as_array().unwrap()
}

fn count(lowered: &Lowered, code: &str) -> usize {
    lowered.codes().iter().filter(|c| **c == code).count()
}

/// A slide's XML with the text of its runs left out: a probe slide's label names the effect it expects.
fn without_run_text(xml: &str) -> String {
    let mut out = String::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<a:t>") {
        out.push_str(&rest[..start + "<a:t>".len()]);
        rest = &rest[start..];
        rest = &rest[rest.find("</a:t>").unwrap()..];
    }
    out.push_str(rest);
    out
}

/// The attended files are the pinned ones (the movie included), and Keynote imported the committed PPTX probe: the
/// source it opened has the probe's parts, each the same apart from its slide labels, except on the slides the
/// import comparison leaves out ([`IMPORT_CHANGED`], [`OTHER_COUNT`]), those written anew since ([`REWRITTEN`]) and
/// those appended since ([`APPENDED`]), with the entries that list them.
#[test]
fn the_attended_files_are_the_pinned_ones() {
    let present = PINS
        .iter()
        .filter(|(name, _)| attended(name).is_some())
        .count();
    eprintln!("{present} of {} attended files present", PINS.len());
    let Some(source) = attended("motion-probe-source.pptx") else {
        return;
    };
    let probe = FileReader::open(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out/motion-probe.pptx"),
    )
    .unwrap();
    let probe = ZipArchive::open(&probe, ZipLimits::default()).unwrap();
    let source = ZipArchive::open(&source, ZipLimits::default()).unwrap();
    let names = |archive: &ZipArchive<&FileReader>| {
        let mut names: Vec<String> = archive.entries().iter().map(|e| e.name.clone()).collect();
        names.sort();
        names
    };
    let ours_names: Vec<String> = names(&probe)
        .into_iter()
        .filter(|name| appended_slide(name).is_none())
        .collect();
    assert_eq!(ours_names, names(&source));
    let bytes = |archive: &ZipArchive<&FileReader>, name: &str| {
        archive
            .read(archive.get(name).unwrap())
            .unwrap()
            .into_owned()
    };
    // The presentation's slide ids name the appended slides by the relationship ids the rels part gives them.
    let rels = zip_text(&probe, "ppt/_rels/presentation.xml.rels");
    let appended_ids: Vec<String> = APPENDED
        .iter()
        .map(|n| {
            let (_, element) =
                without_element(&rels, "<Relationship ", &format!("slides/slide{n}.xml\""));
            let id = &element[element.find(" Id=\"").unwrap() + " Id=\"".len()..];
            format!("r:id=\"{}\"", &id[..id.find('"').unwrap()])
        })
        .collect();
    for name in ours_names {
        if bytes(&probe, &name) == bytes(&source, &name) {
            continue;
        }
        let (mut ours, theirs) = (zip_text(&probe, &name), zip_text(&source, &name));
        ours = without_appended(&name, &ours);
        if name == "ppt/presentation.xml" {
            for id in &appended_ids {
                ours = without_element(&ours, "<p:sldId ", id).0;
            }
        }
        if ours == theirs {
            continue;
        }
        let slide: Option<usize> = name
            .strip_prefix("ppt/slides/slide")
            .and_then(|rest| rest.strip_suffix(".xml"))
            .and_then(|n| n.parse().ok());
        let left_out = slide.is_some_and(|n| {
            IMPORT_CHANGED.contains(&(n as u64))
                || OTHER_COUNT.contains(&n)
                || REWRITTEN.contains(&n)
        });
        assert!(
            left_out || (slide.is_some() && without_run_text(&ours) == without_run_text(&theirs)),
            "{name} differs from the source Keynote imported"
        );
    }
}

/// Every transition and build of Keynote's own decks plays a vocabulary entry, or its fallback reported with
/// KN1402/KN1403; the transition catalog's are each played into the slide after the one storing them.
#[test]
fn keynote_catalog_decks_play_native_or_their_fallback() {
    for name in ["transition-catalog.key", "pptx-import.key"] {
        let Some(reader) = attended(name) else {
            return;
        };
        let lowered = lower_native_reader(&reader, None);
        let (mut transitions, mut steps) = (0, 0);
        let (mut transitions_unplayed, mut steps_unplayed) = (0, 0);
        for slide in slides(&lowered) {
            let transition = &slide["transition"];
            if transition.is_object() {
                transitions += 1;
                transitions_unplayed += usize::from(transition["effect"]["native"].is_null());
            }
            for step in slide["timeline"].as_array().into_iter().flatten() {
                steps += 1;
                steps_unplayed += usize::from(step["effect"]["native"].is_null());
            }
        }
        assert!(transitions_unplayed <= count(&lowered, "KN1403"), "{name}");
        assert!(steps_unplayed <= count(&lowered, "KN1402"), "{name}");
        eprintln!(
            "{name}: {transitions} transitions ({transitions_unplayed} fallback), {steps} steps ({steps_unplayed} fallback)"
        );
    }
    let Some(reader) = attended("transition-catalog.key") else {
        return;
    };
    let stored = lower_reader(&reader, None, slidock_core_api::Limits::default());
    let played = lower_native_reader(&reader, None);
    let effect_of = |slide: &Value| slide["transition"]["effect"]["id"].clone();
    let id = |effect: &str| json!(format!("keynote:{}", effect.replace(' ', "_")));
    assert_eq!(slides(&played).len(), CATALOG.len());
    assert!(slides(&played)[0].get("transition").is_none());
    for (n, effect) in CATALOG.iter().enumerate() {
        if *effect == "none" {
            assert!(
                slides(&stored)[n].get("transition").is_none(),
                "slide {}",
                n + 1
            );
            continue;
        }
        // The parity path keeps it on the slide that stores it; the deck path plays it into the next, 1.5 s long
        // as Keynote's movie plays it.
        assert_eq!(
            effect_of(&slides(&stored)[n]),
            id(effect),
            "slide {}",
            n + 1
        );
        let into = &slides(&played)[n + 1];
        assert_eq!(effect_of(into), id(effect), "slide {}", n + 2);
        assert_eq!(into["transition"]["durationMs"], json!(1500));
    }
    // The last slide's own transition is none: nothing is left out.
    assert!(!played.diagnostics.iter().any(|d| d.code.code() == "KN1409"));
}

fn zip_text(archive: &ZipArchive<&FileReader>, name: &str) -> String {
    let entry = archive.get(name).unwrap();
    String::from_utf8(archive.read(entry).unwrap().into_owned()).unwrap()
}

/// The `p:transition` choice Keynote writes on a slide of its PPTX export: the first child of the first
/// `p:transition` element, from its name to the end of its tag; `None` for an empty one (a cut).
fn exported_transition(xml: &str) -> Option<String> {
    let start = xml.find("<p:transition")?;
    let rest = &xml[start..];
    let open = rest.find('>')?;
    if rest[..open].ends_with('/') {
        return None;
    }
    let body = &rest[open + 1..];
    if body.starts_with("</p:transition>") {
        return None;
    }
    let end = body.find('>')?;
    Some(body[..=end].to_owned())
}

/// Keynote's PPTX export writes catalog slide N's transition on slide N + 1, the IR's convention (a transition into
/// its slide), and writes its default wipe moving right and its default push, move in and reveal moving left.
#[test]
fn keynotes_export_writes_each_transition_into_the_next_slide() {
    let Some(reader) = attended("transition-catalog.pptx") else {
        return;
    };
    let archive = ZipArchive::open(&reader, ZipLimits::default()).unwrap();
    let exported: Vec<Option<String>> = (1..=CATALOG.len())
        .map(|n| exported_transition(&zip_text(&archive, &format!("ppt/slides/slide{n}.xml"))))
        .collect();
    assert_eq!(exported[0], None);
    for (n, effect) in CATALOG.iter().enumerate().skip(1) {
        let previous = CATALOG[n - 1];
        assert_eq!(
            exported[n].is_some(),
            previous != "none",
            "slide {}: {:?} after {previous}, before {effect}",
            n + 1,
            exported[n]
        );
    }
    for (n, child) in [
        (2, "<p159:morph option=\"byObject\"/>"),
        (4, "<p:fade thruBlk=\"1\"/>"),
        (6, "<p:cover dir=\"l\"/>"),
        (7, "<p:push dir=\"l\"/>"),
        (8, "<p:pull dir=\"l\"/>"),
        (9, "<p:wipe dir=\"r\"/>"),
        (10, "<p:circle/>"),
        (11, "<p14:warp dir=\"in\"/>"),
    ] {
        assert_eq!(exported[n - 1].as_deref(), Some(child), "slide {n}");
    }
}

/// The catalog's own defaults as the deck path plays them: Keynote's movie plays the wipe feathered and moving
/// right, the iris opening, the scale growing the incoming slide, the push moving left, and the move in sliding the
/// incoming slide over the still outgoing one (a cover, as the export writes it, `<p:cover dir="l"/>`), unreported.
#[test]
fn catalog_defaults_play_as_keynotes_movie_shows_them() {
    let Some(reader) = attended("transition-catalog.key") else {
        return;
    };
    let played = lower_native_reader(&reader, None);
    let native = |n: usize| slides(&played)[n]["transition"]["effect"]["native"].clone();
    // Catalog slide 8 (wipe) plays into slide 9, and so on.
    assert_eq!(
        native(8),
        json!({ "id": "keynote:apple:wipe", "effect": "wipe", "params": { "dir": "r", "soft": true } })
    );
    assert_eq!(native(9)["params"]["inOut"], json!("out"));
    assert_eq!(native(10)["params"]["inOut"], json!("in"));
    assert_eq!(
        native(5),
        json!({ "id": "keynote:apple:slide", "effect": "cover", "params": { "dir": "l" } })
    );
    assert!(
        !played
            .diagnostics
            .iter()
            .any(|d| d.code.code() == "KN1404" && d.message.contains("apple:slide")),
    );
    assert_eq!(native(6)["params"], json!({ "dir": "l" }));
    assert_eq!(native(7)["params"], json!({ "dir": "l" }));
}

/// The catalog stores a timing curve only on its Magic Move (4, ease-in-out). Its movie plays five of the others,
/// which store none, each on its own curve over the whole 1.5 s (mean progress error of the fit, against CSS
/// ease-in-out's): the dissolve (11.467–12.967 s, 0.0014), the fade through black (18.033–19.533 s, 0.0052) and the
/// push (37.533–39.033 s, 0.0035) on ease-in-out; the move in (31.0–32.5 s) on a curve fast from the start (0.0014,
/// in-out 0.017), the reveal (44.0–45.5 s) on one slow from the start (0.0024, in-out 0.015). Each reads its row's own
/// curve; a row whose effect the movie did not measure carries none, the player's default.
#[test]
fn catalog_transitions_play_their_effects_own_curve_as_keynotes_movie_plays_them() {
    let Some(reader) = attended("transition-catalog.key") else {
        return;
    };
    let played = lower_native_reader(&reader, None);
    let measured = |id: &str| match id {
        "keynote:apple:magic-move-implied-motion-path"
        | "keynote:apple:dissolve"
        | "keynote:com.apple.iWork.Keynote.BLTFadeThruColor"
        | "keynote:apple:push"
        | "keynote:apple:ca-push" => json!("in-out"),
        "keynote:apple:slide" => json!({ "bezier": [0.62, 0.92, 0.62, 0.98] }),
        "keynote:apple:reveal" => json!({ "bezier": [0.48, 0, 0.56, 0.32] }),
        _ => Value::Null,
    };
    let (mut native, mut own) = (0, 0);
    for (n, slide) in slides(&played).iter().enumerate() {
        let transition = &slide["transition"];
        if transition.is_null() || transition["effect"]["native"].is_null() {
            continue;
        }
        let id = transition["effect"]["id"].as_str().unwrap();
        let expected = measured(id);
        own += usize::from(!expected.is_null());
        assert_eq!(transition["easing"], expected, "slide {} {id}", n + 1);
        native += 1;
    }
    assert_eq!((native, own), (27, 7));
    assert!(!played.diagnostics.iter().any(|d| d.code.code() == "KN1408"));
}

/// The probe's expectation for slide `index` (1-based).
fn expected_slide(expect: &Value, index: usize) -> &Value {
    &expect["decks"]["motion-probe.pptx"]["slideList"][index - 1]
}

/// What one pair of entries (Keynote's reading of a probe effect, the probe's expectation) shows.
#[derive(Debug, Default)]
struct Comparison {
    /// Params compared: `dir` wherever both carry one, the rest where both play the same effect.
    compared: usize,
    /// Differences, as `what effect key: ours vs theirs`.
    differing: Vec<(String, String, String)>,
    /// Pairs where Keynote plays another effect than the probe expects.
    other_effect: Vec<String>,
    /// Probe slides whose step count Keynote's import changed.
    other_count: Vec<usize>,
    /// Steps of a preset with no OOXML sub-type: Keynote picks its own effect and direction.
    own_choice: usize,
}

impl Comparison {
    fn pair(&mut self, what: String, native: &Value, expected: &Value) {
        if native["effect"].is_null() || expected["effect"].is_null() {
            return;
        }
        let same = native["effect"] == expected["effect"];
        if !same {
            self.other_effect.push(format!(
                "{what}: {} vs {}",
                native["effect"], expected["effect"]
            ));
        }
        let keys: Vec<String> = match native["params"].as_object() {
            Some(params) if same => params.keys().cloned().collect(),
            _ => vec!["dir".to_owned()],
        };
        for key in keys {
            let (Some(ours), Some(theirs)) =
                (native["params"].get(&key), expected["params"].get(&key))
            else {
                continue;
            };
            self.compared += 1;
            if *ours != ooxml_token(theirs) {
                self.differing.push((
                    format!("{what} {}", native["effect"]),
                    key,
                    format!("{ours} vs {theirs}"),
                ));
            }
        }
    }
}

/// An expected param in the vocabulary's spelling: the probe's expectation spells a doors orientation out
/// (`horizontal`), the vocabulary writes OOXML's token (`horz`).
fn ooxml_token(value: &Value) -> Value {
    match value.as_str() {
        Some("horizontal") => json!("horz"),
        Some("vertical") => json!("vert"),
        _ => value.clone(),
    }
}

/// The differences between Keynote's reading of the probe and the probe's expectation that come from Keynote itself,
/// each with its source-app reason: `(effect, param, kind)`, the kind `"exit"`, `"entrance"` or `"transition"` (any
/// when empty).
const KEYNOTE_DIFFERS: [(&str, &str, &str, &str); 6] = [
    (
        "iris",
        "shape",
        "",
        "Keynote's iris has one shape, the circle: the import reads box, diamond and plus as it, and its build movies \
         play each as a circle",
    ),
    (
        "blinds",
        "orient",
        "",
        "Keynote's blinds are vertical, as its movie of the transition catalog plays them",
    ),
    (
        "doors",
        "orient",
        "",
        "Keynote's doorway opens vertically, as its movie of the transition catalog plays it",
    ),
    (
        "zoom",
        "origin",
        "",
        "Keynote's scale build has no origin option: its import keeps PowerPoint's zoom and drops the slide-centre \
         origin",
    ),
    (
        "zoom",
        "slight",
        "",
        "Keynote's scale build has no slight option: its import keeps PowerPoint's zoom and drops the slight scale",
    ),
    (
        "iris",
        "inOut",
        "exit",
        "PPTX only: Keynote plays PowerPoint's exit iris `in` (`box(in)`, transition out) closing to the centre, the \
         `out` entrance reversed (its build movie), and exports that build as `box(in)` again; slidock plays the stated \
         filter backwards, SMIL's `mode=\"out\"` as LibreOffice's engine plays it, a hole opening from the centre",
    ),
];

/// Keynote's import of the PPTX motion probe, played on the deck path, lines up with the probe slide by slide (its
/// transitions move back onto the slides PowerPoint stores them on). Every pair is accounted for: a direction number
/// reads as the OOXML token it came from wherever both sides carry a `dir` (whatever effect each plays), the other
/// params agree wherever both play the same effect unless Keynote itself differs ([`KEYNOTE_DIFFERS`]), and the pairs
/// that cannot compare are counted: another effect, a step count Keynote's import changed, a preset with no sub-type.
#[test]
fn keynotes_import_of_the_pptx_probe_reads_as_the_probe_expects() {
    let Some(reader) = attended("pptx-import.key") else {
        return;
    };
    let expect: Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../probes/out/motion-probe.expect.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let played = lower_native_reader(&reader, None);
    let mut seen = Comparison::default();
    for (k, slide) in slides(&played).iter().enumerate() {
        let index = k + 1;
        let expected = expected_slide(&expect, index);
        if expected["transition"].is_object() {
            seen.pair(
                format!("slide {index} transition"),
                &slide["transition"]["effect"]["native"],
                &expected["transition"],
            );
        }
        if IMPORT_CHANGED.contains(&(index as u64)) {
            continue;
        }
        let ours = slide["timeline"].as_array().cloned().unwrap_or_default();
        let theirs = expected["steps"].as_array().cloned().unwrap_or_default();
        if ours.len() != theirs.len() {
            seen.other_count.push(index);
            continue;
        }
        for (i, (step, want)) in ours.iter().zip(&theirs).enumerate() {
            if !want["native"].as_str().is_some_and(|id| id.contains('/')) {
                seen.own_choice += 1;
                continue;
            }
            let kind = want["kind"].as_str().unwrap_or_default();
            seen.pair(
                format!("slide {index} step {i} {kind}"),
                &step["effect"]["native"],
                want,
            );
        }
    }
    let unexplained: Vec<&(String, String, String)> = seen
        .differing
        .iter()
        .filter(|(what, key, _)| {
            !KEYNOTE_DIFFERS.iter().any(|(effect, param, kind, _)| {
                what.ends_with(&format!("\"{effect}\""))
                    && key == param
                    && (kind.is_empty() || what.contains(kind))
            })
        })
        .collect();
    eprintln!(
        "pptx-import.key: {} params compared, {} differ as Keynote does ({}), {} pairs play another effect, {} slides \
         another step count {:?}, {} steps Keynote's own choice",
        seen.compared,
        seen.differing.len(),
        KEYNOTE_DIFFERS.len(),
        seen.other_effect.len(),
        seen.other_count.len(),
        seen.other_count,
        seen.own_choice
    );
    assert!(unexplained.is_empty(), "{unexplained:#?}");
    assert!(
        seen.differing.iter().all(|(_, key, _)| key != "dir"),
        "{:#?}",
        seen.differing
    );
    assert!(seen.compared >= 100, "{} params compared", seen.compared);
    // What does not compare stays small and known: Keynote's import changed these slides' step counts.
    assert_eq!(seen.other_count, OTHER_COUNT, "{:#?}", seen.other_effect);
    let inferred: Vec<&String> = played
        .diagnostics
        .iter()
        .map(|d| &d.message)
        .filter(|m| m.contains("is read by inference"))
        .collect();
    for message in inferred {
        let number: u32 = message
            .split("direction ")
            .nth(1)
            .and_then(|rest| rest.split(' ').next())
            .and_then(|n| n.parse().ok())
            .unwrap();
        assert!(
            [41, 42, 43, 44].contains(&number),
            "a pinned number was read by inference: {message}"
        );
    }
}

/// The probe slides whose step count Keynote's import changed without a warning (beside [`IMPORT_CHANGED`]): slide
/// 207 plays, pauses and stops a clip, and Keynote's media builds have no pause.
const OTHER_COUNT: [usize; 1] = [207];

/// Keynote's import writes PowerPoint's click, with-previous and after-previous as its own chunks (referent false
/// with, true after; event triggers 0, 1, 2): the deck path starts each step where the probe expects it (slides
/// 189 and 190; the import drops slide 198's auto-reverse, so its after step starts earlier).
#[test]
fn keynotes_triggers_start_steps_where_the_probe_expects() {
    let Some(reader) = attended("pptx-import.key") else {
        return;
    };
    let expect: Value = serde_json::from_str(
        &std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../probes/out/motion-probe.expect.json"),
        )
        .unwrap(),
    )
    .unwrap();
    let played = lower_native_reader(&reader, None);
    for index in [189, 190] {
        let ours: Vec<Value> = slides(&played)[index - 1]["timeline"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["timing"]["offsetMs"].clone())
            .collect();
        let theirs: Vec<Value> = expected_slide(&expect, index)["steps"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["offsetMs"].clone())
            .collect();
        assert_eq!(ours, theirs, "slide {index}");
        let id = slides(&played)[index - 1]["id"]
            .as_str()
            .unwrap()
            .to_owned();
        let reported: Vec<&str> = played
            .diagnostics
            .iter()
            .filter(|d| d.container.as_ref().is_some_and(|c| c.as_str() == id))
            .filter(|d| d.message.contains("trigger") || d.message.contains("referent"))
            .map(|d| d.message.as_str())
            .collect();
        assert!(reported.is_empty(), "slide {index}: {reported:?}");
    }
}

/// A pinned JSON file, or `None` when it is absent.
fn attended_json(name: &str) -> Option<Value> {
    attended(name)?;
    Some(serde_json::from_slice(&std::fs::read(incoming().join(name)).unwrap()).unwrap())
}

/// The build movies: each trimmed copy, the import slide its first slide is, its slide count, and its pairs.
const MOVIES: [(&str, usize, usize, &str); 2] = [
    ("build-a.key", 142, 18, "build-a.pairs.json"),
    ("build-b.key", 160, 21, "build-b.pairs.json"),
];

/// The PPTX preset a probe label names, `exit.4/16` in `exit-box in [pptx exit.4/16]: …`.
fn label_preset(label: &str) -> &str {
    label
        .split("[pptx ")
        .nth(1)
        .and_then(|rest| rest.split(']').next())
        .unwrap_or_default()
}

/// The trimmed copies keep the import's builds, and each build of theirs pairs, in order, with its run of change in
/// its movie (99 and 96 builds, every slide's count matching): the pairs carry the step's kind, effect, duration,
/// delivery, trigger and delay as the deck path reads them. The movie plays them at Keynote's self-playing cadence:
/// the first build 2.0 s after its slide appears, each next 2.0 s after the previous one ends (within 0.25 s, the
/// movie's 30 fps and the change threshold), a 500 ms build's visible change 0.25–0.55 s long and an appear's one
/// frame.
#[test]
fn keynotes_build_movies_pair_every_build_with_its_change() {
    let Some(import) = attended("pptx-import.key") else {
        return;
    };
    let import = lower_native_reader(&import, None);
    for (name, first, count, pairs_name) in MOVIES {
        let (Some(reader), Some(pairs)) = (attended(name), attended_json(pairs_name)) else {
            return;
        };
        let played = lower_native_reader(&reader, None);
        assert_eq!(slides(&played).len(), count, "{name}");
        let pairs = pairs.as_array().unwrap();
        let mut paired = 0;
        for (k, slide) in slides(&played).iter().enumerate() {
            let index = first + k;
            let steps = slide["timeline"].as_array().cloned().unwrap_or_default();
            assert_eq!(
                steps,
                slides(&import)[index - 1]["timeline"]
                    .as_array()
                    .cloned()
                    .unwrap_or_default(),
                "{name} slide {index}"
            );
            let runs: Vec<&Value> = pairs
                .iter()
                .filter(|pair| pair["slide"] == json!(index))
                .collect();
            assert_eq!(steps.len(), runs.len(), "{name} slide {index}");
            let mut previous_end: Option<f64> = None;
            for (step, run) in steps.iter().zip(&runs) {
                let id = step["effect"]["id"].as_str().unwrap();
                assert_eq!(
                    run["effect"],
                    json!(id.rsplit(':').next().unwrap()),
                    "{name} {index}"
                );
                for (ours, theirs) in [
                    ("kind", "kind"),
                    ("durationMs", "ms"),
                    ("delivery", "delivery"),
                    ("trigger", "trigger"),
                    ("delayMs", "delayMs"),
                ] {
                    assert_eq!(step[ours], run[theirs], "{name} {index} {ours}");
                }
                let t = run["t"].as_f64().unwrap();
                let len = run["len"].as_f64().unwrap();
                let since = t - previous_end.unwrap_or(run["slideStart"].as_f64().unwrap());
                assert!((since - 2.0).abs() <= 0.25, "{name} {index} {t}: {since}");
                let appear = step["effect"]["native"]["effect"] == json!("appear");
                let played_s = if appear {
                    assert!(len <= 0.15, "{name} {index} {t}: {len}");
                    0.0
                } else {
                    let ms = run["ms"].as_f64().unwrap() / 1000.0;
                    if ms == 0.5 {
                        assert!((0.25..=0.55).contains(&len), "{name} {index} {t}: {len}");
                    }
                    ms
                };
                previous_end = Some(t + played_s);
                paired += 1;
            }
        }
        assert_eq!(paired, pairs.len(), "{name}");
        eprintln!("{name}: {paired} builds paired with their runs of change");
    }
}

/// What Keynote's build movies show, as the deck path plays it, pair by pair: every wipe has a feathered edge and
/// moves as the movie shows it (slide 148: a split horizontal top to bottom, a split vertical left to right, the
/// diagonal strips from the corner opposite where they head); every iris is a circle; the zoom fades; an iris or zoom
/// `in`/`out` (PPTX sub-type 16/32, Keynote's 41/42 and 43/44) plays an entrance as named and an exit as the
/// reverse of the entrance of the other name (exit iris 41 closes to the centre, exit zoom 43 grows, 44 shrinks; the
/// contract and fade-out-and-zoom exits shrink, magnify grows). None of it is reported as a reading by inference.
#[test]
fn keynotes_build_movies_settle_the_wipe_iris_and_zoom() {
    let mut held = [0usize; 4];
    for (name, first, _, pairs_name) in MOVIES {
        let (Some(reader), Some(pairs)) = (attended(name), attended_json(pairs_name)) else {
            return;
        };
        let played = lower_native_reader(&reader, None);
        for pair in pairs.as_array().unwrap() {
            let index = pair["slide"].as_u64().unwrap() as usize;
            let slide = &slides(&played)[index - first];
            let place = pairs
                .as_array()
                .unwrap()
                .iter()
                .filter(|p| p["slide"] == pair["slide"])
                .position(|p| p == pair)
                .unwrap();
            let step = &slide["timeline"][place];
            let native = &step["effect"]["native"];
            let label = pair["label"].as_str().unwrap();
            let preset = label_preset(label);
            let exit = pair["kind"] == json!("exit");
            match pair["effect"].as_str().unwrap() {
                "wipe" => {
                    assert_eq!(native["params"]["soft"], json!(true), "{label}");
                    let shown = [
                        ("split horizontal", "d"),
                        ("split vertical", "r"),
                        ("corner=rd", "ld"),
                        ("corner=ru", "lu"),
                    ]
                    .iter()
                    .find(|(what, _)| index == 148 && label.contains(what));
                    if let Some((_, dir)) = shown {
                        assert_eq!(native["params"]["dir"], json!(dir), "{label}");
                        held[0] += 1;
                    }
                }
                "wipe-iris" => {
                    assert_eq!(native["effect"], json!("iris"), "{label}");
                    assert_eq!(native["params"]["shape"], json!("circle"), "{label}");
                    held[1] += 1;
                }
                "zoom" => {
                    assert_eq!(native["effect"], json!("zoom"), "{label}");
                    assert_eq!(native["params"]["fade"], json!(true), "{label}");
                    held[2] += 1;
                }
                _ => continue,
            }
            let shown = match (preset.split_once('/').map(|(_, sub)| sub), exit) {
                (Some("16"), false) | (Some("32"), true) => Some("in"),
                (Some("32"), false) | (Some("16"), true) => Some("out"),
                (None, true) if preset == "exit.55" || preset == "exit.53" => Some("in"),
                (None, true) if preset == "exit.51" => Some("out"),
                _ => None,
            };
            if let (Some(in_out), "iris" | "zoom") = (shown, native["effect"].as_str().unwrap()) {
                assert_eq!(native["params"]["inOut"], json!(in_out), "{label}");
                held[3] += 1;
            }
        }
        let read: Vec<&String> = played
            .diagnostics
            .iter()
            .map(|d| &d.message)
            .filter(|m| m.contains("by inference") || m.contains("hard edge"))
            .collect();
        assert!(read.is_empty(), "{name}: {read:#?}");
    }
    eprintln!(
        "build movies: {} wipe directions, {} irises, {} zooms, {} in/out readings held",
        held[0], held[1], held[2], held[3]
    );
    // The pinned movies hold 6 wipe directions on slide 148, 18 irises, 21 zooms and 23 in/out readings.
    assert_eq!(held, [6, 18, 21, 23]);
}

/// Slide units (the IR's points; the movies' 960 × 540) per slide pixel, the unit a translate track moves by.
const UNITS_PER_PX: f64 = 0.75;

/// A track's position at `progress` (0 to 1): its keyframes joined by straight lines, as the player plays them.
fn track_at(frames: &[(f64, f64, f64)], progress: f64) -> (f64, f64) {
    let after = frames
        .iter()
        .position(|f| f.0 >= progress)
        .unwrap_or(frames.len() - 1);
    let (o1, x1, y1) = frames[after];
    let Some(&(o0, x0, y0)) = after.checked_sub(1).map(|k| &frames[k]) else {
        return (x1, y1);
    };
    let k = if o1 > o0 {
        (progress - o0) / (o1 - o0)
    } else {
        1.0
    };
    (x0 + (x1 - x0) * k, y0 + (y1 - y0) * k)
}

/// How far `p` is from the track's path.
fn to_path(frames: &[(f64, f64, f64)], (px, py): (f64, f64)) -> f64 {
    frames
        .windows(2)
        .map(|leg| {
            let ((_, ax, ay), (_, bx, by)) = (leg[0], leg[1]);
            let (dx, dy) = (bx - ax, by - ay);
            let length = dx * dx + dy * dy;
            let t = if length > 0.0 {
                (((px - ax) * dx + (py - ay) * dy) / length).clamp(0.0, 1.0)
            } else {
                0.0
            };
            slidock_math::hypot(px - ax - t * dx, py - ay - t * dy)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Every motion-path build of the build movies (64 presets on subset slides 40–47, the custom curve on 65) plays
/// natively as Keynote's movie shows it (`build-paths.json`): a translate over its stored 500 ms from the element's
/// static position, along the stored nodes in the slide's own units (the element frames' points), the way back from
/// a closed path's last node a straight line, linear by distance. Every measured frame lies within 0.75 slide units
/// of the played path and within 2.5 of where it plays at that time (the run's start phase as measured: within a
/// frame); a measured end is the played end. Keynote's movie measures neither `curveStyle`, the offsets, align to
/// path nor the flips: none is set on these builds.
#[test]
fn motion_path_builds_play_the_tracks_keynotes_movies_show() {
    let movies = [
        "build-c1.m4v",
        "build-c2.m4v",
        "build-c3.m4v",
        "build-c4.m4v",
        "build-d1.m4v",
    ];
    if movies.iter().any(|movie| attended(movie).is_none()) {
        return;
    }
    let (Some(subset), Some(readings)) = (
        attended("build-subset.key"),
        attended_json("build-paths.json"),
    ) else {
        return;
    };
    let played = lower_native_reader(&subset, None);
    let builds = readings["builds"].as_array().unwrap();
    assert_eq!(builds.len(), 65);
    let (mut on_path, mut in_time, mut ends) = (0.0_f64, 0.0_f64, 0);
    for build in builds {
        let label = build["label"].as_str().unwrap();
        let slide = &slides(&played)[build["slide"].as_u64().unwrap() as usize - 1];
        let steps: Vec<&Value> = slide["timeline"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|step| step["target"]["element"] == build["element"])
            .collect();
        let [step] = steps.as_slice() else {
            panic!("{label}: {} steps", steps.len());
        };
        assert_eq!(
            step["effect"]["id"],
            json!("keynote:apple:action-motion-path")
        );
        assert!(step.get("easing").is_none(), "{label}");
        let native = &step["effect"]["native"];
        assert_eq!(native["effect"], json!("behaviors"), "{label}");
        let [track] = native["params"].as_array().unwrap().as_slice() else {
            panic!("{label}: {native}");
        };
        assert_eq!(track["property"], json!("translate"), "{label}");
        assert_eq!(track["beginMs"], json!(0), "{label}");
        assert!(track.get("easing").is_none(), "{label}");
        let duration = build["native"]["durationMs"].as_f64().unwrap();
        assert_eq!(track["durationMs"].as_f64(), Some(duration), "{label}");
        assert_eq!(step["durationMs"].as_f64(), Some(duration), "{label}");
        let frames: Vec<(f64, f64, f64)> = track["frames"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| {
                let value = |axis: &str| f["value"][axis].as_f64().unwrap() * UNITS_PER_PX;
                (f["offset"].as_f64().unwrap(), value("x"), value("y"))
            })
            .collect();
        let measured = &build["measured"];
        assert_eq!(frames[0], (0.0, 0.0, 0.0), "{label}");
        assert_eq!(measured["startOffset"], json!([0.0, 0.0]), "{label}");
        let t0 = build["metrics"]["timing"]["linear"]["t0"].as_f64().unwrap();
        assert!(t0.abs() < 1.0 / 30.0, "{label}: starts {t0} s late");
        let seconds = duration / 1000.0;
        for point in measured["points"].as_array().unwrap() {
            let [t, x, y] = [0, 1, 2].map(|k| point[k].as_f64().unwrap());
            let path = to_path(&frames, (x, y));
            assert!(path <= 0.75, "{label} at {t} s: {path} off the path");
            on_path = on_path.max(path);
            let (ex, ey) = track_at(&frames, ((t - t0) / seconds).clamp(0.0, 1.0));
            let error = slidock_math::hypot(x - ex, y - ey);
            assert!(
                error <= 2.5,
                "{label} at {t} s: ({x}, {y}) against ({ex}, {ey})"
            );
            in_time = in_time.max(error);
        }
        if measured["endMeasured"] == json!(true) {
            let end = measured["end"].as_array().unwrap();
            let last = frames[frames.len() - 1];
            let (x, y) = (end[0].as_f64().unwrap(), end[1].as_f64().unwrap());
            assert!(
                slidock_math::hypot(x - last.1, y - last.2) <= 0.75,
                "{label}: ends {last:?}"
            );
            ends += 1;
        }
    }
    let reported: Vec<&String> = played
        .diagnostics
        .iter()
        .map(|d| &d.message)
        .filter(|m| m.contains("motion path"))
        .collect();
    assert!(reported.is_empty(), "{reported:#?}");
    eprintln!(
        "motion paths: {} builds native, at most {on_path:.2} off the path and {in_time:.2} off in time, {ends} ends",
        builds.len()
    );
}
