//! The fonts lane's pinned files (`bench/fonts/fonts-lane.lock.json`): Google Fonts as the CLI fetches it (the
//! offline stand-in for live Google), the bundled substitutes (the CLI's own pins), and the renderer's one last-resort
//! face. None is a host font, so nothing the lane ships comes from this computer.
//!
//! The Google part is what the CLI's deck path asked `css2` for when the lock was recorded ([`super::record`]): each
//! `css2` response as served to the CLI (one static TrueType file per face), and every `fonts.gstatic.com` file those
//! responses name, each with its sha256. The CLI looks a URL up in its cache before fetching it, under
//! `<SLIDOCK_CACHE_DIR>/google/<sha256 of the URL>`, so a cache holding these answers serves the compiler's own `css2`
//! plan with the faces live Google would: the faces come in as `google`, ranked as Google's.
//!
//! `prepare` materialises them under `<cache>/fonts-lane/`:
//!
//! ```text
//! files/<sha256>.<ext>          every file, each hash checked
//! cli-cache/google/<sha>        each css2 response and gstatic file, named by the sha256 of its URL
//! cli-cache/bundled/<sha>.ttf   the substitutes, where the CLI's cache keeps them
//! fallback/<file>               the renderer's /fonts
//! ```

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use slidock_bench_schema::lanes::FontSource as LaneSource;
use slidock_fonts::google::{CSS2_ENDPOINT, google_catalog, parse_css2};

use crate::cache::{CacheLayout, remove, write_atomic};
use crate::error::{BenchError, Result};
use crate::fonts::{
    FontEntry, FontSource, entry_problems, fetch_font, font_cache_name, font_set_id,
};
use crate::hash::{sha256, sha256_file};

/// The lock's path in the repository.
pub const LANE_LOCK: &str = "bench/fonts/fonts-lane.lock.json";
/// `fonts-lane.lock.json`'s version.
pub const LANE_LOCK_VERSION: u8 = 2;
/// Where every font file a `css2` response names lives.
pub const GSTATIC: &str = "https://fonts.gstatic.com/s/";
/// The folders of `<cache>/fonts-lane/`; `prepare` removes anything else.
const LAYOUT: [&str; 5] = ["files", "cli-cache", "fallback", "work", "profile"];

/// A `css2` request the CLI made, and the response it got.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Css2Response {
    pub url: String,
    /// Of `css`.
    pub sha256: String,
    pub css: String,
}

/// Google Fonts as the CLI fetched it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GoogleLock {
    pub css2: Vec<Css2Response>,
    /// Every file the responses name, at its `fonts.gstatic.com` URL.
    pub files: Vec<FontEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LaneLock {
    pub version: u8,
    pub google: GoogleLock,
    /// The bundled substitutes.
    pub substitutes: Vec<FontEntry>,
    /// The renderer's last-resort face: exactly one.
    pub fallback: Vec<FontEntry>,
}

impl LaneLock {
    fn entries(&self) -> impl Iterator<Item = (FontEntry, LaneRole)> + '_ {
        self.google
            .files
            .iter()
            .map(|e| (e.clone(), LaneRole::Google))
            .chain(
                self.substitutes
                    .iter()
                    .map(|e| (e.clone(), LaneRole::Substitute)),
            )
            .chain(
                self.fallback
                    .iter()
                    .map(|e| (e.clone(), LaneRole::Fallback)),
            )
    }
}

/// What a lane file is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaneRole {
    Google,
    Substitute,
    Fallback,
}

/// An entry's pinned URL, when it has one and no archive member.
fn plain_url(entry: &FontEntry) -> Option<&str> {
    match &entry.source {
        FontSource::Url { url, member: None } => Some(url),
        _ => None,
    }
}

/// What is wrong with the Google part: every response must read and match its hash, and the files must be exactly
/// the ones the responses name, each with the family the response gives and that family's catalog licence.
pub fn google_problems(google: &GoogleLock) -> Vec<String> {
    let mut problems = Vec::new();
    let mut named: BTreeMap<String, String> = BTreeMap::new();
    let mut urls = BTreeSet::new();
    for response in &google.css2 {
        let url = &response.url;
        if !url.starts_with(&format!("{CSS2_ENDPOINT}?")) {
            problems.push(format!("{url}: not a css2 request"));
        }
        if !urls.insert(url.as_str()) {
            problems.push(format!("{url}: listed twice"));
        }
        if sha256(&response.css) != response.sha256 {
            problems.push(format!("{url}: the response does not match its sha256"));
        }
        match parse_css2(&response.css) {
            Ok(faces) => {
                for face in faces {
                    named.insert(face.url, face.family);
                }
            }
            Err(error) => problems.push(format!("{url}: {error}")),
        }
    }
    let mut files = BTreeSet::new();
    for entry in &google.files {
        let Some(url) = plain_url(entry) else {
            problems.push(format!("{}: not a pinned URL", entry.file));
            continue;
        };
        if !url.starts_with(GSTATIC) {
            problems.push(format!("{url}: not on fonts.gstatic.com"));
        }
        files.insert(url);
        match named.get(url) {
            None => problems.push(format!("{url}: no css2 response names it")),
            Some(family) if *family != entry.family => problems.push(format!(
                "{url}: the response names {family}, the lock {}",
                entry.family
            )),
            Some(family) => {
                let licence = google_catalog()
                    .find(family)
                    .map(|found| found.family.licence.spdx());
                if licence != Some(entry.licence.as_str()) {
                    problems.push(format!(
                        "{url}: licence {}, the catalog says {}",
                        entry.licence,
                        licence.unwrap_or("nothing")
                    ));
                }
            }
        }
    }
    for url in named.keys() {
        if !files.contains(url.as_str()) {
            problems.push(format!("{url}: named by a css2 response, not pinned"));
        }
    }
    problems
}

pub fn parse_lane_lock(text: &str) -> Result<LaneLock> {
    let lock: LaneLock = serde_json::from_str(text)
        .map_err(|error| BenchError::failed(format!("{LANE_LOCK}: {error}")))?;
    let mut problems = Vec::new();
    if lock.version != LANE_LOCK_VERSION {
        problems.push(format!("version is not {LANE_LOCK_VERSION}"));
    }
    if lock.fallback.len() != 1 {
        problems.push("the fallback is one file".to_owned());
    }
    let all: Vec<FontEntry> = lock.entries().map(|(entry, _)| entry).collect();
    problems.extend(entry_problems(&all));
    problems.extend(google_problems(&lock.google));
    for entry in &all {
        if matches!(entry.source, FontSource::Host { .. }) {
            problems.push(format!(
                "{}: a host font; the lane takes pinned files only",
                entry.file
            ));
        }
    }
    let mut seen = BTreeSet::new();
    for entry in &all {
        if !seen.insert(entry.sha256.as_str()) {
            problems.push(format!("{}: listed twice", entry.file));
        }
    }
    if problems.is_empty() {
        Ok(lock)
    } else {
        Err(BenchError::failed(format!(
            "{LANE_LOCK}: {}",
            problems.join("; ")
        )))
    }
}

pub fn load_lane_lock(path: &Path) -> Result<LaneLock> {
    parse_lane_lock(&std::fs::read_to_string(path).map_err(BenchError::io(path))?)
}

/// The CLI cache's name for a URL's body: `google/<sha256 of the URL>`.
pub fn cli_cache_name(url: &str) -> String {
    sha256(url.as_bytes())
}

/// A file the lane provides, with the PostScript names of its faces.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProvidedFile {
    pub family: String,
    pub sha256: String,
    pub postscript_names: Vec<String>,
    pub source: LaneSource,
    /// Where the cache holds it.
    pub path: PathBuf,
    /// Its name in the lock.
    pub file: String,
}

/// The lane's folders, materialised.
#[derive(Clone, Debug)]
pub struct LaneFonts {
    pub root: PathBuf,
    /// The compiler's `SLIDOCK_CACHE_DIR`.
    pub cli_cache: PathBuf,
    /// Mounted at /fonts in the renderer.
    pub fallback: PathBuf,
    /// The sha256 of the sorted hashes of every file.
    pub font_set_id: String,
    pub lock_sha256: String,
    /// The Google and substitute files.
    pub provided: Vec<ProvidedFile>,
}

/// Hard-links (or copies, across devices) `source` to `target`.
pub fn link(source: &Path, target: &Path) -> Result<()> {
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(BenchError::io(parent))?;
    }
    if std::fs::hard_link(source, target).is_err() {
        std::fs::copy(source, target).map_err(BenchError::io(target))?;
    }
    Ok(())
}

/// Removes every entry of `dir` whose name `keep` refuses.
fn prune(dir: &Path, keep: impl Fn(&str) -> bool) -> Result<()> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(BenchError::io(dir)(error)),
    };
    for entry in entries {
        let entry = entry.map_err(BenchError::io(dir))?;
        let name = entry.file_name();
        if !name.to_str().is_some_and(&keep) {
            remove(&dir.join(name))?;
        }
    }
    Ok(())
}

/// Writes the CLI's cache for the lock: the substitutes under `bundled/`, the Google answers under `google/`.
pub fn write_cli_cache(lock: &LaneLock, files: &Path, cli_cache: &Path) -> Result<()> {
    remove(cli_cache)?;
    for entry in &lock.substitutes {
        link(
            &files.join(font_cache_name(entry)),
            &cli_cache
                .join("bundled")
                .join(format!("{}.ttf", entry.sha256)),
        )?;
    }
    let google = cli_cache.join("google");
    for response in &lock.google.css2 {
        write_atomic(&google.join(cli_cache_name(&response.url)), &response.css)?;
    }
    for entry in &lock.google.files {
        if let Some(url) = plain_url(entry) {
            link(
                &files.join(font_cache_name(entry)),
                &google.join(cli_cache_name(url)),
            )?;
        }
    }
    Ok(())
}

/// Downloads what the cache lacks (each hash checked) and lays out the lane's folders afresh.
pub async fn prepare(lock_path: &Path, layout: &CacheLayout) -> Result<LaneFonts> {
    let lock = load_lane_lock(lock_path)?;
    let root = layout.root.join("fonts-lane");
    prune(&root, |name| LAYOUT.contains(&name))?;
    let files = root.join("files");
    std::fs::create_dir_all(&files).map_err(BenchError::io(&files))?;
    let wanted: BTreeSet<String> = lock
        .entries()
        .map(|(entry, _)| font_cache_name(&entry))
        .collect();
    prune(&files, |name| wanted.contains(name))?;
    for (entry, _) in lock.entries() {
        let target = files.join(font_cache_name(&entry));
        if target.exists() && sha256_file(&target)? == entry.sha256 {
            continue;
        }
        let bytes = fetch_font(&entry, layout).await?;
        let actual = sha256(&bytes);
        if actual != entry.sha256 {
            return Err(BenchError::failed(format!(
                "{}: sha256 {actual}, the lane lock says {}",
                entry.file, entry.sha256
            )));
        }
        write_atomic(&target, bytes)?;
    }
    let hashes: Vec<String> = lock.entries().map(|(entry, _)| entry.sha256).collect();
    let lanes = LaneFonts {
        cli_cache: root.join("cli-cache"),
        fallback: root.join("fallback"),
        font_set_id: font_set_id(hashes.iter().map(String::as_str))?,
        lock_sha256: sha256_file(lock_path)?,
        provided: Vec::new(),
        root,
    };
    write_cli_cache(&lock, &files, &lanes.cli_cache)?;
    remove(&lanes.fallback)?;
    let mut provided = Vec::new();
    for (entry, role) in lock.entries() {
        let cached = files.join(font_cache_name(&entry));
        let source = match role {
            LaneRole::Google => LaneSource::Google,
            LaneRole::Substitute => LaneSource::Substitute,
            LaneRole::Fallback => {
                link(&cached, &lanes.fallback.join(&entry.file))?;
                continue;
            }
        };
        let bytes = std::fs::read(&cached).map_err(BenchError::io(&cached))?;
        let postscript_names = slidock_fonts::read_sfnt(&bytes)
            .map_err(|error| BenchError::failed(format!("{}: {error}", entry.file)))?
            .into_iter()
            .map(|face| face.postscript_name)
            .collect();
        provided.push(ProvidedFile {
            family: entry.family.clone(),
            sha256: entry.sha256.clone(),
            postscript_names,
            source,
            path: cached,
            file: entry.file.clone(),
        });
    }
    Ok(LaneFonts { provided, ..lanes })
}

#[cfg(test)]
mod tests {
    use slidock_fonts::google::css2_url;
    use slidock_ir::fonts::FaceStyle;

    use super::*;

    fn committed() -> String {
        std::fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../..")
                .join(LANE_LOCK),
        )
        .unwrap()
    }

    fn css(rules: &[(&str, u32, &str)]) -> String {
        rules
            .iter()
            .map(|(family, weight, url)| {
                format!(
                    "@font-face {{\n  font-family: '{family}';\n  font-style: normal;\n  font-weight: {weight};\n  \
                     font-display: block;\n  src: url({url}) format('truetype');\n}}\n"
                )
            })
            .collect()
    }

    fn google(rules: &[(&str, u32, &str)]) -> GoogleLock {
        let css = css(rules);
        GoogleLock {
            css2: vec![Css2Response {
                url: format!("{CSS2_ENDPOINT}?family=Oswald:wght@500&display=block"),
                sha256: sha256(&css),
                css,
            }],
            files: rules
                .iter()
                .map(|(family, weight, url)| FontEntry {
                    family: (*family).into(),
                    file: format!("{weight}.ttf"),
                    sha256: sha256(url),
                    licence: "OFL-1.1".into(),
                    source: FontSource::Url {
                        url: (*url).into(),
                        member: None,
                    },
                })
                .collect(),
        }
    }

    #[test]
    fn the_committed_lock_pins_open_files_only() {
        let lock = parse_lane_lock(&committed()).unwrap();
        assert!(!lock.google.css2.is_empty() && !lock.google.files.is_empty());
        assert!(!lock.substitutes.is_empty());
        for (entry, _) in lock.entries() {
            assert!(
                ["OFL-1.1", "Apache-2.0", "UFL-1.0", "Bitstream-Vera"]
                    .contains(&entry.licence.as_str()),
                "{}: {}",
                entry.file,
                entry.licence
            );
        }
    }

    /// Each pinned response is a request the compiler makes: the `css2` URL of its own family and faces.
    #[test]
    fn each_pinned_response_answers_the_request_the_compiler_builds() {
        let lock = parse_lane_lock(&committed()).unwrap();
        for response in &lock.google.css2 {
            let faces = parse_css2(&response.css).unwrap();
            let family = &faces[0].family;
            let slots: Vec<(u32, FaceStyle)> = faces
                .iter()
                .map(|face| (face.weight.min, face.style))
                .collect();
            assert!(
                faces
                    .iter()
                    .all(|face| face.weight.min == face.weight.max && &face.family == family),
                "{}: one static file per face",
                response.url
            );
            assert_eq!(
                css2_url(google_catalog(), family, &slots).as_deref(),
                Some(response.url.as_str())
            );
        }
    }

    #[test]
    fn the_google_files_are_exactly_the_ones_the_responses_name() {
        let a = "https://fonts.gstatic.com/s/oswald/v1/a.ttf";
        let b = "https://fonts.gstatic.com/s/oswald/v1/b.ttf";
        assert_eq!(
            google_problems(&google(&[("Oswald", 500, a)])),
            Vec::<String>::new()
        );

        let mut extra = google(&[("Oswald", 500, a), ("Oswald", 700, b)]);
        extra.css2[0].css = css(&[("Oswald", 500, a)]);
        extra.css2[0].sha256 = sha256(&extra.css2[0].css);
        assert_eq!(
            google_problems(&extra),
            [format!("{b}: no css2 response names it")]
        );

        let mut missing = google(&[("Oswald", 500, a), ("Oswald", 700, b)]);
        missing.files.pop();
        assert_eq!(
            google_problems(&missing),
            [format!("{b}: named by a css2 response, not pinned")]
        );

        let mut edited = google(&[("Oswald", 500, a)]);
        edited.css2[0].css.push(' ');
        assert!(google_problems(&edited)[0].contains("does not match its sha256"));

        let mut licence = google(&[("Oswald", 500, a)]);
        licence.files[0].licence = "Apache-2.0".into();
        assert!(google_problems(&licence)[0].contains("the catalog says OFL-1.1"));

        let elsewhere = "https://example.com/s/oswald/a.ttf";
        assert!(
            google_problems(&google(&[("Oswald", 500, elsewhere)]))
                .iter()
                .any(|p| p.contains("A rule's font file is not on fonts.gstatic.com"))
        );
    }

    #[test]
    fn the_cli_cache_holds_each_answer_under_the_sha256_of_its_url() {
        let dir = tempfile::tempdir().unwrap();
        let files = dir.path().join("files");
        let url = "https://fonts.gstatic.com/s/oswald/v1/a.ttf";
        let mut lock = parse_lane_lock(&committed()).unwrap();
        lock.google = google(&[("Oswald", 500, url)]);
        lock.google.files[0].sha256 = sha256(b"font");
        write_atomic(&files.join(font_cache_name(&lock.google.files[0])), b"font").unwrap();
        for entry in &lock.substitutes {
            write_atomic(&files.join(font_cache_name(entry)), b"sub").unwrap();
        }
        let cache = dir.path().join("cli-cache");
        write_atomic(&cache.join("google/stale"), b"x").unwrap();
        write_cli_cache(&lock, &files, &cache).unwrap();
        let google = cache.join("google");
        // The CLI's name: `Sha256::of(url.as_bytes()).to_hex()`.
        assert_eq!(
            cli_cache_name(url),
            hex::encode(<sha2::Sha256 as sha2::Digest>::digest(url.as_bytes()))
        );
        assert_eq!(
            std::fs::read(google.join(cli_cache_name(url))).unwrap(),
            b"font"
        );
        assert_eq!(
            std::fs::read_to_string(google.join(cli_cache_name(&lock.google.css2[0].url))).unwrap(),
            lock.google.css2[0].css
        );
        assert!(!google.join("stale").exists());
        assert_eq!(
            std::fs::read_dir(cache.join("bundled")).unwrap().count(),
            lock.substitutes.len()
        );
    }

    #[test]
    fn prepare_keeps_only_its_own_layout() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["files", "google", "work", "stray.txt"] {
            write_atomic(&dir.path().join(name).join("x"), b"x").unwrap();
        }
        prune(dir.path(), |name| LAYOUT.contains(&name)).unwrap();
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().into_string().unwrap())
            .collect();
        left.sort();
        assert_eq!(left, ["files", "work"]);
    }

    #[test]
    fn a_host_font_or_a_second_fallback_is_refused() {
        let mut value: serde_json::Value = serde_json::from_str(&committed()).unwrap();
        value["substitutes"][0]["source"] =
            serde_json::json!({"kind": "host", "paths": ["/Library/Fonts/A.ttf"]});
        let error = parse_lane_lock(&value.to_string()).unwrap_err().to_string();
        assert!(error.contains("a host font"), "{error}");

        let mut value: serde_json::Value = serde_json::from_str(&committed()).unwrap();
        let first = value["substitutes"][0].clone();
        value["fallback"].as_array_mut().unwrap().push(first);
        let error = parse_lane_lock(&value.to_string()).unwrap_err().to_string();
        assert!(
            error.contains("the fallback is one file") && error.contains("listed twice"),
            "{error}"
        );
    }
}
