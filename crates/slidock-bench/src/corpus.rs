//! The corpus as the bench reads it: the manifest, the (presentation, source) pairs a
//! selection names, the holdout wall, and content-addressed resolution of corpus files: a manifest `file` is a name
//! hint, and a file is found by its sha256.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use slidock_bench_schema::corpus::{CorpusManifest, Oracle, Presentation, Source};
use slidock_bench_schema::{Frontend, PresentationId, RowId, Tier};

use crate::cache::write_atomic;
use crate::error::{BenchError, Result};
use crate::hash::FileHashes;

pub fn load_manifest(path: &Path) -> Result<CorpusManifest> {
    let text = std::fs::read_to_string(path).map_err(BenchError::io(path))?;
    CorpusManifest::parse(&text)
        .map_err(|error| BenchError::failed(format!("{}: {error}", path.display())))
}

/// The manifest as kept: canonical JSON, two-space indent.
pub fn save_manifest(path: &Path, manifest: &CorpusManifest) -> Result<()> {
    write_atomic(path, manifest.to_canonical()?)
}

/// What a command selects.
#[derive(Clone, Debug, Default)]
pub struct Selection {
    pub tiers: Vec<Tier>,
    pub presentations: Option<Vec<PresentationId>>,
    pub frontends: Option<Vec<Frontend>>,
}

/// The (presentation, source) pairs a selection names, in manifest order.
pub fn select_pairs<'a>(
    manifest: &'a CorpusManifest,
    selection: &Selection,
) -> Vec<(&'a Presentation, &'a Source)> {
    manifest
        .presentations
        .iter()
        .filter(|p| selection.tiers.contains(&p.tier))
        .filter(|p| {
            selection
                .presentations
                .as_ref()
                .is_none_or(|ids| ids.contains(&p.id))
        })
        .flat_map(|p| {
            p.sources
                .iter()
                .filter(|s| {
                    selection
                        .frontends
                        .as_ref()
                        .is_none_or(|fronts| fronts.contains(&s.frontend))
                })
                .map(move |s| (p, s))
        })
        .collect()
}

fn holdout(manifest: &CorpusManifest) -> impl Iterator<Item = &Presentation> {
    manifest
        .presentations
        .iter()
        .filter(|p| p.tier == Tier::Holdout)
}

/// The holdout's presentation ids.
pub fn holdout_ids(manifest: &CorpusManifest) -> std::collections::BTreeSet<PresentationId> {
    holdout(manifest).map(|p| p.id.clone()).collect()
}

/// Every corpus file name of a holdout presentation (sources and oracle).
pub fn holdout_file_names(manifest: &CorpusManifest) -> Vec<String> {
    let mut names = Vec::new();
    for presentation in holdout(manifest) {
        names.extend(presentation.sources.iter().map(|s| s.file.clone()));
        names.push(match &presentation.oracle {
            Oracle::PdfFile(oracle) => oracle.file.clone(),
            Oracle::KeynoteExport(oracle) => oracle.source.clone(),
        });
    }
    names
}

/// Refuses (exit 2) when a tier, presentation or row names the holdout. Every command but the gate calls this
/// before it reads any corpus file.
pub fn refuse_holdout(
    manifest: &CorpusManifest,
    tiers: &[Tier],
    presentations: &[PresentationId],
    rows: &[RowId],
    command: &str,
) -> Result<()> {
    let named = presentations
        .iter()
        .chain(rows.iter().map(RowId::presentation));
    let holdout_named = named
        .into_iter()
        .any(|id| holdout(manifest).any(|p| &p.id == id));
    if tiers.contains(&Tier::Holdout) || holdout_named {
        return Err(BenchError::refused(format!(
            "{command} never reads the holdout; only the gate scores it"
        )));
    }
    Ok(())
}

/// A file a presentation reads from the corpus: its sources and, for a PDF oracle, the oracle.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CorpusFile {
    pub file: String,
    pub sha256: Option<String>,
}

pub fn corpus_files(presentation: &Presentation) -> Vec<CorpusFile> {
    let mut files: Vec<CorpusFile> = presentation
        .sources
        .iter()
        .map(|s| CorpusFile {
            file: s.file.clone(),
            sha256: s.sha256.as_ref().map(|sha| sha.as_str().to_owned()),
        })
        .collect();
    if let Oracle::PdfFile(oracle) = &presentation.oracle
        && !files.iter().any(|f| f.file == oracle.file)
    {
        files.push(CorpusFile {
            file: oracle.file.clone(),
            sha256: oracle.sha256.as_ref().map(|sha| sha.as_str().to_owned()),
        });
    }
    files
}

/// A corpus file that is missing or differs from the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HashCheck {
    pub file: String,
    pub expected: Option<String>,
}

/// The corpus folder, resolved by content: a file is looked up by its name hint first and, when that misses, in an
/// index of the folder by sha256 (holdout files left unread).
pub struct Corpus {
    dir: PathBuf,
    hashes: FileHashes,
    skip: Vec<String>,
    index: Mutex<Option<BTreeMap<String, PathBuf>>>,
}

impl Corpus {
    pub fn new(dir: &Path, hashes: FileHashes, manifest: &CorpusManifest) -> Corpus {
        Corpus {
            dir: dir.to_path_buf(),
            hashes,
            skip: holdout_file_names(manifest),
            index: Mutex::new(None),
        }
    }

    /// The path of the file with this sha256, if the corpus holds it.
    pub fn resolve(&self, hint: &str, sha256: &str) -> Result<Option<PathBuf>> {
        let hinted = self.dir.join(hint);
        if hinted.is_file() && self.hashes.of(&hinted)? == sha256 {
            return Ok(Some(hinted));
        }
        let mut index = self
            .index
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if index.is_none() {
            *index = Some(self.build_index()?);
        }
        Ok(index.as_ref().and_then(|map| map.get(sha256).cloned()))
    }

    fn build_index(&self) -> Result<BTreeMap<String, PathBuf>> {
        let mut map = BTreeMap::new();
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&self.dir)
            .map_err(BenchError::io(&self.dir))?
            .filter_map(|entry| entry.ok().map(|entry| entry.path()))
            .collect();
        entries.sort();
        for path in entries {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            if name.starts_with('.') || self.skip.contains(&name) || !path.is_file() {
                continue;
            }
            let sha = self.hashes.of(&path)?;
            map.entry(sha).or_insert(path);
        }
        Ok(map)
    }

    /// The file of a source, by its sha256.
    pub fn source_file(&self, source: &Source) -> Result<Option<PathBuf>> {
        match &source.sha256 {
            Some(sha) => self.resolve(&source.file, sha.as_str()),
            None => Ok(None),
        }
    }

    /// Every file of the presentation that is missing or differs from the manifest. Empty when all match.
    pub fn verify(&self, presentation: &Presentation) -> Result<Vec<HashCheck>> {
        let mut problems = Vec::new();
        for file in corpus_files(presentation) {
            let found = match &file.sha256 {
                Some(sha) => self.resolve(&file.file, sha)?,
                None => None,
            };
            if found.is_none() {
                problems.push(HashCheck {
                    file: file.file,
                    expected: file.sha256,
                });
            }
        }
        Ok(problems)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use slidock_bench_schema::Sha256;

    fn manifest(dir: &Path) -> CorpusManifest {
        let deck = std::fs::read(dir.join("a.pptx")).unwrap();
        let pdf = std::fs::read(dir.join("a.pdf")).unwrap();
        let text = serde_json::json!({
            "version": 1,
            "presentations": [
                {
                    "id": "demo",
                    "tier": "smoke",
                    "sources": [{
                        "frontend": "pptx",
                        "file": "renamed.pptx",
                        "sha256": Sha256::of(&deck).as_str(),
                        "bytes": deck.len(),
                        "rows": ["pp-1"]
                    }],
                    "oracle": {"kind": "pdf-file", "file": "a.pdf", "sha256": Sha256::of(&pdf).as_str()}
                },
                {
                    "id": "secret",
                    "tier": "holdout",
                    "sources": [{"frontend": "pptx", "file": "secret.pptx", "sha256": null, "bytes": null}],
                    "oracle": {"kind": "pdf-file", "file": "secret.pdf", "sha256": null}
                }
            ]
        });
        CorpusManifest::parse(&text.to_string()).unwrap()
    }

    fn corpus() -> (tempfile::TempDir, CorpusManifest) {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.pptx"), "deck").unwrap();
        std::fs::write(dir.path().join("a.pdf"), "oracle").unwrap();
        std::fs::write(dir.path().join("secret.pptx"), "holdout").unwrap();
        let manifest = manifest(dir.path());
        (dir, manifest)
    }

    #[test]
    fn files_resolve_by_content_not_by_name() {
        let (dir, manifest) = corpus();
        let hashes = FileHashes::new(&dir.path().join(".hashes.json"), false);
        let corpus = Corpus::new(dir.path(), hashes, &manifest);
        let presentation = &manifest.presentations[0];
        assert_eq!(
            corpus.source_file(&presentation.sources[0]).unwrap(),
            Some(dir.path().join("a.pptx"))
        );
        assert!(corpus.verify(presentation).unwrap().is_empty());
        std::fs::write(dir.path().join("a.pdf"), "edited").unwrap();
        let hashes = FileHashes::new(&dir.path().join(".hashes.json"), false);
        let corpus = Corpus::new(dir.path(), hashes, &manifest);
        let problems = corpus.verify(presentation).unwrap();
        assert_eq!(problems.len(), 1);
        assert_eq!(problems[0].file, "a.pdf");
    }

    #[test]
    fn the_holdout_is_refused_and_never_indexed() {
        let (dir, manifest) = corpus();
        let secret: PresentationId = "secret".parse().unwrap();
        assert!(refuse_holdout(&manifest, &[], &[secret], &[], "run").is_err());
        assert!(refuse_holdout(&manifest, &[Tier::Holdout], &[], &[], "run").is_err());
        let row: RowId = "secret:pp-1:pptx".parse().unwrap();
        assert!(refuse_holdout(&manifest, &[], &[], &[row], "run").is_err());
        assert!(refuse_holdout(&manifest, &Tier::DEFAULT, &[], &[], "run").is_ok());
        let hashes = FileHashes::new(&dir.path().join(".hashes.json"), false);
        let corpus = Corpus::new(dir.path(), hashes, &manifest);
        let index = corpus.build_index().unwrap();
        assert!(!index.values().any(|path| path.ends_with("secret.pptx")));
        assert_eq!(index.len(), 2);
    }

    #[test]
    fn selections_keep_manifest_order() {
        let (_dir, manifest) = corpus();
        let all = Selection {
            tiers: Tier::DEFAULT.to_vec(),
            ..Selection::default()
        };
        assert_eq!(select_pairs(&manifest, &all).len(), 1);
        let none = Selection {
            frontends: Some(vec![Frontend::Odp]),
            ..all
        };
        assert!(select_pairs(&manifest, &none).is_empty());
    }
}
