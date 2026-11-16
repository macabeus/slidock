//! Where the bench reads and writes. The repository holds code and public configuration (`bench/`); the corpus,
//! its manifest, the oracles, the cache and the baseline are private and live under `$SLIDOCK_PRIVATE`
//! (default `~/.slidock`), never in the repository.

use std::path::{Path, PathBuf};

use crate::error::{BenchError, Result};

#[derive(Clone, Debug)]
pub struct BenchEnv {
    /// The repository root this binary was built from.
    pub repo_root: PathBuf,
    /// `bench/`: the renderer recipe and the font lock.
    pub bench_dir: PathBuf,
    /// `SLIDOCK_CORPUS_DIR`, or `$SLIDOCK_PRIVATE/corpus`: the corpus files, read-only, found by sha256.
    pub corpus_dir: PathBuf,
    /// `SLIDOCK_BENCH_CACHE`, or `$SLIDOCK_PRIVATE/bench/cache`.
    pub cache_dir: PathBuf,
    /// `SLIDOCK_CORPUS_MANIFEST`, or `$SLIDOCK_PRIVATE/bench/corpus/manifest.json`.
    pub manifest_path: PathBuf,
    /// `SLIDOCK_KEYNOTE_ORACLES`, or `$SLIDOCK_PRIVATE/bench/oracles/keynote`: Keynote's exports,
    /// `<sourceSha>/<exporterId>.pdf`, written only by the attended `oracle` command.
    pub keynote_dir: PathBuf,
    /// `SLIDOCK_STAGES_ORACLES`, or `$SLIDOCK_PRIVATE/oracles/keynote-stages`: Keynote's `all stages` exports,
    /// `<sourceSha>/*.pdf`, the motion lane's stage oracles.
    pub stages_dir: PathBuf,
    /// `SLIDOCK_MOVIE_ORACLES`, or `$SLIDOCK_PRIVATE/oracles/keynote-movies`: Keynote's movie exports,
    /// `<sourceSha>/*.m4v`, the motion lane's transition oracles.
    pub movies_dir: PathBuf,
    /// `SLIDOCK_BENCH_BASELINE`, or `$SLIDOCK_PRIVATE/bench/baseline`: the baseline results, one file per lane.
    pub baseline_dir: PathBuf,
    /// `$SLIDOCK_PRIVATE/golden`, where `cargo xtask golden --record` writes the golden sets, one folder per
    /// recording commit (parity).
    pub golden_dir: PathBuf,
    /// `SLIDOCK_GOLDEN_MANIFEST`, or the committed golden manifest ([`crate::golden::MANIFEST`]): which set parity
    /// compares against.
    pub golden_manifest: PathBuf,
    pub fonts_lock_path: PathBuf,
    /// `SLIDOCK_COMPILER`: a prebuilt `slidock` binary; otherwise `run` builds the working copy's.
    pub compiler: Option<PathBuf>,
    /// Where cargo builds the compiler: `CARGO_TARGET_DIR`, or `target/` in the repository.
    pub target_dir: PathBuf,
}

impl BenchEnv {
    pub fn from_process() -> Result<BenchEnv> {
        let repo_root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .canonicalize()
            .map_err(BenchError::io(env!("CARGO_MANIFEST_DIR")))?;
        BenchEnv::from_vars(repo_root, |name| std::env::var(name).ok())
    }

    /// The paths under the variables `lookup` reads; an empty value counts as unset.
    pub fn from_vars(
        repo_root: PathBuf,
        lookup: impl Fn(&str) -> Option<String>,
    ) -> Result<BenchEnv> {
        let var = |name: &str| lookup(name).filter(|value| !value.is_empty());
        let private = match (var("SLIDOCK_PRIVATE"), var("HOME")) {
            (Some(dir), _) => PathBuf::from(dir),
            (None, Some(home)) => PathBuf::from(home).join(".slidock"),
            (None, None) => {
                return Err(BenchError::refused("HOME is not set; set SLIDOCK_PRIVATE"));
            }
        };
        let bench = private.join("bench");
        let path = |name: &str, default: PathBuf| var(name).map_or(default, PathBuf::from);
        Ok(BenchEnv {
            bench_dir: repo_root.join("bench"),
            corpus_dir: path("SLIDOCK_CORPUS_DIR", private.join("corpus")),
            cache_dir: path("SLIDOCK_BENCH_CACHE", bench.join("cache")),
            manifest_path: path(
                "SLIDOCK_CORPUS_MANIFEST",
                bench.join("corpus/manifest.json"),
            ),
            keynote_dir: path("SLIDOCK_KEYNOTE_ORACLES", bench.join("oracles/keynote")),
            stages_dir: path(
                "SLIDOCK_STAGES_ORACLES",
                private.join("oracles/keynote-stages"),
            ),
            movies_dir: path(
                "SLIDOCK_MOVIE_ORACLES",
                private.join("oracles/keynote-movies"),
            ),
            baseline_dir: path("SLIDOCK_BENCH_BASELINE", bench.join("baseline")),
            golden_dir: private.join("golden"),
            golden_manifest: path(
                "SLIDOCK_GOLDEN_MANIFEST",
                repo_root.join(crate::golden::MANIFEST),
            ),
            fonts_lock_path: repo_root.join("bench/fonts/fonts.lock.json"),
            compiler: var("SLIDOCK_COMPILER").map(PathBuf::from),
            target_dir: path("CARGO_TARGET_DIR", repo_root.join("target")),
            repo_root,
        })
    }

    /// The static lane's baseline file.
    pub fn baseline_path(&self) -> PathBuf {
        self.baseline_dir.join("results.json")
    }

    pub fn require_corpus_dir(&self) -> Result<&Path> {
        if self.corpus_dir.is_dir() {
            Ok(&self.corpus_dir)
        } else {
            Err(BenchError::refused(format!(
                "the corpus folder {} does not exist; set SLIDOCK_CORPUS_DIR",
                self.corpus_dir.display()
            )))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(vars: &[(&str, &str)]) -> BenchEnv {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
            .collect();
        BenchEnv::from_vars(PathBuf::from("/repo"), |name| {
            vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())
        })
        .unwrap()
    }

    #[test]
    fn the_goldens_sit_under_the_private_root_wherever_the_corpus_is() {
        let env = env_of(&[
            ("HOME", "/home/me"),
            ("SLIDOCK_PRIVATE", "/private-root"),
            ("SLIDOCK_CORPUS_DIR", "/data/corpus"),
        ]);
        assert_eq!(env.corpus_dir, Path::new("/data/corpus"));
        assert_eq!(env.golden_dir, Path::new("/private-root/golden"));
        assert_eq!(
            env.golden_manifest,
            Path::new("/repo/tests/golden/manifest.json")
        );
        let home = env_of(&[("HOME", "/home/me"), ("SLIDOCK_PRIVATE", "")]);
        assert_eq!(home.golden_dir, Path::new("/home/me/.slidock/golden"));
        // The recorder writes under the private root only, so the bench reads nowhere else.
        let named = env_of(&[
            ("HOME", "/home/me"),
            ("SLIDOCK_GOLDEN", "/elsewhere"),
            ("SLIDOCK_GOLDEN_MANIFEST", "/candidate.json"),
        ]);
        assert_eq!(named.golden_dir, Path::new("/home/me/.slidock/golden"));
        assert_eq!(named.golden_manifest, Path::new("/candidate.json"));
    }
}
