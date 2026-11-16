//! slidock's goldens, as parity reads them. `cargo xtask golden --record` writes them privately under
//! `$SLIDOCK_PRIVATE/golden/<commit>/` (`<commit>` is the recording's 12-digit commit),
//! and the committed manifest (`tests/golden/manifest.json`, version 1) names that commit and pins every output by
//! sha256 (`outputs`, paths relative to the recording). A corpus pair's static-lane output is the CLI's `--out`
//! folder of `compile --emit documents,report,ir --fonts`, at `corpus/<presentation>.<frontend>/`: the document set,
//! `report.json` and `presentation-ir.json`.
//!
//! [`GoldenSet::open`] reads the manifest and checks every output it pins before anything reads one: a missing file,
//! a changed byte, a path that is not a plain relative path, a symbolic link or anything but a regular file on the
//! way, or a file past [`MAX_FILE_BYTES`] refuses the set; another manifest version is refused.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::Deserialize;
use slidock_bench_schema::Sha256;
use slidock_bench_schema::parity::GoldenIdentity;

use crate::error::{BenchError, Result};

/// The committed manifest, relative to the repository.
pub const MANIFEST: &str = "tests/golden/manifest.json";

/// The folder of the corpus pairs' static-lane output in a recording.
const CORPUS: &str = "corpus";

/// At most this many pinned files: a manifest naming more is refused.
const MAX_FILES: usize = 1 << 20;

/// The largest manifest read.
const MAX_MANIFEST_BYTES: u64 = 256 << 20;

/// The largest pinned file: the CLI's input limit, which no output of a compile passes.
pub const MAX_FILE_BYTES: u64 = 2 << 30;

/// Up to `limit` bytes of the regular file at `path`, which is not a symbolic link; `None` when it is absent.
fn read_bounded(path: &Path, limit: u64) -> Result<Option<Vec<u8>>> {
    let metadata = match std::fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(BenchError::io(path)(error)),
    };
    if !metadata.file_type().is_file() {
        return Err(BenchError::refused(format!(
            "{}: not a regular file",
            path.display()
        )));
    }
    let file = std::fs::File::open(path).map_err(BenchError::io(path))?;
    let mut bytes = Vec::new();
    file.take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(BenchError::io(path))?;
    if bytes.len() as u64 > limit {
        return Err(BenchError::refused(format!(
            "{}: larger than {limit} bytes",
            path.display()
        )));
    }
    Ok(Some(bytes))
}

/// A pinned output's sha256: every folder from `dir` down a real folder and the file a regular file, no symbolic
/// link on the way, the file at most [`MAX_FILE_BYTES`], read in bounded steps.
fn pinned_sha256(dir: &Path, relative: &Path) -> std::result::Result<String, &'static str> {
    use sha2::Digest;
    let mut path = dir.to_path_buf();
    let components: Vec<_> = relative.components().collect();
    let last = components.len().saturating_sub(1);
    for (index, component) in components.into_iter().enumerate() {
        path.push(component);
        let metadata = std::fs::symlink_metadata(&path).map_err(|_| "missing")?;
        let kind = metadata.file_type();
        if kind.is_symlink() {
            return Err("a symbolic link");
        }
        let expected = if index == last {
            kind.is_file()
        } else {
            kind.is_dir()
        };
        if !expected {
            return Err("not a regular file");
        }
        if index == last && metadata.len() > MAX_FILE_BYTES {
            return Err("larger than the largest output");
        }
    }
    let file = std::fs::File::open(&path).map_err(|_| "missing")?;
    let mut reader = file.take(MAX_FILE_BYTES.saturating_add(1));
    let mut hasher = sha2::Sha256::new();
    let mut buffer = vec![0u8; 8 << 20];
    let mut total: u64 = 0;
    loop {
        let n = reader.read(&mut buffer).map_err(|_| "unreadable")?;
        if n == 0 {
            break;
        }
        total = total.saturating_add(n as u64);
        if total > MAX_FILE_BYTES {
            return Err("larger than the largest output");
        }
        hasher.update(buffer.get(..n).unwrap_or_default());
    }
    Ok(hex::encode(hasher.finalize()))
}

/// The manifest's fields parity reads; the recorder owns the rest (the reason, the pairs' sources, the inputs).
#[derive(Deserialize)]
struct Manifest {
    version: u32,
    commit: String,
    outputs: BTreeMap<String, Sha256>,
}

/// A golden set whose every pinned file was found with its pinned sha256.
#[derive(Clone, Debug)]
pub struct GoldenSet {
    pub identity: GoldenIdentity,
    /// `<root>/<commit>`.
    pub dir: PathBuf,
    files: BTreeMap<String, Sha256>,
}

/// Whether a manifest's commit names a recording: 12 to 40 hex digits.
fn is_commit(commit: &str) -> bool {
    (12..=40).contains(&commit.len()) && commit.bytes().all(|b| b.is_ascii_hexdigit())
}

/// A manifest path as relative components: no root, no `.`, no `..`, no empty component.
fn relative(path: &str) -> Option<PathBuf> {
    let parsed = Path::new(path);
    let plain = !path.is_empty()
        && !path.contains('\\')
        && parsed
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
        && !path.split('/').any(str::is_empty);
    plain.then(|| parsed.to_path_buf())
}

impl GoldenSet {
    /// The set the manifest at `manifest` pins, under `root`; `Ok(Err(reason))` when the manifest, the recording it
    /// names or its folder is absent.
    pub fn open(manifest: &Path, root: &Path) -> Result<std::result::Result<GoldenSet, String>> {
        let Some(text) = read_bounded(manifest, MAX_MANIFEST_BYTES)? else {
            return Ok(Err(format!("no golden manifest at {}", manifest.display())));
        };
        let parsed: Manifest = serde_json::from_slice(&text)
            .map_err(|error| BenchError::refused(format!("{}: {error}", manifest.display())))?;
        if parsed.version != 1 {
            return Err(BenchError::refused(format!(
                "{}: version {} (this bench reads version 1)",
                manifest.display(),
                parsed.version
            )));
        }
        if parsed.commit.is_empty() {
            return Ok(Err(format!(
                "{} names no recording yet",
                manifest.display()
            )));
        }
        if !is_commit(&parsed.commit) {
            return Err(BenchError::refused(format!(
                "{}: commit {:?} is not a hex commit",
                manifest.display(),
                parsed.commit
            )));
        }
        if parsed.outputs.is_empty() || parsed.outputs.len() > MAX_FILES {
            return Err(BenchError::refused(format!(
                "{}: pins {} outputs",
                manifest.display(),
                parsed.outputs.len()
            )));
        }
        let dir = root.join(&parsed.commit);
        if !dir.is_dir() {
            return Ok(Err(format!(
                "no golden files at {} (the manifest names commit {})",
                dir.display(),
                parsed.commit
            )));
        }
        let set = GoldenSet {
            identity: GoldenIdentity {
                commit: parsed.commit.clone(),
                manifest: Sha256::of(&text),
            },
            dir,
            files: parsed.outputs,
        };
        set.verify()?;
        Ok(Ok(set))
    }

    /// Every pinned output is there with its sha256.
    fn verify(&self) -> Result<()> {
        let mut problems = Vec::new();
        for (path, pinned) in &self.files {
            let Some(relative) = relative(path) else {
                problems.push(format!("{path}: not a plain relative path"));
                continue;
            };
            match pinned_sha256(&self.dir, &relative) {
                Ok(found) if found == pinned.as_str() => {}
                Ok(_) => problems.push(format!("{path}: its sha256 differs from the manifest's")),
                Err(why) => problems.push(format!("{path}: {why}")),
            }
        }
        if problems.is_empty() {
            return Ok(());
        }
        let shown: Vec<&String> = problems.iter().take(10).collect();
        Err(BenchError::refused(format!(
            "the goldens under {} do not match the manifest ({} files): {shown:?}",
            self.dir.display(),
            problems.len()
        )))
    }

    /// The corpus pairs with a static-lane output: `<presentation>.<frontend>`, each pinned with its report.
    pub fn static_pairs(&self) -> Vec<String> {
        self.files
            .keys()
            .filter_map(|path| {
                let rest = path.strip_prefix(CORPUS)?.strip_prefix('/')?;
                let (pair, file) = rest.split_once('/')?;
                (file == "report.json").then(|| pair.to_owned())
            })
            .collect()
    }

    /// A pair's static-lane output folder, when the set holds it.
    pub fn static_pair(&self, presentation: &str, frontend: &str) -> Option<PathBuf> {
        let pair = format!("{presentation}.{frontend}");
        self.files
            .contains_key(&format!("{CORPUS}/{pair}/report.json"))
            .then(|| self.dir.join(CORPUS).join(pair))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, path: &str, text: &str) -> (String, Sha256) {
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, text).unwrap();
        (path.to_owned(), Sha256::of(text))
    }

    const COMMIT: &str = "0123456789ab";

    /// A recording of two corpus pairs and a fixture under `root`, its manifest at `<root>/manifest.json`, in the
    /// recorder's layout.
    fn set_up(root: &Path) -> PathBuf {
        let dir = root.join(COMMIT);
        let outputs: BTreeMap<String, Sha256> = [
            write(&dir, "corpus/demo.pptx/report.json", "{}"),
            write(&dir, "corpus/demo.pptx/presentation-ir.json", "{\"a\":1}"),
            write(&dir, "corpus/demo.pptx/slides/s1.html", "<main></main>"),
            write(&dir, "corpus/demo.odp/report.json", "{}"),
            write(&dir, "fixtures/basic/report.json", "{}"),
            write(&dir, "slidock-fonts/faces.json", "[]"),
        ]
        .into_iter()
        .collect();
        let manifest = root.join("manifest.json");
        let value = serde_json::json!({
            "version": 1,
            "commit": COMMIT,
            "reason": "test",
            "pairs": { "demo.pptx": "corpus/demo.pptx", "demo.odp": "corpus/demo.odp" },
            "inputs": { "corpus/demo.pptx": "0".repeat(64) },
            "outputs": outputs,
        });
        std::fs::write(&manifest, serde_json::to_string(&value).unwrap()).unwrap();
        manifest
    }

    fn rewrite(manifest: &Path, edit: impl FnOnce(&mut serde_json::Value)) {
        let mut value: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(manifest).unwrap()).unwrap();
        edit(&mut value);
        std::fs::write(manifest, value.to_string()).unwrap();
    }

    #[test]
    fn opens_a_recording_whose_outputs_match_and_lists_its_corpus_pairs() {
        let root = tempfile::tempdir().unwrap();
        let manifest = set_up(root.path());
        let set = GoldenSet::open(&manifest, root.path()).unwrap().unwrap();
        assert_eq!(set.identity.commit, COMMIT);
        assert_eq!(
            set.identity.manifest,
            Sha256::of(std::fs::read(&manifest).unwrap())
        );
        assert_eq!(set.static_pairs(), ["demo.odp", "demo.pptx"]);
        assert_eq!(
            set.static_pair("demo", "pptx"),
            Some(root.path().join(COMMIT).join("corpus/demo.pptx"))
        );
        assert_eq!(set.static_pair("demo", "keynote"), None);
    }

    #[test]
    fn no_recording_yet_or_an_absent_folder_is_a_reason_not_an_error() {
        let root = tempfile::tempdir().unwrap();
        let absent = GoldenSet::open(&root.path().join("none.json"), root.path()).unwrap();
        assert!(absent.unwrap_err().contains("no golden manifest"));
        let manifest = set_up(root.path());
        let elsewhere = root.path().join("elsewhere");
        let absent = GoldenSet::open(&manifest, &elsewhere).unwrap();
        assert!(absent.unwrap_err().contains("no golden files"));
        rewrite(&manifest, |value| {
            value["commit"] = serde_json::json!("");
            value["outputs"] = serde_json::json!({});
        });
        let none = GoldenSet::open(&manifest, root.path()).unwrap();
        assert!(none.unwrap_err().contains("names no recording yet"));
    }

    #[test]
    fn a_changed_byte_or_a_missing_output_refuses_the_set() {
        let root = tempfile::tempdir().unwrap();
        let manifest = set_up(root.path());
        let file = root
            .path()
            .join(COMMIT)
            .join("corpus/demo.pptx/slides/s1.html");
        std::fs::write(&file, "<main></main>\n").unwrap();
        let error = GoldenSet::open(&manifest, root.path()).unwrap_err();
        assert!(
            error.to_string().contains("s1.html: its sha256 differs"),
            "{error}"
        );
        std::fs::remove_file(&file).unwrap();
        let error = GoldenSet::open(&manifest, root.path()).unwrap_err();
        assert!(error.to_string().contains("s1.html: missing"), "{error}");
    }

    #[test]
    fn a_symbolic_link_or_a_file_that_is_not_regular_is_refused_without_reading_it() {
        let root = tempfile::tempdir().unwrap();
        let manifest = set_up(root.path());
        let dir = root.path().join(COMMIT);
        // A link to an endless file in place of a pinned output.
        let report = dir.join("corpus/demo.odp/report.json");
        std::fs::remove_file(&report).unwrap();
        std::os::unix::fs::symlink("/dev/zero", &report).unwrap();
        let error = GoldenSet::open(&manifest, root.path()).unwrap_err();
        assert!(
            error.to_string().contains("report.json: a symbolic link"),
            "{error}"
        );
        // A link to a copy of the pinned file, out of the recording, is refused all the same.
        let outside = root.path().join("outside.json");
        std::fs::write(&outside, "{}").unwrap();
        std::fs::remove_file(&report).unwrap();
        std::os::unix::fs::symlink(&outside, &report).unwrap();
        let error = GoldenSet::open(&manifest, root.path()).unwrap_err();
        assert!(error.to_string().contains("a symbolic link"), "{error}");
        std::fs::remove_file(&report).unwrap();
        std::fs::write(&report, "{}").unwrap();
        GoldenSet::open(&manifest, root.path()).unwrap().unwrap();
        // A linked folder on the way.
        let pair = dir.join("corpus/demo.pptx");
        let moved = root.path().join("moved");
        std::fs::rename(&pair, &moved).unwrap();
        std::os::unix::fs::symlink(&moved, &pair).unwrap();
        let error = GoldenSet::open(&manifest, root.path()).unwrap_err();
        assert!(error.to_string().contains("a symbolic link"), "{error}");
        std::fs::remove_file(&pair).unwrap();
        std::fs::rename(&moved, &pair).unwrap();
        // A folder where a file is pinned.
        let fixture = dir.join("fixtures/basic/report.json");
        std::fs::remove_file(&fixture).unwrap();
        std::fs::create_dir(&fixture).unwrap();
        let error = GoldenSet::open(&manifest, root.path()).unwrap_err();
        assert!(error.to_string().contains("not a regular file"), "{error}");
        // A manifest that is itself a link.
        std::fs::remove_dir(&fixture).unwrap();
        std::fs::write(&fixture, "{}").unwrap();
        let linked = root.path().join("linked.json");
        std::os::unix::fs::symlink(&manifest, &linked).unwrap();
        let error = GoldenSet::open(&linked, root.path()).unwrap_err();
        assert!(error.to_string().contains("not a regular file"), "{error}");
    }

    #[test]
    fn a_read_past_its_bound_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("f");
        std::fs::write(&file, "12345").unwrap();
        assert_eq!(read_bounded(&file, 5).unwrap().unwrap(), b"12345");
        assert!(
            read_bounded(&file, 4)
                .unwrap_err()
                .to_string()
                .contains("larger than 4 bytes")
        );
        assert!(
            read_bounded(&root.path().join("none"), 4)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn a_path_outside_the_recording_a_bad_commit_or_another_version_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let manifest = set_up(root.path());
        let original = std::fs::read_to_string(&manifest).unwrap();
        for path in [
            "../escape.json",
            "/etc/passwd",
            "corpus//report.json",
            "./a",
            "",
        ] {
            std::fs::write(&manifest, &original).unwrap();
            rewrite(&manifest, |value| {
                value["outputs"][path] = serde_json::json!("0".repeat(64));
            });
            let error = GoldenSet::open(&manifest, root.path()).unwrap_err();
            assert!(
                error.to_string().contains("not a plain relative path"),
                "{path}: {error}"
            );
        }
        for commit in ["0123", "z123456789ab", "../0123456789ab"] {
            std::fs::write(&manifest, &original).unwrap();
            rewrite(&manifest, |value| {
                value["commit"] = serde_json::json!(commit)
            });
            let error = GoldenSet::open(&manifest, root.path()).unwrap_err();
            assert!(
                error.to_string().contains("not a hex commit"),
                "{commit}: {error}"
            );
        }
        std::fs::write(&manifest, &original).unwrap();
        rewrite(&manifest, |value| value["version"] = serde_json::json!(2));
        let error = GoldenSet::open(&manifest, root.path()).unwrap_err();
        assert!(error.to_string().contains("version 2"), "{error}");
    }
}
