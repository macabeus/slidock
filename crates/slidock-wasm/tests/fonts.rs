//! The worker's host with fonts on: a presentation's embedded faces ship, `finish` re-runs after a font
//! arrives and equals one finish with every font, a restricted face draws in the preview and ships only when its
//! family is included, a left-out family ships no file, and `LICENSES.txt` lists every shipped face.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use serde_json::{Value, json};
use slidock::{BundleOptions, BytesReader, FontSource};
use slidock_io::{ZipArchive, ZipLimits};
use slidock_wasm::host::{Host, Printed, Swaps};
use slidock_wasm::protocol::ErrorKind;

/// The font probe LibreOffice wrote with its fonts embedded (`probes/fonts/build.sh`; OFL faces).
const PROBE: &[u8] = include_bytes!("../../../probes/out/fonts-probe.odp");
/// Barlow Condensed Regular cut to a few letters (OFL-1.1), from slidock-fonts' fixtures.
const BARLOW: &[u8] = include_bytes!("../../slidock-fonts/tests/data/container/barlow.ttf");
const NAME: &str = "barlow.presentation-ir.json";
const SCHEMA: &str = slidock::SCHEMA_URL;

/// One slide of Barlow Condensed text, as agent-written IR.
fn barlow_deck() -> Vec<u8> {
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
        "text": { "body": {}, "paragraphs": [{ "level": 0, "props": {}, "inlines": [{ "kind": "run",
            "text": "Bars cog", "props": { "font": { "kind": "family", "family": "Barlow Condensed" }, "size": 24,
                       "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } } }] }] }
    });
    serde_json::to_vec(&json!({
        "$schema": SCHEMA, "size": { "w": 720, "h": 405 },
        "defaults": { "body": { "anchor": "top", "autofit": { "kind": "none" },
                                "insets": { "b": 3.6, "l": 7.2, "r": 7.2, "t": 3.6 }, "overflow": "spread",
                                "wrap": "square" }, "text": text },
        "themes": {}, "masters": {}, "layouts": {}, "fonts": {}, "media": {}, "phase": "source",
        "slides": [{ "id": "ir-s1", "origin": { "kind": "agent" }, "hidden": false, "showLayout": true,
                     "timeline": [], "elements": [shape] }]
    }))
    .unwrap()
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

fn open(name: &str, bytes: Vec<u8>) -> Host {
    let mut host = Host::new();
    host.open(name, Box::new(BytesReader::new(bytes)), true)
        .unwrap();
    host
}

fn finished(host: &mut Host) -> Printed {
    host.finish().unwrap();
    host.print().unwrap()
}

fn bundled(host: &mut Host, options: &BundleOptions) -> (Vec<u8>, slidock::BundleReport) {
    let mut zip = Vec::new();
    let report = host
        .bundle(options, Swaps::default(), &mut |chunk| {
            zip.extend_from_slice(chunk);
            Ok(())
        })
        .unwrap();
    (zip, report)
}

/// A zip entry's text.
fn entry(zip: &[u8], name: &str) -> String {
    let archive = ZipArchive::open(BytesReader::new(zip.to_vec()), ZipLimits::default()).unwrap();
    let found = archive.entries().iter().find(|e| e.name == name).unwrap();
    String::from_utf8(archive.read(found).unwrap().into_owned()).unwrap()
}

fn font_faces(html: &str) -> usize {
    html.matches("@font-face{").count()
}

#[test]
fn the_probe_deck_ships_its_embedded_faces_and_lists_each_in_licenses() {
    let mut host = open("fonts-probe.odp", PROBE.to_vec());
    let printed = finished(&mut host);
    let faces: Vec<String> = printed
        .fonts
        .iter()
        .map(|face| {
            format!(
                "{} {} {:?}",
                face.licence.family, face.licence.weight, face.style
            )
        })
        .collect();
    assert_eq!(
        faces,
        [
            "Barlow Condensed 400 Normal",
            "Barlow Condensed 700 Normal",
            "Caveat 400 700 Normal",
            "Lato 400 Normal",
            "Lato 400 Italic",
            "Lato 700 Normal",
            "Lato 700 Italic",
            "PT Serif 400 Normal",
            "PT Serif 400 Italic",
            "PT Serif 700 Normal",
            "PT Serif 700 Italic",
        ]
    );
    assert!(
        printed
            .fonts
            .iter()
            .all(|face| face.source == FontSource::Embedded && face.subset && !face.notice)
    );
    // Two embedded files are declared for an italic face they are not; the text draws neither italic face, so no
    // shipped face stands in for one.
    let mislabeled: Vec<&str> = printed
        .font_issues
        .iter()
        .map(|issue| issue.code())
        .filter(|code| *code == "MID5410")
        .collect();
    assert_eq!(mislabeled.len(), 2);
    let report = printed.faces();
    assert_eq!(report.len(), 11);
    assert!(report.iter().all(|face| face.shipped && face.bytes > 0));
    assert!(report.iter().all(|face| !face.demoted && !face.restricted));
    assert_eq!(printed.font_rules().count(), 11);
    let (zip, bundle) = bundled(&mut host, &BundleOptions::default());
    let html = entry(&zip, "index.html");
    assert!(html.contains("<style id=\"sd-fonts\">@font-face{font-family:\"Barlow Condensed\";"));
    assert_eq!(font_faces(&html), 11);
    let licences = entry(&zip, "LICENSES.txt");
    for face in &printed.fonts {
        assert!(
            licences.contains(&face.licence.text()),
            "{}",
            face.licence.family
        );
    }
    assert!(bundle.fonts_left_out.is_empty());
    // The file of each rule is the bytes the zip embeds.
    assert_eq!(host.font_bytes(0).unwrap(), printed.fonts[0].rules[0].bytes);
    assert_eq!(host.font_bytes(11).unwrap_err().kind, ErrorKind::Request);
}

#[test]
fn finishing_again_after_a_font_arrives_equals_one_finish_with_it() {
    let mut late = open(NAME, barlow_deck());
    let before = finished(&mut late);
    assert!(before.fonts.is_empty());
    let needs = late.font_needs().unwrap();
    assert!(needs.iter().any(|need| need.family == "Barlow Condensed"));
    late.add_font(BARLOW.to_vec(), FontSource::User, None)
        .unwrap();
    let after = finished(&mut late);
    assert_eq!(after.fonts.len(), 1);
    let face = &after.fonts[0];
    assert_eq!(
        (face.source, face.notice, face.licence.family.as_str()),
        (FontSource::User, true, "Barlow Condensed")
    );
    let (late_zip, late_report) = bundled(&mut late, &BundleOptions::default());

    let mut early = open(NAME, barlow_deck());
    early
        .add_font(BARLOW.to_vec(), FontSource::User, None)
        .unwrap();
    assert_eq!(finished(&mut early), after);
    let (early_zip, early_report) = bundled(&mut early, &BundleOptions::default());
    assert!(early_zip == late_zip, "the zips differ");
    assert_eq!(early_report, late_report);
    assert_eq!(font_faces(&entry(&late_zip, "index.html")), 1);
    assert_eq!(
        late_report.fonts_with_notice,
        Some(vec!["Barlow Condensed".to_owned()])
    );

    // Left out, the user's family ships no file, and no licence.
    let leave_out = BundleOptions {
        leave_out: ["Barlow Condensed".to_owned()].into(),
        ..BundleOptions::default()
    };
    let (zip, report) = bundled(&mut late, &leave_out);
    assert_eq!(report.fonts_left_out, ["Barlow Condensed"]);
    assert_eq!(font_faces(&entry(&zip, "index.html")), 0);
    assert!(!entry(&zip, "LICENSES.txt").contains("Barlow"));
}

#[test]
fn a_restricted_face_draws_in_the_preview_and_ships_in_the_download_only_when_included() {
    let mut host = open(NAME, barlow_deck());
    // Restricted licence embedding, and no editable or preview-and-print bit.
    host.add_font(with_fs_type(BARLOW, 0x0002), FontSource::User, None)
        .unwrap();
    let printed = finished(&mut host);
    assert_eq!(printed.fonts.len(), 1);
    assert!(printed.refused.is_empty());
    let issue = printed
        .font_issues
        .iter()
        .find(|issue| issue.code() == "MID5413")
        .unwrap();
    assert!(
        issue.to_string().contains(
            "the face is restricted from embedding; the preview draws with it, and the download leaves it out \
             unless its family is included"
        ),
        "{issue}"
    );
    let report = printed.faces();
    assert_eq!(report.len(), 1);
    assert!(report[0].restricted && report[0].shipped && report[0].notice);
    assert_eq!(report[0].serves[0].family, "Barlow Condensed");
    let drawn = printed
        .deck
        .fonts
        .iter()
        .find(|face| face.family == "Barlow Condensed")
        .unwrap();
    assert_eq!(drawn.source, slidock::deck::FontSource::User);

    // The download leaves it out by default.
    let (zip, report) = bundled(&mut host, &BundleOptions::default());
    assert_eq!(report.fonts_left_out, ["Barlow Condensed"]);
    assert_eq!(report.fonts_with_notice, None);
    assert_eq!(font_faces(&entry(&zip, "index.html")), 0);
    assert!(!entry(&zip, "LICENSES.txt").contains("Barlow"));

    // Included by the user, it ships with the notice.
    let include = BundleOptions {
        include: ["Barlow Condensed".to_owned()].into(),
        ..BundleOptions::default()
    };
    let (zip, report) = bundled(&mut host, &include);
    assert!(report.fonts_left_out.is_empty());
    assert_eq!(
        report.fonts_with_notice,
        Some(vec!["Barlow Condensed".to_owned()])
    );
    assert_eq!(font_faces(&entry(&zip, "index.html")), 1);
    assert!(
        entry(&zip, "LICENSES.txt").contains("Barlow Condensed 400 normal (added by the user)")
    );

    // A bitmap-only face never binds, the preview included.
    let mut bitmap = open(NAME, barlow_deck());
    bitmap
        .add_font(with_fs_type(BARLOW, 0x0202), FontSource::User, None)
        .unwrap();
    let printed = finished(&mut bitmap);
    assert!(printed.fonts.is_empty());
    let report = printed.faces();
    assert!(report[0].restricted && !report[0].shipped && report[0].refused.is_some());
    assert!(
        printed
            .deck
            .fonts
            .iter()
            .all(|face| face.family != "Barlow Condensed" || face.licence.is_none())
    );
    // Without fonts (the parity path) nothing ships, so the viewer's fonts draw and the face binds as installed.
    let mut parity = Host::new();
    parity
        .open(NAME, Box::new(BytesReader::new(barlow_deck())), false)
        .unwrap();
    parity
        .add_font(with_fs_type(BARLOW, 0x0002), FontSource::User, None)
        .unwrap();
    let printed = finished(&mut parity);
    assert!(printed.refused.is_empty() && printed.font_issues.is_empty());
    let face = printed
        .deck
        .fonts
        .iter()
        .find(|face| face.family == "Barlow Condensed")
        .unwrap();
    assert_eq!(face.source, slidock::deck::FontSource::User);
    // Editable embedding ships.
    let mut host = open(NAME, barlow_deck());
    host.add_font(with_fs_type(BARLOW, 0x0008), FontSource::User, None)
        .unwrap();
    assert_eq!(finished(&mut host).fonts.len(), 1);
}

#[test]
fn a_chunk_s_range_must_be_a_unicode_range() {
    let mut host = open(NAME, barlow_deck());
    let error = host
        .add_font(BARLOW.to_vec(), FontSource::Google, Some("latin"))
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Request);
    host.add_font(BARLOW.to_vec(), FontSource::Google, Some("U+0-FF"))
        .unwrap();
    let printed = finished(&mut host);
    assert_eq!(printed.fonts.len(), 1);
    // The chunk's rule claims only code points of its range.
    let range = printed.fonts[0].rules[0].unicode_range.as_ref().unwrap();
    assert!(range.iter().all(|point| point <= 0xFF));
}

#[test]
fn the_deck_names_the_substitute_faces_it_draws_with_no_file_and_a_bundled_file_ships_as_the_floor()
{
    let mut host = open(NAME, barlow_deck());
    let printed = finished(&mut host);
    // No source has Barlow Condensed: the generic sans-serif family, Liberation Sans, draws it, metrics only.
    let names: Vec<(&str, &str)> = printed
        .substitutes
        .iter()
        .map(|face| (face.family.as_str(), face.postscript_name.as_str()))
        .collect();
    assert_eq!(names, [("Liberation Sans", "LiberationSans")]);
    assert!(printed.fonts.is_empty());

    // A bundled file fills no face slot the deck asks for, binds under its own family, and ships with no notice.
    let needs = host
        .add_font(BARLOW.to_vec(), FontSource::Bundled, None)
        .unwrap();
    assert!(needs.iter().any(|need| need.family == "Barlow Condensed"));
    let printed = finished(&mut host);
    assert_eq!(printed.fonts.len(), 1);
    let face = &printed.fonts[0];
    assert_eq!(
        (face.source, face.notice, face.licence.family.as_str()),
        (FontSource::Bundled, false, "Barlow Condensed")
    );
    let report = printed.faces();
    assert_eq!(
        report[0].source,
        slidock_wasm::protocol::AddedFontSource::Bundled
    );
    let (zip, bundle) = bundled(&mut host, &BundleOptions::default());
    assert_eq!(font_faces(&entry(&zip, "index.html")), 1);
    assert!(entry(&zip, "LICENSES.txt").contains("(a slidock substitute)"));
    assert_eq!(bundle.fonts_with_notice, None);
}
