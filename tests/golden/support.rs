//! The golden files the `golden` suites compare with: slidock's own outputs, recorded by `cargo xtask golden --record`
//! into `$SLIDOCK_PRIVATE/golden/<commit>/` (corpus-derived, private, never committed) and pinned by SHA-256 in
//! `tests/golden/manifest.json`, which also pins the inputs they were made from and the suites read: the corpus pairs,
//! the bench's font set, the private fixtures in `$SLIDOCK_PRIVATE/golden/fixtures/` and the Keynote motion
//! expectations in `$SLIDOCK_PRIVATE/expect/keynote-motion/`.
//!
//! The repository names a corpus source by its presentation id only, `corpus:<presentation>.<frontend>`: the private
//! corpus manifest (`$SLIDOCK_PRIVATE/bench/corpus/manifest.json`) gives that presentation's source file for the
//! frontend, under `$SLIDOCK_PRIVATE/corpus/`.
//!
//! A recording holds:
//! - `corpus/<pair>/`: `slidock compile <source> --frontend <frontend> --emit documents,report,ir --fonts
//!   $SLIDOCK_PRIVATE/bench/cache/fonts`, the bench's static lane, for every pair of the manifest;
//! - `fixtures/<name>/`: `slidock compile <fixture> --emit documents,ir,report` for every IR fixture
//!   (`golden/fixtures/ir/<name>.presentation-ir.json`);
//! - `<crate>/…`: what a crate's own suite records (an object graph, a font's fields, a frontend's lowering).
//!
//! A suite skips, saying so, when the recording the manifest names is absent. A pinned file whose bytes differ from its
//! SHA-256, or an output that differs from its golden, fails it. While `cargo xtask golden --record` runs the suites,
//! `SLIDOCK_GOLDEN_RECORD` names the folder being recorded: [`Golden::record`] writes there, and [`Golden::compare`]
//! reads the outputs the CLI wrote there before.
//!
//! Each crate's suites include this file by path: `#[path = "../../../tests/golden/support.rs"] mod golden;`.
#![allow(
    dead_code,
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::indexing_slicing,
    reason = "test support: each suite uses part of it, and a broken recording fails the suite"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

/// The folder `cargo xtask golden --record` is recording into.
pub const RECORD_ENV: &str = "SLIDOCK_GOLDEN_RECORD";

/// The repository root.
pub fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// `$SLIDOCK_PRIVATE`, by default `~/.slidock`.
pub fn private() -> PathBuf {
    std::env::var_os("SLIDOCK_PRIVATE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").unwrap()).join(".slidock"))
}

/// The font set the static lane compiles with.
pub const FONTS: &str = "bench/cache/fonts";

/// The corpus manifest, relative to `$SLIDOCK_PRIVATE`.
pub const CORPUS_MANIFEST: &str = "bench/corpus/manifest.json";

/// The file of `corpus:<presentation>.<frontend>` (without its prefix), or `None` when the corpus manifest is absent.
/// The manifest's presentation of that id names the source file of that frontend, under `$SLIDOCK_PRIVATE/corpus/`.
pub fn corpus_source(source: &str) -> Option<PathBuf> {
    let path = private().join(CORPUS_MANIFEST);
    let text = std::fs::read_to_string(&path).ok()?;
    let manifest: Value =
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let (id, frontend) = source
        .rsplit_once('.')
        .unwrap_or_else(|| panic!("corpus:{source}: a source is <presentation>.<frontend>"));
    let presentation = manifest["presentations"]
        .as_array()
        .unwrap()
        .iter()
        .find(|presentation| presentation["id"] == id)
        .unwrap_or_else(|| panic!("{} has no presentation {id}", path.display()));
    let file = presentation["sources"]
        .as_array()
        .unwrap()
        .iter()
        .find(|source| source["frontend"] == frontend)
        .and_then(|source| source["file"].as_str())
        .unwrap_or_else(|| panic!("{}: {id} has no {frontend} source", path.display()));
    Some(private().join("corpus").join(file))
}

/// The file a pinned input names: `repo:<path>` in the repository, `corpus:<presentation>.<frontend>` in the corpus,
/// any other path under `$SLIDOCK_PRIVATE`.
pub fn input_path(input: &str) -> PathBuf {
    if let Some(rest) = input.strip_prefix("repo:") {
        repo().join(rest)
    } else if let Some(source) = input.strip_prefix("corpus:") {
        corpus_source(source).unwrap_or_else(|| {
            panic!(
                "{input}: the corpus manifest {} is absent",
                private().join(CORPUS_MANIFEST).display()
            )
        })
    } else {
        private().join(input)
    }
}

/// A corpus pair of the manifest: `<presentation>.<frontend>` and its source file.
#[derive(Clone, Debug)]
pub struct Pair {
    pub name: String,
    pub presentation: String,
    pub frontend: String,
    /// The pinned input: `corpus:<presentation>.<frontend>`, or `repo:<path>` in the repository.
    pub input: String,
}

pub struct Golden {
    commit: String,
    /// Where the outputs are read and written: `$SLIDOCK_PRIVATE/golden/<commit>`, or the folder being recorded.
    root: PathBuf,
    recording: bool,
    pairs: Vec<Pair>,
    inputs: BTreeMap<String, String>,
    outputs: BTreeMap<String, String>,
}

fn strings(manifest: &Value, key: &str) -> BTreeMap<String, String> {
    manifest[key]
        .as_object()
        .unwrap_or_else(|| panic!("tests/golden/manifest.json has no {key}"))
        .iter()
        .map(|(path, sha)| (path.clone(), sha.as_str().unwrap().to_owned()))
        .collect()
}

/// SHA-256s already checked in this process, by path: a 1 GB source is hashed once.
static CHECKED: Mutex<BTreeMap<PathBuf, String>> = Mutex::new(BTreeMap::new());

fn sha256_file(path: &Path) -> String {
    if let Some(sha) = CHECKED.lock().unwrap().get(path) {
        return sha.clone();
    }
    let bytes = std::fs::read(path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    let sha = slidock_canon::sha256_hex(&bytes);
    CHECKED
        .lock()
        .unwrap()
        .insert(path.to_path_buf(), sha.clone());
    sha
}

impl Golden {
    /// The recording the manifest names, or `None` once the suite has said it skips because that recording is absent.
    pub fn open(suite: &str) -> Option<Golden> {
        let path = repo().join("tests/golden/manifest.json");
        let manifest: Value = serde_json::from_slice(
            &std::fs::read(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display())),
        )
        .unwrap();
        assert_eq!(manifest["version"], 1, "{}: version", path.display());
        let commit = manifest["commit"].as_str().unwrap().to_owned();
        let recording_into = std::env::var_os(RECORD_ENV);
        if commit.is_empty() && recording_into.is_none() {
            eprintln!("skipped {suite}: tests/golden/manifest.json names no recording yet");
            return None;
        }
        let (root, recording) = match recording_into {
            Some(dir) => (PathBuf::from(dir), true),
            None => (private().join("golden").join(&commit), false),
        };
        if !recording && !root.is_dir() {
            eprintln!(
                "skipped {suite}: the golden files of {commit} are absent ({}). They are slidock's outputs on the \
                 private corpus and font set, which recording them needs too",
                root.display()
            );
            return None;
        }
        let pairs = strings(&manifest, "pairs")
            .into_iter()
            .map(|(name, input)| {
                let (presentation, frontend) = name.rsplit_once('.').unwrap();
                Pair {
                    presentation: presentation.to_owned(),
                    frontend: frontend.to_owned(),
                    name,
                    input,
                }
            })
            .collect();
        Some(Golden {
            commit,
            root,
            recording,
            pairs,
            inputs: strings(&manifest, "inputs"),
            outputs: if recording {
                BTreeMap::new()
            } else {
                strings(&manifest, "outputs")
            },
        })
    }

    pub fn commit(&self) -> &str {
        &self.commit
    }

    pub fn recording(&self) -> bool {
        self.recording
    }

    /// The corpus pairs, by name.
    pub fn pairs(&self) -> &[Pair] {
        &self.pairs
    }

    /// A pinned input, checked against its SHA-256 (while recording, it is pinned afterwards).
    pub fn input(&self, input: &str) -> PathBuf {
        let path = input_path(input);
        if !self.recording {
            let pinned = self
                .inputs
                .get(input)
                .unwrap_or_else(|| panic!("{input} is not pinned in tests/golden/manifest.json"));
            let sha = sha256_file(&path);
            assert_eq!(
                &sha,
                pinned,
                "{} differs from the file the goldens were recorded from",
                path.display()
            );
        }
        path
    }

    /// The source of a pair.
    pub fn source(&self, pair: &Pair) -> PathBuf {
        self.input(&pair.input)
    }

    /// Every file under `$SLIDOCK_PRIVATE/<dir>/`, in path order, each checked against its pin: a file there that is
    /// not pinned, or a pinned one that is missing, fails (while recording, every file there is pinned afterwards).
    pub fn pinned_files(&self, dir: &str) -> Vec<PathBuf> {
        let folder = private().join(dir);
        let present: Vec<String> = list_tree(&folder)
            .into_iter()
            .map(|file| format!("{dir}/{file}"))
            .collect();
        if !self.recording {
            let prefix = format!("{dir}/");
            let pinned: Vec<&String> = self
                .inputs
                .keys()
                .filter(|input| input.starts_with(&prefix))
                .collect();
            assert_eq!(
                present.iter().collect::<Vec<_>>(),
                pinned,
                "{} holds other files than tests/golden/manifest.json pins",
                folder.display()
            );
        }
        present.iter().map(|input| self.input(input)).collect()
    }

    /// The font set the static lane compiles with, each file checked against its pin.
    pub fn fonts(&self) -> PathBuf {
        self.pinned_files(FONTS);
        private().join(FONTS)
    }

    /// A private fixture: `$SLIDOCK_PRIVATE/golden/fixtures/<path>`, checked against its pin.
    pub fn fixture(&self, path: &str) -> PathBuf {
        self.input(&format!("golden/fixtures/{path}"))
    }

    /// The pinned fixtures under `golden/fixtures/<dir>/` (not in subfolders), by file name.
    pub fn fixture_names(&self, dir: &str) -> Vec<String> {
        let prefix = format!("golden/fixtures/{dir}/");
        let names: Vec<String> = if self.recording {
            std::fs::read_dir(private().join(&prefix))
                .unwrap()
                .map(|entry| entry.unwrap())
                .filter(|entry| entry.file_type().unwrap().is_file())
                .map(|entry| entry.file_name().into_string().unwrap())
                .collect()
        } else {
            self.inputs
                .keys()
                .filter_map(|path| path.strip_prefix(&prefix))
                .filter(|name| !name.contains('/'))
                .map(str::to_owned)
                .collect()
        };
        let mut names = names;
        names.sort();
        names
    }

    /// The IR fixtures' names: `golden/fixtures/ir/<name>.presentation-ir.json`.
    pub fn ir_fixtures(&self) -> Vec<String> {
        self.fixture_names("ir")
            .into_iter()
            .filter_map(|name| {
                name.strip_suffix(".presentation-ir.json")
                    .map(str::to_owned)
            })
            .collect()
    }

    /// The folder of the recording.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// An output's bytes, checked against its pin (while recording, read as the CLI wrote it).
    pub fn output(&self, path: &str) -> Vec<u8> {
        let file = self.root.join(path);
        let bytes =
            std::fs::read(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        if !self.recording {
            let pinned = self
                .outputs
                .get(path)
                .unwrap_or_else(|| panic!("{path} is not a golden output of {}", self.commit));
            assert_eq!(
                &slidock_canon::sha256_hex(&bytes),
                pinned,
                "{} differs from its pin in tests/golden/manifest.json",
                file.display()
            );
        }
        bytes
    }

    /// An output's text.
    pub fn output_text(&self, path: &str) -> String {
        String::from_utf8(self.output(path)).unwrap()
    }

    /// The paths of the outputs under `<prefix>/`, relative to it.
    pub fn output_paths(&self, prefix: &str) -> Vec<String> {
        let start = format!("{prefix}/");
        let mut paths: Vec<String> = if self.recording {
            let mut paths = Vec::new();
            let dir = self.root.join(prefix);
            let mut stack = vec![dir.clone()];
            while let Some(next) = stack.pop() {
                for entry in std::fs::read_dir(&next).unwrap() {
                    let path = entry.unwrap().path();
                    if path.is_dir() {
                        stack.push(path);
                    } else {
                        let relative = path.strip_prefix(&dir).unwrap().to_str().unwrap();
                        paths.push(relative.replace('\\', "/"));
                    }
                }
            }
            paths
        } else {
            self.outputs
                .keys()
                .filter_map(|path| path.strip_prefix(&start))
                .map(str::to_owned)
                .collect()
        };
        assert!(
            !paths.is_empty(),
            "the recording of {} has no {prefix}/",
            self.commit
        );
        paths.sort();
        paths
    }

    /// The outputs under `<prefix>/`, by path relative to it.
    pub fn outputs_under(&self, prefix: &str) -> BTreeMap<String, Vec<u8>> {
        self.output_paths(prefix)
            .into_iter()
            .map(|path| {
                let bytes = self.output(&format!("{prefix}/{path}"));
                (path, bytes)
            })
            .collect()
    }

    /// `actual` against the output the CLI recorded at `path`: `None` when they are the same bytes.
    pub fn compare(&self, path: &str, actual: &[u8]) -> Option<String> {
        let golden = self.output(path);
        (golden != actual).then(|| describe(path, &golden, actual))
    }

    /// The files of `actual` against the outputs under `<prefix>/`: the same paths, each the same bytes. Each
    /// difference is one problem.
    pub fn compare_tree(&self, prefix: &str, actual: &BTreeMap<String, Vec<u8>>) -> Vec<String> {
        let mut problems = self.compare_paths(prefix, actual.keys().cloned().collect());
        let golden = self.output_paths(prefix);
        for (path, bytes) in actual {
            if golden.contains(path) {
                problems.extend(self.compare(&format!("{prefix}/{path}"), bytes));
            }
        }
        problems
    }

    /// The paths a set writes against the outputs under `<prefix>/`: one problem per missing or extra path.
    pub fn compare_paths(&self, prefix: &str, mut actual: Vec<String>) -> Vec<String> {
        actual.sort();
        let golden = self.output_paths(prefix);
        let mut problems: Vec<String> = golden
            .iter()
            .filter(|path| !actual.contains(path))
            .map(|path| format!("{prefix}/{path}: missing"))
            .collect();
        problems.extend(
            actual
                .iter()
                .filter(|path| !golden.contains(path))
                .map(|path| format!("{prefix}/{path}: not in the golden set")),
        );
        problems
    }

    /// A crate's own output: written while recording, else compared with its golden (`None` when equal).
    pub fn record(&self, path: &str, actual: &[u8]) -> Option<String> {
        if self.recording {
            let file = self.root.join(path);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            assert!(!file.exists(), "{path} is recorded twice");
            std::fs::write(&file, actual).unwrap();
            return None;
        }
        self.compare(path, actual)
    }
}

/// Where two texts first differ, or the sizes of two binaries.
pub fn describe(path: &str, golden: &[u8], actual: &[u8]) -> String {
    match (std::str::from_utf8(golden), std::str::from_utf8(actual)) {
        (Ok(golden), Ok(actual)) => {
            let line = golden
                .lines()
                .zip(actual.lines())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| golden.lines().count().min(actual.lines().count()));
            let at = |text: &str| {
                text.lines()
                    .nth(line)
                    .unwrap_or("")
                    .chars()
                    .take(160)
                    .collect::<String>()
            };
            format!(
                "{path}: differs at line {}\n  golden: {}\n  actual: {}",
                line + 1,
                at(golden),
                at(actual)
            )
        }
        _ => format!(
            "{path}: the bytes differ ({} golden, {} actual)",
            golden.len(),
            actual.len()
        ),
    }
}

/// Every file under `dir`, by `/`-separated path relative to it, in path order.
pub fn list_tree(dir: &Path) -> Vec<String> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let entries =
            std::fs::read_dir(&next).unwrap_or_else(|error| panic!("{}: {error}", next.display()));
        for entry in entries {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path.strip_prefix(dir).unwrap().to_str().unwrap();
            files.push(relative.replace('\\', "/"));
        }
    }
    files.sort();
    files
}

/// Every file under `dir`, by path relative to it, as bytes.
pub fn read_tree(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    let mut files = BTreeMap::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path.strip_prefix(dir).unwrap().to_str().unwrap();
            files.insert(relative.replace('\\', "/"), std::fs::read(&path).unwrap());
        }
    }
    files
}
