//! The public report (`report --public`): the single-file report over the presentations `bench/public.json` names,
//! and nothing of any other. The list is default-deny: a presentation it does not name has no row, id, count or share
//! of a summary in the report, whose summaries are computed again over the rows it publishes. Only static runs are
//! published, without parity (its report covers every pair of the corpus). A row names its source as
//! `<presentation>.<extension>`, never by the corpus file name, and no path at all; the oracle, render and heatmap of
//! the rows of the published classes are WebP files beside the page, under `images/<run>/<row>/<kind>.webp`.
//! [`check_payload`] holds a payload to that: the generation runs it with the corpus manifest's names and writes
//! nothing on a finding, and a test runs it over the committed report with the names the repository holds.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;
use slidock_bench_schema::api::{ImageFrom, ImageKind, ImageRef, RowDetail, RowViewer};
use slidock_bench_schema::corpus::{CorpusManifest, Oracle};
use slidock_bench_schema::results::{Outcome, Results};
use slidock_bench_schema::runs::RunEntry;
use slidock_bench_schema::{Frontend, Lane, PairKey, PresentationId, RowId};

use crate::error::{BenchError, Result};

/// The presentations the public report may name, relative to the repository.
pub const ALLOWLIST: &str = "bench/public.json";
const ALLOWLIST_SCHEMA: &str = "slidock-bench-public@1";
/// The folder the web app's build copies to `<base>/bench/`, relative to the repository.
pub const PUBLIC_DIR: &str = "apps/web/public/bench";
/// The page's images, relative to its folder.
pub const IMAGES_DIR: &str = "images";

/// Text no public payload holds: home and temporary folders, the private tree, installed fonts, file URLs, the
/// holdout.
const FORBIDDEN: [&str; 12] = [
    "/Users/",
    "/home/",
    "/private/",
    "/var/folders/",
    "/tmp/",
    "~/",
    ".slidock",
    "SLIDOCK_PRIVATE",
    "Desktop",
    "Library/Fonts",
    "file:",
    "holdout",
];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AllowlistFile {
    schema: String,
    presentations: Vec<PresentationId>,
}

/// `bench/public.json`'s presentations.
pub fn read_allowlist(path: &Path) -> Result<BTreeSet<PresentationId>> {
    let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
    let file: AllowlistFile = slidock_bench_schema::parse(&text)?;
    if file.schema != ALLOWLIST_SCHEMA {
        return Err(BenchError::failed(format!(
            "{}: schema is {:?}, not {ALLOWLIST_SCHEMA}",
            path.display(),
            file.schema
        )));
    }
    let mut allow = BTreeSet::new();
    for id in file.presentations {
        if !allow.insert(id.clone()) {
            return Err(BenchError::failed(format!(
                "{}: {id} is listed twice",
                path.display()
            )));
        }
    }
    Ok(allow)
}

/// The classes whose rows a public report carries images of, as the page reads them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PublicScope {
    pub images: Vec<Outcome>,
}

const fn extension(frontend: Frontend) -> &'static str {
    match frontend {
        Frontend::Pptx => "pptx",
        Frontend::Odp => "odp",
        Frontend::Keynote => "key",
        Frontend::Ir => "json",
    }
}

/// The name a public row gives its source file.
pub fn public_file(row: &RowId) -> String {
    format!("{}.{}", row.presentation(), extension(row.frontend()))
}

/// A row id as a file name: `:` as `__` (as the run folders name rows), every other byte outside `[A-Za-z0-9._-]` as
/// `~` and its two hex digits, so the name needs no escaping in a URL.
fn row_file(row: &RowId) -> String {
    let mut out = String::new();
    for byte in row.to_string().bytes() {
        match byte {
            b':' => out.push_str("__"),
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'.' | b'_' | b'-' => out.push(byte as char),
            _ => out.push_str(&format!("~{byte:02x}")),
        }
    }
    out
}

/// Where the page finds a row's image, relative to its folder.
pub fn image_path(run: &str, row: &RowId, kind: &str) -> String {
    format!(
        "{IMAGES_DIR}/{}/{}/{kind}.webp",
        run.get(..12).unwrap_or(run),
        row_file(row)
    )
}

/// A static run's results over the allowlist: its rows, its stamp's pairs and its summary.
pub fn scope_results(mut results: Results, allow: &BTreeSet<PresentationId>) -> Result<Results> {
    if results.stamp.lane != Lane::Static {
        return Err(BenchError::refused(format!(
            "the public report publishes static runs only, not a {:?} run",
            results.stamp.lane
        )));
    }
    results
        .rows
        .retain(|id, _| allow.contains(id.presentation()));
    results
        .stamp
        .corpus
        .retain(|pair, _| allow.contains(pair.presentation()));
    if let Some(perf) = results.stamp.perf.as_mut() {
        perf.decks
            .retain(|pair, _| allow.contains(pair.presentation()));
    }
    results.stamp.parity = None;
    results.summary = crate::results::summary(&results.rows);
    results.holdout = None;
    results.timelines = None;
    results.font_pairs = None;
    results.shell_pairs = None;
    results.motion_pairs = None;
    results.positions = None;
    Ok(results)
}

/// A run's index entry for its scoped results: their summary, and the hash of their stamp.
pub fn scope_entry(mut entry: RunEntry, results: &Results) -> Result<RunEntry> {
    entry.summary = results.summary.clone();
    entry.stamp = slidock_bench_schema::sha256_json(&results.stamp)?;
    Ok(entry)
}

/// A row's detail as published: its source named by [`public_file`], its oracle, render and heatmap at the paths
/// `published` gives (the others say `note`), and no golden or motion frame.
pub fn scope_detail(
    mut detail: RowDetail,
    published: &BTreeMap<ImageKind, String>,
    note: &str,
) -> RowDetail {
    detail.viewer = RowViewer {
        available: false,
        file: public_file(&detail.id),
    };
    let mut images = BTreeMap::new();
    for kind in [ImageKind::Oracle, ImageKind::Render, ImageKind::Heatmap] {
        let image = match (published.get(&kind), detail.images.get(&kind)) {
            (Some(url), Some(ImageRef::Found { from, run, .. })) => ImageRef::Found {
                url: url.clone(),
                from: *from,
                run: run.clone(),
            },
            (Some(url), _) => ImageRef::Found {
                url: url.clone(),
                from: ImageFrom::Run,
                run: None,
            },
            (None, _) => ImageRef::Missing {
                missing: note.to_owned(),
            },
        };
        images.insert(kind, image);
    }
    detail.images = images;
    detail
}

/// What a payload must not hold: text anywhere, and presentation ids as whole words.
#[derive(Clone, Debug, Default)]
pub struct Forbidden {
    texts: BTreeSet<String>,
    ids: BTreeSet<String>,
}

impl Forbidden {
    /// Paths and the private tree ([`FORBIDDEN`]).
    pub fn generic() -> Forbidden {
        Forbidden {
            texts: FORBIDDEN.iter().map(|t| (*t).to_owned()).collect(),
            ids: BTreeSet::new(),
        }
    }

    pub fn text(mut self, text: impl Into<String>) -> Forbidden {
        let text = text.into();
        if !text.is_empty() {
            self.texts.insert(text);
        }
        self
    }

    fn id(&mut self, id: &str, allow: &BTreeSet<PresentationId>) {
        if !allow.iter().any(|a| a.as_str() == id) {
            self.ids.insert(id.to_owned());
        }
    }

    /// The corpus ids the repository names outside the allowlist: the pairs of `bench/budgets.json` and the corpus
    /// pairs of `tests/golden/manifest.json`.
    pub fn repository(
        mut self,
        repo: &Path,
        allow: &BTreeSet<PresentationId>,
    ) -> Result<Forbidden> {
        let read = |relative: &str| -> Result<Value> {
            let path = repo.join(relative);
            let text = std::fs::read_to_string(&path).map_err(BenchError::io(&path))?;
            serde_json::from_str(&text)
                .map_err(|e| BenchError::failed(format!("{}: {e}", path.display())))
        };
        let budgets = read("bench/budgets.json")?;
        for pair in budgets
            .get("decks")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|decks| decks.keys())
        {
            if let Ok(pair) = pair.parse::<PairKey>() {
                self.id(pair.presentation().as_str(), allow);
            }
        }
        let golden = read(crate::golden::MANIFEST)?;
        for source in golden
            .get("pairs")
            .and_then(Value::as_object)
            .into_iter()
            .flat_map(|pairs| pairs.values())
            .filter_map(Value::as_str)
        {
            if let Some((id, _)) = source
                .strip_prefix("corpus:")
                .and_then(|rest| rest.rsplit_once('.'))
            {
                self.id(id, allow);
            }
        }
        Ok(self)
    }

    /// The corpus manifest's names: every presentation id outside the allowlist, every corpus file name (and, outside
    /// the allowlist, its stem) but the public names, and every title that is not its id.
    pub fn manifest(
        mut self,
        manifest: &CorpusManifest,
        allow: &BTreeSet<PresentationId>,
    ) -> Forbidden {
        for presentation in &manifest.presentations {
            self.id(presentation.id.as_str(), allow);
            let mut files: Vec<&str> = presentation
                .sources
                .iter()
                .map(|s| s.file.as_str())
                .collect();
            files.push(match &presentation.oracle {
                Oracle::PdfFile(oracle) => &oracle.file,
                Oracle::KeynoteExport(oracle) => &oracle.source,
            });
            for file in files {
                let own = presentation
                    .sources
                    .iter()
                    .any(|s| file == format!("{}.{}", presentation.id, extension(s.frontend)));
                if own {
                    continue;
                }
                self.texts.insert(file.to_owned());
                // A stem inside a public id (a probe's `gradient-probe`) names nothing private.
                let stem = file.rsplit_once('.').map_or(file, |(stem, _)| stem);
                if stem.chars().count() >= 4 && !allow.iter().any(|a| a.as_str().contains(stem)) {
                    self.texts.insert(stem.to_owned());
                }
            }
            if let Some(title) = &presentation.title
                && title != presentation.id.as_str()
            {
                self.texts.insert(title.clone());
            }
        }
        self
    }
}

/// `id` appears in `text` as a whole word: not inside a longer run of `[a-z0-9-]`.
fn holds_word(text: &str, id: &str) -> bool {
    let word = |c: char| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-';
    text.match_indices(id).any(|(at, _)| {
        let before = text.get(..at).and_then(|t| t.chars().next_back());
        let after = text.get(at + id.len()..).and_then(|t| t.chars().next());
        !before.is_some_and(word) && !after.is_some_and(word)
    })
}

/// The presentation a token names as a row id or a pair key.
fn named_presentation(token: &str) -> Option<PresentationId> {
    if let Ok(row) = token.parse::<RowId>() {
        return Some(row.presentation().clone());
    }
    token
        .parse::<PairKey>()
        .ok()
        .map(|pair| pair.presentation().clone())
}

fn check_text(
    text: &str,
    at: &str,
    allow: &BTreeSet<PresentationId>,
    forbidden: &Forbidden,
    problems: &mut Vec<String>,
) {
    for needle in &forbidden.texts {
        if text.contains(needle.as_str()) {
            problems.push(format!("{at}: holds {needle:?}"));
        }
    }
    for id in &forbidden.ids {
        if holds_word(text, id) {
            problems.push(format!("{at}: names {id}"));
        }
    }
    let punctuation = |c: char| "\"'`,;()[]{}<>".contains(c);
    for token in text.split_whitespace().map(|t| t.trim_matches(punctuation)) {
        if let Some(id) = named_presentation(token)
            && !allow.contains(&id)
        {
            problems.push(format!("{at}: names {id} ({token})"));
        }
    }
}

fn walk(
    value: &Value,
    at: &str,
    allow: &BTreeSet<PresentationId>,
    forbidden: &Forbidden,
    problems: &mut Vec<String>,
) {
    match value {
        Value::String(text) => check_text(text, at, allow, forbidden, problems),
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                walk(item, &format!("{at}[{i}]"), allow, forbidden, problems);
            }
        }
        Value::Object(map) => {
            for (key, item) in map {
                let here = format!("{at}.{key}");
                check_text(key, &format!("{here} (key)"), allow, forbidden, problems);
                if key == "presentation"
                    && let Some(id) = item.as_str()
                    && !allow.iter().any(|a| a.as_str() == id)
                {
                    problems.push(format!("{here}: {id} is not in {ALLOWLIST}"));
                }
                walk(item, &here, allow, forbidden, problems);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// Every reason `payload` may not be published: a presentation outside the allowlist named anywhere (a row id or
/// pair key in any key or text, a `presentation` field, a forbidden id as a word), or a forbidden text.
pub fn check_payload(
    payload: &Value,
    allow: &BTreeSet<PresentationId>,
    forbidden: &Forbidden,
) -> Vec<String> {
    let mut problems = Vec::new();
    if payload.get("public").is_none_or(Value::is_null) {
        problems.push("the payload is not marked public".to_owned());
    }
    walk(payload, "$", allow, forbidden, &mut problems);
    problems
}

/// Every image path a payload names.
pub fn image_paths(payload: &Value) -> BTreeSet<String> {
    fn collect(value: &Value, out: &mut BTreeSet<String>) {
        match value {
            Value::String(text) if text.starts_with(&format!("{IMAGES_DIR}/")) => {
                out.insert(text.clone());
            }
            Value::Array(items) => items.iter().for_each(|item| collect(item, out)),
            Value::Object(map) => map.values().for_each(|item| collect(item, out)),
            _ => {}
        }
    }
    let mut out = BTreeSet::new();
    collect(payload, &mut out);
    out
}

/// Refuses a folder that holds anything but a public report's page and images, which the generation replaces.
pub fn check_out_dir(out: &Path) -> Result<()> {
    let Ok(entries) = std::fs::read_dir(out) else {
        return Ok(());
    };
    for entry in entries {
        let entry = entry.map_err(BenchError::io(out))?;
        let name = entry.file_name();
        let ours = (name == "index.html" && entry.path().is_file())
            || (name == IMAGES_DIR && entry.path().is_dir());
        if !ours {
            return Err(BenchError::refused(format!(
                "{} holds {}, which a public report does not write: pass an empty folder or the report's own",
                out.display(),
                name.to_string_lossy()
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    fn repo() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn allowlist() -> BTreeSet<PresentationId> {
        read_allowlist(&repo().join(ALLOWLIST)).unwrap()
    }

    const SLOT: &str = "<script type=\"application/json\" id=\"slidock-bench-data\">";

    fn committed() -> (String, Value) {
        let page = std::fs::read_to_string(repo().join(PUBLIC_DIR).join("index.html")).unwrap();
        let start = page.find(SLOT).unwrap() + SLOT.len();
        let end = start + page[start..].find("</script>").unwrap();
        let payload = serde_json::from_str(&page[start..end]).unwrap();
        (page, payload)
    }

    fn forbidden(allow: &BTreeSet<PresentationId>) -> Forbidden {
        Forbidden::generic().repository(&repo(), allow).unwrap()
    }

    fn files(dir: &Path, prefix: &str, out: &mut BTreeSet<String>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let name = format!("{prefix}/{}", entry.file_name().to_string_lossy());
            if entry.path().is_dir() {
                files(&entry.path(), &name, out);
            } else {
                out.insert(name);
            }
        }
    }

    #[test]
    fn the_allowlist_names_presentations_once() {
        let allow = allowlist();
        assert!(!allow.is_empty());
        let dir = tempfile::tempdir().unwrap();
        let twice = dir.path().join("public.json");
        std::fs::write(
            &twice,
            r#"{"schema":"slidock-bench-public@1","presentations":["a","a"]}"#,
        )
        .unwrap();
        assert!(read_allowlist(&twice).is_err());
        std::fs::write(&twice, r#"{"schema":"other@1","presentations":[]}"#).unwrap();
        assert!(read_allowlist(&twice).is_err());
    }

    #[test]
    fn the_committed_report_names_only_the_allowlist() {
        let allow = allowlist();
        let (_, payload) = committed();
        let problems = check_payload(&payload, &allow, &forbidden(&allow));
        assert!(problems.is_empty(), "{problems:#?}");
        let results = payload["results"].as_object().unwrap();
        assert!(!results.is_empty());
        for (run, results) in results {
            for (id, record) in results["rows"].as_object().unwrap() {
                let row: RowId = id.parse().unwrap();
                assert!(allow.contains(row.presentation()), "{run} {id}");
                assert_eq!(record["presentation"], row.presentation().as_str());
            }
        }
        for (key, detail) in payload["rows"].as_object().unwrap() {
            let row: RowId = detail["id"].as_str().unwrap().parse().unwrap();
            assert!(key.ends_with(&format!(" {row}")));
            assert_eq!(detail["viewer"]["file"], public_file(&row));
        }
        assert!(payload["parity"].as_object().unwrap().is_empty());
        assert!(payload["status"].is_null());
    }

    #[test]
    fn the_committed_images_are_the_ones_the_report_names() {
        let allow = allowlist();
        let (_, payload) = committed();
        let named = image_paths(&payload);
        let mut found = BTreeSet::new();
        let images = repo().join(PUBLIC_DIR).join(IMAGES_DIR);
        if images.is_dir() {
            files(&images, IMAGES_DIR, &mut found);
        }
        assert_eq!(named, found);
        for path in &named {
            let parts: Vec<&str> = path.split('/').collect();
            let [_, run, row, kind] = parts.as_slice() else {
                panic!("{path}");
            };
            assert_eq!(run.len(), 12, "{path}");
            let presentation = row.split("__").next().unwrap();
            assert!(allow.iter().any(|a| a.as_str() == presentation), "{path}");
            assert!(
                ["oracle.webp", "render.webp", "heatmap.webp"].contains(kind),
                "{path}"
            );
            let bytes = std::fs::read(repo().join(PUBLIC_DIR).join(path)).unwrap();
            assert!(
                bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WEBP"),
                "{path}"
            );
        }
    }

    #[test]
    fn the_committed_page_runs_only_its_own_script() {
        use base64::Engine;
        let (page, _) = committed();
        let open = "<script type=\"module\" crossorigin>";
        assert_eq!(page.matches("<script type=\"module\"").count(), 1);
        let start = page.find(open).unwrap() + open.len();
        let end = start + page[start..].find("</script>").unwrap();
        let hash = base64::engine::general_purpose::STANDARD.encode(sha2::Digest::finalize(
            <sha2::Sha256 as sha2::Digest>::new_with_prefix(&page[start..end]),
        ));
        let csp = format!("script-src 'sha256-{hash}'");
        let meta = page.find("http-equiv=\"Content-Security-Policy\"").unwrap();
        assert!(meta < start);
        assert!(
            page.contains(&csp),
            "the page's CSP does not allow its script ({csp})"
        );
        assert!(page.contains("default-src 'none'"));
    }

    type Injection<'a> = (&'a str, Box<dyn Fn(&mut Value) + 'a>);

    #[test]
    fn a_private_name_or_path_fails_the_check() {
        let allow = allowlist();
        let forbidden = forbidden(&allow)
            .text("Private Deck.key")
            .text("/opt/corpus");
        let (_, payload) = committed();
        assert!(check_payload(&payload, &allow, &forbidden).is_empty());
        let private = forbidden.ids.iter().next().unwrap().clone();
        let run = payload["results"]
            .as_object()
            .unwrap()
            .keys()
            .next()
            .unwrap()
            .clone();
        let injections: Vec<Injection<'_>> = vec![
            (
                "a row of another deck",
                Box::new(|p: &mut Value| {
                    p["results"][&run]["rows"][format!("{private}:kn-1:keynote")] = Value::Null;
                }),
            ),
            (
                "a pair of another deck",
                Box::new(|p: &mut Value| {
                    p["results"][&run]["stamp"]["corpus"][format!("{private}:pptx")] =
                        Value::String("0".repeat(64));
                }),
            ),
            (
                "another deck in a text",
                Box::new(|p: &mut Value| {
                    p["runs"]["schema"] = Value::String(format!("as {private} shows"));
                }),
            ),
            (
                "a presentation outside the list",
                Box::new(|p: &mut Value| {
                    p["results"][&run]["rows"]["x"] =
                        serde_json::json!({ "presentation": "new-deck" });
                }),
            ),
            (
                "a font path",
                Box::new(|p: &mut Value| {
                    p["results"][&run]["stamp"]["classifier"] =
                        Value::String("/Users/someone/Library/Fonts/A.ttf".into());
                }),
            ),
            (
                "a private file name",
                Box::new(|p: &mut Value| {
                    p["results"][&run]["stamp"]["classifier"] =
                        Value::String("Private Deck.key".into());
                }),
            ),
            (
                "an unmarked payload",
                Box::new(|p: &mut Value| {
                    p["public"] = Value::Null;
                }),
            ),
        ];
        for (what, inject) in injections {
            let mut copy = payload.clone();
            inject(&mut copy);
            assert!(
                !check_payload(&copy, &allow, &forbidden).is_empty(),
                "{what} passes the check"
            );
        }
    }

    #[test]
    fn ids_match_as_whole_words() {
        assert!(holds_word("see tables here", "tables"));
        assert!(holds_word("tables:pptx", "tables"));
        assert!(!holds_word("tables-keynote", "tables"));
        assert!(!holds_word("retro-tables", "tables"));
        assert!(!holds_word("timetables", "tables"));
    }

    #[test]
    fn rows_name_files_and_images_safely() {
        let row: RowId = "retro:kn-12:keynote".parse().unwrap();
        assert_eq!(public_file(&row), "retro.key");
        assert_eq!(
            image_path(&"a".repeat(64), &row, "render"),
            format!(
                "images/{}/retro__kn-12__keynote/render.webp",
                "a".repeat(12)
            )
        );
        let step: RowId = "deck:s1:pptx#t@250".parse().unwrap();
        assert_eq!(row_file(&step), "deck__s1__pptx~23t~40250");
        assert_eq!(public_file(&step), "deck.pptx");
    }

    #[test]
    fn results_keep_the_allowlisted_rows_and_their_summary() {
        let text = std::fs::read_to_string(
            repo().join("crates/slidock-bench-schema/tests/data/results.json"),
        )
        .unwrap();
        let results = Results::parse(&text).unwrap();
        let some = results.rows.keys().next().unwrap().presentation().clone();
        let scoped = scope_results(results.clone(), &BTreeSet::from([some.clone()])).unwrap();
        assert!(scoped.rows.keys().all(|id| id.presentation() == &some));
        assert!(
            scoped
                .stamp
                .corpus
                .keys()
                .all(|pair| pair.presentation() == &some)
        );
        assert_eq!(scoped.summary, crate::results::summary(&scoped.rows));
        let none = scope_results(results, &BTreeSet::new()).unwrap();
        assert!(none.rows.is_empty() && none.stamp.corpus.is_empty());
        assert!(none.summary.by_frontend.is_empty());
    }
}
