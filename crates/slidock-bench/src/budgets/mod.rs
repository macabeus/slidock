//! `budgets`: what the compiler costs to ship and to run. It measures the wasm module (raw and brotli
//! quality 11), the player's builds (raw and gzip level 9), and per corpus pair the compile to a zip, natively (the
//! CLI, its peak resident set) and as wasm in headless Chromium through the app's own compile worker (its linear
//! memory); stamps the measurement into a run of the same compiler; and holds it against `bench/budgets.json`. CPU
//! times are judged only on a quiet machine ([`load`]).

pub mod load;
pub mod native;
pub mod wasm;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use serde::{Deserialize, Serialize};
use slidock_bench_schema::PairKey;
use slidock_bench_schema::stamp::{
    CompileCost, DeckCost, PerfStamp, PlayerSizes, ScriptSize, WasmSize,
};

use crate::error::{BenchError, Exit, Result};
use crate::exec::{ExecOptions, args, exec_text};

/// `bench/budgets.json`'s format.
pub const BUDGETS_SCHEMA: &str = "slidock-budgets@1";

/// What `--record` adds to a size and to a compile's peak memory, in percent.
pub const HEADROOM_PERCENT: u64 = 5;

/// What `--record` adds to a compile's CPU time, in percent: the same code measured 7 to 17 % more CPU time when
/// other processes held a core, at a load still below the core count ([`load`] refuses to judge above it).
pub const TIME_HEADROOM_PERCENT: u64 = 20;

/// What `--record` adds to a compile's CPU time beyond the 20 %, in milliseconds: three of the wasm leg's 10 ms CPU
/// time steps. The fastest of 5 runs spreads by 20 ms on a small deck, more than its 20 % (16 ms of 80 ms).
pub const TIME_FLOOR_MS: u64 = 30;

/// What `--record` adds to a compile's peak memory beyond the 5 %, in bytes: the largest of 5 native peaks spreads by
/// up to 3 MiB on a small deck on a loaded machine (a 5 % of 1.4 MiB).
pub const PEAK_FLOOR_BYTES: u64 = 8 * 1024 * 1024;

/// The player's build test, whose `BUDGET_BYTES` is the player's gzip budget.
pub const PLAYER_BUILD_TEST: &str = "packages/player/test/build.test.ts";

/// The player's gzip budget as its build test holds it: the one `const BUDGET_BYTES = …;` line, a product of integer
/// literals (`38_144`, `36 * 1024`). Anything else is refused, so `--record` never writes a budget the test does not
/// hold.
pub fn player_gzip_budget(repo: &Path) -> Result<u64> {
    let path = repo.join(PLAYER_BUILD_TEST);
    let text = std::fs::read_to_string(&path).map_err(BenchError::io(&path))?;
    parse_player_gzip_budget(&text)
        .map_err(|why| BenchError::refused(format!("{}: {why}", path.display())))
}

fn parse_player_gzip_budget(text: &str) -> std::result::Result<u64, String> {
    let mut lines = text
        .lines()
        .filter_map(|line| line.trim().strip_prefix("const BUDGET_BYTES = "));
    let (Some(line), None) = (lines.next(), lines.next()) else {
        return Err("one `const BUDGET_BYTES = …;` line expected".into());
    };
    let expression = line
        .strip_suffix(';')
        .ok_or_else(|| format!("`{line}` does not end with `;`"))?;
    let mut product: u64 = 1;
    for factor in expression.split('*').map(str::trim) {
        let digits = factor.replace('_', "");
        if digits.is_empty()
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || factor.starts_with('_')
            || factor.ends_with('_')
        {
            return Err(format!("`{factor}` is not an integer literal"));
        }
        let value: u64 = digits
            .parse()
            .map_err(|_| format!("`{factor}` is out of range"))?;
        product = product
            .checked_mul(value)
            .ok_or_else(|| format!("`{expression}` is out of range"))?;
    }
    if product == 0 {
        return Err("a budget of 0 bytes".into());
    }
    Ok(product)
}

/// Threads that keep cores busy while `--record` measures. A compile's CPU time depends on the clock its cores run
/// at, which is higher on a quiet machine than with every core busy (retro's wasm leg: 15.2 s quiet, 16.0 to 16.3 s
/// on a loaded machine, whatever the load). Recording at the busy clock sets budgets that a check holds either way.
pub struct BusyCores {
    stop: Arc<AtomicBool>,
    threads: Vec<std::thread::JoinHandle<u64>>,
}

impl BusyCores {
    /// The threads `--record` starts: every core but two (the compile and the browser's own threads), at least one.
    pub fn count() -> usize {
        std::thread::available_parallelism()
            .map_or(1, std::num::NonZeroUsize::get)
            .saturating_sub(2)
            .max(1)
    }

    /// Starts `count` threads that spin until [`BusyCores::stop`].
    pub fn start(count: usize) -> BusyCores {
        let stop = Arc::new(AtomicBool::new(false));
        let threads = (0..count)
            .map(|_| {
                let stop = Arc::clone(&stop);
                std::thread::spawn(move || {
                    let mut turns = 0u64;
                    while !stop.load(Ordering::Relaxed) {
                        turns = std::hint::black_box(turns.wrapping_add(1));
                    }
                    turns
                })
            })
            .collect();
        BusyCores { stop, threads }
    }

    /// Stops the threads; how many turns each spun.
    pub fn stop(self) -> Vec<u64> {
        self.stop.store(true, Ordering::Relaxed);
        self.threads
            .into_iter()
            .map(|thread| thread.join().unwrap_or(0))
            .collect()
    }
}

/// The ceilings `budgets --check` holds a measurement to, in the measurement's own shape.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Budgets {
    pub schema: String,
    pub wasm: WasmSize,
    pub player: PlayerSizes,
    pub decks: BTreeMap<PairKey, DeckCost>,
}

impl Budgets {
    /// Reads a budgets file; another schema, or a field it does not know, is refused.
    pub fn load(path: &Path) -> Result<Budgets> {
        let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
        let budgets: Budgets = serde_json::from_str(&text)
            .map_err(|error| BenchError::refused(format!("{}: {error}", path.display())))?;
        if budgets.schema != BUDGETS_SCHEMA {
            return Err(BenchError::refused(format!(
                "{}: schema {:?} is not {BUDGETS_SCHEMA}",
                path.display(),
                budgets.schema
            )));
        }
        Ok(budgets)
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        crate::cache::write_json(path, self)
    }
}

/// `value` with the headroom: `percent` of it rounded up, then `floor` more.
pub fn with_headroom(value: u64, percent: u64, floor: u64) -> u64 {
    let percent = (u128::from(value) * u128::from(percent)).div_ceil(100);
    value
        .saturating_add(u64::try_from(percent).unwrap_or(u64::MAX))
        .saturating_add(floor)
}

/// The budgets a measurement sets: a compile's time plus 20 % and [`TIME_FLOOR_MS`], its peak plus 5 % and
/// [`PEAK_FLOOR_BYTES`], each size plus 5 % (a size does not vary), but the player's gzip sizes at `player_gzip`, the
/// budget its build test holds ([`player_gzip_budget`]), whatever was measured.
pub fn record(perf: &PerfStamp, player_gzip: u64) -> Budgets {
    let cost = |c: &CompileCost| CompileCost {
        ms: with_headroom(c.ms, TIME_HEADROOM_PERCENT, TIME_FLOOR_MS),
        peak_bytes: with_headroom(c.peak_bytes, HEADROOM_PERCENT, PEAK_FLOOR_BYTES),
    };
    let script = |s: &ScriptSize| ScriptSize {
        raw: with_headroom(s.raw, HEADROOM_PERCENT, 0),
        gzip9: player_gzip,
    };
    Budgets {
        schema: BUDGETS_SCHEMA.to_owned(),
        wasm: WasmSize {
            raw: with_headroom(perf.wasm.raw, HEADROOM_PERCENT, 0),
            brotli11: with_headroom(perf.wasm.brotli11, HEADROOM_PERCENT, 0),
        },
        player: PlayerSizes {
            iife: script(&perf.player.iife),
            esm: script(&perf.player.esm),
        },
        decks: perf
            .decks
            .iter()
            .map(|(pair, deck)| {
                (
                    pair.clone(),
                    DeckCost {
                        native: cost(&deck.native),
                        wasm: cost(&deck.wasm),
                    },
                )
            })
            .collect(),
    }
}

/// One measured value against its budget.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Line {
    /// Where it sits in the budgets file: `wasm.brotli11`, `decks.<pair>.native.ms`.
    pub field: String,
    pub measured: Option<u64>,
    pub budget: Option<u64>,
}

impl Line {
    /// Within budget: both known and the measured value at most the budget. A value with no budget, or a budget
    /// whose value was not measured, is over.
    pub fn within(&self) -> bool {
        matches!((self.measured, self.budget), (Some(m), Some(b)) if m <= b)
    }

    /// A compile's CPU time (`….ms`), which only a quiet machine judges.
    pub fn is_time(&self) -> bool {
        self.field.ends_with(".ms")
    }
}

/// What a check concludes from its lines and the machine's load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Judgement {
    /// The fields over their budget, in line order.
    pub over: Vec<String>,
    /// The CPU times left unjudged because the machine was loaded.
    pub unjudged: Vec<String>,
}

impl Judgement {
    /// Every line judged, but CPU times when the machine was not `quiet`: a size or a peak is judged at any load.
    pub fn of(lines: &[Line], quiet: bool) -> Judgement {
        let (judged, unjudged): (Vec<&Line>, Vec<&Line>) =
            lines.iter().partition(|line| quiet || !line.is_time());
        Judgement {
            over: judged
                .into_iter()
                .filter(|line| !line.within())
                .map(|line| line.field.clone())
                .collect(),
            unjudged: unjudged
                .into_iter()
                .map(|line| line.field.clone())
                .collect(),
        }
    }

    /// Exit 1 with a value over, else 3 with times unjudged, else 0.
    pub fn exit(&self) -> Exit {
        if !self.over.is_empty() {
            Exit::Failed
        } else if !self.unjudged.is_empty() {
            Exit::NothingCompared
        } else {
            Exit::Ok
        }
    }
}

fn push_cost(
    lines: &mut Vec<Line>,
    prefix: &str,
    measured: Option<&CompileCost>,
    budget: Option<&CompileCost>,
) {
    lines.push(Line {
        field: format!("{prefix}.ms"),
        measured: measured.map(|c| c.ms),
        budget: budget.map(|c| c.ms),
    });
    lines.push(Line {
        field: format!("{prefix}.peakBytes"),
        measured: measured.map(|c| c.peak_bytes),
        budget: budget.map(|c| c.peak_bytes),
    });
}

/// Every value of the measurement against `budgets`. `selected` names the pairs the measurement was asked for:
/// a budgeted pair outside it is not compared, and one inside it that was not measured is over, as is a measured
/// pair with no budget.
pub fn compare(perf: &PerfStamp, budgets: &Budgets, selected: &[PairKey]) -> Vec<Line> {
    let mut lines = vec![
        Line {
            field: "wasm.raw".into(),
            measured: Some(perf.wasm.raw),
            budget: Some(budgets.wasm.raw),
        },
        Line {
            field: "wasm.brotli11".into(),
            measured: Some(perf.wasm.brotli11),
            budget: Some(budgets.wasm.brotli11),
        },
    ];
    for (name, measured, budget) in [
        ("iife", &perf.player.iife, &budgets.player.iife),
        ("esm", &perf.player.esm, &budgets.player.esm),
    ] {
        lines.push(Line {
            field: format!("player.{name}.raw"),
            measured: Some(measured.raw),
            budget: Some(budget.raw),
        });
        lines.push(Line {
            field: format!("player.{name}.gzip9"),
            measured: Some(measured.gzip9),
            budget: Some(budget.gzip9),
        });
    }
    let mut pairs: Vec<&PairKey> = perf
        .decks
        .keys()
        .chain(budgets.decks.keys().filter(|pair| selected.contains(pair)))
        .collect();
    pairs.sort();
    pairs.dedup();
    for pair in pairs {
        let measured = perf.decks.get(pair);
        let budget = budgets.decks.get(pair);
        push_cost(
            &mut lines,
            &format!("decks.{pair}.native"),
            measured.map(|d| &d.native),
            budget.map(|d| &d.native),
        );
        push_cost(
            &mut lines,
            &format!("decks.{pair}.wasm"),
            measured.map(|d| &d.wasm),
            budget.map(|d| &d.wasm),
        );
    }
    lines
}

/// The size of the wasm module, and its sha256.
pub fn wasm_size(path: &Path) -> Result<(WasmSize, String)> {
    let bytes = std::fs::read(path).map_err(BenchError::io(path))?;
    let raw = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    Ok((
        WasmSize {
            raw,
            brotli11: brotli11(&bytes)?,
        },
        crate::hash::sha256(&bytes),
    ))
}

/// The length of `bytes` compressed with brotli at quality 11 and the encoder's default window (22 bits).
pub fn brotli11(bytes: &[u8]) -> Result<u64> {
    let params = brotli::enc::BrotliEncoderParams {
        quality: 11,
        lgwin: 22,
        ..brotli::enc::BrotliEncoderParams::default()
    };
    let mut input = bytes;
    let mut out = Vec::new();
    brotli::BrotliCompress(&mut input, &mut out, &params)
        .map_err(|error| BenchError::failed(format!("brotli: {error}")))?;
    Ok(u64::try_from(out.len()).unwrap_or(u64::MAX))
}

/// The player's two builds, as `pnpm --filter @slidock/player build` writes them.
pub fn player_files(repo: &Path) -> [PathBuf; 2] {
    let dist = repo.join("packages/player/dist");
    [dist.join("player.iife.js"), dist.join("player.mjs")]
}

/// Measures with Node's zlib at level 9, as the player's build test does (`gzipSync(bytes, { level: 9 })`), so the
/// budget means one number in both places.
const GZIP_SIZES_JS: &str = "const zlib = require('node:zlib'); const fs = require('node:fs'); \
process.stdout.write(JSON.stringify(process.argv.slice(1).map((f) => { const b = fs.readFileSync(f); \
return [b.length, zlib.gzipSync(b, { level: 9 }).length]; })));";

/// The player's sizes (`node` gzips them).
pub async fn player_sizes(repo: &Path) -> Result<PlayerSizes> {
    let [iife, esm] = player_files(repo);
    for file in [&iife, &esm] {
        if !file.is_file() {
            return Err(BenchError::refused(format!(
                "{} is missing: run `pnpm --filter @slidock/player build`",
                file.display()
            )));
        }
    }
    let mut arguments = vec!["-e".to_owned(), GZIP_SIZES_JS.to_owned()];
    arguments.extend([&iife, &esm].map(|f| f.to_string_lossy().into_owned()));
    let text = exec_text(
        "node",
        &args(arguments),
        &ExecOptions::timeout(std::time::Duration::from_secs(60)),
    )
    .await?;
    parse_player_sizes(&text)
}

fn parse_player_sizes(text: &str) -> Result<PlayerSizes> {
    let sizes: Vec<[u64; 2]> = serde_json::from_str(text.trim())
        .map_err(|error| BenchError::failed(format!("player sizes {text:?}: {error}")))?;
    match sizes.as_slice() {
        [[iife_raw, iife_gzip], [esm_raw, esm_gzip]] => Ok(PlayerSizes {
            iife: ScriptSize {
                raw: *iife_raw,
                gzip9: *iife_gzip,
            },
            esm: ScriptSize {
                raw: *esm_raw,
                gzip9: *esm_gzip,
            },
        }),
        _ => Err(BenchError::failed(format!(
            "player sizes: two files expected, got {text:?}"
        ))),
    }
}

/// Of several runs of one compile, the fastest time and the largest peak.
pub fn combine(runs: &[CompileCost]) -> Option<CompileCost> {
    Some(CompileCost {
        ms: runs.iter().map(|c| c.ms).min()?,
        peak_bytes: runs.iter().map(|c| c.peak_bytes).max()?,
    })
}

/// Thousands separated with commas, for the report.
pub fn grouped(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cost(ms: u64, peak_bytes: u64) -> CompileCost {
        CompileCost { ms, peak_bytes }
    }

    fn pair(text: &str) -> PairKey {
        text.parse().unwrap()
    }

    fn perf() -> PerfStamp {
        PerfStamp {
            wasm: WasmSize {
                raw: 1_000,
                brotli11: 300,
            },
            wasm_sha256: slidock_bench_schema::Sha256::new("a".repeat(64)).unwrap(),
            player: PlayerSizes {
                iife: ScriptSize {
                    raw: 100_000,
                    gzip9: 36_000,
                },
                esm: ScriptSize {
                    raw: 99_000,
                    gzip9: 35_900,
                },
            },
            decks: BTreeMap::from([(
                pair("retro:keynote"),
                DeckCost {
                    native: cost(2_000, 400),
                    wasm: cost(5_000, 600),
                },
            )]),
            runs: 3,
            chromium: "148.0.7778.0".into(),
        }
    }

    #[test]
    fn the_headroom_is_a_percentage_rounded_up_then_the_floor() {
        assert_eq!(with_headroom(100, 5, 0), 105);
        assert_eq!(with_headroom(101, 5, 0), 107);
        assert_eq!(with_headroom(0, 5, 0), 0);
        assert_eq!(with_headroom(u64::MAX, 5, 0), u64::MAX);
        assert_eq!(
            with_headroom(u64::MAX / 2, 5, 0),
            u64::MAX / 2 + (u64::MAX / 2).div_ceil(20)
        );
        assert_eq!(with_headroom(100, 5, 30), 135);
        assert_eq!(with_headroom(1_000, 5, 30), 1_080);
        assert_eq!(with_headroom(0, 5, 30), 30);
        assert_eq!(with_headroom(u64::MAX, 5, 30), u64::MAX);
        assert_eq!(with_headroom(100, 20, 30), 150);
        assert_eq!(with_headroom(101, 20, 30), 152);
        assert_eq!(with_headroom(u64::MAX, 20, 30), u64::MAX);
    }

    #[test]
    fn a_decks_time_budget_holds_a_busier_machines_noise() {
        // A large deck's wasm leg can read 2,350 ms in one run and 2,760 in another on the same code (17 % more)
        // with two daemons holding a core: within 20 %. Natively, a small deck's 47 ms reads 53 and its wasm 310
        // reads 330.
        let measured = |native_ms, wasm_ms| PerfStamp {
            decks: BTreeMap::from([(
                pair("pptx-talk:pptx"),
                DeckCost {
                    native: cost(native_ms, 200_000_000),
                    wasm: cost(wasm_ms, 166_000_000),
                },
            )]),
            ..perf()
        };
        let selected = [pair("pptx-talk:pptx")];
        let over = |budgets: &Budgets, native_ms, wasm_ms| -> Vec<String> {
            compare(&measured(native_ms, wasm_ms), budgets, &selected)
                .into_iter()
                .filter(|l| !l.within())
                .map(|l| l.field)
                .collect()
        };
        let budgets = record(&measured(800, 2_350), GZIP);
        assert!(over(&budgets, 936, 2_760).is_empty());
        // Past 20 % and the floor is over.
        assert_eq!(
            over(
                &budgets,
                800 + 160 + TIME_FLOOR_MS + 1,
                2_350 + 470 + TIME_FLOOR_MS + 1
            ),
            [
                "decks.pptx-talk:pptx.native.ms",
                "decks.pptx-talk:pptx.wasm.ms"
            ]
        );
        let small = record(&measured(47, 310), GZIP);
        assert!(over(&small, 53, 330).is_empty());
        // A peak a few MiB over a small deck's 5 %, as one of 5 native runs reads on a loaded machine.
        let mut peak = measured(47, 310);
        if let Some(deck) = peak.decks.get_mut(&pair("pptx-talk:pptx")) {
            deck.native.peak_bytes = 200_000_000 + 10_000_000 + 7 * 1024 * 1024;
        }
        assert!(compare(&peak, &small, &selected).iter().all(Line::within));
        // Memory keeps 5 %: 6 % more and past the floor is over.
        if let Some(deck) = peak.decks.get_mut(&pair("pptx-talk:pptx")) {
            deck.native.peak_bytes = 212_000_000 + PEAK_FLOOR_BYTES + 1;
        }
        assert_eq!(
            compare(&peak, &small, &selected)
                .into_iter()
                .filter(|l| !l.within())
                .map(|l| l.field)
                .collect::<Vec<_>>(),
            ["decks.pptx-talk:pptx.native.peakBytes"]
        );
    }

    #[test]
    fn a_loaded_machine_judges_sizes_and_peaks_but_not_times() {
        let measured = perf();
        let selected = [pair("retro:keynote")];
        let budgets = record(&measured, GZIP);
        let times = [
            "decks.retro:keynote.native.ms",
            "decks.retro:keynote.wasm.ms",
        ];
        // Within budget: 0 when quiet, 3 when loaded (the times unjudged).
        let lines = compare(&measured, &budgets, &selected);
        let quiet = Judgement::of(&lines, true);
        assert_eq!((quiet.over.len(), quiet.unjudged.len()), (0, 0));
        assert_eq!(quiet.exit().code(), 0);
        let loaded = Judgement::of(&lines, false);
        assert!(loaded.over.is_empty());
        assert_eq!(loaded.unjudged, times);
        assert_eq!(loaded.exit().code(), 3);
        // A time over its budget fails on a quiet machine and is left unjudged on a loaded one.
        let mut slow = budgets.clone();
        slow.decks.get_mut(&pair("retro:keynote")).unwrap().wasm.ms = 4_999;
        let lines = compare(&measured, &slow, &selected);
        let quiet = Judgement::of(&lines, true);
        assert_eq!(quiet.over, ["decks.retro:keynote.wasm.ms"]);
        assert_eq!(quiet.exit().code(), 1);
        let loaded = Judgement::of(&lines, false);
        assert!(loaded.over.is_empty());
        assert_eq!(loaded.exit().code(), 3);
        // A size or a peak over its budget fails at any load.
        for lowered in [
            {
                let mut b = budgets.clone();
                b.wasm.raw = 999;
                b
            },
            {
                let mut b = budgets.clone();
                b.decks
                    .get_mut(&pair("retro:keynote"))
                    .unwrap()
                    .native
                    .peak_bytes = 399;
                b
            },
        ] {
            let lines = compare(&measured, &lowered, &selected);
            for quiet in [true, false] {
                let judgement = Judgement::of(&lines, quiet);
                assert_eq!(judgement.over.len(), 1, "{judgement:?}");
                assert_eq!(judgement.exit().code(), 1);
            }
        }
        // A pair measured with no budget is over at any load, through its peaks.
        let mut unbudgeted = budgets;
        unbudgeted.decks.clear();
        let loaded = Judgement::of(&compare(&measured, &unbudgeted, &selected), false);
        assert_eq!(
            loaded.over,
            [
                "decks.retro:keynote.native.peakBytes",
                "decks.retro:keynote.wasm.peakBytes"
            ]
        );
        assert_eq!(loaded.exit().code(), 1);
    }

    /// The player's gzip budget the tests record with.
    const GZIP: u64 = 38_144;

    #[test]
    fn the_players_gzip_budget_is_its_build_tests() {
        let parse = |text: &str| parse_player_gzip_budget(text);
        assert_eq!(parse("const BUDGET_BYTES = 38_144;\n"), Ok(38_144));
        assert_eq!(
            parse("/** doc */\n  const BUDGET_BYTES = 36 * 1024;\nx"),
            Ok(36_864)
        );
        // Anything but a product of integer literals, one line of it, is refused.
        for text in [
            "",
            "const BUDGET = 1;",
            "const BUDGET_BYTES = 36 * KIB;",
            "const BUDGET_BYTES = 0x9000;",
            "const BUDGET_BYTES = 1.5 * 1024;",
            "const BUDGET_BYTES = -1;",
            "const BUDGET_BYTES = _1;",
            "const BUDGET_BYTES = 1_;",
            "const BUDGET_BYTES = 36 *;",
            "const BUDGET_BYTES = 36 * 1024",
            "const BUDGET_BYTES = 0;",
            "const BUDGET_BYTES = 99999999999 * 99999999999;",
            "const BUDGET_BYTES = 1;\nconst BUDGET_BYTES = 2;",
        ] {
            assert!(parse(text).is_err(), "{text:?}");
        }
    }

    #[test]
    fn the_committed_budgets_hold_the_players_build_test_budget() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let budget = player_gzip_budget(&repo).unwrap();
        let budgets = Budgets::load(&repo.join("bench/budgets.json")).unwrap();
        assert_eq!(budgets.player.iife.gzip9, budget);
        assert_eq!(budgets.player.esm.gzip9, budget);
    }

    #[test]
    fn recording_adds_the_headroom_but_keeps_the_players_gzip_budget() {
        let budgets = record(&perf(), GZIP);
        assert_eq!(budgets.schema, BUDGETS_SCHEMA);
        assert_eq!(
            budgets.wasm,
            WasmSize {
                raw: 1_050,
                brotli11: 315
            }
        );
        assert_eq!(
            budgets.player.iife,
            ScriptSize {
                raw: 105_000,
                gzip9: GZIP
            }
        );
        assert_eq!(budgets.player.esm.gzip9, GZIP);
        // A compile's time plus 20 % and its floor, its peak plus 5 % and its floor.
        let retro = budgets.decks.get(&pair("retro:keynote")).unwrap();
        assert_eq!(
            retro.native,
            cost(2_400 + TIME_FLOOR_MS, 420 + PEAK_FLOOR_BYTES)
        );
        assert_eq!(
            retro.wasm,
            cost(6_000 + TIME_FLOOR_MS, 630 + PEAK_FLOOR_BYTES)
        );
    }

    #[test]
    fn a_measurement_is_within_its_own_recorded_budgets_and_over_lowered_ones() {
        let measured = perf();
        let selected = [pair("retro:keynote")];
        let budgets = record(&measured, GZIP);
        let lines = compare(&measured, &budgets, &selected);
        assert_eq!(lines.len(), 2 + 4 + 4);
        assert!(lines.iter().all(Line::within), "{lines:?}");
        // Each value lowered below its measurement is the one line over.
        let mut lowered = budgets.clone();
        lowered.wasm.brotli11 = 299;
        let over: Vec<String> = compare(&measured, &lowered, &selected)
            .into_iter()
            .filter(|l| !l.within())
            .map(|l| l.field)
            .collect();
        assert_eq!(over, ["wasm.brotli11"]);
        let mut lowered = budgets.clone();
        lowered
            .decks
            .get_mut(&pair("retro:keynote"))
            .unwrap()
            .wasm
            .peak_bytes = 599;
        let over: Vec<String> = compare(&measured, &lowered, &selected)
            .into_iter()
            .filter(|l| !l.within())
            .map(|l| l.field)
            .collect();
        assert_eq!(over, ["decks.retro:keynote.wasm.peakBytes"]);
        // Equal is within.
        let mut exact = budgets;
        exact.player.iife.gzip9 = 36_000;
        assert!(
            compare(&measured, &exact, &selected)
                .iter()
                .all(Line::within)
        );
    }

    #[test]
    fn a_pair_on_one_side_only_is_over_and_an_unselected_budget_is_not_compared() {
        let measured = perf();
        let mut budgets = record(&measured, GZIP);
        let selected = [pair("retro:keynote"), pair("keynote-basic:keynote")];
        budgets.decks.insert(
            pair("keynote-basic:keynote"),
            DeckCost {
                native: cost(1, 1),
                wasm: cost(1, 1),
            },
        );
        budgets.decks.insert(
            pair("other:pptx"),
            DeckCost {
                native: cost(1, 1),
                wasm: cost(1, 1),
            },
        );
        let over: Vec<Line> = compare(&measured, &budgets, &selected)
            .into_iter()
            .filter(|l| !l.within())
            .collect();
        assert_eq!(over.len(), 4);
        assert!(
            over.iter().all(
                |l| l.field.starts_with("decks.keynote-basic:keynote.") && l.measured.is_none()
            )
        );
        let mut unbudgeted = record(&measured, GZIP);
        unbudgeted.decks.clear();
        let over: Vec<Line> = compare(&measured, &unbudgeted, &selected[..1])
            .into_iter()
            .filter(|l| !l.within())
            .collect();
        assert_eq!(over.len(), 4);
        assert!(over.iter().all(|l| l.budget.is_none()));
    }

    #[test]
    fn a_budgets_file_of_another_schema_or_with_unknown_fields_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("budgets.json");
        let budgets = record(&perf(), GZIP);
        budgets.save(&path).unwrap();
        assert_eq!(Budgets::load(&path).unwrap(), budgets);
        let text = std::fs::read_to_string(&path).unwrap();
        std::fs::write(&path, text.replace(BUDGETS_SCHEMA, "slidock-budgets@2")).unwrap();
        assert!(
            Budgets::load(&path)
                .unwrap_err()
                .to_string()
                .contains("slidock-budgets@2")
        );
        std::fs::write(
            &path,
            text.replace("\"brotli11\"", "\"extra\": 1,\n    \"brotli11\""),
        )
        .unwrap();
        assert!(Budgets::load(&path).is_err());
    }

    #[test]
    fn busy_cores_spin_until_stopped() {
        assert!(BusyCores::count() >= 1);
        let busy = BusyCores::start(2);
        std::thread::sleep(std::time::Duration::from_millis(50));
        let turns = busy.stop();
        assert_eq!(turns.len(), 2);
        assert!(turns.iter().all(|&t| t > 0), "{turns:?}");
    }

    #[test]
    fn runs_combine_as_the_fastest_time_and_the_largest_peak() {
        assert_eq!(
            combine(&[cost(30, 5), cost(20, 7), cost(25, 6)]),
            Some(cost(20, 7))
        );
        assert_eq!(combine(&[]), None);
    }

    #[test]
    fn brotli_measures_what_a_brotli_decoder_reads_back() {
        let bytes: Vec<u8> = (0..20_000u32).map(|i| (i % 251) as u8).collect();
        let size = brotli11(&bytes).unwrap();
        assert!(size > 0 && size < 1_000, "{size}");
        let params = brotli::enc::BrotliEncoderParams {
            quality: 11,
            lgwin: 22,
            ..brotli::enc::BrotliEncoderParams::default()
        };
        let mut compressed = Vec::new();
        brotli::BrotliCompress(&mut bytes.as_slice(), &mut compressed, &params).unwrap();
        let mut back = Vec::new();
        brotli::BrotliDecompress(&mut compressed.as_slice(), &mut back).unwrap();
        assert_eq!(back, bytes);
        assert_eq!(compressed.len() as u64, size);
    }

    #[test]
    fn player_sizes_are_two_files_raw_and_gzipped() {
        assert_eq!(
            parse_player_sizes("[[10,4],[9,3]]\n").unwrap(),
            PlayerSizes {
                iife: ScriptSize { raw: 10, gzip9: 4 },
                esm: ScriptSize { raw: 9, gzip9: 3 },
            }
        );
        assert!(parse_player_sizes("[[10,4]]").is_err());
        assert!(parse_player_sizes("oops").is_err());
    }

    #[test]
    fn numbers_print_grouped_by_thousands() {
        assert_eq!(grouped(0), "0");
        assert_eq!(grouped(999), "999");
        assert_eq!(grouped(36_864), "36,864");
        assert_eq!(grouped(15_908_913), "15,908,913");
    }
}
