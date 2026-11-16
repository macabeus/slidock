//! `lo-roundtrip`: what LibreOffice keeps of the motion probes. Each probe deck of `bench/motion/probes.json` is
//! opened and saved by LibreOffice headless (`soffice --convert-to`, in the pinned `slidock-probes-lo` image built
//! from `probes/fonts/Dockerfile`, no network) as ODP and as PPTX; the four results and the two sources are bundled
//! by the CLI (the deck path, Google Fonts off), and each transition and step of a result is compared with the
//! source's at the same place (the same slide, the same step of it): kept (effect, params, trigger, delivery and
//! duration equal), changed (the same effect, another field moved), replaced (another effect, or the fallback), or
//! dropped; a step the source does not have is added. The report is `bench/motion/lo-roundtrip.json`, keyed by the
//! probes' sha256: `lo-roundtrip --check` (and a test of the committed file) fails when it is not the report of the
//! probes `bench/motion/probes.json` pins.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use slidock_bench_schema::Frontend;

use crate::cache::{remove, write_json};
use crate::compile::{CompileStatus, compiler_binary, run_compiler};
use crate::error::{BenchError, Exit, Result};
use crate::exec::{ExecOptions, args, exec};
use crate::hash::sha256_file;
use crate::motion_lane::deck::{DeckSlide, DeckStep, DeckTransition, MotionDeck, read_deck};
use crate::motion_lane::verdict::{MotionProbe, PROBES, load_probes};
use crate::setup::Context;

/// The LibreOffice image (LibreOffice 25.2.3.2 from Debian trixie at a pinned snapshot).
pub const LO_IMAGE: &str = "slidock-probes-lo:trixie-20261030";
/// Its recipe, relative to the repository.
const LO_RECIPE: &str = "probes/fonts";
/// The report, relative to the repository.
pub const REPORT: &str = "bench/motion/lo-roundtrip.json";
/// The report's version.
pub const REPORT_VERSION: u8 = 3;
const CONVERT_TIMEOUT: Duration = Duration::from_secs(600);

/// One item (a transition or a step) as the comparison sees it.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Shape {
    /// The vocabulary effect, or `fallback:<name>` when the deck names none.
    pub effect: String,
    pub native: String,
    /// As compact JSON: a path's commands stay on one line of the report.
    #[serde(serialize_with = "compact")]
    pub params: Value,
    pub trigger: Option<String>,
    pub delivery: Option<String>,
    pub duration_ms: Value,
}

fn compact<S: serde::Serializer>(
    value: &Value,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

fn effect_of(effect: &Option<String>, fallback: &str) -> String {
    effect
        .clone()
        .unwrap_or_else(|| format!("fallback:{fallback}"))
}

fn step_shape(step: &DeckStep) -> Shape {
    Shape {
        effect: effect_of(&step.effect, &step.fallback),
        native: step.native.clone(),
        params: step.params.clone().unwrap_or(Value::Null),
        trigger: Some(step.trigger.clone()),
        delivery: step.delivery.clone(),
        duration_ms: step.duration_ms.clone(),
    }
}

fn transition_shape(transition: &DeckTransition) -> Shape {
    Shape {
        effect: effect_of(&transition.effect, &transition.fallback),
        native: transition.native.clone(),
        params: transition.params.clone().unwrap_or(Value::Null),
        trigger: None,
        delivery: None,
        duration_ms: transition.duration_ms.clone(),
    }
}

/// What became of an item.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Fate {
    Kept,
    Changed,
    Replaced,
    Dropped,
    Added,
}

/// One item of one round trip.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ItemFate {
    pub slide: usize,
    /// `transition` or `step <n>`.
    pub item: String,
    pub fate: Fate,
    /// The source's item (`None` for an added one).
    pub source: Option<Shape>,
    /// LibreOffice's (`None` for a dropped one).
    pub result: Option<Shape>,
    /// The fields that moved, for a changed item, comma-separated.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub moved: String,
}

/// The vocabulary's random build and transition (LibreOffice's `ooo-entrance-random`, `ooo-exit-random`, `random`).
const RANDOM: &str = "random";

/// Compares two items at the same place. `same_format` also holds the native id; a random effect's duration is not
/// compared.
pub fn fate(
    source: Option<&Shape>,
    result: Option<&Shape>,
    same_format: bool,
) -> (Fate, Vec<String>) {
    match (source, result) {
        (None, None) => (Fate::Kept, Vec::new()),
        (Some(_), None) => (Fate::Dropped, Vec::new()),
        (None, Some(_)) => (Fate::Added, Vec::new()),
        (Some(a), Some(b)) if a.effect != b.effect => (Fate::Replaced, Vec::new()),
        (Some(a), Some(b)) => {
            let mut moved = Vec::new();
            if a.params != b.params {
                moved.push("params".to_owned());
            }
            if a.trigger != b.trigger {
                moved.push("trigger".to_owned());
            }
            if a.delivery != b.delivery {
                moved.push("delivery".to_owned());
            }
            // LibreOffice plays a random effect as a preset it picks each time it loads the deck, with that
            // preset's duration: the duration it saves is its pick, not a fate.
            if a.duration_ms != b.duration_ms && a.effect != RANDOM {
                moved.push("durationMs".to_owned());
            }
            if same_format && a.native != b.native {
                moved.push("native".to_owned());
            }
            if moved.is_empty() {
                (Fate::Kept, moved)
            } else {
                (Fate::Changed, moved)
            }
        }
    }
}

/// Every item of a result deck against the source deck, slide by slide.
pub fn compare(source: &MotionDeck, result: &MotionDeck, same_format: bool) -> Vec<ItemFate> {
    let mut out = Vec::new();
    let empty = DeckSlide {
        id: String::new(),
        hidden: false,
        timeline: Vec::new(),
        transition: None,
        match_keys: None,
    };
    let count = source.slides.len().max(result.slides.len());
    for index in 0..count {
        let a = source.slides.get(index).unwrap_or(&empty);
        let b = result.slides.get(index).unwrap_or(&empty);
        let (ta, tb) = (
            a.transition.as_ref().map(transition_shape),
            b.transition.as_ref().map(transition_shape),
        );
        if ta.is_some() || tb.is_some() {
            let (fate, moved) = fate(ta.as_ref(), tb.as_ref(), same_format);
            out.push(ItemFate {
                slide: index,
                item: "transition".to_owned(),
                fate,
                source: ta,
                result: tb,
                moved: moved.join(","),
            });
        }
        for n in 0..a.timeline.len().max(b.timeline.len()) {
            let (sa, sb) = (
                a.timeline.get(n).map(step_shape),
                b.timeline.get(n).map(step_shape),
            );
            let (fate, moved) = fate(sa.as_ref(), sb.as_ref(), same_format);
            out.push(ItemFate {
                slide: index,
                item: format!("step {n}"),
                fate,
                source: sa,
                result: sb,
                moved: moved.join(","),
            });
        }
    }
    out
}

/// Per source effect, how many items met each fate.
pub fn by_effect(items: &[ItemFate]) -> BTreeMap<String, BTreeMap<Fate, u32>> {
    let mut out: BTreeMap<String, BTreeMap<Fate, u32>> = BTreeMap::new();
    for item in items {
        let effect = item
            .source
            .as_ref()
            .or(item.result.as_ref())
            .map_or_else(String::new, |s| s.effect.clone());
        *out.entry(effect).or_default().entry(item.fate).or_default() += 1;
    }
    out
}

/// The slide counts of a source and of what LibreOffice saved.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub struct Slides {
    pub source: usize,
    pub result: usize,
}

/// One round trip: a source deck saved by LibreOffice in a format. What LibreOffice wrote is not pinned: it stamps its
/// save time in the file, so its sha256 differs on every run.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Trip {
    pub source: String,
    pub source_sha256: String,
    pub saved_as: Frontend,
    pub slides: Slides,
    pub totals: BTreeMap<Fate, u32>,
    pub by_effect: BTreeMap<String, BTreeMap<Fate, u32>>,
    /// Every item LibreOffice did not keep as it was.
    pub items: Vec<ItemFate>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Report {
    pub version: u8,
    /// The probes it is the report of: each deck's sha256 by `<id>:<frontend>`.
    pub probes: BTreeMap<String, String>,
    pub image: String,
    pub image_id: String,
    pub libreoffice: String,
    pub trips: Vec<Trip>,
}

/// A probe's key in the report: `<id>:<frontend>`.
fn probe_key(probe: &MotionProbe) -> String {
    format!("{}:{}", probe.id, probe.frontend)
}

/// A round trip's source name in the report: `<id>-<frontend>`.
fn source_name(probe: &MotionProbe) -> String {
    format!("{}-{}", probe.id, probe.frontend)
}

/// What the staleness check reads of a report.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReportKeys {
    version: u8,
    #[serde(default)]
    probes: BTreeMap<String, String>,
    trips: Vec<TripKey>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct TripKey {
    source: String,
    source_sha256: String,
    saved_as: Frontend,
}

/// Why a report (its text) is not the one of `probes`: its version, the probes it names, or a round trip of another
/// source or a missing one. Empty when it is current.
pub fn stale(report: &str, probes: &[MotionProbe]) -> Vec<String> {
    let keys: ReportKeys = match serde_json::from_str(report) {
        Ok(keys) => keys,
        Err(error) => return vec![format!("it does not read: {error}")],
    };
    let mut out = Vec::new();
    if keys.version != REPORT_VERSION {
        out.push(format!("version {}, not {REPORT_VERSION}", keys.version));
    }
    let wanted: BTreeMap<String, String> = probes
        .iter()
        .map(|p| (probe_key(p), p.sha256.to_string()))
        .collect();
    for (key, sha) in &wanted {
        match keys.probes.get(key) {
            Some(have) if have == sha => {}
            Some(have) => out.push(format!(
                "{key}: the report is of {have}, the probe is {sha}"
            )),
            None => out.push(format!("{key}: not in the report")),
        }
    }
    for key in keys.probes.keys().filter(|k| !wanted.contains_key(*k)) {
        out.push(format!("{key}: in the report, not a probe"));
    }
    for probe in probes {
        let name = source_name(probe);
        for saved_as in [Frontend::Odp, Frontend::Pptx] {
            let trips: Vec<&TripKey> = keys
                .trips
                .iter()
                .filter(|t| t.source == name && t.saved_as == saved_as)
                .collect();
            match trips.as_slice() {
                [trip] if trip.source_sha256 == probe.sha256.as_str() => {}
                [trip] => out.push(format!(
                    "{name} saved as {saved_as}: a round trip of {}, the probe is {}",
                    trip.source_sha256, probe.sha256
                )),
                [] => out.push(format!("{name} saved as {saved_as}: no round trip")),
                _ => out.push(format!(
                    "{name} saved as {saved_as}: more than one round trip"
                )),
            }
        }
    }
    let names: Vec<String> = probes.iter().map(source_name).collect();
    for trip in keys.trips.iter().filter(|t| !names.contains(&t.source)) {
        out.push(format!("{}: a round trip of no probe", trip.source));
    }
    out
}

/// `lo-roundtrip --check`: the committed report against the probes `bench/motion/probes.json` pins.
pub fn check(ctx: &Context) -> Result<Exit> {
    let repo = &ctx.env.repo_root;
    let probes = load_probes(&repo.join(PROBES))?.probes;
    let path = repo.join(REPORT);
    let text = std::fs::read_to_string(&path).map_err(BenchError::io(&path))?;
    let problems = stale(&text, &probes);
    for problem in &problems {
        println!("stale      {REPORT}: {problem}");
    }
    if problems.is_empty() {
        println!("{REPORT}: the report of the current probes");
    } else {
        println!("run `slidock-bench lo-roundtrip` (Docker) to write it again");
    }
    Ok(Exit::check(problems.is_empty()))
}

async fn docker(arguments: &[&str], timeout: Duration) -> Result<crate::exec::ExecResult> {
    exec(
        "docker",
        &args(arguments.iter().copied()),
        &ExecOptions::timeout(timeout),
    )
    .await
}

/// The image's id, built from its recipe when it is missing.
async fn ensure_image(repo: &Path) -> Result<String> {
    let inspect = |_: ()| {
        docker(
            &["image", "inspect", "--format", "{{.Id}}", LO_IMAGE],
            Duration::from_secs(60),
        )
    };
    let mut found = inspect(()).await?;
    if !found.ok() {
        let recipe = repo.join(LO_RECIPE);
        let recipe = recipe.to_string_lossy();
        let built = docker(
            &["build", "-t", LO_IMAGE, &recipe],
            Duration::from_secs(1800),
        )
        .await?;
        if !built.ok() {
            return Err(BenchError::failed(format!(
                "docker build {LO_IMAGE}: {}",
                built.stderr.trim()
            )));
        }
        found = inspect(()).await?;
    }
    Ok(String::from_utf8_lossy(&found.stdout).trim().to_owned())
}

pub async fn lo_roundtrip(ctx: &Context) -> Result<Exit> {
    let repo = &ctx.env.repo_root;
    let probes = load_probes(&repo.join(PROBES))?.probes;
    let image_id = ensure_image(repo).await?;
    let work = ctx.cache.root.join("lo-roundtrip");
    remove(&work)?;
    let input = work.join("in");
    std::fs::create_dir_all(&input).map_err(BenchError::io(&input))?;
    let mut sources: Vec<(String, Frontend, PathBuf, String)> = Vec::new();
    for probe in &probes {
        let file = repo.join(&probe.file);
        let sha = sha256_file(&file)?;
        if sha != probe.sha256.as_str() {
            return Err(BenchError::refused(format!(
                "{}: sha256 {sha}, {PROBES} says {}",
                probe.file, probe.sha256
            )));
        }
        let name = source_name(probe);
        let copy = input.join(format!("{name}.{}", probe.frontend.as_str()));
        std::fs::copy(&file, &copy).map_err(BenchError::io(&file))?;
        sources.push((name, probe.frontend, copy, sha));
    }
    let mount = format!("{}:/work", work.display());
    let script = "soffice --version > /work/version.txt 2>/dev/null; cd /work/in && for to in odp pptx; do \
                  mkdir -p /work/out/$to && soffice --headless --norestore --convert-to $to --outdir /work/out/$to *; done";
    let converted = docker(
        &[
            "run",
            "--rm",
            "--network",
            "none",
            "-v",
            &mount,
            "-e",
            "HOME=/tmp/home",
            LO_IMAGE,
            "sh",
            "-c",
            script,
        ],
        CONVERT_TIMEOUT,
    )
    .await?;
    if !converted.ok() {
        return Err(BenchError::failed(format!(
            "soffice: {}",
            converted.stderr.trim()
        )));
    }
    let libreoffice = std::fs::read_to_string(work.join("version.txt"))
        .unwrap_or_default()
        .trim()
        .to_owned();
    let binary = compiler_binary(repo, &ctx.env.target_dir, ctx.env.compiler.as_deref()).await?;
    let bundle = |file: PathBuf, frontend: Frontend, name: String| {
        let binary = binary.clone();
        let out = work.join("bundles").join(name);
        async move {
            let arguments = vec![
                "compile".to_owned(),
                file.to_string_lossy().into_owned(),
                "--frontend".to_owned(),
                frontend.to_string(),
                "--out".to_owned(),
                out.to_string_lossy().into_owned(),
                "--no-google".to_owned(),
            ];
            let outcome = run_compiler(&binary, &arguments, CONVERT_TIMEOUT, true).await?;
            if !matches!(outcome.status, CompileStatus::Ok | CompileStatus::Errors) {
                return Err(BenchError::failed(format!(
                    "{} did not bundle ({:?})",
                    file.display(),
                    outcome.status
                )));
            }
            read_deck(&out)
        }
    };
    let mut trips = Vec::new();
    for (name, frontend, copy, sha) in &sources {
        let source = bundle(copy.clone(), *frontend, name.clone()).await?;
        for saved_as in [Frontend::Odp, Frontend::Pptx] {
            let file = work
                .join("out")
                .join(saved_as.as_str())
                .join(format!("{name}.{}", saved_as.as_str()));
            if !file.is_file() {
                return Err(BenchError::failed(format!(
                    "LibreOffice wrote no {}",
                    file.display()
                )));
            }
            let result = bundle(file.clone(), saved_as, format!("{name}-as-{saved_as}")).await?;
            let items = compare(&source, &result, saved_as == *frontend);
            let mut totals: BTreeMap<Fate, u32> = BTreeMap::new();
            for item in &items {
                *totals.entry(item.fate).or_default() += 1;
            }
            println!(
                "{name} saved as {saved_as}: {} slides of {}; {}",
                result.slides.len(),
                source.slides.len(),
                totals
                    .iter()
                    .map(|(fate, n)| format!("{fate:?} {n}").to_lowercase())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            trips.push(Trip {
                source: name.clone(),
                source_sha256: sha.clone(),
                saved_as,
                slides: Slides {
                    source: source.slides.len(),
                    result: result.slides.len(),
                },
                totals,
                by_effect: by_effect(&items),
                items: items.into_iter().filter(|i| i.fate != Fate::Kept).collect(),
            });
        }
    }
    let report = Report {
        version: REPORT_VERSION,
        probes: probes
            .iter()
            .map(|p| (probe_key(p), p.sha256.to_string()))
            .collect(),
        image: LO_IMAGE.to_owned(),
        image_id,
        libreoffice,
        trips,
    };
    let path = repo.join(REPORT);
    write_json(&path, &report)?;
    remove(&work)?;
    println!("report: {}", path.display());
    Ok(Exit::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(effect: &str, params: Value, duration: u32) -> Shape {
        Shape {
            effect: effect.into(),
            native: format!("pptx:{effect}"),
            params,
            trigger: Some("click".into()),
            delivery: Some("all".into()),
            duration_ms: duration.into(),
        }
    }

    #[test]
    fn each_item_is_kept_changed_replaced_dropped_or_added() {
        let fly = shape("flyIn", serde_json::json!({"dir": "u"}), 500);
        assert_eq!(fate(Some(&fly), Some(&fly), true), (Fate::Kept, vec![]));
        let slower = shape("flyIn", serde_json::json!({"dir": "u"}), 1000);
        assert_eq!(
            fate(Some(&fly), Some(&slower), true),
            (Fate::Changed, vec!["durationMs".to_owned()])
        );
        let turned = shape("flyIn", serde_json::json!({"dir": "d"}), 500);
        assert_eq!(fate(Some(&fly), Some(&turned), false).0, Fate::Changed);
        let fade = shape("fade", Value::Null, 500);
        assert_eq!(fate(Some(&fly), Some(&fade), true).0, Fate::Replaced);
        assert_eq!(fate(Some(&fly), None, true).0, Fate::Dropped);
        assert_eq!(fate(None, Some(&fly), true).0, Fate::Added);
        // A random effect's duration is LibreOffice's pick of a preset, which differs from load to load.
        let random = shape("random", Value::Null, 500);
        let picked = shape("random", Value::Null, 2000);
        assert_eq!(
            fate(Some(&random), Some(&picked), true),
            (Fate::Kept, vec![])
        );
        let mut later = picked.clone();
        later.trigger = Some("afterPrevious".into());
        assert_eq!(
            fate(Some(&random), Some(&later), true),
            (Fate::Changed, vec!["trigger".to_owned()])
        );
        // Across formats the native ids differ by design; within one they must not.
        let mut odp = fly.clone();
        odp.native = "odp:ooo-entrance-fly-in".into();
        assert_eq!(fate(Some(&fly), Some(&odp), false).0, Fate::Kept);
        assert_eq!(
            fate(Some(&fly), Some(&odp), true),
            (Fate::Changed, vec!["native".to_owned()])
        );
    }

    fn probe(frontend: Frontend, sha: char) -> MotionProbe {
        serde_json::from_value(serde_json::json!({
            "id": "motion-probe", "frontend": frontend, "file": "probes/out/x", "sha256": sha.to_string().repeat(64),
            "expect": "probes/out/x.expect.json", "deck": "x"
        }))
        .unwrap()
    }

    fn report(probes: &[MotionProbe]) -> String {
        let trips: Vec<Value> = probes
            .iter()
            .flat_map(|p| {
                [Frontend::Odp, Frontend::Pptx].map(|saved_as| {
                    serde_json::json!({"source": source_name(p), "sourceSha256": p.sha256, "savedAs": saved_as})
                })
            })
            .collect();
        let keys: BTreeMap<String, String> = probes
            .iter()
            .map(|p| (probe_key(p), p.sha256.to_string()))
            .collect();
        serde_json::json!({"version": REPORT_VERSION, "probes": keys, "trips": trips}).to_string()
    }

    #[test]
    fn a_report_of_other_probes_is_stale() {
        let probes = [probe(Frontend::Pptx, 'a'), probe(Frontend::Odp, 'b')];
        assert_eq!(stale(&report(&probes), &probes), Vec::<String>::new());
        // The probes were regenerated since the report.
        let now = [probe(Frontend::Pptx, 'c'), probe(Frontend::Odp, 'b')];
        let problems = stale(&report(&probes), &now);
        assert_eq!(
            problems,
            [
                format!(
                    "motion-probe:pptx: the report is of {}, the probe is {}",
                    "a".repeat(64),
                    "c".repeat(64)
                ),
                format!(
                    "motion-probe-pptx saved as odp: a round trip of {}, the probe is {}",
                    "a".repeat(64),
                    "c".repeat(64)
                ),
                format!(
                    "motion-probe-pptx saved as pptx: a round trip of {}, the probe is {}",
                    "a".repeat(64),
                    "c".repeat(64)
                ),
            ]
        );
        // A probe the report lacks, a report of an earlier version (made by another rule), and text that does not
        // read.
        let one = [probe(Frontend::Pptx, 'a')];
        assert_eq!(stale(&report(&one), &probes).len(), 3);
        let old = report(&probes).replace("\"version\":3", "\"version\":2");
        assert_eq!(stale(&old, &probes), ["version 2, not 3"]);
        assert!(stale("{", &probes)[0].starts_with("it does not read"));
    }

    #[test]
    fn the_committed_report_is_of_the_current_probes() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let probes = load_probes(&repo.join(PROBES)).unwrap().probes;
        let text = std::fs::read_to_string(repo.join(REPORT)).unwrap();
        let problems = stale(&text, &probes);
        assert!(
            problems.is_empty(),
            "{REPORT} is stale (run `slidock-bench lo-roundtrip`): {problems:?}"
        );
    }

    #[test]
    fn counts_fates_per_source_effect() {
        let fly = shape("flyIn", Value::Null, 500);
        let item = |fate, source: Option<&Shape>, result: Option<&Shape>| ItemFate {
            slide: 0,
            item: "step 0".into(),
            fate,
            source: source.cloned(),
            result: result.cloned(),
            moved: String::new(),
        };
        let counts = by_effect(&[
            item(Fate::Kept, Some(&fly), Some(&fly)),
            item(Fate::Dropped, Some(&fly), None),
            item(Fate::Added, None, Some(&fly)),
        ]);
        assert_eq!(counts["flyIn"][&Fate::Kept], 1);
        assert_eq!(counts["flyIn"][&Fate::Dropped], 1);
        assert_eq!(counts["flyIn"][&Fate::Added], 1);
    }
}
