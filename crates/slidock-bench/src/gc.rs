//! The cache's garbage collection (`gc [--keep N]`): run folders are the cache's bulk (a motion or fonts run holds
//! every deck's bundle, media included), so the collector drops old runs and keeps
//!
//! - the run `runs/latest` names, and the run of every baseline file (`<baseline>/*.json`);
//! - per lane, the newest `keep` finished runs (by the time their `results.json` was written), so the host lane's
//!   static run and the shell lane's motion run still find theirs;
//! - a run folder with no `results.json` that changed within [`UNFINISHED_GRACE`] of the newest change in `runs/`
//!   (the run that just finished): a run still being written;
//! - the runs an explain folder names.
//!
//! Every run ends with [`collect_past`]: past [`GC_LIMIT`] of the cache, it collects with [`DEFAULT_KEEP`], then
//! with fewer runs per lane (down to one) until the cache fits. A dropped folder is renamed out of `runs/` first, so a
//! reader never sees half a run under a valid key.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::{Duration, UNIX_EPOCH};

use serde::Deserialize;
use slidock_bench_schema::Lane;
use slidock_bench_schema::stamp::Stamp;

use crate::cache::{CacheLayout, remove};
use crate::commands::explain::EXPLAIN_RUN_FILE;
use crate::error::{BenchError, Result};
use crate::results::{LATEST, results_path};

/// The cache size past which every run collects.
pub const GC_LIMIT: u64 = 20 << 30;
/// Finished runs kept per lane.
pub const DEFAULT_KEEP: usize = 5;
/// An unfinished run folder younger than this is a run in progress.
pub const UNFINISHED_GRACE: Duration = Duration::from_secs(6 * 3600);
/// The prefix a folder takes in `runs/` while it is removed.
const DROPPING: &str = ".gc-";

/// What a collection did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct GcReport {
    /// The run keys removed, oldest first.
    pub removed: Vec<String>,
    /// Bytes on disk before and after.
    pub before: u64,
    pub after: u64,
}

/// A run folder as the collector sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
struct RunFolder {
    key: String,
    /// `None` for a folder with no readable `results.json` (unfinished, or written by another bench).
    lane: Option<Lane>,
    /// When its `results.json` was written, else when the folder last changed, since the Unix epoch.
    time: Duration,
}

/// Bytes a tree takes on disk, each file once (hard links share their blocks); symbolic links are not followed.
pub fn disk_usage(root: &Path) -> u64 {
    let mut seen = BTreeSet::new();
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(path) = stack.pop() {
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&path) {
                stack.extend(entries.filter_map(|e| e.ok()).map(|e| e.path()));
            }
        } else if meta.is_file() {
            let (id, bytes) = on_disk(&meta);
            if id.is_none_or(|id| seen.insert(id)) {
                total = total.saturating_add(bytes);
            }
        }
    }
    total
}

#[cfg(unix)]
fn on_disk(meta: &std::fs::Metadata) -> (Option<(u64, u64)>, u64) {
    use std::os::unix::fs::MetadataExt;
    let id = (meta.nlink() > 1).then(|| (meta.dev(), meta.ino()));
    (id, meta.blocks().saturating_mul(512))
}

#[cfg(not(unix))]
fn on_disk(meta: &std::fs::Metadata) -> (Option<(u64, u64)>, u64) {
    (None, meta.len())
}

fn is_run_key(name: &str) -> bool {
    name.len() == 64
        && name
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

#[derive(Deserialize)]
struct StampOnly {
    stamp: StampLane,
}

#[derive(Deserialize)]
struct StampLane {
    lane: Lane,
}

fn run_folders(runs: &Path) -> Vec<RunFolder> {
    let Ok(entries) = std::fs::read_dir(runs) else {
        return Vec::new();
    };
    let mut folders: Vec<RunFolder> = entries
        .filter_map(|e| e.ok())
        .filter_map(|entry| {
            let key = entry.file_name().to_str()?.to_owned();
            let meta = std::fs::symlink_metadata(entry.path()).ok()?;
            if !meta.is_dir() || !is_run_key(&key) {
                return None;
            }
            let results = results_path(&entry.path());
            let finished = std::fs::read_to_string(&results)
                .ok()
                .and_then(|text| serde_json::from_str::<StampOnly>(&text).ok());
            let time = match &finished {
                Some(_) => modified(&results),
                None => newest_change(&entry.path()),
            }
            .unwrap_or_default();
            Some(RunFolder {
                key,
                lane: finished.map(|f| f.stamp.lane),
                time,
            })
        })
        .collect();
    folders.sort_by(|a, b| a.time.cmp(&b.time).then_with(|| a.key.cmp(&b.key)));
    folders
}

/// When a file or folder last changed, since the Unix epoch.
fn modified(path: &Path) -> Option<Duration> {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()?
        .duration_since(UNIX_EPOCH)
        .ok()
}

/// The latest change in a folder's top level (a run in progress writes there first: `bundles/`, `frames/`, …).
fn newest_change(dir: &Path) -> Option<Duration> {
    let children = std::fs::read_dir(dir)
        .ok()?
        .filter_map(|e| e.ok())
        .filter_map(|e| modified(&e.path()));
    modified(dir).into_iter().chain(children).max()
}

/// The run keys nothing may drop: `runs/latest`'s, each baseline file's, and each explain folder's.
pub fn protected_runs(cache: &CacheLayout, baseline_dir: &Path) -> BTreeSet<String> {
    let mut keys = BTreeSet::new();
    if let Ok(target) = std::fs::read_link(cache.runs.join(LATEST))
        && let Some(name) = target.file_name().and_then(|n| n.to_str())
    {
        keys.insert(name.to_owned());
    }
    #[derive(Deserialize)]
    struct WithStamp {
        stamp: Stamp,
    }
    if let Ok(entries) = std::fs::read_dir(baseline_dir) {
        for path in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
            if path.extension().is_some_and(|x| x == "json")
                && let Ok(text) = std::fs::read_to_string(&path)
                && let Ok(file) = serde_json::from_str::<WithStamp>(&text)
                && let Ok(key) = file.stamp.run_key()
            {
                keys.insert(key.as_str().to_owned());
            }
        }
    }
    if let Ok(entries) = std::fs::read_dir(cache.root.join("explain")) {
        for dir in entries.filter_map(|e| e.ok()).map(|e| e.path()) {
            if let Ok(text) = std::fs::read_to_string(dir.join(EXPLAIN_RUN_FILE)) {
                let key = text.trim();
                if is_run_key(key) {
                    keys.insert(key.to_owned());
                }
            }
        }
    }
    keys
}

/// The run keys a collection keeping `keep` finished runs per lane drops, oldest first.
fn doomed(folders: &[RunFolder], keep: usize, protected: &BTreeSet<String>) -> Vec<String> {
    let now = folders.iter().map(|f| f.time).max().unwrap_or_default();
    let mut kept_per_lane: BTreeMap<Lane, usize> = BTreeMap::new();
    let mut keep_set: BTreeSet<&str> = BTreeSet::new();
    for folder in folders.iter().rev() {
        match folder.lane {
            Some(lane) => {
                let kept = kept_per_lane.entry(lane).or_default();
                if *kept < keep {
                    *kept += 1;
                    keep_set.insert(&folder.key);
                }
            }
            None => {
                let age = now.saturating_sub(folder.time);
                if age < UNFINISHED_GRACE {
                    keep_set.insert(&folder.key);
                }
            }
        }
    }
    folders
        .iter()
        .filter(|f| !keep_set.contains(f.key.as_str()) && !protected.contains(&f.key))
        .map(|f| f.key.clone())
        .collect()
}

/// Removes a run folder: renamed out of the keyed names first, then deleted.
fn drop_run(runs: &Path, key: &str) -> Result<()> {
    let from = runs.join(key);
    let to = runs.join(format!("{DROPPING}{key}"));
    remove(&to)?;
    std::fs::rename(&from, &to).map_err(BenchError::io(&from))?;
    remove(&to)
}

/// Leftovers of an interrupted collection.
fn drop_leftovers(runs: &Path) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(runs) else {
        return Ok(());
    };
    for entry in entries.filter_map(|e| e.ok()) {
        if entry
            .file_name()
            .to_str()
            .is_some_and(|n| n.starts_with(DROPPING))
        {
            remove(&entry.path())?;
        }
    }
    Ok(())
}

/// Collects the cache, keeping `keep` finished runs per lane besides the protected ones.
pub fn collect(cache: &CacheLayout, keep: usize, protected: &BTreeSet<String>) -> Result<GcReport> {
    let before = disk_usage(&cache.root);
    drop_leftovers(&cache.runs)?;
    let removed = doomed(&run_folders(&cache.runs), keep, protected);
    for key in &removed {
        drop_run(&cache.runs, key)?;
    }
    Ok(GcReport {
        removed,
        before,
        after: disk_usage(&cache.root),
    })
}

/// Past `limit`, collects with [`DEFAULT_KEEP`], then fewer runs per lane until the cache fits or one is left per
/// lane; `None` when the cache was within the limit.
pub fn collect_past(
    cache: &CacheLayout,
    limit: u64,
    protected: &BTreeSet<String>,
) -> Result<Option<GcReport>> {
    let before = disk_usage(&cache.root);
    if before <= limit {
        return Ok(None);
    }
    let mut report = GcReport {
        before,
        after: before,
        ..GcReport::default()
    };
    for keep in (1..=DEFAULT_KEEP).rev() {
        let pass = collect(cache, keep, protected)?;
        report.removed.extend(pass.removed);
        report.after = pass.after;
        if report.after <= limit {
            break;
        }
    }
    Ok(Some(report))
}

/// The line a collection prints.
pub fn describe(report: &GcReport) -> String {
    let gib = |bytes: u64| bytes as f64 / f64::from(1u32 << 30);
    format!(
        "gc: {} run(s) removed, cache {:.1} GiB → {:.1} GiB",
        report.removed.len(),
        gib(report.before),
        gib(report.after)
    )
}

/// Collects after a run: the cache's own limit, the protected runs read from disk. A failure is reported, never
/// fatal to the run that just finished.
pub fn after_run(cache: &CacheLayout, baseline_dir: &Path) {
    let protected = protected_runs(cache, baseline_dir);
    match collect_past(cache, GC_LIMIT, &protected) {
        Ok(Some(report)) => {
            eprintln!("{}", describe(&report));
            if report.after > GC_LIMIT {
                eprintln!(
                    "gc: the cache still holds more than {} GiB in what it keeps (latest, baselines, one run per lane)",
                    GC_LIMIT >> 30
                );
            }
        }
        Ok(None) => {}
        Err(error) => eprintln!("gc: {error}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    /// A finished run of `lane` written `age` ago, holding `bytes` of payload.
    fn finished(cache: &CacheLayout, n: u8, lane: &str, age: Duration, bytes: usize) {
        let dir = cache.runs.join(key(n));
        std::fs::create_dir_all(dir.join("frames")).unwrap();
        std::fs::write(dir.join("frames/blob"), vec![7u8; bytes]).unwrap();
        let results = results_path(&dir);
        std::fs::write(&results, format!("{{\"stamp\":{{\"lane\":\"{lane}\"}}}}")).unwrap();
        let time = UNIX_EPOCH + (NOW - age);
        std::fs::File::options()
            .write(true)
            .open(&results)
            .unwrap()
            .set_modified(time)
            .unwrap();
    }

    fn unfinished(cache: &CacheLayout, n: u8, age: Duration) {
        let dir = cache.runs.join(key(n));
        std::fs::create_dir_all(dir.join("bundles")).unwrap();
        let time = UNIX_EPOCH + (NOW - age);
        for path in [dir.join("bundles"), dir.clone()] {
            std::fs::File::open(&path)
                .unwrap()
                .set_modified(time)
                .unwrap();
        }
    }

    fn remaining(cache: &CacheLayout) -> BTreeSet<String> {
        std::fs::read_dir(&cache.runs)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect()
    }

    const HOUR: Duration = Duration::from_secs(3600);
    /// The tests' present: the newest change of each test cache is a run written at most an hour before it.
    const NOW: Duration = Duration::from_secs(1_800_000_000);

    #[test]
    fn keeps_the_newest_runs_per_lane_and_every_protected_run() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CacheLayout::new(dir.path());
        // Seven static runs (1 newest … 7 oldest), three motion runs, one fonts run.
        for n in 1..=7u8 {
            finished(&cache, n, "static", HOUR * u32::from(n), 10);
        }
        for n in 11..=13u8 {
            finished(&cache, n, "motion", HOUR * u32::from(n), 10);
        }
        finished(&cache, 21, "fonts", HOUR * 100, 10);
        // A run in progress, and one abandoned two days ago.
        unfinished(&cache, 30, HOUR);
        unfinished(&cache, 31, HOUR * 48);
        // The oldest static run is the baseline's.
        let protected = BTreeSet::from([key(7)]);
        let report = collect(&cache, 2, &protected).unwrap();
        // Oldest first: the abandoned run (48 h), the third motion run (13 h), static 6, 5, 4, 3.
        assert_eq!(
            report.removed,
            [key(31), key(13), key(6), key(5), key(4), key(3)]
        );
        let left = remaining(&cache);
        for kept in [1u8, 2, 7, 11, 12, 21, 30] {
            assert!(left.contains(&key(kept)), "run {kept} kept");
        }
        assert_eq!(left.len(), 7, "{left:?}");
        assert!(report.after < report.before);
    }

    #[test]
    fn collects_only_past_the_limit_and_down_to_one_run_per_lane() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CacheLayout::new(dir.path());
        for n in 1..=8u8 {
            finished(&cache, n, "motion", HOUR * u32::from(n), 64 * 1024);
        }
        let none = BTreeSet::new();
        let size = disk_usage(&cache.root);
        // Within the limit: nothing happens.
        assert_eq!(collect_past(&cache, size, &none).unwrap(), None);
        assert_eq!(remaining(&cache).len(), 8);
        // Past it, the first pass keeps five; the cache is still too big for a two-run limit, so it goes on.
        let limit = size / 8 * 2 + 4096;
        let report = collect_past(&cache, limit, &none).unwrap().unwrap();
        assert!(report.after <= limit, "{report:?}");
        let left = remaining(&cache);
        assert!(left.contains(&key(1)) && left.contains(&key(2)), "{left:?}");
        assert_eq!(left.len(), 2);
        // The newest run of a lane always stays, whatever the limit.
        let report = collect_past(&cache, 0, &none).unwrap().unwrap();
        assert!(report.after > 0);
        assert_eq!(remaining(&cache), BTreeSet::from([key(1)]));
    }

    #[test]
    fn protects_latest_the_baselines_and_explain_folders() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CacheLayout::new(&dir.path().join("cache"));
        std::fs::create_dir_all(&cache.runs).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(key(3), cache.runs.join(LATEST)).unwrap();
        let explain = cache.root.join("explain/pptx-talk__p1__pptx");
        std::fs::create_dir_all(&explain).unwrap();
        std::fs::write(explain.join("run.txt"), format!("{}\n", key(4))).unwrap();
        std::fs::write(cache.root.join("explain/x"), "not a folder").unwrap();
        let baseline = dir.path().join("baseline");
        std::fs::create_dir_all(&baseline).unwrap();
        std::fs::write(
            baseline.join("results.json"),
            "{\"not\": \"a results file\"}",
        )
        .unwrap();
        let protected = protected_runs(&cache, &baseline);
        #[cfg(unix)]
        assert!(protected.contains(&key(3)));
        assert!(protected.contains(&key(4)));
        // The latest link is never a run folder the collector walks, nor removed.
        finished(&cache, 3, "static", HOUR * 9, 1);
        finished(&cache, 4, "static", HOUR * 8, 1);
        finished(&cache, 5, "static", HOUR, 1);
        let report = collect(&cache, 1, &protected).unwrap();
        assert!(report.removed.is_empty(), "{report:?}");
        #[cfg(unix)]
        assert!(cache.runs.join(LATEST).exists());
    }

    #[test]
    fn hard_links_count_once_and_leftovers_go() {
        let dir = tempfile::tempdir().unwrap();
        let cache = CacheLayout::new(dir.path());
        let a = cache.root.join("media/a.bin");
        std::fs::create_dir_all(a.parent().unwrap()).unwrap();
        std::fs::write(&a, vec![1u8; 256 * 1024]).unwrap();
        let one = disk_usage(&cache.root);
        std::fs::hard_link(&a, cache.root.join("media/b.bin")).unwrap();
        assert_eq!(disk_usage(&cache.root), one);
        let leftover = cache.runs.join(format!("{DROPPING}{}", key(9)));
        std::fs::create_dir_all(&leftover).unwrap();
        collect(&cache, 1, &BTreeSet::new()).unwrap();
        assert!(!leftover.exists());
    }

    #[test]
    fn only_run_keys_are_run_folders() {
        assert!(is_run_key(&key(1)));
        assert!(!is_run_key("latest"));
        assert!(!is_run_key(&"A".repeat(64)));
        assert!(!is_run_key(&format!("{DROPPING}{}", key(1))));
    }
}
