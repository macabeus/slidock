//! The determinism gate (native and wasm builds write identical bytes). Every selected pair compiles in three legs
//! into fresh folders, each against the bench's font folder, as `run` compiles:
//!
//! - **A**, the native CLI in manifest order, `--verify boundaries`, `TZ=UTC LANG=C LC_ALL=C`: the document set with
//!   its report and IR, and the bundled deck zip (`--fonts-off`: the parity path, no font source and no face shipped);
//! - **A′**, the native CLI again in reverse order, `--verify each`, `TZ=Pacific/Chatham LANG=LC_ALL=tr_TR.UTF-8`;
//! - **B**, the wasm build under Node (`bench/wasm-cli.mjs`, which adds the folder's font files one by one, in the
//!   order the CLI reads them) in reverse order under A′'s zone and locale: the document set and the zip. A pair too
//!   large for wasm32's address space is `native-only`, never skipped silently.
//!
//! A′ must equal A byte for byte (documents, report, IR, zip), and B must equal A (documents, zip). Then, for three
//! rows per pair (first, middle, last), the `--only` compile's slide and layout files must equal A's; with `--score`,
//! every row is scored twice, the second pass in reverse order on the first pass's compile, the records and renders
//! must be equal, and that compile (the one `run` makes) must equal A's; and no JSON or HTML output, in a folder or
//! in a deck zip, may name the cache, the corpus, the home folder, or a temporary or user path. The report names
//! the native binary and the wasm module it compared, and its hash is stamped into the baseline. The work folder is
//! removed when the gate ends, however it ends.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::Serialize;
use slidock_bench_schema::corpus::{Presentation, Source};
use slidock_bench_schema::{RowId, Sha256};
use slidock_format::paths::{layout_path, slide_path};

use crate::cache::{remove, write_atomic};
use crate::compile::{COMPILE_TIMEOUT, RSS_LIMIT_BYTES};
use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, args, exec};
use crate::renderer::route::{manifest_of, read_compiled};

/// Inputs above this size run natively only: wasm32 addresses 4 GiB, and the module holds the deck's output.
pub const WASM_MAX_INPUT_BYTES: u64 = 512 * 1024 * 1024;

/// The first difference between two outputs: where, the byte offset, and what.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Difference {
    pub path: String,
    pub offset: u64,
    pub detail: String,
}

/// The first byte two buffers differ at.
pub fn first_byte_difference(a: &[u8], b: &[u8]) -> Option<u64> {
    let differing = a.iter().zip(b).position(|(x, y)| x != y);
    match differing {
        Some(at) => Some(at as u64),
        None if a.len() == b.len() => None,
        None => Some(a.len().min(b.len()) as u64),
    }
}

/// Every file under `dir`, relative and sorted.
fn files_under(dir: &Path) -> Result<Vec<String>> {
    let mut files = Vec::new();
    if !dir.is_dir() {
        return Ok(files);
    }
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).map_err(BenchError::io(&next))? {
            let entry = entry.map_err(BenchError::io(&next))?;
            let path = entry.path();
            let kind = entry.file_type().map_err(BenchError::io(&path))?;
            if kind.is_dir() {
                stack.push(path);
            } else if kind.is_file()
                && let Ok(relative) = path.strip_prefix(dir)
            {
                files.push(relative.to_string_lossy().into_owned());
            }
        }
    }
    files.sort();
    Ok(files)
}

/// The first difference between two folders' file lists and bytes, leaving out files `skip` names.
pub fn compare_trees(a: &Path, b: &Path, label: &str, skip: &[&str]) -> Result<Option<Difference>> {
    let keep = |files: Vec<String>| -> Vec<String> {
        files
            .into_iter()
            .filter(|f| !skip.contains(&f.as_str()))
            .collect()
    };
    let left = keep(files_under(a)?);
    let right = keep(files_under(b)?);
    if let Some(only) = left.iter().find(|f| !right.contains(f)) {
        return Ok(Some(Difference {
            path: format!("{label}/{only}"),
            offset: 0,
            detail: "only in the first".into(),
        }));
    }
    if let Some(only) = right.iter().find(|f| !left.contains(f)) {
        return Ok(Some(Difference {
            path: format!("{label}/{only}"),
            offset: 0,
            detail: "only in the second".into(),
        }));
    }
    for file in &left {
        let x = std::fs::read(a.join(file)).map_err(BenchError::io(a.join(file)))?;
        let y = std::fs::read(b.join(file)).map_err(BenchError::io(b.join(file)))?;
        if let Some(offset) = first_byte_difference(&x, &y) {
            return Ok(Some(Difference {
                path: format!("{label}/{file}"),
                offset,
                detail: "bytes differ".into(),
            }));
        }
    }
    Ok(None)
}

/// The first difference between two files.
pub fn compare_files(a: &Path, b: &Path, label: &str) -> Option<Difference> {
    match (std::fs::read(a), std::fs::read(b)) {
        (Ok(x), Ok(y)) => first_byte_difference(&x, &y).map(|offset| Difference {
            path: label.to_owned(),
            offset,
            detail: "bytes differ".into(),
        }),
        (Ok(_), Err(_)) => Some(Difference {
            path: label.to_owned(),
            offset: 0,
            detail: "only in the first".into(),
        }),
        (Err(_), Ok(_)) => Some(Difference {
            path: label.to_owned(),
            offset: 0,
            detail: "only in the second".into(),
        }),
        (Err(_), Err(_)) => Some(Difference {
            path: label.to_owned(),
            offset: 0,
            detail: "written by neither".into(),
        }),
    }
}

fn is_text_output(name: &str) -> bool {
    name.ends_with(".json") || name.ends_with(".html")
}

/// The first forbidden string in `text`, named `path`.
fn scrub_text(path: &str, text: &[u8], forbidden: &[String]) -> Option<Difference> {
    forbidden
        .iter()
        .filter(|n| !n.is_empty())
        .find_map(|needle| {
            find(text, needle.as_bytes()).map(|offset| Difference {
                path: path.to_owned(),
                offset: offset as u64,
                detail: format!("contains {needle}"),
            })
        })
}

/// The JSON and HTML entries of a zip, scrubbed; an entry is named `<zip>!<entry>`.
fn scrub_zip(zip: &Path, name: &str, forbidden: &[String]) -> Result<Option<Difference>> {
    use slidock_io::FileReader;
    use slidock_io::zip::{ZipArchive, ZipLimits};
    let unreadable =
        |error: &dyn std::fmt::Display| BenchError::failed(format!("{}: {error}", zip.display()));
    let reader = FileReader::open(zip).map_err(BenchError::io(zip))?;
    let limits = ZipLimits {
        entry_bytes: 4 << 30,
        memory_bytes: u64::MAX,
        ..ZipLimits::default()
    };
    let archive = ZipArchive::open(reader, limits).map_err(|e| unreadable(&e))?;
    for entry in archive.entries() {
        if !is_text_output(&entry.name) {
            continue;
        }
        let text = archive.read(entry).map_err(|e| unreadable(&e))?;
        if let Some(found) = scrub_text(&format!("{name}!{}", entry.name), &text, forbidden) {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// The first JSON or HTML output under `dir`, or inside a zip there, that holds a forbidden string.
pub fn scrub(dir: &Path, forbidden: &[String]) -> Result<Option<Difference>> {
    for file in files_under(dir)? {
        let path = dir.join(&file);
        let found = if is_text_output(&file) {
            let text = std::fs::read(&path).map_err(BenchError::io(&path))?;
            scrub_text(&file, &text, forbidden)
        } else if file.ends_with(".zip") {
            scrub_zip(&path, &file, forbidden)?
        } else {
            None
        };
        if found.is_some() {
            return Ok(found);
        }
    }
    Ok(None)
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// A leg's process environment.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegEnv {
    /// `TZ=UTC LANG=C LC_ALL=C`.
    Plain,
    /// `TZ=Pacific/Chatham LANG=LC_ALL=tr_TR.UTF-8`.
    Shifted,
}

impl LegEnv {
    fn vars(self) -> BTreeMap<String, String> {
        let (tz, lang) = match self {
            LegEnv::Plain => ("UTC", "C"),
            LegEnv::Shifted => ("Pacific/Chatham", "tr_TR.UTF-8"),
        };
        BTreeMap::from([
            ("TZ".to_owned(), tz.to_owned()),
            ("LANG".to_owned(), lang.to_owned()),
            ("LC_ALL".to_owned(), lang.to_owned()),
        ])
    }
}

/// How a leg's compile ended.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LegRun {
    pub code: Option<i32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
}

async fn run(program: &Path, arguments: &[String], env: LegEnv) -> Result<LegRun> {
    let options = ExecOptions {
        env: env.vars(),
        timeout: Some(COMPILE_TIMEOUT),
        quiet: false,
        rss_limit_bytes: Some(RSS_LIMIT_BYTES),
        max_output_bytes: Some(16 * 1024 * 1024),
        ..ExecOptions::default()
    };
    let result = exec(program.as_os_str(), &args(arguments), &options).await?;
    let problem = if result.timed_out {
        Some("timeout".to_owned())
    } else if result.out_of_memory {
        Some("memory".to_owned())
    } else if !matches!(result.code, Some(0 | 1)) {
        Some(result.stderr.lines().last().unwrap_or("crashed").to_owned())
    } else {
        None
    };
    Ok(LegRun {
        code: result.code,
        problem,
    })
}

/// A native leg of one pair: the document set with report and IR in `<dir>/documents`, the deck in `<dir>/deck.zip`.
pub async fn native_leg(
    legs: &Legs<'_>,
    input: &Path,
    frontend: &str,
    dir: &Path,
    verify: &str,
    env: LegEnv,
    only: Option<&str>,
) -> Result<Vec<LegRun>> {
    let binary = legs.binary;
    remove(dir)?;
    std::fs::create_dir_all(dir).map_err(BenchError::io(dir))?;
    let input = input.to_string_lossy().into_owned();
    let fonts = legs.fonts.to_string_lossy().into_owned();
    let documents = dir.join("documents").to_string_lossy().into_owned();
    let mut emit: Vec<String> = vec![
        "compile".into(),
        input.clone(),
        "--frontend".into(),
        frontend.into(),
        "--verify".into(),
        verify.into(),
        "--fonts".into(),
        fonts.clone(),
        "--out".into(),
        documents,
        "--emit".into(),
    ];
    match only {
        Some(slide) => {
            emit.push("documents".into());
            emit.push("--only".into());
            emit.push(slide.into());
            Ok(vec![run(binary, &emit, env).await?])
        }
        None => {
            emit.push("documents,report,ir".into());
            let documents = run(binary, &emit, env).await?;
            let zip: Vec<String> = vec![
                "compile".into(),
                input,
                "--frontend".into(),
                frontend.into(),
                "--verify".into(),
                verify.into(),
                "--fonts".into(),
                fonts,
                "--fonts-off".into(),
                "-o".into(),
                dir.join("deck.zip").to_string_lossy().into_owned(),
            ];
            let deck = run(binary, &zip, env).await?;
            Ok(vec![documents, deck])
        }
    }
}

/// The wasm leg of one pair, through `bench/wasm-cli.mjs` under Node, with the font folder's files added.
pub async fn wasm_leg(
    legs: &Legs<'_>,
    input: &Path,
    name: &str,
    dir: &Path,
    env: LegEnv,
) -> Result<LegRun> {
    remove(dir)?;
    std::fs::create_dir_all(dir).map_err(BenchError::io(dir))?;
    let arguments: Vec<String> = vec![
        legs.script.to_string_lossy().into_owned(),
        legs.pkg.to_string_lossy().into_owned(),
        input.to_string_lossy().into_owned(),
        name.to_owned(),
        dir.join("documents").to_string_lossy().into_owned(),
        dir.join("deck.zip").to_string_lossy().into_owned(),
        legs.fonts.to_string_lossy().into_owned(),
    ];
    let options = ExecOptions {
        env: env.vars(),
        timeout: Some(COMPILE_TIMEOUT),
        max_output_bytes: Some(16 * 1024 * 1024),
        ..ExecOptions::default()
    };
    let result = exec("node", &args(arguments), &options).await?;
    #[derive(serde::Deserialize)]
    struct Reply {
        stage: Option<String>,
        error: Option<String>,
    }
    let reply: Option<Reply> = serde_json::from_slice(&result.stdout).ok();
    let problem = match (&reply, result.code) {
        (_, None) if result.timed_out => Some("timeout".to_owned()),
        (
            Some(Reply {
                stage: Some(stage),
                error,
            }),
            _,
        ) => Some(format!("{stage}: {}", error.as_deref().unwrap_or("failed"))),
        (Some(_), Some(0)) => None,
        _ => Some(
            result
                .stderr
                .lines()
                .last()
                .unwrap_or("node failed")
                .to_owned(),
        ),
    };
    Ok(LegRun {
        code: result.code,
        problem,
    })
}

/// How the wasm leg of a pair went.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WasmStatus {
    Compared,
    /// The input is larger than [`WASM_MAX_INPUT_BYTES`].
    NativeOnly,
}

/// One `--only` row: its slide and layout files against the whole compile's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct OnlyRow {
    pub row: RowId,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub difference: Option<Difference>,
}

/// One pair's legs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PairReport {
    pub native: Vec<LegRun>,
    pub reverse: Vec<LegRun>,
    pub wasm: WasmStatus,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasm_run: Option<LegRun>,
    /// A′ against A: documents, report, IR and zip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub native_difference: Option<Difference>,
    /// B against A: documents and zip.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub wasm_difference: Option<Difference>,
    pub only: Vec<OnlyRow>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scrub: Option<Difference>,
}

impl PairReport {
    pub fn passed(&self) -> bool {
        let ran = |runs: &[LegRun]| runs.iter().all(|r| r.problem.is_none());
        ran(&self.native)
            && ran(&self.reverse)
            && self.wasm_run.as_ref().is_none_or(|r| r.problem.is_none())
            && self.native_difference.is_none()
            && self.wasm_difference.is_none()
            && self.only.iter().all(|r| r.difference.is_none())
            && self.scrub.is_none()
    }
}

/// Rows scored twice.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ScoreReport {
    pub rows: u32,
    /// Rows whose record or render differs between the passes.
    pub differing: Vec<RowId>,
    /// Pairs whose scored compile (the one `run` makes) differs from leg A's.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub compile_differences: BTreeMap<String, Difference>,
}

impl ScoreReport {
    pub fn passed(&self) -> bool {
        self.differing.is_empty() && self.compile_differences.is_empty()
    }
}

/// Where the wasm module came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum WasmFrom {
    /// Built from the working copy by the gate.
    Built,
    /// `SLIDOCK_WASM_PKG`, as it was.
    Prebuilt,
}

/// The builds the legs ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Builds {
    /// sha256 of the native `slidock` binary.
    pub native: Sha256,
    /// sha256 of `slidock_wasm_bg.wasm`.
    pub wasm: Sha256,
    pub wasm_from: WasmFrom,
}

/// `slidock-determinism@1`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DeterminismReport {
    pub schema: &'static str,
    /// The compiler content id (the working copy's, with the bench).
    pub compiler: Sha256,
    pub builds: Builds,
    pub pairs: BTreeMap<String, PairReport>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<ScoreReport>,
    pub passed: bool,
}

pub const REPORT_SCHEMA: &str = "slidock-determinism@1";

/// The three rows the `--only` check compiles: the first, the middle and the last.
pub fn sample_rows(rows: &[RowId]) -> Vec<RowId> {
    let mut picked: Vec<RowId> = Vec::new();
    if let (Some(first), Some(last)) = (rows.first(), rows.last()) {
        for row in [first, rows.get(rows.len() / 2).unwrap_or(first), last] {
            if !picked.contains(row) {
                picked.push(row.clone());
            }
        }
    }
    picked
}

/// The `--only` compile's slide and layout files against the whole compile's.
pub fn compare_only(whole: &Path, only: &Path, slide: &str) -> Result<Option<Difference>> {
    let files = read_compiled(whole)?;
    let manifest = manifest_of(&files)?;
    let entry = manifest.slides.iter().find(|s| s.id == slide);
    let Some(entry) = entry else {
        return Ok(Some(Difference {
            path: slide_path(slide),
            offset: 0,
            detail: "the whole compile has no such slide".into(),
        }));
    };
    let mut paths = vec![slide_path(slide)];
    if let Some(layout) = &entry.layout {
        paths.push(layout_path(layout));
    }
    for path in paths {
        if let Some(difference) = compare_files(&whole.join(&path), &only.join(&path), &path) {
            return Ok(Some(difference));
        }
    }
    Ok(None)
}

/// What a determinism run needs.
pub struct Legs<'a> {
    pub binary: &'a Path,
    pub script: &'a Path,
    pub pkg: &'a Path,
    /// The font folder every leg compiles against (`run`'s).
    pub fonts: &'a Path,
    pub work: &'a Path,
    pub forbidden: &'a [String],
}

/// Runs `body` in a fresh `work` folder and removes the folder when it ends, whether it succeeds or fails.
pub async fn in_work_dir<T, F>(work: &Path, body: F) -> Result<T>
where
    F: std::future::Future<Output = Result<T>>,
{
    remove(work)?;
    std::fs::create_dir_all(work).map_err(BenchError::io(work))?;
    let outcome = body.await;
    let removed = remove(work);
    let value = outcome?;
    removed?;
    Ok(value)
}

/// The first difference between a pair's scored compile (`compiled/<p>.<fe>`, `reports/<p>.<fe>.json` and
/// `ir/<p>.<fe>.ir.json` of a run folder) and leg A's document set, which holds its report and IR inside.
pub fn compare_scored_compile(
    leg: &Path,
    run_dir: &Path,
    presentation: &str,
    frontend: &str,
) -> Result<Option<Difference>> {
    let paths = crate::compile::pair_paths(run_dir, presentation, frontend);
    let documents = leg.join("documents");
    Ok(compare_trees(
        &documents,
        &paths.compiled,
        "documents",
        &["report.json", "presentation-ir.json"],
    )?
    .or_else(|| compare_files(&documents.join("report.json"), &paths.report, "report.json"))
    .or_else(|| {
        compare_files(
            &documents.join("presentation-ir.json"),
            &paths.ir,
            "presentation-ir.json",
        )
    }))
}

/// The paths the scrub forbids: the cache, the corpus, the home folder, temporary and user folders.
pub fn forbidden_strings(cache: &Path, corpus: &Path) -> Vec<String> {
    let mut out = vec![
        cache.to_string_lossy().into_owned(),
        corpus.to_string_lossy().into_owned(),
    ];
    if let Ok(home) = std::env::var("HOME")
        && !home.is_empty()
    {
        out.push(home);
    }
    out.extend(["/tmp/", "/private/", "/Users/"].map(str::to_owned));
    out
}

pub fn pair_dir(work: &Path, leg: &str, presentation: &Presentation, source: &Source) -> PathBuf {
    work.join(leg)
        .join(format!("{}.{}", presentation.id, source.frontend))
}

/// Runs the legs of every pair (`files` gives each pair's corpus file and its rows) and compares them.
pub async fn run_legs(
    legs: &Legs<'_>,
    pairs: &[(&Presentation, &Source, PathBuf, Vec<RowId>)],
    log: fn(&str),
) -> Result<BTreeMap<String, PairReport>> {
    let mut reports: BTreeMap<String, PairReport> = BTreeMap::new();
    let mut native: BTreeMap<String, Vec<LegRun>> = BTreeMap::new();
    for (presentation, source, file, _) in pairs {
        let key = format!("{}:{}", presentation.id, source.frontend);
        let dir = pair_dir(legs.work, "a", presentation, source);
        log(&format!("{key}: native, manifest order"));
        let runs = native_leg(
            legs,
            file,
            source.frontend.as_str(),
            &dir,
            "boundaries",
            LegEnv::Plain,
            None,
        )
        .await?;
        native.insert(key, runs);
    }
    for (presentation, source, file, rows) in pairs.iter().rev() {
        let key = format!("{}:{}", presentation.id, source.frontend);
        let a = pair_dir(legs.work, "a", presentation, source);
        let reverse_dir = pair_dir(legs.work, "a-reverse", presentation, source);
        log(&format!("{key}: native, reverse order, --verify each"));
        let reverse = native_leg(
            legs,
            file,
            source.frontend.as_str(),
            &reverse_dir,
            "each",
            LegEnv::Shifted,
            None,
        )
        .await?;
        let native_difference = compare_trees(&a, &reverse_dir, "native", &[])?;
        let size = std::fs::metadata(file).map_err(BenchError::io(file))?.len();
        let b = pair_dir(legs.work, "b", presentation, source);
        let (wasm, wasm_run, wasm_difference) = if size > WASM_MAX_INPUT_BYTES {
            (WasmStatus::NativeOnly, None, None)
        } else {
            log(&format!("{key}: wasm under Node, reverse order"));
            let name = file
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let run = wasm_leg(legs, file, &name, &b, LegEnv::Shifted).await?;
            let difference = compare_trees(
                &a.join("documents"),
                &b.join("documents"),
                "documents",
                &["report.json", "presentation-ir.json"],
            )?
            .or_else(|| compare_files(&a.join("deck.zip"), &b.join("deck.zip"), "deck.zip"));
            (WasmStatus::Compared, Some(run), difference)
        };
        let mut only = Vec::new();
        for row in sample_rows(rows) {
            let slide = row.slide().as_str().to_owned();
            let dir = legs
                .work
                .join("only")
                .join(format!("{}.{}", presentation.id, source.frontend))
                .join(crate::results::row_file_name(&row));
            let runs = native_leg(
                legs,
                file,
                source.frontend.as_str(),
                &dir,
                "boundaries",
                LegEnv::Plain,
                Some(&slide),
            )
            .await?;
            let difference = match runs.iter().find_map(|r| r.problem.clone()) {
                Some(problem) => Some(Difference {
                    path: slide_path(&slide),
                    offset: 0,
                    detail: format!("--only compile failed: {problem}"),
                }),
                None => compare_only(&a.join("documents"), &dir.join("documents"), &slide)?,
            };
            only.push(OnlyRow { row, difference });
        }
        let mut scrubbed = None;
        for dir in [&a, &reverse_dir, &b] {
            if scrubbed.is_none() {
                scrubbed = scrub(dir, legs.forbidden)?;
            }
        }
        let key = format!("{}:{}", presentation.id, source.frontend);
        let report = PairReport {
            native: native.remove(&key).unwrap_or_default(),
            reverse,
            wasm,
            wasm_run,
            native_difference,
            wasm_difference,
            only,
            scrub: scrubbed,
        };
        log(&format!(
            "{key}: {}",
            if report.passed() {
                "deterministic"
            } else {
                "DIFFERS"
            }
        ));
        reports.insert(key, report);
    }
    Ok(reports)
}

/// Writes the report to `<dir>/<sha256>.json` and returns its hash.
pub fn write_report(dir: &Path, report: &DeterminismReport) -> Result<(Sha256, PathBuf)> {
    let text = slidock_bench_schema::to_canonical(report)?;
    let digest = crate::hash::sha256_json(report)?;
    let sha = Sha256::new(digest)?;
    let path = dir.join(format!("{}.json", sha.as_str()));
    write_atomic(&path, text)?;
    Ok((sha, path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_first_differing_byte_or_the_shorter_length() {
        assert_eq!(first_byte_difference(b"abc", b"abc"), None);
        assert_eq!(first_byte_difference(b"abc", b"abd"), Some(2));
        assert_eq!(first_byte_difference(b"ab", b"abc"), Some(2));
    }

    #[test]
    fn trees_compare_by_file_list_then_bytes() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        for dir in [a.path(), b.path()] {
            std::fs::create_dir_all(dir.join("slides")).unwrap();
            std::fs::write(dir.join("slides/s.html"), "<p>same</p>").unwrap();
            std::fs::write(dir.join("report.json"), "{}").unwrap();
        }
        assert_eq!(compare_trees(a.path(), b.path(), "x", &[]).unwrap(), None);
        std::fs::write(b.path().join("slides/s.html"), "<p>sane</p>").unwrap();
        let difference = compare_trees(a.path(), b.path(), "x", &[])
            .unwrap()
            .unwrap();
        assert_eq!(difference.path, "x/slides/s.html");
        assert_eq!(difference.offset, 5);
        std::fs::remove_file(b.path().join("report.json")).unwrap();
        std::fs::write(b.path().join("slides/s.html"), "<p>same</p>").unwrap();
        let only = compare_trees(a.path(), b.path(), "x", &[])
            .unwrap()
            .unwrap();
        assert_eq!(only.detail, "only in the first");
        assert_eq!(
            compare_trees(a.path(), b.path(), "x", &["report.json"]).unwrap(),
            None
        );
    }

    #[test]
    fn the_scrub_finds_forbidden_paths_in_text_outputs_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.html"), "<p>fine</p>").unwrap();
        std::fs::write(dir.path().join("b.png"), "/Users/someone").unwrap();
        let forbidden = vec!["/Users/".to_owned(), String::new()];
        assert_eq!(scrub(dir.path(), &forbidden).unwrap(), None);
        std::fs::write(dir.path().join("c.json"), "{\"p\":\"/Users/me/x\"}").unwrap();
        let found = scrub(dir.path(), &forbidden).unwrap().unwrap();
        assert_eq!((found.path.as_str(), found.offset), ("c.json", 6));
        std::fs::remove_file(dir.path().join("c.json")).unwrap();
        std::fs::write(dir.path().join("d.html"), "<img src=\"/Users/me/a.png\">").unwrap();
        assert_eq!(
            scrub(dir.path(), &forbidden).unwrap().map(|d| d.path),
            Some("d.html".to_owned())
        );
    }

    #[test]
    fn the_scrub_reads_the_text_entries_of_zips() {
        let dir = tempfile::tempdir().unwrap();
        let zip = |entries: &[(&str, &str)]| {
            let mut bytes = std::io::Cursor::new(Vec::new());
            let mut writer = ::zip::ZipWriter::new(&mut bytes);
            for (name, text) in entries {
                writer
                    .start_file(*name, ::zip::write::SimpleFileOptions::default())
                    .unwrap();
                std::io::Write::write_all(&mut writer, text.as_bytes()).unwrap();
            }
            writer.finish().unwrap();
            bytes.into_inner()
        };
        let forbidden = vec!["/Users/".to_owned()];
        std::fs::write(
            dir.path().join("deck.zip"),
            zip(&[("index.html", "<p>fine</p>"), ("media/a.png", "/Users/x")]),
        )
        .unwrap();
        assert_eq!(scrub(dir.path(), &forbidden).unwrap(), None);
        std::fs::write(
            dir.path().join("deck.zip"),
            zip(&[("index.html", "<img src=\"/Users/me/a.png\">")]),
        )
        .unwrap();
        let found = scrub(dir.path(), &forbidden).unwrap().unwrap();
        assert_eq!(
            (found.path.as_str(), found.offset),
            ("deck.zip!index.html", 10)
        );
    }

    #[tokio::test]
    async fn the_work_folder_goes_however_the_gate_ends() {
        let root = tempfile::tempdir().unwrap();
        let work = root.path().join("work");
        std::fs::create_dir_all(work.join("stale")).unwrap();
        let ok = in_work_dir(&work, async {
            assert!(!work.join("stale").exists());
            std::fs::write(work.join("a.zip"), "x").map_err(BenchError::io(&work))?;
            Ok(7)
        })
        .await;
        assert_eq!(ok.unwrap(), 7);
        assert!(!work.exists());
        let failed: Result<()> = in_work_dir(&work, async {
            std::fs::write(work.join("a.zip"), "x").map_err(BenchError::io(&work))?;
            Err(BenchError::failed("a leg crashed"))
        })
        .await;
        assert!(failed.is_err());
        assert!(!work.exists());
    }

    #[test]
    fn the_scored_compile_is_held_to_leg_a() {
        let root = tempfile::tempdir().unwrap();
        let leg = root.path().join("a");
        let run = root.path().join("run");
        let documents = leg.join("documents");
        std::fs::create_dir_all(documents.join("slides")).unwrap();
        std::fs::write(documents.join("slides/s.html"), "<p>s</p>").unwrap();
        std::fs::write(documents.join("report.json"), "{}").unwrap();
        std::fs::write(documents.join("presentation-ir.json"), "{\"a\":1}").unwrap();
        let paths = crate::compile::pair_paths(&run, "p", "pptx");
        std::fs::create_dir_all(paths.compiled.join("slides")).unwrap();
        std::fs::create_dir_all(paths.ir.parent().unwrap()).unwrap();
        std::fs::create_dir_all(paths.report.parent().unwrap()).unwrap();
        std::fs::write(paths.compiled.join("slides/s.html"), "<p>s</p>").unwrap();
        std::fs::write(&paths.report, "{}").unwrap();
        std::fs::write(&paths.ir, "{\"a\":1}").unwrap();
        assert_eq!(
            compare_scored_compile(&leg, &run, "p", "pptx").unwrap(),
            None
        );
        std::fs::write(&paths.ir, "{\"a\":2}").unwrap();
        let found = compare_scored_compile(&leg, &run, "p", "pptx")
            .unwrap()
            .unwrap();
        assert_eq!(
            (found.path.as_str(), found.offset),
            ("presentation-ir.json", 5)
        );
    }

    #[test]
    fn three_sample_rows_first_middle_last() {
        let ids: Vec<RowId> = (1..=5)
            .map(|n| format!("p:s{n}:pptx").parse().unwrap())
            .collect();
        let picked: Vec<String> = sample_rows(&ids)
            .iter()
            .map(|r| r.slide().as_str().to_owned())
            .collect();
        assert_eq!(picked, ["s1", "s3", "s5"]);
        assert_eq!(sample_rows(&ids[..1]).len(), 1);
        assert!(sample_rows(&[]).is_empty());
    }
}
