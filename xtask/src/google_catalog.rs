//! `cargo xtask google-catalog`: the offline Google Fonts catalog (`crates/slidock-fonts/data/google-fonts.txt`),
//! generated from the `METADATA.pb` files of google/fonts at a pinned commit. Only the open-licence trees (`ofl/`,
//! `apache/`, `ufl/`) are read, so the catalog never names a family Google serves under another licence.
//!
//! The format is one family per line, tab-separated, read by `slidock_fonts::google`:
//! `family  generic  licence  faces  axes  subsets`, where faces are `n`/`i` (upright/italic) followed by weights
//! (`n400,700`) or a weight range (`n100-900`), axes are `tag:min:max`, and subsets index the `@subsets` line.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;

/// The google/fonts commit the catalog is generated from.
pub const COMMIT: &str = "6c1dbe27dcf7d817d4b62cec033ba77fa8e2a5ec";
const REPOSITORY: &str = "https://github.com/google/fonts.git";
const OUT: &str = "crates/slidock-fonts/data/google-fonts.txt";
/// The licence trees and the SPDX id of their licence.
const TREES: &[(&str, &str, &str)] = &[
    ("ofl", "OFL", "OFL-1.1"),
    ("apache", "APACHE2", "Apache-2.0"),
    ("ufl", "UFL", "UFL-1.0"),
];
/// Google's own subset for menus (the family name's letters), not a script.
const SKIPPED_SUBSETS: &[&str] = &["menu"];

struct Args {
    checkout: PathBuf,
    out: PathBuf,
    check: bool,
}

fn parse_args(args: &[String]) -> Result<Args, String> {
    let root = workspace_root()?;
    let private = std::env::var_os("SLIDOCK_PRIVATE")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".slidock")))
        .ok_or("set SLIDOCK_PRIVATE or HOME")?;
    let mut parsed = Args {
        checkout: private.join("tools/google-fonts"),
        out: root.join(OUT),
        check: false,
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--checkout" => {
                parsed.checkout = rest.next().ok_or("--checkout needs a folder")?.into()
            }
            "--out" => parsed.out = rest.next().ok_or("--out needs a file")?.into(),
            "--check" => parsed.check = true,
            other => return Err(format!("google-catalog: unknown argument {other}")),
        }
    }
    Ok(parsed)
}

fn workspace_root() -> Result<PathBuf, String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| "xtask has no parent folder".to_owned())
}

pub fn run(args: &[String]) -> Result<(), String> {
    let args = parse_args(args)?;
    ensure_checkout(&args.checkout)?;
    let families = read_families(&args.checkout)?;
    let text = render(&families);
    if args.check {
        let committed = std::fs::read_to_string(&args.out)
            .map_err(|error| format!("{}: {error}", args.out.display()))?;
        if committed != text {
            return Err(format!(
                "{} is not what google/fonts@{COMMIT} generates; run `cargo xtask google-catalog`",
                args.out.display()
            ));
        }
        eprintln!(
            "{}: up to date ({} families)",
            args.out.display(),
            families.len()
        );
        return Ok(());
    }
    std::fs::write(&args.out, text).map_err(|error| format!("{}: {error}", args.out.display()))?;
    eprintln!("{}: {} families", args.out.display(), families.len());
    Ok(())
}

fn git(dir: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        .output()
        .map_err(|error| format!("git: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
}

/// A sparse, blob-less, depth-1 checkout of the pinned commit holding only the `METADATA.pb` files.
fn ensure_checkout(dir: &Path) -> Result<(), String> {
    if !dir.join(".git").exists() {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        git(dir, &["init", "-q", "."])?;
        git(dir, &["remote", "add", "origin", REPOSITORY])?;
    }
    if git(dir, &["rev-parse", "HEAD"]).ok().as_deref() == Some(COMMIT) {
        return Ok(());
    }
    git(
        dir,
        &["sparse-checkout", "set", "--no-cone", "/*/*/METADATA.pb"],
    )?;
    git(
        dir,
        &[
            "fetch",
            "-q",
            "--depth",
            "1",
            "--filter=blob:none",
            "origin",
            COMMIT,
        ],
    )?;
    git(
        dir,
        &[
            "-c",
            "advice.detachedHead=false",
            "checkout",
            "-q",
            "FETCH_HEAD",
        ],
    )?;
    let head = git(dir, &["rev-parse", "HEAD"])?;
    if head != COMMIT {
        return Err(format!("{}: HEAD is {head}, not {COMMIT}", dir.display()));
    }
    Ok(())
}

/// A family as the catalog records it.
#[derive(Debug, Default, PartialEq)]
struct Family {
    name: String,
    generic: &'static str,
    licence: &'static str,
    /// Per style (`n`, `i`): the static weights.
    weights: BTreeMap<char, BTreeSet<u32>>,
    axes: Vec<(String, String, String)>,
    subsets: BTreeSet<String>,
}

fn read_families(checkout: &Path) -> Result<Vec<Family>, String> {
    let mut families: Vec<Family> = Vec::new();
    let mut keys = BTreeSet::new();
    for (tree, licence, spdx) in TREES {
        let dir = checkout.join(tree);
        let mut entries: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map_err(|error| format!("{}: {error}", dir.display()))?
            .filter_map(Result::ok)
            .map(|entry| entry.path().join("METADATA.pb"))
            .filter(|path| path.is_file())
            .collect();
        entries.sort();
        for path in entries {
            let text = std::fs::read_to_string(&path)
                .map_err(|error| format!("{}: {error}", path.display()))?;
            let metadata =
                parse_metadata(&text).map_err(|error| format!("{}: {error}", path.display()))?;
            if metadata.licence != *licence {
                return Err(format!(
                    "{}: licence {} in the {tree} tree",
                    path.display(),
                    metadata.licence
                ));
            }
            let family = metadata.family(spdx)?;
            let key = font_key(&family.name);
            if !keys.insert(key) {
                eprintln!(
                    "{}: {} appears twice; the first is kept",
                    path.display(),
                    family.name
                );
                continue;
            }
            families.push(family);
        }
    }
    families.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(families)
}

/// `slidock_ir::fonts::font_key` on ASCII whitespace: lowercase ASCII, no spaces or hyphens.
fn font_key(family: &str) -> String {
    family
        .chars()
        .filter(|c| *c != '-' && !c.is_whitespace())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

#[derive(Debug, Default)]
struct FontEntry {
    style: String,
    weight: Option<u32>,
}

#[derive(Debug, Default)]
struct Axis {
    tag: String,
    min: String,
    max: String,
}

#[derive(Debug, Default)]
struct Metadata {
    name: String,
    licence: String,
    category: Option<String>,
    fonts: Vec<FontEntry>,
    subsets: Vec<String>,
    axes: Vec<Axis>,
}

impl Metadata {
    fn family(&self, spdx: &'static str) -> Result<Family, String> {
        if self.name.is_empty() {
            return Err("no name".to_owned());
        }
        let generic = match self.category.as_deref() {
            Some("SANS_SERIF") | None => "sans-serif",
            Some("SERIF") => "serif",
            Some("MONOSPACE") => "monospace",
            Some("HANDWRITING") => "cursive",
            Some("DISPLAY") => "fantasy",
            Some(other) => return Err(format!("unknown category {other}")),
        };
        let wght = self.axes.iter().find(|axis| axis.tag == "wght");
        let mut weights: BTreeMap<char, BTreeSet<u32>> = BTreeMap::new();
        for font in &self.fonts {
            let style = match font.style.as_str() {
                "normal" => 'n',
                "italic" => 'i',
                other => return Err(format!("unknown style {other}")),
            };
            let entry = weights.entry(style).or_default();
            match wght {
                Some(axis) => {
                    entry.insert(axis_weight(&axis.min)?);
                    entry.insert(axis_weight(&axis.max)?);
                }
                None => {
                    entry.insert(font.weight.ok_or("a font without a weight")?);
                }
            }
        }
        if weights.is_empty() {
            return Err("no fonts".to_owned());
        }
        Ok(Family {
            name: self.name.clone(),
            generic,
            licence: spdx,
            weights,
            axes: self
                .axes
                .iter()
                .map(|axis| (axis.tag.clone(), number(&axis.min), number(&axis.max)))
                .collect(),
            subsets: self
                .subsets
                .iter()
                .filter(|subset| !SKIPPED_SUBSETS.contains(&subset.as_str()))
                .cloned()
                .collect(),
        })
    }
}

fn axis_weight(value: &str) -> Result<u32, String> {
    value
        .parse::<f64>()
        .ok()
        .filter(|weight| weight.fract() == 0.0 && (1.0..=1000.0).contains(weight))
        .map(|weight| weight as u32)
        .ok_or_else(|| format!("wght {value} is not a weight"))
}

/// A decimal as written, without a trailing `.0`.
fn number(value: &str) -> String {
    value.strip_suffix(".0").unwrap_or(value).to_owned()
}

/// The fields of a `METADATA.pb` (protobuf text format) the catalog needs: `name`, `license`, the first
/// `category`, `fonts { style weight }`, `subsets` and `axes { tag min_value max_value }`.
fn parse_metadata(text: &str) -> Result<Metadata, String> {
    let mut metadata = Metadata::default();
    let mut path: Vec<String> = Vec::new();
    for (index, raw) in text.lines().enumerate() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with('"') {
            continue;
        }
        let at = || format!("line {}", index + 1);
        if line == "}" {
            path.pop()
                .ok_or_else(|| format!("{}: unbalanced }}", at()))?;
            continue;
        }
        if let Some(block) = line.strip_suffix('{') {
            let block = block.trim().trim_end_matches(':').trim().to_owned();
            match (path.len(), block.as_str()) {
                (0, "fonts") => metadata.fonts.push(FontEntry::default()),
                (0, "axes") => metadata.axes.push(Axis::default()),
                _ => {}
            }
            path.push(block);
            continue;
        }
        let (key, value) = line
            .split_once(':')
            .ok_or_else(|| format!("{}: not a field: {line}", at()))?;
        let value = value.trim();
        let parent: Vec<&str> = path.iter().map(String::as_str).collect();
        match (parent.as_slice(), key.trim()) {
            ([], "name") => metadata.name = string(value)?,
            ([], "license") => metadata.licence = string(value)?,
            ([], "category") if metadata.category.is_none() => {
                metadata.category = Some(string(value)?)
            }
            ([], "subsets") => metadata.subsets.push(string(value)?),
            (["fonts"], "style") => {
                if let Some(font) = metadata.fonts.last_mut() {
                    font.style = string(value)?;
                }
            }
            (["fonts"], "weight") => {
                if let Some(font) = metadata.fonts.last_mut() {
                    font.weight = Some(
                        value
                            .parse()
                            .map_err(|_| format!("{}: weight {value}", at()))?,
                    );
                }
            }
            (["axes"], field @ ("tag" | "min_value" | "max_value")) => {
                if let Some(axis) = metadata.axes.last_mut() {
                    match field {
                        "tag" => axis.tag = string(value)?,
                        "min_value" => axis.min = value.to_owned(),
                        _ => axis.max = value.to_owned(),
                    }
                }
            }
            _ => {}
        }
    }
    if !path.is_empty() {
        return Err("unclosed block".to_owned());
    }
    Ok(metadata)
}

/// A quoted text-format string: `\"`, `\\`, `\n`, `\t` and octal byte escapes, as UTF-8.
fn string(value: &str) -> Result<String, String> {
    let inner = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
        .ok_or_else(|| format!("not a string: {value}"))?;
    let mut bytes = Vec::with_capacity(inner.len());
    let mut chars = inner.bytes().peekable();
    while let Some(byte) = chars.next() {
        if byte != b'\\' {
            bytes.push(byte);
            continue;
        }
        match chars.next() {
            Some(b'n') => bytes.push(b'\n'),
            Some(b't') => bytes.push(b'\t'),
            Some(digit @ b'0'..=b'7') => {
                let mut code = u32::from(digit - b'0');
                for _ in 0..2 {
                    match chars.peek() {
                        Some(next @ b'0'..=b'7') => {
                            code = code * 8 + u32::from(next - b'0');
                            chars.next();
                        }
                        _ => break,
                    }
                }
                bytes.push(u8::try_from(code).map_err(|_| format!("bad escape in {value}"))?);
            }
            Some(other) => bytes.push(other),
            None => return Err(format!("dangling escape in {value}")),
        }
    }
    String::from_utf8(bytes).map_err(|_| format!("not UTF-8: {value}"))
}

fn render(families: &[Family]) -> String {
    let subsets: BTreeSet<&str> = families
        .iter()
        .flat_map(|family| family.subsets.iter().map(String::as_str))
        .collect();
    let index: BTreeMap<&str, usize> = subsets.iter().enumerate().map(|(i, s)| (*s, i)).collect();
    let mut out = String::new();
    out.push_str(
        "# The open-licence families of google/fonts (ofl/, apache/, ufl/ METADATA.pb), for slidock-fonts' google module.\n\
         # Generated by `cargo xtask google-catalog`; do not edit.\n\
         # family\tgeneric\tlicence\tfaces\taxes\tsubsets\n",
    );
    let _ = writeln!(out, "@commit {COMMIT}");
    let _ = writeln!(
        out,
        "@subsets {}",
        subsets.iter().copied().collect::<Vec<_>>().join(" ")
    );
    for family in families {
        let faces: Vec<String> = family
            .weights
            .iter()
            .map(|(style, weights)| {
                let list: Vec<String> = weights.iter().map(u32::to_string).collect();
                let ranged = family.axes.iter().any(|(tag, _, _)| tag == "wght");
                if ranged {
                    format!("{style}{}", list.join("-"))
                } else {
                    format!("{style}{}", list.join(","))
                }
            })
            .collect();
        let axes: Vec<String> = family
            .axes
            .iter()
            .map(|(tag, min, max)| format!("{tag}:{min}:{max}"))
            .collect();
        let subsets: Vec<String> = family
            .subsets
            .iter()
            .filter_map(|subset| index.get(subset.as_str()))
            .map(usize::to_string)
            .collect();
        let _ = writeln!(
            out,
            "{}\t{}\t{}\t{}\t{}\t{}",
            family.name,
            family.generic,
            family.licence,
            faces.join(" "),
            axes.join(" "),
            subsets.join(" ")
        );
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const OSWALD: &str = r#"name: "Oswald"
designer: "Vernon Adams"
license: "OFL"
category: "SANS_SERIF"
category: "DISPLAY"
fonts {
  name: "Oswald"
  style: "normal"
  weight: 400
  filename: "Oswald[wght].ttf"
}
subsets: "latin"
subsets: "menu"
axes {
  tag: "wght"
  min_value: 200.0
  max_value: 700.0
}
source {
  files {
    source_file: "OFL.txt"
  }
}
"#;

    #[test]
    fn a_variable_family_takes_its_weight_range_per_style() {
        let family = parse_metadata(OSWALD).unwrap().family("OFL-1.1").unwrap();
        assert_eq!(family.name, "Oswald");
        assert_eq!(family.generic, "sans-serif");
        assert_eq!(family.weights[&'n'], BTreeSet::from([200, 700]));
        assert_eq!(family.subsets, BTreeSet::from(["latin".to_owned()]));
        let line = render(&[family]);
        assert!(
            line.ends_with("Oswald\tsans-serif\tOFL-1.1\tn200-700\twght:200:700\t0\n"),
            "{line}"
        );
    }

    #[test]
    fn a_static_family_lists_its_weights() {
        let text = "name: \"Static \\303\\251\"\nlicense: \"APACHE2\"\ncategory: \"SERIF\"\nfonts {\n  style: \"normal\"\n  weight: 400\n}\nfonts {\n  style: \"italic\"\n  weight: 700\n}\nfonts {\n  style: \"normal\"\n  weight: 700\n}\n";
        let family = parse_metadata(text).unwrap().family("Apache-2.0").unwrap();
        assert_eq!(family.name, "Static é");
        let line = render(&[family]);
        assert!(
            line.ends_with("Static é\tserif\tApache-2.0\ti700 n400,700\t\t\n"),
            "{line}"
        );
    }
}
