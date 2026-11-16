//! The `slidock` binary on presentations of our own, run as a child process: usage and exit codes, the IR and the
//! report on stdout, the document set and its media in a folder, identical outputs on every run, and PPTX, ODP and
//! Keynote inputs sniffed to their frontends.
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

const SCHEMA: &str = slidock::SCHEMA_URL;

/// Runs the CLI with its cache in the working folder (empty: the deck compiles below pass `--offline`, so no test
/// touches the network or the user's cache).
fn slidock(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_slidock"))
        .args(args)
        .current_dir(dir)
        .env("SLIDOCK_CACHE_DIR", dir.join(".cache"))
        .output()
        .unwrap()
}

fn code(output: &Output) -> i32 {
    output.status.code().unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8(output.stdout.clone()).unwrap()
}

fn stderr(output: &Output) -> String {
    String::from_utf8(output.stderr.clone()).unwrap()
}

fn frame(x: f64, y: f64, w: f64, h: f64) -> Value {
    json!({ "x": x, "y": y, "w": w, "h": h, "rotation": 0, "flipH": false, "flipV": false })
}

fn text_shape(id: &str, text: &str) -> Value {
    json!({
        "id": id, "origin": { "kind": "agent" }, "kind": "shape", "frame": frame(60.0, 160.0, 600.0, 80.0),
        "geometry": { "kind": "preset", "name": "rect", "adjust": {} },
        "text": { "body": {}, "paragraphs": [{ "level": 0, "props": {}, "inlines": [{ "kind": "run", "text": text,
            "props": { "font": { "kind": "family", "family": "Liberation Sans" }, "size": 24,
                       "color": { "kind": "rgb", "space": "srgb", "r": 0, "g": 0, "b": 0, "a": 1 } } }] }] }
    })
}

fn slide(id: &str, elements: Vec<Value>) -> Value {
    json!({ "id": id, "origin": { "kind": "agent" }, "hidden": false, "showLayout": true, "timeline": [],
            "elements": elements })
}

fn presentation(slides: Vec<Value>) -> Value {
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
    json!({
        "$schema": SCHEMA, "size": { "w": 720, "h": 405 },
        "defaults": { "body": { "anchor": "top", "autofit": { "kind": "none" },
                                "insets": { "b": 3.6, "l": 7.2, "r": 7.2, "t": 3.6 }, "overflow": "spread",
                                "wrap": "square" }, "text": text },
        "themes": {}, "masters": {}, "layouts": {}, "fonts": {}, "media": {}, "phase": "source", "slides": slides
    })
}

/// A 1 × 1 24-bit BMP holding one red pixel.
fn red_bmp() -> Vec<u8> {
    let mut bmp = vec![0u8; 58];
    bmp[..2].copy_from_slice(b"BM");
    bmp[2..6].copy_from_slice(&58u32.to_le_bytes());
    bmp[10..14].copy_from_slice(&54u32.to_le_bytes());
    bmp[14..18].copy_from_slice(&40u32.to_le_bytes());
    bmp[18..22].copy_from_slice(&1i32.to_le_bytes());
    bmp[22..26].copy_from_slice(&1i32.to_le_bytes());
    bmp[26..28].copy_from_slice(&1u16.to_le_bytes());
    bmp[28..30].copy_from_slice(&24u16.to_le_bytes());
    bmp[54..57].copy_from_slice(&[0, 0, 255]);
    bmp
}

fn sha256(bytes: &[u8]) -> String {
    slidock_canon::sha256_hex(bytes)
}

/// One slide saying hello.
fn hello() -> Value {
    presentation(vec![slide(
        "ir-s1",
        vec![text_shape("ir-s1.hello", "Hello")],
    )])
}

/// Two slides, the first showing a BMP (which plan-media transcodes to PNG), with the BMP in `media/` beside it.
fn with_bmp(dir: &Path, media: &str) -> PathBuf {
    let bmp = red_bmp();
    let sha = sha256(&bmp);
    let mut p = presentation(vec![
        slide(
            "ir-s1",
            vec![
                json!({ "id": "ir-s1.pic", "origin": { "kind": "agent" }, "kind": "picture",
                         "frame": frame(10.0, 10.0, 100.0, 100.0), "media": sha }),
            ],
        ),
        slide("ir-s2", vec![text_shape("ir-s2.t", "Two")]),
    ]);
    p["media"][&sha] = json!({ "sha256": sha, "type": "image/bmp", "bytes": bmp.len(), "width": 1, "height": 1,
                               "origin": { "kind": "source", "part": "media/red.bmp" } });
    std::fs::create_dir_all(dir.join(media)).unwrap();
    std::fs::write(dir.join(media).join(format!("{sha}.bmp")), &bmp).unwrap();
    write(dir, "bmp.presentation-ir.json", &p)
}

fn write(dir: &Path, name: &str, value: &Value) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, serde_json::to_vec_pretty(value).unwrap()).unwrap();
    path
}

fn workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "hello.presentation-ir.json", &hello());
    std::fs::write(dir.path().join("notes.txt"), "not a presentation").unwrap();
    dir
}

#[test]
fn prints_usage_and_refuses_unknown_commands_and_options() {
    let dir = workspace();
    let dir = dir.path();
    assert_eq!(code(&slidock(dir, &["--help"])), 0);
    assert_eq!(code(&slidock(dir, &[])), 2);
    let unknown = slidock(dir, &["explode"]);
    assert_eq!(code(&unknown), 2);
    assert!(stderr(&unknown).contains("explode"), "{}", stderr(&unknown));
    let input = "hello.presentation-ir.json";
    assert_eq!(
        code(&slidock(dir, &["compile", input, "--phase", "final"])),
        2
    );
    assert_eq!(code(&slidock(dir, &["compile", input, "--emit", "zip"])), 2);
    assert_eq!(code(&slidock(dir, &["compile", input, "--colour"])), 2);
    let frontend = slidock(dir, &["compile", input, "--frontend", "docx"]);
    assert_eq!(code(&frontend), 2);
    let formats = "[possible values: auto, pptx, odp, keynote]";
    assert!(stderr(&frontend).contains(formats), "{}", stderr(&frontend));
    // pdf is an IR source format, but no frontend reads it: a usage error, not a compile that fails.
    let pdf = slidock(dir, &["compile", input, "--frontend", "pdf"]);
    assert_eq!(code(&pdf), 2);
    assert!(stderr(&pdf).contains(formats), "{}", stderr(&pdf));
    assert_eq!(code(&slidock(dir, &["compile"])), 2);
    assert_eq!(
        code(&slidock(dir, &["compile", "missing.presentation-ir.json"])),
        2
    );
    assert_eq!(
        code(&slidock(dir, &["compile", input, "--only", "Not An Id"])),
        2
    );
}

#[test]
fn compiles_to_the_source_phase_and_emits_a_dump_that_reads_back_to_the_same_ir() {
    let dir = workspace();
    let dir = dir.path();
    let first = slidock(
        dir,
        &[
            "compile",
            "hello.presentation-ir.json",
            "--phase",
            "source",
            "--emit",
            "ir",
        ],
    );
    assert_eq!(code(&first), 0, "{}", stderr(&first));
    let dump: Value = serde_json::from_str(&stdout(&first)).unwrap();
    assert_eq!(dump["$schema"], SCHEMA);
    assert_eq!(dump["phase"], "source");
    assert_eq!(dump["slides"][0]["id"], "ir-s1");
    std::fs::write(dir.join("dump.json"), stdout(&first)).unwrap();
    let again = slidock(
        dir,
        &["compile", "dump.json", "--phase", "source", "--emit", "ir"],
    );
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert_eq!(stdout(&again), stdout(&first));
}

#[test]
fn writes_a_canonical_report_of_provenance_diagnostics_features_and_shows() {
    let dir = workspace();
    let dir = dir.path();
    let result = slidock(
        dir,
        &[
            "compile",
            "hello.presentation-ir.json",
            "--phase",
            "source",
            "--emit",
            "report",
        ],
    );
    assert_eq!(code(&result), 0);
    let report: Value = serde_json::from_str(&stdout(&result)).unwrap();
    assert_eq!(report["provenance"]["format"], "ir");
    assert_eq!(report["provenance"]["name"], "hello.presentation-ir.json");
    assert_eq!(report["shows"], json!({ "ir-s1": [] }));
    assert!(report["features"]["ir-s1"].is_array());
    assert!(report.get("stats").is_none());
    assert_eq!(
        stdout(&result),
        slidock_canon::to_canonical_json(&report, slidock_canon::Layout::Indented).unwrap()
    );
    let both = slidock(
        dir,
        &[
            "compile",
            "hello.presentation-ir.json",
            "--emit",
            "ir,report",
        ],
    );
    assert_eq!(code(&both), 2);
}

#[test]
fn resolves_by_default_and_times_the_passes_on_request() {
    let dir = workspace();
    let dir = dir.path();
    let result = slidock(
        dir,
        &[
            "compile",
            "hello.presentation-ir.json",
            "--verify",
            "each",
            "--emit",
            "ir",
        ],
    );
    assert_eq!(code(&result), 0);
    assert!(
        !stderr(&result)
            .lines()
            .any(|line| line.starts_with("error"))
    );
    let dump: Value = serde_json::from_str(&stdout(&result)).unwrap();
    assert_eq!(dump["phase"], "resolved");

    let timed = slidock(
        dir,
        &[
            "compile",
            "hello.presentation-ir.json",
            "--timings",
            "--emit",
            "report",
        ],
    );
    let report: Value = serde_json::from_str(&stdout(&timed)).unwrap();
    assert_eq!(report["stats"]["passes"].as_array().unwrap().len(), 12);
    assert_eq!(
        stderr(&timed)
            .lines()
            .filter(|line| line.starts_with("timing "))
            .count(),
        12
    );
}

#[test]
fn writes_identical_outputs_on_every_run() {
    let dir = workspace();
    let dir = dir.path();
    let outputs = || {
        let report = slidock(
            dir,
            &["compile", "hello.presentation-ir.json", "--emit", "report"],
        );
        let ir = slidock(
            dir,
            &["compile", "hello.presentation-ir.json", "--emit", "ir"],
        );
        stdout(&report) + &stdout(&ir)
    };
    assert_eq!(outputs(), outputs());
}

#[test]
fn refuses_what_is_not_a_presentation_with_exit_code_2() {
    let dir = workspace();
    let refused = slidock(dir.path(), &["compile", "notes.txt", "--emit", "report"]);
    assert_eq!(code(&refused), 2);
    assert!(stderr(&refused).contains("IR0902"));
    let report: Value = serde_json::from_str(&stdout(&refused)).unwrap();
    assert_eq!(report["provenance"], Value::Null);
}

/// An IR document naming no `$schema`, or another, is a refused input like one that is not JSON: exit code 2, one
/// IR0001 at `$schema` from the IR frontend, and no provenance; it never reaches verify-source.
#[test]
fn refuses_an_ir_document_naming_no_schema_or_another_with_exit_code_2() {
    let dir = workspace();
    let dir = dir.path();
    for (name, schema) in [
        ("none", None),
        ("other", Some(json!("urn:slidock:presentation-ir:2"))),
        ("null", Some(Value::Null)),
    ] {
        let mut p = hello();
        let object = p.as_object_mut().unwrap();
        match schema {
            Some(value) => object.insert("$schema".to_owned(), value),
            None => object.remove("$schema"),
        };
        let file = format!("{name}.presentation-ir.json");
        write(dir, &file, &p);
        let refused = slidock(dir, &["compile", &file, "--emit", "report"]);
        assert_eq!(code(&refused), 2, "{name}: {}", stderr(&refused));
        let report: Value = serde_json::from_str(&stdout(&refused)).unwrap();
        assert_eq!(report["provenance"], Value::Null, "{name}");
        let diagnostics: Vec<(&Value, &Value, &Value, &Value)> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| (&d["code"], &d["stage"], &d["emitter"], &d["data"]["path"]))
            .collect();
        assert_eq!(
            diagnostics,
            [(
                &json!("IR0001"),
                &json!("frontend"),
                &json!("ir"),
                &json!("$schema")
            )],
            "{name}"
        );
    }
}

#[test]
fn exits_1_on_a_failed_container() {
    let dir = workspace();
    let mut p = hello();
    p["slides"][0]["elements"][0]["origin"] =
        json!({ "kind": "synth", "pass": "bind-placeholders" });
    write(dir.path(), "synth.presentation-ir.json", &p);
    let result = slidock(dir.path(), &["compile", "synth.presentation-ir.json"]);
    assert_eq!(code(&result), 1);
    assert!(
        stderr(&result).contains("error   IR0011 origin-invalid ir-s1: "),
        "{}",
        stderr(&result)
    );
}

#[test]
fn dumps_each_failed_pass_as_the_pass_and_container_it_failed_on() {
    let dir = workspace();
    // A group whose child space is 1000 times smaller than its frame puts its child far off the point grid.
    let rect = json!({ "id": "ir-s2.g.rect", "origin": { "kind": "agent" }, "kind": "shape",
                       "frame": frame(100_000.0, 0.0, 100_000.0, 10.0),
                       "geometry": { "kind": "preset", "name": "rect", "adjust": {} } });
    let group = json!({ "id": "ir-s2.g", "origin": { "kind": "agent" }, "kind": "group",
                        "frame": frame(0.0, 0.0, 100_000.0, 100.0),
                        "childSpace": { "x": 0, "y": 0, "w": 0.001, "h": 100 }, "children": [rect] });
    let p = presentation(vec![
        slide("ir-s1", vec![text_shape("ir-s1.t", "One")]),
        slide("ir-s2", vec![group]),
    ]);
    write(dir.path(), "far.presentation-ir.json", &p);
    let result = slidock(
        dir.path(),
        &[
            "compile",
            "far.presentation-ir.json",
            "--emit",
            "report",
            "--dump-failures",
            "failures",
        ],
    );
    assert_eq!(code(&result), 1, "{}", stderr(&result));
    let names: Vec<String> = std::fs::read_dir(dir.path().join("failures"))
        .unwrap()
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, ["normalize-frames-ir-s2.json"]);
    let text = std::fs::read_to_string(dir.path().join("failures").join(&names[0])).unwrap();
    let dump: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(
        (&dump["pass"], &dump["container"], &dump["scope"]),
        (&json!("normalize-frames"), &json!("ir-s2"), &json!("slide"))
    );
    assert_eq!(dump["view"]["container"]["id"], "ir-s2");
    // Canonical, indented JSON.
    assert_eq!(
        text,
        slidock_canon::to_canonical_json(&dump, slidock_canon::Layout::Indented).unwrap()
    );

    // Without the option nothing is dumped.
    std::fs::remove_dir_all(dir.path().join("failures")).unwrap();
    let quiet = slidock(
        dir.path(),
        &["compile", "far.presentation-ir.json", "--emit", "report"],
    );
    assert_eq!(code(&quiet), 1);
    assert!(!dir.path().join("failures").exists());
}

#[test]
fn compiles_only_the_slides_asked_for() {
    let dir = workspace();
    with_bmp(dir.path(), "media");
    let result = slidock(
        dir.path(),
        &[
            "compile",
            "bmp.presentation-ir.json",
            "--only",
            "ir-s2",
            "--emit",
            "ir",
        ],
    );
    assert_eq!(code(&result), 0, "{}", stderr(&result));
    let dump: Value = serde_json::from_str(&stdout(&result)).unwrap();
    let ids: Vec<&str> = dump["slides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, ["ir-s2"]);
    let unknown = slidock(
        dir.path(),
        &["compile", "bmp.presentation-ir.json", "--only", "ir-s9"],
    );
    assert_eq!(code(&unknown), 2);
    assert!(stderr(&unknown).contains("IR0913"));
}

#[test]
fn writes_the_document_set_its_media_the_ir_and_the_report_into_the_output_folder() {
    let dir = workspace();
    with_bmp(dir.path(), "media");
    let result = slidock(
        dir.path(),
        &[
            "compile",
            "bmp.presentation-ir.json",
            "--out",
            "out",
            "--emit",
            "documents,ir,report",
        ],
    );
    assert_eq!(code(&result), 0, "{}", stderr(&result));
    let out = dir.path().join("out");
    let png = std::fs::read(out.join(format!("media/{}.png", sha256(&red_bmp())))).unwrap();
    assert_eq!(&png[..4], &[0x89, b'P', b'N', b'G']);
    let manifest: Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("presentation.json")).unwrap())
            .unwrap();
    assert_eq!(manifest["generator"]["name"], "slidock");
    assert!(out.join("slides/ir-s1.html").is_file());
    let dump: Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("presentation-ir.json")).unwrap())
            .unwrap();
    assert_eq!(dump["phase"], "resolved");
    let report: Value =
        serde_json::from_str(&std::fs::read_to_string(out.join("report.json")).unwrap()).unwrap();
    assert_eq!(report["shows"], json!({ "ir-s1": [], "ir-s2": [] }));
}

#[test]
fn reads_media_from_the_folder_given_and_writes_none_with_no_media() {
    let dir = workspace();
    with_bmp(dir.path(), "elsewhere");
    let beside = slidock(
        dir.path(),
        &[
            "compile",
            "bmp.presentation-ir.json",
            "--out",
            "a",
            "--emit",
            "documents",
        ],
    );
    assert!(stderr(&beside).contains("IR0013"), "{}", stderr(&beside));
    let given = slidock(
        dir.path(),
        &[
            "compile",
            "bmp.presentation-ir.json",
            "--out",
            "b",
            "--emit",
            "documents",
            "--media",
            "elsewhere",
        ],
    );
    assert!(!stderr(&given).contains("IR0013"), "{}", stderr(&given));
    assert!(dir.path().join("b/media").is_dir());
    let none = slidock(
        dir.path(),
        &[
            "compile",
            "bmp.presentation-ir.json",
            "--out",
            "c",
            "--emit",
            "documents",
            "--media",
            "elsewhere",
            "--no-media",
        ],
    );
    assert_eq!(code(&none), 0);
    assert!(!dir.path().join("c/media").exists());
    assert!(dir.path().join("c/slides/ir-s1.html").is_file());
    let missing = slidock(
        dir.path(),
        &[
            "compile",
            "bmp.presentation-ir.json",
            "--out",
            "d",
            "--emit",
            "documents",
            "--media",
            "nowhere",
        ],
    );
    assert_eq!(code(&missing), 2);
}

#[test]
fn replaces_only_the_entries_of_the_format_in_an_existing_folder() {
    let dir = workspace();
    let out = dir.path().join("reused");
    std::fs::create_dir_all(out.join("slides")).unwrap();
    std::fs::write(out.join("slides/stale.html"), "old").unwrap();
    std::fs::write(out.join("keep.txt"), "mine").unwrap();
    let result = slidock(
        dir.path(),
        &[
            "compile",
            "hello.presentation-ir.json",
            "--out",
            "reused",
            "--emit",
            "documents",
        ],
    );
    assert_eq!(code(&result), 0);
    assert!(!out.join("slides/stale.html").exists());
    assert!(out.join("slides/ir-s1.html").is_file());
    assert_eq!(
        std::fs::read_to_string(out.join("keep.txt")).unwrap(),
        "mine"
    );
}

#[test]
fn refuses_options_that_need_a_document_set_without_one() {
    let dir = workspace();
    let dir = dir.path();
    let input = "hello.presentation-ir.json";
    assert_eq!(
        code(&slidock(dir, &["compile", input, "--emit", "documents"])),
        2
    );
    assert_eq!(
        code(&slidock(
            dir,
            &["compile", input, "--out", "x", "--phase", "source"]
        )),
        2
    );
    assert_eq!(code(&slidock(dir, &["compile", input, "--no-media"])), 2);
    assert!(!dir.join("x").exists());
}

/// A zip of these files in order, the first stored (as an ODP's `mimetype` must be), the rest deflated.
fn zip(files: &[(&str, &str)]) -> Vec<u8> {
    use std::io::Write;
    use zip::write::SimpleFileOptions;
    let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (index, (path, text)) in files.iter().enumerate() {
        let method = if index == 0 {
            zip::CompressionMethod::Stored
        } else {
            zip::CompressionMethod::Deflated
        };
        writer
            .start_file(
                *path,
                SimpleFileOptions::default().compression_method(method),
            )
            .unwrap();
        writer.write_all(text.as_bytes()).unwrap();
    }
    writer.finish().unwrap().into_inner()
}

const PML: &str = concat!(
    r#"xmlns:a="http://schemas.openxmlformats.org/drawingml/2006/main" "#,
    r#"xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships" "#,
    r#"xmlns:p="http://schemas.openxmlformats.org/presentationml/2006/main""#
);

fn relationships(rels: &[(&str, &str, &str)]) -> String {
    let rels: String = rels
        .iter()
        .map(|(id, kind, target)| {
            format!(
                r#"<Relationship Id="{id}" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/{kind}" Target="{target}"/>"#
            )
        })
        .collect();
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">{rels}</Relationships>"#
    )
}

/// A PPTX of one slide, with no master or layout, holding a blue box that says "Hello".
fn pptx() -> Vec<u8> {
    let types = concat!(
        r#"<?xml version="1.0" encoding="UTF-8"?><Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">"#,
        r#"<Default Extension="xml" ContentType="application/xml"/>"#,
        r#"<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/></Types>"#
    );
    let presentation = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><p:presentation {PML}><p:sldIdLst><p:sldId id="256" r:id="rId2"/></p:sldIdLst><p:sldSz cx="9144000" cy="5143500"/></p:presentation>"#
    );
    let slide = format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?><p:sld {}><p:cSld><p:spTree><p:nvGrpSpPr><p:cNvPr id="1" name=""/>"#,
            r#"<p:cNvGrpSpPr/><p:nvPr/></p:nvGrpSpPr><p:grpSpPr/><p:sp><p:nvSpPr><p:cNvPr id="2" name="Box"/><p:cNvSpPr/>"#,
            r#"<p:nvPr/></p:nvSpPr><p:spPr><a:xfrm><a:off x="914400" y="914400"/><a:ext cx="3657600" cy="914400"/></a:xfrm>"#,
            r#"<a:prstGeom prst="rect"><a:avLst/></a:prstGeom><a:solidFill><a:srgbClr val="336699"/></a:solidFill></p:spPr>"#,
            r#"<p:txBody><a:bodyPr/><a:p><a:r><a:rPr lang="en-US" sz="2400"><a:latin typeface="Liberation Sans"/></a:rPr>"#,
            r#"<a:t>Hello</a:t></a:r></a:p></p:txBody></p:sp></p:spTree></p:cSld></p:sld>"#
        ),
        PML
    );
    zip(&[
        ("[Content_Types].xml", types),
        (
            "_rels/.rels",
            &relationships(&[("rId1", "officeDocument", "ppt/presentation.xml")]),
        ),
        ("ppt/presentation.xml", &presentation),
        (
            "ppt/_rels/presentation.xml.rels",
            &relationships(&[("rId2", "slide", "slides/slide1.xml")]),
        ),
        ("ppt/slides/slide1.xml", &slide),
    ])
}

/// An ODP of one page on a plain master, holding a text box that says "Hello".
fn odp() -> Vec<u8> {
    const OFFICE: &str = "urn:oasis:names:tc:opendocument:xmlns";
    let content = format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?><office:document-content xmlns:office="{o}:office:1.0" "#,
            r#"xmlns:draw="{o}:drawing:1.0" xmlns:text="{o}:text:1.0" xmlns:svg="{o}:svg-compatible:1.0" "#,
            r#"office:version="1.3"><office:body><office:presentation><draw:page draw:name="One" "#,
            r#"draw:master-page-name="Default"><draw:frame svg:x="2cm" svg:y="2cm" svg:width="10cm" svg:height="2cm">"#,
            r#"<draw:text-box><text:p>Hello</text:p></draw:text-box></draw:frame></draw:page></office:presentation>"#,
            r#"</office:body></office:document-content>"#
        ),
        o = OFFICE
    );
    let styles = format!(
        concat!(
            r#"<?xml version="1.0" encoding="UTF-8"?><office:document-styles xmlns:office="{o}:office:1.0" "#,
            r#"xmlns:style="{o}:style:1.0" xmlns:fo="{o}:xsl-fo-compatible:1.0" office:version="1.3">"#,
            r#"<office:automatic-styles><style:page-layout style:name="PM1"><style:page-layout-properties "#,
            r#"fo:page-width="28cm" fo:page-height="15.75cm"/></style:page-layout></office:automatic-styles>"#,
            r#"<office:master-styles><style:master-page style:name="Default" style:page-layout-name="PM1"/>"#,
            r#"</office:master-styles></office:document-styles>"#
        ),
        o = OFFICE
    );
    zip(&[
        (
            "mimetype",
            "application/vnd.oasis.opendocument.presentation",
        ),
        ("content.xml", &content),
        ("styles.xml", &styles),
    ])
}

#[test]
fn sniffs_pptx_odp_and_keynote_inputs_and_compiles_them_to_documents() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    std::fs::write(dir.join("deck.pptx"), pptx()).unwrap();
    // The bytes decide, not the extension.
    std::fs::write(dir.join("deck.zip"), odp()).unwrap();
    for (input, format, slide) in [
        ("deck.pptx", "pptx", "pp-256"),
        ("deck.zip", "odp", "od-p1"),
    ] {
        let out = format!("{format}-out");
        let run = slidock(
            dir,
            &[
                "compile",
                input,
                "--out",
                &out,
                "--emit",
                "documents,report",
            ],
        );
        assert_eq!(code(&run), 0, "{input}: {}", stderr(&run));
        let report: Value =
            serde_json::from_slice(&std::fs::read(dir.join(&out).join("report.json")).unwrap())
                .unwrap();
        assert_eq!(report["provenance"]["format"], format, "{input}");
        assert_eq!(report["diagnostics"], json!([]), "{input}");
        let html =
            std::fs::read_to_string(dir.join(&out).join(format!("slides/{slide}.html"))).unwrap();
        assert!(html.contains(">Hello<"), "{input}: {html}");
    }

    // A Keynote package whose document is not one: the Keynote frontend refuses it.
    std::fs::write(
        dir.join("deck.key"),
        zip(&[("Index/Document.iwa", "\0\u{5}\0\0nope")]),
    )
    .unwrap();
    let keynote = slidock(dir, &["compile", "deck.key", "--emit", "report"]);
    assert_eq!(code(&keynote), 2, "{}", stderr(&keynote));
    let report: Value = serde_json::from_str(&stdout(&keynote)).unwrap();
    assert_eq!(report["diagnostics"][0]["code"], "KN1003");
    assert_eq!(report["diagnostics"][0]["emitter"], "keynote");

    // A zip no frontend reads.
    std::fs::write(dir.join("other.pptx"), zip(&[("a.txt", "a")])).unwrap();
    let other = slidock(dir, &["compile", "other.pptx", "--emit", "report"]);
    assert_eq!(code(&other), 2, "{}", stderr(&other));
    let report: Value = serde_json::from_str(&stdout(&other)).unwrap();
    assert_eq!(report["diagnostics"][0]["code"], "IR0902");

    // A folder is not an input.
    std::fs::create_dir(dir.join("Folder.key")).unwrap();
    assert_eq!(code(&slidock(dir, &["compile", "Folder.key"])), 2);
}

#[test]
fn refuses_a_fonts_folder_it_cannot_read_and_warns_about_a_font_it_cannot_parse() {
    let dir = workspace();
    let dir = dir.path();
    let input = "hello.presentation-ir.json";
    let missing = slidock(
        dir,
        &["compile", input, "--fonts", "missing", "--emit", "ir"],
    );
    assert_eq!(code(&missing), 2);
    assert!(
        stderr(&missing).contains("--fonts missing"),
        "{}",
        stderr(&missing)
    );
    std::fs::create_dir(dir.join("fonts")).unwrap();
    std::fs::write(dir.join("fonts/broken.ttf"), b"not a font").unwrap();
    let broken = slidock(dir, &["compile", input, "--fonts", "fonts", "--emit", "ir"]);
    assert_eq!(code(&broken), 0, "{}", stderr(&broken));
    // The file by its absolute path, from the working folder the process sees (macOS's /private/var for /var).
    let warning = format!(
        "warning font {}: ",
        dir.canonicalize()
            .unwrap()
            .join("fonts/broken.ttf")
            .display()
    );
    assert!(stderr(&broken).starts_with(&warning), "{}", stderr(&broken));
    let plain = slidock(dir, &["compile", input, "--emit", "ir"]);
    assert_eq!(stdout(&broken), stdout(&plain));
}

/// Every entry of a zip, by name, in archive order.
fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    use std::io::Read;
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).unwrap();
    (0..archive.len())
        .map(|index| {
            let mut entry = archive.by_index(index).unwrap();
            let mut data = Vec::new();
            entry.read_to_end(&mut data).unwrap();
            (entry.name().unwrap().into_owned(), data)
        })
        .collect()
}

/// Every file under `dir`, by `/`-separated path, sorted.
fn files_under(dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let name = path.strip_prefix(dir).unwrap().to_str().unwrap();
                files.push((name.replace('\\', "/"), std::fs::read(&path).unwrap()));
            }
        }
    }
    files.sort();
    files
}

/// Needs `slidock_bundle::bundle`: fails while it returns `Unimplemented`.
#[test]
fn writes_the_deck_as_a_zip_or_as_the_folder_the_zip_unpacks_to() {
    let dir = workspace();
    with_bmp(dir.path(), "media");
    let input = "bmp.presentation-ir.json";
    let zipped = slidock(
        dir.path(),
        &["compile", input, "-o", "out/deck.zip", "--offline"],
    );
    assert_eq!(code(&zipped), 0, "{}", stderr(&zipped));
    let zip_bytes = std::fs::read(dir.path().join("out/deck.zip")).unwrap();
    assert!(!dir.path().join("out/deck.zip.part").exists());
    let entries = unzip(&zip_bytes);
    let names: Vec<&str> = entries.iter().map(|(name, _)| name.as_str()).collect();
    let png = format!("media/{}.png", sha256(&red_bmp()));
    assert_eq!(names.first(), Some(&"index.html"));
    assert!(names.contains(&png.as_str()), "{names:?}");
    let html = String::from_utf8(entries[0].1.clone()).unwrap();
    assert!(html.contains("id=\"sd-deck\""), "{html}");

    // The same input writes the same bytes; a folder holds the zip's files.
    let again = slidock(
        dir.path(),
        &["compile", input, "-o", "again.zip", "--offline"],
    );
    assert_eq!(code(&again), 0, "{}", stderr(&again));
    assert_eq!(
        std::fs::read(dir.path().join("again.zip")).unwrap(),
        zip_bytes
    );
    let folder = slidock(dir.path(), &["compile", input, "-o", "deck/", "--offline"]);
    assert_eq!(code(&folder), 0, "{}", stderr(&folder));
    let mut sorted = entries.clone();
    sorted.sort();
    assert_eq!(files_under(&dir.path().join("deck")), sorted);

    // --single-file inlines the media.
    let single = slidock(
        dir.path(),
        &[
            "compile",
            input,
            "-o",
            "single.zip",
            "--single-file",
            "--offline",
        ],
    );
    assert_eq!(code(&single), 0, "{}", stderr(&single));
    let single = unzip(&std::fs::read(dir.path().join("single.zip")).unwrap());
    assert!(single.iter().all(|(name, _)| !name.starts_with("media/")));
    assert!(String::from_utf8_lossy(&single[0].1).contains("data:image/png;base64,"));
}

#[test]
fn refuses_deck_options_that_do_not_fit_and_leaves_no_zip_and_an_unchanged_folder_for_a_refused_input()
 {
    let dir = workspace();
    let dir = dir.path();
    let input = "hello.presentation-ir.json";
    for args in [
        &["compile", input, "--single-file", "--emit", "ir"][..],
        &["compile", input, "-o", "x.zip", "--phase", "source"],
        &["compile", input, "-o", "x.zip", "--timings"],
        &["compile", input, "-o", "x.zip", "--dump-failures", "dumps"],
    ] {
        assert_eq!(code(&slidock(dir, args)), 2, "{args:?}");
    }
    assert!(!dir.join("x.zip").exists() && !dir.join("dumps").exists());

    let refused = slidock(dir, &["compile", "notes.txt", "-o", "notes.zip"]);
    assert_eq!(code(&refused), 2);
    assert!(stderr(&refused).contains("IR0902"), "{}", stderr(&refused));
    assert!(!dir.join("notes.zip").exists());
    assert!(!dir.join("notes.zip.part").exists());

    // The folder given, even the working one, keeps what it held.
    std::fs::create_dir_all(dir.join("site/media")).unwrap();
    std::fs::write(dir.join("site/media/photo.png"), "mine").unwrap();
    std::fs::write(dir.join("site/index.html"), "mine").unwrap();
    std::fs::write(dir.join("site/notes.txt"), "not a deck").unwrap();
    let before = files_under(&dir.join("site"));
    for out in [".", "deck", "nested/deck"] {
        let refused = slidock(&dir.join("site"), &["compile", "notes.txt", "-o", out]);
        assert_eq!(code(&refused), 2, "{out}");
        assert_eq!(files_under(&dir.join("site")), before, "{out}");
    }
    assert!(!dir.join("site/deck").exists() && !dir.join("site/nested").exists());
    assert!(!dir.join("site/.slidock.part").exists());
}

/// The page flags: title, description, public URL, notes and the text view; a public URL that is not an absolute
/// http(s) one is refused with exit 2, as are the flags without a deck.
#[test]
fn writes_the_page_title_description_public_url_and_text_view_it_is_given() {
    let dir = workspace();
    let dir = dir.path();
    let input = "hello.presentation-ir.json";
    let index = |out: &str| std::fs::read_to_string(dir.join(out).join("index.html")).unwrap();

    let plain = slidock(dir, &["compile", input, "-o", "plain", "--offline"]);
    assert_eq!(code(&plain), 0, "{}", stderr(&plain));
    let page = index("plain");
    assert!(page.contains("<article id=\"sd-text\""), "{page}");
    // The slide's only text is its title: the heading, not repeated below it.
    assert!(
        page.contains("<h2><a href=\"#slide-1\">Hello</a></h2>\n</section>"),
        "{page}"
    );
    assert!(page.contains("<title>Hello</title>"), "{page}");
    assert!(!page.contains("rel=\"canonical\""), "{page}");
    assert!(
        !stderr(&plain).contains("Google reads"),
        "{}",
        stderr(&plain)
    );

    let given = slidock(
        dir,
        &[
            "compile",
            input,
            "-o",
            "given",
            "--offline",
            "--title",
            "My talk",
            "--description",
            "",
            "--site-url",
            "https://example.com/talk/",
            "--no-text-view",
        ],
    );
    assert_eq!(code(&given), 0, "{}", stderr(&given));
    let page = index("given");
    assert!(page.contains("<title>My talk</title>"), "{page}");
    assert!(!page.contains("name=\"description\""), "{page}");
    assert!(
        page.contains("<link rel=\"canonical\" href=\"https://example.com/talk/\">"),
        "{page}"
    );
    assert!(!page.contains("id=\"sd-text\""), "{page}");
    assert_eq!(deck_json(&page)["title"], "My talk");

    for bad in [
        "ftp://example.com/",
        "talk/index.html",
        "https://example.com/#x",
    ] {
        let refused = slidock(dir, &["compile", input, "-o", "bad", "--site-url", bad]);
        assert_eq!(code(&refused), 2, "{bad}");
        assert!(
            stderr(&refused).contains("public URL"),
            "{}",
            stderr(&refused)
        );
        assert!(!dir.join("bad").exists(), "{bad}");
    }
    for args in [
        &["compile", input, "--emit", "ir", "--title", "x"][..],
        &["compile", input, "--emit", "ir", "--no-presenter"],
        &["compile", input, "--emit", "ir", "--no-notes"],
        &["compile", input, "--emit", "ir", "--notes-on-page"],
        &["compile", input, "--emit", "ir", "--no-text-view"],
    ] {
        assert_eq!(code(&slidock(dir, args)), 2, "{args:?}");
    }
}

/// `--no-presenter` leaves the presenter view's script out and records none; notes on the page need notes and the
/// text view; `--notes` is gone.
#[test]
fn ships_the_presenter_view_unless_told_not_to() {
    let dir = workspace();
    let dir = dir.path();
    let input = "hello.presentation-ir.json";
    let index = |out: &str| std::fs::read_to_string(dir.join(out).join("index.html")).unwrap();

    let with = slidock(dir, &["compile", input, "-o", "with", "--offline"]);
    assert_eq!(code(&with), 0, "{}", stderr(&with));
    let page = index("with");
    assert!(
        page.contains("{ presenter: SlidockPresenter }"),
        "the boot hands the player its presenter view"
    );
    assert_eq!(page.matches("<script").count(), 4);
    assert!(deck_json(&page)["player"]["presenter"].is_string());

    let without = slidock(
        dir,
        &[
            "compile",
            input,
            "-o",
            "without",
            "--offline",
            "--no-presenter",
        ],
    );
    assert_eq!(code(&without), 0, "{}", stderr(&without));
    let page = index("without");
    assert!(!page.contains("SlidockPresenter"));
    assert_eq!(page.matches("<script").count(), 3);
    assert!(deck_json(&page)["player"]["presenter"].is_null());

    for args in [
        &["--no-notes", "--notes-on-page"][..],
        &["--notes-on-page", "--no-text-view"],
        &["--notes", "page"],
    ] {
        let refused = slidock(
            dir,
            &[&["compile", input, "-o", "refused"][..], args].concat(),
        );
        assert_eq!(code(&refused), 2, "{args:?}");
        assert!(!dir.join("refused").exists(), "{args:?}");
    }
}

/// The deck JSON in a deck folder's `index.html`.
fn deck_json(index: &str) -> Value {
    let start = index.find("id=\"sd-deck\">").unwrap() + "id=\"sd-deck\">".len();
    let end = start + index[start..].find("</script>").unwrap();
    serde_json::from_str(&index[start..end]).unwrap()
}

/// The motion fixture of `slidock-ir`: native steps, transitions and match keys.
#[test]
fn a_deck_keeps_the_native_motion_data_the_parity_path_leaves_out() {
    let dir = workspace();
    std::fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../slidock-ir/tests/data/motion.presentation-ir.json"),
        dir.path().join("motion.presentation-ir.json"),
    )
    .unwrap();
    let input = "motion.presentation-ir.json";
    let deck = slidock(
        dir.path(),
        &["compile", input, "-o", "deck/", "--offline", "-v"],
    );
    assert_eq!(code(&deck), 0, "{}", stderr(&deck));
    let index = std::fs::read_to_string(dir.path().join("deck/index.html")).unwrap();
    let json = deck_json(&index);
    let slides = json["slides"].as_array().unwrap();
    assert_eq!(slides[0]["timeline"][1]["effect"], "flyIn");
    assert_eq!(slides[0]["timeline"][1]["stagger"]["perUnit"]["ms"], 150);
    assert_eq!(slides[1]["transition"]["effect"], "morph");
    assert_eq!(slides[2]["transition"]["params"]["dir"], "l");
    assert_eq!(slides[2]["transition"]["easing"], "in-out");
    assert_eq!(index.matches("data-sd-match=\"card\"").count(), 2);
    assert_eq!(slides[0]["matchKeys"]["ir-s1.card"], "card");
    assert_eq!(slides[1]["matchKeys"]["ir-s2.card"], "card");
    // The opaque step prints only its fallback, and says so (an info diagnostic, listed at -v).
    assert!(stderr(&deck).contains("HTML6010"), "{}", stderr(&deck));

    // --fonts-off and --emit compile without it.
    let off = slidock(dir.path(), &["compile", input, "-o", "off/", "--fonts-off"]);
    assert_eq!(code(&off), 0, "{}", stderr(&off));
    let index = std::fs::read_to_string(dir.path().join("off/index.html")).unwrap();
    let json = deck_json(&index);
    for slide in json["slides"].as_array().unwrap() {
        for step in slide["timeline"].as_array().unwrap() {
            assert!(
                step.get("effect").is_none() && step.get("stagger").is_none(),
                "{step}"
            );
        }
        if let Some(transition) = slide["transition"].as_object() {
            assert!(!transition.contains_key("effect") && !transition.contains_key("easing"));
        }
        assert!(slide.get("matchKeys").is_none(), "{slide}");
    }
    // The embedded player names the attribute; no element carries it.
    assert!(!index.contains("data-sd-match=") && !stderr(&off).contains("HTML6010"));
    let emitted = slidock(
        dir.path(),
        &["compile", input, "-o", "emitted/", "--emit", "documents,ir"],
    );
    assert_eq!(code(&emitted), 0, "{}", stderr(&emitted));
    let ir = std::fs::read_to_string(dir.path().join("emitted/presentation-ir.json")).unwrap();
    for gone in ["\"native\"", "\"timing\"", "\"matchKey\"", "\"easing\""] {
        assert!(!ir.contains(gone), "{gone}");
    }
    let slide = std::fs::read_to_string(dir.path().join("emitted/slides/ir-s1.html")).unwrap();
    assert!(!slide.contains("data-sd-timeline") && !slide.contains("data-sd-match"));

    // --native-motion: the IR a deck plays, for the bench to compare its deck with.
    let native = slidock(
        dir.path(),
        &[
            "compile",
            input,
            "-o",
            "native/",
            "--emit",
            "ir",
            "--native-motion",
        ],
    );
    assert_eq!(code(&native), 0, "{}", stderr(&native));
    let ir = std::fs::read_to_string(dir.path().join("native/presentation-ir.json")).unwrap();
    for kept in ["\"native\"", "\"matchKey\"", "\"easing\""] {
        assert!(ir.contains(kept), "{kept}");
    }
    for args in [
        vec!["compile", input, "-o", "x/", "--native-motion"],
        vec![
            "compile",
            input,
            "-o",
            "x/",
            "--emit",
            "documents,ir",
            "--native-motion",
        ],
        vec!["compile", input, "--emit", "report", "--native-motion"],
    ] {
        let refused = slidock(dir.path(), &args);
        assert_eq!(code(&refused), 2, "{args:?}");
        let err = stderr(&refused);
        assert!(
            err.contains("--native-motion needs --emit ir")
                || err.contains("required arguments were not provided:\n  --emit <WHAT>"),
            "{err}"
        );
    }
}

/// The parity path leaves the native motion data out only once it verifies: malformed native data fails the same
/// container, with the same diagnostics, on the deck path and on the parity path.
#[test]
fn malformed_native_motion_data_fails_on_every_path() {
    let dir = workspace();
    let fixture = std::fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../slidock-ir/tests/data/motion.presentation-ir.json"),
    )
    .unwrap();
    let mut json: Value = serde_json::from_str(&fixture).unwrap();
    json["slides"][0]["elements"][0]["matchKey"] = Value::from(5);
    json["slides"][2]["timeline"][0]["timing"] =
        serde_json::json!({ "repeat": { "count": 0 }, "bogus": 1 });
    std::fs::write(
        dir.path().join("bad.presentation-ir.json"),
        serde_json::to_string(&json).unwrap(),
    )
    .unwrap();
    let input = "bad.presentation-ir.json";
    let runs = [
        ("deck", vec!["compile", input, "-o", "deck/", "--offline"]),
        ("off", vec!["compile", input, "-o", "off/", "--fonts-off"]),
        (
            "emitted",
            vec![
                "compile",
                input,
                "-o",
                "emitted/",
                "--emit",
                "documents,report,ir",
            ],
        ),
    ];
    let mut reports = Vec::new();
    for (name, args) in runs {
        let run = slidock(dir.path(), &args);
        let err = stderr(&run);
        assert_eq!(code(&run), 1, "{name}: {err}");
        for path in [
            "slides[0].elements[0].matchKey",
            "slides[2].timeline[0].timing",
        ] {
            assert!(err.contains(path), "{name} {path}: {err}");
        }
        let mut lines: Vec<&str> = err.lines().filter(|line| line.contains("IR0001")).collect();
        lines.sort_unstable();
        reports.push(lines.join("\n"));
    }
    assert!(!reports[0].is_empty());
    assert_eq!(reports[0], reports[1]);
    assert_eq!(reports[0], reports[2]);
}

/// The picture probe decks (`cargo xtask probes-pictures`): each vector picture is drawn from its SVG, written to
/// stretch to its frame and to show what its own viewport showed, unless the allow-list refuses the SVG, which leaves
/// the PNG preview and a diagnostic naming the refusal.
#[test]
fn draws_the_picture_probes_vectors_stretched_to_their_frames_or_reports_their_previews() {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out");
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    for (deck, refused) in [
        ("picture-probe.pptx", "PX2310"),
        ("picture-probe.odp", "OD9306"),
    ] {
        std::fs::copy(probes.join(deck), dir.join(deck)).unwrap();
        let out = format!("{deck}-out");
        let run = slidock(
            dir,
            &[
                "compile",
                deck,
                "--out",
                &out,
                "--emit",
                "documents,report,ir",
            ],
        );
        assert_eq!(code(&run), 0, "{deck}: {}", stderr(&run));
        let out = dir.join(&out);
        let ir: Value =
            serde_json::from_slice(&std::fs::read(out.join("presentation-ir.json")).unwrap())
                .unwrap();
        let elements = ir["slides"][0]["elements"].as_array().unwrap();
        let written = |name: &str| -> (String, Vec<u8>) {
            let element = elements.iter().find(|e| e["name"] == name).unwrap();
            let sha = element["media"].as_str().unwrap();
            let kind = ir["media"][sha]["type"].as_str().unwrap().to_owned();
            let ext = if kind == "image/svg+xml" {
                "svg"
            } else {
                "png"
            };
            let bytes = std::fs::read(out.join(format!("media/{sha}.{ext}"))).unwrap();
            (kind, bytes)
        };
        for (name, view_box) in [
            ("Stretched vector", "viewBox=\"0 0 100 100\""),
            ("Sliced vector", "viewBox=\"0 50 100 50\""),
            ("Plain vector", "viewBox=\"0 0 200 100\""),
        ] {
            let (kind, bytes) = written(name);
            assert_eq!(kind, "image/svg+xml", "{deck} {name}");
            let svg = String::from_utf8(bytes).unwrap();
            let root = &svg[..svg.find('>').unwrap()];
            assert!(
                root.contains(view_box) && root.contains("preserveAspectRatio=\"none\""),
                "{deck} {name}: {root}"
            );
            assert_eq!(
                root.matches("preserveAspectRatio").count(),
                1,
                "{deck} {name}"
            );
        }
        assert_eq!(written("Escaped url vector").0, "image/png", "{deck}");
        let report: Value =
            serde_json::from_slice(&std::fs::read(out.join("report.json")).unwrap()).unwrap();
        let codes: Vec<&str> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["code"].as_str().unwrap())
            .collect();
        // The slide has no text to title it (MID5801).
        assert_eq!(codes, ["MID5801", refused], "{deck}");
        let message = report["diagnostics"][1]["message"].as_str().unwrap();
        assert!(
            message.ends_with("the value of fill holds an escape or a function other than a colour, a transform or url(#…)"),
            "{deck}: {message}"
        );
    }
}

/// The picture-labels probe decks (`cargo xtask probes-pictures`): a vector picture that draws text in a font its
/// own style sheet embeds, behind a public DOCTYPE (as Excalidraw exports one), and one whose root keeps `xml:space`,
/// are drawn from their SVGs, the DOCTYPE dropped; a sheet whose font loads from another site, a DOCTYPE with an
/// internal subset, labels in a family the sheet does not embed and a font hint that leaves a quote open are refused,
/// their previews drawn and reported.
#[test]
fn draws_the_picture_labels_probes_text_vectors_from_their_svgs_and_reports_the_refused() {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out");
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    for (deck, refused) in [
        ("picture-labels-probe.pptx", "PX2310"),
        ("picture-labels-probe.odp", "OD9306"),
    ] {
        std::fs::copy(probes.join(deck), dir.join(deck)).unwrap();
        let out = format!("{deck}-out");
        let run = slidock(
            dir,
            &[
                "compile",
                deck,
                "--out",
                &out,
                "--emit",
                "documents,report,ir",
            ],
        );
        assert_eq!(code(&run), 0, "{deck}: {}", stderr(&run));
        let out = dir.join(&out);
        let ir: Value =
            serde_json::from_slice(&std::fs::read(out.join("presentation-ir.json")).unwrap())
                .unwrap();
        let elements = ir["slides"][0]["elements"].as_array().unwrap();
        let written = |name: &str| -> (String, Vec<u8>) {
            let element = elements.iter().find(|e| e["name"] == name).unwrap();
            let sha = element["media"].as_str().unwrap();
            let kind = ir["media"][sha]["type"].as_str().unwrap().to_owned();
            let ext = if kind == "image/svg+xml" {
                "svg"
            } else {
                "png"
            };
            let bytes = std::fs::read(out.join(format!("media/{sha}.{ext}"))).unwrap();
            (kind, bytes)
        };
        let (kind, chart) = written("Labelled chart vector");
        assert_eq!(kind, "image/svg+xml", "{deck}");
        let chart = String::from_utf8(chart).unwrap();
        assert!(
            chart.starts_with(r#"<?xml version="1.0" standalone="no"?><svg "#),
            "{deck}: {}",
            &chart[..120]
        );
        for kept in [
            "@font-face { font-family: Probe Hand; src: url(data:font/woff2;base64,d09GMgABAAAAA",
            ">Cobras <tspan fill=\"#FE7867\">100</tspan></text>",
            r#"xml:space="preserve""#,
        ] {
            assert!(chart.contains(kept), "{deck}: {kept}");
        }
        assert_eq!(
            written("Preserved space vector").0,
            "image/svg+xml",
            "{deck}"
        );
        for name in [
            "Remote font vector",
            "Internal subset vector",
            "Fallback font vector",
            "Open quote vector",
        ] {
            assert_eq!(written(name).0, "image/png", "{deck} {name}");
        }
        let report: Value =
            serde_json::from_slice(&std::fs::read(out.join("report.json")).unwrap()).unwrap();
        let diagnostics = report["diagnostics"].as_array().unwrap();
        let codes: Vec<&str> = diagnostics
            .iter()
            .map(|d| d["code"].as_str().unwrap())
            .collect();
        // The slide has no text to title it (MID5801).
        assert_eq!(
            codes,
            ["MID5801", refused, refused, refused, refused],
            "{deck}"
        );
        let reasons: Vec<&str> = diagnostics[1..]
            .iter()
            .map(|d| d["data"]["reason"].as_str().unwrap())
            .collect();
        assert_eq!(
            reasons,
            [
                "the SVG's style sheet has a font descriptor whose value is not shown",
                "the SVG does not parse (doctype)",
                "the SVG draws text in a font it does not embed",
                "the SVG's style sheet has a font descriptor whose value is not shown",
            ],
            "{deck}"
        );
    }
}

/// The picture-types probe decks (`cargo xtask probes-pictures`): ISO base media pictures are typed by their `ftyp`
/// brands, the compatible ones included, and a brand list that names nothing known is unrecognised, never a video. An
/// omitted picture's marker centres its label, so a frame that starts off the slide still shows it.
#[test]
fn types_the_picture_types_probes_by_their_brands_and_centres_an_omitted_pictures_label() {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out");
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    for deck in ["picture-types-probe.pptx", "picture-types-probe.odp"] {
        std::fs::copy(probes.join(deck), dir.join(deck)).unwrap();
        let out = format!("{deck}-out");
        let run = slidock(
            dir,
            &[
                "compile",
                deck,
                "--out",
                &out,
                "--emit",
                "documents,report,ir",
            ],
        );
        assert_eq!(code(&run), 0, "{deck}: {}", stderr(&run));
        let out = dir.join(&out);
        let ir: Value =
            serde_json::from_slice(&std::fs::read(out.join("presentation-ir.json")).unwrap())
                .unwrap();
        let slide = &ir["slides"][0];
        let elements = slide["elements"].as_array().unwrap();
        let element = |name: &str| elements.iter().find(|e| e["name"] == name).unwrap();
        let media = |name: &str| &ir["media"][element(name)["media"].as_str().unwrap()];
        for (name, kind, action) in [
            ("Off-slide omitted picture", "image/heic", "omit"),
            ("AVIF under mif1", "image/avif", "copy"),
            ("HEIF image sequence", "image/heic", "omit"),
            ("Unknown ISO brand", "application/octet-stream", "omit"),
        ] {
            assert_eq!(media(name)["type"], kind, "{deck} {name}");
            assert_eq!(media(name)["plan"]["action"], action, "{deck} {name}");
        }
        let avif = media("AVIF under mif1")["plan"]["output"]["file"]
            .as_str()
            .unwrap();
        assert!(out.join(avif).is_file(), "{deck}: {avif}");
        let report: Value =
            serde_json::from_slice(&std::fs::read(out.join("report.json")).unwrap()).unwrap();
        let mut declined: Vec<(&str, &str)> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .map(|d| {
                (
                    d["code"].as_str().unwrap(),
                    d["data"]["type"].as_str().unwrap_or(""),
                )
            })
            .collect();
        declined.sort_unstable();
        assert_eq!(
            declined,
            [
                ("MID5601", "application/octet-stream"),
                ("MID5601", "image/heic"),
                ("MID5601", "image/heic"),
                ("MID5801", ""),
            ],
            "{deck}"
        );
        // The omitted picture's frame starts above and left of the slide, its centre on it.
        let id = element("Off-slide omitted picture")["id"].as_str().unwrap();
        let html = std::fs::read_to_string(
            out.join(format!("slides/{}.html", slide["id"].as_str().unwrap())),
        )
        .unwrap();
        let at = html.find(&format!("data-el=\"{id}\"")).unwrap();
        let marker = &html[at..][html[at..].find("<div data-sd-marker").unwrap()..];
        let marker = &marker[..marker.find("</div>").unwrap()];
        assert!(
            marker.contains("display:flex;align-items:center;justify-content:center")
                && marker.ends_with(">Image not included"),
            "{deck}: {marker}"
        );
    }
}

/// The shrink probe decks (`cargo xtask probes-shrink`), bundled with their Liberation Sans: each box that stores no
/// scale is fitted by PowerPoint's steps, at the font scale and line spacing LibreOffice 25.2 fits the PPTX at (the
/// `normAutofit` it writes when it saves it), and a stored scale is kept. From the ODP LibreOffice fits List and
/// Wrapping at 92.5 % and Overflow at 32.5 %, one step larger: it lays ODF lines out at the font's own height, shorter
/// than the `size-1.2` lines both frontends draw. Before the rule, every box that stores no scale kept its size.
#[test]
fn fits_the_shrink_probes_by_powerpoints_steps_as_libreoffice_does() {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out");
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    // (font scale, the paragraphs' line height: 1.2 less the line spacing taken off, times 1.2).
    let office = [
        ("1", "1.2"),
        ("1", "1.08"),
        ("0.85", "0.96"),
        ("0.85", "0.96"),
        ("0.25", "0.96"),
    ];
    let stored = [("0.625", "0.96"), ("1", "1.08")];
    for (deck, expected) in [
        (
            "shrink-probe.pptx",
            office.iter().chain(&stored).collect::<Vec<_>>(),
        ),
        ("shrink-probe.odp", office.iter().collect()),
    ] {
        std::fs::copy(probes.join(deck), dir.join(deck)).unwrap();
        let out = format!("{deck}-deck/");
        let run = slidock(dir, &["compile", deck, "-o", &out, "--offline"]);
        assert_eq!(code(&run), 0, "{deck}: {}", stderr(&run));
        let html = std::fs::read_to_string(dir.join(&out).join("index.html")).unwrap();
        let mut fitted = Vec::new();
        let mut rest = html.as_str();
        while let Some(at) = rest.find("data-autofit=\"shrink\"") {
            rest = &rest[at..];
            let value = |rest: &str, key: &str| {
                let from = rest.find(key).unwrap() + key.len();
                rest[from..from + rest[from..].find(['"', ';']).unwrap()].to_owned()
            };
            fitted.push((
                value(rest, "data-font-scale=\""),
                value(rest, "data-fitted=\""),
                value(rest, "line-height:"),
            ));
            rest = &rest[1..];
        }
        let expected: Vec<(String, String, String)> = expected
            .iter()
            .map(|(scale, line)| ((*scale).to_owned(), "true".to_owned(), (*line).to_owned()))
            .collect();
        assert_eq!(fitted, expected, "{deck}");
    }
}

/// The line-ends probe decks (`cargo xtask probes-lines`): an arrowhead on a curve, or past a short last segment,
/// points from the path's point its length back along the path to the tip, and the line stops its body back along the
/// path. The heads are LibreOffice 25.2's, from its PDF of the ODP (the connector's from its PDF of the PPTX), in each
/// drawing's frame, within 0.1 pt: before the rule, the hooked arrow's head was 1.9 pt off and the short last
/// segment's 21 pt (it pointed down the 8 pt turn). A path shorter than its two heads' bodies keeps both, each placed
/// on the whole path.
#[test]
fn points_the_line_ends_probes_heads_along_their_paths_as_libreoffice_does() {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out");
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let hooked: &[&[[f64; 2]]] = &[&[[100.017, 120.028], [119.095, 108.348], [120.823, 128.275]]];
    let arc: &[&[[f64; 2]]] = &[
        &[
            [0.0, 160.027],
            [-8.986, 139.533],
            [0.508, 150.02],
            [11.0, 140.549],
        ],
        &[[240.037, 0.01], [220.168, 10.298], [219.883, -9.715]],
    ];
    let short: &[&[[f64; 2]]] = &[&[[300.027, 8.025], [277.833, 5.217], [288.973, -11.421]]];
    let connector: &[&[[f64; 2]]] =
        &[&[[239.993, 159.977], [219.354, 168.541], [220.774, 148.582]]];
    // Two heads with overlapping bodies: each is placed on the whole path, and the line under them is not drawn.
    let short_bend: &[&[[f64; 2]]] = &[
        &[[-0.002, -0.005], [21.88, 4.672], [9.379, 20.317]],
        &[[10.032, 8.015], [-11.854, 3.339], [0.648, -12.31]],
    ];
    let short_curve: &[&[[f64; 2]]] = &[
        &[[-0.002, 8.034], [11.08, -11.412], [22.193, 5.257]],
        &[[12.015, 0.014], [0.933, 19.46], [-10.18, 2.79]],
    ];
    let under: &[&[[f64; 2]]] = &[
        &[[-0.002, 0.005], [20.007, -10.002], [20.007, 10.007]],
        &[[24.035, 6.042], [1.699, 7.289], [9.633, -11.081]],
    ];
    let short_connector: &[&[[f64; 2]]] = &[
        &[[-0.002, -0.004], [21.853, 4.7], [9.352, 20.29]],
        &[[9.974, 7.988], [-11.881, 3.28], [0.62, -12.31]],
    ];
    for deck in ["line-ends-probe.pptx", "line-ends-probe.odp"] {
        std::fs::copy(probes.join(deck), dir.join(deck)).unwrap();
        let out = format!("{deck}-out");
        let run = slidock(dir, &["compile", deck, "--out", &out, "--emit", "ir"]);
        assert_eq!(code(&run), 0, "{deck}: {}", stderr(&run));
        let ir: Value = serde_json::from_slice(
            &std::fs::read(dir.join(&out).join("presentation-ir.json")).unwrap(),
        )
        .unwrap();
        let elements = ir["slides"][0]["elements"].as_array().unwrap();
        let mut expected = vec![
            ("Hooked arrow", hooked),
            ("Arc arrow", arc),
            ("Short last segment arrow", short),
            ("Short bend, two heads", short_bend),
            ("Short curve, two heads", short_curve),
            ("Bend under two heads", under),
        ];
        if deck.ends_with(".pptx") {
            expected.push(("Curved connector", connector));
            expected.push(("Short bent connector", short_connector));
        }
        for (name, heads) in expected {
            let element = elements.iter().find(|e| e["name"] == name).unwrap();
            let paths = element["geometry"]["paths"].as_array().unwrap();
            let points = |path: &Value| -> Vec<[f64; 2]> {
                path["segments"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter(|s| s[0] != "Z")
                    .map(|s| [s[1].as_f64().unwrap(), s[2].as_f64().unwrap()])
                    .collect()
            };
            let ends: Vec<Vec<[f64; 2]>> = paths
                .iter()
                .filter(|p| p["role"] == "line-end")
                .map(points)
                .collect();
            assert_eq!(ends.len(), heads.len(), "{deck} {name}");
            for (ours, libreoffice) in ends.iter().zip(heads) {
                for (a, b) in ours.iter().zip(libreoffice.iter()) {
                    assert!(
                        (a[0] - b[0]).abs() <= 0.1 && (a[1] - b[1]).abs() <= 0.1,
                        "{deck} {name}: {ours:?} against {libreoffice:?}"
                    );
                }
            }
            if name.contains("two heads") || name == "Short bent connector" {
                let line = points(&paths[0]);
                assert!(
                    line.windows(2).all(|pair| pair[0] == pair[1]),
                    "{deck} {name}: a line is drawn, {line:?}"
                );
            }
            if name == "Short last segment arrow" {
                // 20 pt back along the path: 8 on the turn, 12 on the first segment.
                let line = points(&paths[0]);
                assert_eq!(line.last(), Some(&[288.0, 0.0]), "{deck}");
            }
        }
    }
}

/// Compiles a probe deck of `probes/out` into `dir` and reads the output `--emit` names.
fn probe_output(dir: &Path, deck: &str, emit: &str) -> Value {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out");
    std::fs::copy(probes.join(deck), dir.join(deck)).unwrap();
    let out = format!("{deck}-{emit}");
    let run = slidock(dir, &["compile", deck, "--out", &out, "--emit", emit]);
    assert_eq!(code(&run), 0, "{deck}: {}", stderr(&run));
    let file = if emit == "ir" {
        "presentation-ir.json"
    } else {
        "report.json"
    };
    serde_json::from_slice(&std::fs::read(dir.join(&out).join(file)).unwrap()).unwrap()
}

/// The alt-text probe (`probes/out/alt-probe.pptx`, picture-probe's four vectors with alt text of each kind): a
/// picture PowerPoint marks decorative (`adec:decorative val="1"`) has no alt text, whatever its `descr` says; the
/// others keep `descr`, else `title`.
#[test]
fn says_nothing_for_a_picture_marked_decorative() {
    let dir = tempfile::tempdir().unwrap();
    let ir = probe_output(dir.path(), "alt-probe.pptx", "ir");
    let alt = |name: &str| {
        let elements = ir["slides"][0]["elements"].as_array().unwrap();
        let element = elements.iter().find(|e| e["name"] == name).unwrap();
        element.get("alt").cloned()
    };
    assert_eq!(alt("Plain vector"), None);
    assert_eq!(
        alt("Stretched vector"),
        Some(Value::from("A red circle on white"))
    );
    assert_eq!(alt("Sliced vector"), Some(Value::from("Title only alt")));
    assert_eq!(
        alt("Escaped url vector"),
        Some(Value::from("Linked picture"))
    );

    let documents = dir.path().join("alt-probe-documents");
    let run = slidock(
        dir.path(),
        &[
            "compile",
            "alt-probe.pptx",
            "--out",
            "alt-probe-documents",
            "--emit",
            "documents",
        ],
    );
    assert_eq!(code(&run), 0, "{}", stderr(&run));
    let manifest: Value =
        serde_json::from_slice(&std::fs::read(documents.join("presentation.json")).unwrap())
            .unwrap();
    let slide = manifest["slides"][0]["file"].as_str().unwrap();
    let html = std::fs::read_to_string(documents.join(slide)).unwrap();
    assert!(!html.contains("Marked decorative"), "{html}");
}

/// The shadow probe twins (`cargo xtask probes-shadows`): each unblurred box shadow (A01–A08 by direction, A10 and
/// A11 by distance) is offset as LibreOffice 25.2 draws it, within 0.1 pt. LibreOffice's offsets were measured on its
/// PDF of each twin, rasterised at 20 px/pt: the shadow's far edges against the box's, to a sub-pixel.
#[test]
fn offsets_the_shadow_probes_unblurred_shadows_as_libreoffice_does() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let pptx: &[(usize, [f64; 2])] = &[
        (1, [20.032, 0.0]),
        (2, [14.148, 14.148]),
        (3, [0.0, 20.076]),
        (4, [-14.15, 14.148]),
        (5, [-20.032, 0.0]),
        (6, [-14.15, -14.148]),
        (7, [0.0, -19.985]),
        (8, [14.148, -14.148]),
        (10, [2.374, 2.374]),
        (11, [42.398, 42.431]),
    ];
    let odp: &[(usize, [f64; 2])] = &[
        (1, [19.988, 0.0]),
        (2, [14.193, 14.15]),
        (3, [0.0, 20.043]),
        (4, [-14.15, 14.15]),
        (5, [-20.032, 0.0]),
        (6, [-14.15, -14.15]),
        (7, [0.0, -20.043]),
        (8, [14.193, -14.15]),
        (10, [2.318, 2.332]),
        (11, [42.45, 42.418]),
    ];
    for (deck, expected) in [("shadow-probe.pptx", pptx), ("shadow-probe.odp", odp)] {
        let ir = probe_output(dir, deck, "ir");
        for (slide, libreoffice) in expected {
            let elements = ir["slides"][slide - 1]["elements"].as_array().unwrap();
            let shadows: Vec<&Value> = elements
                .iter()
                .filter_map(|e| e.get("shadow").filter(|s| !s.is_null()))
                .collect();
            assert_eq!(shadows.len(), 1, "{deck} A{slide:02}: {shadows:?}");
            let shadow = shadows[0];
            let ours = [
                shadow["dx"].as_f64().unwrap(),
                shadow["dy"].as_f64().unwrap(),
            ];
            assert!(
                (ours[0] - libreoffice[0]).abs() <= 0.1 && (ours[1] - libreoffice[1]).abs() <= 0.1,
                "{deck} A{slide:02}: {ours:?} against {libreoffice:?}"
            );
            assert_eq!(shadow["sigma"], 0, "{deck} A{slide:02}");
            assert_eq!(shadow["color"]["a"], 0.5, "{deck} A{slide:02}");
        }
    }
}

/// The shadows the probes hold but the IR does not draw are each reported once, on their slide, with the reason:
/// the OOXML-only probe's inner, preset, scaled or skewed, table and effect-DAG shadows; the LibreOffice-only probe's
/// table and group shadows; and, of the twins, only the ODP's video (LibreOffice draws no shadow on a media frame).
#[test]
fn reports_each_undrawn_shadow_of_the_shadow_probes_on_its_slide() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let ooxml: &[(usize, &str)] = &[
        (6, "inner shadow"),
        (7, "preset shadow shdw1"),
        (8, "preset shadow shdw13"),
        (9, "preset shadow shdw17"),
        (10, "inner shadow"),
        (11, "scaled or skewed shadow"),
        (12, "scaled or skewed shadow"),
        (13, "scaled or skewed shadow"),
        (14, "scaled or skewed shadow"),
        (19, "table shadow"),
        (22, "effect DAG"),
    ];
    let odp_only: &[(usize, &str)] = &[(12, "table shadow"), (13, "group shadow")];
    let twin_odp: &[(usize, &str)] = &[(36, "media shadow")];
    for (deck, code, expected) in [
        ("shadow-probe-ooxml.pptx", "PX2111", ooxml),
        ("shadow-probe-odp.odp", "OD9104", odp_only),
        ("shadow-probe.pptx", "PX2111", &[]),
        ("shadow-probe.odp", "OD9104", twin_odp),
    ] {
        let ir = probe_output(dir, deck, "ir");
        let report = probe_output(dir, deck, "report");
        let slides: Vec<&str> = ir["slides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|s| s["id"].as_str().unwrap())
            .collect();
        let mut reported: Vec<(usize, String)> = report["diagnostics"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|d| ["KN1304", "PX2111", "OD9104"].contains(&d["code"].as_str().unwrap()))
            .map(|d| {
                assert_eq!(d["code"], code, "{deck}: {d}");
                let slide = slides.iter().position(|s| d["container"] == *s).unwrap() + 1;
                let message = d["message"].as_str().unwrap();
                let reason = message.strip_prefix("A shadow is not drawn: ").unwrap();
                (slide, reason.to_owned())
            })
            .collect();
        reported.sort();
        let expected: Vec<(usize, String)> = expected
            .iter()
            .map(|(slide, reason)| (*slide, (*reason).to_owned()))
            .collect();
        assert_eq!(reported, expected, "{deck}");
    }
}

/// A copy of `probes/out/picture-probe.pptx` in `dir`, whose deck reports one lossy and one info diagnostic.
fn picture_probe(dir: &Path) -> &'static str {
    let probes = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../probes/out");
    std::fs::copy(
        probes.join("picture-probe.pptx"),
        dir.join("picture-probe.pptx"),
    )
    .unwrap();
    "picture-probe.pptx"
}

#[test]
fn writes_the_deck_beside_the_working_folder_as_the_inputs_name_and_says_where() {
    let dir = workspace();
    let dir = dir.path();
    std::fs::create_dir(dir.join("in")).unwrap();
    with_bmp(&dir.join("in"), "media");
    let written = slidock(
        dir,
        &["compile", "in/bmp.presentation-ir.json", "--offline"],
    );
    assert_eq!(code(&written), 0, "{}", stderr(&written));
    assert!(dir.join("bmp.presentation-ir.zip").is_file());
    assert!(!dir.join("in/bmp.presentation-ir.zip").exists());
    assert!(
        stderr(&written)
            .lines()
            .any(|line| line == "wrote bmp.presentation-ir.zip"),
        "{}",
        stderr(&written)
    );
    // The same bytes as with -o.
    let named = slidock(
        dir,
        &[
            "compile",
            "in/bmp.presentation-ir.json",
            "-o",
            "named.zip",
            "--offline",
        ],
    );
    assert_eq!(code(&named), 0, "{}", stderr(&named));
    assert_eq!(
        std::fs::read(dir.join("bmp.presentation-ir.zip")).unwrap(),
        std::fs::read(dir.join("named.zip")).unwrap()
    );

    // An output that is the input is refused, the input untouched.
    let input = std::fs::read(dir.join("named.zip")).unwrap();
    std::fs::write(dir.join("talk.zip"), &input).unwrap();
    for args in [
        &["compile", "talk.zip", "--offline"][..],
        &["compile", "talk.zip", "-o", "./talk.zip", "--offline"],
    ] {
        let refused = slidock(dir, args);
        assert_eq!(code(&refused), 2, "{args:?}");
        assert!(
            stderr(&refused).contains("is the input; give another --out"),
            "{}",
            stderr(&refused)
        );
        assert_eq!(std::fs::read(dir.join("talk.zip")).unwrap(), input);
    }
}

#[test]
fn compiles_an_input_given_without_the_compile_command() {
    let dir = workspace();
    let dir = dir.path();
    with_bmp(dir, "media");
    let short = slidock(
        dir,
        &["bmp.presentation-ir.json", "-o", "short.zip", "--offline"],
    );
    assert_eq!(code(&short), 0, "{}", stderr(&short));
    let long = slidock(
        dir,
        &[
            "compile",
            "bmp.presentation-ir.json",
            "-o",
            "long.zip",
            "--offline",
        ],
    );
    assert_eq!(code(&long), 0, "{}", stderr(&long));
    assert_eq!(
        std::fs::read(dir.join("short.zip")).unwrap(),
        std::fs::read(dir.join("long.zip")).unwrap()
    );
    // Options and commands keep their meaning.
    assert!(stdout(&slidock(dir, &["--version"])).starts_with("slidock "));
    assert!(stdout(&slidock(dir, &["help", "compile"])).contains("Usage: slidock compile"));
    let missing = slidock(dir, &["missing.pptx"]);
    assert_eq!(code(&missing), 2);
    assert!(
        stderr(&missing).contains("missing.pptx"),
        "{}",
        stderr(&missing)
    );
}

#[test]
fn the_help_lists_no_developer_option() {
    let dir = workspace();
    let dir = dir.path();
    for help in [
        &["--help"][..],
        &["-h"],
        &["compile", "--help"],
        &["compile", "-h"],
    ] {
        let out = slidock(dir, help);
        assert_eq!(code(&out), 0, "{help:?}");
        let text = stdout(&out);
        for hidden in [
            "--verify",
            "--phase",
            "--dump-failures",
            "--timings",
            "--native-motion",
            "--fonts-off",
            "--media ",
            "parity",
            "container",
            "IR JSON",
            "PLAN",
        ] {
            assert!(!text.contains(hidden), "{help:?} lists {hidden}:\n{text}");
        }
    }
    let compile = stdout(&slidock(dir, &["compile", "--help"]));
    assert!(
        compile.contains("[possible values: auto, pptx, odp, keynote]"),
        "{compile}"
    );
    assert!(compile.contains("Google Fonts"), "{compile}");
    assert!(compile.contains("Exit status:"), "{compile}");
    // Hidden, the options still work.
    let timed = slidock(
        dir,
        &[
            "compile",
            "hello.presentation-ir.json",
            "--emit",
            "ir",
            "--timings",
            "--verify",
            "each",
        ],
    );
    assert_eq!(code(&timed), 0, "{}", stderr(&timed));
}

#[test]
fn a_conflict_names_only_the_options_given() {
    let dir = workspace();
    let dir = dir.path();
    let input = "hello.presentation-ir.json";
    let all_fonts = [
        "--no-embedded",
        "--system-fonts",
        "--no-google",
        "--no-substitutes",
        "--fonts-off",
    ];
    let page = [
        "--title",
        "--description",
        "--site-url",
        "--no-presenter",
        "--no-notes",
        "--notes-on-page",
    ];
    let offline = slidock(dir, &["compile", input, "--emit", "ir", "--offline"]);
    assert_eq!(code(&offline), 2);
    let err = stderr(&offline);
    assert!(err.contains("'--offline'"), "{err}");
    for other in all_fonts.iter().chain(&page) {
        assert!(!err.contains(other), "{other}: {err}");
    }
    let title = slidock(
        dir,
        &["compile", input, "--emit", "report", "--title", "Talk"],
    );
    assert_eq!(code(&title), 2);
    let err = stderr(&title);
    assert!(err.contains("'--title <TITLE>'"), "{err}");
    for other in all_fonts.iter().chain(&page[1..]).chain(&["--offline"]) {
        assert!(!err.contains(other), "{other}: {err}");
    }
    // Without --emit, they are deck options and compile.
    let deck = slidock(
        dir,
        &[
            "compile",
            input,
            "--offline",
            "--title",
            "Talk",
            "-o",
            "t.zip",
        ],
    );
    assert_eq!(code(&deck), 0, "{}", stderr(&deck));
}

#[test]
fn lists_warnings_and_losses_with_a_summary_errors_only_with_q_and_everything_with_v() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let input = picture_probe(dir);
    let run = |flags: &[&str]| {
        let mut args = vec![input, "-o", "deck.zip", "--offline"];
        args.extend(flags);
        let out = slidock(dir, &args);
        assert_eq!(code(&out), 0, "{flags:?}: {}", stderr(&out));
        stderr(&out)
    };
    let listed = |err: &str, severity: &str| {
        err.lines()
            .filter(|line| line.starts_with(&format!("{severity:<7} ")))
            .count()
    };

    let normal = run(&[]);
    assert_eq!(listed(&normal, "lossy"), 1, "{normal}");
    assert_eq!(listed(&normal, "info"), 0, "{normal}");
    assert!(
        normal.contains("lossy   PX2310 svg-blip-refused pp-256 pp-256.5: The picture is drawn from its raster preview"),
        "{normal}"
    );
    let tail: Vec<&str> = normal.lines().rev().take(2).collect();
    assert_eq!(
        tail,
        [
            "wrote deck.zip",
            "diagnostics: 1 lossy, 1 info (1 not listed; -v lists them)"
        ],
        "{normal}"
    );

    let verbose = run(&["-v"]);
    assert_eq!(listed(&verbose, "lossy"), 1, "{verbose}");
    assert_eq!(listed(&verbose, "info"), 1, "{verbose}");
    assert!(
        verbose.contains("\ndiagnostics: 1 lossy, 1 info\n"),
        "{verbose}"
    );

    assert_eq!(run(&["-q"]), "");
    assert_eq!(code(&slidock(dir, &[input, "-q", "-v"])), 2);
}

#[test]
fn compiles_the_slides_only_names_by_number_or_id() {
    let dir = workspace();
    let dir = dir.path();
    with_bmp(dir, "media");
    let input = "bmp.presentation-ir.json";
    let slides = |only: &str| {
        let out = slidock(dir, &["compile", input, "--emit", "ir", "--only", only]);
        assert_eq!(code(&out), 0, "{only}: {}", stderr(&out));
        let ir: Value = serde_json::from_str(&stdout(&out)).unwrap();
        ir["slides"]
            .as_array()
            .unwrap()
            .iter()
            .map(|slide| slide["id"].as_str().unwrap().to_owned())
            .collect::<Vec<_>>()
    };
    assert_eq!(slides("2"), ["ir-s2"]);
    assert_eq!(slides("ir-s2"), ["ir-s2"]);
    assert_eq!(slides("1,ir-s2"), ["ir-s1", "ir-s2"]);
    for (only, said) in [
        ("3", "there is no slide 3; the presentation has 2"),
        ("0", "there is no slide 0; the presentation has 2"),
        ("Not An Id", "Not An Id is not a slide number or a slide id"),
    ] {
        let out = slidock(dir, &["compile", input, "--emit", "ir", "--only", only]);
        assert_eq!(code(&out), 2, "{only}");
        assert!(stderr(&out).contains(said), "{only}: {}", stderr(&out));
    }
}

/// Runs the CLI with stdout or stderr a pipe whose reader is gone before it writes.
fn closed_pipe(dir: &Path, args: &[&str], stream: &str) -> Output {
    use std::process::Stdio;
    let mut command = Command::new(env!("CARGO_BIN_EXE_slidock"));
    command
        .args(args)
        .current_dir(dir)
        .env("SLIDOCK_CACHE_DIR", dir.join(".cache"))
        .stdin(Stdio::null());
    if stream == "stdout" {
        command.stdout(Stdio::piped()).stderr(Stdio::piped());
    } else {
        command.stdout(Stdio::null()).stderr(Stdio::piped());
    }
    let mut child = command.spawn().unwrap();
    if stream == "stdout" {
        drop(child.stdout.take());
    } else {
        drop(child.stderr.take());
    }
    child.wait_with_output().unwrap()
}

#[test]
fn a_closed_stdout_or_stderr_ends_the_output_quietly() {
    let dir = tempfile::tempdir().unwrap();
    let dir = dir.path();
    let input = picture_probe(dir);
    // The deck is still written; nothing panics.
    let deck = closed_pipe(dir, &[input, "-o", "deck.zip", "--offline", "-v"], "stderr");
    assert_eq!(code(&deck), 0);
    assert!(dir.join("deck.zip").is_file());
    let report = closed_pipe(dir, &["compile", input, "--emit", "report"], "stdout");
    assert_eq!(code(&report), 0, "{}", stderr(&report));
    assert!(!stderr(&report).contains("error"), "{}", stderr(&report));
}

#[test]
fn stops_a_compile_past_its_timeout_and_times_nothing_it_was_not_asked_to() {
    let dir = workspace();
    let dir = dir.path();
    let input = "hello.presentation-ir.json";
    let stopped = slidock(
        dir,
        &["compile", input, "--timeout", "0", "--emit", "report"],
    );
    assert_eq!(code(&stopped), 1);
    assert!(
        stderr(&stopped)
            .contains("IR0905 stage-timeout: analyze stopped: over the compile's 0 ms limit"),
        "{}",
        stderr(&stopped)
    );
    let deck = slidock(
        dir,
        &[
            "compile",
            input,
            "--timeout",
            "0",
            "-o",
            "deck.zip",
            "--offline",
        ],
    );
    assert_eq!(code(&deck), 1);
    assert!(stderr(&deck).contains("IR0905"), "{}", stderr(&deck));
    assert!(!dir.join("deck.zip").exists());
    let within = slidock(
        dir,
        &["compile", input, "--timeout", "600", "--emit", "report"],
    );
    assert_eq!(code(&within), 0, "{}", stderr(&within));
    let report: Value = serde_json::from_str(&stdout(&within)).unwrap();
    assert_eq!(report["stats"], Value::Null);
    assert!(!stderr(&within).contains("timing "));
    let deck = slidock(
        dir,
        &[
            "compile",
            input,
            "--timeout",
            "600",
            "-o",
            "deck.zip",
            "--offline",
        ],
    );
    assert_eq!(code(&deck), 0, "{}", stderr(&deck));
    assert!(dir.join("deck.zip").exists());
}
