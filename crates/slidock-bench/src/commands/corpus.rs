//! `corpus locate`.

use crate::corpus::{Corpus, corpus_files, load_manifest};
use crate::error::{Exit, Result};
use crate::hash::FileHashes;
use crate::setup::Context;

/// Prints each non-holdout corpus file's resolution by sha256.
pub fn locate(ctx: &Context) -> Result<Exit> {
    let manifest = load_manifest(&ctx.env.manifest_path)?;
    let dir = ctx.env.require_corpus_dir()?;
    let corpus = Corpus::new(dir, FileHashes::new(&ctx.cache.hashes, true), &manifest);
    let mut ok = true;
    for presentation in manifest
        .presentations
        .iter()
        .filter(|p| p.tier != slidock_bench_schema::Tier::Holdout)
    {
        for file in corpus_files(presentation) {
            let found = match &file.sha256 {
                Some(sha) => corpus.resolve(&file.file, sha)?,
                None => None,
            };
            let sha = file.sha256.as_deref().unwrap_or("-");
            match found {
                Some(path) => println!("{} {sha} {}", presentation.id, path.display()),
                None => {
                    ok = false;
                    println!("{} {sha} missing ({})", presentation.id, file.file);
                }
            }
        }
    }
    Ok(Exit::check(ok))
}
