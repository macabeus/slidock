//! The pinned font set. `bench/fonts/fonts.lock.json` names every file the renderer and the compiler may use, with its
//! sha256: redistributable fonts from pinned release URLs, and fonts this Mac holds (Apple and user fonts), copied from
//! their paths. `fonts sync` materialises them as `<cache>/fonts/<sha256>.<ext>`; the same folder is mounted into the
//! renderer and passed to the compiler as `--fonts`, so bindings, metrics and pixels share one set.

use std::collections::BTreeSet;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::cache::{CacheLayout, write_atomic};
use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, args, exec};
use crate::hash::{sha256, sha256_file, sha256_json};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase", deny_unknown_fields)]
pub enum FontSource {
    /// A pinned https URL, or a member of the release archive it names.
    Url {
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        member: Option<String>,
    },
    /// Candidate paths on the reference Mac, `~` for the home folder; the first that exists is used.
    Host { paths: Vec<String> },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontEntry {
    pub family: String,
    pub file: String,
    pub sha256: String,
    pub licence: String,
    pub source: FontSource,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FontLock {
    pub version: u8,
    pub fonts: Vec<FontEntry>,
}

pub fn load_font_lock(path: &Path) -> Result<FontLock> {
    let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
    let lock: FontLock = serde_json::from_str(&text)
        .map_err(|error| BenchError::failed(format!("{}: {error}", path.display())))?;
    let mut problems = Vec::new();
    if lock.version != 1 {
        problems.push("version is not 1".to_owned());
    }
    problems.extend(entry_problems(&lock.fonts));
    if problems.is_empty() {
        Ok(lock)
    } else {
        Err(BenchError::failed(format!(
            "{}: {}",
            path.display(),
            problems.join("; ")
        )))
    }
}

/// What is wrong with lock entries: a file name that is not a font's, a malformed hash, a URL that is not https.
pub fn entry_problems(entries: &[FontEntry]) -> Vec<String> {
    let mut problems = Vec::new();
    for entry in entries {
        let ext = extension(&entry.file);
        if !matches!(ext.as_str(), ".ttf" | ".otf" | ".ttc") || entry.file.contains(['/', '\\']) {
            problems.push(format!("{}: not a font file name", entry.file));
        }
        if !slidock_format::paths::is_sha256(&entry.sha256) {
            problems.push(format!("{}: sha256 is not 64 hex digits", entry.file));
        }
        if let FontSource::Url { url, .. } = &entry.source
            && !url.starts_with("https://")
        {
            problems.push(format!("{}: the URL is not https", entry.file));
        }
    }
    problems
}

/// A file name's extension, lowercase, with its dot.
fn extension(name: &str) -> String {
    name.rfind('.')
        .and_then(|at| name.get(at..))
        .map(str::to_ascii_lowercase)
        .unwrap_or_default()
}

/// The cache file of a font: named by its hash, so every entry and every version of a file has its own name.
pub fn font_cache_name(entry: &FontEntry) -> String {
    format!("{}{}", entry.sha256, extension(&entry.file))
}

/// The identity of a set of font files: the sha256 of their sorted, unique hashes.
pub fn font_set_id<'a>(hashes: impl IntoIterator<Item = &'a str>) -> Result<String> {
    let set: BTreeSet<&str> = hashes.into_iter().collect();
    let sorted: Vec<&str> = set.into_iter().collect();
    sha256_json(&sorted)
}

fn expand_home(path: &str) -> PathBuf {
    match (path.strip_prefix("~/"), std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => PathBuf::from(home).join(rest),
        _ => PathBuf::from(path),
    }
}

/// Downloads a URL once into the archive cache (keyed by the URL) and returns the local path.
async fn download(url: &str, layout: &CacheLayout) -> Result<PathBuf> {
    let path_part = url.split(['?', '#']).next().unwrap_or(url);
    let last = path_part.rsplit('/').next().unwrap_or("");
    let ext = match last.rfind('.') {
        Some(at) if at > 0 => last.get(at..).unwrap_or(".bin").to_owned(),
        _ => ".bin".to_owned(),
    };
    let target = layout.font_archives.join(format!("{}{ext}", sha256(url)));
    if target.exists() {
        return Ok(target);
    }
    let owned = url.to_owned();
    let bytes = tokio::task::spawn_blocking(move || -> Result<Vec<u8>> {
        let mut response = ureq::get(&owned)
            .call()
            .map_err(|error| BenchError::failed(format!("GET {owned}: {error}")))?;
        let mut bytes = Vec::new();
        response
            .body_mut()
            .as_reader()
            .read_to_end(&mut bytes)
            .map_err(|error| BenchError::failed(format!("GET {owned}: {error}")))?;
        Ok(bytes)
    })
    .await
    .map_err(BenchError::failed)??;
    write_atomic(&target, bytes)?;
    Ok(target)
}

/// The bytes of one archive member, as unzip or tar write them to stdout.
async fn extract_member(archive: &Path, member: &str) -> Result<Vec<u8>> {
    let archive_arg = archive.to_string_lossy().into_owned();
    let zip = archive_arg.ends_with(".zip");
    let (tool, arguments) = if zip {
        ("unzip", args(["-p", archive_arg.as_str(), member]))
    } else {
        ("tar", args(["-xOf", archive_arg.as_str(), member]))
    };
    let result = exec(
        tool,
        &arguments,
        &ExecOptions::timeout(Duration::from_secs(120)),
    )
    .await?;
    if !result.ok() || result.stdout.is_empty() {
        return Err(BenchError::failed(format!(
            "{member} is not in {}",
            archive.display()
        )));
    }
    Ok(result.stdout)
}

/// The bytes an entry's source gives today (downloaded, extracted or read from the host).
pub async fn fetch_font(entry: &FontEntry, layout: &CacheLayout) -> Result<Vec<u8>> {
    match &entry.source {
        FontSource::Host { paths } => {
            let path = paths
                .iter()
                .map(|p| expand_home(p))
                .find(|p| p.exists())
                .ok_or_else(|| {
                    BenchError::failed(format!(
                        "{}: none of {} exists on this machine",
                        entry.file,
                        paths.join(", ")
                    ))
                })?;
            std::fs::read(&path).map_err(BenchError::io(&path))
        }
        FontSource::Url { url, member } => {
            let local = download(url, layout).await?;
            match member {
                Some(member) => extract_member(&local, member).await,
                None => std::fs::read(&local).map_err(BenchError::io(&local)),
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct SyncReport {
    pub written: Vec<String>,
    pub present: Vec<String>,
    pub failed: Vec<(String, String)>,
}

/// Materialises every lock entry in `<cache>/fonts`, verifying each hash, and removes the files the lock leaves
/// out.
pub async fn sync_fonts(lock: &FontLock, layout: &CacheLayout) -> Result<SyncReport> {
    std::fs::create_dir_all(&layout.fonts).map_err(BenchError::io(&layout.fonts))?;
    let mut report = SyncReport::default();
    for entry in &lock.fonts {
        let target = layout.fonts.join(font_cache_name(entry));
        if target.exists() && sha256_file(&target)? == entry.sha256 {
            report.present.push(entry.file.clone());
            continue;
        }
        let fetched = fetch_font(entry, layout).await.and_then(|bytes| {
            let actual = sha256(&bytes);
            if actual == entry.sha256 {
                write_atomic(&target, bytes)
            } else {
                Err(BenchError::failed(format!(
                    "sha256 {actual}, the lock says {}",
                    entry.sha256
                )))
            }
        });
        match fetched {
            Ok(()) => report.written.push(entry.file.clone()),
            Err(error) => report.failed.push((entry.file.clone(), error.to_string())),
        }
    }
    let wanted: BTreeSet<String> = lock.fonts.iter().map(font_cache_name).collect();
    for item in std::fs::read_dir(&layout.fonts).map_err(BenchError::io(&layout.fonts))? {
        let item = item.map_err(BenchError::io(&layout.fonts))?;
        let name = item.file_name().to_string_lossy().into_owned();
        if !wanted.contains(&name) {
            crate::cache::remove(&item.path())?;
        }
    }
    Ok(report)
}

/// Re-hashes every entry from its source, for review when a source changed on purpose.
pub async fn relock_fonts(lock: &FontLock, layout: &CacheLayout) -> Result<FontLock> {
    let mut fonts = Vec::new();
    for entry in &lock.fonts {
        let bytes = fetch_font(entry, layout).await?;
        fonts.push(FontEntry {
            sha256: sha256(&bytes),
            ..entry.clone()
        });
    }
    fonts.sort_by(|a, b| slidock_canon::utf16::cmp(&a.file, &b.file));
    Ok(FontLock { version: 1, fonts })
}

/// The cache's font files against the lock (file names carry the hashes; `sync` verified the bytes).
#[derive(Debug)]
pub struct FontSetState {
    pub lock_id: String,
    pub present_id: String,
    pub missing: Vec<String>,
    pub extra: Vec<String>,
}

pub fn font_set_state(lock: &FontLock, layout: &CacheLayout) -> Result<FontSetState> {
    let mut present: Vec<String> = Vec::new();
    if layout.fonts.is_dir() {
        for item in std::fs::read_dir(&layout.fonts).map_err(BenchError::io(&layout.fonts))? {
            let name = item
                .map_err(BenchError::io(&layout.fonts))?
                .file_name()
                .to_string_lossy()
                .into_owned();
            let hashed = name.get(..64).is_some_and(slidock_format::paths::is_sha256)
                && name.get(64..65) == Some(".");
            if hashed {
                present.push(name);
            }
        }
    }
    present.sort();
    let names: BTreeSet<String> = lock.fonts.iter().map(font_cache_name).collect();
    Ok(FontSetState {
        lock_id: font_set_id(lock.fonts.iter().map(|e| e.sha256.as_str()))?,
        present_id: font_set_id(present.iter().filter_map(|name| name.get(..64)))?,
        missing: lock
            .fonts
            .iter()
            .filter(|entry| !present.contains(&font_cache_name(entry)))
            .map(|entry| entry.file.clone())
            .collect(),
        extra: present
            .into_iter()
            .filter(|name| !names.contains(name))
            .collect(),
    })
}

/// Hard-links (or copies, across devices) the synced files into `dir` under their own names: all of them, or only
/// the ones from pinned release URLs. Font scanners skip symbolic links, so these are real files.
pub fn link_fonts(
    lock: &FontLock,
    layout: &CacheLayout,
    dir: &Path,
    releases_only: bool,
) -> Result<Vec<String>> {
    std::fs::create_dir_all(dir).map_err(BenchError::io(dir))?;
    let mut linked = Vec::new();
    for entry in &lock.fonts {
        if releases_only && !matches!(entry.source, FontSource::Url { .. }) {
            continue;
        }
        let source = layout.fonts.join(font_cache_name(entry));
        if !source.exists() {
            return Err(BenchError::refused(format!(
                "{} is not synced; run `slidock-bench fonts sync`",
                entry.file
            )));
        }
        let target = dir.join(&entry.file);
        if std::fs::hard_link(&source, &target).is_err() {
            std::fs::copy(&source, &target).map_err(BenchError::io(&target))?;
        }
        linked.push(entry.file.clone());
    }
    Ok(linked)
}

/// The font set the cache holds. A set that differs from the lock is refused (`font-set-mismatch`) unless drift
/// is allowed, which stamps the results with `fonts.drift`.
pub fn check_font_set(
    lock: &FontLock,
    layout: &CacheLayout,
    allow_drift: bool,
) -> Result<(String, bool)> {
    let state = font_set_state(lock, layout)?;
    let drift = state.present_id != state.lock_id;
    if drift && !allow_drift {
        let missing = if state.missing.is_empty() {
            String::new()
        } else {
            format!(" (missing: {})", state.missing.join(", "))
        };
        return Err(BenchError::refused(format!(
            "font-set-mismatch: the cache's fonts differ from fonts.lock.json{missing}; run `slidock-bench fonts sync`"
        )));
    }
    Ok((state.present_id, drift))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(file: &str, bytes: &str, source: FontSource) -> FontEntry {
        FontEntry {
            family: "F".into(),
            file: file.into(),
            sha256: sha256(bytes),
            licence: "test".into(),
            source,
        }
    }

    #[tokio::test]
    async fn sync_verifies_hashes_and_drops_strays() {
        let dir = tempfile::tempdir().unwrap();
        let host = dir.path().join("Font.TTF");
        std::fs::write(&host, "font bytes").unwrap();
        let layout = CacheLayout::new(&dir.path().join("cache"));
        let good = entry(
            "Font.TTF",
            "font bytes",
            FontSource::Host {
                paths: vec!["/nonexistent".into(), host.to_string_lossy().into_owned()],
            },
        );
        let bad = FontEntry {
            sha256: sha256("other"),
            file: "Bad.otf".into(),
            ..good.clone()
        };
        std::fs::create_dir_all(&layout.fonts).unwrap();
        std::fs::write(layout.fonts.join("stray.ttf"), "x").unwrap();
        let lock = FontLock {
            version: 1,
            fonts: vec![good.clone(), bad],
        };
        let report = sync_fonts(&lock, &layout).await.unwrap();
        assert_eq!(report.written, ["Font.TTF"]);
        assert_eq!(report.failed.len(), 1);
        assert!(layout.fonts.join(format!("{}.ttf", good.sha256)).exists());
        assert!(!layout.fonts.join("stray.ttf").exists());

        let state = font_set_state(&lock, &layout).unwrap();
        assert_eq!(state.missing, ["Bad.otf"]);
        assert_ne!(state.lock_id, state.present_id);
        assert!(check_font_set(&lock, &layout, false).is_err());
        assert!(check_font_set(&lock, &layout, true).unwrap().1);
        let whole = FontLock {
            version: 1,
            fonts: vec![good],
        };
        assert!(!check_font_set(&whole, &layout, false).unwrap().1);
    }

    #[test]
    fn the_font_set_id_is_the_hash_of_the_sorted_hashes() {
        let id = font_set_id(["b", "a", "b"]).unwrap();
        assert_eq!(id, sha256("[\"a\",\"b\"]"));
    }

    #[test]
    fn the_lock_names_font_files_with_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lock.json");
        let lock = FontLock {
            version: 1,
            fonts: vec![entry(
                "a/b.ttf",
                "x",
                FontSource::Url {
                    url: "http://insecure".into(),
                    member: None,
                },
            )],
        };
        std::fs::write(&path, serde_json::to_string(&lock).unwrap()).unwrap();
        let error = load_font_lock(&path).unwrap_err().to_string();
        assert!(
            error.contains("not a font file name") && error.contains("not https"),
            "{error}"
        );
    }

    #[test]
    fn reads_the_committed_lock() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../bench/fonts/fonts.lock.json");
        let lock = load_font_lock(&path).unwrap();
        assert_eq!(lock.fonts.len(), 63);
        // The pinned font set's id.
        assert_eq!(
            font_set_id(lock.fonts.iter().map(|e| e.sha256.as_str())).unwrap(),
            "f1dc33e59980dc1939f6d2b0d841c2f3b2de1e8fb33e83eec102ce2bbeaa1771"
        );
    }
}
