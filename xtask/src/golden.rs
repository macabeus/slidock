//! `cargo xtask golden`: slidock's golden files, its own outputs frozen privately under
//! `$SLIDOCK_PRIVATE/golden/<commit>/` and pinned by SHA-256 in `tests/golden/manifest.json` (`tests/golden/support.rs`
//! documents the layout the suites read).
//!
//! `--record --reason <text>` builds the release CLI at a clean HEAD, compiles every corpus pair of the manifest with
//! the bench's static-lane arguments and every IR fixture of `$SLIDOCK_PRIVATE/golden/fixtures/ir/`, runs each crate's
//! `golden` suites so they record their own outputs (the facade's and the CLI's last: they compare with what the others
//! record), scores the bench metrics' corpus goldens again (`$SLIDOCK_PRIVATE/golden/scoring/`, whose `scores.json`
//! files, index and committed pin slidock-bench-metrics' suite rewrites), and writes the manifest: the commit, the
//! reason, the pairs, every input (sources, the bench's font set, the fixtures, the Keynote motion expectations, the
//! scoring goldens) and every output, each by SHA-256.
//!
//! `--check` verifies every pinned file, compiles the pairs and fixtures again with the CLI built at the working tree
//! and compares the files byte for byte, then runs every crate's `golden` suites. Both skip, saying so, when the
//! recording the manifest names is absent.
//!
//! `--pin --reason <text>` pins the files the expectation folders (the Keynote motion expectations, the scoring
//! goldens) gained since the recording, keeping its commit, outputs and pins, and adds the reason to the manifest's; a
//! pinned input that differs or is gone, or a new file in a folder that feeds the outputs (the font set, the
//! fixtures), refuses it (that takes a new recording).
//!
//! A crate whose Cargo.toml says `release = true` under `[package.metadata.golden]` has its suites run in release:
//! those that take over a minute in a debug build (the metrics' scoring, the Keynote frontend's, the facade's and the
//! CLI's).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

const USAGE: &str =
    "usage: cargo xtask golden (--record --reason <text> | --check | --pin --reason <text>)";

/// The manifest, relative to the repository.
const MANIFEST: &str = "tests/golden/manifest.json";
/// The bench's font set, relative to `$SLIDOCK_PRIVATE`.
const FONTS: &str = "bench/cache/fonts";
/// The corpus manifest, relative to `$SLIDOCK_PRIVATE`.
const CORPUS_MANIFEST: &str = "bench/corpus/manifest.json";
/// The private fixtures, relative to `$SLIDOCK_PRIVATE`.
const FIXTURES: &str = "golden/fixtures";
/// The Keynote motion expectations slidock-keynote's golden suite reads, relative to `$SLIDOCK_PRIVATE`.
const EXPECTATIONS: &str = "expect/keynote-motion";
/// The bench metrics' corpus goldens (`rows/<row>/`: the scoring inputs and `scores.json`, and `index.json`), relative
/// to `$SLIDOCK_PRIVATE`.
const SCORING: &str = "golden/scoring";
/// The crate whose golden suite scores [`SCORING`], and records it again with `UPDATE_GOLDEN` and `GOLDEN_REASON`.
const SCORING_CRATE: &str = "slidock-bench-metrics";
/// The private folders pinned whole: every file in each, and no other.
const PINNED_DIRS: [&str; 4] = [FONTS, FIXTURES, EXPECTATIONS, SCORING];
/// The pinned folders whose files feed the outputs the goldens hold, so that a file they gain takes a new recording;
/// the others hold expectations the suites compare with.
const FEEDING_DIRS: [&str; 2] = [FONTS, FIXTURES];
/// What an IR fixture's file name ends with.
const IR_SUFFIX: &str = ".presentation-ir.json";
/// Crates whose names start so read the goldens; they record nothing of their own.
const READERS: &str = "slidock-bench";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

fn private_dir() -> PathBuf {
    match std::env::var_os("SLIDOCK_PRIVATE") {
        Some(dir) => PathBuf::from(dir),
        None => PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".slidock"),
    }
}

fn target_dir() -> PathBuf {
    match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => repo_root().join("target"),
    }
}

/// The manifest's contents.
#[derive(Clone, Debug, Default, PartialEq)]
struct Manifest {
    commit: String,
    reason: String,
    /// `<presentation>.<frontend>` → its source: `corpus:<presentation>.<frontend>`, or `repo:<path>`.
    pairs: BTreeMap<String, String>,
    inputs: BTreeMap<String, String>,
    outputs: BTreeMap<String, String>,
}

pub fn run(args: &[String]) -> Result<(), String> {
    match args {
        [flag] if flag == "--check" => check(),
        [flag, reason_flag, reason] if flag == "--record" && reason_flag == "--reason" => {
            record(reason)
        }
        [flag, reason_flag, reason] if flag == "--pin" && reason_flag == "--reason" => pin(reason),
        _ => Err(USAGE.to_owned()),
    }
}

fn read_manifest() -> Result<Manifest, String> {
    let path = repo_root().join(MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest = parse_manifest(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if print_manifest(&manifest) != text {
        return Err(format!(
            "{}: not in the layout `cargo xtask golden --record` writes",
            path.display()
        ));
    }
    Ok(manifest)
}

fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(hex(&Sha256::digest(&bytes)))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Every file under `dir`, by `/`-separated path relative to it.
fn files_under(dir: &Path) -> Result<Vec<String>, String> {
    let mut files = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        let entries = std::fs::read_dir(&next).map_err(|e| format!("{}: {e}", next.display()))?;
        for entry in entries {
            let path = entry.map_err(|e| e.to_string())?.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            let relative = path
                .strip_prefix(dir)
                .map_err(|e| e.to_string())?
                .to_str()
                .ok_or_else(|| format!("{}: not UTF-8", path.display()))?;
            files.push(relative.replace('\\', "/"));
        }
    }
    files.sort();
    Ok(files)
}

/// The file a pinned input names: `repo:<path>` in the repository, `corpus:<presentation>.<frontend>` in the corpus
/// ([`corpus_source`]), any other path under `private`.
fn input_path(private: &Path, input: &str) -> Result<PathBuf, String> {
    if let Some(rest) = input.strip_prefix("repo:") {
        Ok(repo_root().join(rest))
    } else if let Some(source) = input.strip_prefix("corpus:") {
        corpus_source(private, source)
    } else {
        Ok(private.join(input))
    }
}

/// The file of a corpus source, `<presentation>.<frontend>`: the corpus manifest's presentation of that id names the
/// source file of that frontend, under `private/corpus/`. The repository names corpus decks by id only.
fn corpus_source(private: &Path, source: &str) -> Result<PathBuf, String> {
    let path = private.join(CORPUS_MANIFEST);
    let text = std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
    let manifest: serde_json::Value =
        serde_json::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    let (id, frontend) = source
        .rsplit_once('.')
        .ok_or_else(|| format!("corpus:{source}: a source is <presentation>.<frontend>"))?;
    let presentation = manifest["presentations"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|presentation| presentation["id"] == id)
        .ok_or_else(|| format!("{}: no presentation {id}", path.display()))?;
    let file = presentation["sources"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|source| source["frontend"] == frontend)
        .and_then(|source| source["file"].as_str())
        .ok_or_else(|| format!("{}: {id} has no {frontend} source", path.display()))?;
    Ok(private.join("corpus").join(file))
}

/// What a pinned input can be against the files on disk.
#[derive(Debug, PartialEq)]
enum InputProblem {
    /// Its bytes differ from its pin.
    Differs(String),
    /// It is pinned, but gone.
    Gone(String),
    /// A file in a pinned folder that is not pinned, with its SHA-256.
    Unpinned(String, String),
}

impl std::fmt::Display for InputProblem {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InputProblem::Differs(input) => write!(f, "input {input}: differs from its pin"),
            InputProblem::Gone(input) => write!(f, "input {input}: pinned, but gone"),
            InputProblem::Unpinned(input, _) => write!(f, "input {input}: not pinned"),
        }
    }
}

/// Every pinned input checked against `private` (and the repository), and every file of the pinned folders that is
/// not pinned.
fn input_problems(manifest: &Manifest, private: &Path) -> Result<Vec<InputProblem>, String> {
    let mut problems = Vec::new();
    for (input, sha) in &manifest.inputs {
        let path = input_path(private, input)?;
        if !path.is_file() {
            problems.push(InputProblem::Gone(input.clone()));
        } else if &sha256_file(&path)? != sha {
            problems.push(InputProblem::Differs(input.clone()));
        }
    }
    for dir in PINNED_DIRS {
        let folder = private.join(dir);
        if !folder.is_dir() {
            continue;
        }
        for file in files_under(&folder)? {
            let input = format!("{dir}/{file}");
            if !manifest.inputs.contains_key(&input) {
                let sha = sha256_file(&folder.join(&file))?;
                problems.push(InputProblem::Unpinned(input, sha));
            }
        }
    }
    Ok(problems)
}

/// The scoring goldens the manifest pins: their files and their rows.
fn scoring_counts(manifest: &Manifest) -> (usize, usize) {
    let prefix = format!("{SCORING}/");
    let files: Vec<&str> = manifest
        .inputs
        .keys()
        .filter_map(|input| input.strip_prefix(&prefix))
        .collect();
    let rows: std::collections::BTreeSet<&str> = files
        .iter()
        .filter_map(|file| file.strip_prefix("rows/")?.split_once('/'))
        .map(|(row, _)| row)
        .collect();
    (files.len(), rows.len())
}

fn git(args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .args(args)
        .current_dir(repo_root())
        .output()
        .map_err(|e| format!("git: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

fn run_tool(command: &mut Command, what: &str) -> Result<(), String> {
    let output = command.output().map_err(|e| format!("{what}: {e}"))?;
    if output.status.success() {
        return Ok(());
    }
    Err(format!(
        "{what} exited {:?}:\n{}{}",
        output.status.code(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    ))
}

fn cargo() -> Command {
    let mut command = Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    command.current_dir(repo_root());
    command
}

/// The release CLI built from the working tree.
fn build_cli() -> Result<PathBuf, String> {
    eprintln!("building the release CLI");
    run_tool(
        cargo().args(["build", "--release", "--locked", "-p", "slidock-cli"]),
        "cargo build --release -p slidock-cli",
    )?;
    Ok(target_dir().join("release/slidock"))
}

/// One compile with fixed locale and time zone.
fn compile(cli: &Path, input: &Path, out: &Path, args: &[&str]) -> Result<(), String> {
    let mut command = Command::new(cli);
    command
        .arg("compile")
        .arg(input)
        .arg("--out")
        .arg(out)
        .args(args)
        .env("TZ", "UTC")
        .env("LANG", "C")
        .env("LC_ALL", "C");
    run_tool(
        &mut command,
        &format!("slidock compile {}", input.display()),
    )
}

/// The frontend a pair names: the text after its last `.`.
fn frontend(pair: &str) -> Result<&str, String> {
    pair.rsplit_once('.')
        .map(|(_, frontend)| frontend)
        .ok_or_else(|| format!("{pair}: a pair is <presentation>.<frontend>"))
}

/// The IR fixtures' names.
fn ir_fixtures() -> Result<Vec<String>, String> {
    let dir = private_dir().join(FIXTURES).join("ir");
    let entries = std::fs::read_dir(&dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut names = Vec::new();
    for entry in entries {
        let name = entry.map_err(|e| e.to_string())?.file_name();
        if let Some(stem) = name.to_str().and_then(|n| n.strip_suffix(IR_SUFFIX)) {
            names.push(stem.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// Compiles every pair and IR fixture into `out`: `corpus/<pair>/` and `fixtures/<name>/`.
fn compile_all(cli: &Path, manifest: &Manifest, out: &Path) -> Result<usize, String> {
    let fonts = private_dir().join(FONTS);
    let fonts = fonts
        .to_str()
        .ok_or("the font folder's path is not UTF-8")?;
    let mut count = 0;
    for (pair, source) in &manifest.pairs {
        eprintln!("compiling {pair}");
        let args = [
            "--frontend",
            frontend(pair)?,
            "--emit",
            "documents,report,ir",
            "--fonts",
            fonts,
        ];
        compile(
            cli,
            &input_path(&private_dir(), source)?,
            &out.join("corpus").join(pair),
            &args,
        )?;
        count += 1;
    }
    for name in ir_fixtures()? {
        let input = private_dir()
            .join(FIXTURES)
            .join("ir")
            .join(format!("{name}{IR_SUFFIX}"));
        let args = ["--emit", "documents,ir,report"];
        compile(cli, &input, &out.join("fixtures").join(&name), &args)?;
        count += 1;
    }
    Ok(count)
}

/// Crates whose suites compare with what other crates' suites record, so they run last.
const LAST: [&str; 2] = ["slidock", "slidock-cli"];

/// A crate with `golden` suites, and whether they run in release.
#[derive(Clone, Debug, PartialEq)]
struct Suite {
    name: String,
    release: bool,
}

/// The suite a crate's Cargo.toml describes, if it has a `golden` feature: `release = true` under
/// `[package.metadata.golden]` runs it in release, and any other key or value there is refused.
fn suite_of(toml: &str) -> Result<Option<Suite>, String> {
    let mut name = None;
    let mut golden = false;
    let mut release = false;
    let mut section = "";
    for line in toml.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            section = line;
            continue;
        }
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        match section {
            "[package]" => {
                if let Some(rest) = line.strip_prefix("name = \"") {
                    name = rest.strip_suffix('"').map(str::to_owned);
                }
            }
            "[features]" => golden |= line.starts_with("golden = "),
            "[package.metadata.golden]" => match line {
                "release = true" => release = true,
                "release = false" => release = false,
                _ => {
                    return Err(format!(
                        "[package.metadata.golden]: {line:?} is not `release = true|false`"
                    ));
                }
            },
            _ => {}
        }
    }
    if !golden {
        return Ok(None);
    }
    let name = name.ok_or("no package name")?;
    Ok(Some(Suite { name, release }))
}

/// The crates with a `golden` feature, by package name; [`LAST`] after the rest.
fn golden_crates() -> Result<Vec<Suite>, String> {
    let crates = repo_root().join("crates");
    let entries = std::fs::read_dir(&crates).map_err(|e| format!("{}: {e}", crates.display()))?;
    let mut suites = Vec::new();
    for entry in entries {
        let toml = entry.map_err(|e| e.to_string())?.path().join("Cargo.toml");
        let Ok(text) = std::fs::read_to_string(&toml) else {
            continue;
        };
        if let Some(suite) = suite_of(&text).map_err(|e| format!("{}: {e}", toml.display()))? {
            suites.push(suite);
        }
    }
    suites.sort_by_key(|suite| (LAST.contains(&suite.name.as_str()), suite.name.clone()));
    Ok(suites)
}

/// `cargo test` of a crate's `golden` suites, in release when the crate asks for it.
fn suite_command(suite: &Suite) -> Command {
    let mut command = cargo();
    command.args(["test", "--locked"]);
    if suite.release {
        command.arg("--release");
    }
    command.args(["-p", &suite.name, "--features", "golden", "--no-fail-fast"]);
    command
}

fn run_suites(suites: &[Suite], record_into: Option<&Path>) -> Result<(), String> {
    for suite in suites {
        let profile = if suite.release { " --release" } else { "" };
        eprintln!("cargo test{profile} -p {} --features golden", suite.name);
        let mut command = suite_command(suite);
        if let Some(dir) = record_into {
            command.env("SLIDOCK_GOLDEN_RECORD", dir);
        }
        run_tool(
            &mut command,
            &format!("the golden suites of {}", suite.name),
        )?;
    }
    Ok(())
}

/// Scores the bench metrics' corpus goldens again, rewriting their `scores.json` files, index and committed pin.
fn record_scoring(suites: &[Suite], reason: &str) -> Result<(), String> {
    let suite = suites
        .iter()
        .find(|suite| suite.name == SCORING_CRATE)
        .ok_or_else(|| format!("{SCORING_CRATE} has no golden suite"))?;
    eprintln!("scoring {SCORING} again ({SCORING_CRATE})");
    let mut command = suite_command(suite);
    command
        .args(["--test", "golden"])
        .env("UPDATE_GOLDEN", "1")
        .env("GOLDEN_REASON", reason);
    run_tool(
        &mut command,
        &format!("the scoring goldens of {SCORING_CRATE}"),
    )
}

/// Every file of the pinned folders with its SHA-256, and the pairs' sources.
fn pinned_inputs(
    pairs: &BTreeMap<String, String>,
    private: &Path,
) -> Result<BTreeMap<String, String>, String> {
    let mut inputs = BTreeMap::new();
    for source in pairs.values() {
        inputs.insert(source.clone(), sha256_file(&input_path(private, source)?)?);
    }
    for dir in PINNED_DIRS {
        for file in files_under(&private.join(dir))? {
            let input = format!("{dir}/{file}");
            inputs.insert(input.clone(), sha256_file(&input_path(private, &input)?)?);
        }
    }
    Ok(inputs)
}

fn record(reason: &str) -> Result<(), String> {
    if reason.trim().is_empty() {
        return Err("--reason states why the goldens are recorded".to_owned());
    }
    if !git(&["status", "--porcelain"])?.is_empty() {
        return Err(
            "the working tree has changes: the goldens are recorded at a commit".to_owned(),
        );
    }
    let commit = git(&["rev-parse", "--short=12", "HEAD"])?;
    let old = read_manifest()?;
    let golden = private_dir().join("golden");
    let out = golden.join(&commit);
    if out.exists() {
        return Err(format!(
            "{} exists: remove it to record again",
            out.display()
        ));
    }
    let partial = golden.join(format!("{commit}.partial"));
    if partial.exists() {
        std::fs::remove_dir_all(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;
    }
    std::fs::create_dir_all(&partial).map_err(|e| format!("{}: {e}", partial.display()))?;

    let cli = build_cli()?;
    let compiled = compile_all(&cli, &old, &partial)?;
    let suites = golden_crates()?;
    let crates: Vec<Suite> = suites
        .iter()
        .filter(|suite| !suite.name.starts_with(READERS))
        .cloned()
        .collect();
    run_suites(&crates, Some(&partial))?;
    record_scoring(&suites, reason.trim())?;

    let inputs = pinned_inputs(&old.pairs, &private_dir())?;
    let mut outputs = BTreeMap::new();
    let mut bytes = 0u64;
    for file in files_under(&partial)? {
        let path = partial.join(&file);
        bytes += std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
        outputs.insert(file, sha256_file(&path)?);
    }
    std::fs::rename(&partial, &out).map_err(|e| format!("{}: {e}", out.display()))?;
    let manifest = Manifest {
        commit,
        reason: reason.trim().to_owned(),
        pairs: old.pairs,
        inputs,
        outputs,
    };
    let path = repo_root().join(MANIFEST);
    std::fs::write(&path, print_manifest(&manifest))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let (scoring_files, scoring_rows) = scoring_counts(&manifest);
    println!(
        "recorded {} ({} compiles, {} crates' suites): {} outputs, {bytes} bytes, {} inputs pinned ({scoring_files} \
         scoring files of {scoring_rows} rows scored again); {} and {SCORING_CRATE}'s scoring pin rewritten",
        out.display(),
        compiled,
        crates.len(),
        manifest.outputs.len(),
        manifest.inputs.len(),
        MANIFEST
    );
    Ok(())
}

/// The manifest with the pins of every file the expectation folders gained, and `reason` added to its reason; a pinned
/// input that differs or is gone, or a new file in a folder that feeds the outputs, refuses it.
fn with_new_pins(manifest: Manifest, private: &Path, reason: &str) -> Result<Manifest, String> {
    if reason.trim().is_empty() {
        return Err("--reason states why the inputs are pinned".to_owned());
    }
    let mut manifest = manifest;
    let mut refusals = Vec::new();
    let mut added = BTreeMap::new();
    for problem in input_problems(&manifest, private)? {
        match problem {
            InputProblem::Unpinned(input, _) if feeds_outputs(&input) => {
                refusals.push(format!(
                    "input {input}: not pinned, and it feeds the outputs"
                ));
            }
            InputProblem::Unpinned(input, sha) => {
                added.insert(input, sha);
            }
            other => refusals.push(other.to_string()),
        }
    }
    if !refusals.is_empty() {
        refusals.push(
            "an input that feeds the outputs changed or is new, or a pin no longer holds: the goldens are recorded \
             again with --record"
                .to_owned(),
        );
        return Err(refusals.join("\n"));
    }
    if added.is_empty() {
        return Err("every file of the pinned folders is pinned already".to_owned());
    }
    manifest.inputs.extend(added);
    manifest.reason = format!("{} {}", manifest.reason, reason.trim());
    Ok(manifest)
}

/// Whether `input` lies in a pinned folder that feeds the outputs.
fn feeds_outputs(input: &str) -> bool {
    FEEDING_DIRS.iter().any(|dir| {
        input
            .strip_prefix(dir)
            .is_some_and(|rest| rest.starts_with('/'))
    })
}

fn pin(reason: &str) -> Result<(), String> {
    let old = read_manifest()?;
    let before = old.inputs.len();
    let manifest = with_new_pins(old, &private_dir(), reason)?;
    let path = repo_root().join(MANIFEST);
    std::fs::write(&path, print_manifest(&manifest))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let (scoring_files, scoring_rows) = scoring_counts(&manifest);
    println!(
        "pinned {} new inputs of {}; {} inputs ({scoring_files} scoring files of {scoring_rows} rows); {} rewritten",
        manifest.inputs.len() - before,
        manifest.commit,
        manifest.inputs.len(),
        MANIFEST
    );
    Ok(())
}

fn check() -> Result<(), String> {
    let manifest = read_manifest()?;
    let root = private_dir().join("golden").join(&manifest.commit);
    if manifest.commit.is_empty() || !root.is_dir() {
        println!(
            "skipped: the golden files of {:?} are absent ({}). They are slidock's outputs on the private corpus \
             and font set, which recording them needs too, so only a maintainer holding those checks them",
            manifest.commit,
            root.display()
        );
        return Ok(());
    }
    let mut problems: Vec<String> = input_problems(&manifest, &private_dir())?
        .iter()
        .map(ToString::to_string)
        .collect();
    for (output, sha) in &manifest.outputs {
        let actual = sha256_file(&root.join(output))?;
        if &actual != sha {
            problems.push(format!("golden {output}: differs from its pin"));
        }
    }
    let recorded = files_under(&root)?;
    for extra in recorded
        .iter()
        .filter(|file| !manifest.outputs.contains_key(*file))
    {
        problems.push(format!("golden {extra}: not pinned"));
    }
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    let (scoring_files, scoring_rows) = scoring_counts(&manifest);
    if scoring_files == 0 {
        return Err(format!("{MANIFEST} pins no scoring goldens ({SCORING})"));
    }
    println!(
        "{} inputs ({scoring_files} scoring files of {scoring_rows} rows) and {} goldens of {} match their pins",
        manifest.inputs.len(),
        manifest.outputs.len(),
        manifest.commit
    );

    let cli = build_cli()?;
    let work = target_dir().join("golden-check");
    if work.exists() {
        std::fs::remove_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    }
    let compiled = compile_all(&cli, &manifest, &work)?;
    let written = files_under(&work)?;
    let mut compared = 0;
    for file in &written {
        let Some(sha) = manifest.outputs.get(file) else {
            problems.push(format!("{file}: written, but not in the golden set"));
            continue;
        };
        compared += 1;
        if &sha256_file(&work.join(file))? != sha {
            problems.push(format!("{file}: differs from its golden"));
        }
    }
    for file in manifest.outputs.keys() {
        let compiled_output = file.starts_with("corpus/") || file.starts_with("fixtures/");
        if compiled_output && !written.contains(file) {
            problems.push(format!("{file}: not written"));
        }
    }
    println!(
        "{compiled} compiles, {compared} files compared, {} differ",
        problems.len()
    );
    if !problems.is_empty() {
        return Err(problems.join("\n"));
    }
    std::fs::remove_dir_all(&work).map_err(|e| format!("{}: {e}", work.display()))?;
    let crates = golden_crates()?;
    run_suites(&crates, None)?;
    println!("the golden suites of {} crates pass", crates.len());
    Ok(())
}

/// A JSON string literal.
fn quote(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if u32::from(c) < 0x20 => out.push_str(&format!("\\u{:04x}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn print_map(name: &str, map: &BTreeMap<String, String>, last: bool) -> String {
    let mut out = format!("  {}: {{", quote(name));
    let entries: Vec<String> = map
        .iter()
        .map(|(key, value)| format!("\n    {}: {}", quote(key), quote(value)))
        .collect();
    out.push_str(&entries.join(","));
    if !map.is_empty() {
        out.push_str("\n  ");
    }
    out.push('}');
    if !last {
        out.push(',');
    }
    out.push('\n');
    out
}

/// The manifest in its one layout: keys in this order, maps sorted, two-space indents.
fn print_manifest(manifest: &Manifest) -> String {
    let mut out = String::from("{\n  \"version\": 1,\n");
    out.push_str(&format!("  \"commit\": {},\n", quote(&manifest.commit)));
    out.push_str(&format!("  \"reason\": {},\n", quote(&manifest.reason)));
    out.push_str(&print_map("pairs", &manifest.pairs, false));
    out.push_str(&print_map("inputs", &manifest.inputs, false));
    out.push_str(&print_map("outputs", &manifest.outputs, true));
    out.push_str("}\n");
    out
}

/// A JSON value, as far as the manifest uses JSON: objects, strings and integers.
#[derive(Debug, PartialEq)]
enum Json {
    Object(Vec<(String, Json)>),
    String(String),
    Integer(i64),
}

struct Parser<'a> {
    text: &'a [u8],
    at: usize,
}

impl Parser<'_> {
    fn error(&self, what: &str) -> String {
        format!("{what} at byte {}", self.at)
    }

    fn skip_space(&mut self) {
        while self
            .text
            .get(self.at)
            .is_some_and(|b| b.is_ascii_whitespace())
        {
            self.at += 1;
        }
    }

    fn expect(&mut self, byte: u8) -> Result<(), String> {
        self.skip_space();
        if self.text.get(self.at) == Some(&byte) {
            self.at += 1;
            Ok(())
        } else {
            Err(self.error(&format!("expected {:?}", char::from(byte))))
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json, String> {
        self.skip_space();
        match self.text.get(self.at) {
            Some(b'{') if depth < 4 => self.object(depth + 1),
            Some(b'"') => self.string().map(Json::String),
            Some(b) if b.is_ascii_digit() => {
                let start = self.at;
                while self.text.get(self.at).is_some_and(u8::is_ascii_digit) {
                    self.at += 1;
                }
                let digits = std::str::from_utf8(self.text.get(start..self.at).unwrap_or_default())
                    .map_err(|e| e.to_string())?;
                digits
                    .parse()
                    .map(Json::Integer)
                    .map_err(|_| self.error("an integer out of range"))
            }
            _ => Err(self.error("expected an object, a string or an integer")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Json, String> {
        self.expect(b'{')?;
        let mut entries = Vec::new();
        self.skip_space();
        if self.text.get(self.at) == Some(&b'}') {
            self.at += 1;
            return Ok(Json::Object(entries));
        }
        loop {
            self.skip_space();
            let key = self.string()?;
            self.expect(b':')?;
            entries.push((key, self.value(depth)?));
            self.skip_space();
            match self.text.get(self.at) {
                Some(b',') => self.at += 1,
                Some(b'}') => {
                    self.at += 1;
                    return Ok(Json::Object(entries));
                }
                _ => return Err(self.error("expected , or }")),
            }
        }
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect(b'"')?;
        let mut out = Vec::new();
        loop {
            let byte = *self
                .text
                .get(self.at)
                .ok_or_else(|| self.error("an unterminated string"))?;
            self.at += 1;
            match byte {
                b'"' => return String::from_utf8(out).map_err(|e| e.to_string()),
                b'\\' => {
                    let escape = *self
                        .text
                        .get(self.at)
                        .ok_or_else(|| self.error("an unterminated escape"))?;
                    self.at += 1;
                    match escape {
                        b'"' | b'\\' | b'/' => out.push(escape),
                        b'n' => out.push(b'\n'),
                        b'u' => {
                            let digits = self
                                .text
                                .get(self.at..self.at + 4)
                                .and_then(|d| std::str::from_utf8(d).ok())
                                .and_then(|d| u32::from_str_radix(d, 16).ok())
                                .and_then(char::from_u32)
                                .ok_or_else(|| {
                                    self.error("a \\u escape the manifest never writes")
                                })?;
                            self.at += 4;
                            let mut buffer = [0; 4];
                            out.extend_from_slice(digits.encode_utf8(&mut buffer).as_bytes());
                        }
                        _ => return Err(self.error("an escape the manifest never writes")),
                    }
                }
                byte if byte < 0x20 => return Err(self.error("a control character in a string")),
                byte => out.push(byte),
            }
        }
    }
}

fn parse_manifest(text: &str) -> Result<Manifest, String> {
    let mut parser = Parser {
        text: text.as_bytes(),
        at: 0,
    };
    let Json::Object(entries) = parser.value(0)? else {
        return Err("the manifest is a JSON object".to_owned());
    };
    parser.skip_space();
    if parser.at != text.len() {
        return Err(parser.error("text after the manifest"));
    }
    let mut manifest = Manifest::default();
    let mut version = None;
    let map = |value: Json, key: &str| -> Result<BTreeMap<String, String>, String> {
        let Json::Object(entries) = value else {
            return Err(format!("{key} is an object"));
        };
        entries
            .into_iter()
            .map(|(k, v)| match v {
                Json::String(s) => Ok((k, s)),
                _ => Err(format!("{key}.{k} is a string")),
            })
            .collect()
    };
    for (key, value) in entries {
        match (key.as_str(), value) {
            ("version", Json::Integer(n)) => version = Some(n),
            ("commit", Json::String(s)) => manifest.commit = s,
            ("reason", Json::String(s)) => manifest.reason = s,
            ("pairs", value) => manifest.pairs = map(value, "pairs")?,
            ("inputs", value) => manifest.inputs = map(value, "inputs")?,
            ("outputs", value) => manifest.outputs = map(value, "outputs")?,
            (key, _) => return Err(format!("an unknown or mistyped key {key:?}")),
        }
    }
    if version != Some(1) {
        return Err("version is 1".to_owned());
    }
    Ok(manifest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Manifest {
        let mut manifest = Manifest {
            commit: "0123456789ab".to_owned(),
            reason: "a \"quoted\" reason\\ on two\nlines".to_owned(),
            ..Manifest::default()
        };
        manifest
            .pairs
            .insert("deck.pptx".to_owned(), "corpus:deck.pptx".to_owned());
        manifest
            .inputs
            .insert("corpus:deck.pptx".to_owned(), "ab".repeat(32));
        manifest
    }

    #[test]
    fn prints_and_reads_back_its_manifest() {
        let manifest = sample();
        let text = print_manifest(&manifest);
        assert_eq!(parse_manifest(&text).unwrap(), manifest);
        assert!(text.contains("\n  \"outputs\": {}\n}\n"));
    }

    #[test]
    fn refuses_what_it_never_writes() {
        let text = print_manifest(&sample());
        assert!(parse_manifest(&text.replace("\"version\": 1", "\"version\": 2")).is_err());
        assert!(parse_manifest(&text.replace("\"commit\"", "\"comit\"")).is_err());
        assert!(parse_manifest(&format!("{text}x")).is_err());
        assert!(parse_manifest(&text.replace("\"pairs\": {", "\"pairs\": {\"a\": 1,")).is_err());
        assert!(parse_manifest("{\"a\": {\"b\": {\"c\": {\"d\": {}}}}}").is_err());
    }

    /// A private folder for one test, removed when it ends.
    struct Private(PathBuf);

    impl Private {
        fn new(name: &str) -> Self {
            let dir =
                std::env::temp_dir().join(format!("xtask-golden-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Private(dir)
        }

        fn write(&self, input: &str, bytes: &[u8]) {
            let path = self.0.join(input);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, bytes).unwrap();
        }
    }

    impl Drop for Private {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    /// A manifest pinning every file `private` holds.
    fn pinning_all(private: &Private) -> Manifest {
        let mut manifest = sample();
        manifest.pairs.clear();
        manifest.inputs = pinned_inputs(&BTreeMap::new(), &private.0).unwrap();
        manifest
    }

    const ROW: &str = "golden/scoring/rows/deck__s1__pptx";

    fn private_with_scoring(name: &str) -> Private {
        let private = Private::new(name);
        private.write(&format!("{ROW}/oracle.png"), b"oracle");
        private.write(&format!("{ROW}/scores.json"), b"{}");
        private.write("golden/scoring/rows/deck__s2__pptx/scores.json", b"{}");
        private.write("golden/scoring/index.json", b"{}");
        private.write("bench/cache/fonts/a.ttf", b"font");
        private.write("golden/fixtures/ir/min.presentation-ir.json", b"{}");
        private.write("expect/keynote-motion/deck.json", b"{}");
        private
    }

    #[test]
    fn pins_and_checks_every_scoring_golden() {
        let private = private_with_scoring("check");
        let manifest = pinning_all(&private);
        assert_eq!(scoring_counts(&manifest), (4, 2));
        assert_eq!(input_problems(&manifest, &private.0).unwrap(), []);

        private.write(&format!("{ROW}/scores.json"), b"{\"moved\": 1}");
        private.write(&format!("{ROW}/textless.png"), b"new");
        std::fs::remove_file(private.0.join(ROW).join("oracle.png")).unwrap();
        let problems = input_problems(&manifest, &private.0).unwrap();
        let textless = format!("{ROW}/textless.png");
        assert_eq!(
            problems,
            [
                InputProblem::Gone(format!("{ROW}/oracle.png")),
                InputProblem::Differs(format!("{ROW}/scores.json")),
                InputProblem::Unpinned(textless, hex(&Sha256::digest(b"new"))),
            ]
        );
    }

    #[test]
    fn pins_only_new_files_and_only_while_every_pin_holds() {
        let private = private_with_scoring("pin");
        let mut manifest = pinning_all(&private);
        let scoring: Vec<String> = manifest
            .inputs
            .keys()
            .filter(|input| input.starts_with("golden/scoring/"))
            .cloned()
            .collect();
        for input in &scoring {
            manifest.inputs.remove(input);
        }
        assert_eq!(scoring_counts(&manifest), (0, 0));
        let reason = manifest.reason.clone();
        let pinned =
            with_new_pins(manifest, &private.0, " the scoring goldens join the set ").unwrap();
        assert_eq!(scoring_counts(&pinned), (4, 2));
        assert_eq!(
            pinned,
            pinning_all(&private)
                .with_reason(&format!("{reason} the scoring goldens join the set"))
        );
        assert!(with_new_pins(pinned.clone(), &private.0, "again").is_err());
        assert!(with_new_pins(pinned.clone(), &private.0, " ").is_err());

        private.write("bench/cache/fonts/a.ttf", b"another font");
        private.write("golden/scoring/rows/deck__s3__pptx/scores.json", b"{}");
        let refused = with_new_pins(pinned, &private.0, "a new row").unwrap_err();
        assert!(
            refused.contains("input bench/cache/fonts/a.ttf: differs from its pin"),
            "{refused}"
        );
    }

    /// Only the expectations the suites compare with can gain a pin: a new font or fixture changes or adds outputs, so
    /// it takes a new recording.
    #[test]
    fn pins_no_new_file_that_feeds_the_outputs() {
        let private = private_with_scoring("feeds");
        let manifest = pinning_all(&private);
        for (input, bytes) in [
            ("golden/fixtures/ir/new.presentation-ir.json", &b"{}"[..]),
            ("bench/cache/fonts/b.ttf", b"another face"),
        ] {
            private.write(input, bytes);
            let refused = with_new_pins(manifest.clone(), &private.0, "a new input").unwrap_err();
            assert!(
                refused.contains(&format!(
                    "input {input}: not pinned, and it feeds the outputs"
                )),
                "{refused}"
            );
            std::fs::remove_file(private.0.join(input)).unwrap();
        }
        private.write("expect/keynote-motion/other.json", b"{}");
        let pinned = with_new_pins(manifest, &private.0, "a new expectation").unwrap();
        assert!(
            pinned
                .inputs
                .contains_key("expect/keynote-motion/other.json")
        );
    }

    impl Manifest {
        fn with_reason(mut self, reason: &str) -> Self {
            self.reason = reason.to_owned();
            self
        }
    }

    #[test]
    fn runs_a_suite_in_release_when_its_crate_asks() {
        let toml = |metadata: &str| {
            format!(
                "[package]\nname = \"slidock-x\"\n\n[features]\nhost = []\ngolden = [\"host\"]\n{metadata}\n[dependencies]\nname = \"y\"\n"
            )
        };
        let suite = |release| Suite {
            name: "slidock-x".to_owned(),
            release,
        };
        assert_eq!(suite_of(&toml("")).unwrap(), Some(suite(false)));
        let release = "\n[package.metadata.golden]\n# Minutes in debug.\nrelease = true\n";
        assert_eq!(suite_of(&toml(release)).unwrap(), Some(suite(true)));
        assert_eq!(
            suite_of(&toml("\n[package.metadata.golden]\nrelease = false\n")).unwrap(),
            Some(suite(false))
        );
        for other in ["release = 1", "release = \"true\"", "profile = \"release\""] {
            let metadata = format!("\n[package.metadata.golden]\n{other}\n");
            assert!(suite_of(&toml(&metadata)).is_err(), "{other}");
        }
        assert_eq!(suite_of("[package]\nname = \"slidock-y\"\n").unwrap(), None);
        let args: Vec<String> = suite_command(&suite(true))
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args[..3], ["test", "--locked", "--release"]);
        let args: Vec<String> = suite_command(&suite(false))
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert!(!args.contains(&"--release".to_owned()));
    }

    /// The suites that take over a minute in a debug build run in release.
    #[test]
    fn the_slow_suites_run_in_release() {
        let release: Vec<String> = golden_crates()
            .unwrap()
            .into_iter()
            .filter(|suite| suite.release)
            .map(|suite| suite.name)
            .collect();
        assert_eq!(
            release,
            [SCORING_CRATE, "slidock-keynote", "slidock", "slidock-cli"]
        );
    }

    #[test]
    fn finds_a_corpus_source_by_its_presentation_id() {
        let private = Private::new("corpus");
        private.write(
            CORPUS_MANIFEST,
            br#"{"version": 1, "presentations": [{"id": "deck", "sources": [
                {"frontend": "pptx", "file": "Any Name.pptx"}, {"frontend": "odp", "file": "Any Name.odp"}]}]}"#,
        );
        assert_eq!(
            input_path(&private.0, "corpus:deck.odp").unwrap(),
            private.0.join("corpus/Any Name.odp")
        );
        assert_eq!(
            input_path(&private.0, "golden/scoring/index.json").unwrap(),
            private.0.join("golden/scoring/index.json")
        );
        for missing in ["corpus:deck.keynote", "corpus:other.pptx", "corpus:deck"] {
            assert!(input_path(&private.0, missing).is_err(), "{missing}");
        }
    }

    #[test]
    fn names_a_pairs_frontend_by_its_suffix() {
        assert_eq!(frontend("tables-keynote.keynote").unwrap(), "keynote");
        assert!(frontend("tables").is_err());
    }
}
