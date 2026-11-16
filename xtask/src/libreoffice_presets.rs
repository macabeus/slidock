//! `cargo xtask libreoffice-presets`: the LibreOffice tables of `slidock-presets`, generated at a pinned release tag
//! of LibreOffice core: the ODP preset catalogs (`src/libreoffice/effects.rs` and `transitions.rs`, from
//! `sd/xml/effects.xml` and `sd/xml/transitions.xml`), PowerPoint's preset ids and sub-type numbers
//! (`src/libreoffice/pptx.rs`, from `oox/source/ppt/commontimenodecontext.cxx`) and the transition preset each ODF 1.0
//! transition style migrates to (`src/libreoffice/migration.rs`, from `sd/source/core/EffectMigration.cxx`). The
//! source files are MPL-2.0, and so are the generated files: each keeps the source's licence header. The motion probe
//! generator (`probes_libreoffice.rs`) reads the same tag.
//!
//! The sources are fetched once (with `curl`, from the pinned commit) into `$SLIDOCK_PRIVATE/tools` and checked
//! against their SHA-256 before anything is read. The output is formatted with `rustfmt`, so a regeneration is a
//! no-op.

use std::fmt::Write as _;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use sha2::{Digest, Sha256};

/// The LibreOffice core commit the catalogs are generated from: tag `libreoffice-25.2.3.2`.
pub const COMMIT: &str = "bbb074479178df812d175f709636b368952c2ce3";
const RAW: &str = "https://raw.githubusercontent.com/LibreOffice/core";
/// Each source's folder in LibreOffice core and name, its SHA-256 at [`COMMIT`], and the file it generates under
/// `crates/slidock-presets/src/libreoffice`.
const SOURCES: &[(&str, &str, &str, &str)] = &[
    (
        "sd/xml",
        "effects.xml",
        "ce150ba36c14559abc48aa2d61561ffef9386e8acd1a419037cfa04c6d9f6103",
        "effects.rs",
    ),
    (
        "sd/xml",
        "transitions.xml",
        "6385fb101f409fdeb2625440d6cce8edd604c604dd28eb628b21fea6c0adf67d",
        "transitions.rs",
    ),
    (
        "oox/source/ppt",
        "commontimenodecontext.cxx",
        "d995b2cdc66f43047db2eef78c0ea2722e0704d243920e71ff7fa8f3892e0f66",
        "pptx.rs",
    ),
    (
        "sd/source/core",
        "EffectMigration.cxx",
        "a6577026fd317ab4b5ae7f12681ee702e49f6d6008c00242fa50c3c0a071d4c1",
        "migration.rs",
    ),
];
const OUT_DIR: &str = "crates/slidock-presets/src/libreoffice";

struct Args {
    dir: PathBuf,
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
        dir: private.join("tools/libreoffice-sd-xml").join(COMMIT),
        out: root.join(OUT_DIR),
        check: false,
    };
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--dir" => parsed.dir = rest.next().ok_or("--dir needs a folder")?.into(),
            "--out" => parsed.out = rest.next().ok_or("--out needs a folder")?.into(),
            "--check" => parsed.check = true,
            other => return Err(format!("libreoffice-presets: unknown argument {other}")),
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
    for (folder, source, sha256, output) in SOURCES {
        let text = read_source(&args.dir, folder, source, sha256)?;
        let generated = rustfmt(&generate(folder, source, sha256, &text)?)?;
        let out = args.out.join(output);
        if args.check {
            let committed = std::fs::read_to_string(&out)
                .map_err(|error| format!("{}: {error}", out.display()))?;
            if committed != generated {
                return Err(format!(
                    "{} is not what LibreOffice core@{COMMIT} generates; run `cargo xtask libreoffice-presets`",
                    out.display()
                ));
            }
            eprintln!("{}: up to date", out.display());
        } else {
            std::fs::write(&out, generated)
                .map_err(|error| format!("{}: {error}", out.display()))?;
            eprintln!("{}: written", out.display());
        }
    }
    Ok(())
}

fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::new(), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

/// The source's text, fetched into `dir` when missing, and checked against its pinned SHA-256.
fn read_source(dir: &Path, folder: &str, name: &str, sha256: &str) -> Result<String, String> {
    let path = dir.join(name);
    if !path.exists() {
        std::fs::create_dir_all(dir).map_err(|error| format!("{}: {error}", dir.display()))?;
        let url = format!("{RAW}/{COMMIT}/{folder}/{name}");
        let status = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--output",
            ])
            .arg(&path)
            .arg(&url)
            .status()
            .map_err(|error| format!("curl: {error}"))?;
        if !status.success() {
            return Err(format!("curl {url}: {status}"));
        }
    }
    let bytes = std::fs::read(&path).map_err(|error| format!("{}: {error}", path.display()))?;
    let actual = sha256_hex(&bytes);
    if actual != sha256 {
        return Err(format!(
            "{}: SHA-256 {actual}, expected {sha256}",
            path.display()
        ));
    }
    String::from_utf8(bytes).map_err(|error| format!("{}: {error}", path.display()))
}

fn rustfmt(source: &str) -> Result<String, String> {
    let mut child = Command::new("rustfmt")
        .args(["--edition", "2024", "--emit", "stdout"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("rustfmt: {error}"))?;
    child
        .stdin
        .take()
        .ok_or("rustfmt: no stdin")?
        .write_all(source.as_bytes())
        .map_err(|error| format!("rustfmt: {error}"))?;
    let output = child
        .wait_with_output()
        .map_err(|error| format!("rustfmt: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "rustfmt: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout).map_err(|error| format!("rustfmt: {error}"))
}

/// One start tag: its name and attributes, in order.
#[derive(Debug, PartialEq)]
struct Tag {
    name: String,
    attributes: Vec<(String, String)>,
}

impl Tag {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

fn unescape(value: &str) -> String {
    value
        .replace("&quot;", "\"")
        .replace("&apos;", "'")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

/// The source's first comment (its licence header), and its start tags outside comments, in order. The files are
/// LibreOffice's own, plain and well-formed; this reads the subset of XML they use.
fn scan(text: &str) -> Result<(String, Vec<Tag>), String> {
    let mut header = None;
    let mut tags = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('<') {
        rest = rest.get(start..).ok_or("scan: bad offset")?;
        if let Some(comment) = rest.strip_prefix("<!--") {
            let end = comment.find("-->").ok_or("scan: an unclosed comment")?;
            if header.is_none() {
                header = Some(comment.get(..end).ok_or("scan: bad offset")?.to_owned());
            }
            rest = comment.get(end + 3..).ok_or("scan: bad offset")?;
            continue;
        }
        let mut end = None;
        let mut quote = None;
        for (i, c) in rest.char_indices() {
            match (quote, c) {
                (None, '"' | '\'') => quote = Some(c),
                (Some(open), c) if c == open => quote = None,
                (None, '>') => {
                    end = Some(i);
                    break;
                }
                _ => {}
            }
        }
        let end = end.ok_or("scan: an unclosed tag")?;
        let inner = rest.get(1..end).ok_or("scan: bad offset")?;
        rest = rest.get(end + 1..).ok_or("scan: bad offset")?;
        if inner.starts_with('/') || inner.starts_with('?') || inner.starts_with('!') {
            continue;
        }
        tags.push(parse_tag(inner.trim_end_matches('/'))?);
    }
    Ok((header.ok_or("scan: no licence header")?, tags))
}

fn parse_tag(inner: &str) -> Result<Tag, String> {
    let inner = inner.trim();
    let (name, mut rest) = inner.split_once(char::is_whitespace).unwrap_or((inner, ""));
    let mut attributes = Vec::new();
    loop {
        rest = rest.trim_start();
        if rest.is_empty() {
            break;
        }
        let (key, after) = rest
            .split_once('=')
            .ok_or_else(|| format!("scan: an attribute without a value in <{name}>"))?;
        let after = after.trim_start();
        let quote = after
            .chars()
            .next()
            .ok_or("scan: a missing attribute value")?;
        if quote != '"' && quote != '\'' {
            return Err(format!("scan: an unquoted attribute value in <{name}>"));
        }
        let value_end = after
            .get(1..)
            .and_then(|value| value.find(quote))
            .ok_or("scan: an unclosed attribute value")?;
        let value = after.get(1..1 + value_end).ok_or("scan: bad offset")?;
        attributes.push((key.trim().to_owned(), unescape(value)));
        rest = after.get(value_end + 2..).ok_or("scan: bad offset")?;
    }
    Ok(Tag {
        name: name.to_owned(),
        attributes,
    })
}

fn optional(value: Option<&str>) -> String {
    value.map_or_else(|| "None".to_owned(), |value| format!("Some({value:?})"))
}

/// The licence header as line comments, then the generated-file notice.
fn preamble(header: &str, folder: &str, source: &str, sha256: &str) -> String {
    let mut out = String::new();
    for line in header.trim_end().trim_start_matches('\n').lines() {
        let line = line.trim_end();
        if line.is_empty() {
            out.push_str("//\n");
        } else {
            let _ = writeln!(out, "//{line}");
        }
    }
    let _ = writeln!(
        out,
        "\n// Generated by `cargo xtask libreoffice-presets` from LibreOffice core `{folder}/{source}` at\n// {COMMIT}\n// (SHA-256 {sha256}). Do not edit.\n"
    );
    out
}

fn generate(folder: &str, source: &str, sha256: &str, text: &str) -> Result<String, String> {
    if source == "EffectMigration.cxx" {
        return generate_migration(folder, source, sha256, text);
    }
    if source.ends_with(".cxx") {
        return generate_pptx(folder, source, sha256, text);
    }
    let (header, tags) = scan(text)?;
    let mut out = preamble(&header, folder, source, sha256);
    match source {
        "effects.xml" => {
            out.push_str("use super::EffectPreset;\n\n/// Every preset of `effects.xml`, in file order.\npub const EFFECTS: &[EffectPreset] = &[\n");
            // A preset is a `par`, or an `iterate` for the by-letter and by-word ones.
            for tag in tags
                .iter()
                .filter(|tag| tag.name == "anim:par" || tag.name == "anim:iterate")
            {
                let Some(id) = tag.attribute("pres:preset-id") else {
                    continue;
                };
                let class = tag
                    .attribute("pres:preset-class")
                    .ok_or_else(|| format!("{source}: {id} has no preset class"))?;
                let _ = writeln!(
                    out,
                    "    EffectPreset {{ class: {class:?}, id: {id:?}, subtype: {}, iterate: {} }},",
                    optional(tag.attribute("pres:preset-sub-type")),
                    optional(tag.attribute("anim:iterate-type")),
                );
            }
        }
        _ => {
            out.push_str("use super::TransitionPreset;\n\n/// Every preset of `transitions.xml`, in file order.\npub const TRANSITIONS: &[TransitionPreset] = &[\n");
            let mut open: Option<&str> = None;
            for tag in &tags {
                match tag.name.as_str() {
                    "anim:par" => open = tag.attribute("pres:preset-id"),
                    "anim:transitionFilter" => {
                        let id = open
                            .take()
                            .ok_or_else(|| format!("{source}: a filter outside a preset"))?;
                        let smil_type = tag
                            .attribute("smil:type")
                            .ok_or_else(|| format!("{source}: {id} has no smil:type"))?;
                        let _ = writeln!(
                            out,
                            "    TransitionPreset {{ id: {id:?}, smil_type: {smil_type:?}, subtype: {}, reverse: {}, fade_color: {}, lo_subtype: {} }},",
                            optional(tag.attribute("smil:subtype")),
                            tag.attribute("smil:direction") == Some("reverse"),
                            optional(tag.attribute("smil:fadeColor")),
                            optional(tag.attribute("loext:subtype")),
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    out.push_str("];\n");
    Ok(out)
}

/// The text between `start` and the first `end` after it.
fn between<'a>(text: &'a str, start: &str, end: &str) -> Result<&'a str, String> {
    let from = text.find(start).ok_or_else(|| format!("no `{start}`"))?;
    let rest = text.get(from + start.len()..).ok_or("bad offset")?;
    let to = rest
        .find(end)
        .ok_or_else(|| format!("no `{end}` after `{start}`"))?;
    rest.get(..to).ok_or_else(|| "bad offset".to_owned())
}

/// The string literals and integers of one `{ … }` table row, in order.
fn row_fields(row: &str) -> Vec<String> {
    row.split(',')
        .map(|field| {
            field
                .trim()
                .trim_end_matches("_ustr")
                .trim_matches('"')
                .to_owned()
        })
        .collect()
}

fn number(field: &str, what: &str) -> Result<u32, String> {
    field
        .parse()
        .map_err(|_| format!("commontimenodecontext.cxx: {what} `{field}` is not a number"))
}

/// The number after `prefix` on a line, up to its closing parenthesis or colon.
fn number_after(line: &str, prefix: &str) -> Result<Option<u32>, String> {
    let Some(at) = line.find(prefix) else {
        return Ok(None);
    };
    let rest = line.get(at + prefix.len()..).ok_or("bad offset")?;
    let digits: String = rest
        .trim_start()
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    number(&digits, prefix).map(Some)
}

/// A C++ source's MPL-2.0 licence header: its first block comment that names the licence.
fn cxx_header<'t>(text: &'t str, source: &str) -> Result<&'t str, String> {
    text.split("/*")
        .skip(1)
        .filter_map(|comment| comment.split_once("*/").map(|(inner, _)| inner))
        .find(|comment| comment.contains("Mozilla Public"))
        .ok_or_else(|| format!("{source}: no licence header"))
}

/// `EffectMigration.cxx`'s `deprecated_FadeEffect_conversion_table`: each ODF 1.0 transition style (`FadeEffect`)
/// and the `transitions.xml` preset LibreOffice migrates it to, in file order (a style's first row is the one
/// `EffectMigration::SetFadeEffect` takes). The file is LibreOffice's own and pinned by its SHA-256; this reads the
/// shape the table takes there, and fails on any other.
fn generate_migration(
    folder: &str,
    source: &str,
    sha256: &str,
    text: &str,
) -> Result<String, String> {
    let mut out = preamble(cxx_header(text, source)?, folder, source, sha256);
    let list = between(
        text,
        "deprecated_FadeEffect_conversion_table[] =",
        "{ FadeEffect_NONE, nullptr }",
    )?;
    let mut rows = Vec::new();
    for line in list.lines().map(str::trim) {
        let Some(row) = line.strip_prefix('{').filter(|row| !row.is_empty()) else {
            continue;
        };
        let row = row
            .split("//")
            .next()
            .and_then(|row| row.trim().strip_suffix("},"))
            .ok_or_else(|| format!("{source}: a row `{line}`"))?;
        let fields = row_fields(row);
        let [effect, preset] = fields.as_slice() else {
            return Err(format!("{source}: a row `{line}`"));
        };
        let effect = effect
            .strip_prefix("FadeEffect_")
            .filter(|name| name.bytes().all(|b| b.is_ascii_uppercase() || b == b'_'))
            .ok_or_else(|| format!("{source}: a fade effect `{effect}`"))?;
        rows.push((effect.to_owned(), preset.clone()));
    }
    if rows.is_empty() {
        return Err(format!("{source}: the table came out empty"));
    }
    out.push_str("/// Each ODF 1.0 transition style (`FadeEffect`, without its prefix) and the `transitions.xml` preset LibreOffice\n/// migrates it to (`deprecated_FadeEffect_conversion_table`), in file order: a style's first row is the one\n/// `EffectMigration::SetFadeEffect` takes.\npub const FADE_EFFECTS: &[(&str, &str)] = &[\n");
    for (effect, preset) in &rows {
        let _ = writeln!(out, "    ({effect:?}, {preset:?}),");
    }
    out.push_str("];\n");
    Ok(out)
}

/// PowerPoint's preset tables in `commontimenodecontext.cxx`: `convert_subtype` (sub-type numbers and names),
/// `preset_mapping` (class, preset id, `ooo-*` id), and `getConvertedSubType`'s per-preset exceptions. The file is
/// LibreOffice's own and pinned by its SHA-256; this reads the shapes those three take there, and fails on any other.
fn generate_pptx(folder: &str, source: &str, sha256: &str, text: &str) -> Result<String, String> {
    let mut out = preamble(cxx_header(text, source)?, folder, source, sha256);

    let mut subtypes = Vec::new();
    let list = between(text, "convert_subtype::getList()", "{  0, nullptr }")?;
    for line in list.lines().map(str::trim) {
        if let Some(row) = line
            .strip_prefix('{')
            .and_then(|row| row.strip_suffix("},"))
        {
            let fields = row_fields(row);
            let [id, name] = fields.as_slice() else {
                return Err(format!("{source}: a sub-type row `{line}`"));
            };
            subtypes.push((number(id, "a sub-type")?, name.clone()));
        }
    }

    let mut presets = Vec::new();
    let list = between(text, "preset_mapping::getList()", "{ 0,0,nullptr }")?;
    for line in list.lines().map(str::trim) {
        if let Some(row) = line
            .strip_prefix('{')
            .and_then(|row| row.strip_suffix("},"))
        {
            let fields = row_fields(row);
            let [class, id, name] = fields.as_slice() else {
                return Err(format!("{source}: a preset row `{line}`"));
            };
            let class = match class.rsplit("::").next() {
                Some("ENTRANCE") => "entrance",
                Some("EXIT") => "exit",
                Some("EMPHASIS") => "emphasis",
                Some("MOTIONPATH") => "motion-path",
                _ => return Err(format!("{source}: a preset class `{class}`")),
            };
            presets.push((class, number(id, "a preset id")?, name.clone()));
        }
    }

    let mut overrides = Vec::new();
    let mut kept = Vec::new();
    let body = between(text, "OUString getConvertedSubType(", "\n}\n")?;
    let mut preset = None;
    let mut subtype = None;
    for line in body.lines().map(str::trim) {
        if let Some(id) = number_after(line, "nPresetId !=")? {
            kept.push(id);
        } else if let Some(id) = number_after(line, "nPresetId ==")? {
            preset = Some(id);
        } else if let Some(number) = number_after(line, "nPresetSubType ==")? {
            subtype = Some(number);
        } else if let Some(number) = number_after(line, "case")? {
            subtype = Some(number);
        }
        if let Some(at) = line.find("pStr = \"") {
            let name = line
                .get(at + "pStr = \"".len()..)
                .and_then(|rest| rest.split('"').next())
                .ok_or("bad offset")?;
            let preset = preset.ok_or_else(|| format!("{source}: `{line}` outside a preset"))?;
            let number = subtype
                .take()
                .ok_or_else(|| format!("{source}: `{line}` without a sub-type"))?;
            overrides.push((preset, number, name.to_owned()));
        }
    }
    if subtypes.is_empty() || presets.is_empty() || overrides.is_empty() || kept.is_empty() {
        return Err(format!("{source}: a table came out empty"));
    }

    out.push_str("use super::PptxPreset;\n\n");
    out.push_str("/// PowerPoint's preset ids and the `ooo-*` preset each is (`preset_mapping`), in file order.\npub const PRESETS: &[PptxPreset] = &[\n");
    for (class, id, name) in &presets {
        let _ = writeln!(
            out,
            "    PptxPreset {{ class: {class:?}, id: {id}, name: {name:?} }},"
        );
    }
    out.push_str("];\n\n/// `presetSubtype` numbers and the sub-type each converts to (`convert_subtype`), in file order.\npub const SUBTYPES: &[(u32, &str)] = &[\n");
    for (id, name) in &subtypes {
        let _ = writeln!(out, "    ({id}, {name:?}),");
    }
    out.push_str("];\n\n/// The entrance and exit presets that convert a sub-type number their own way (`getConvertedSubType`): preset\n/// id, number, sub-type.\npub const SUBTYPE_EXCEPTIONS: &[(u32, u32, &str)] = &[\n");
    for (preset, number, name) in &overrides {
        let _ = writeln!(out, "    ({preset}, {number}, {name:?}),");
    }
    out.push_str("];\n\n/// The entrance and exit presets whose sub-type stays a number (`getConvertedSubType`).\npub const SUBTYPES_AS_NUMBERS: &[u32] = &[");
    let kept: Vec<String> = kept.iter().map(ToString::to_string).collect();
    out.push_str(&kept.join(", "));
    out.push_str("];\n");
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scans_tags_and_skips_comments() {
        let text = "<?xml version=\"1.0\"?>\n<!--\n * licence\n-->\n<a x=\"1\" y='a>b'>\n<!-- <b z=\"2\"/> -->\n<c w=\"&amp;\"/></a>";
        let (header, tags) = scan(text).unwrap();
        assert_eq!(header, "\n * licence\n");
        assert_eq!(
            tags,
            [
                Tag {
                    name: "a".into(),
                    attributes: vec![("x".into(), "1".into()), ("y".into(), "a>b".into())]
                },
                Tag {
                    name: "c".into(),
                    attributes: vec![("w".into(), "&".into())]
                },
            ]
        );
        assert!(scan("<!-- open").is_err());
        assert!(scan("<!-- h --><a x=1>").is_err());
    }

    #[test]
    fn generates_rows_in_file_order() {
        let text = "<!-- licence -->\n<anim:par pres:preset-id=\"wipe-up\"><anim:transitionFilter smil:type=\"barWipe\" smil:subtype=\"topToBottom\" smil:direction=\"reverse\"/></anim:par>";
        let out = generate("sd/xml", "transitions.xml", "00", text).unwrap();
        assert!(out.starts_with("// licence\n"), "{out}");
        assert!(out.contains("TransitionPreset { id: \"wipe-up\", smil_type: \"barWipe\", subtype: Some(\"topToBottom\"), reverse: true, fade_color: None, lo_subtype: None },"));
        assert!(
            out.contains("from LibreOffice core `sd/xml/transitions.xml`"),
            "{out}"
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    #[test]
    fn reads_the_fade_effect_migration_table() {
        let text = "/* -*- Mode: C++ -*- */\n/*\n * Mozilla Public License\n */\nderecated_FadeEffect_conversion_table_entry const deprecated_FadeEffect_conversion_table[] =\n{\n// OOo 1.x transitions\n    { FadeEffect_FADE_FROM_LEFT,            \"wipe-right\" },\n    { FadeEffect_STRETCH_FROM_TOP,          \"wipe-down\" },  // todo\n\n    { FadeEffect_NONE, nullptr }\n};\n";
        let out = generate("sd/source/core", "EffectMigration.cxx", "00", text).unwrap();
        assert!(
            out.starts_with("// * Mozilla Public License\n\n// Generated"),
            "{out}"
        );
        assert!(
            out.contains("    (\"FADE_FROM_LEFT\", \"wipe-right\"),\n    (\"STRETCH_FROM_TOP\", \"wipe-down\"),\n];"),
            "{out}"
        );
        // Any other row shape fails rather than being skipped.
        let odd = text.replace("FadeEffect_FADE_FROM_LEFT", "FADE_FROM_LEFT");
        assert!(generate("sd/source/core", "EffectMigration.cxx", "00", &odd).is_err());
        let empty =
            "/* Mozilla Public */ conversion_table[] =\n{\n    { FadeEffect_NONE, nullptr }";
        assert!(generate("sd/source/core", "EffectMigration.cxx", "00", empty).is_err());
    }

    #[test]
    fn reads_the_pptx_tables() {
        let text = "/* -*- Mode: C++ -*- */\n/*\n * Mozilla Public License\n */\nconst convert_subtype* convert_subtype::getList()\n{\n    {   1, \"from-top\" },\n    {  0, nullptr }\n};\nconst preset_mapping* preset_mapping::getList()\n{\n        { css::presentation::EffectPresetClass::EXIT, 2    ,\"ooo-exit-fly-out\" },\n        { 0,0,nullptr }\n};\nOUString getConvertedSubType( sal_Int16 c )\n{\n        if( nPresetId != 21 )\n        {\n            if( nPresetId == 17 )\n            {\n                if( nPresetSubType == 10 )\n                    pStr = \"across\";\n            }\n            else if( nPresetId == 5 )\n            {\n                switch( nPresetSubType )\n                {\n                case  5: pStr = \"downward\"; break;\n                }\n            }\n        }\n}\n";
        let out = generate("oox/source/ppt", "commontimenodecontext.cxx", "00", text).unwrap();
        assert!(
            out.starts_with("// * Mozilla Public License\n\n// Generated"),
            "{out}"
        );
        assert!(
            out.contains("PptxPreset { class: \"exit\", id: 2, name: \"ooo-exit-fly-out\" },"),
            "{out}"
        );
        assert!(out.contains("    (1, \"from-top\"),"), "{out}");
        assert!(
            out.contains("    (17, 10, \"across\"),\n    (5, 5, \"downward\"),"),
            "{out}"
        );
        assert!(
            out.contains("SUBTYPES_AS_NUMBERS: &[u32] = &[21];"),
            "{out}"
        );
        assert!(
            generate(
                "oox/source/ppt",
                "commontimenodecontext.cxx",
                "00",
                "/* Mozilla Public */"
            )
            .is_err()
        );
    }
}
