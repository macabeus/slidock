//! Native and wasm write the same bytes (the bench's determinism gate), on every corpus pair of the golden
//! manifest, from `$SLIDOCK_PRIVATE/corpus` (`--features golden`; skips when the golden files are absent). The wasm
//! side is the built module (`pnpm wasm:build`) under Node, run by `packages/wasm/test/node/compile.ts`; the corpus
//! tests skip when the golden files are absent, and every test fails when Node or the module is missing.
//!
//! - Document sets: `slidock compile --emit documents` and the wasm compiler's document set (text and media) are
//!   the same files, byte for byte.
//! - Decks, Google Fonts off on both sides: `slidock compile -o deck.zip --no-google` and the zip the compile worker
//!   streams for `open`, `finish` and `bundle` are the same bytes, the input's embedded faces shipped, in chunks of
//!   1 MiB but the last; every medium the worker serves as a slice of the input holds the bytes the compiler writes
//!   for it. Both ship the same bundled substitutes: the CLI from the files built into it, the worker as `bundled`
//!   files of `slidock-substitutes` (`crates/slidock-substitutes/fonts/`, as the app adds them) after a `finish` names the faces drawn.
//!   The same holds for the smoke deck the web app's e2e suite writes and the font probes, and for the probes with a
//!   dropped font (`--fonts` natively, `addFont` as `user` in the worker).
//! - The 1 GB Keynote deck (`retro.keynote`) runs natively only: two runs write the same zip. Its wasm run is a
//!   manual measurement (`packages/wasm/README.md`).
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

mod support;

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use serde_json::Value;
use slidock::Sha256;
use support::{Pair, corpus_pairs, golden, read_json, read_tree};

/// The pair that runs natively only.
const LARGEST: &str = "retro.keynote";

const CHUNK_BYTES: u64 = 1024 * 1024;

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn pairs() -> Vec<Pair> {
    let pairs = corpus_pairs(golden().unwrap());
    assert_eq!(pairs.len(), 10, "the corpus holds ten non-PDF pairs");
    assert!(pairs.iter().any(|pair| pair.name == LARGEST));
    pairs
}

/// The bundled substitutes, which the CLI builds in and the web app serves.
fn core() -> PathBuf {
    repo().join("crates/slidock-substitutes/fonts")
}

/// `compile.ts`' `--core` argument.
fn core_argument() -> OsString {
    let mut argument = OsString::from("--core=");
    argument.push(core());
    argument
}

/// An empty cache for the CLI: with Google Fonts off it reads nothing from it.
fn cache() -> &'static Path {
    static CACHE: OnceLock<PathBuf> = OnceLock::new();
    CACHE.get_or_init(|| tempfile::tempdir().unwrap().keep())
}

fn slidock(args: &[&std::ffi::OsStr]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_slidock"))
        .args(args)
        .env("SLIDOCK_CACHE_DIR", cache())
        .output()
        .unwrap()
}

/// What the wasm side wrote for every pair but the largest: `<dir>/<pair>/{documents/,deck.zip,result.json}`.
/// Node runs once per test process.
fn wasm_outputs() -> &'static Path {
    static OUT: OnceLock<PathBuf> = OnceLock::new();
    OUT.get_or_init(|| {
        let module = repo().join("packages/wasm/pkg/slidock_wasm_bg.wasm");
        assert!(
            module.is_file(),
            "{} is missing: run `pnpm wasm:build`",
            module.display()
        );
        let out = tempfile::tempdir().unwrap().keep();
        let scripts = repo().join("packages/wasm/test/node");
        let mut node = Command::new("node");
        node.arg("--import")
            .arg(scripts.join("ts-resolve.ts"))
            .arg(scripts.join("compile.ts"))
            .arg(&out)
            .arg(core_argument());
        for pair in pairs().iter().filter(|pair| pair.name != LARGEST) {
            let mut argument = std::ffi::OsString::from(format!("{}=", pair.name));
            argument.push(golden().unwrap().source(pair));
            node.arg(argument);
        }
        let run = node.output().unwrap();
        assert!(
            run.status.success(),
            "node: {}{}",
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        );
        out
    })
}

/// Runs `compile.ts` into a fresh folder with these arguments after it.
fn run_node(arguments: &[std::ffi::OsString]) -> PathBuf {
    let module = repo().join("packages/wasm/pkg/slidock_wasm_bg.wasm");
    assert!(
        module.is_file(),
        "{} is missing: run `pnpm wasm:build`",
        module.display()
    );
    let out = tempfile::tempdir().unwrap().keep();
    let scripts = repo().join("packages/wasm/test/node");
    let run = Command::new("node")
        .arg("--import")
        .arg(scripts.join("ts-resolve.ts"))
        .arg(scripts.join("compile.ts"))
        .arg(&out)
        .arg(core_argument())
        .args(arguments)
        .output()
        .unwrap();
    assert!(
        run.status.success(),
        "node: {}{}",
        String::from_utf8_lossy(&run.stdout),
        String::from_utf8_lossy(&run.stderr)
    );
    out
}

fn result(pair: &Pair) -> Value {
    read_json(&wasm_outputs().join(&pair.name).join("result.json"))
}

#[test]
fn native_and_wasm_print_the_same_document_sets() {
    if golden().is_none() {
        return;
    }
    let native = tempfile::tempdir().unwrap();
    let mut failures = Vec::new();
    for pair in pairs().iter().filter(|pair| pair.name != LARGEST) {
        let documents = result(pair)["documents"].clone();
        if documents.get("error").is_some() {
            failures.push(format!("{}: wasm: {}", pair.name, documents["error"]));
            continue;
        }
        let target = native.path().join(&pair.name);
        let run = slidock(&[
            "compile".as_ref(),
            golden().unwrap().source(pair).as_os_str(),
            "--out".as_ref(),
            target.as_os_str(),
            "--emit".as_ref(),
            "documents".as_ref(),
        ]);
        assert!(run.status.code().unwrap() <= 1, "{}: {run:?}", pair.name);
        let ours = read_tree(&target);
        let theirs = read_tree(&wasm_outputs().join(&pair.name).join("documents"));
        assert!(!ours.is_empty(), "{}", pair.name);
        let paths = |tree: &std::collections::BTreeMap<String, Vec<u8>>| {
            tree.keys().cloned().collect::<Vec<_>>()
        };
        if paths(&ours) != paths(&theirs) {
            failures.push(format!("{}: the file sets differ", pair.name));
            continue;
        }
        for (path, bytes) in &ours {
            if theirs[path] != *bytes {
                failures.push(format!("{}: {path} differs", pair.name));
            }
        }
        eprintln!("{}: {} files identical", pair.name, ours.len());
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// Needs `slidock_bundle::{slide_templates, bundle}`: fails while they return `Unimplemented`.
#[test]
fn native_and_wasm_write_the_same_zips() {
    if golden().is_none() {
        return;
    }
    let native = tempfile::tempdir().unwrap();
    let mut failures = Vec::new();
    let mut with_substitutes = 0;
    for pair in pairs().iter().filter(|pair| pair.name != LARGEST) {
        let zip = native.path().join(format!("{}.zip", pair.name));
        let run = slidock(&[
            "compile".as_ref(),
            golden().unwrap().source(pair).as_os_str(),
            "-o".as_ref(),
            zip.as_os_str(),
            "--no-google".as_ref(),
        ]);
        if run.status.code().unwrap() > 1 || !zip.is_file() {
            failures.push(format!(
                "{}: native: {}",
                pair.name,
                String::from_utf8_lossy(&run.stderr)
            ));
            continue;
        }
        let worker = result(pair)["worker"].clone();
        let errors = worker["errors"].as_array().unwrap();
        if !errors.is_empty() {
            failures.push(format!("{}: wasm: {errors:?}", pair.name));
            continue;
        }
        let ours = std::fs::read(&zip).unwrap();
        let theirs = std::fs::read(wasm_outputs().join(&pair.name).join("deck.zip")).unwrap();
        let mut archive = zip::ZipArchive::new(std::io::Cursor::new(&ours)).unwrap();
        let mut licences = String::new();
        std::io::Read::read_to_string(&mut archive.by_name("LICENSES.txt").unwrap(), &mut licences)
            .unwrap();
        if licences.contains("(a slidock substitute)") {
            with_substitutes += 1;
        }
        if ours != theirs {
            failures.push(format!(
                "{}: the zips differ ({} and {} bytes)",
                pair.name,
                ours.len(),
                theirs.len()
            ));
        }
        let chunks: Vec<u64> = worker["chunks"]
            .as_array()
            .unwrap()
            .iter()
            .map(|n| n.as_u64().unwrap())
            .collect();
        let (last, whole) = chunks.split_last().unwrap();
        if whole.iter().any(|&n| n != CHUNK_BYTES) || *last == 0 || *last > CHUNK_BYTES {
            failures.push(format!("{}: chunks {chunks:?}", pair.name));
        }
        let slices = worker["rangeMedia"].as_object().unwrap();
        if slices.values().any(|same| same != true) {
            failures.push(format!("{}: media slices {slices:?}", pair.name));
        }
        eprintln!(
            "{}: {} bytes identical (sha256 {}), {} chunks, {} media slices, peak wasm memory {} MiB",
            pair.name,
            ours.len(),
            Sha256::of(&ours).to_hex(),
            chunks.len(),
            slices.len(),
            worker["peakMemory"].as_u64().unwrap() / CHUNK_BYTES
        );
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
    eprintln!("{with_substitutes} decks ship bundled substitutes");
    assert!(with_substitutes > 0);
}

/// Needs `slidock_bundle::bundle`: fails while it returns `Unimplemented`.
#[test]
fn the_largest_deck_writes_the_same_zip_twice_natively() {
    if golden().is_none() {
        return;
    }
    let pair = pairs()
        .into_iter()
        .find(|pair| pair.name == LARGEST)
        .unwrap();
    let out = tempfile::tempdir().unwrap();
    let zips: Vec<Vec<u8>> = ["a.zip", "b.zip"]
        .iter()
        .map(|name| {
            let zip = out.path().join(name);
            let run = slidock(&[
                "compile".as_ref(),
                golden().unwrap().source(&pair).as_os_str(),
                "-o".as_ref(),
                zip.as_os_str(),
                "--no-google".as_ref(),
            ]);
            assert!(
                run.status.code().unwrap() <= 1 && zip.is_file(),
                "{}",
                String::from_utf8_lossy(&run.stderr)
            );
            std::fs::read(&zip).unwrap()
        })
        .collect();
    assert_eq!(zips[0].len(), zips[1].len());
    assert!(zips[0] == zips[1], "two runs wrote different zips");
}

/// Writes the web app's e2e smoke deck (`apps/web/e2e/fixture/pptx.ts`) into `dir`.
fn smoke_deck(dir: &Path) -> PathBuf {
    let path = dir.join("slidock-smoke.pptx");
    let fixture = repo().join("apps/web/e2e/fixture/pptx.ts");
    let script = format!(
        "import {{ writeFileSync }} from 'node:fs'; import {{ smokePptx }} from {:?}; writeFileSync({:?}, smokePptx());",
        fixture.to_str().unwrap(),
        path.to_str().unwrap()
    );
    let run = Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .output()
        .unwrap();
    assert!(
        run.status.success() && path.is_file(),
        "node: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    path
}

#[test]
fn native_and_wasm_write_the_same_zips_on_the_smoke_deck_and_the_probes() {
    let work = tempfile::tempdir().unwrap();
    let probes = repo().join("probes/out");
    let decks = [
        ("smoke.pptx", smoke_deck(work.path())),
        ("probe.odp", probes.join("fonts-probe.odp")),
        ("probe.pptx", probes.join("fonts-probe.pptx")),
    ];
    let arguments: Vec<OsString> = decks
        .iter()
        .map(|(name, input)| {
            let mut argument = OsString::from(format!("{name}="));
            argument.push(input);
            argument
        })
        .collect();
    let wasm = run_node(&arguments);
    let mut failures = Vec::new();
    for (name, input) in &decks {
        let zip = work.path().join(format!("{name}.zip"));
        let run = slidock(&[
            "compile".as_ref(),
            input.as_os_str(),
            "-o".as_ref(),
            zip.as_os_str(),
            "--no-google".as_ref(),
        ]);
        assert!(run.status.code().unwrap() <= 1, "{name}: {run:?}");
        let worker = read_json(&wasm.join(name).join("result.json"))["worker"].clone();
        assert_eq!(worker["errors"], serde_json::json!([]), "{name}");
        let ours = std::fs::read(&zip).unwrap();
        let theirs = std::fs::read(wasm.join(name).join("deck.zip")).unwrap();
        let rules = shipped_rules(&zip);
        let substitutes = rules
            .iter()
            .filter(|rule| {
                ["\"Liberation ", "\"DejaVu ", "\"Carlito\"", "\"Caladea\""]
                    .iter()
                    .any(|family| rule.starts_with(family))
            })
            .count();
        eprintln!(
            "{name}: {} and {} bytes, sha256 {} and {}, {} faces shipped ({substitutes} substitutes)",
            ours.len(),
            theirs.len(),
            Sha256::of(&ours).to_hex(),
            Sha256::of(&theirs).to_hex(),
            rules.len(),
        );
        if ours != theirs {
            failures.push(format!("{name}: the zips differ"));
        }
        if substitutes == 0 && *name != "probe.odp" {
            failures.push(format!("{name}: ships no substitute: {rules:?}"));
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

/// The faces a deck ships: its `@font-face` rules' families, weights and styles, in order.
fn shipped_rules(zip: &Path) -> Vec<String> {
    let mut archive = zip::ZipArchive::new(std::fs::File::open(zip).unwrap()).unwrap();
    let mut html = String::new();
    std::io::Read::read_to_string(&mut archive.by_name("index.html").unwrap(), &mut html).unwrap();
    html.split("@font-face{")
        .skip(1)
        .map(|rule| {
            let field = |name: &str| {
                rule.split(';')
                    .find_map(|part| part.strip_prefix(name))
                    .unwrap_or("")
                    .to_owned()
            };
            format!(
                "{} {} {}",
                field("font-family:"),
                field("font-weight:"),
                field("font-style:")
            )
        })
        .collect()
}

#[test]
fn native_and_wasm_ship_the_same_fonts_from_embedded_and_dropped_files() {
    let probes = repo().join("probes/out");
    let font = repo().join("crates/slidock-fonts/tests/data/container/barlow.ttf");
    let dropped = tempfile::tempdir().unwrap();
    std::fs::copy(&font, dropped.path().join("barlow.ttf")).unwrap();
    let pairs = [
        ("probe.odp", "fonts-probe.odp"),
        ("probe.pptx", "fonts-probe.pptx"),
    ];
    let mut arguments: Vec<std::ffi::OsString> = vec![{
        let mut argument = std::ffi::OsString::from("--font=");
        argument.push(&font);
        argument
    }];
    for (name, file) in pairs {
        let mut argument = std::ffi::OsString::from(format!("{name}="));
        argument.push(probes.join(file));
        arguments.push(argument);
    }
    let wasm = run_node(&arguments);
    let native = tempfile::tempdir().unwrap();
    let mut shipped = Vec::new();
    for (name, file) in pairs {
        let zip = native.path().join(format!("{name}.zip"));
        let run = slidock(&[
            "compile".as_ref(),
            probes.join(file).as_os_str(),
            "-o".as_ref(),
            zip.as_os_str(),
            "--no-google".as_ref(),
            "--fonts".as_ref(),
            dropped.path().as_os_str(),
        ]);
        assert!(run.status.code().unwrap() <= 1, "{name}: {run:?}");
        let theirs = wasm.join(name).join("deck.zip");
        let worker = read_json(&wasm.join(name).join("result.json"))["worker"].clone();
        assert_eq!(worker["errors"], serde_json::json!([]), "{name}");
        assert!(
            std::fs::read(&zip).unwrap() == std::fs::read(&theirs).unwrap(),
            "{name}: the zips differ"
        );
        shipped.push(shipped_rules(&zip));
    }
    // The ODP ships its eleven embedded faces (the dropped Barlow Condensed fills no slot embedded faces fill); the
    // PPTX embeds none, so the dropped file is its one shipped face of a family it names, and the bundled substitutes
    // draw the rest. Rules define the families the faces are bound as, hiding installed fonts of those names; the last
    // resort's rule (the letters the dropped Barlow subset lacks) shares DejaVu Sans's descriptor with Symbol's
    // stand-in.
    assert_eq!(shipped[0].len(), 11, "{:?}", shipped[0]);
    assert_eq!(
        shipped[1],
        [
            "\"Barlow Condensed\" 400 normal",
            "\"DejaVu Sans\" 400 normal",
            "\"DejaVu Sans\" 400 normal",
            "\"Liberation Sans\" 400 normal",
            "\"Liberation Sans\" 400 italic",
            "\"Liberation Sans\" 700 normal",
            "\"Liberation Sans\" 700 italic",
            "\"Liberation Serif\" 400 normal",
            "\"Liberation Serif\" 400 italic",
            "\"Liberation Serif\" 700 normal",
            "\"Liberation Serif\" 700 italic",
        ]
    );
}

/// The refusal codes of a worker error in a `result.json`.
fn refusal_codes(error: &Value) -> Vec<String> {
    assert_eq!(error["kind"], "refused", "{error}");
    error["diagnostics"]
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["code"].as_str().unwrap().to_owned())
        .collect()
}

/// The wasm build leaves the IR input out: an IR document the native CLI compiles is refused by the module, through the
/// compiler called directly and through the worker alike, as an input no frontend reads (IR0902). The document is built
/// here, so the test runs without the golden files.
#[test]
fn the_wasm_module_refuses_an_ir_document_the_cli_compiles() {
    use slidock_ir::fixture::{Slide, TextBox, presentation_json};
    use slidock_ir::ids::NodeId;
    let out = tempfile::tempdir().unwrap();
    let slide = Slide::new(
        NodeId::new("ir-s1").unwrap(),
        vec![TextBox::new(
            NodeId::new("ir-s1.a").unwrap(),
            [72.0, 72.0, 288.0, 72.0],
            &["Hello"],
        )],
    );
    let input = out.path().join("min.presentation-ir.json");
    std::fs::write(
        &input,
        serde_json::to_vec(&presentation_json(&[slide]).unwrap()).unwrap(),
    )
    .unwrap();
    let native = slidock(&[
        "compile".as_ref(),
        input.as_os_str(),
        "--out".as_ref(),
        out.path().join("min").as_os_str(),
        "--emit".as_ref(),
        "documents".as_ref(),
    ]);
    assert!(
        native.status.success(),
        "{}",
        String::from_utf8_lossy(&native.stderr)
    );
    let mut argument = OsString::from("min=");
    argument.push(&input);
    let wasm = run_node(&[argument]);
    let result = read_json(&wasm.join("min/result.json"));
    assert_eq!(refusal_codes(&result["documents"]["error"]), ["IR0902"]);
    let errors = result["worker"]["errors"].as_array().unwrap();
    assert_eq!(errors[0]["id"], 1, "{errors:?}");
    assert_eq!(refusal_codes(&errors[0]["error"]), ["IR0902"]);
    assert_eq!(result["worker"]["slides"], Value::Null);
}
