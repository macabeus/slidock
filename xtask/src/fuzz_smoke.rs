//! `cargo xtask fuzz-smoke`: every cargo-fuzz target in `fuzz/` built once, then run for a while from its seeds. A
//! target's corpus is `fuzz/corpus/<target>` (gitignored), seeded from `fuzz/seeds/<target>` and the committed
//! fixtures and probe decks the target reads (`fuzz/README.md`). A run given a further corpus (`--extra`) writes
//! what it finds into `fuzz/corpus-extra/<target>` (gitignored) instead, so `corpus/` never holds inputs derived from
//! it. Any crash, timeout or out-of-memory fails the run; libFuzzer leaves the input in `fuzz/artifacts/<target>/`.
//! Each target's executions, coverage and corpus size are printed.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::Duration;

const USAGE: &str = "usage: cargo xtask fuzz-smoke [--seconds <n>] [--only <target,...>] [--jobs <n>] [--extra <dir>]

  Builds the fuzz targets with cargo-fuzz and runs each for --seconds (at least 1, default 60) with
  -rss_limit_mb=2048 and -timeout=10, --jobs at a time (default 1). --extra adds <dir>/<target> as a further,
  read-only corpus, such as one seeded from the private corpus; what such a run finds goes to
  fuzz/corpus-extra/<target>, never to fuzz/corpus/<target>. The nightly toolchain is $SLIDOCK_FUZZ_TOOLCHAIN
  (default nightly-2026-10-25). Logs are in <target dir>/fuzz-smoke/<target>.log.";

const DEFAULT_TOOLCHAIN: &str = "nightly-2026-10-25";
const RSS_LIMIT_MB: u32 = 2048;
const TIMEOUT_SECONDS: u32 = 10;
/// The font container fixtures the font targets start from.
const CONTAINER_FIXTURES: &str = "crates/slidock-fonts/tests/data/container";
/// The media decoder targets, which all start from `fuzz/seeds/media`.
const MEDIA_TARGETS: [&str; 6] = ["tiff", "bmp", "svg", "image_size", "heif", "movie"];
/// Our probe decks the zip target starts from (the fonts probe ODP is left out: 2.3 MB of fonts).
const ZIP_PROBES: [&str; 3] = [
    "probes/out/fonts-probe.pptx",
    "probes/out/motion-probe.pptx",
    "probes/out/motion-probe.odp",
];

#[derive(Debug)]
struct Args {
    seconds: u32,
    only: Option<Vec<String>>,
    jobs: usize,
    extra: Option<PathBuf>,
}

fn parse(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        seconds: 60,
        only: None,
        jobs: 1,
        extra: None,
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        let value = rest.next().ok_or_else(|| USAGE.to_owned());
        match arg.as_str() {
            // libFuzzer reads `-max_total_time=0` as no limit.
            "--seconds" => {
                parsed.seconds = number(value?)?;
                if parsed.seconds == 0 {
                    return Err(format!("--seconds must be at least 1\n\n{USAGE}"));
                }
            }
            "--jobs" => parsed.jobs = number::<usize>(value?)?.max(1),
            "--only" => parsed.only = Some(value?.split(',').map(str::to_owned).collect()),
            "--extra" => parsed.extra = Some(PathBuf::from(value?)),
            _ => return Err(USAGE.to_owned()),
        }
    }
    Ok(parsed)
}

fn number<T: std::str::FromStr>(value: &str) -> Result<T, String> {
    value
        .parse()
        .map_err(|_| format!("{value} is not a number\n\n{USAGE}"))
}

pub fn run(args: &[String]) -> Result<(), String> {
    let args = parse(args)?;
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    let fuzz = root.join("fuzz");
    let manifest = fs::read_to_string(fuzz.join("Cargo.toml")).map_err(|e| e.to_string())?;
    let mut targets = fuzz_targets(&manifest);
    if let Some(only) = &args.only {
        if let Some(unknown) = only.iter().find(|name| !targets.contains(name)) {
            return Err(format!(
                "no fuzz target {unknown}; the targets are {}",
                targets.join(", ")
            ));
        }
        targets.retain(|name| only.contains(name));
    }
    let toolchain =
        std::env::var("SLIDOCK_FUZZ_TOOLCHAIN").unwrap_or_else(|_| DEFAULT_TOOLCHAIN.to_owned());
    let target_dir = match std::env::var_os("CARGO_TARGET_DIR") {
        Some(dir) => PathBuf::from(dir),
        None => root.join("target"),
    };
    let logs = target_dir.join("fuzz-smoke");
    fs::create_dir_all(&logs).map_err(|e| e.to_string())?;

    let status = cargo_fuzz(&toolchain, &fuzz, &target_dir)
        .arg("build")
        .status()
        .map_err(|e| format!("cargo +{toolchain} fuzz build: {e}"))?;
    if !status.success() {
        return Err(format!("cargo +{toolchain} fuzz build failed ({status})"));
    }

    let mut pending: Vec<&str> = targets.iter().map(String::as_str).rev().collect();
    let mut running: Vec<(&str, Child)> = Vec::new();
    let mut results: Vec<(&str, bool, Stats)> = Vec::new();
    while !pending.is_empty() || !running.is_empty() {
        while running.len() < args.jobs
            && let Some(target) = pending.pop()
        {
            let mut corpora = corpora(&fuzz, target, args.extra.as_deref());
            if args.extra.is_some() && corpora.last().is_some_and(|extra| !extra.is_dir()) {
                corpora.pop();
            }
            if let Some(written) = corpora.first() {
                fs::create_dir_all(written).map_err(|e| format!("{}: {e}", written.display()))?;
            }
            seed(&root, target, &fuzz.join("corpus").join(target))?;
            let log =
                fs::File::create(logs.join(format!("{target}.log"))).map_err(|e| e.to_string())?;
            let mut command = cargo_fuzz(&toolchain, &fuzz, &target_dir);
            command.args(["run", target]).args(&corpora);
            command
                .arg("--")
                .args(libfuzzer_flags(target, args.seconds));
            let child = command
                .stdout(Stdio::null())
                .stderr(log)
                .spawn()
                .map_err(|e| format!("{target}: {e}"))?;
            println!("{target}: running for {} s", args.seconds);
            running.push((target, child));
        }
        std::thread::sleep(Duration::from_millis(250));
        let mut still = Vec::new();
        for (target, mut child) in running {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(status) => {
                    let log =
                        fs::read_to_string(logs.join(format!("{target}.log"))).unwrap_or_default();
                    let stats = Stats::read(&log);
                    println!(
                        "{target}: {} {}",
                        if status.success() { "ok" } else { "FAILED" },
                        stats
                    );
                    results.push((target, status.success(), stats));
                }
                None => still.push((target, child)),
            }
        }
        running = still;
    }

    println!(
        "\n{:<8} {:>12} {:>8} {:>8} {:>8}  result",
        "target", "execs", "cov", "ft", "corpus"
    );
    for (target, ok, stats) in &results {
        println!(
            "{target:<8} {:>12} {:>8} {:>8} {:>8}  {}",
            stats.execs.unwrap_or(0),
            stats.cov.unwrap_or(0),
            stats.features.unwrap_or(0),
            stats.corpus.unwrap_or(0),
            if *ok { "ok" } else { "FAILED" }
        );
    }
    let failed: Vec<&str> = results
        .iter()
        .filter(|(_, ok, _)| !ok)
        .map(|(target, ..)| *target)
        .collect();
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!(
            "fuzz targets failed: {} (inputs in fuzz/artifacts/<target>/, logs in {})",
            failed.join(", "),
            logs.display()
        ))
    }
}

fn cargo_fuzz(toolchain: &str, fuzz: &Path, target_dir: &Path) -> Command {
    let mut command = Command::new("cargo");
    command
        .arg(format!("+{toolchain}"))
        .arg("fuzz")
        .current_dir(fuzz)
        .env("CARGO_TARGET_DIR", target_dir)
        // `cargo xtask` runs under the pinned stable toolchain; the nightly is picked by `+toolchain` alone.
        .env_remove("RUSTUP_TOOLCHAIN")
        .env_remove("RUSTC")
        .env_remove("RUSTDOC")
        .env_remove("CARGO");
    command
}

/// The libFuzzer flags of a run: its time, the memory and per-input time limits, and the input size the target
/// reads (none: libFuzzer's guess from the corpus).
fn libfuzzer_flags(target: &str, seconds: u32) -> Vec<String> {
    let mut flags = vec![
        format!("-max_total_time={seconds}"),
        format!("-rss_limit_mb={RSS_LIMIT_MB}"),
        format!("-timeout={TIMEOUT_SECONDS}"),
        "-print_final_stats=1".to_owned(),
    ];
    if let Some(max_len) = max_len(target) {
        flags.push(format!("-max_len={max_len}"));
    }
    flags
}

/// The largest input a target is given, where libFuzzer's default would be too small or too large.
fn max_len(target: &str) -> Option<u32> {
    match target {
        "zip" | "compile" => Some(512 * 1024),
        "xml" | "iwa" => Some(64 * 1024),
        target if MEDIA_TARGETS.contains(&target) => Some(64 * 1024),
        "proto" => Some(16 * 1024),
        _ => None,
    }
}

/// The `[[bin]]` names of the fuzz crate's manifest, in order.
fn fuzz_targets(manifest: &str) -> Vec<String> {
    let mut targets = Vec::new();
    let mut in_bin = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_bin = line == "[[bin]]";
        } else if in_bin
            && let Some(value) = line.strip_prefix("name").map(str::trim_start)
            && let Some(name) = value.strip_prefix('=').map(|v| v.trim().trim_matches('"'))
        {
            targets.push(name.to_owned());
        }
    }
    targets
}

/// A run's corpus directories, the one libFuzzer writes first: `corpus/<target>`, or with a further corpus
/// `corpus-extra/<target>`, then `corpus/<target>` and the further one, both only read (the further one only where
/// it exists).
fn corpora(fuzz: &Path, target: &str, extra: Option<&Path>) -> Vec<PathBuf> {
    let corpus = fuzz.join("corpus").join(target);
    match extra {
        None => vec![corpus],
        Some(extra) => vec![
            fuzz.join("corpus-extra").join(target),
            corpus,
            extra.join(target),
        ],
    }
}

/// Copies a target's seeds into its corpus, keeping what else the corpus holds and refreshing a copy whose seed
/// changed. Each is named `seed-<name>.seed`, so formatters pass over a copied `README.md`.
fn seed(root: &Path, target: &str, corpus: &Path) -> Result<(), String> {
    fs::create_dir_all(corpus).map_err(|e| e.to_string())?;
    for (name, bytes) in seeds(root, target)? {
        let path = corpus.join(format!("seed-{name}.seed"));
        if fs::read(&path).ok().as_deref() != Some(bytes.as_slice()) {
            fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        }
    }
    Ok(())
}

fn files(dir: &Path) -> Result<Vec<(String, Vec<u8>)>, String> {
    if !dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut out = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_file()
            && let Some(name) = path.file_name().and_then(|name| name.to_str())
        {
            out.push((name.to_owned(), fs::read(&path).map_err(|e| e.to_string())?));
        }
    }
    out.sort();
    Ok(out)
}

/// A target's seeds: `fuzz/seeds/<target>`, and the fixtures and probes it reads, as `fuzz/README.md` lists them.
fn seeds(root: &Path, target: &str) -> Result<Vec<(String, Vec<u8>)>, String> {
    let mut out = files(&root.join("fuzz/seeds").join(target))?;
    if MEDIA_TARGETS.contains(&target) {
        out.extend(files(&root.join("fuzz/seeds/media"))?);
    }
    let fixtures = || files(&root.join(CONTAINER_FIXTURES));
    let with_extension = |extension: &str| -> Result<Vec<(String, Vec<u8>)>, String> {
        let suffix = format!(".{extension}");
        Ok(fixtures()?
            .into_iter()
            .filter(|(name, _)| name.ends_with(&suffix))
            .collect())
    };
    match target {
        "sniff" => out.extend(fixtures()?),
        "eot" | "woff" | "woff2" => out.extend(with_extension(target)?),
        "mtx" => out.extend(
            with_extension("eot")?
                .into_iter()
                .filter(|(name, _)| name.ends_with("-mtx.eot"))
                .filter_map(|(name, bytes)| Some((name, mtx_font_data(&bytes)?.to_vec()))),
        ),
        "unwrap" => out.extend(fixtures()?.into_iter().map(|(name, bytes)| {
            let mut hinted = vec![0];
            hinted.extend(bytes);
            (name, hinted)
        })),
        "zip" => {
            for probe in ZIP_PROBES {
                let path = root.join(probe);
                let bytes = fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
                let name = probe.rsplit('/').next().unwrap_or(probe);
                out.push((name.to_owned(), bytes));
            }
        }
        _ => {}
    }
    Ok(out)
}

/// An EOT's font data: the `FontDataSize` bytes before `EOTSize`.
fn mtx_font_data(eot: &[u8]) -> Option<&[u8]> {
    let word = |at: usize| -> Option<usize> {
        let bytes: [u8; 4] = eot.get(at..at + 4)?.try_into().ok()?;
        usize::try_from(u32::from_le_bytes(bytes)).ok()
    };
    let (size, data) = (word(0)?, word(4)?);
    eot.get(size.checked_sub(data)?..size)
}

/// What libFuzzer printed at the end of a run.
#[derive(Debug, Default, PartialEq, Eq)]
struct Stats {
    execs: Option<u64>,
    cov: Option<u64>,
    features: Option<u64>,
    corpus: Option<u64>,
}

impl Stats {
    fn read(log: &str) -> Stats {
        let mut stats = Stats::default();
        for line in log.lines() {
            if let Some(value) = line.strip_prefix("stat::number_of_executed_units:") {
                stats.execs = value.trim().parse().ok();
            }
            if line.contains(" cov: ") {
                let field = |key: &str| {
                    let rest = line.split(key).nth(1)?;
                    rest.trim_start()
                        .split(|c: char| !c.is_ascii_digit())
                        .next()?
                        .parse()
                        .ok()
                };
                stats.cov = field(" cov: ").or(stats.cov);
                stats.features = field(" ft: ").or(stats.features);
                stats.corpus = field(" corp: ").or(stats.corpus);
            }
        }
        stats
    }
}

impl std::fmt::Display for Stats {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let show = |value: Option<u64>| value.map_or("?".to_owned(), |v| v.to_string());
        write!(
            f,
            "execs {} cov {} ft {} corpus {}",
            show(self.execs),
            show(self.cov),
            show(self.features),
            show(self.corpus)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_the_bins_of_the_fuzz_manifest() {
        let manifest = "[package]\nname = \"slidock-fuzz\"\n\n[dependencies]\nname = \"x\"\n\n[[bin]]\nname = \"zip\"\n\
                        path = \"fuzz_targets/zip.rs\"\n\n[[bin]]\nname = \"xml\"\n";
        assert_eq!(fuzz_targets(manifest), ["zip", "xml"]);
    }

    #[test]
    fn every_target_of_the_fuzz_crate_is_listed() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let manifest = fs::read_to_string(root.join("fuzz/Cargo.toml")).unwrap();
        let targets = fuzz_targets(&manifest);
        for target in [
            "zip", "xml", "iwa", "proto", "woff2", "mtx", "eot", "woff", "ctf", "sniff", "unwrap",
        ] {
            assert!(
                targets.iter().any(|t| t == target),
                "{target} in {targets:?}"
            );
        }
        for target in &targets {
            assert!(
                root.join(format!("fuzz/fuzz_targets/{target}.rs"))
                    .is_file()
            );
            assert!(
                !seeds(&root, target).unwrap().is_empty(),
                "{target} has no seeds"
            );
        }
    }

    #[test]
    fn reads_the_final_stats() {
        let log = "#2\tINITED cov: 10 ft: 11 corp: 1/1b exec/s: 0 rss: 30Mb\n\
                   #4096\tpulse  cov: 40 ft: 52 corp: 7/90b lim: 43 exec/s: 2048 rss: 31Mb\n\
                   #9000\tDONE   cov: 41 ft: 55 corp: 8/97b lim: 4096 exec/s: 4500 rss: 31Mb\n\
                   Done 9000 runs in 2 second(s)\n\
                   stat::number_of_executed_units: 9000\n\
                   stat::peak_rss_mb:              31\n";
        assert_eq!(
            Stats::read(log),
            Stats {
                execs: Some(9000),
                cov: Some(41),
                features: Some(55),
                corpus: Some(8),
            }
        );
        assert_eq!(Stats::read("no stats"), Stats::default());
    }

    #[test]
    fn takes_an_eots_font_data_for_mtx() {
        let mut eot = 12u32.to_le_bytes().to_vec();
        eot.extend(3u32.to_le_bytes());
        eot.extend([0, 7, 8, 9]);
        assert_eq!(mtx_font_data(&eot), Some(&[7, 8, 9][..]));
        assert_eq!(mtx_font_data(&eot[..6]), None);
        let mut lying = 20u32.to_le_bytes().to_vec();
        lying.extend(30u32.to_le_bytes());
        assert_eq!(mtx_font_data(&lying), None);
    }

    #[test]
    fn refuses_a_run_of_no_time() {
        let args = |seconds: &str| parse(&["--seconds".to_owned(), seconds.to_owned()]);
        assert!(
            args("0")
                .unwrap_err()
                .starts_with("--seconds must be at least 1")
        );
        assert_eq!(args("1").unwrap().seconds, 1);
        assert_eq!(parse(&[]).unwrap().seconds, 60);
    }

    #[test]
    fn writes_a_further_corpus_runs_finds_apart() {
        let fuzz = Path::new("fuzz");
        assert_eq!(corpora(fuzz, "zip", None), [fuzz.join("corpus/zip")]);
        assert_eq!(
            corpora(fuzz, "zip", Some(Path::new("private"))),
            [
                fuzz.join("corpus-extra/zip"),
                fuzz.join("corpus/zip"),
                Path::new("private/zip").to_path_buf()
            ]
        );
    }

    #[test]
    fn refreshes_a_seed_copy_whose_seed_changed() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
        let corpus =
            std::env::temp_dir().join(format!("slidock-fuzz-smoke-{}", std::process::id()));
        let _ = fs::remove_dir_all(&corpus);
        let copy = corpus.join("seed-empty.zip.seed");
        let found = corpus.join("found");
        fs::create_dir_all(&corpus).unwrap();
        fs::write(&copy, b"an older seed").unwrap();
        fs::write(&found, b"an input libFuzzer kept").unwrap();
        seed(&root, "zip", &corpus).unwrap();
        assert_eq!(
            fs::read(&copy).unwrap(),
            fs::read(root.join("fuzz/seeds/zip/empty.zip")).unwrap()
        );
        assert_eq!(fs::read(&found).unwrap(), b"an input libFuzzer kept");
        fs::remove_dir_all(&corpus).unwrap();
    }

    #[test]
    fn gives_the_limits_on_every_run() {
        let flags = libfuzzer_flags("zip", 60);
        assert!(flags.contains(&"-max_total_time=60".to_owned()));
        assert!(flags.contains(&"-rss_limit_mb=2048".to_owned()));
        assert!(flags.contains(&"-timeout=10".to_owned()));
        assert!(flags.contains(&"-max_len=524288".to_owned()));
        assert!(
            !libfuzzer_flags("woff2", 1)
                .iter()
                .any(|flag| flag.starts_with("-max_len"))
        );
    }
}
