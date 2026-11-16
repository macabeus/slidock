//! The stamp's inputs: the compiler content id over the trees the compiler is built from, the `Cargo.lock` closure
//! digest, the toolchain versions, the host and the harness hash. Two runs compare only when their stamps agree on
//! everything but the compiler.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration;

use slidock_bench_schema::Sha256;
use slidock_bench_schema::stamp::{
    CompilerStamp, ContentEntry, DepsStamp, HarnessInputs, HostStamp, content_id,
};

use crate::error::{BenchError, Result};
use crate::exec::{ExecOptions, args, exec};
use crate::git::{TreeEntry, file_at, head_commit, tree_at, tree_of_working_copy};
use crate::hash::sha256;

/// The binary the compiler under test is built as; its `Cargo.lock` closure names the compiler crates.
pub const COMPILER_PACKAGE: &str = "slidock-cli";

/// Paths of a crate that do not reach its build output.
const OUTSIDE_THE_BUILD: &[&str] = &["tests/", "examples/", "benches/", "fuzz/"];

/// Files outside the crates that shape every build: the workspace manifest, the toolchain and the lockfile.
const BUILD_FILES: &[&str] = &["Cargo.toml", "Cargo.lock", "rust-toolchain.toml"];

/// Files outside a compiler crate that the crate embeds in what it writes (through its `build.rs` or
/// `include_str!`), by crate: the license the decks' notices carry. (The decks' player is the crate's own copy,
/// `crates/slidock-bundle/player/`.)
const EMBEDDED: &[(&str, &[&str])] = &[("slidock-bundle", &["LICENSE"])];

/// The bench's own trees, which the full content id adds to the compiler's.
const BENCH_CONTENT: &[&str] = &[
    "crates/slidock-bench",
    "crates/slidock-bench-metrics",
    "crates/slidock-bench-schema",
    "bench/docker",
    "bench/fonts",
    "bench/corpus",
];

/// What the harness hash covers besides the severities: everything that decides a score but the compiler.
const HARNESS_PATHS: &[&str] = &[
    "crates/slidock-bench/src",
    "crates/slidock-bench/Cargo.toml",
    "crates/slidock-bench-metrics/src",
    "crates/slidock-bench-metrics/Cargo.toml",
    "crates/slidock-bench-schema/src",
    "crates/slidock-bench-schema/Cargo.toml",
    "bench/docker",
    "bench/fonts/fonts.lock.json",
];

/// One `[[package]]` of a `Cargo.lock`: its identity and its dependency specs (`name`, `name version` or
/// `name version (source)`, whichever the lockfile needs to tell versions apart).
#[derive(Clone, Debug)]
struct LockPackage {
    name: String,
    version: String,
    /// Workspace members have none.
    source: Option<String>,
    checksum: Option<String>,
    dependencies: Vec<String>,
}

fn unquote(value: &str) -> String {
    value.trim().trim_matches('"').to_owned()
}

fn parse_lock(text: &str) -> Vec<LockPackage> {
    let mut packages = Vec::new();
    for block in text.split("[[package]]\n").skip(1) {
        let block = block.trim_end_matches('\n');
        let mut package = LockPackage {
            name: String::new(),
            version: String::new(),
            source: None,
            checksum: None,
            dependencies: Vec::new(),
        };
        let mut in_dependencies = false;
        for line in block.lines() {
            if in_dependencies {
                if line.starts_with(']') {
                    in_dependencies = false;
                } else {
                    package
                        .dependencies
                        .push(unquote(line.trim().trim_end_matches(',')));
                }
                continue;
            }
            if let Some((key, value)) = line.split_once(" = ") {
                match key {
                    "name" => package.name = unquote(value),
                    "version" => package.version = unquote(value),
                    "source" => package.source = Some(unquote(value)),
                    "checksum" => package.checksum = Some(unquote(value)),
                    "dependencies" if value.trim() == "[" => in_dependencies = true,
                    _ => {}
                }
            }
        }
        packages.push(package);
    }
    packages
}

/// The package a dependency spec names, when exactly one does.
fn find_package(packages: &[LockPackage], spec: &str) -> Option<usize> {
    let mut parts = spec.split(' ');
    let name = parts.next()?;
    let version = parts.next();
    let matching: Vec<usize> = packages
        .iter()
        .enumerate()
        .filter(|(_, p)| p.name == name && version.is_none_or(|v| p.version == v))
        .map(|(i, _)| i)
        .collect();
    match matching.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// A package as the digest reads it: its identity and its dependencies resolved to `name version`. Cargo spells a
/// dependency `name` or `name version` depending on which other versions the whole lockfile holds, so the spelling
/// would move the digest on an unrelated addition; the resolved form does not.
fn package_line(packages: &[LockPackage], package: &LockPackage) -> String {
    let mut dependencies: Vec<String> = package
        .dependencies
        .iter()
        .filter_map(|spec| find_package(packages, spec))
        .filter_map(|i| packages.get(i))
        .map(|d| format!("{} {}", d.name, d.version))
        .collect();
    dependencies.sort();
    format!(
        "{} {} {} {} [{}]",
        package.name,
        package.version,
        package.source.as_deref().unwrap_or("-"),
        package.checksum.as_deref().unwrap_or("-"),
        dependencies.join(", ")
    )
}

/// The packages reachable from `root`, by index into `packages`. A dependency the lockfile lacks fails, so a
/// broken closure never passes for an unchanged one.
fn lock_closure(packages: &[LockPackage], root: &str) -> Result<BTreeSet<usize>> {
    let find = |spec: &str| find_package(packages, spec);
    let start = packages
        .iter()
        .position(|p| p.name == root && p.source.is_none())
        .ok_or_else(|| BenchError::failed(format!("Cargo.lock has no workspace package {root}")))?;
    let mut seen = BTreeSet::new();
    let mut pending = vec![start];
    while let Some(index) = pending.pop() {
        if !seen.insert(index) {
            continue;
        }
        let Some(package) = packages.get(index) else {
            continue;
        };
        for spec in &package.dependencies {
            let found = find(spec).ok_or_else(|| {
                BenchError::failed(format!(
                    "Cargo.lock: {} depends on {spec}, which it lacks",
                    package.name
                ))
            })?;
            pending.push(found);
        }
    }
    Ok(seen)
}

/// The compiler's view of a `Cargo.lock`: the digest of the entries the compiler reaches, so an unrelated bump
/// leaves it alone, and the workspace crates among them.
struct LockClosure {
    digest: String,
    crates: Vec<String>,
}

fn closure_of(text: &str) -> Result<LockClosure> {
    let packages = parse_lock(text);
    let reached = lock_closure(&packages, COMPILER_PACKAGE)?;
    let mut texts: Vec<String> = Vec::new();
    let mut crates = Vec::new();
    for package in reached.iter().filter_map(|&i| packages.get(i)) {
        texts.push(package_line(&packages, package));
        if package.source.is_none() {
            crates.push(package.name.clone());
        }
    }
    texts.sort_by(|a, b| slidock_canon::utf16::cmp(a, b));
    crates.sort();
    Ok(LockClosure {
        digest: sha256(texts.join("\n")),
        crates,
    })
}

async fn lock_at(repo: &Path, rev: Option<&str>) -> Result<LockClosure> {
    let bytes = file_at(repo, rev, "Cargo.lock").await?;
    closure_of(&String::from_utf8_lossy(&bytes))
}

/// The repository paths of the compiler: its crates, what they embed and the build files.
fn compiler_paths(closure: &LockClosure) -> Vec<String> {
    let mut paths: Vec<String> = closure
        .crates
        .iter()
        .map(|name| format!("crates/{name}"))
        .collect();
    for (name, embedded) in EMBEDDED {
        if closure.crates.iter().any(|c| c == name) {
            paths.extend(embedded.iter().map(|path| (*path).to_owned()));
        }
    }
    paths.extend(BUILD_FILES.iter().map(|path| (*path).to_owned()));
    paths
}

/// Leaves out the files of a crate that do not reach its build (tests, examples, READMEs).
fn in_build(entry: &TreeEntry) -> bool {
    let Some(rest) = entry.path.strip_prefix("crates/") else {
        return true;
    };
    let inside = rest.split_once('/').map_or("", |(_, inside)| inside);
    !OUTSIDE_THE_BUILD.iter().any(|dir| inside.starts_with(dir)) && !inside.ends_with(".md")
}

/// Tree entries with `Cargo.lock` standing for its closure digest.
async fn content_entries(
    repo: &Path,
    rev: Option<&str>,
    with_bench: bool,
) -> Result<Vec<ContentEntry>> {
    let closure = lock_at(repo, rev).await?;
    let mut paths = compiler_paths(&closure);
    if with_bench {
        paths.extend(BENCH_CONTENT.iter().map(|path| (*path).to_owned()));
    }
    let entries = match rev {
        Some(rev) => tree_at(repo, rev, &paths).await?,
        None => tree_of_working_copy(repo, &paths).await?,
    };
    Ok(entries
        .into_iter()
        .filter(in_build)
        .map(|entry| ContentEntry {
            blob: if entry.path == "Cargo.lock" {
                closure.digest.clone()
            } else {
                entry.blob
            },
            mode: entry.mode,
            path: entry.path,
        })
        .collect())
}

/// The content id of the working copy (`rev` `None`) or of a revision. `with_bench` adds the bench's trees.
pub async fn content_id_at(repo: &Path, rev: Option<&str>, with_bench: bool) -> Result<Sha256> {
    Ok(content_id(&content_entries(repo, rev, with_bench).await?))
}

/// The working copy's content id (compiler and bench), HEAD's commit, and whether the working copy differs from
/// HEAD there.
pub async fn compiler_stamp(repo: &Path) -> Result<CompilerStamp> {
    let content = content_id_at(repo, None, true).await?;
    let head = content_id_at(repo, Some("HEAD"), true).await?;
    Ok(CompilerStamp {
        dirty: content != head,
        content,
        commit: head_commit(repo).await?,
    })
}

/// A tool's version from `<tool> --version`: the first token that starts with a digit, or `missing`.
async fn version_of(repo: &Path, tool: &str) -> String {
    let options = ExecOptions {
        cwd: Some(repo.to_path_buf()),
        ..ExecOptions::timeout(Duration::from_secs(30))
    };
    match exec(tool, &args(["--version"]), &options).await {
        Ok(result) if result.ok() => result
            .stdout_text()
            .split_whitespace()
            .map(|token| token.trim_start_matches('v'))
            .find(|token| token.starts_with(|c: char| c.is_ascii_digit()))
            .map_or_else(|| "missing".to_owned(), str::to_owned),
        _ => "missing".to_owned(),
    }
}

pub async fn deps_stamp(repo: &Path) -> Result<DepsStamp> {
    let node = version_of(repo, "node").await;
    Ok(DepsStamp {
        cargo_lock: Sha256::new(lock_at(repo, None).await?.digest)?,
        rustc: version_of(repo, "rustc").await,
        wasm_pack: version_of(repo, "wasm-pack").await,
        node: match node.split('.').next() {
            Some(major) if node != "missing" => format!("{major}.x"),
            _ => node,
        },
    })
}

/// The host as Node names it (`darwin`, `arm64`).
pub fn host_stamp() -> HostStamp {
    let platform = match std::env::consts::OS {
        "macos" => "darwin",
        other => other,
    };
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        "x86_64" => "x64",
        other => other,
    };
    HostStamp {
        platform: platform.to_owned(),
        arch: arch.to_owned(),
    }
}

/// `code → severity` from slidock-ir's registry: which diagnostics decline a row is part of the harness.
pub fn severity_map() -> BTreeMap<String, String> {
    slidock_ir::diagnostics::DiagnosticCode::ALL
        .iter()
        .map(|code| (code.code().to_owned(), code.severity().as_str().to_owned()))
        .collect()
}

/// The harness hash of the working copy, or of a revision.
pub async fn harness_hash(repo: &Path, rev: Option<&str>) -> Result<Sha256> {
    let paths: Vec<String> = HARNESS_PATHS
        .iter()
        .map(|path| (*path).to_owned())
        .collect();
    let entries = match rev {
        Some(rev) => tree_at(repo, rev, &paths).await?,
        None => tree_of_working_copy(repo, &paths).await?,
    };
    let mut files = BTreeMap::new();
    for entry in entries {
        let bytes = file_at(repo, rev, &entry.path).await?;
        files.insert(entry.path, Sha256::of(bytes));
    }
    Ok(HarnessInputs {
        files,
        severities: severity_map(),
    }
    .hash()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOCK: &str = r#"# This file is automatically @generated by Cargo.
version = 4

[[package]]
name = "a"
version = "1.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "00"

[[package]]
name = "b"
version = "2.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
checksum = "11"
dependencies = [
 "a",
]

[[package]]
name = "slidock-cli"
version = "0.1.0"
dependencies = [
 "b",
 "slidock-core",
]

[[package]]
name = "slidock-core"
version = "0.1.0"

[[package]]
name = "unrelated"
version = "9.0.0"
source = "registry+https://github.com/rust-lang/crates.io-index"
"#;

    #[test]
    fn the_closure_ignores_unrelated_packages() {
        let closure = closure_of(LOCK).unwrap();
        assert_eq!(closure.crates, ["slidock-cli", "slidock-core"]);
        let bumped = LOCK.replace("version = \"9.0.0\"", "version = \"9.0.1\"");
        assert_eq!(closure_of(&bumped).unwrap().digest, closure.digest);
        let moved = LOCK.replace("checksum = \"00\"", "checksum = \"01\"");
        assert_ne!(closure_of(&moved).unwrap().digest, closure.digest);
        // A second version of `a` elsewhere makes Cargo spell the dependency `a 1.0.0`: the same closure.
        let spelled = LOCK.replace(" \"a\",\n", " \"a 1.0.0\",\n").replace(
            "[[package]]\nname = \"unrelated\"",
            "[[package]]\nname = \"a\"\nversion = \"2.0.0\"\n\n[[package]]\nname = \"unrelated\"",
        );
        assert_eq!(closure_of(&spelled).unwrap().digest, closure.digest);
        let broken = LOCK.replace(" \"a\",\n", " \"missing\",\n");
        assert!(closure_of(&broken).is_err());
    }

    #[test]
    fn tests_and_readmes_stay_out_of_the_content() {
        let entry = |path: &str| TreeEntry {
            mode: "100644".into(),
            blob: String::new(),
            path: path.into(),
        };
        assert!(in_build(&entry("crates/slidock-ir/src/lib.rs")));
        assert!(in_build(&entry("crates/slidock-presets/data/x.json")));
        assert!(in_build(&entry("Cargo.toml")));
        assert!(!in_build(&entry("crates/slidock-ir/tests/golden.rs")));
        assert!(!in_build(&entry("crates/slidock-ir/README.md")));
    }

    #[tokio::test]
    async fn the_repository_closure_reaches_the_compiler_crates() {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let closure = lock_at(&repo, None).await.unwrap();
        for name in [
            "slidock-cli",
            "slidock",
            "slidock-ir",
            "slidock-format",
            "slidock-html",
        ] {
            assert!(closure.crates.iter().any(|c| c == name), "{name}");
        }
        assert!(
            !closure
                .crates
                .iter()
                .any(|c| c.starts_with("slidock-bench"))
        );
    }

    #[test]
    fn the_compiler_paths_cover_what_its_crates_embed() {
        let paths = compiler_paths(&closure_of(LOCK).unwrap());
        assert!(!paths.iter().any(|p| p.starts_with("crates/slidock-bundle")));
        let with_bundle = LOCK.replace(
            " \"slidock-core\",\n]",
            " \"slidock-bundle\",\n \"slidock-core\",\n]",
        ) + "\n[[package]]\nname = \"slidock-bundle\"\nversion = \"0.1.0\"\n";
        let paths = compiler_paths(&closure_of(&with_bundle).unwrap());
        for path in ["crates/slidock-bundle", "LICENSE"] {
            assert!(paths.iter().any(|p| p == path), "{path}");
        }
    }

    #[tokio::test]
    async fn the_working_copy_content_names_the_embedded_player() {
        if !crate::git::installed() {
            return;
        }
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let entries = content_entries(&repo, None, false).await.unwrap();
        let has = |prefix: &str| entries.iter().any(|e| e.path.starts_with(prefix));
        assert!(has("crates/slidock-bundle/player/player.iife.js"));
        assert!(has("crates/slidock-bundle/player/presenter.iife.js"));
        assert!(has("LICENSE"));
        assert!(!has("crates/slidock-bundle/tests/"));
        assert!(!has("packages/player/"));
    }

    #[test]
    fn severities_cover_the_registry() {
        let map = severity_map();
        assert_eq!(
            map.len(),
            slidock_ir::diagnostics::DiagnosticCode::ALL.len()
        );
        assert!(map.values().any(|severity| severity == "decline"));
    }
}
