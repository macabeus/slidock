//! The golden files through the binary (`--features golden`, tests/golden; skips when they are absent).
//!
//! - `slidock compile --emit documents,ir,report` on every private IR fixture, into a temporary folder, writes the
//!   files recorded for it, byte for byte: the document set, its media, the resolved IR and the report.
//! - Every corpus pair of the golden manifest, compiled from its pinned source with the bench's static-lane arguments
//!   (`--frontend`, `--emit documents,report,ir`, `--fonts` with the pinned font set), writes the files recorded for
//!   it, byte for byte.
//!
//! Run with `--nocapture` for the per-pair table.
#![cfg(feature = "golden")]
#![allow(
    clippy::unwrap_used,
    clippy::indexing_slicing,
    clippy::panic,
    reason = "test code"
)]

use std::path::Path;
use std::process::Command;

mod support;

use support::{Golden, golden};

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

/// Runs the binary, which must succeed with no error line.
fn compile(input: &Path, out: &Path, args: &[&str]) {
    let run = Command::new(env!("CARGO_BIN_EXE_slidock"))
        .arg("compile")
        .arg(input)
        .arg("--out")
        .arg(out)
        .args(args)
        .env("TZ", "UTC")
        .env("LANG", "C")
        .env("LC_ALL", "C")
        .output()
        .unwrap();
    let stderr = text(&run.stderr);
    assert_eq!(run.status.code(), Some(0), "{}: {stderr}", input.display());
    assert!(
        !stderr.lines().any(|line| line.starts_with("error")),
        "{}: {stderr}",
        input.display()
    );
}

/// Every file the compile wrote under `dir` against the recording under `<prefix>/`, one file at a time; returns the
/// differences and the files compared.
fn compare_written(golden: &Golden, prefix: &str, dir: &Path) -> (Vec<String>, usize) {
    let recorded = golden.output_paths(prefix);
    let mut written = Vec::new();
    let mut problems = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(dir)
                .unwrap()
                .to_str()
                .unwrap()
                .replace('\\', "/");
            if recorded.contains(&relative) {
                let bytes = std::fs::read(&path).unwrap();
                problems.extend(golden.compare(&format!("{prefix}/{relative}"), &bytes));
            }
            written.push(relative);
        }
    }
    let compared = written.len();
    problems.extend(golden.compare_paths(prefix, written));
    (problems, compared)
}

#[test]
fn slidock_compile_writes_every_ir_fixture_as_recorded() {
    let Some(golden) = golden() else {
        return;
    };
    let out = tempfile::tempdir().unwrap();
    let names = golden.ir_fixtures();
    assert_eq!(names.len(), 6, "the private fixtures hold six IR fixtures");
    let mut failures = Vec::new();
    let mut compared = 0;
    for name in &names {
        let target = out.path().join(name);
        let input = golden.fixture(&format!("ir/{name}.presentation-ir.json"));
        compile(&input, &target, &["--emit", "documents,ir,report"]);
        let (problems, files) = compare_written(golden, &format!("fixtures/{name}"), &target);
        failures.extend(problems);
        compared += files;
    }
    eprintln!(
        "slidock compile: {} IR fixtures, {compared} files compared, {} differ",
        names.len(),
        failures.len()
    );
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn every_corpus_pair_compiles_as_recorded() {
    let Some(golden) = golden() else {
        return;
    };
    let pairs = golden.pairs();
    assert!(
        support::corpus_pairs(golden).len() >= 10,
        "the corpus holds ten non-PDF pairs"
    );
    let fonts = golden.fonts();
    let fonts = fonts.to_str().unwrap();
    let out = tempfile::tempdir().unwrap();
    let mut failures = Vec::new();
    let mut table = vec![format!("{:<24} {:>6} {:>7}", "pair", "files", "differ")];
    for pair in pairs {
        let target = out.path().join(&pair.name);
        let args = [
            "--frontend",
            pair.frontend.as_str(),
            "--emit",
            "documents,report,ir",
            "--fonts",
            fonts,
        ];
        compile(&golden.source(pair), &target, &args);
        let (problems, files) = compare_written(golden, &format!("corpus/{}", pair.name), &target);
        table.push(format!(
            "{:<24} {files:>6} {:>7}",
            pair.name,
            problems.len()
        ));
        failures.extend(problems);
        std::fs::remove_dir_all(&target).unwrap();
    }
    eprintln!("{}", table.join("\n"));
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}
