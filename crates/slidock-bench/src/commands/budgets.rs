//! `budgets` and its helper `measure-child`. See [`crate::budgets`].

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Args;
use slidock_bench_schema::stamp::{CompileCost, DeckCost, PerfStamp};
use slidock_bench_schema::{PairKey, Sha256, Tier};

use crate::budgets::load::{self, Load};
use crate::budgets::native::{measure_child, measure_compile};
use crate::budgets::wasm::{WasmLeg, app_build};
use crate::budgets::{
    Budgets, BusyCores, Judgement, Line, combine, compare, grouped, player_gzip_budget,
    player_sizes, record, wasm_size,
};
use crate::compile::{COMPILE_TIMEOUT, compiler_binary};
use crate::corpus::{Corpus, Selection, load_manifest, refuse_holdout, select_pairs};
use crate::error::{BenchError, Exit, Result};
use crate::hash::FileHashes;
use crate::results::{latest_run_dir, read_results, results_path, write_results};
use crate::setup::{Context, parse_list, parse_tier};

#[derive(Debug, Args)]
pub struct BudgetsArgs {
    /// Exits 1 when a measured value passes its budget, or has none; 3 when the machine's 1-minute load rose above
    /// its core count while measuring, which leaves the CPU times unjudged (sizes and peaks are judged at any load).
    #[arg(long)]
    check: bool,
    /// Writes the budgets file from this measurement, taken with every core but two kept busy: a compile's time plus
    /// 20 % and 30 ms, its peak plus 5 % and 8 MiB, each size plus 5 %, the player's gzip budgets at the one its build
    /// test holds (39,680 B). Refused (exit 3) when the 1-minute load is above the core count before it starts, or
    /// above the cores and its own busy threads while it measures.
    #[arg(long, conflicts_with = "check")]
    record: bool,
    /// The budgets file [default: bench/budgets.json].
    #[arg(long)]
    budgets: Option<PathBuf>,
    #[arg(long)]
    presentation: Option<String>,
    /// Tiers to select [default: smoke,real].
    #[arg(long)]
    tier: Option<String>,
    /// Compiles per pair and leg: the time is the fastest, the peak the largest.
    #[arg(long, default_value_t = 5)]
    runs: u32,
    /// The run whose stamp takes the measurement (`latest` or a run key), when its compiler is the working copy's;
    /// `none` stamps nothing.
    #[arg(long, default_value = "latest")]
    run: String,
}

fn log_line(line: &str) {
    eprintln!("{line}");
}

/// The measured wasm module: `SLIDOCK_WASM_PKG`'s, or the working copy's `pnpm wasm:build` output.
fn wasm_file(repo: &Path) -> Result<PathBuf> {
    let pkg = std::env::var("SLIDOCK_WASM_PKG")
        .ok()
        .filter(|v| !v.is_empty())
        .map_or_else(|| repo.join("packages/wasm/pkg"), PathBuf::from);
    let file = pkg.join("slidock_wasm_bg.wasm");
    if file.is_file() {
        Ok(file)
    } else {
        Err(BenchError::refused(format!(
            "{} is missing: run `pnpm wasm:build`",
            file.display()
        )))
    }
}

/// Writes the measurement into the stamp of `run` when that run's compiler is `content`; says why not otherwise.
fn stamp_run(runs: &Path, run: &str, content: &Sha256, perf: &PerfStamp) -> Result<Option<String>> {
    let dir = match run {
        "none" => return Ok(Some("not stamped (--run none)".into())),
        "latest" => match latest_run_dir(runs) {
            Some(dir) => dir,
            None => return Ok(Some("not stamped: the cache has no runs/latest".into())),
        },
        key => {
            let key = Sha256::new(key.to_owned()).map_err(|_| {
                BenchError::usage(format!("--run {key}: `latest`, `none` or a run key"))
            })?;
            runs.join(key.as_str())
        }
    };
    let file = results_path(&dir);
    let mut results = read_results(&file)?;
    let name = dir
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    if &results.stamp.compiler.content != content {
        return Ok(Some(format!(
            "not stamped: runs/{name} is of compiler {}, the working copy is {}",
            results
                .stamp
                .compiler
                .content
                .as_str()
                .get(..12)
                .unwrap_or_default(),
            content.as_str().get(..12).unwrap_or_default(),
        )));
    }
    results.stamp.perf = Some(perf.clone());
    write_results(&file, &results)?;
    Ok(Some(format!("stamped runs/{name}")))
}

/// Keeps a measurement taken on a quiet machine: `<root>/<content id>.json` and the stamp of `run` take it. One taken
/// above the load bound is only printed, so no file holds CPU times that were not judged.
fn keep(
    machine: &Machine,
    root: &Path,
    runs: &Path,
    run: &str,
    content: &Sha256,
    perf: &PerfStamp,
) -> Result<Vec<String>> {
    if !machine.quiet() {
        return Ok(vec![format!(
            "measurement not kept or stamped: the 1-minute load {} is above {}",
            machine.load,
            machine.bound()
        )]);
    }
    let record_path = root.join(format!("{}.json", content.as_str()));
    crate::cache::write_json(&record_path, perf)?;
    let mut notes = vec![format!("measurement: {}", record_path.display())];
    notes.extend(stamp_run(runs, run, content, perf)?);
    Ok(notes)
}

fn print_cost(pair: &PairKey, leg: &str, cost: &CompileCost) {
    println!(
        "  {pair:<40} {leg:<6} {:>9} ms {:>15} B",
        grouped(cost.ms),
        grouped(cost.peak_bytes)
    );
}

fn print_perf(perf: &PerfStamp) {
    println!(
        "wasm     {:>12} B raw  {:>11} B brotli q11  ({})",
        grouped(perf.wasm.raw),
        grouped(perf.wasm.brotli11),
        perf.wasm_sha256.as_str().get(..12).unwrap_or_default()
    );
    for (name, size) in [("iife", &perf.player.iife), ("esm", &perf.player.esm)] {
        println!(
            "player   {name:<4} {:>7} B raw  {:>11} B gzip -9",
            grouped(size.raw),
            grouped(size.gzip9)
        );
    }
    println!(
        "compiles to a zip (CPU time, the fastest of {}; peak, the largest; wasm in Chromium {}):",
        perf.runs, perf.chromium
    );
    for (pair, deck) in &perf.decks {
        print_cost(pair, "native", &deck.native);
        print_cost(pair, "wasm", &deck.wasm);
    }
}

fn print_lines(lines: &[Line], quiet: bool) {
    let show = |value: Option<u64>| value.map_or_else(|| "none".to_owned(), grouped);
    for line in lines {
        let mark = if !quiet && line.is_time() {
            " ---- "
        } else if line.within() {
            "  ok  "
        } else {
            " OVER "
        };
        println!(
            "{mark} {:<52} {:>15} of {:>15}",
            line.field,
            show(line.measured),
            show(line.budget)
        );
    }
}

/// The highest 1-minute load read while measuring, and what it is held to: the core count, plus the busy threads of
/// `--record`'s own, which load the machine on purpose.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Machine {
    load: Load,
    cores: u64,
    busy: u64,
    readings: u32,
}

impl Machine {
    fn quiet(&self) -> bool {
        self.load.quiet_on(self.cores.saturating_add(self.busy))
    }

    /// `the 10 cores`, or `the 10 cores and this run's 8 busy threads`.
    fn bound(&self) -> String {
        match self.busy {
            0 => format!("the {} cores", self.cores),
            busy => format!(
                "the {} cores and this run's {busy} busy threads",
                self.cores
            ),
        }
    }

    fn describe(&self) -> String {
        format!(
            "1-minute load {} (the highest of {} readings), held to {}: {}",
            self.load,
            self.readings,
            self.bound(),
            if self.quiet() {
                "quiet"
            } else if self.busy > 0 {
                "loaded, not recorded"
            } else {
                "loaded, CPU times not judged"
            }
        )
    }
}

/// The check's last line: `pass`, how many values are over and which (`2 over: wasm.raw, decks.<pair>.wasm.ms`),
/// and whether the times went unjudged.
fn verdict(judgement: &Judgement, machine: &Machine) -> String {
    let mut parts = Vec::new();
    if !judgement.over.is_empty() {
        parts.push(format!(
            "{} over: {}",
            judgement.over.len(),
            judgement.over.join(", ")
        ));
    }
    if !judgement.unjudged.is_empty() {
        parts.push(format!(
            "{} CPU times not judged: the 1-minute load {} is above {}",
            judgement.unjudged.len(),
            machine.load,
            machine.bound()
        ));
    }
    if parts.is_empty() {
        "pass".to_owned()
    } else {
        parts.join("; ")
    }
}

/// Reads the load into `machine`, keeping the highest.
async fn sample(machine: &mut Machine) -> Result<()> {
    let load = load::read().await?;
    machine.load = machine.load.max(load);
    machine.readings = machine.readings.saturating_add(1);
    Ok(())
}

pub async fn budgets(ctx: &Context, args: BudgetsArgs) -> Result<Exit> {
    if args.runs == 0 {
        return Err(BenchError::usage("--runs is a positive integer"));
    }
    let tiers = match crate::setup::list(args.tier.as_deref()) {
        Some(items) => items
            .iter()
            .map(|t| parse_tier(t).ok_or_else(|| BenchError::usage(format!("unknown tier {t}"))))
            .collect::<Result<Vec<_>>>()?,
        None => Tier::DEFAULT.to_vec(),
    };
    let presentations = parse_list(args.presentation.as_deref(), "presentation")?;
    let repo = &ctx.env.repo_root;
    let budgets_path = args
        .budgets
        .clone()
        .unwrap_or_else(|| ctx.env.bench_dir.join("budgets.json"));
    // Read before measuring: a broken budgets file is refused before the compiles.
    let budgets = if args.check {
        Some(Budgets::load(&budgets_path)?)
    } else {
        None
    };
    let player_gzip = if args.record {
        Some(player_gzip_budget(repo)?)
    } else {
        None
    };

    let mut machine = Machine {
        load: Load::from_hundredths(0),
        cores: load::cores(),
        busy: 0,
        readings: 0,
    };
    sample(&mut machine).await?;
    if args.record && !machine.quiet() {
        eprintln!("{}", machine.describe());
        println!(
            "budgets: not recorded: the 1-minute load {} is above {}",
            machine.load,
            machine.bound()
        );
        return Ok(Exit::NothingCompared);
    }

    let manifest = load_manifest(&ctx.env.manifest_path)?;
    refuse_holdout(
        &manifest,
        &tiers,
        presentations.as_deref().unwrap_or_default(),
        &[],
        "budgets",
    )?;
    let corpus = Corpus::new(
        ctx.env.require_corpus_dir()?,
        FileHashes::new(&ctx.cache.hashes, true),
        &manifest,
    );
    let selection = Selection {
        tiers,
        presentations,
        frontends: None,
    };
    let mut pairs = Vec::new();
    for (presentation, source) in select_pairs(&manifest, &selection) {
        let file = corpus.source_file(source)?.ok_or_else(|| {
            BenchError::refused(format!(
                "{}: {} is not in the corpus",
                presentation.id, source.file
            ))
        })?;
        pairs.push((PairKey::new(presentation.id.clone(), source.frontend), file));
    }
    if pairs.is_empty() {
        return Err(BenchError::usage("the selection holds no pair"));
    }

    let (wasm, wasm_sha256) = wasm_size(&wasm_file(repo)?)?;
    let player = player_sizes(repo).await?;
    let app = app_build(&repo.join("apps/web/dist"), &wasm_sha256)?;
    let binary = compiler_binary(repo, &ctx.env.target_dir, ctx.env.compiler.as_deref()).await?;
    let bench = std::env::current_exe()
        .map_err(|error| BenchError::failed(format!("this binary: {error}")))?;
    let shell = crate::renderer::native::default_binary()
        .ok_or_else(|| BenchError::refused("no headless shell: set SLIDOCK_BENCH_CHROMIUM"))?;
    let root = ctx.cache.root.join("budgets");
    let work = root.join("work");
    crate::cache::remove(&work)?;
    std::fs::create_dir_all(&work).map_err(BenchError::io(&work))?;
    let leg = WasmLeg::start(&ctx.env.bench_dir, &shell, &root.join("profile"), &app).await?;
    let busy = args.record.then(|| {
        let count = BusyCores::count();
        eprintln!("recording with {count} busy cores");
        machine.busy = u64::try_from(count).unwrap_or(u64::MAX);
        BusyCores::start(count)
    });
    let measured = measure_pairs(
        &leg,
        &bench,
        &binary,
        &pairs,
        &work,
        args.runs,
        &mut machine,
    )
    .await;
    if let Some(busy) = busy {
        busy.stop();
    }
    let chromium = leg.chromium.clone();
    let stopped = leg.stop().await;
    crate::cache::remove(&work)?;
    let decks = measured?;
    stopped?;

    let perf = PerfStamp {
        wasm,
        wasm_sha256: Sha256::new(wasm_sha256)?,
        player,
        decks,
        runs: args.runs,
        chromium,
    };
    let content = crate::stamp::compiler_stamp(repo).await?.content;
    print_perf(&perf);
    println!("{}", machine.describe());
    for note in keep(&machine, &root, &ctx.cache.runs, &args.run, &content, &perf)? {
        eprintln!("{note}");
    }
    if let Some(player_gzip) = player_gzip {
        if !machine.quiet() {
            println!(
                "budgets: not recorded: the 1-minute load {} rose above {}",
                machine.load,
                machine.bound()
            );
            return Ok(Exit::NothingCompared);
        }
        record(&perf, player_gzip).save(&budgets_path)?;
        println!("recorded {}", budgets_path.display());
    }
    match budgets {
        Some(budgets) => {
            let selected: Vec<PairKey> = pairs.iter().map(|(pair, _)| pair.clone()).collect();
            let lines = compare(&perf, &budgets, &selected);
            print_lines(&lines, machine.quiet());
            let judgement = Judgement::of(&lines, machine.quiet());
            println!("budgets: {}", verdict(&judgement, &machine));
            Ok(judgement.exit())
        }
        None => Ok(Exit::Ok),
    }
}

/// Each pair `runs` times on each leg, the native and the wasm compile of a run one after the other; the load is
/// read into `machine` after each pair.
async fn measure_pairs(
    leg: &WasmLeg,
    bench: &Path,
    binary: &Path,
    pairs: &[(PairKey, PathBuf)],
    work: &Path,
    runs: u32,
    machine: &mut Machine,
) -> Result<BTreeMap<PairKey, DeckCost>> {
    let mut decks = BTreeMap::new();
    for (pair, input) in pairs {
        let zip = work.join(format!("{}.zip", pair.to_string().replace(':', ".")));
        let mut native = Vec::new();
        let mut wasm = Vec::new();
        for run in 1..=runs {
            let cost = measure_compile(bench, binary, input, &zip, COMPILE_TIMEOUT).await?;
            crate::cache::remove(&zip)?;
            log_line(&format!(
                "{pair} run {run}: native {} ms, {} B",
                cost.ms,
                grouped(cost.peak_bytes)
            ));
            native.push(cost);
            let cost = leg.compile(input, COMPILE_TIMEOUT).await?;
            log_line(&format!(
                "{pair} run {run}: wasm {} ms, {} B",
                cost.ms,
                grouped(cost.peak_bytes)
            ));
            wasm.push(cost);
        }
        if let (Some(native), Some(wasm)) = (combine(&native), combine(&wasm)) {
            decks.insert(pair.clone(), DeckCost { native, wasm });
        }
        sample(machine).await?;
        log_line(&format!("{pair}: {}", machine.describe()));
    }
    Ok(decks)
}

/// `measure-child`: runs the command after `--` and prints its [`crate::budgets::native::ChildCost`] as JSON.
pub async fn measure_child_command(timeout_secs: u64, command: Vec<OsString>) -> Result<Exit> {
    let Some((program, arguments)) = command.split_first() else {
        return Err(BenchError::usage("measure-child needs a command after --"));
    };
    let cost = measure_child(program, arguments, Duration::from_secs(timeout_secs)).await?;
    let text = serde_json::to_string(&cost)
        .map_err(|error| BenchError::failed(format!("measure-child: {error}")))?;
    println!("{text}");
    Ok(Exit::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_bench_schema::stamp::{PlayerSizes, ScriptSize, WasmSize};

    fn perf() -> PerfStamp {
        let cost = CompileCost {
            ms: 1,
            peak_bytes: 2,
        };
        PerfStamp {
            wasm: WasmSize {
                raw: 3,
                brotli11: 1,
            },
            wasm_sha256: Sha256::new("b".repeat(64)).unwrap(),
            player: PlayerSizes {
                iife: ScriptSize { raw: 2, gzip9: 1 },
                esm: ScriptSize { raw: 2, gzip9: 1 },
            },
            decks: BTreeMap::from([(
                "demo-a:pptx".parse().unwrap(),
                DeckCost {
                    native: cost,
                    wasm: cost,
                },
            )]),
            runs: 1,
            chromium: "148.0.7778.0".into(),
        }
    }

    #[test]
    fn the_verdict_names_each_value_over_and_the_times_left_unjudged() {
        let line = |field: &str, measured, budget| Line {
            field: field.to_owned(),
            measured,
            budget,
        };
        let machine = |hundredths| Machine {
            load: Load::from_hundredths(hundredths),
            cores: 10,
            busy: 0,
            readings: 3,
        };
        let quiet = machine(1000);
        let loaded = machine(1001);
        let judge = |lines: &[Line], machine: &Machine| {
            let judgement = Judgement::of(lines, machine.quiet());
            (verdict(&judgement, machine), judgement.exit().code())
        };
        let within = [
            line("wasm.raw", Some(1), Some(1)),
            line("decks.a:pptx.wasm.ms", Some(1), Some(1)),
        ];
        assert_eq!(judge(&within, &quiet), ("pass".to_owned(), 0));
        assert_eq!(
            judge(&within, &loaded),
            (
                "1 CPU times not judged: the 1-minute load 10.01 is above the 10 cores".to_owned(),
                3
            )
        );
        let over = [
            line("wasm.raw", Some(2), Some(1)),
            line("wasm.brotli11", Some(1), Some(1)),
            line("decks.a:pptx.wasm.ms", None, Some(1)),
        ];
        assert_eq!(
            judge(&over, &quiet),
            ("2 over: wasm.raw, decks.a:pptx.wasm.ms".to_owned(), 1)
        );
        assert_eq!(
            judge(&over, &loaded),
            (
                "1 over: wasm.raw; 1 CPU times not judged: the 1-minute load 10.01 is above the 10 cores"
                    .to_owned(),
                1
            )
        );
        assert!(
            quiet.describe().ends_with(": quiet"),
            "{}",
            quiet.describe()
        );
        assert!(loaded.describe().contains("not judged"));
    }

    #[test]
    fn a_record_holds_the_load_to_the_cores_and_its_own_busy_threads() {
        let recording = |hundredths| Machine {
            load: Load::from_hundredths(hundredths),
            cores: 10,
            busy: 8,
            readings: 11,
        };
        assert!(recording(1800).quiet());
        let loaded = recording(1801);
        assert!(!loaded.quiet());
        assert_eq!(
            loaded.describe(),
            "1-minute load 18.01 (the highest of 11 readings), held to the 10 cores and this run's 8 busy threads: \
             loaded, not recorded"
        );
    }

    #[test]
    fn a_measurement_taken_above_the_load_bound_is_neither_kept_nor_stamped() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path().join("runs");
        let root = dir.path().join("budgets");
        let key = "c".repeat(64);
        let results_file = runs.join(&key).join("results.json");
        std::fs::create_dir_all(runs.join(&key)).unwrap();
        write_results(&results_file, &crate::test_support::results(Vec::new())).unwrap();
        crate::results::set_latest(&runs, &key).unwrap();
        let content = read_results(&results_file).unwrap().stamp.compiler.content;
        let record_file = root.join(format!("{}.json", content.as_str()));
        let machine = |hundredths, busy| Machine {
            load: Load::from_hundredths(hundredths),
            cores: 10,
            busy,
            readings: 2,
        };

        for loaded in [machine(1001, 0), machine(1801, 8)] {
            let notes = keep(&loaded, &root, &runs, "latest", &content, &perf()).unwrap();
            assert_eq!(notes.len(), 1, "{notes:?}");
            assert!(notes[0].starts_with("measurement not kept or stamped: the 1-minute load"));
            assert!(!record_file.exists());
            assert!(read_results(&results_file).unwrap().stamp.perf.is_none());
        }

        let notes = keep(&machine(1800, 8), &root, &runs, "latest", &content, &perf()).unwrap();
        assert_eq!(
            notes,
            [
                format!("measurement: {}", record_file.display()),
                format!("stamped runs/{key}")
            ]
        );
        let kept: PerfStamp =
            serde_json::from_slice(&std::fs::read(&record_file).unwrap()).unwrap();
        assert_eq!(kept, perf());
        assert_eq!(
            read_results(&results_file).unwrap().stamp.perf,
            Some(perf())
        );
    }

    #[test]
    fn only_a_run_of_the_same_compiler_takes_the_measurement() {
        let dir = tempfile::tempdir().unwrap();
        let runs = dir.path();
        let key = "c".repeat(64);
        std::fs::create_dir_all(runs.join(&key)).unwrap();
        write_results(
            &runs.join(&key).join("results.json"),
            &crate::test_support::results(Vec::new()),
        )
        .unwrap();
        crate::results::set_latest(runs, &key).unwrap();
        let results = read_results(&runs.join(&key).join("results.json")).unwrap();
        let ours = results.stamp.compiler.content.clone();
        let other = Sha256::new("9".repeat(64)).unwrap();

        let note = stamp_run(runs, "latest", &other, &perf()).unwrap().unwrap();
        assert!(note.starts_with("not stamped: runs/cccc"), "{note}");
        assert!(
            read_results(&runs.join(&key).join("results.json"))
                .unwrap()
                .stamp
                .perf
                .is_none()
        );

        let note = stamp_run(runs, "latest", &ours, &perf()).unwrap().unwrap();
        assert_eq!(note, format!("stamped runs/{key}"));
        let stamped = read_results(&runs.join(&key).join("results.json")).unwrap();
        assert_eq!(stamped.stamp.perf, Some(perf()));
        // The run keeps its key: the measurement is stamped after the run.
        assert_eq!(
            stamped.stamp.run_key().unwrap(),
            results.stamp.run_key().unwrap()
        );

        assert!(
            stamp_run(runs, "none", &ours, &perf())
                .unwrap()
                .unwrap()
                .contains("--run none")
        );
        assert!(stamp_run(runs, "nope", &ours, &perf()).is_err());
        let empty = tempfile::tempdir().unwrap();
        assert!(
            stamp_run(empty.path(), "latest", &ours, &perf())
                .unwrap()
                .unwrap()
                .contains("no runs/latest")
        );
    }
}
