//! `fonts lane-lock`: records the Google part of `bench/fonts/fonts-lane.lock.json` from the CLI's own fetches.
//!
//! Every lane deck is compiled as the lane compiles it, into a fresh CLI cache holding only the substitutes. A first
//! compile with no network lists the `css2` requests the deck makes (`warning font GET …`); a compile with the
//! network then lets the CLI fetch them and the files they name, with its own `User-Agent`, into the cache; a last
//! compile with no network must make no request. The lock then pins each `css2` URL with its response and every file
//! the responses name with its sha256; nothing else may be in the cache.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use slidock_bench_schema::Tier;
use slidock_fonts::google::{CSS2_ENDPOINT, google_catalog, parse_css2};

use super::lock::{
    Css2Response, GoogleLock, LANE_LOCK, LaneLock, cli_cache_name, load_lane_lock, parse_lane_lock,
    prepare, write_cli_cache,
};
use super::{FontsRequest, RendererChoice, blocked_fetches, deck_arguments, lane_env, lane_pairs};
use crate::cache::{remove, write_json};
use crate::compile::{COMPILE_TIMEOUT, CompileStatus, compiler_binary, run_compiler_with};
use crate::corpus::load_manifest;
use crate::error::{BenchError, Exit, Result};
use crate::fonts::{FontEntry, FontSource};
use crate::hash::sha256;
use crate::setup::Context;

/// A cached body of the record's CLI cache.
fn cached(cache: &Path, url: &str) -> Result<Vec<u8>> {
    let path = cache.join("google").join(cli_cache_name(url));
    std::fs::read(&path).map_err(BenchError::io(&path))
}

/// The Google part of the lock for these `css2` requests, from a CLI cache that holds their answers. Every entry of
/// the cache's `google/` must be one of them or a file they name.
pub fn google_lock(cache: &Path, requests: &BTreeSet<String>) -> Result<GoogleLock> {
    let mut css2 = Vec::new();
    let mut files: BTreeMap<String, FontEntry> = BTreeMap::new();
    for url in requests {
        let css = String::from_utf8(cached(cache, url)?)
            .map_err(|_| BenchError::failed(format!("{url}: the response is not UTF-8")))?;
        let faces =
            parse_css2(&css).map_err(|error| BenchError::failed(format!("{url}: {error}")))?;
        for face in faces {
            if files.contains_key(&face.url) {
                continue;
            }
            let bytes = cached(cache, &face.url)?;
            let licence = google_catalog()
                .find(&face.family)
                .map(|found| found.family.licence.spdx())
                .ok_or_else(|| {
                    BenchError::failed(format!("{}: not a catalog family", face.family))
                })?;
            let file = face.url.rsplit('/').next().unwrap_or_default().to_owned();
            files.insert(
                face.url.clone(),
                FontEntry {
                    family: face.family.clone(),
                    file,
                    sha256: sha256(&bytes),
                    licence: licence.to_owned(),
                    source: FontSource::Url {
                        url: face.url.clone(),
                        member: None,
                    },
                },
            );
        }
        css2.push(Css2Response {
            url: url.clone(),
            sha256: sha256(&css),
            css,
        });
    }
    let known: BTreeSet<String> = requests
        .iter()
        .chain(files.keys())
        .map(|url| cli_cache_name(url))
        .collect();
    let dir = cache.join("google");
    let mut unknown = Vec::new();
    for entry in std::fs::read_dir(&dir).map_err(BenchError::io(&dir))? {
        let name = entry
            .map_err(BenchError::io(&dir))?
            .file_name()
            .to_string_lossy()
            .into_owned();
        if !known.contains(&name) {
            unknown.push(name);
        }
    }
    if !unknown.is_empty() {
        return Err(BenchError::failed(format!(
            "the CLI fetched what no recorded css2 request names: {}",
            unknown.join(", ")
        )));
    }
    let mut files: Vec<FontEntry> = files.into_values().collect();
    files.sort_by(|a, b| (&a.family, &a.file).cmp(&(&b.family, &b.file)));
    Ok(GoogleLock { css2, files })
}

/// Records the lock's Google part over every lane deck (the corpus's smoke and real tiers and the probes).
pub async fn record_lane_lock(ctx: &Context) -> Result<Exit> {
    let repo = &ctx.env.repo_root;
    let lock_path = repo.join(LANE_LOCK);
    let fonts = prepare(&lock_path, &ctx.cache).await?;
    let lock = load_lane_lock(&lock_path)?;
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    let request = FontsRequest {
        tiers: vec![Tier::Smoke, Tier::Real, Tier::Probe],
        presentations: None,
        frontends: None,
        only: None,
        concurrency: 1,
        renderer: RendererChoice::Container,
    };
    let pairs = lane_pairs(ctx, &manifest, &request)?;
    let binary = compiler_binary(repo, &ctx.env.target_dir, ctx.env.compiler.as_deref()).await?;
    let work = fonts.root.join("work").join("record");
    remove(&work)?;
    let cache = work.join("cli-cache");
    let substitutes_only = LaneLock {
        google: GoogleLock::default(),
        ..lock.clone()
    };
    write_cli_cache(&substitutes_only, &fonts.root.join("files"), &cache)?;
    let blocked = lane_env(&cache);
    let open = BTreeMap::from([(
        "SLIDOCK_CACHE_DIR".to_owned(),
        cache.to_string_lossy().into_owned(),
    )]);
    let mut requests = BTreeSet::new();
    for lane_pair in &pairs {
        let pair = &lane_pair.pair;
        let out = work.join("out");
        let arguments = deck_arguments(&lane_pair.file, pair.frontend(), &out);
        let compile = |env: BTreeMap<String, String>| {
            let arguments = arguments.clone();
            let binary = binary.clone();
            let out = out.clone();
            async move {
                remove(&out)?;
                let compiled =
                    run_compiler_with(&binary, &arguments, COMPILE_TIMEOUT, false, env).await?;
                if !matches!(compiled.status, CompileStatus::Ok | CompileStatus::Errors) {
                    return Err(BenchError::failed(format!(
                        "the compile ended {:?}",
                        compiled.status
                    )));
                }
                Ok(compiled)
            }
        };
        let asked = blocked_fetches(&compile(blocked.clone()).await?.stderr);
        if let Some(other) = asked
            .iter()
            .find(|url| !url.starts_with(&format!("{CSS2_ENDPOINT}?")))
        {
            return Err(BenchError::failed(format!(
                "{pair}: the compile fetches {other}, which is not a css2 request (a substitute missing from the lock?)"
            )));
        }
        if !asked.is_empty() {
            let failed = blocked_fetches(&compile(open.clone()).await?.stderr);
            if !failed.is_empty() {
                return Err(BenchError::failed(format!(
                    "{pair}: the CLI could not fetch {}",
                    failed.join(", ")
                )));
            }
            let left = blocked_fetches(&compile(blocked.clone()).await?.stderr);
            if !left.is_empty() {
                return Err(BenchError::failed(format!(
                    "{pair}: the cache still lacks {}",
                    left.join(", ")
                )));
            }
        }
        println!("{pair}: {} css2 requests not asked before", asked.len());
        requests.extend(asked);
    }
    let google = google_lock(&cache, &requests)?;
    let recorded = LaneLock { google, ..lock };
    parse_lane_lock(
        &serde_json::to_string(&recorded).map_err(|error| BenchError::failed(error.to_string()))?,
    )?;
    write_json(&lock_path, &recorded)?;
    remove(&work)?;
    println!(
        "{} css2 responses and {} files recorded into {}; review the diff",
        recorded.google.css2.len(),
        recorded.google.files.len(),
        lock_path.display()
    );
    Ok(Exit::Ok)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::write_atomic;

    const OSWALD: &str = "https://fonts.googleapis.com/css2?family=Oswald:wght@500&display=block";
    const FILE: &str = "https://fonts.gstatic.com/s/oswald/v57/medium.ttf";

    fn cache_with(entries: &[(&str, &[u8])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (url, body) in entries {
            write_atomic(&dir.path().join("google").join(cli_cache_name(url)), body).unwrap();
        }
        dir
    }

    fn response() -> String {
        format!(
            "@font-face {{\n  font-family: 'Oswald';\n  font-style: normal;\n  font-weight: 500;\n  \
             font-display: block;\n  src: url({FILE}) format('truetype');\n}}\n"
        )
    }

    #[test]
    fn the_cache_becomes_the_lock_s_google_part() {
        let css = response();
        let dir = cache_with(&[(OSWALD, css.as_bytes()), (FILE, b"font")]);
        let google = google_lock(dir.path(), &BTreeSet::from([OSWALD.to_owned()])).unwrap();
        assert_eq!(google.css2.len(), 1);
        assert_eq!(google.css2[0].sha256, sha256(&css));
        assert_eq!(google.files.len(), 1);
        let file = &google.files[0];
        assert_eq!(
            (
                file.family.as_str(),
                file.file.as_str(),
                file.licence.as_str()
            ),
            ("Oswald", "medium.ttf", "OFL-1.1")
        );
        assert_eq!(file.sha256, sha256(b"font"));
        assert_eq!(
            super::super::lock::google_problems(&google),
            Vec::<String>::new()
        );
    }

    #[test]
    fn a_fetch_no_recorded_request_names_is_refused() {
        let css = response();
        let dir = cache_with(&[
            (OSWALD, css.as_bytes()),
            (FILE, b"font"),
            ("https://fonts.gstatic.com/s/x.ttf", b"x"),
        ]);
        let error = google_lock(dir.path(), &BTreeSet::from([OSWALD.to_owned()]))
            .unwrap_err()
            .to_string();
        assert!(
            error.contains(&cli_cache_name("https://fonts.gstatic.com/s/x.ttf")),
            "{error}"
        );

        let dir = cache_with(&[(OSWALD, css.as_bytes())]);
        assert!(google_lock(dir.path(), &BTreeSet::from([OSWALD.to_owned()])).is_err());
    }
}
