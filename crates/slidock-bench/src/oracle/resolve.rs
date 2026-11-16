//! From a manifest presentation to its oracle pages: the PDF (Google's export in the corpus, or Keynote's export on
//! disk), its page sizes, and per page the raster and the words. Problems become row outcomes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use slidock_bench_metrics::RenderErrorReason;
use slidock_bench_schema::corpus::{Oracle, Presentation};
use slidock_bench_schema::{PresentationId, Sha256, Tier};
use tokio::sync::OnceCell;

use crate::corpus::Corpus;
use crate::error::{BenchError, Result};
use crate::hash::{sha256_file, sha256_json};
use crate::oracle::keynote::export_path;
use crate::oracle::pdf::{OraclePdf, PageSize, oracle_copy, page_sizes};
use crate::oracle::raster::{PageRaster, raster_key, raster_page, raster_size};
use crate::oracle::text_boxes::{PageText, page_text, text_key};
use crate::renderer::docker::Renderer;

#[derive(Clone, Debug)]
pub struct PresentationOracle {
    pub pdf: OraclePdf,
    pub pages: Vec<PageSize>,
}

/// Why an oracle cannot be used.
#[derive(Clone, Debug)]
pub struct OracleProblem {
    pub reason: RenderErrorReason,
    pub detail: String,
}

/// The key page hashes are recorded under: the raster key when every page shares one, else a hash of the keys.
pub fn presentation_raster_key(keys: &[String]) -> Result<String> {
    match keys {
        [first, rest @ ..] if rest.iter().all(|k| k == first) => Ok(first.clone()),
        _ => sha256_json(keys),
    }
}

/// The page raster hashes recorded for an oracle under this poppler.
pub enum Recorded {
    /// Every page's sha256.
    Hashes(Vec<Sha256>),
    /// No rasters are recorded under this raster key: `oracle --record` must run first.
    Missing,
    /// The holdout records none; the gate scores it once.
    Unchecked,
}

fn page_hashes(oracle: &Oracle) -> Option<&BTreeMap<String, Vec<Sha256>>> {
    match oracle {
        Oracle::PdfFile(o) => o.page_hashes.as_ref(),
        Oracle::KeynoteExport(o) => o.page_hashes.as_ref(),
    }
}

/// The key the pages of `pages` record their rasters under.
pub fn pages_key(pages: &[PageSize], poppler: &str) -> Result<String> {
    let keys = pages
        .iter()
        .map(|size| raster_key(raster_size(*size), poppler))
        .collect::<Result<Vec<_>>>()?;
    presentation_raster_key(&keys)
}

pub fn recorded_rasters(
    presentation: &Presentation,
    pages: &[PageSize],
    poppler: &str,
) -> Result<Recorded> {
    let key = pages_key(pages, poppler)?;
    Ok(
        match page_hashes(&presentation.oracle).and_then(|h| h.get(&key)) {
            Some(hashes) => Recorded::Hashes(hashes.clone()),
            None if presentation.tier == Tier::Holdout => Recorded::Unchecked,
            None => Recorded::Missing,
        },
    )
}

/// Why a Keynote export differs from the presentation's recorded oracle, if it does. The holdout records none.
pub fn export_identity(presentation: &Presentation, sha256: &str) -> Option<OracleProblem> {
    let Oracle::KeynoteExport(oracle) = &presentation.oracle else {
        return None;
    };
    if presentation.tier == Tier::Holdout
        || oracle.sha256.as_ref().map(Sha256::as_str) == Some(sha256)
    {
        return None;
    }
    let detail = if oracle.sha256.is_some() {
        "the export on disk is not the recorded one"
    } else {
        "no export sha256 is recorded"
    };
    Some(OracleProblem {
        reason: RenderErrorReason::OracleIdentity,
        detail: format!("{detail}; run `slidock-bench oracle --record`"),
    })
}

/// The file an oracle PDF is read from: the corpus PDF, or the Keynote export of the oracle's source.
pub fn oracle_file(
    corpus: &Corpus,
    keynote_dir: &Path,
    presentation: &Presentation,
) -> Result<std::result::Result<(PathBuf, Option<String>), OracleProblem>> {
    let missing = |detail: String| {
        Ok(Err(OracleProblem {
            reason: RenderErrorReason::OracleMissing,
            detail,
        }))
    };
    match &presentation.oracle {
        Oracle::PdfFile(oracle) => {
            let Some(sha) = &oracle.sha256 else {
                return missing("the oracle has no recorded sha256".to_owned());
            };
            match corpus.resolve(&oracle.file, sha.as_str())? {
                Some(path) => Ok(Ok((path, Some(sha.as_str().to_owned())))),
                None => missing(format!("{} is not in the corpus", oracle.file)),
            }
        }
        Oracle::KeynoteExport(oracle) => {
            let source = presentation
                .sources
                .iter()
                .find(|s| s.file == oracle.source);
            let Some(sha) = source.and_then(|s| s.sha256.as_ref()) else {
                return missing(format!("{} is not a hashed source", oracle.source));
            };
            let path = export_path(keynote_dir, sha.as_str(), &oracle.exporter);
            if path.is_file() {
                Ok(Ok((path, None)))
            } else {
                missing(
                    "no Keynote export on disk; run `slidock-bench oracle` (attended)".to_owned(),
                )
            }
        }
    }
}

/// The presentation's oracle PDF, copied where the renderer sees it. The caller has verified the corpus hashes; a
/// Keynote export must be the one the manifest records, unless `recording` (`oracle --record` checks it itself).
pub async fn presentation_oracle(
    renderer: &Renderer,
    corpus: &Corpus,
    keynote_dir: &Path,
    presentation: &Presentation,
    recording: bool,
) -> Result<std::result::Result<PresentationOracle, OracleProblem>> {
    let (file, known) = match oracle_file(corpus, keynote_dir, presentation)? {
        Ok(found) => found,
        Err(problem) => return Ok(Err(problem)),
    };
    let sha = match known {
        Some(sha) => sha,
        None => {
            let sha = sha256_file(&file)?;
            if !recording && let Some(problem) = export_identity(presentation, &sha) {
                return Ok(Err(problem));
            }
            sha
        }
    };
    let pdf = oracle_copy(&renderer.work, &file, &sha)?;
    let pages = page_sizes(renderer, &pdf).await?;
    Ok(Ok(PresentationOracle { pdf, pages }))
}

/// One oracle page: its raster (with its size and key) and its words.
#[derive(Clone, Debug)]
pub struct OraclePage {
    pub size: PageSize,
    pub raster: PageRaster,
    pub text: PageText,
}

/// One page's raster and words (1-based page).
pub async fn oracle_page(
    renderer: &Renderer,
    oracle: &PresentationOracle,
    page: u32,
    poppler: &str,
    no_cache: bool,
) -> Result<OraclePage> {
    let size = *page
        .checked_sub(1)
        .and_then(|i| oracle.pages.get(i as usize))
        .ok_or_else(|| BenchError::failed(format!("the oracle has no page {page}")))?;
    let pixels = raster_size(size);
    let key = raster_key(pixels, poppler)?;
    let raster = raster_page(renderer, &oracle.pdf, page, pixels, &key, no_cache).await?;
    let text = page_text(renderer, &oracle.pdf, page, &text_key(poppler)?, no_cache).await?;
    Ok(OraclePage { size, raster, text })
}

/// A page that could not be used: its raster differs from the recorded one, or extracting it failed.
#[derive(Clone, Debug)]
pub enum PageError {
    Identity(String),
    Failed(String),
}

type PageCell = Arc<OnceCell<std::result::Result<Arc<OraclePage>, PageError>>>;

/// A presentation's oracle, resolved once per run; pages are rasterised on first use.
pub struct ResolvedOracle {
    renderer: Arc<Renderer>,
    poppler: String,
    no_cache: bool,
    presentation: PresentationId,
    oracle: PresentationOracle,
    hashes: Option<Vec<Sha256>>,
    pages: Mutex<BTreeMap<u32, PageCell>>,
}

impl ResolvedOracle {
    pub fn page_count(&self) -> usize {
        self.oracle.pages.len()
    }

    /// The page, checked against its recorded raster.
    pub async fn page(&self, n: u32) -> std::result::Result<Arc<OraclePage>, PageError> {
        let cell = {
            let mut pages = self
                .pages
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            pages.entry(n).or_default().clone()
        };
        cell.get_or_init(|| async move {
            let data = oracle_page(
                &self.renderer,
                &self.oracle,
                n,
                &self.poppler,
                self.no_cache,
            )
            .await
            .map_err(|error| PageError::Failed(error.to_string()))?;
            if let Some(hashes) = &self.hashes {
                let recorded = n.checked_sub(1).and_then(|i| hashes.get(i as usize));
                if recorded.map(Sha256::as_str) != Some(data.raster.sha256.as_str()) {
                    return Err(PageError::Identity(format!(
                        "page {n} of {}'s oracle differs from its recorded raster",
                        self.presentation
                    )));
                }
            }
            Ok(Arc::new(data))
        })
        .await
        .clone()
    }
}

type Resolved = std::result::Result<Arc<ResolvedOracle>, RenderErrorReason>;
type ResolvedCell = Arc<OnceCell<std::result::Result<Resolved, String>>>;

/// Resolves each presentation's oracle once per run. An oracle with no rasters recorded under this raster key is
/// an `oracle-identity` problem for its pairs (run `oracle --record`).
pub struct OracleResolver {
    pub renderer: Arc<Renderer>,
    pub poppler: String,
    pub keynote_dir: PathBuf,
    pub no_cache: bool,
    resolved: Mutex<BTreeMap<PresentationId, ResolvedCell>>,
}

impl OracleResolver {
    pub fn new(
        renderer: Arc<Renderer>,
        poppler: String,
        keynote_dir: PathBuf,
        no_cache: bool,
    ) -> OracleResolver {
        OracleResolver {
            renderer,
            poppler,
            keynote_dir,
            no_cache,
            resolved: Mutex::new(BTreeMap::new()),
        }
    }

    pub async fn resolve(&self, corpus: &Corpus, presentation: &Presentation) -> Result<Resolved> {
        let cell = {
            let mut resolved = self
                .resolved
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            resolved.entry(presentation.id.clone()).or_default().clone()
        };
        cell.get_or_init(|| async {
            self.resolve_once(corpus, presentation)
                .await
                .map_err(|error| error.to_string())
        })
        .await
        .clone()
        .map_err(BenchError::Failed)
    }

    async fn resolve_once(&self, corpus: &Corpus, presentation: &Presentation) -> Result<Resolved> {
        let oracle = match presentation_oracle(
            &self.renderer,
            corpus,
            &self.keynote_dir,
            presentation,
            false,
        )
        .await?
        {
            Ok(oracle) => oracle,
            Err(problem) => return Ok(Err(problem.reason)),
        };
        let hashes = match recorded_rasters(presentation, &oracle.pages, &self.poppler)? {
            Recorded::Hashes(hashes) => Some(hashes),
            Recorded::Unchecked => None,
            Recorded::Missing => return Ok(Err(RenderErrorReason::OracleIdentity)),
        };
        Ok(Ok(Arc::new(ResolvedOracle {
            renderer: self.renderer.clone(),
            poppler: self.poppler.clone(),
            no_cache: self.no_cache,
            presentation: presentation.id.clone(),
            oracle,
            hashes,
            pages: Mutex::new(BTreeMap::new()),
        })))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_key_or_the_hash_of_all() {
        let a = "a".repeat(64);
        let b = "b".repeat(64);
        assert_eq!(presentation_raster_key(&[a.clone(), a.clone()]).unwrap(), a);
        assert_eq!(
            presentation_raster_key(&[a.clone(), b.clone()]).unwrap(),
            sha256_json(&[a, b]).unwrap()
        );
    }
}
