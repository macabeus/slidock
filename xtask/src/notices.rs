//! `cargo xtask notices`: writes `THIRD-PARTY-NOTICES.txt` (at the root, and the copy the slidock command prints,
//! `crates/slidock-cli/THIRD-PARTY-NOTICES.txt`): slidock's licence and NOTICE, the Rust crates the slidock command
//! (every target platform) and slidock-wasm (wasm32-unknown-unknown) link, the JavaScript packages the web app ships,
//! and the licence files of each, every distinct text once. The same lock files and installed packages write the
//! same bytes.
//!
//! The crates are `cargo tree`'s normal dependencies without proc macros (build scripts, macros and dev dependencies
//! ship nothing); their licence files are those beside each crate's manifest (`LICENSE*`, `LICENCE*`, `COPYING*`,
//! `NOTICE*`, `COPYRIGHT*`, `UNLICENSE*`), else the SPDX text in `LICENSES/` of each id the crate's licence names,
//! its copyright line naming the crate's authors. The JavaScript packages are [`JS_PACKAGES`], read from `node_modules` (`pnpm install`); the web
//! app's build checks that every package its bundle holds is listed.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

const USAGE: &str = "usage: cargo xtask notices [--check]";

/// Where the notices are written, relative to the repository.
const OUTPUTS: [&str; 2] = [
    "THIRD-PARTY-NOTICES.txt",
    "crates/slidock-cli/THIRD-PARTY-NOTICES.txt",
];

/// The binaries whose crates are listed: (package, `--target`, label).
const ROOTS: [(&str, &str, &str); 2] = [
    ("slidock-cli", "all", "cli"),
    ("slidock-wasm", "wasm32-unknown-unknown", "wasm"),
];

/// A JavaScript package the web app ships.
struct JsPackage {
    name: &'static str,
    /// The package whose `node_modules` it is resolved from (`None`: the web app's).
    from: Option<&'static str>,
    /// What of it ships.
    ships: &'static str,
    /// The licence file's text up to this line, when the file also holds what does not ship.
    until: Option<&'static str>,
}

/// The JavaScript packages in the web app's build (its bundle holds code from these and from slidock's own).
const JS_PACKAGES: [JsPackage; 5] = [
    JsPackage {
        name: "react",
        from: None,
        ships: "the web app",
        until: None,
    },
    JsPackage {
        name: "react-dom",
        from: None,
        ships: "the web app",
        until: None,
    },
    JsPackage {
        name: "scheduler",
        from: Some("react-dom"),
        ships: "the web app (through react-dom)",
        until: None,
    },
    JsPackage {
        name: "tailwindcss",
        from: None,
        ships: "the web app's base styles and utilities",
        until: None,
    },
    JsPackage {
        name: "vite",
        from: None,
        ships: "the web app's module preload helper",
        until: Some("# Licenses of bundled dependencies"),
    },
];

/// The web app's folder, whose `node_modules` holds [`JS_PACKAGES`].
const WEB_APP: &str = "apps/web";

/// The bundled substitute fonts the web app serves and the slidock command builds in, with their licences.
const SUBSTITUTES: &str = "crates/slidock-substitutes/fonts";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..")
}

pub fn run(args: &[String]) -> Result<(), String> {
    let check = match args {
        [] => false,
        [flag] if flag == "--check" => true,
        _ => return Err(USAGE.to_owned()),
    };
    let root = repo_root();
    let text = notices(&root)?;
    if check {
        let stale: Vec<&str> = OUTPUTS
            .into_iter()
            .filter(|path| std::fs::read_to_string(root.join(path)).ok().as_deref() != Some(&text))
            .collect();
        if !stale.is_empty() {
            return Err(format!(
                "stale (run cargo xtask notices): {}",
                stale.join(", ")
            ));
        }
        println!("{} are up to date", OUTPUTS.join(" and "));
    } else {
        for path in OUTPUTS {
            let path = root.join(path);
            std::fs::write(&path, &text).map_err(|e| format!("{}: {e}", path.display()))?;
        }
        println!("wrote {} ({} bytes)", OUTPUTS.join(" and "), text.len());
    }
    Ok(())
}

fn read(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

/// A licence text as it is printed: `\n` line ends, no trailing spaces, no blank lines around it.
fn tidy(text: &str) -> String {
    let lines: Vec<&str> = text.lines().map(str::trim_end).collect();
    lines.join("\n").trim_matches('\n').to_owned()
}

fn cargo(root: &Path, args: &[&str]) -> Result<String, String> {
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let output = Command::new(&cargo)
        .args(args)
        .current_dir(root)
        .output()
        .map_err(|e| format!("{cargo}: {e}"))?;
    if !output.status.success() {
        return Err(format!(
            "cargo {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    String::from_utf8(output.stdout).map_err(|e| format!("cargo {}: {e}", args.join(" ")))
}

/// A third-party package and its licence files.
struct Package {
    /// `name version`.
    label: String,
    licence: String,
    /// What ships it: `cli`, `wasm`, or what of a JavaScript package ships.
    shipped_in: Vec<String>,
    /// The texts of its licence files, each tidied.
    texts: Vec<String>,
}

/// The registry crates each root links, by (name, version): the labels of the roots.
fn linked_crates(root: &Path) -> Result<BTreeMap<(String, String), Vec<&'static str>>, String> {
    let mut crates: BTreeMap<(String, String), Vec<&'static str>> = BTreeMap::new();
    for (package, target, label) in ROOTS {
        let tree = cargo(
            root,
            &[
                "tree",
                "--locked",
                "-p",
                package,
                "--target",
                target,
                "-e",
                "normal,no-proc-macro",
                "--prefix",
                "none",
                "--format",
                "{p}",
            ],
        )?;
        for line in tree.lines() {
            let line = line.trim_end_matches(" (*)").trim();
            // Workspace crates print their path: they are slidock's own.
            if line.is_empty() || line.ends_with(')') {
                continue;
            }
            let (name, version) = line
                .split_once(" v")
                .ok_or_else(|| format!("cargo tree printed {line:?}"))?;
            let roots = crates
                .entry((name.to_owned(), version.to_owned()))
                .or_default();
            if !roots.contains(&label) {
                roots.push(label);
            }
        }
    }
    Ok(crates)
}

fn is_licence_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    [
        "license",
        "licence",
        "copying",
        "notice",
        "copyright",
        "unlicense",
    ]
    .iter()
    .any(|prefix| lower.starts_with(prefix))
}

/// The licence files beside a package's manifest, by file name.
fn licence_files(dir: &Path) -> Result<Vec<String>, String> {
    let mut names = Vec::new();
    for entry in std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))? {
        let entry = entry.map_err(|e| format!("{}: {e}", dir.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if is_licence_file(&name) && entry.path().is_file() {
            names.push(name);
        }
    }
    names.sort();
    names
        .iter()
        .map(|name| read(&dir.join(name)).map(|text| tidy(&text)))
        .collect()
}

/// The SPDX ids an expression names (`MIT OR Apache-2.0`, `Apache-2.0/MIT`, `(MIT OR Apache-2.0) AND
/// Unicode-3.0`).
fn spdx_ids(expression: &str) -> Vec<String> {
    let ids: BTreeSet<String> = expression
        .split(|c: char| c.is_whitespace() || matches!(c, '(' | ')' | '/'))
        .filter(|word| !word.is_empty() && !matches!(*word, "OR" | "AND" | "WITH"))
        .map(str::to_owned)
        .collect();
    ids.into_iter().collect()
}

fn crate_packages(root: &Path) -> Result<Vec<Package>, String> {
    let linked = linked_crates(root)?;
    let metadata: Value = serde_json::from_str(&cargo(
        root,
        &["metadata", "--locked", "--format-version", "1"],
    )?)
    .map_err(|e| format!("cargo metadata: {e}"))?;
    let listed = metadata
        .get("packages")
        .and_then(Value::as_array)
        .ok_or("cargo metadata lists no packages")?;
    let mut packages = Vec::new();
    for ((name, version), roots) in linked {
        let found = listed
            .iter()
            .find(|p| {
                p.get("name").and_then(Value::as_str) == Some(name.as_str())
                    && p.get("version").and_then(Value::as_str) == Some(version.as_str())
            })
            .ok_or_else(|| format!("cargo metadata lacks {name} {version}"))?;
        let field = |key: &str| found.get(key).and_then(Value::as_str);
        let licence = field("license").unwrap_or("").to_owned();
        let manifest = PathBuf::from(field("manifest_path").ok_or("no manifest path")?);
        let dir = manifest.parent().ok_or("a manifest outside any folder")?;
        let mut texts = licence_files(dir)?;
        if let Some(file) = field("license_file") {
            let text = tidy(&read(&dir.join(file))?);
            if !texts.contains(&text) {
                texts.push(text);
            }
        }
        if texts.is_empty() {
            let authors: Vec<&str> = found
                .get("authors")
                .and_then(Value::as_array)
                .map(|authors| authors.iter().filter_map(Value::as_str).collect())
                .unwrap_or_default();
            for id in spdx_ids(&licence) {
                let path = root.join("LICENSES").join(format!("{id}.txt"));
                let text = read(&path).map_err(|e| {
                    format!("{name} {version} ships no licence file, and {e}: add the SPDX text")
                })?;
                // The SPDX text's copyright line is a template: the crate's authors hold it.
                let holders = format!("Copyright (c) {}", authors.join(", "));
                let text: Vec<&str> = text
                    .lines()
                    .map(|line| {
                        if line.starts_with("Copyright (c) <") {
                            holders.as_str()
                        } else {
                            line
                        }
                    })
                    .collect();
                texts.push(format!(
                    "{name} {version} ships no licence file; its licence is {licence}, the SPDX text:\n\n{}",
                    tidy(&text.join("\n"))
                ));
            }
        }
        if texts.is_empty() {
            return Err(format!("{name} {version} has no licence"));
        }
        packages.push(Package {
            label: format!("{name} {version}"),
            licence,
            shipped_in: roots.iter().map(|root| (*root).to_owned()).collect(),
            texts,
        });
    }
    Ok(packages)
}

/// Node's lookup of `name` from `dir`: `node_modules/<name>` in `dir` or its nearest ancestor that has one.
fn resolve(dir: &Path, name: &str) -> Result<PathBuf, String> {
    dir.ancestors()
        .filter(|ancestor| ancestor.file_name().is_none_or(|n| n != "node_modules"))
        .map(|ancestor| ancestor.join("node_modules").join(name))
        .find(|candidate| candidate.join("package.json").is_file())
        .ok_or_else(|| {
            format!(
                "{name} is not installed for {} (run pnpm install)",
                dir.display()
            )
        })?
        .canonicalize()
        .map_err(|e| format!("{name}: {e}"))
}

fn js_packages(root: &Path) -> Result<Vec<Package>, String> {
    let web = root.join(WEB_APP);
    let mut resolved: BTreeMap<&str, PathBuf> = BTreeMap::new();
    let mut packages = Vec::new();
    for package in &JS_PACKAGES {
        let from = match package.from {
            None => web.clone(),
            Some(parent) => resolved
                .get(parent)
                .cloned()
                .ok_or_else(|| format!("{parent} is resolved after {}", package.name))?,
        };
        let dir = resolve(&from, package.name)?;
        let manifest: Value = serde_json::from_str(&read(&dir.join("package.json"))?)
            .map_err(|e| format!("{}: {e}", package.name))?;
        let version = manifest
            .get("version")
            .and_then(Value::as_str)
            .ok_or_else(|| format!("{} has no version", package.name))?;
        let licence = manifest
            .get("license")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        let mut texts = licence_files(&dir)?;
        if let Some(marker) = package.until {
            for text in &mut texts {
                let end = text
                    .find(marker)
                    .ok_or_else(|| format!("{}'s licence lacks {marker:?}", package.name))?;
                *text = tidy(&text[..end]);
            }
        }
        if texts.is_empty() {
            return Err(format!("{} {version} ships no licence file", package.name));
        }
        packages.push(Package {
            label: format!("{} {version}", package.name),
            licence,
            shipped_in: vec![package.ships.to_owned()],
            texts,
        });
        resolved.insert(package.name, dir);
    }
    Ok(packages)
}

/// The substitute faces' licence files, as `core.json` lists them: (file, text).
fn substitute_licences(root: &Path) -> Result<Vec<(String, String)>, String> {
    let dir = root.join(SUBSTITUTES);
    let manifest: Value = serde_json::from_str(&read(&dir.join("core.json"))?)
        .map_err(|e| format!("{SUBSTITUTES}/core.json: {e}"))?;
    let files: BTreeSet<&str> = manifest
        .get("faces")
        .and_then(Value::as_array)
        .ok_or("core.json lists no faces")?
        .iter()
        .filter_map(|face| face.get("licenceFile").and_then(Value::as_str))
        .collect();
    files
        .into_iter()
        .map(|file| Ok((file.to_owned(), tidy(&read(&dir.join(file))?))))
        .collect()
}

/// A licence text's key: its words, so texts that differ only in spacing are one.
fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn underline(title: &str, rule: char) -> String {
    format!(
        "{title}\n{}\n",
        rule.to_string().repeat(title.chars().count())
    )
}

fn table(out: &mut String, packages: &[Package]) {
    let width = packages
        .iter()
        .map(|p| p.label.chars().count())
        .max()
        .unwrap_or(0);
    let licence_width = packages
        .iter()
        .map(|p| p.licence.chars().count())
        .max()
        .unwrap_or(0);
    for package in packages {
        let line = format!(
            "  {:width$}  {:licence_width$}  {}",
            package.label,
            package.licence,
            package.shipped_in.join(", ")
        );
        out.push_str(line.trim_end());
        out.push('\n');
    }
}

fn notices(root: &Path) -> Result<String, String> {
    let crates = crate_packages(root)?;
    let js = js_packages(root)?;
    let fonts = substitute_licences(root)?;
    let mut out = String::new();
    out.push_str(&underline("slidock: licences and third-party notices", '='));
    out.push_str(
        "\nGenerated by `cargo xtask notices`; do not edit. The slidock command prints it (`slidock licenses`), and\n\
         the web app serves it.\n\n",
    );
    out.push_str(&underline("slidock's licence (LICENSE)", '-'));
    out.push('\n');
    out.push_str(&tidy(&read(&root.join("LICENSE"))?));
    out.push_str("\n\n");
    out.push_str(&underline("slidock's notice (NOTICE)", '-'));
    out.push('\n');
    out.push_str(&tidy(&read(&root.join("NOTICE"))?));
    out.push_str("\n\n");

    out.push_str(&underline("Rust crates", '-'));
    out.push_str(
        "\nThe slidock command (cli, every target platform) and slidock-wasm, the web app's compiler (wasm), link\n\
         these crates. Each crate's licence files follow, under \"Licence texts\". cssparser and dtoa-short are under\n\
         the Mozilla Public License 2.0: their source is at https://crates.io/crates/cssparser/0.38.0 and\n\
         https://crates.io/crates/dtoa-short/0.3.5 (repositories https://github.com/servo/rust-cssparser and\n\
         https://github.com/servo/dtoa-short).\n\n",
    );
    table(&mut out, &crates);
    out.push('\n');
    out.push_str(&underline("JavaScript packages", '-'));
    out.push_str("\nThe web app's build holds code from these packages:\n\n");
    table(&mut out, &js);
    out.push('\n');

    let mut groups: Vec<(String, String, Vec<String>)> = Vec::new();
    for package in crates.iter().chain(&js) {
        for text in &package.texts {
            let key = words(text);
            match groups.iter_mut().find(|(held, _, _)| *held == key) {
                Some((_, _, users)) => {
                    if !users.contains(&package.label) {
                        users.push(package.label.clone());
                    }
                }
                None => groups.push((key, text.clone(), vec![package.label.clone()])),
            }
        }
    }
    out.push_str(&underline("Licence texts", '-'));
    out.push_str(
        "\nEach distinct licence file of the packages above, once, with the packages that ship it.\n",
    );
    for (_, text, users) in &groups {
        out.push_str("\n\n");
        out.push_str(&"=".repeat(80));
        out.push_str("\nFor ");
        out.push_str(&users.join(", "));
        out.push_str(":\n\n");
        out.push_str(text);
        out.push('\n');
    }
    out.push_str("\n\n");
    out.push_str(&underline("Fonts", '-'));
    out.push_str(
        "\nThe bundled substitute faces (Liberation, DejaVu Sans, Carlito, Caladea), which the web app serves and the\n\
         slidock command builds in, are under these licences:\n",
    );
    for (file, text) in &fonts {
        out.push_str("\n\n");
        out.push_str(&"=".repeat(80));
        out.push_str(&format!("\n{file}:\n\n{text}\n"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spdx_ids_reads_every_expression_form() {
        assert_eq!(spdx_ids("MIT OR Apache-2.0"), ["Apache-2.0", "MIT"]);
        assert_eq!(spdx_ids("Apache-2.0/MIT"), ["Apache-2.0", "MIT"]);
        assert_eq!(
            spdx_ids("(MIT OR Apache-2.0) AND Unicode-3.0"),
            ["Apache-2.0", "MIT", "Unicode-3.0"]
        );
        assert_eq!(
            spdx_ids("Apache-2.0 WITH LLVM-exception OR MIT"),
            ["Apache-2.0", "LLVM-exception", "MIT"]
        );
    }

    #[test]
    fn licence_files_are_found_by_name() {
        for name in [
            "LICENSE",
            "LICENSE-MIT",
            "license-apache-2.0",
            "LICENCE.txt",
            "COPYING",
            "NOTICE",
            "UNLICENSE",
        ] {
            assert!(is_licence_file(name), "{name}");
        }
        for name in ["README.md", "Cargo.toml", "src"] {
            assert!(!is_licence_file(name), "{name}");
        }
    }

    #[test]
    fn texts_are_tidied_and_keyed_by_their_words() {
        assert_eq!(tidy("\n\nMIT  \r\nLicense\n\n"), "MIT\nLicense");
        assert_eq!(words("a\n  b\tc"), words("a b c"));
    }
}
