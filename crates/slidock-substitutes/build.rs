//! Writes `$OUT_DIR/faces.rs`: one `Face` per entry of `fonts/core.json`, in the manifest's order, each file built in
//! from beside the manifest (feature `embed`; without it the list is empty). A malformed manifest or a missing file
//! fails the build.

use std::error::Error;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use serde_json::Value;

fn main() -> Result<(), Box<dyn Error>> {
    let out = PathBuf::from(std::env::var("OUT_DIR")?).join("faces.rs");
    let code = if std::env::var_os("CARGO_FEATURE_EMBED").is_some() {
        let fonts = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR")?).join("fonts");
        faces(&fonts)
            .map_err(|message| format!("{}: {message}", fonts.join("core.json").display()))?
    } else {
        "pub const FACES: &[Face] = &[];\n".to_owned()
    };
    std::fs::write(&out, code)?;
    Ok(())
}

/// A file name the manifest may give: a plain `.woff2` name, no folder.
fn plain_woff2(name: &str) -> bool {
    name.len() > ".woff2".len()
        && name.ends_with(".woff2")
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
}

fn faces(fonts: &Path) -> Result<String, String> {
    let manifest = fonts.join("core.json");
    println!("cargo::rerun-if-changed={}", manifest.display());
    let text = std::fs::read_to_string(&manifest).map_err(|e| e.to_string())?;
    let json: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let entries = json
        .get("faces")
        .and_then(Value::as_array)
        .ok_or("no `faces` list")?;
    let mut code = String::from("pub const FACES: &[Face] = &[\n");
    for (index, entry) in entries.iter().enumerate() {
        let text = |key: &str| {
            entry
                .get(key)
                .and_then(Value::as_str)
                .ok_or(format!("face {index}: no `{key}`"))
        };
        let (family, file, sha256, licence) = (
            text("family")?,
            text("file")?,
            text("sha256")?,
            text("licence")?,
        );
        let weight = entry
            .get("weight")
            .and_then(Value::as_u64)
            .filter(|weight| (1..=1000).contains(weight))
            .ok_or(format!("face {index}: no `weight` in 1..=1000"))?;
        let style = match text("style")? {
            "normal" => "Normal",
            "italic" => "Italic",
            other => return Err(format!("face {index}: style {other}")),
        };
        if !plain_woff2(file) {
            return Err(format!(
                "face {index}: {file} is not a plain .woff2 file name"
            ));
        }
        if sha256.len() != 64
            || !sha256
                .bytes()
                .all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(format!("face {index}: {sha256} is not a SHA-256"));
        }
        let path = fonts.join(file);
        if !path.is_file() {
            return Err(format!("{} is missing", path.display()));
        }
        println!("cargo::rerun-if-changed={}", path.display());
        let path = path
            .to_str()
            .ok_or(format!("{} is not UTF-8", path.display()))?;
        writeln!(
            code,
            "    Face {{ family: {family:?}, weight: {weight}, style: Style::{style}, file: {file:?}, \
             sha256: {sha256:?}, licence: {licence:?}, bytes: include_bytes!({path:?}) }},"
        )
        .map_err(|e| e.to_string())?;
    }
    code.push_str("];\n");
    Ok(code)
}
