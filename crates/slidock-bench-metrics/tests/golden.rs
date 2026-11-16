//! The metrics on the corpus goldens. Each row's scoring inputs under `$SLIDOCK_PRIVATE/golden/scoring/rows/<row>/`
//! (`$SLIDOCK_PRIVATE` defaults to `~/.slidock`): `oracle.png`, `render.png`, its text-less twin `textless.png`, and
//! `inputs.json` (the slide size, the oracle page's size and words, the measured DOM words, numbers as hex IEEE-754
//! bit patterns) score to that row's `scores.json` byte for byte: the raw scores under `HostLibm` as bit patterns, the
//! quantised record and the class. Under `Fdlibm` the quantised record and the class are the same. `index.json` lists
//! every file of the set with its sha256, and the committed `tests/golden/scoring.json` pins the index's sha256, so a
//! changed byte in any input or score fails. The test says it skipped, and passes, when the set is absent.
//!
//! `UPDATE_GOLDEN=1 GOLDEN_REASON=<why> cargo test --release -p slidock-bench-metrics --features golden --test golden`
//! rewrites every `scores.json`, the index and the pin, which records the reason. `cargo xtask golden --record` runs
//! it so, and `tests/golden/manifest.json` pins every file of the set as well; `cargo xtask golden --check` runs this
//! suite in release (the crate's `[package.metadata.golden]`).

#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use slidock_bench_metrics::{
    Fdlibm, HostLibm, Libm, OracleBlock, OracleLine, OracleText, OracleWord, Rect, Rgba, Scores,
    Size, TextWord, decode_png, oracle_words, quantise_scores, score_render, scored_class,
};

/// The committed pin of the set.
const PIN: &str = "tests/golden/scoring.json";

fn root() -> PathBuf {
    std::env::var_os("SLIDOCK_PRIVATE")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var_os("HOME").unwrap()).join(".slidock"))
        .join("golden/scoring")
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

/// The files' JSON: keys in the order built (the index's sorted), two-space indent, trailing newline.
fn canonical(value: &Value) -> String {
    let mut text = serde_json::to_string_pretty(value).unwrap();
    text.push('\n');
    text
}

fn float(value: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(value.as_str().expect("a bit pattern"), 16).expect("hex"))
}

fn bits(value: f64) -> Value {
    json!(format!("{:016x}", value.to_bits()))
}

fn size(value: &Value) -> Size {
    Size {
        w: float(&value["w"]),
        h: float(&value["h"]),
    }
}

fn list<'a>(value: &'a Value, key: &str) -> &'a Vec<Value> {
    value[key].as_array().expect(key)
}

fn oracle_text(value: &Value) -> OracleText {
    OracleText {
        width: float(&value["width"]),
        height: float(&value["height"]),
        blocks: list(value, "blocks")
            .iter()
            .map(|block| OracleBlock {
                lines: list(block, "lines")
                    .iter()
                    .map(|line| OracleLine {
                        words: list(line, "words")
                            .iter()
                            .map(|w| OracleWord {
                                text: w["text"].as_str().expect("text").to_owned(),
                                x_min: float(&w["xMin"]),
                                y_min: float(&w["yMin"]),
                                x_max: float(&w["xMax"]),
                                y_max: float(&w["yMax"]),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

/// A score set as `scores.json` holds it: every number as its bit pattern, `null` where it has none.
fn numbers(scores: &Scores) -> Value {
    let t = &scores.text;
    let optional = |x: Option<f64>| x.map_or(Value::Null, bits);
    json!({
        "ssim": { "mean": bits(scores.ssim.mean), "p1": bits(scores.ssim.p1) },
        "color": { "mean": bits(scores.color.mean), "p99": bits(scores.color.p99) },
        "pixel": bits(scores.pixel),
        "text": {
            "oracleWords": t.oracle_words,
            "domWords": t.dom_words,
            "matched": t.matched,
            "recall": bits(t.recall),
            "precision": bits(t.precision),
            "medianPm": optional(t.median_pm),
            "p95Pm": optional(t.p95_pm),
            "heightRatioMedian": optional(t.height_ratio_median),
        },
    })
}

struct Inputs {
    oracle: Rgba,
    render: Rgba,
    textless: Rgba,
    words: Vec<TextWord>,
    dom: Vec<TextWord>,
    slide: Size,
}

fn read_inputs(dir: &Path) -> Inputs {
    let read = |name: &str| {
        let path = dir.join(name);
        decode_png(&std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display())))
            .expect("decode")
    };
    let inputs: Value =
        serde_json::from_slice(&std::fs::read(dir.join("inputs.json")).expect("inputs.json"))
            .expect("json");
    let slide = size(&inputs["slide"]);
    let words = oracle_words(
        &oracle_text(&inputs["oracle"]["text"]),
        size(&inputs["oracle"]["size"]),
        slide,
    );
    let dom = list(&inputs, "measure")
        .iter()
        .map(|w| TextWord {
            text: w["text"].as_str().expect("text").to_owned(),
            rect: Rect {
                x: float(&w["x"]),
                y: float(&w["y"]),
                w: float(&w["w"]),
                h: float(&w["h"]),
            },
        })
        .collect();
    Inputs {
        oracle: read("oracle.png"),
        render: read("render.png"),
        textless: read("textless.png"),
        words,
        dom,
        slide,
    }
}

fn score<M: Libm>(inputs: &Inputs) -> Scores {
    score_render::<M>(
        &inputs.oracle,
        &inputs.render,
        &inputs.textless,
        &inputs.words,
        &inputs.dom,
        inputs.slide,
    )
    .expect("scores")
}

/// A row's `scores.json` text under `HostLibm`, and whether `Fdlibm` gives the same quantised record and class.
fn scores_of(dir: &Path) -> (String, bool) {
    let inputs = read_inputs(dir);
    let host = score::<HostLibm>(&inputs);
    let portable = score::<Fdlibm>(&inputs);
    let class = |scores: &Scores| scored_class(scores).outcome().as_str();
    let same = numbers(&quantise_scores(&host)) == numbers(&quantise_scores(&portable))
        && class(&host) == class(&portable);
    let text = canonical(&json!({
        "raw": numbers(&host),
        "quantised": numbers(&quantise_scores(&host)),
        "class": class(&host),
    }));
    (text, same)
}

/// Every row folder (one holding `inputs.json`), sorted.
fn row_dirs(root: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::fs::read_dir(root.join("rows"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.join("inputs.json").is_file())
        .collect();
    dirs.sort();
    dirs
}

/// `index.json`: every file under `rows/` with its sha256.
fn index(root: &Path, dirs: &[PathBuf]) -> String {
    let mut files = BTreeMap::new();
    for dir in dirs {
        let mut names: Vec<PathBuf> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .collect();
        names.sort();
        for path in names {
            let relative = path
                .strip_prefix(root)
                .unwrap()
                .to_string_lossy()
                .into_owned();
            files.insert(relative, json!(sha256(&std::fs::read(&path).unwrap())));
        }
    }
    canonical(&json!({ "files": files }))
}

/// Scores every row on all cores.
fn score_all(dirs: &[PathBuf]) -> Vec<(PathBuf, String, bool)> {
    let threads = std::thread::available_parallelism().map_or(4, usize::from);
    let chunk = dirs.len().div_ceil(threads).max(1);
    std::thread::scope(|scope| {
        let handles: Vec<_> = dirs
            .chunks(chunk)
            .map(|part| {
                scope.spawn(|| {
                    part.iter()
                        .map(|dir| {
                            let (text, same) = scores_of(dir);
                            (dir.clone(), text, same)
                        })
                        .collect::<Vec<_>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().expect("thread"))
            .collect()
    })
}

fn record(root: &Path, pin_path: &Path, dirs: &[PathBuf]) {
    let reason = std::env::var("GOLDEN_REASON").unwrap_or_default();
    assert!(
        !reason.trim().is_empty(),
        "re-recording states its reason: set GOLDEN_REASON"
    );
    for (dir, text, _) in score_all(dirs) {
        std::fs::write(dir.join("scores.json"), text).unwrap();
    }
    let index = index(root, dirs);
    std::fs::write(root.join("index.json"), &index).unwrap();
    let pin = canonical(&json!({
        "index": sha256(index.as_bytes()),
        "reason": reason,
        "rows": dirs.len(),
    }));
    std::fs::write(pin_path, pin).unwrap();
    eprintln!("recorded {} rows: {}", dirs.len(), pin_path.display());
}

#[test]
fn every_scoring_input_scores_to_its_golden() {
    let root = root();
    if !root.join("rows").is_dir() {
        eprintln!("skipped: no scoring goldens at {}", root.display());
        return;
    }
    let pin_path = Path::new(env!("CARGO_MANIFEST_DIR")).join(PIN);
    let dirs = row_dirs(&root);
    if std::env::var_os("UPDATE_GOLDEN").is_some() {
        record(&root, &pin_path, &dirs);
        return;
    }
    let pin: Value = serde_json::from_slice(&std::fs::read(&pin_path).unwrap()).unwrap();
    assert_eq!(pin["rows"], json!(dirs.len()), "rows in {}", root.display());
    let index_text = std::fs::read_to_string(root.join("index.json")).unwrap();
    assert_eq!(
        json!(sha256(index_text.as_bytes())),
        pin["index"],
        "{}/index.json is not the pinned one",
        root.display()
    );
    let found = index(&root, &dirs);
    if found != index_text {
        let pinned: Value = serde_json::from_str(&index_text).unwrap();
        let found: Value = serde_json::from_str(&found).unwrap();
        let changed: Vec<&String> = pinned["files"]
            .as_object()
            .unwrap()
            .keys()
            .chain(found["files"].as_object().unwrap().keys())
            .filter(|path| pinned["files"].get(path.as_str()) != found["files"].get(path.as_str()))
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .take(10)
            .collect();
        panic!("files differ from the pinned index: {changed:?}");
    }
    let mut failures = Vec::new();
    for (dir, text, same) in score_all(&dirs) {
        let name = dir.file_name().unwrap().to_string_lossy().into_owned();
        let golden = std::fs::read_to_string(dir.join("scores.json")).unwrap();
        if text != golden {
            failures.push(format!("{name}: scores differ from scores.json"));
        }
        if !same {
            failures.push(format!(
                "{name}: Fdlibm moves the quantised record or the class"
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} rows fail: {:#?}",
        failures.len(),
        &failures[..failures.len().min(20)]
    );
    eprintln!("{} rows score to their goldens", dirs.len());
}
