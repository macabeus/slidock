//! Reading slidock's own repository: tree entries and file contents, from a revision or from the working copy.
//! Read-only: blob ids of the working copy are computed here, never written to the object store.

use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::time::Duration;

use sha1::{Digest, Sha1};

use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, args, exec, exec_text};

/// A file of a tree: its mode, git blob id and repository path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TreeEntry {
    pub mode: String,
    pub blob: String,
    pub path: String,
}

const GIT_TIMEOUT: Duration = Duration::from_secs(120);

async fn git(repo: &Path, arguments: &[&str]) -> Result<String> {
    let mut all = vec!["--no-optional-locks", "-C"];
    let repo = repo.to_string_lossy();
    all.push(&repo);
    all.extend_from_slice(arguments);
    exec_text("git", &args(all), &ExecOptions::timeout(GIT_TIMEOUT)).await
}

/// Whether `git` runs here; the tests that read the repository through it skip, saying so, when it does not.
#[cfg(test)]
pub(crate) fn installed() -> bool {
    let found = std::process::Command::new("git")
        .arg("--version")
        .output()
        .is_ok();
    if !found {
        eprintln!("skipped: `git` is not installed, so the repository is not read");
    }
    found
}

/// The git blob id of some bytes.
pub fn blob_id(bytes: &[u8]) -> String {
    let mut hasher = Sha1::new();
    hasher.update(format!("blob {}\0", bytes.len()).as_bytes());
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

/// Files under `paths` at a revision (submodules left out).
pub async fn tree_at(repo: &Path, rev: &str, paths: &[String]) -> Result<Vec<TreeEntry>> {
    let mut arguments = vec!["ls-tree", "-r", "--full-tree", rev, "--"];
    arguments.extend(paths.iter().map(String::as_str));
    let text = git(repo, &arguments).await?;
    let mut entries = Vec::new();
    for line in text.lines().filter(|line| !line.is_empty()) {
        let Some((meta, path)) = line.split_once('\t') else {
            continue;
        };
        let parts: Vec<&str> = meta.split(' ').collect();
        if let [mode, _, blob] = parts.as_slice()
            && *mode != "160000"
        {
            entries.push(TreeEntry {
                mode: (*mode).to_owned(),
                blob: (*blob).to_owned(),
                path: path.to_owned(),
            });
        }
    }
    Ok(entries)
}

/// Files under `paths` as they are on disk now (tracked and untracked, ignored files left out), hashed as blobs.
pub async fn tree_of_working_copy(repo: &Path, paths: &[String]) -> Result<Vec<TreeEntry>> {
    let mut arguments = vec![
        "ls-files",
        "--cached",
        "--others",
        "--exclude-standard",
        "--",
    ];
    arguments.extend(paths.iter().map(String::as_str));
    let listed = git(repo, &arguments).await?;
    let mut files: Vec<&str> = listed.lines().filter(|line| !line.is_empty()).collect();
    files.sort_unstable();
    files.dedup();
    let mut entries = Vec::new();
    for path in files {
        let full = repo.join(path);
        let Ok(meta) = std::fs::symlink_metadata(&full) else {
            continue;
        };
        let (mode, bytes) = if meta.file_type().is_symlink() {
            let target = std::fs::read_link(&full).map_err(BenchError::io(&full))?;
            ("120000", target.to_string_lossy().into_owned().into_bytes())
        } else {
            let mode = if meta.permissions().mode() & 0o111 != 0 {
                "100755"
            } else {
                "100644"
            };
            (mode, std::fs::read(&full).map_err(BenchError::io(&full))?)
        };
        entries.push(TreeEntry {
            mode: mode.to_owned(),
            blob: blob_id(&bytes),
            path: path.to_owned(),
        });
    }
    Ok(entries)
}

/// A file's bytes at a revision, or on disk when `rev` is `None`.
pub async fn file_at(repo: &Path, rev: Option<&str>, path: &str) -> Result<Vec<u8>> {
    let Some(rev) = rev else {
        let full = repo.join(path);
        return std::fs::read(&full).map_err(BenchError::io(&full));
    };
    let spec = format!("{rev}:{path}");
    let repo_arg = repo.to_string_lossy();
    let result = exec(
        "git",
        &args(["--no-optional-locks", "-C", &repo_arg, "show", &spec]),
        &ExecOptions::timeout(GIT_TIMEOUT),
    )
    .await?;
    if !result.ok() {
        return Err(BenchError::failed(format!(
            "{path} is not in {rev}: {}",
            result.stderr.trim()
        )));
    }
    Ok(result.stdout)
}

pub async fn head_commit(repo: &Path) -> Result<String> {
    Ok(git(repo, &["rev-parse", "HEAD"]).await?.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blob_ids_are_gits() {
        assert_eq!(blob_id(b""), "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391");
        assert_eq!(
            blob_id(b"hello\n"),
            "ce013625030ba8dba906f756967f9e9ca394464a"
        );
    }

    #[tokio::test]
    async fn working_copy_blobs_match_the_index() {
        if !installed() {
            return;
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let paths = vec!["rust-toolchain.toml".to_owned()];
        let ours = tree_of_working_copy(&repo, &paths).await.unwrap();
        let theirs = git(&repo, &["hash-object", "rust-toolchain.toml"])
            .await
            .unwrap();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0].blob, theirs.trim());
        assert_eq!(ours[0].mode, "100644");
    }
}
